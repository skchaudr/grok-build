# Review: Pi lane workflow (trace-deck `99629c1`) before it builds SPEC-usable

Read-only review, 2026-10-26, Claude Fable 5.1. Sources: trace-deck `.pi/` at `99629c1` on the Air; pi-subagents 0.82.1 (same version on Air and Mini, `package.json`); Pi 1.4.0 on both; `~/.pi/agent/settings.json` on both; grok-build on the Mini at `cf91b26f`.

## 1. Verdict: AMEND. Rebuild on pi-workflows; do not iterate on this lane.

The engine Sab chose is @osolmaz/pi-workflows. The lane at `99629c1`, and its copy at `6ddabdd`, run on pi-subagents. That is the finding (A0). A1-A9 document why the pi-subagents lane would also fail on SPEC-usable; they are evidence against continuing on it, not a to-do list.

## 2. Amendments, ranked

**A0. Wrong engine. The chosen engine is pi-workflows; the lane is pi-subagents code.**
- Evidence: Sab's direction for today was @osolmaz/pi-workflows (HARNESS-FACTS "Pi workflow packages", 2026-10-26). The lane calls `runs.run` (`lane.js:3`, `brief.js:3`) and is launched via the `subagent` tool (`lane.md:5`); on the Air that API exists only in pi-subagents (13 files use `runs.run(`; pi-workflows and pi-extensible-workflows use it in 0 files). pi-workflows is `defineWorkflow` in `.pi/workflows/<name>.workflow.ts`; it does not depend on pi-subagents. trace-deck `PLAN.md` names neither package. The switch was never stated. `6ddabdd` (2026-10-26 21:15, Sonnet 5.5) copied the lane into eight `opus-proposal-*` files that apply A1-A9 and still run on pi-subagents.
- Consequence: A1-A9 fix a lane on an engine Sab did not pick. Applying them does not make the lane acceptable; it makes the switch look reviewed. This review's first version missed it, which is the failure the "workflow review" stage exists to catch.
- Change: stop work on `99629c1` and `6ddabdd`. Build the six stages as a pi-workflows `.pi/workflows/<name>.workflow.ts` (`defineWorkflow` + `agent({prompt, expectedOutput})`, HARNESS-FACTS:57). A1-A9 do not transfer: the 12,000-char gate cap, 16 KiB args, 120 s gate timeout and `subagent` semantics are pi-subagents facts. The pi-workflows build gets its own workflow review against its own limits before it touches SPEC-usable.

**A1. Gate JSON exceeds the 12,000-character typed-gate cap.**
- Evidence: `gate.sh:8` prints `output.slice(-6000)` plus `diff.slice(0, 60000)` as one JSON line. `src/runs/shared/acceptance.js:1221` `TYPED_VERIFY_OUTPUT_MAX_BYTES = 12_000`; `:1138` truncates stdout with a `...[truncated]` marker; `:1239` turns a truncated document into `structuredOutputError`, run `failed`. `lane.js:4-12` then sees no `structuredOutput`, runs the fix round, fails again, and returns `stage: "fix"` with `run.output` (worker prose) as the "gate" text.
- Consequence: any slice whose diff plus test tail exceed about 11k characters after JSON escaping fails with green tests and a misleading message. A cargo test tail alone is several thousand characters.
- Change: gate JSON carries `{ ok, output: <last 2000 chars> }` only. The reviewer gets the diff from a tool: the builtin `reviewer` already has `watchdog_diff` (`agents/reviewer.md`, `docs/watchdog.md:74`), so td-reviewer gains `tools: read, watchdog_diff`.

**A2. The brief is pasted into `args.brief`, which is capped at 16 KiB.**
- Evidence: `lane.md:5` "args: { brief: <the brief's full contents> }". `src/workflows/workflow-resources.js:6,10,116-117` reject args over 16384 bytes and any string over 16 KiB. `docs/workflows.md:425` says to pass briefs as files. `brief.js:12` requires every edited range quoted verbatim.
- Consequence: T0 alone moves about 300 lines of `traces_modal.rs` verbatim; the brief will not fit, and `/lane` refuses to launch.
- Change: `args.brief` becomes the path. td-worker gets `read` (tools: `read, edit, write`) with the instruction "read only the brief"; `lane.js` passes `reads: [args.brief]`, which prepends `[Read from: <path>]` to the task (`src/runs/background/async-execution.js:1698-1700`). `reads:` alone does nothing without a read tool.

**A3. The gate's default timeout is 120 s.**
- Evidence: `acceptance.js:1190` `DEFAULT_VERIFY_TIMEOUT_MS = 120_000`, used at `:1276` and `:1414` when `timeoutMs` is absent; `lane.js:2` sets none.
- Consequence: `cargo test -p xai-grok-pager --lib traces_` (SPEC-usable T0 verify) is unlikely to finish in 120 s; the gate aborts, the lane fails. Not measured (no builds on Sab's machines for this review).
- Change: `gate: { command, output: "json", timeoutMs: <per repo> }`; make `timeoutMs` a lane parameter.

**A4. `contact_supervisor` cannot work under `async: false`.**
- Evidence: `td-worker.md:18` tells the worker to call it when the brief is incomplete and stop. `lane.md:5` and `brief.md:5` force `async: false`. `docs/workflows.md:17`: a child awaited by an `async: false` workflow gets a tool error from `contact_supervisor`, not a reply.
- Consequence: the only escape hatch for an incomplete brief errors out; `lane.js` cannot tell "worker stopped, brief incomplete" from a gate failure and spends the fix round on the same brief.
- Change: drop `contact_supervisor` from td-worker; the worker ends its reply with one line `BLOCKED: <what is missing>` when it cannot proceed, and `lane.js` returns `{ ok: false, stage: "implement", reason }` before running the gate decision. Missing facts are the scout review's job (A6).

**A5. A missing or failed review passes, and an empty diff passes.**
- Evidence: `lane.js:18-19` `verdict ?? "missing"`, `ok: verdict !== "BLOCK"`; `lane.md:7` commits on `ok`. A reviewer that times out, errors, or omits the verdict line yields `ok: true`. `gate.sh:7` can produce an empty diff (worker's `oldText` never matched, tree unchanged, tests green); nothing checks it.
- Consequence: the "complete" condition is weaker than the target shape's stage 6; the parent commits unreviewed or no-op work.
- Change: `ok: review.ok && (verdict === "OK" || verdict === "OK with notes")`; before review, `if (!g.diff?.trim()) return { ok: false, stage: "implement", reason: "no diff" }` (with A1, test diff emptiness in `gate.sh` and return `{ ok: false, output: "no diff" }`).

**A6. Stages 3 and 5 of the target shape are missing or under-informed.**
- Evidence: `brief.js` runs the scout and returns; no review of the brief before `lane.js` (stage 3 absent). `brief.js:7-15` never asks the scout whether the slice's premises hold. `lane.js:14-17` gives the reviewer brief + diff + gate tail, not the slice text, so it cannot judge against original intent (stage 5). The scout is the builtin at `thinking: low` (`agents/scout.md:5`).
- Consequence: an unsupported premise in SPEC-usable (the spec marks several "(U)") reaches the builder unchallenged, and the build reviewer cannot catch drift from the slice.
- Change: one `runs.run("scout-review", { agent: td-reviewer, task: "Slice: <path>. Brief: <path>. Verify every quoted range and fact against the repo. End with `Brief verdict: OK|BLOCK`." })` before implement, gated on the verdict line; pass the slice path to the build reviewer too. Add to the scout task: "state which premises of the slice you could not confirm".

**A7. Model and thinking per role are whatever the parent session runs.**
- Evidence: no `model:` in `td-worker.md`, `td-reviewer.md`, `scout.md`; no `subagents` key in `~/.pi/agent/settings.json` on either machine. Resolution ends at the parent session model (`docs/models.md:13`); `defaultModel` is `gemini-3.8-flash-high` on both machines (`settings.json:3`). Builtins inherit the Pi default (`docs/agents.md:38`).
- Consequence: scout, builder and both reviewers run on the same model, chosen by whoever opened the parent session, not by role.
- Change: `model: <provider/id:level>` in each agent file (or `subagents.agentOverrides.<name>.model`). Which model per role is Sab's call; the frontmatter row is the parameter.

**A8. Not installed where SPEC-usable must run.**
- Evidence: SPEC-usable:12 "Verification runs on sab-mini". grok-build on the Mini has no `.pi/` directory. The Air's grok-build is `main...origin/main [ahead 1, behind 119]`. `gate.sh:4` runs `bun run test && bunx tsc --noEmit`; `brief.js:14` hardcodes that acceptance line. Workflow and agent discovery are per project (`docs/agents.md:19-24`; `tool-reference.md:17` resolves `workflow:` against the request cwd).
- Consequence: the workflow cannot execute any grok-build slice today.
- Change: commit `.pi/{agents,workflows,prompts,gate.sh}` into grok-build (Mini checkout). `gate.sh` runs the command it is given: `eval "$*"` with `lane.js` building `command: "sh .pi/gate.sh " + args.verify` (each slice's **Verify** line, e.g. SPEC-usable:48). `brief.js` takes `args.verify` for the acceptance line.

**A9. Concurrent slices in one checkout mix their diffs.**
- Evidence: `gate.sh:6-7` stages and diffs the whole tree; `lane.md:7` commits the whole tree. Per-child `worktree: true` exists (`docs/workflows.md:462-470`), the gate runs inside it (`tool-reference.md:442`), but the worktree is removed after capture and the patch lands in a handoff manifest that nothing in `lane.js` applies. grok-build `AGENTS.md:5-7` (landed in `ff10175e`, after this review started) requires independent steps to run concurrently, so serial is a temporary exception, not the end state.
- Consequence: `runs.all` over N lanes in the shared cwd would review and commit other lanes' changes.
- Change: run lanes serially, one `/lane` per slice, until a patch-apply step exists. Not a blocker for the first slices (T0, B0 are serial by the spec).

## 3. First library entry

Stage order, verdict lines, parameters and fixed points below carry over to the pi-workflows build; the file names and APIs do not (they are pi-subagents). Files (committed per repo under `.pi/`): `workflows/slice.js` (scout → scout review → implement → gate → one fix → build review; returns `{ ok, stage, verdict, reason }`), `prompts/slice.md` (parent: read nothing, one `subagent` call, commit on `ok`), `agents/{scout,worker,reviewer}.md`, `gate.sh` (runs `$*`, prints `{ ok, output }`).

Varies per repo or role: `args.verify` (gate command, from the slice's Verify line), gate `timeoutMs`, `model:` per agent file, the excluded paths in `gate.sh` (`:!.pi`).

Fixed: the stage order and verdict lines (`Brief verdict:`, `Merge verdict:`), the gate JSON shape, `ok` requiring an explicit OK, briefs passed by path with `reads:`, workers with `read, edit, write` and no bash, reviewers with `read, watchdog_diff` and no write, `async: true` at the top level.

## 4. Not verified

- Actual cargo test wall time for the T0 gate on the Mini (A3 is from the code default, not a measurement).
- Whether `watchdog_diff` sees intent-to-add files the way `gate.sh:6` prepares them.
- Whether a managed worktree shares the cargo target dir (relevant only once A9's worktree path is used).
- Output file directories are created (`src/runs/shared/single-output.js:287` mkdirs), so `.pi/briefs/` missing is not a blocker; not run.
- The Air's grok-build checkout (119 behind origin/main) was not inspected beyond `git status`; the review assumes the Mini checkout is the one the lane runs in.
