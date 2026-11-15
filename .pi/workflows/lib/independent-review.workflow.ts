import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { action, agent, compute, defineWorkflow, manualEffect } from "@osolmaz/pi-workflows";
import { digest, parseModel, parseReview } from "./landing-contracts.mjs";

type Input = {
  cwd: string;
  goal: string;
  acceptance: string[];
  context: string[];
  revision: string;
  model: ReturnType<typeof parseModel>;
  previous: unknown;
  label: string;
};
const head = (cwd: string) =>
  execFileSync("git", ["rev-parse", "HEAD"], { cwd, encoding: "utf8" }).trim();
export default defineWorkflow({
  source: import.meta.url,
  name: "independent-review",
  input: (raw): Input => {
    const v = raw as Input;
    if (
      !v ||
      !path.isAbsolute(v.cwd) ||
      !v.goal?.trim() ||
      !Array.isArray(v.acceptance) ||
      !v.acceptance.length ||
      !Array.isArray(v.context) ||
      !/^[a-f0-9]{40}$/.test(v.revision) ||
      !/^[a-z0-9-]+$/.test(v.label)
    )
      throw new Error("Review needs checkout, exact revision, goal, acceptance, context and label");
    return { ...v, model: parseModel(v.model) };
  },
  startAt: "prepare",
  maxSteps: 6,
  nodes: {
    prepare: action({
      effect: manualEffect("landing.review-artifacts"),
      run: ({ input, state }) => {
        const v = input as Input;
        if (head(v.cwd) !== v.revision) throw new Error("Review revision is no longer HEAD");
        const dir = path.join(v.cwd, ".pi", "lane", state.runId);
        mkdirSync(dir, { recursive: true });
        return {
          file: path.join(dir, `${v.label}-${randomUUID()}.json`),
          goalDigest: digest({ goal: v.goal, acceptance: v.acceptance }),
        };
      },
    }),
    // New reviewer subagent for each visit; its feedback stays in this origin builder session.
    review: agent({
      timeoutMs: null,
      allowedTools: ["subagent", "read", "rex_finish"],
      prompt: ({ input, outputs }) => {
        const v = input as Input,
          p = outputs.prepare as any;
        return [
          "Your only role in this stage is obtaining an independent clean-context review. Do not review or edit the implementation yourself.",
          `Launch exactly one subagent with fork:false, cwd:${JSON.stringify(v.cwd)}, model:${JSON.stringify(`${v.model.provider}/${v.model.model}:${v.model.thinking}`)}, name:${JSON.stringify(v.label)}. No turn quota.`,
          "Give it exactly the review assignment below; finish your turn so its automatic result can arrive. Do not poll or launch another reviewer on uncertain completion.",
          "REVIEW ASSIGNMENT:",
          `Independently audit revision ${v.revision} in ${v.cwd} against this goal: ${v.goal}`,
          `Acceptance: ${JSON.stringify(v.acceptance)}. Relevant context/evidence: ${JSON.stringify(v.context)}.`,
          `Confirm actual model ${v.model.provider}/${v.model.model}; if unavailable, report blocked, never substitute.`,
          "Inspect the real code and tests, not just the builder's summary. Identify incomplete requested behaviour, concrete correctness defects, and missing decisive tests. No Cargo or installs. No source edits, commits, cleanup, or further delegation.",
          v.previous
            ? `Previous review (verify fixes, do not merely repeat): ${JSON.stringify(v.previous)}`
            : "",
          `Write only ${p.file}: JSON {revision:'${v.revision}',goalDigest:'${p.goalDigest}',verdict:'ready'|'changes'|'blocked',findings:[{id,severity:'P0'|'P1'|'P2',file,message}],summary}.`,
          "Ready means no unresolved findings. Changes needs concrete evidence-backed findings. Blocked means review could not establish an outcome; explain why. Include detailed actionable findings in that file; return its path and a concise outcome to this builder context only.",
          "END REVIEW ASSIGNMENT.",
          `When its result arrives, read ${p.file}, close the completed reviewer with rex_finish, and submit {file:${JSON.stringify(p.file)}}. A failed/unavailable review submits {blocked:'reason'} instead. Never fabricate the review yourself.`,
        ].join("\n");
      },
      expectedOutput: "{file:string} or {blocked:string}",
      validate: (raw, { input, outputs }) => {
        const v = raw as any;
        if (typeof v?.blocked === "string" && v.blocked.trim())
          return {
            review: {
              revision: (input as Input).revision,
              goalDigest: (outputs.prepare as any).goalDigest,
              verdict: "blocked",
              findings: [],
              summary: v.blocked,
            },
          };
        if (v?.file !== (outputs.prepare as any).file)
          throw new Error("Expected exact review artifact");
        const review = parseReview(JSON.parse(readFileSync(v.file, "utf8")));
        if (
          head((input as Input).cwd) !== review.revision ||
          review.revision !== (input as Input).revision ||
          review.goalDigest !== (outputs.prepare as any).goalDigest
        )
          throw new Error("Stale or mismatched review");
        if (
          execFileSync("git", ["status", "--porcelain", "--", ".", ":!.worktrees/"], {
            cwd: (input as Input).cwd,
            encoding: "utf8",
          }).trim()
        )
          throw new Error("Tracked source changed during review");
        return { review, file: v.file };
      },
    }),
    route: compute({ run: ({ outputs }) => ({ route: (outputs.review as any).review.verdict }) }),
    ready: compute({ run: ({ outputs }) => (outputs.review as any).review }),
    changes: compute({ run: ({ outputs }) => (outputs.review as any).review }),
    blocked: compute({
      run: ({ input, outputs, results }) =>
        (outputs.review as any)?.review ?? {
          revision: (input as Input).revision,
          goalDigest: (outputs.prepare as any)?.goalDigest,
          verdict: "blocked",
          findings: [],
          summary: results.review?.error ?? results.prepare?.error ?? "Review unavailable",
        },
    }),
  },
  exits: { ready: { from: "ready" }, changes: { from: "changes" }, blocked: { from: "blocked" } },
  edges: [
    {
      from: "prepare",
      switch: {
        on: "$result.outcome",
        cases: { ok: "review", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "review",
      switch: {
        on: "$result.outcome",
        cases: { ok: "route", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "route",
      switch: { on: "$.route", cases: { ready: "ready", changes: "changes", blocked: "blocked" } },
    },
  ],
});
