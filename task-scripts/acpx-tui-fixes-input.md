# acpx TUI slices: Claude's input for the plan (to merge with Gemini's)

## Base and cost
- Base: grok-build `main` (58db6e5d). Driver: `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/`
  (`agent.rs`, `piw_driver.rs`, `project.rs`); pager side: `crates/codegen/xai-grok-pager/src/piw/` (`gate.rs` …).
- NOT `~/repos/grok-build-acpx-src`: it is on branch `acpx-tui-slices` with an unpushed commit 9dd18a9
  ("Committed after second pass changes not captured", 153 files, -187,855 lines). Sab should look at that
  commit before anything builds on it.
- Baseline: 2026-11-19 `acpx-tui.py`, Argon, ~7k tokens in per slice, median 8.7k returned, ≤ $0.17/slice.
  Engine is `skills/parallel-slices/scripts/slices.py` only; excerpts under 32 KB; `--effort low`.

## Issues seen in the beta8 run (one line each, with what is known)
1. A parked gate is invisible: not inline, not in `/acpx`, not in `/piw`. The gate lives in the session's
   `waiting_piw` (agent.rs:160, set at :863-905); the decision on the server targets the run's origin session
   (here `cli-session-6c75…`). After cargo_check failed in 105 ms the gate was not in `waiting_piw`.
2. Plain text at that gate cancelled the run. Only text matching a label answers (agent.rs:1445-1456,
   `parse_command`); anything else is a normal prompt. Combined with 1, any reply killed the run.
3. `cargo_check` showed `✓ ok` while its output was `ok:false, exitCode:101`. Node status vs step result.
4. The diagram draws every branch (Air path on a Mini run) instead of the path taken.
5. Diagram drawing/formatting is poor (Sab's words: "a fucking mess").
6. compute/shell steps show no live activity (gather and cargo check were invisible).
7. The gate question carries no reason (the compiler error), only an exit code in its header.
8. A waiting gate is not marked "needs your answer"; it looks like a busy step.
9. A step card does not show files changed, or flag files outside the step's list.
10. A step card does not show the current file or progress.
11. A pop-up covered the run view mid-run.
12. No way to see a run's state without reading the server (`runs`, the state DB).

## Slice shape
- 1, 2, 3 are bugs in existing driver code (agent.rs, piw_driver.rs): coordinator fixes with tests, not
  generation slices. They also need a repro first (start a run headless, park it, answer from Grok Build).
- 4, 7, 8, 9, 10, 12 fit new files: a run-path view (taken path only), a gate panel (reason + choices +
  "waiting for you"), a step-card detail block (files changed vs allowed list, current file, exit code),
  a run status line. One new file each, coordinator wires them.
- 5 and 11 need Sab's view of what good looks like before a slice can be written.
- 6 needs the server to emit activity for compute/shell steps (pi-workflows side) before a UI slice can
  show it — a slice written against a wire format that does not exist yet waits (skill rule).
