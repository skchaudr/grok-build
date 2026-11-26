import { spawnSync } from "node:child_process";
import {
  agent,
  allowSettingsPath,
  choice,
  compute,
  defineHumanChoices,
  defineWorkflow,
  humanDecision,
  humanDecisionEdge,
  idempotentEffect,
  settingsRoute,
  shell,
  textInput,
  workflowSettings,
} from "@osolmaz/pi-workflows";

// Bounded Sonnet repair loop for the gathered beta8 merge tree.
// cargo_check fails -> repair_limit -> scout_and_plan (read-only) -> approve_plan (you) -> sonnet_repair
// -> cargo_check ... at most maxRepairs times; then your_review.
// Every Claude step states its tools (allowedTools); pi-workflows runs it in Claude's default mode and
// denies anything else, so no step inherits the interactive bypassPermissions setting.
const WORKTREE = "/Volumes/Mini-SSD/offload/repos/grok-build/.worktrees/beta8-rest";
const READ_TOOLS = ["Read", "Grep", "Glob"];
const REPAIR_TOOLS = [...READ_TOOLS, "Edit", "Write", "Bash"];

type Settings = { maxRepairs: number };
type RepairOutput = { notes: string };
type PlanOutput = { findings: string; plan: string; files: string[] };
type PlanAnswer = { choice: "approve" } | { choice: "revise"; input: { instructions: string } } | { choice: "stop" };

const settings = workflowSettings({
  initial: { maxRepairs: 3 },
  parse: (value) => {
    const v = value as Partial<Settings> | null;
    if (!v || !Number.isInteger(v.maxRepairs) || (v.maxRepairs as number) < 0) {
      throw new Error("maxRepairs must be a non-negative integer");
    }
    return { maxRepairs: v.maxRepairs as number };
  },
  paths: [allowSettingsPath("/maxRepairs", { read: ["human"], replace: ["human"] })],
});

const gateChoices = defineHumanChoices({
  accept: choice({ label: "Accept the tree" }),
  repair: choice({ label: "One more scout, plan and repair" }),
  stop: choice({ label: "Stop" }),
});

const planChoices = defineHumanChoices({
  approve: choice({ label: "Approve: Sonnet edits per this plan" }),
  revise: choice({
    label: "Revise the plan",
    input: textInput({ name: "instructions", prompt: "What should the plan change?" }),
  }),
  stop: choice({ label: "Stop" }),
});

function cargoErrors(results: Record<string, { error?: string } | undefined>): string {
  return results.cargo_check?.error ?? "(no diagnostics captured)";
}

function earlierRepairs(steps: Array<{ nodeId: string; outcome?: string; output?: unknown }>): string[] {
  return steps
    .filter((s) => s.nodeId === "sonnet_repair" && s.outcome === "ok")
    .map((s, i) => `${i + 1}. ${(s.output as RepairOutput).notes}`);
}

function git(args: string[]): { status: number | null; stdout: string } {
  const r = spawnSync("git", args, { cwd: WORKTREE, encoding: "utf8" });
  return { status: r.status, stdout: r.stdout ?? "" };
}

export default defineWorkflow({
  name: "beta8-repair",
  title: "Repair beta8-rest until cargo check passes",
  settings,
  startAt: "check_disk_space",
  maxSteps: 60,
  nodes: {
    // Stops the run (no failure route) instead of handing a full disk to Sonnet.
    check_disk_space: shell({
      effect: idempotentEffect("beta8-repair.check_disk_space"),
      timeoutMs: 30_000,
      exec: () => ({
        command: "bash",
        cwd: WORKTREE,
        args: [
          "-c",
          "FREE_GB=$(df -g /Volumes/Mini-SSD | awk 'NR==2 {print $4}'); " +
            'if [ "$FREE_GB" -lt 15 ]; then echo "only ${FREE_GB}GB free on Mini-SSD" >&2; exit 2; fi; ' +
            'echo "${FREE_GB}GB free"',
        ],
      }),
    }),

    // Non-zero exit fails the node, so the card shows the failure; diagnostics go to stderr -> result.error.
    cargo_check: shell({
      effect: idempotentEffect("beta8-repair.cargo_check"),
      timeoutMs: 5_400_000,
      exec: () => ({
        command: "nice",
        args: ["-n", "15", "cargo", "check", "--workspace", "--bins", "--message-format", "short"],
        cwd: WORKTREE,
      }),
    }),

    repair_limit: settingsRoute({
      run: ({ state, settings }) => {
        const used = state.steps.filter((s) => s.nodeId === "sonnet_repair").length;
        return { route: used < (settings as Settings).maxRepairs ? "under_limit" : "limit_reached", used };
      },
    }),

    // Read-only pass: what is broken, which merge side it comes from, and the exact edits proposed.
    scout_and_plan: agent({
      agent: "claude",
      model: "sonnet",
      allowedTools: READ_TOOLS,
      timeoutMs: 1_200_000,
      prompt: ({ results, outputs, state }) => {
        const earlier = earlierRepairs(state.steps);
        const answer = outputs.approve_plan as PlanAnswer | undefined;
        const previous = outputs.scout_and_plan as PlanOutput | undefined;
        return [
          `Read-only scouting inside ${WORKTREE}: the gathered Grok Build + Dwsy v0.1.14-beta.8 merge (conflicts resolved and staged, merge not committed).`,
          "You can only read files (Read, Grep, Glob). Do not try to edit or run commands.",
          "`cargo check --workspace --bins` fails with:",
          "```",
          cargoErrors(results),
          "```",
          ...(earlier.length > 0 ? ["Earlier repair passes in this run:", ...earlier, ""] : []),
          ...(answer?.choice === "revise" && previous
            ? ["Your previous plan:", previous.plan, "The reviewer asked for these changes:", answer.input.instructions, ""]
            : []),
          "1. Group the errors by cause. For each, say which side it comes from: our fork (multi-host, traces, acpx, piw, Pi/Claude notification routing, isolated grok-home wiring) or upstream beta.8 (mention catalogs, working messages, timeline, thinking_stream_mode, tool-call density, emphasis), and what the other side expects.",
          "2. Propose the exact edits that fix them while keeping both sides' intent: file, item, and change. No deleting or stubbing features to make it compile.",
          'Submit {"findings": "<causes and sides>", "plan": "<numbered edits>", "files": ["<path relative to the worktree>"]}.',
        ].join("\n");
      },
      expectedOutput: '{"findings": "...", "plan": "...", "files": ["crates/..."]}',
      validate: (raw: unknown): PlanOutput => {
        const out = raw as Partial<PlanOutput> | null;
        if (!out || typeof out.findings !== "string" || !out.findings.trim() || typeof out.plan !== "string" || !out.plan.trim()) {
          throw new Error("Submit non-empty findings and plan strings.");
        }
        if (!Array.isArray(out.files) || out.files.length === 0 || out.files.some((f) => typeof f !== "string" || !f.trim())) {
          throw new Error("files must list the paths the plan edits.");
        }
        return { findings: out.findings.trim(), plan: out.plan.trim(), files: out.files.map((f) => f.trim()) };
      },
    }),

    approve_plan: humanDecision({
      audience: "operator",
      choices: planChoices,
      request: ({ outputs, state }) => {
        const plan = outputs.scout_and_plan as PlanOutput;
        const pass = state.steps.filter((s) => s.nodeId === "sonnet_repair").length + 1;
        return {
          title: `Approve repair plan ${pass} (${plan.files.length} file(s))`,
          subject: { plan },
          presentation: {
            schema: "pi-workflows.decision-presentation.v1",
            summary: "Approve to let Sonnet edit (Read, Grep, Glob, Edit, Write, Bash; no commit), revise, or stop.",
            blocks: [
              { kind: "preformatted" as const, text: plan.findings },
              { kind: "preformatted" as const, text: plan.plan },
              { kind: "preformatted" as const, text: plan.files.join("\n") },
            ],
          },
        };
      },
    }),

    sonnet_repair: agent({
      agent: "claude",
      model: "sonnet",
      allowedTools: REPAIR_TOOLS,
      timeoutMs: 3_600_000,
      prompt: ({ results, outputs, state }) => {
        const plan = outputs.scout_and_plan as PlanOutput;
        const earlier = earlierRepairs(state.steps);
        return [
          `Work strictly inside ${WORKTREE}. It holds the gathered Grok Build + Dwsy v0.1.14-beta.8 merge (conflicts resolved and staged, merge not committed).`,
          "`cargo check --workspace --bins` fails with:",
          "```",
          cargoErrors(results),
          "```",
          ...(earlier.length > 0 ? ["Earlier repair passes in this run:", ...earlier, ""] : []),
          "Carry out this approved plan:",
          plan.plan,
          `Files in the plan: ${plan.files.join(", ")}`,
          "If an edit the errors need is outside the plan, make it only if it is the same kind of fix, and list it in your notes. If the plan is wrong, stop and say why in your notes instead of improvising a different fix.",
          "Do not delete or stub out features to make it compile. Stage edited files with `git add <file>`. Do NOT commit and do not abort the merge.",
          "You may run `nice -n 15 cargo check --workspace --bins --message-format short` yourself.",
          'When done, submit {"notes": "<what you fixed, anything outside the plan, and anything you could not fix>"}.',
        ].join("\n");
      },
      expectedOutput: '{"notes": "what was fixed"}',
      validate: (raw: unknown): RepairOutput => {
        const out = raw as Partial<RepairOutput> | null;
        if (!out || typeof out.notes !== "string" || out.notes.trim().length === 0) {
          throw new Error('Submit {"notes": "<what you fixed>"} with non-empty notes.');
        }
        if (git(["rev-parse", "-q", "--verify", "MERGE_HEAD"]).status !== 0) {
          throw new Error("The merge is no longer in progress (committed or aborted); that was not allowed.");
        }
        const unmerged = git(["diff", "--name-only", "--diff-filter=U"]).stdout.trim();
        if (unmerged) throw new Error(`Unmerged files remain:\n${unmerged}`);
        const markers = git(["grep", "-lE", "^(<<<<<<<|>>>>>>>)( |$)", "--", "crates", "Cargo.toml"]);
        if (markers.status === 0) throw new Error(`Conflict markers in:\n${markers.stdout.trim()}`);
        return { notes: out.notes.trim() };
      },
    }),

    your_review: humanDecision({
      audience: "operator",
      choices: gateChoices,
      request: ({ results, state }) => {
        const passed = results.cargo_check?.outcome === "ok";
        const repairs = state.steps.filter((s) => s.nodeId === "sonnet_repair");
        const notes = repairs
          .filter((s) => s.outcome === "ok")
          .map((s, i) => `${i + 1}. ${(s.output as RepairOutput).notes}`)
          .join("\n");
        return {
          title: passed
            ? `cargo check passed after ${repairs.length} repair(s)`
            : `cargo check still failing after ${repairs.length} repair(s)`,
          subject: { cargo_check: results.cargo_check, sonnet_repair: results.sonnet_repair },
          presentation: {
            schema: "pi-workflows.decision-presentation.v1",
            summary: passed ? "Accept, or stop." : "Another scout, plan and repair pass, or stop.",
            blocks: [
              ...(notes ? [{ kind: "preformatted" as const, text: notes }] : []),
              { kind: "preformatted" as const, text: results.cargo_check?.error ?? "(no errors)" },
            ],
          },
        };
      },
    }),

    accepted: compute({ run: ({ results }) => ({ ok: true, worktree: WORKTREE, cargo_check: results.cargo_check?.outcome }) }),
    stopped: compute({ run: () => ({ ok: false, worktree: WORKTREE }) }),
  },
  edges: [
    { from: "check_disk_space", to: "cargo_check" },
    { from: "cargo_check", switch: { on: "$result.outcome", cases: { ok: "your_review", failed: "repair_limit", timed_out: "repair_limit" } } },
    { from: "repair_limit", switch: { on: "$.route", cases: { under_limit: "scout_and_plan", limit_reached: "your_review" } } },
    { from: "scout_and_plan", switch: { on: "$result.outcome", cases: { ok: "approve_plan", failed: "your_review", timed_out: "your_review" } } },
    humanDecisionEdge({
      from: "approve_plan",
      choices: planChoices,
      cases: { approve: "sonnet_repair", revise: "scout_and_plan", stop: "stopped" },
    }),
    { from: "sonnet_repair", switch: { on: "$result.outcome", cases: { ok: "check_disk_space", failed: "your_review", timed_out: "your_review" } } },
    humanDecisionEdge({
      from: "your_review",
      choices: gateChoices,
      cases: { accept: "accepted", repair: "scout_and_plan", stop: "stopped" },
    }),
  ],
});
