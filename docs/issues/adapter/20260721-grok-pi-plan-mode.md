# Grok-Pi Plan Mode Integration

**Status:** Implemented — verification in progress  
**Scope:** `pi-grok-adapter` lifecycle bridge plus `grok-pi` composition-only extension injection.

## Objective

Let the Pager's existing Plan Mode control constrain a Pi-owned agent without adding a second TUI or modifying Pi core.

## Architecture

| Owner | Responsibility |
|---|---|
| Pager | Existing Plan toggle and native `x.ai/exit_plan_mode` approval UI |
| Adapter | Plan state machine, reminder projection, state persistence, ACP mode updates |
| Injected Pi extension | Pre-execution `tool_call` write gate and model-callable `exit_plan_mode` tool |
| Pi core | Agent loop, session JSONL, tool execution |

## Delivered slices

- [x] `PiPlanTracker`: `Inactive → Pending → Active → ExitPending → Inactive`, full/sparse/re-entry/exit reminder rules, unit coverage.
- [x] ACP `SessionModeState` advertised by `new_session` and `load_session`; `CurrentModeUpdate` confirms mode changes.
- [x] Prompt prefix injection and non-truncating `<session>.plan.md` sidecar creation.
- [x] Process-private control JSON to injected `tool_call` gate: block `bash`, `edit`, and `write`, except exact plan-file writes.
- [x] Pi extension `exit_plan_mode` tool opens native Pager PlanApprovalView through `x.ai/exit_plan_mode`.
- [x] `<session>.plan-mode.json` snapshots survive restart; transient states collapse via tracker restore semantics.
- [x] Added source-identity allowlist entries for the new composition private module and extension source.

## Constraints kept

- No Pi core source modification.
- `pi-grok-adapter` remains headless/library-only; it has no terminal/UI code.
- Pager uses existing native mode and plan-approval surfaces.
- Plan data is session-private JSONL sidecars, never a shared root `plan.md`.

## Verification

| Command | Result |
|---|---|
| `cargo test -p pi-grok-adapter --lib` | PASS — 93 tests |
| `cargo check -p xai-grok-pager-bin --bin grok-pi` | PASS (existing dead-code warnings only) |
| `verify_native_grok.py --workspace . --pi-source pi-main` | Existing baseline/renderer/slash blockers remain; architecture/headless checks PASS. New source files are explicitly allowlisted. |

## Remaining manual validation

1. Launch `grok-pi`, toggle Plan mode, send a prompt; confirm native mode update and plan-file reminder.
2. Verify `edit`/`write` outside the sidecar and `bash` are rejected before execution.
3. Write the plan sidecar, call `exit_plan_mode`, approve and reject once each in the Pager native approval surface.
4. Restart and resume while active; confirm `.plan-mode.json` restores Plan mode.
