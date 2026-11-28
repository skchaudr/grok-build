# Packet-writer pilot: can a one-shot worker write the slice packet?

Task chosen: packet 7, inspector body cursor and range (contract in `acpx-disclosure-packets.md`, 3b landed in `92f80438`).
Generator: `gemini-3.8-flash-high` via `slices acp` (zero tools; the runner rejects any tool call).
Implementer (step 4): the existing path, `gemini-4-neon` medium, run only if the returned packet passes review.

## Preparation (wall clock, 2026-11-27)

| Step | Clock |
|---|---|
| Request received | 21:42 |
| Read the runbook, packets index, one rendered packet, the task file's `task()` shape, `agy models` | 21:42 to 21:44 |
| Bundle built (`build_bundle.sh`, `gen-tasks.py`) and dry-rendered | 21:44:04 to 21:45:29 |

Bundle: `_bundle/objective.md` (2.4 KB, verbatim packet 7 text and the shared key contract), `_bundle/repo-map.md` (13.6 KB, generated item outline with line numbers for three files), 7 verbatim source ranges of `inspector.rs`, `detail.rs` and `acpx_modal.rs`, and 4 exemplar ranges of `acpx-disclosure-slices.py` (helpers, `attempt-detail`, `bare-command-picker`, `detail-focus`: task text plus its range list). Rendered: 13 excerpts, 61.4 KB, about 15.7k input tokens.

Scout allowance used: 0 tool calls (none allowed). The repository map is the substitute for discovery.

## Open before dispatch

- `slices acp` checks `effective.thought_level` against `--thought-level`. Whether `gemini-3.8-flash-high` reports a `thought_level` option is not known until a call is made.
- The runner's scratch directory comes from `tempfile.mkdtemp`; the packets index requires it repo-local, so the live run sets `TMPDIR` to a folder under `packetgen-v1/`.

## Result 1: Flash writer (gemini-3.8-flash, thought_level=high)

First send failed in 2 s with 0 quota: the ACP agent advertises `gemini-3.8-flash`, not `agy models`' `gemini-3.8-flash-high`; effort goes in `thought_level` (`dispatch-attempt1-model-id.log`). Resend 23:38:29 to 23:43:05: PASS, 0 tool calls, total 256 s, first code 230 s, input 34,172, output 33,566 (thought 30,279, so about 3.3k visible), returned 104 lines / 10.7 KB. Quota used 0.13% weekly, 0.50% 5h.

Review (before the implementer ran):
- Accuracy: 10 of 10 ranges start on an item line and end on the line before the next item; every named helper in detail.rs (`move_cursor`, `toggle_visual`, `handle_esc`, `selected_range`, `ensure_cursor_visible`) and the named test exist. QUESTIONS: none.
- Compatibility: renders through the existing `task()` and `slices acp` unchanged (`inspector-focus-tasks.py` only splits on the three headings): 44.0 KB, 10 excerpts, each under 32 KB.
- Sufficiency gaps, predicted: (1) `per_tab_scroll_memory_and_clamping` (inspector.rs 1054-1099) drives G/g scroll and breaks under the new contract but is outside every range; (2) test helpers `sample_shell_step`, `row_text`, `render_text` and the `TabKind` and `InspectorOutcome` definitions are not attached, so new tests must guess them; (3) the spec leaves choices open ("per-tab in TabEntry or InspectorState", "half the body height (or page step)", cursor unit is a visual row where Opus's is a source line) and omits the empty-artifact tab case.
- Baseline: Opus's packet 7 (`inspector-copy`, v8) attaches all of inspector.rs, 60.3 KB, 13 ranges; run4 return: Neon medium, 257 s, 32,522 in, 34,530 out, 1,608 lines.

Step 4 dispatches the Flash packet unchanged to Neon medium to measure what those gaps cost.

## Result 2: Neon medium on the Flash packet, unedited (Rex pane `slice-inspector-focus-flashpacket`, 23:44:28 to 23:48:45)

PASS, 0 tool calls, total 234.8 s, first code 120.0 s, input 27,211, output 35,833 (thought 16,530), 1,895 lines.
Opus packet 7 baseline (run4): 257.0 s, first code 147.6 s, input 32,522, output 34,530 (thought 18,380), 1,608 lines.

Matching against the base commit `2dee6724` (the working tree already carries someone's applied packet 7, so the check used `git show` copies, not the tree): Flash edit list 7 of 7 OLD blocks match exactly once; Opus edit list 8 of 8.

Predicted gaps, as they showed in the return:
- The unattached tests: the Flash return ends with a conditional coordinator follow-up ("if any existing unit tests assert ... update those assertions") instead of changing `per_tab_scroll_memory_and_clamping`. Opus's return has no follow-ups.
- Test helpers: Flash's new tests call `row_text`, which was not in any attached excerpt (it exists with that name); `render_text` is not used. Opus's tests use both, with `render_text` quoted in OLD.
- Test count: Flash adds 11 tests and rewrites 2; Opus adds 10.

Not measured: neither edit list was compiled or run. Compile and test results decide whether the gaps cost anything.
