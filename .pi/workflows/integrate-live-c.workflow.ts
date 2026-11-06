import { fileURLToPath } from "node:url";
import {
  agent, assistantMessage, compute, defineWorkflow, idempotentEffect, manualEffect, shell,
} from "@osolmaz/pi-workflows";

const cwd = fileURLToPath(new URL("../../", import.meta.url));
const branch = "integrate/traces-live-c";
const live = "66dfb6157f8be2354dfca215ebe25665880828e9";

// DRAFT: inspect this file before starting it. Uses the originating Pi model.
// No subagents, Rex launches, worktrees, pushes, installs, or deployment.
export default defineWorkflow({
  name: "integrate-live-c",
  title: "Integrate Grok live calls/hooks with Prototype C",
  startAt: "inspect_grok_work",
  maxSteps: 10,
  nodes: {
    inspect_grok_work: agent({
      allowedTools: ["read", "bash"],
      timeoutMs: 600_000,
      prompt: () => [
        `Work only in ${cwd}. This step is inspection: do not edit, checkout, merge, or build.`,
        "Goal: C wins; bring in Grok's live calls/hooks integration.",
        `Inspect the two commits 2b420bae and ${live}, their existing focused tests and any readily available verification evidence.`,
        "Do the minimum focused review needed to catch an obvious blocker. Do not audit the whole repo or redesign the product.",
        "Inspect overlapping changes with main only; follow references only to resolve a concrete integration question.",
        "Confirm main contains Prototype C (1d7a903d); record main's full SHA as base. Confirm the live commit exists.",
        `Require the current branch to be main, no tracked/staged edits, and ${branch} not to exist. Preserve untracked files and other worktrees.`,
        "Set ready=false for a prerequisite failure or concrete integration blocker. Distinguish inspected test code, reported verification, and tests actually run.",
      ].join("\n"),
      expectedOutput: '{ "ready": true or false, "base": "full main SHA", "findings": "Concise evidence and integration notes, or blocker" }',
      validate: (value) => {
        const v = value as { ready?: boolean; base?: string; findings?: string };
        if (typeof v.ready !== "boolean" || typeof v.findings !== "string" || !v.findings.trim()) throw new Error("Need readiness and evidence or blocker.");
        if (v.ready && !/^[a-f0-9]{40}$/.test(v.base ?? "")) throw new Error("Need the inspected main SHA.");
        return v;
      },
    }),
    integrate: agent({
      allowedTools: ["read", "bash", "edit", "write"],
      timeoutMs: 1_800_000,
      prompt: ({ outputs }) => [
        `Work in ${cwd}. Inspection: ${JSON.stringify(outputs.inspect_grok_work)}`,
        `Verify main still equals the inspected base and tracked/staged files are clean. Create ${branch} from that main, using this checkout and its existing build directory.`,
        `Merge ${live} into the integration branch. Resolve conflicts semantically: retain C's board, navigation and existing behavior, adding the live calls/hooks. Do not blindly select an entire side.`,
        "Keep scope to this integration. Preserve other worktrees and all unrelated/untracked files. No stash, cleanup, push, install or deployment.",
        "Commit the candidate integration on this branch. Stage explicit task paths only, never git add -A or git add .",
        "Do not change this workflow. If blocked, preserve partial work and return ready=false with the exact reason; do not improvise around it.",
      ].join("\n"),
      expectedOutput: '{ "ready": true or false, "findings": "What was integrated, or the blocker" }',
      validate: (value) => {
        const v = value as { ready?: boolean; findings?: string };
        if (typeof v.ready !== "boolean" || typeof v.findings !== "string") throw new Error("Need readiness and findings.");
        return v;
      },
    }),
    verify: shell({
      effect: idempotentEffect("integrate-live-c.verify"),
      timeoutMs: 1_860_000,
      exec: () => ({
        command: "bash",
        args: ["-c", [
          "set -euo pipefail",
          `test "$(git branch --show-current)" = '${branch}'`,
          "git diff --quiet && git diff --cached --quiet",
          "git diff --check",
          "git merge-base --is-ancestor 1d7a903d HEAD",
          `git merge-base --is-ancestor '${live}' HEAD`,
          "export CARGO_TARGET_DIR=\"$PWD/target\"",
          "cargo test -p xai-grok-traces --lib merge::tests::",
          "cargo test -p xai-grok-pager --lib acp::live_feed::tests::",
          "cargo test -p xai-grok-pager --lib traces_modal::",
          "git rev-parse HEAD",
        ].join("\n")],
        cwd,
        timeoutMs: 1_800_000,
        maxOutputChars: 30_000,
      }),
      parse: (r) => {
        const log = r.stdout + "\n" + r.stderr;
        const passed = [...log.matchAll(/test result: ok\. (\d+) passed/g)];
        if (passed.length !== 3 || passed.some((m) => Number(m[1]) === 0)) throw new Error("Each focused test group must actually run and pass tests.");
        const revision = r.stdout.trim().split(/\r?\n/).pop()!;
        if (!/^[a-f0-9]{40}$/.test(revision)) throw new Error("Missing verified revision.");
        return { revision, log };
      },
    }),
    repair_route: compute({
      run: ({ outputs }) => ({ route: outputs.repair === undefined ? "repair" : "blocked" }),
    }),
    repair: agent({
      allowedTools: ["read", "bash", "edit", "write"],
      timeoutMs: 1_800_000,
      prompt: ({ results }) => [
        `In ${cwd} on ${branch}, fix only the integration failures below, preserving C and the live behavior.`,
        "One repair pass is allowed. Commit explicit task files only. Do not change the workflow, weaken/delete checks, run competing builds, or touch unrelated files/worktrees.",
        "The program will rerun all three focused checks before merging. If unable to fix, report the exact blocker and preserve the work.",
        JSON.stringify(results.verify),
      ].join("\n"),
      expectedOutput: '{ "findings": "Repair performed or exact blocker" }',
    }),
    merge: shell({
      effect: manualEffect("integrate-live-c.merge"),
      timeoutMs: 60_000,
      exec: ({ outputs }) => {
        const base = (outputs.inspect_grok_work as { base: string }).base;
        const revision = (outputs.verify as { revision: string }).revision;
        return {
          command: "bash",
          args: ["-c", [
            "set -euo pipefail",
            `test "$(git branch --show-current)" = '${branch}'`,
            "git diff --quiet && git diff --cached --quiet",
            `test "$(git rev-parse HEAD)" = '${revision}'`,
            `test "$(git rev-parse main)" = '${base}'`,
            "git switch main",
            `git merge --ff-only '${revision}'`,
            "git rev-parse HEAD",
          ].join("\n")],
          cwd, timeoutMs: 30_000, maxOutputChars: 4_000,
        };
      },
    }),
    finish: agent({
      allowedTools: [], timeoutMs: 300_000,
      prompt: ({ outputs }) => `Report the merged revision and the three checks that passed, briefly and inline. State that no real-session or sab-air runtime verification, push, install or deployment was performed. Evidence: ${JSON.stringify(outputs.merge)}`,
      expectedOutput: assistantMessage(),
    }),
    blocked: agent({
      allowedTools: [], timeoutMs: 300_000,
      prompt: ({ outputs, results }) => `Report the exact blocker and preserved branch/work state briefly. Do not restart, merge, or claim completion. Evidence: ${JSON.stringify({ inspection: outputs.inspect_grok_work, integration: outputs.integrate, verification: results.verify, repair: outputs.repair })}`,
      expectedOutput: assistantMessage(),
    }),
  },
  edges: [
    { from: "inspect_grok_work", switch: { on: "$.ready", cases: { true: "integrate", false: "blocked" } } },
    { from: "integrate", switch: { on: "$.ready", cases: { true: "verify", false: "blocked" } } },
    { from: "verify", switch: { on: "$result.outcome", cases: { ok: "merge", failed: "repair_route", timed_out: "blocked", cancelled: "blocked" } } },
    { from: "repair_route", switch: { on: "$.route", cases: { repair: "repair", blocked: "blocked" } } },
    { from: "repair", to: "verify" },
    { from: "merge", to: "finish" },
  ],
});
