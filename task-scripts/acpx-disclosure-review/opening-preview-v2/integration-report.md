# Opening-screen preview: integration report

Packet `acpx-opening-preview` (second of Sol's two opening-screen packets). Prompt `376d1b57`, return `464696dc`, on top of ledger delta `2bf86ead`.

## Landed

Commit `c6f0e26a2b1b21cb200fc760013e77ee3aa3729c` on `main`, pushed. Product file: `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` only. Evidence in `opening-preview-v2/evidence/`. Trailer `Co-authored-by: claude-code sonnet-5.5`.

**Not installed.** `scripts/install-fork.sh` was not run, per the handoff.

## Edits

All 10 returned edits applied; `evidence/apply-dryrun.txt` shows each OLD block matching current `main` exactly once before `--write` (`apply-write.txt`).

## Review repairs (made inside this packet)

1. **Diagnostic bytes.** `diagnostic_lines_from_text` no longer trims. Lines keep indentation, trailing spaces and tabs; only the line terminator is dropped (blank and whitespace-only lines are still skipped). Tests: `diagnostic_lines_keep_original_bytes_except_the_line_terminator`, `preview_copy_returns_diagnostic_line_with_trailing_space_and_tab` (`y` and a `v` range copy return the exact bytes).
2. **Generic error versus compiler stderr.** `step_diagnostic_lines` used `error` first, so a one-line step error ("command failed") hid multi-line stderr. Now a one-line `error` yields to the first multi-line source among stderr, output tail, stdout; a multi-line `error` still wins; a one-line `error` with nothing else is still shown. Test: `one_line_step_error_does_not_hide_compiler_stderr`. Trade-off: if a one-line error is the real message and stderr is long noise, the stderr is what shows; the `Failed at <attempt> (exit N)` header is unchanged.
3. **Cancelled override.** `is_run_cancelled` dropped the `status_detail` "cancel" substring test, and `build_run_preview_lines` dropped the same test for runs that are not loaded. A run now shows Cancelled only when its status is not Running, Waiting or Completed and its last step's effective outcome is Cancelled. Test: `known_run_status_survives_cancellation_text_and_earlier_cancelled_attempt` covers a completed run with cancel text (stays Completed), a completed run ending on a cancelled step (stays Completed), a failed run with cancel text and a failed last step (Failed), a failed run with an earlier cancelled attempt and a failed last step (Failed), and a run saved as failed that stopped on a cancelled step (Cancelled, "Cancelled at check#1").

Added `focused_preview_swallows_workflow_action_keys`: with the preview focused, Enter, Space, `r`, `a`, `c`, `x`, `1`, Left and Right all return Unchanged, and the mode, focus and selected run do not change.

## One test fix

Run 1 (`test-run1.log`, 246 passed, 1 failed): the packet's `failed_cargo_check_preview_copies_exact_multiline_diagnostic_wide_and_narrow` expected `y` in the narrow layout to copy the error line. The preview cursor intentionally stays where it was left across Esc and Tab (`set_focused` clamps it, does not reset it), so after the earlier range it sat on the `-->` line and `y` copied that. I changed the test, not the code: it now asserts `y` copies the `-->` line, presses `k`, then asserts `y` copies the error line.

## Test

`cargo test --release -p xai-grok-pager acpx`, three runs, one cargo process at a time:

- run 1 `test-run1.log`: `FAILED. 246 passed; 1 failed` (above)
- run 2 `test-run2.log`: `ok. 247 passed; 0 failed`
- run 3 `test-run3.log` (final, exit 0): `test result: ok. 248 passed; 0 failed; 1 ignored; 0 measured; 11422 filtered out`

Covered by passing tests: wide and narrow focus and copy, Esc unwinding (range first, then focus), preserved list row and refresh/filter/tab reachability (`preview_refresh_open_back_tab_reachability_filter_and_empty_layouts`), answered native history (`waiting_group_and_individual_wait_and_answered_wait_preview_evidence`, shows "Answered: approve"), and no workflow actions from preview keys.

## Blockers

None found. Nothing was checked on a rendered screen of the installed build; all evidence is unit and render tests.
