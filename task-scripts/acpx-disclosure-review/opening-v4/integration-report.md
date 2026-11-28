# Opening-screen ledger delta: integration report

Packet `acpx-opening-ledger-delta` (first of Sol's two opening-screen packets). Return `129bb962`, product base `d04d7bb3`. One integrator, one cargo build at a time.

## Landed

Commit `2bf86ead148c0322e048cae9f743bae445459bfc` on `main`, pushed. Product files: `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` and `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs`. Evidence in `opening-v4/evidence/`. Trailer `Co-authored-by: claude-code sonnet-5.5`.

**Not installed.** `scripts/install-fork.sh` was not run, held until the preview packet lands.

## Edits

All 20 edits applied. Before writing, `evidence/apply_edits.py` (dry run, `apply-dryrun.txt`) confirmed every OLD block matches current `main` exactly once. Write step: `apply-write.txt`. The return's "Coordinator follow-ups" section (extra `flow_path: None` on `RowExtra` literals) needed no work: after the 20 edits the crate compiled and no `RowExtra` literal was missing the field.

## Fixes

One round, two edits, both in `ledger.rs`. The first test run did not compile (`evidence/test-run1.log`, exit 101): `error: cannot explicitly borrow within an implicitly-borrowing pattern` at the two `let FlowSelection::Run(ref id) = self.flow_selection.as_ref()? else` and `let FlowSelection::WaitingGroup(ref key) = ...` lines in `selected_run` and `selected_waiting_group`. Removed `ref` in both, as the compiler suggested; same binding, no behaviour change, no test touched.

## Test

`cargo test --release -p xai-grok-pager acpx`, second run (`evidence/test-run2.log`, exit 0):

`test result: ok. 239 passed; 0 failed; 1 ignored; 0 measured; 11422 filtered out`

Acceptance tests, all `ok` in that log:

- `ledger::tests::five_unanswered_and_two_answered_native_waits_group_as_five_with_two_under_recent_outcomes`
- `ledger::tests::answered_gate_counts_only_unanswered_and_moves_answered_to_finished`
- `ledger::tests::answered_gate_row_count_includes_the_answered_gate_and_matches_the_rail`
- `ledger::tests::unanswered_waiting_run_remains_in_needs_you_and_reverts_when_answer_removed`
- `ledger::tests::stalled_running_and_answered_waiting_same_flow_name_stay_individual`
- `ledger::tests::waiting_groups_fold_ship_and_mixed_talk_expand_navigate_and_survive_refresh`
- `ledger::tests::waiting_groups_separate_by_source_root_and_recorded_flow_path`
- `ledger::tests::finished_group_folds_expands_navigates_and_keeps_pending_individual`
- `ledger::tests::finished_group_search_matches_older_id_and_refresh_preserves_selection`
- `ledger::tests::inline_preview_replaces_question_and_path_renders_cursor_wrapping_and_scrolls`
- `ledger::tests::selected_unanswered_waiting_run_wraps_200_char_question_and_unselected_stays_one_row`

## Interface for the preview packet (read from `ledger.rs` at `2bf86ead`)

- `pub(super) struct InlinePreviewLine { pub(super) text: String, pub(super) cursor: bool, pub(super) selected: bool }` (`ledger.rs:44`)
- `pub(super) fn set_inline_preview(&mut self, lines: Option<Vec<InlinePreviewLine>>)` (`ledger.rs:407`). While set, the selected row's own question/path is suppressed in favour of the lines.
- `pub fn selected_waiting_group(&self) -> Option<(&str, usize)>`: flow name and member count; `None` outside the Flows tab or when a run is selected (`ledger.rs:377`).
- `pub fn selected_run_is_finished_group(&self) -> bool` (`ledger.rs:390`)
- `pub fn selected_run(&self) -> Option<&RunSummary>` returns `None` when a waiting group is selected.
- `RowExtra` gained `flow_path`, filled by the cached `row_extra` producer in `acpx_modal.rs`.

## Not done, by the brief

No install, no second worker, no live gate action. Nothing was checked on a rendered screen; the render tests above are unit tests.
