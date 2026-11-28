# Run1 integration report: packet 1, executed timeline (the pilot)

## Edits

All 6 edits from `run1/grok-build/task-scripts/acpx-disclosure-returns/executed-timeline.edits.md` applied to `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs`, each OLD block matching exactly once. Behaviour kept as returned. No other file touched.

## Fixes

None. No test changed, no code adjusted.

## Test

`cargo test --release -p xai-grok-pager acpx` (one cycle, `cargo fmt` skipped):

`test result: ok. 234 passed; 0 failed; 1 ignored; 0 measured; 11422 filtered out`

## Commit

`a7e66127` on `main`, pushed. Only `run.rs`, trailer `Co-authored-by: claude-code sonnet-5.5`.

## Install

`scripts/install-fork.sh`, exit 0, release build in 2m 07s. Checksums match here and on sab-mini@sab-air:

- grok-fork `acdda74b0793`
- grok-traces `4ed4422c5114`
- grok-acpx-acp `564192835115`
- grok-pi-acp `11fdb8dd1610`

Acceptance (fold/expand on Space/Right, build → repair → build order, untaken switch branch absent from Skipped, waiting stage and choices visible) was checked by the unit tests only, not on the installed build's screen.

## Review points 1–4: what I observed (not decided)

1. **Answered gate after the engine moves on.** In `rebuild_tree` (`run.rs:795-800`) a step is added to `seen_finished_attempts`, and so collapsed, only when it is not the waiting or running stage. While the run is `Waiting` the gate step is the waiting stage and stays expanded (test `answered_gate_keeps_expanded_choices_across_set_answered_and_sync`). Once the status leaves `Waiting`, the gate step is seen for the first time as finished and collapses on that sync. No test covers the move-on step itself.
2. **Orphan flow node in Skipped.** Skipped now draws from `prune_run_for_graph(&self.run)` (`run.rs:920`, `run.rs:1310`). In the fixture used by `detail_target_for_step_child_live_root_and_skipped_node` (edit 4), `cleanup` was given an edge from `deploy` (`run.rs:2445-2449`) so it stays in Skipped. A node with no edge to it is dropped from Skipped by that pruning. I did not run a flow with an orphan node.
3. **`E` toggle-all.** `shift_e_toggles_all_folds` (`run.rs:2315`) now expects the first `E` to expand (roots start collapsed), the second to collapse to 4 rows (3 steps plus the skipped root), the third to expand again. It passes.
4. **Recorded live step.** `running_loop_with_recorded_live_step_keeps_live_attempt_expanded_after_repair` uses `finished_ms = 0` for the last step. The loader (`acpx/bundle.rs:1000-1001`) maps a missing `finishedAt` to 0, so that path is reachable if acpx writes a step before it finishes. This repo only reads those files; I did not look at the writer, so whether it does is unverified.
