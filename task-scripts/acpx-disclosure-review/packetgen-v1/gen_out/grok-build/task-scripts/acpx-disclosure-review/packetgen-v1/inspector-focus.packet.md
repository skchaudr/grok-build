SLICE_ID:
inspector-focus

SOURCE_RANGES:
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L57-223
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L224-450
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L451-489
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L581-687
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L1135-1220
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L634-676
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L750-775
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L1004-1014
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L1383-1412
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs#L45-337

SPEC:
Packet 7: Inspector body cursor and range.

Ownership:
- `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs`: edit `TabEntry`, `InspectorState`, `InspectorState::new`, `handle_key`, `scroll`, `render`, `shortcuts`, `max_scroll_y`, `compute_visual_lines`, `render_body`, `VisualLine`, and tests in `mod tests`.
- `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs`: edit only Step-mode items: `handle_step_key`, `active_shortcut_labels`, `STEP_KEYS`, and Step-mode tests in `mod tests`.
- Attached read-only: `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` (the landed packet 3b implementation) as the reference pattern to reuse and mirror.
- Do not edit `detail.rs`, `run.rs`, `ledger.rs`, `modal_view.rs`, `RUN_KEYS`, `LEDGER_KEYS`, `row_extra`, or any Mode::Run / Mode::Ledger routing. Packet 2 owns `row_extra`, Packet 3b owns Run-mode focus and `RUN_KEYS`, and Packet 4 owns modal preview and `LEDGER_KEYS`. Edits in `acpx_modal.rs` must stay strictly disjoint to avoid coordinator integration conflicts.

Goal:
In the inspector (opened when pressing Enter on a step), the operator wants linewise inspection and copying identical to the pager's line viewer and the landed detail pane, plus arrow-key step navigation.
Acceptance sentence: In an open step inspector, the operator can switch between steps using Left/Right arrows without leaving the view, navigate the body text with a line cursor (j/k, Up/Down), select a range with v, copy the exact cursor line or visual range with y (unwrapped without added breaks), and copy the entire tab with Y.

Behaviour contract:
- In Mode::Step, the inspector body is focused.
- Cursor navigation: `j`/`k` and `Down`/`Up` move a line cursor down and up by one visual row and keep it visible (`ensure_cursor_visible`, scrolling `scroll_y` as necessary). `PageDown`/`PageUp` move the cursor by half the body height (or page step); `Ctrl-d`/`Ctrl-u` move by half the body height; `g`/`Home` move cursor to row 0; `G`/`End` move cursor to the last visual row.
- Visual selection: `v` starts or ends a visual line range anchored at the cursor (`toggle_visual`).
- Copy:
  - `y` copies the cursor line (when visual mode is off) or the visual range (when visual mode is on). Copy returns `InspectorOutcome::Copy(text)`, which `handle_step_key` passes through as `AcpxOutcome::Copy(text)` so the standard copy notice is reported. After `y` copies, visual mode is cleared.
  - `Y` copies the whole active tab (`tab.copy` if `Some`, otherwise `tab.text`), preserving the full-tab copy behavior previously on `y`.
  - Copy is plain text: no ANSI styles, line numbers, gutter spacing, indentation, or breaks added by wrapping; a wrapped source line copies as its exact whole source line. A multi-line range joins touched source lines with `\n`.
- Step navigation:
  - `Left` and `[` return `InspectorOutcome::Prev`, switching to the previous step.
  - `Right` and `]` return `InspectorOutcome::Next`, switching to the next step.
  - Horizontal scrolling: `h` and `l` keep horizontal scroll (left and right by `HORIZ_SCROLL_STEP`) when wrapping is disabled (`!self.wrap`).
- Tab switching:
  - `Tab` and `Shift+Tab` (or `BackTab`) cycle forward and backward through inspector tabs.
  - Digit keys `1` through `7` switch directly to that tab index.
  - Switching tabs clears visual mode (`visual_anchor = None`) and clamps the cursor to the visual row count of the newly selected tab.
- Wrapping:
  - `w` toggles wrapping (`self.wrap = !self.wrap`). Visual lines are recomputed, and cursor is clamped and kept visible.
- Escape handling:
  - If visual mode is active (`visual_anchor.is_some()`), `Esc` ends visual mode (`visual_anchor = None`) and stays in the inspector (`InspectorOutcome::Changed`).
  - If visual mode is not active, `Esc` returns `InspectorOutcome::Back`, which returns to Mode::Run and selects the same step row in the run view.
- Footer and Help overlay:
  - `shortcuts(&self)` and `active_shortcut_labels` show step and cursor keys: `Tab/1-7 tab`, `←/→/[/] step`, `j/k line`, `v range`, `y copy`, `Y all`, `w wrap`, (and `r replay` if conversation is present), `Esc back` (or `Esc unselect` when visual mode is active).
  - `STEP_KEYS` in `acpx_modal.rs` is updated to document the new keymap:
    - `"Tab / 1-7"`: `"Switch inspector tab"`
    - `"← / → / [ / ]"`: `"Previous / next step in run"`
    - `"j / k / ↑ / ↓"`: `"Move line cursor"`
    - `"v"`: `"Start or end visual range"`
    - `"y"`: `"Copy cursor line or visual range"`
    - `"Y"`: `"Copy whole tab content"`
    - `"h / l"`: `"Horizontal scroll (when unwrapped)"`
    - `"w"`: `"Toggle line wrapping"`
    - `"PgUp / PgDn"`: `"Page up / down"`
    - `"r"`: `"Replay ACP conversation for this step"`
    - `"?"`: `"Toggle this key reference"`
    - `"Esc"`: `"End visual selection / back to run view"`

Representation guidance (mirroring detail.rs):
- Look at `views/acpx_modal/detail.rs#L45-337`. Mirror `CopyUnit` (`start`, `end`, `text`) and the cursor methods: `move_cursor`, `toggle_visual`, `handle_esc`, `selected_range`, and `ensure_cursor_visible`.
- In `TabEntry` (or `InspectorState`), maintain `cursor: usize` and `visual_anchor: Option<usize>`. If maintained per-tab in `TabEntry`, tabs remember their cursor position; initialize with `cursor: 0` and `visual_anchor: None`.
- In `compute_visual_lines` and `VisualLine`: associate each visual line with its source line unit or maintain a list of `CopyUnit`s for the active tab, where `start..end` are visual line row indices and `text` is the exact source line from `tab.text`.
- When copying with `y`: lookup the unit(s) covering the cursor row (or `selected_range`), and return their source `text` joined by `\n`. Pointers on wrapped continuation lines (where `line_no` is `None`) copy the complete unwrapped source line without wrapping breaks.
- In `render_body`: when row `v_idx` matches the cursor (or falls within `selected_range` when visual mode is active), highlight the row using `theme.bg_highlight`.
- In `handle_step_key`: ensure `InspectorOutcome::Prev`, `Next`, and `Copy(text)` flow into `AcpxOutcome::Changed` and `AcpxOutcome::Copy(text)` as already supported.

Tests:
- In `inspector.rs`:
  - Update `copy_returns_active_tab_full_text`: verify `Y` returns full active tab text; verify `y` returns the cursor line.
  - Update `prev_next_and_back_keys`: verify `KeyCode::Left` returns `InspectorOutcome::Prev` and `KeyCode::Right` returns `InspectorOutcome::Next`, alongside `[` and `]`.
  - Add test `cursor_movement_and_scroll_following`: moving cursor with `j`/`k` and `Down`/`Up` clamps to bounds and updates `scroll_y` via `ensure_cursor_visible`.
  - Add test `visual_range_selection_and_esc_unwind`: pressing `v` sets anchor; moving cursor and pressing `Esc` cancels visual mode without returning `Back`; pressing `Esc` again returns `InspectorOutcome::Back`.
  - Add test `wrapped_line_copies_full_source_unwrapped`: wrap a long line across multiple visual lines; verify pressing `y` on the second visual line copies the complete source line without breaks.
  - Add test `visual_range_multiline_copy`: select across multiple visual lines and verify `y` copies the exact touched source lines joined with `\n`.
  - Add test `json_output_tab_y_copies_line_and_capital_y_copies_raw_json`: on a tab with `copy: Some(raw_json)`, verify `y` copies the current visual line while `Y` copies the full raw JSON.
  - Add test `inspector_render_shows_cursor_highlight`: verify rendered buffer has `theme.bg_highlight` on the cursor row.
- In `acpx_modal.rs`:
  - Update or add test `step_arrow_keys_navigate_steps_and_esc_preserves_selection`: verify Left and Right arrow keys advance and retreat steps in `handle_step_key`.

Stated once:
- `detail.rs` is read-only.
- No new external dependencies, no unsafe code, no panics on small or 0x0 buffer areas.

RANGE_NOTES:
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L57-223: Covers TabEntry, InspectorState, and InspectorState::new to add cursor and visual anchor state fields and initialization.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L224-450: Covers handle_key, scroll, render, shortcuts, and max_scroll_y to implement cursor navigation, visual range, copy keys (y and Y), arrow step keys, and footer shortcut labels.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L451-489: Covers compute_visual_lines to associate visual lines with their unwrapped source line copy units.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L581-687: Covers render_body and VisualLine to render cursor and visual range highlight with bg_highlight.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs#L1135-1220: Covers existing copy and navigation tests to adapt copy_returns_active_tab_full_text and prev_next_and_back_keys, and anchor new cursor/range/copy tests.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L634-676: Covers handle_step_key in the modal to route copy and step navigation outcomes.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L750-775: Covers active_shortcut_labels to ensure Step-mode shortcut labels include cursor, step, and copy indicators.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L1004-1014: Covers STEP_KEYS to update the Step mode help overlay table with cursor, visual range, y/Y, and Left/Right keys.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal.rs#L1383-1412: Covers step_next_bracket_advances_and_esc_preserves_run_selection to test Left/Right arrow step navigation and Esc unwind.
grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs#L45-337: Read-only reference excerpt covering CopyUnit, Rows, DetailPanel, and cursor/visual helper methods landed in packet 3b.

QUESTIONS:
none
