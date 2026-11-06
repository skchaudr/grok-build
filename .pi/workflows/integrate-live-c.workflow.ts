import { fileURLToPath } from "node:url";
import {
  agent, assistantMessage, choice, compute, defineHumanChoices, defineWorkflow,
  humanDecision, humanDecisionEdge, idempotentEffect, manualEffect, shell, textInput,
} from "@osolmaz/pi-workflows";

const cwd = fileURLToPath(new URL("../../", import.meta.url));
const branch = "integrate/traces-live-c";
const live = "66dfb6157f8be2354dfca215ebe25665880828e9";
type Change = { id: string; title: string; files: string[]; issues: string; dependencies: string[] };
type Inspection = { base: string; changes: Change[]; findings: string };
type Selection = { valid: boolean; reason: string; base: string; changes: Change[] };
const selectionChoices = defineHumanChoices({
  integrate: choice({ label: "Integrate selected changes", input: textInput({
    name: "ids", prompt: "Which change IDs? Separate IDs with spaces or commas.", minLength: 1,
  }) }),
  end: choice({ label: "End this workflow" }),
});
const recoveryChoices = defineHumanChoices({
  repair: choice({ label: "Authorize recovery", input: textInput({
    name: "instructions", prompt: "What repair/recovery work do you authorize?", minLength: 1,
  }) }),
  end: choice({ label: "End; design another workflow separately" }),
});

// Uses the originating Pi model and the engine's default 100-step ceiling.
// Human gates wait indefinitely. Each recovery requires a new human authorization.
export default defineWorkflow({
  name: "integrate-live-c",
  title: "Inspect Grok → choose changes → integrate with C → build/test",
  startAt: "inspect_grok_work",
  nodes: {
    inspect_grok_work: agent({
      allowedTools: ["read", "bash"], timeoutMs: 600_000,
      prompt: () => [
        `Inspect in ${cwd}. Read-only: no editing, checkout, build, or tests.`,
        `Identify the changes introduced by 2b420bae and ${live}, compared with Prototype C on main.`,
        "Produce a selectable list of coherent changes with stable IDs, affected files, bugs/issues/unknowns, and dependencies on other listed IDs. Include the compile fix with its affected change.",
        "Evaluate the relevant implementation and existing tests/evidence. Follow references only to resolve a concrete issue; do not audit the whole repo. Distinguish inspected code from previously reported verification.",
        `Record main's full SHA as base. Check that main contains C (1d7a903d), the live commit exists, tracked/staged files are clean, and ${branch} does not exist. Report prerequisite problems in findings.`,
        "Your list will be presented at a human gate. Do not choose which changes to integrate or repair them yourself.",
      ].join("\n"),
      expectedOutput: '{ "base": "full main SHA", "changes": [{ "id": "stable-id", "title": "Change", "files": ["path"], "issues": "Bugs, issues, unknowns, or none found", "dependencies": ["other-id"] }], "findings": "Evidence and prerequisite findings" }',
      validate: (value) => {
        const v = value as Inspection;
        if (!/^[a-f0-9]{40}$/.test(v.base) || !Array.isArray(v.changes) || !v.changes.length || !v.findings?.trim()) throw new Error("Need base revision, change list and findings.");
        const ids = v.changes.map((c) => c.id);
        if (new Set(ids).size !== ids.length) throw new Error("Change IDs must be unique.");
        for (const c of v.changes) {
          if (!/^[a-zA-Z0-9_-]+$/.test(c.id) || !c.title?.trim() || !Array.isArray(c.files) || !c.files.length || !c.issues?.trim() || !Array.isArray(c.dependencies) || c.dependencies.some((id) => !ids.includes(id))) throw new Error("Each change needs an ID, title, files, issue assessment and known dependencies.");
        }
        return v;
      },
    }),
    choose_changes: humanDecision({
      audience: "operator", choices: selectionChoices,
      request: ({ outputs }) => {
        const inspection = outputs.inspect_grok_work as Inspection;
        const previous = outputs.select_changes as Selection | undefined;
        return {
          title: "Choose Grok changes to integrate into Prototype C",
          subject: inspection,
          presentation: {
            schema: "pi-workflows.decision-presentation.v1",
            summary: "Review Grok's changes and issues; choose what to integrate. C remains the baseline.",
            blocks: [
              { kind: "paragraph", text: inspection.findings },
              ...(previous && !previous.valid ? [{ kind: "paragraph" as const, text: previous.reason }] : []),
              ...inspection.changes.flatMap((c) => [
                { kind: "section" as const, title: `${c.id}: ${c.title}` },
                { kind: "paragraph" as const, text: `Files: ${c.files.join(", ")}\nIssues: ${c.issues}\nDependencies: ${c.dependencies.join(", ") || "none"}` },
              ]),
            ],
          },
        };
      },
    }),
    select_changes: compute({
      run: ({ outputs }): Selection => {
        const inspection = outputs.inspect_grok_work as Inspection;
        const answer = outputs.choose_changes as { input: { ids: string } };
        const ids = [...new Set(answer.input.ids.trim().split(/[\s,]+/))];
        const unknown = ids.filter((id) => !inspection.changes.some((c) => c.id === id));
        const changes = inspection.changes.filter((c) => ids.includes(c.id));
        const missing = [...new Set(changes.flatMap((c) => c.dependencies).filter((id) => !ids.includes(id)))];
        const reason = unknown.length ? `Unknown IDs: ${unknown.join(", ")}. Please choose again.` : missing.length ? `Required dependencies not selected: ${missing.join(", ")}. Please choose again; nothing is added automatically.` : "Selection accepted.";
        return { valid: !unknown.length && !missing.length && changes.length > 0, reason, base: inspection.base, changes };
      },
    }),
    integrate: agent({
      allowedTools: ["read", "bash", "edit", "write"], timeoutMs: 1_800_000,
      prompt: ({ outputs }) => [
        `In ${cwd}, integrate only these human-selected changes: ${JSON.stringify(outputs.select_changes)}`,
        `Confirm main still equals the selected base, contains C, tracked/staged files are clean, and ${branch} does not exist. If not, return ready=false with the reason.`,
        `Create ${branch} from that main in this checkout. Apply only approved parts of the live commits ending at ${live}. Do not merge the entire live branch unless every listed change was selected.`,
        "Preserve C's board, navigation and existing behavior. Do not blindly select one side of a conflict or add unselected changes/dependencies.",
        "Commit the candidate using explicit task paths only. Preserve unrelated/untracked files and other worktrees. No stash, cleanup, workflow edits, push or deployment.",
        "Do not build or run tests here; the following program step does that for the first time. Return ready=true when the committed candidate is ready, or ready=false with a concrete problem for the human recovery gate.",
      ].join("\n"),
      expectedOutput: '{ "ready": true or false, "findings": "Selected work integrated, or the problem" }',
    }),
    candidate_ready: compute({
      run: ({ outputs }) => {
        const candidate = (outputs.repair ?? outputs.integrate) as { ready?: boolean } | undefined;
        return { ready: candidate?.ready === true };
      },
    }),
    verify: shell({
      effect: idempotentEffect("integrate-live-c.verify"), timeoutMs: 1_860_000,
      exec: () => ({
        command: "bash", cwd, timeoutMs: 1_800_000, maxOutputChars: 30_000,
        args: ["-c", [
          "set -euo pipefail",
          `test "$(git branch --show-current)" = '${branch}'`,
          "git diff --quiet && git diff --cached --quiet",
          "git merge-base --is-ancestor 1d7a903d HEAD",
          "export CARGO_TARGET_DIR=\"$PWD/target\"",
          "cargo build --release -p xai-grok-pager-bin",
          "cargo test --release -p xai-grok-traces --lib",
          "cargo test --release -p xai-grok-pager --lib traces_modal::",
          // This module exists only when the corresponding live-feed change was selected.
          "if test -f crates/codegen/xai-grok-pager/src/acp/live_feed.rs; then cargo test --release -p xai-grok-pager --lib acp::live_feed::tests::; fi",
          "git rev-parse HEAD",
        ].join("\n")],
      }),
      parse: (r) => {
        const log = `${r.stdout}\n${r.stderr}`;
        const passed = [...log.matchAll(/test result: ok\. (\d+) passed/g)];
        if (passed.length < 2 || passed.some((m) => Number(m[1]) === 0)) throw new Error("The selected test groups must actually run and pass tests.");
        const revision = r.stdout.trim().split(/\r?\n/).pop()!;
        if (!/^[a-f0-9]{40}$/.test(revision)) throw new Error("Missing verified revision.");
        return { revision, log };
      },
    }),
    recovery_gate: humanDecision({
      audience: "operator", choices: recoveryChoices,
      request: ({ outputs, results }) => {
        const evidence = { selection: outputs.select_changes, integration: results.integrate, verification: results.verify, repair: results.repair, merge: results.merge };
        return {
          title: "Integration/build problem: recover or end?", subject: evidence,
          presentation: {
            schema: "pi-workflows.decision-presentation.v1",
            summary: "Choose whether to authorize recovery here or end this run and design another workflow.",
            blocks: [
              { kind: "paragraph", text: "The saved step results are available in /piw. The excerpt below shows the current failure context; it may omit longer logs." },
              { kind: "preformatted", text: JSON.stringify(evidence, null, 2).slice(-15_000) },
            ],
          },
        };
      },
    }),
    repair: agent({
      allowedTools: ["read", "bash", "edit", "write"], timeoutMs: 1_800_000,
      prompt: ({ outputs, results }) => [
        `Work in ${cwd}. Human-authorized recovery: ${JSON.stringify(outputs.recovery_gate)}`,
        `Approved selection: ${JSON.stringify(outputs.select_changes)}`,
        `Failure evidence: ${JSON.stringify({ integration: results.integrate, verification: results.verify, repair: results.repair, merge: results.merge })}`,
        "Perform only the authorized recovery while preserving C and the selected scope. Commit explicit task files only. Do not weaken checks, edit the workflow, touch unrelated work, push or deploy.",
        "Do not build/test here; return ready=true when the committed candidate is ready for the program's build/test step. Return ready=false with the reason if another human decision is needed.",
        "A failure returns to the human gate; this authorization does not permit automatic repeated repair attempts.",
      ].join("\n"),
      expectedOutput: '{ "ready": true or false, "findings": "Recovery performed, or problem requiring a human decision" }',
    }),
    merge: shell({
      effect: manualEffect("integrate-live-c.merge"), timeoutMs: 60_000,
      exec: ({ outputs }) => {
        const { base } = outputs.select_changes as Selection;
        const { revision } = outputs.verify as { revision: string };
        return {
          command: "bash", cwd, timeoutMs: 30_000, maxOutputChars: 4_000,
          args: ["-c", [
            "set -euo pipefail",
            `test "$(git branch --show-current)" = '${branch}'`,
            "git diff --quiet && git diff --cached --quiet",
            `test "$(git rev-parse HEAD)" = '${revision}'`,
            `test "$(git rev-parse main)" = '${base}'`,
            "git switch main", `git merge --ff-only '${revision}'`, "git rev-parse HEAD",
          ].join("\n")],
        };
      },
    }),
    finish: agent({
      allowedTools: [], timeoutMs: 300_000,
      prompt: ({ outputs }) => `Briefly report the selected changes, verified revision, build/tests and merge result inline. Use only recorded evidence: ${JSON.stringify({ selection: outputs.select_changes, verification: outputs.verify, merge: outputs.merge })}`,
      expectedOutput: assistantMessage(),
    }),
    ended: agent({
      allowedTools: [], timeoutMs: 300_000,
      prompt: ({ outputs }) => `Briefly report where the operator ended the workflow, the recorded work/branch state, and any issue to carry into a separately designed workflow. Evidence: ${JSON.stringify({ selection: outputs.select_changes, integration: outputs.integrate, recovery: outputs.recovery_gate, repair: outputs.repair })}`,
      expectedOutput: assistantMessage(),
    }),
  },
  edges: [
    { from: "inspect_grok_work", to: "choose_changes" },
    humanDecisionEdge({ from: "choose_changes", choices: selectionChoices, cases: { integrate: "select_changes", end: "ended" } }),
    { from: "select_changes", switch: { on: "$.valid", cases: { true: "integrate", false: "choose_changes" } } },
    { from: "integrate", switch: { on: "$result.outcome", cases: { ok: "candidate_ready", failed: "recovery_gate", timed_out: "recovery_gate" } } },
    { from: "candidate_ready", switch: { on: "$.ready", cases: { true: "verify", false: "recovery_gate" } } },
    { from: "verify", switch: { on: "$result.outcome", cases: { ok: "merge", failed: "recovery_gate", timed_out: "recovery_gate" } } },
    humanDecisionEdge({ from: "recovery_gate", choices: recoveryChoices, cases: { repair: "repair", end: "ended" } }),
    { from: "repair", switch: { on: "$result.outcome", cases: { ok: "candidate_ready", failed: "recovery_gate", timed_out: "recovery_gate" } } },
    { from: "merge", switch: { on: "$result.outcome", cases: { ok: "finish", failed: "recovery_gate", timed_out: "recovery_gate" } } },
  ],
});
