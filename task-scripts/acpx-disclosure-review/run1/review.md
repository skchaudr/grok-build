# executed-timeline pilot: return review

Return: `grok-build/task-scripts/acpx-disclosure-returns/executed-timeline.edits.md` (6 edits, 1140 lines). Usage: `_returns/executed-timeline.usage.json` — 229 s, 0 tool calls, `end_turn`, 36.0k in, 33.5k out (21.9k thinking). Base: `run.rs` at a3267d75; unchanged through c8662887.

## Format

All six OLD blocks match `run.rs` exactly once; none overlap. No "Coordinator follow-ups" section. Only `run.rs` is edited. Applied to a scratch copy: `executed-timeline.applied.diff` (+330 test lines, about 60 production lines). Not compiled or tested.

## Production change (edits 1–2)

- `RunViewState.seen_finished_attempts: HashSet<String>`; `rebuild_tree` collapses a finished step root only the first time its `attempt_id` is seen, after the tree is rebuilt. A user's expansion survives `sync` and the noise toggle because a seen attempt is never collapsed again.
- Waiting stage and the running stage are excluded from the default.
- Live loop attempt: `running_step_idx` is the last step when it is on `current_node` and `is_live_step` (`finished_ms <= 0` or before `started_ms`). Before, any earlier step on the current node suppressed the `live/<node>` root, so build → repair → build (live) showed no live stage; now it does.
- Skipped section now draws from `prune_run_for_graph(&self.run)`, keeping only `NotRun` nodes.
- Names used all exist: `HashSet` import, `prune_run_for_graph` import, `FoldState::collapse`, `Step.started_ms/finished_ms: i64`, `run_mut`, `make_step`, `set_answered`, `sync`.

## For integration

1. Answered gate: it stays expanded while the run is still `Waiting` (tested through `set_answered` and `sync`). Once the engine moves the run on, the gate step is no longer the waiting stage, so it counts as newly finished and collapses on the next sync. The packet says answering does not collapse it; it does not say what happens after the engine moves on. Decide whether that is wanted.
2. Edit 4 changes an existing fixture: `cleanup` gets an edge from `deploy`, because pruning now drops an unreachable node from Skipped. Confirm an orphan flow node vanishing from Skipped is intended.
3. `E` (toggle all): the first press now expands everything, since roots start collapsed; the test was updated to match.
4. The "recorded live step" fixture uses `finished_ms = 0`. Whether acpx ever writes a step before it finishes is unverified; if not, only the no-step `live/<node>` path is reachable in practice.
5. Test keys such as `live/build/10` are inferred from existing live-root rendering; the targeted tests will confirm them.

Checks to run: `cargo fmt`, then `cargo test --release -p xai-grok-pager acpx_modal::run`.
