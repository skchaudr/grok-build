# SPEC: Pi lane (draft)

Draft 2026-10-26, Sonnet 5.5. Nothing here has run live: no Pi session, model call or cargo build was started. Engine: @osolmaz/pi-workflows 0.17.7 (`PW` = `~/.pi/agent/npm/node_modules/@osolmaz/pi-workflows`), children: pi-subagents 0.82.1 (`PS` = `~/.pi/agent/npm/node_modules/pi-subagents`), `TR` = `crates/codegen/xai-grok-traces/src`. Files: `.pi/workflows/step.workflow.ts`, `.pi/agents/lane-{scout,brief-reviewer,worker,build-reviewer}.md`, `.pi/gate.sh`, `.pi/prompts/step.md`. Run: `/step <path-to-step.md>`.

## 1. Goal

One lane that turns one step (AGENTS.md:5: one file or function, diff under ~5k chars) into a reviewed, gated, committed diff on grok-build itself. Every stage runs as a Pi session under `~/.pi/agent/sessions`, which `grok-traces` indexes (`TR/roots.rs:73`), so a stage reads in the traces viewer beside an ordinary "go" session (section 7). The previous lane ran on pi-subagents; this one is a pi-workflows DAG (REVIEW-pi-workflow.md A0).

## 2. Stages and who runs them

The DAG runs in the origin Pi. Every `agent` node is a prompt into that one conversation (`PW/README.md:12-14,224-226`), so a stage that needs fresh context or another model is a pi-subagents child, started by an `agent` node with `allowedTools: ["subagent"]` (`PW/docs/WORKFLOWS.md:212`). Smoke-verified on the Mini: such a node delegates to a child in its own session (SMOKE-pi-dag.md C2); `tools: codemode, read` works in a child (C3). Each such node passes `context: "fresh"` explicitly (explicit wins, `PS/docs/tool-reference.md:82`), receives only paths, and submits `{ok}`; the substance lands in files. `node type` below is the pi-workflows type (`PW/README.md:298-314`).

- **prepare**. `action` (function, `idempotentEffect`). Requires a clean tree (so the later diff is exactly the step), creates `.pi/lane/<run-id>/`, copies the step to `step.md`, reads the step's header lines `Verify:` (required), `Timeout-ms:`, `Builder:`. Emits the absolute paths used below.
- **scout**. `agent` node, child `lane-scout`. Tools `codemode, read, grep, find, ls, write`; fresh. Model param: frontmatter `model:`, default `cliproxy/gemini-3.8-flash-high`. Receives `step.md` and `brief.md` paths. Emits `brief.md` (the child's final reply, saved by `output` + `outputMode: "file-only"`, `PS/docs/tool-reference.md:172-174`): goal, edits as `path:start-end` plus first/last line, verify, expected diff, `## Unconfirmed premises`, not-in-scope. The same agent in `LANE split` mode emits `steps.md` (section 5).
- **scout review**. `agent` node, child `lane-brief-reviewer`. Tools `read, grep, find, ls`; fresh; default `cliproxy/gpt-6-sol` (differs from the scout). Receives step and brief paths. Emits `brief-review.md` ending `Brief verdict: OK|BLOCK`. `brief_route` (`compute`) parses that line from the file (missing = BLOCK; the relayed `{ok}` is not trusted). BLOCK -> `scout_retry` (scout again, given the review path) -> `brief_review` -> second BLOCK -> `brief_stop` (`checkpoint`).
- **build**. `agent` node, child `lane-worker` (default) or `claude-code-writer` | `codex-exec-writer` | `cursor-agent-writer` (pi-subagents builtins, `PS/agents/*.md`), chosen by the step's `Builder:` line. `lane-worker` tools `codemode, read, grep, find, ls, bash, edit, write`, no `contact_supervisor` (REVIEW A4); fresh; default `cliproxy/claude-sonnet-5-5-high`. External adapters use their CLI's own default model (Codex and Cursor refuse a `model`, `PS/docs/agents.md:122`). Receives step and brief paths. Emits tree edits; a reply with `BLOCKED: ...` becomes `{ok:false}`, and `gate_route` stops.
- **gate**. `shell` node (`idempotentEffect`) runs `.pi/gate.sh "<verify>"` (section 4), then `diff` (`shell`) runs `git add -N . && git diff > diff.patch` and prints `{chars}`, then `gate_route` (`compute`): gate ok and diff non-empty -> review; otherwise one `fix`, then `build_stop` (`checkpoint`). A gate node that fails or times out goes to `build_stop`.
- **build review**. `agent` node, child `lane-build-reviewer`. Tools `read, grep, find, ls` (reads `diff.patch`, so it needs no `bash`/`watchdog_diff`); fresh; default `cliproxy/gpt-6-sol` (differs from the builder). Receives step, brief, diff, gate paths. Emits `review.md` ending `Merge verdict: OK|OK with notes|BLOCK`.
- **complete**. `compute`: `ok` only when gate ok, `diff.chars > 0` and the verdict parsed from `review.md` is `OK` or `OK with notes` (missing = not ok). ok -> `commit`; not ok -> `fix` once (same builder child, `LANE fix`, given `review.md` and `gate.json`) -> gate again -> review again; still not ok -> `build_stop`.
- **commit**. `shell` node (`manualEffect`): `git add -A && git commit`, subject `lane: <step title>`, trailer `Co-authored-by: <tool> <model>` (SPEC-usable.md:11): `pi <lane-worker model>` by default, `claude-code|codex|cursor-agent default` for the external builders. The model comes from the project settings override, else the frontmatter `model:`.

## 3. Payloads

Between stages only paths under `.pi/lane/<run-id>/` move: `step.md`, `brief.md`, `brief-review.md`, `gate.json`, `diff.patch`, `review.md`. The dir is gitignored (`/.pi/lane/` in `.gitignore`).

Limits behind the old lane's choice: pi-subagents `args` max 16 KiB (`PS/docs/workflows.md:425`); typed-gate stdout max 12,000 chars (`PS/docs/tool-reference.md:457`, `PS/src/runs/shared/acceptance.js:1221`). Those two bind only a pi-subagents `workflow:`/`gate:` call, which this lane does not use, so they do not force paths here. What does: every stage shares the origin conversation (`PW/README.md:224-226`), so pasted briefs would accumulate in one context; one agent prompt is capped at 96,000 chars and text over 4,000 chars is cut to a head/tail reference (`PW/src/workflows/prompt-evidence.ts:23,29`; `PW/docs/WORKFLOWS.md:894-910`); T0 alone moves about 7k chars of code (section 5). Shell stdout is capped at 1,000,000 chars (`PW/docs/WORKFLOWS.md:388`), so `gate.sh` keeps 2,000 by its own contract.

## 4. Gate contract

`sh .pi/gate.sh "<verify command>"` runs it with `eval` in the repo root, always exits 0, prints one JSON line `{"ok":bool,"output":"<last 2000 chars of stdout+stderr>"}`; no argument prints `ok:false`. The node writes the line to `gate.json` too. `timeoutMs` comes from the step's `Timeout-ms:` (default 1,800,000, a placeholder, unmeasured). The node's own deadline is that plus 60 s, because the engine default is 15 min (`PW/src/workflows/engine.ts:74`; `PW/docs/WORKFLOWS.md:270`). On timeout the engine kills the whole process group (`PW/src/workflows/shell.ts:76,106`). Shell nodes run in the project path (`PW/src/server/workflow-runner-supervisor.ts:61`, `PW/src/extension/index.ts:963`, `shell.ts:71`). Cargo runs on the Mini only (SPEC-usable.md:10).

## 5. From a SPEC-usable slice to steps

The scout's first job on a slice is `LANE split`: it reads the slice and writes `steps.md` (numbered: file, what moves or changes, expected diff, verify, dependencies) plus one `step-NN.md` per line, each with `Verify:` and optional `Timeout-ms:`, `Builder:`. Start it by hand: `subagent` with agent `lane-scout`, task lines `LANE split <id>`, `slice: <path>`, `steps: <dir>/steps.md`. Then `/step <dir>/step-NN.md` per step, in order.

Worked example, T0 (SPEC-usable.md:41-48). Sizes are measured chars of `crates/codegen/xai-grok-pager/src/views/traces_modal.rs`; diff is about twice the moved text (removed plus added) plus `mod`/`use` lines. All steps use T0's line-48 verify. The SPEC's line numbers (`shortcuts` 3425-3470) have drifted; the actual range is 3415-3466, so the scout must re-read.

| # | Step | Moved | Diff | Needs |
|---|---|---|---|---|
| 1 | New `views/traces_body.rs` + `mod` line (`views/mod.rs:28-30`) + `pub(super) use`: `pretty` (2848-2857), `TEXT_KEYS`, `INPUT_KEYS` (2859-2862) | 746 | ~1.7k | none |
| 2 | `push_chunk` (2900-2907), `collect_text` (2877-2898), `body_text` (2864-2875) | 1667 | ~3.5k | 1 |
| 3 | `body_of` (2909-2916), `compose_body` (2918-2929), `RESULT_SEP`, `RESULT_ERR_SEP` (47-49), `NO_MORE` (41-42) | 1179 | ~2.5k | 2 |
| 4 | `wrap_body` (2729-2762) | 1193 | ~2.6k | 1 |
| 5 | `continuation_source` (4195-4200), `split_result` (4220-4234), `continuation` (4285-4304) | 1825 | ~3.9k | 1 |
| 6 | New `views/traces_hints.rs` + `mod` line: `shortcuts` incl. its `plain` closure (3415-3466) | 1735 | ~3.7k | none |

Cuts are by dependency (`body_of` calls `pretty` and `body_text`; `compose_body` calls `body_of`): a moved function cannot call a function left in `traces_modal`, because its helpers are private to that module. Steps 1-5 edit the same two files and run in order. Step 6 is independent but not a pure move: `shortcuts` reads `state.typing`, a private field of `TracesModalState` (`traces_modal.rs:99`) of a private enum `Typing`, so it needs visibility edits; the scout lists that under Unconfirmed premises and the brief reviewer holds it to the step's "no behaviour change".

## 6. Parameters

- Verify command, gate timeout, builder adapter: per step, header lines in the step file (`Verify:`, `Timeout-ms:`, `Builder:`).
- Model per role: per repo, the `model:` line of each agent file. Per-run and settings overrides exist (`PS/docs/models.md:14`); a project `subagents.agentOverrides` entry for `lane-worker` is read for the trailer, a user-level one is not. Origin Pi model (the relay for `agent` nodes) is whatever the session runs.
- Concurrency: serial for now, a temporary exception to AGENTS.md:7. A Pi session holds one active interactive run (`PW/docs/WORKFLOWS.md:15`) and `prepare` requires a clean tree. `worktree: true` exists on pi-subagents children (`PS/docs/tool-reference.md:70`, `PS/docs/workflows.md:462-470`); concurrent steps need it plus a patch-apply step that applies the child's patch to the main tree, which nothing does today (REVIEW A9).

## 7. Measurement

`grok-traces` indexes Pi under root `pi-sessions` = `~/.pi/agent/sessions` (`TR/roots.rs:73`), db `~/.grok/traces/traces.sqlite` (`TR/index.rs:172-174`). Per session (`TR/index.rs:86-99`, `SessionRow` `TR/query.rs:88-131`): `model`, `cwd`, `started_at`, `updated_at`, `event_count`, `user_count`, `assistant_count`, `tool_count`, `error_count`, `input_tokens`, `output_tokens`, `duration_ms`, `status`, `context_tokens`, `context_window`, `native_id`, `parent_native`, `label`, `agent_type`, `failed`, `now_text`. Per event (`TR/index.rs:104-111`, `EventRow` `TR/query.rs:146-164`): `event_type`, `kind`, `role`, `title`, `preview`, `timestamp`, `duration_ms`, `input_tokens`, `output_tokens`, `turn`, `step`, `depth`, `model`, `is_error`, `noise`, `detail`.

Populated per stage by a Pi child: `model` (the per-role pick), `tool_count`, `error_count`, `input_tokens`, `output_tokens`, `duration_ms`, `event_count`, per-event `model`, `duration_ms`, `is_error`. `title` is the first line of the first user message (`TR/index.rs:373-380`), so the task's first line `LANE <stage> <run-id>` names stage and run, and `SessionFilter.text` finds them (`TR/query.rs:252`). The origin session populates the same fields, with the `workflow` tool calls as events.

Child sessions land under the parent's session dir only when the origin Pi has a persisted session (`PS/src/extension/index.js:132-136`, `PS/src/runs/foreground/subagent-executor.js:7366-7371,7381`); with none they go to a temp dir (`index.js:140`), which is not a root (the smoke ran `--no-session`, SMOKE-pi-dag.md:3). Not populated: `parent_native`/`label` for Pi children resolve badly. `links.rs` gives a path of 3+ components `agent_type=task`, `label=<file stem>` and `parent_native=<dir stem>` (`TR/links.rs:88-99`); a pi-subagents child path has five components (`<cwd>/<parent>/<runId>/run-N/session.jsonl`), so its `parent_native` will not equal the origin's `native_id` and the dispatch tree will not link, and `label` reads `session`. Group by the `LANE` title instead. External builders leave no Pi row: Claude Code runs with no session persistence (`PS/docs/agents.md:116`), Codex ephemeral (`:72`).

## 8. Open questions

1. Does `output` + `outputMode: "file-only"` on a `subagent` call write the child's final reply to the given path? Documented (`PS/docs/tool-reference.md:172-174`), not run; if it fails, `brief.md`/review files never appear and the routes stop.
2. Does the pi-workflows server (a global process started on demand) have `cargo` and `node` on its PATH? Shell nodes inherit its env (`PW/src/workflows/shell.ts:75`); unchecked on the Mini.
3. Does a child's first user message begin with the task's first line (the `LANE` title), or does pi-subagents prepend text? Unchecked; and the origin must be a persisted session for child rows to be indexed.
4. Gate timeout (1,800,000) and agent-node timeout (1,800,000) are placeholders; the T0 verify wall time is unmeasured.
5. Trailer model for `lane-worker` ignores a user-level `~/.pi/agent/settings.json` override.
6. `steps.md` is not reviewed before steps run; Sab's rule is that no brief is dispatched unreviewed. Add a split-review stage?
7. The pty test named in T0's Accept (`traces_modal_ledger_trajectory_inspector`) lives in `xai-grok-pager-pty-harness`, outside T0's Verify line; its cargo command is not in the spec.
8. Tracked `.pi/opus-proposal-*` (pi-subagents lane, superseded per REVIEW A0) is still on `main`; delete or keep?
9. Per-role model defaults are my picks from `enabledModels` in `~/.pi/agent/settings.json`; Sab's call.
