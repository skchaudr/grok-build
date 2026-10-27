// Lane: one step (AGENTS.md) -> scout -> brief review -> build -> gate -> build review -> commit.
// Spec: docs/pi-lane/SPEC.md. Run it with `/step <path-to-step.md>` (.pi/prompts/step.md).
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import {
  action,
  agent,
  checkpoint,
  compute,
  defineWorkflow,
  idempotentEffect,
  manualEffect,
  shell,
  type WorkflowNodeContext,
} from "@osolmaz/pi-workflows";

type LaneInput = { step: string };

type Prep = {
  runId: string;
  dir: string; // absolute .pi/lane/<run-id>
  step: string;
  brief: string;
  briefReview: string;
  gate: string;
  diffPatch: string;
  review: string;
  verify: string;
  gateTimeoutMs: number;
  title: string;
  builder: { agent: string; tool: string; model: string };
};

// Builder adapters (pi-subagents builtins, docs/agents.md "Codex/Claude Code/Cursor CLI profiles").
const BUILDERS: Record<string, { agent: string; tool: string }> = {
  worker: { agent: "lane-worker", tool: "pi" },
  "claude-code-writer": { agent: "claude-code-writer", tool: "claude-code" },
  "codex-exec-writer": { agent: "codex-exec-writer", tool: "codex" },
  "cursor-agent-writer": { agent: "cursor-agent-writer", tool: "cursor-agent" },
};
const DEFAULT_GATE_TIMEOUT_MS = 1_800_000; // placeholder, not measured; override per step with `Timeout-ms:`
const AGENT_TIMEOUT_MS = 1_800_000; // engine default is 15 min of active node time (docs/WORKFLOWS.md "agent")

const prep = (ctx: WorkflowNodeContext) => ctx.outputs.prepare as Prep;

/** Last capture of `re` (global, multiline) in a file; "missing" when the file or the line is absent. */
function verdictOf(file: string, re: RegExp): string {
  if (!existsSync(file)) return "missing";
  const all = [...readFileSync(file, "utf8").matchAll(re)];
  return all.length ? all[all.length - 1][1] : "missing";
}

/** Origin-Pi agent node that only runs one pi-subagents child; the child's work lands in files. */
function delegate(stage: string, child: string, task: string[], output?: string): string {
  return [
    `Lane stage: ${stage}. Your only job is to run one child agent and report whether it finished. Read nothing, edit nothing, run nothing else.`,
    `Call the subagent tool once with: agent "${child}", async true, context "fresh"${
      output ? `, output "${output}", outputMode "file-only"` : ""
    }, and exactly this task text:`,
    "<<<",
    ...task,
    ">>>",
    `Wait until the child finishes (poll its status). Submit {"ok": true}, or {"ok": false, "note": "<one line>"} if it failed or its reply starts with BLOCKED.`,
  ].join("\n");
}
const DELEGATED = `{ "ok": true } or { "ok": false, "note": "one line" }`;

const scoutPrompt = (ctx: WorkflowNodeContext, retry: boolean) => {
  const p = prep(ctx);
  return delegate(retry ? "scout (retry)" : "scout", "lane-scout", [
    `LANE scout ${p.runId}`,
    `step: ${p.step}`,
    `brief: ${p.brief}`,
    ...(retry ? [`previous review (fix what it names): ${p.briefReview}`] : []),
  ], p.brief);
};

const builderPrompt = (ctx: WorkflowNodeContext, fix: boolean) => {
  const p = prep(ctx);
  return delegate(fix ? "fix" : "build", p.builder.agent, [
    `LANE ${fix ? "fix" : "build"} ${p.runId}`,
    `step: ${p.step}`,
    `brief: ${p.brief}`,
    ...(fix ? [`review: ${p.review} (exists only if a review ran)`, `gate: ${p.gate}`] : []),
  ]);
};

const stop = (reason: string) => (ctx: WorkflowNodeContext) => {
  const p = prep(ctx);
  return { reason, run: p.dir, brief: p.brief, briefReview: p.briefReview, gate: p.gate, diff: p.diffPatch, review: p.review };
};

export default defineWorkflow<LaneInput>({
  name: "step",
  input: (value): LaneInput => {
    const v = (value ?? {}) as { step?: unknown; task?: unknown };
    const step = typeof v.step === "string" ? v.step : v.task; // `/workflow step <path>` arrives as {task}
    if (typeof step !== "string" || step.trim() === "") throw new Error('input needs {"step": "<path-to-step.md>"}');
    return { step: step.trim() };
  },
  title: ({ input }) => `step: ${path.basename((input as LaneInput).step)}`,
  startAt: "prepare",
  nodes: {
    // Clean-tree check, run dir, step copy, header parse (Verify / Timeout-ms / Builder).
    prepare: action({
      effect: idempotentEffect("lane.prepare"),
      run: ({ input, state }): Prep => {
        const src = path.resolve(process.cwd(), (input as LaneInput).step);
        if (!existsSync(src)) throw new Error(`step file not found: ${src}`);
        const dirty = execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim();
        if (dirty) throw new Error(`working tree not clean (commit or stash first):\n${dirty.slice(0, 1000)}`);
        const dir = path.resolve(process.cwd(), ".pi/lane", state.runId);
        mkdirSync(dir, { recursive: true });
        const step = path.join(dir, "step.md");
        copyFileSync(src, step);
        const text = readFileSync(step, "utf8");
        const header = (key: string) => text.match(new RegExp(`^${key}:[ \\t]*(.+)$`, "m"))?.[1].trim();
        const verify = header("Verify")?.replace(/^`(.*)`$/, "$1");
        if (!verify) throw new Error("step file has no `Verify: <command>` line");
        const builderKey = header("Builder") ?? "worker";
        const b = BUILDERS[builderKey];
        if (!b) throw new Error(`Builder must be one of ${Object.keys(BUILDERS).join(", ")}`);
        // Trailer model: lane-worker's effective model (project settings override, then user, then frontmatter); external CLIs use their own default.
        let model = "default";
        if (builderKey === "worker") {
          const override = (file: string): string | undefined =>
            existsSync(file) ? JSON.parse(readFileSync(file, "utf8"))?.subagents?.agentOverrides?.["lane-worker"]?.model : undefined;
          model =
            override(".pi/settings.json") ??
            override(path.join(homedir(), ".pi/agent/settings.json")) ??
            readFileSync(".pi/agents/lane-worker.md", "utf8").match(/^model:[ \t]*(\S+)/m)?.[1] ??
            "default";
        }
        return {
          runId: state.runId,
          dir,
          step,
          brief: path.join(dir, "brief.md"),
          briefReview: path.join(dir, "brief-review.md"),
          gate: path.join(dir, "gate.json"),
          diffPatch: path.join(dir, "diff.patch"),
          review: path.join(dir, "review.md"),
          verify,
          gateTimeoutMs: Number(header("Timeout-ms")) || DEFAULT_GATE_TIMEOUT_MS,
          title: text.match(/^#\s+(.+)$/m)?.[1].trim() ?? path.basename((input as LaneInput).step),
          builder: { ...b, model },
        };
      },
    }),

    // Stage 1: scout. Writes brief.md (path in `output`), lists unconfirmed premises inside it.
    scout: agent({
      allowedTools: ["subagent"],
      timeoutMs: AGENT_TIMEOUT_MS,
      prompt: (ctx) => scoutPrompt(ctx, false),
      expectedOutput: DELEGATED,
    }),

    // Stage 2: brief review on another model. Verdict is read from brief-review.md, not relayed.
    brief_review: agent({
      allowedTools: ["subagent"],
      timeoutMs: AGENT_TIMEOUT_MS,
      prompt: (ctx) => {
        const p = prep(ctx);
        return delegate("brief review", "lane-brief-reviewer", [
          `LANE brief-review ${p.runId}`,
          `step: ${p.step}`,
          `brief: ${p.brief}`,
        ], p.briefReview);
      },
      expectedOutput: DELEGATED,
    }),
    brief_route: compute({
      run: (ctx) => {
        const p = prep(ctx);
        const v = verdictOf(p.briefReview, /^Brief verdict:[ \t]*(OK|BLOCK)[ \t]*$/gm);
        const hasBrief = existsSync(p.brief) && statSync(p.brief).size > 0;
        const route = v === "OK" && hasBrief ? "build" : ctx.outputs.scout_retry === undefined ? "retry" : "stop";
        return { route, verdict: v, hasBrief };
      },
    }),
    scout_retry: agent({
      allowedTools: ["subagent"],
      timeoutMs: AGENT_TIMEOUT_MS,
      prompt: (ctx) => scoutPrompt(ctx, true),
      expectedOutput: DELEGATED,
    }),
    brief_stop: checkpoint({ summary: "brief blocked twice; a human decides", run: stop("brief review BLOCK after one retry") }),

    // Stage 3: build.
    build: agent({
      allowedTools: ["subagent"],
      timeoutMs: AGENT_TIMEOUT_MS,
      prompt: (ctx) => builderPrompt(ctx, false),
      expectedOutput: DELEGATED,
    }),

    // A builder that reports BLOCKED or failed skips the gate: no wasted verify run.
    build_route: compute({
      run: (ctx) => {
        const last = (ctx.outputs.fix ?? ctx.outputs.build) as { ok?: boolean; note?: string };
        return { route: last.ok === false ? "stop" : "gate", ...(last.note !== undefined ? { note: last.note } : {}) };
      },
    }),

    // Stage 4: gate. Verify command runs in a process group; the node's timeout kills the whole tree.
    gate: shell({
      effect: idempotentEffect("lane.gate"),
      timeoutMs: (ctx) => prep(ctx).gateTimeoutMs + 60_000,
      exec: (ctx) => {
        const p = prep(ctx);
        return {
          command: "sh",
          args: ["-c", 'sh .pi/gate.sh "$1" > "$2"; cat "$2"', "lane-gate", p.verify, p.gate],
          timeoutMs: p.gateTimeoutMs,
          maxOutputChars: 20_000,
        };
      },
      parse: (r) => JSON.parse(r.stdout.trim().split("\n").pop() ?? "{}") as { ok: boolean; output: string },
    }),
    // Writes diff.patch for the reviewer and reports its size; the tree was clean at prepare, so this is the step's diff.
    diff: shell({
      effect: idempotentEffect("lane.diff"),
      exec: (ctx) => ({
        command: "sh",
        args: [
          "-c",
          'git add -N . && git diff -- . ":!.pi" > "$1" && printf \'{"chars":%s}\\n\' "$(wc -c < "$1" | tr -d " ")"',
          "lane-diff",
          prep(ctx).diffPatch,
        ],
        timeoutMs: 60_000,
      }),
      parse: (r) => JSON.parse(r.stdout.trim().split("\n").pop() ?? "{}") as { chars: number },
    }),
    gate_route: compute({
      run: (ctx) => {
        const g = ctx.outputs.gate as { ok: boolean };
        const d = ctx.outputs.diff as { chars: number };
        const route = g.ok && d.chars > 0 ? "review" : ctx.outputs.fix === undefined ? "fix" : "stop";
        return { route, gateOk: g.ok, diffChars: d.chars };
      },
    }),

    // Stage 5: build review on another model; reads step, brief, diff.patch, gate.json by path.
    build_review: agent({
      allowedTools: ["subagent"],
      timeoutMs: AGENT_TIMEOUT_MS,
      prompt: (ctx) => {
        const p = prep(ctx);
        return delegate("build review", "lane-build-reviewer", [
          `LANE build-review ${p.runId}`,
          `step: ${p.step}`,
          `brief: ${p.brief}`,
          `diff: ${p.diffPatch}`,
          `gate: ${p.gate}`,
        ], p.review);
      },
      expectedOutput: DELEGATED,
    }),

    // Stage 6: complete. ok only when gate ok, diff non-empty and verdict OK / OK with notes.
    complete: compute({
      run: (ctx) => {
        const p = prep(ctx);
        const g = ctx.outputs.gate as { ok: boolean };
        const d = ctx.outputs.diff as { chars: number };
        const verdict = verdictOf(p.review, /^Merge verdict:[ \t]*(OK with notes|OK|BLOCK)[ \t]*$/gm);
        const ok = g.ok === true && d.chars > 0 && (verdict === "OK" || verdict === "OK with notes");
        const route = ok ? "commit" : ctx.outputs.fix === undefined ? "fix" : "stop";
        return { ok, route, verdict, gateOk: g.ok, diffChars: d.chars };
      },
    }),
    fix: agent({
      allowedTools: ["subagent"],
      timeoutMs: AGENT_TIMEOUT_MS,
      prompt: (ctx) => builderPrompt(ctx, true),
      expectedOutput: DELEGATED,
    }),
    build_stop: checkpoint({ summary: "gate or build review failed after one fix round; a human decides", run: stop("gate/review not OK after one fix round") }),

    // Commit with the builder's trailer (SPEC-usable: `Co-authored-by: <tool> <model>`).
    commit: shell({
      effect: manualEffect("lane.commit"),
      exec: (ctx) => {
        const p = prep(ctx);
        const verdict = (ctx.outputs.complete as { verdict: string }).verdict;
        return {
          command: "sh",
          args: [
            "-c",
            'git add -A -- . ":!.pi" && git commit -q "$@" && printf \'{"commit":"%s"}\\n\' "$(git rev-parse HEAD)"',
            "lane-commit",
            "-m",
            `lane: ${p.title}`.slice(0, 72),
            "-m",
            `Step: ${(ctx.input as LaneInput).step}\nRun: ${p.runId}\nMerge verdict: ${verdict}`,
            "-m",
            `Co-authored-by: ${p.builder.tool} ${p.builder.model}`,
          ],
          timeoutMs: 60_000,
        };
      },
      parse: (r) => JSON.parse(r.stdout.trim().split("\n").pop() ?? "{}") as { commit: string },
    }),
  },
  edges: [
    { from: "prepare", to: "scout" },
    { from: "scout", to: "brief_review" },
    { from: "brief_review", to: "brief_route" },
    { from: "brief_route", switch: { on: "$.route", cases: { build: "build", retry: "scout_retry", stop: "brief_stop" } } },
    { from: "scout_retry", to: "brief_review" },
    { from: "build", to: "build_route" },
    { from: "build_route", switch: { on: "$.route", cases: { gate: "gate", stop: "build_stop" } } },
    { from: "gate", switch: { on: "$result.outcome", cases: { ok: "diff", failed: "build_stop", timed_out: "build_stop", cancelled: "build_stop" } } },
    { from: "diff", to: "gate_route" },
    { from: "gate_route", switch: { on: "$.route", cases: { review: "build_review", fix: "fix", stop: "build_stop" } } },
    { from: "build_review", to: "complete" },
    { from: "complete", switch: { on: "$.route", cases: { commit: "commit", fix: "fix", stop: "build_stop" } } },
    { from: "fix", to: "build_route" },
  ],
});
