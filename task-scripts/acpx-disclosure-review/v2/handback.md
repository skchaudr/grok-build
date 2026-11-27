# Packet repair handback (v2)

Base: grok-build `9409da87` (target Rust files clean at render time). pi-workflows `3ba95d8` for the PIW excerpts. Engine `~/repos/pi-workflows/skills/parallel-slices/scripts/slices.py` unchanged.

Rendered packets: `task-scripts/acpx-disclosure-review/v2/_packets/` (identical copies in `v2/_prompts/`). The first-review packets in `task-scripts/acpx-disclosure-review/_packets/` are untouched.

Changed: `task-scripts/acpx-disclosure-slices.py`, `task-scripts/acpx-disclosure-packets.md`, this file.

## Departure from the written assignment

The assignment asked for Git diff returns with repo-relative headers, checked with `git apply --check --recount`. Sab's later go replaced that: code packets 1–3 now return exact old/new edit lists (`<id>.edits.md`). Each edit quotes an existing block verbatim and gives the complete replacement; the coordinator matches each OLD exactly once on the pinned base, stops on zero or several matches, applies with the edit tool and lets Git produce the diff. The `--recount` refinement no longer applies. Inner code fences are `~~~` so the engine's outer-fence extraction stays intact.

## Blockers addressed

1. **Ledger.** Probe test `ledger.rs:2191-2278` attached (excerpt now 2124–2278), so its `RowExtra` literals and the `5/262` rail assertion are visible and assigned to the worker. Contract fixed: add `steps_ran: Option<usize>`, keep `rail`; coordinator adds `steps_ran` to literals outside the excerpts (`ledger.rs` ~2719, 2823, 2844, 2879, 2889, 2902, 2953, 2963) and the Space row to the help table (`acpx_modal.rs:856ff`). Fold key Space (unbound in Flows; Tab/BackTab/Left/Right/h/l stay tab switching). Tie-break greatest `started_ms`, then greatest `run_id`. Tests added: stalled Running and answered Waiting stay individual; group under Recent and Name orders.
2. **Attempt detail.** Preview implementation, cache and tests removed; packet edits `detail.rs` only. `acpx_modal.rs:775-839` kept read-only for the call shape. Index §3 records the preview as deferred until landing review. Evidence inputs grounded: `step_evidence.rs` 32–337 (through `read_attempt_tool_calls`) and 339–505 (complete `extract_step_validations_with_outcome`). Worker told not to delete or rename functions and names the out-of-excerpt callers (conversation helpers, `append_value_lines`, inspector's `value_plain_text`, run.rs's `value_inline_summary`).
3. **Gate design.** Added `gate.rs` 1–373 (answer target/parse/`answer_request`, coordinator state, `GateAnswerer`) and 475–813 (`AnswerMachine::poll`, send, refusal, connection-loss restart, finish/abort); `agent.rs` 1124–1217 (gate question paths) and 1389–1527 (occupied plain-text submission, `answer_waiting_gate`). Matrix must separate client behavior from protocol listing and cover both chat submit paths. Landing now names its attention predicate as a dependency on packet 5 and designs against the current ungrouped ledger.

Timeline refinements: run.rs tests 2025–2757 attached whole and `jump_current_or_waiting` complete (700–1075); `step_digest.rs:307-351` (`active_branch_nodes`) attached. Answered gate stage stays expanded. Tests added for the live repeated attempt and the answered gate.

## Checks run

- `slices acp task-scripts/acpx-disclosure-slices.py --dry-run --output task-scripts/acpx-disclosure-review/v2`: 5 packets rendered, nothing sent. This checks prompt construction only.
- Packet input: executed-timeline 19.5k, finished-run-ledger 23.7k, attempt-detail 28.9k, acpx-landing 8.6k, cross-surface-gate 21.6k tokens. Prior ACP receipts (`acpx-piw-slices_out/_returns/*.usage.json`): 17.9k–27.5k in, all `end_turn`, 0 tool calls, 87–174 s.
- Rendered timeline task section read in full.

## Open items

- No independent review of v2 yet.
- Pilot recommendation: `executed-timeline` alone. Not launched.
- Before live dispatch: `slices acp` creates its worker scratch with `tempfile.mkdtemp`, which follows `TMPDIR`; set `TMPDIR` to a repository-local directory for that run.
- Unknown until the pilot: whether the worker copies OLD blocks verbatim, and whether attempt-detail (largest packet) stays within the 360 s timeout.
