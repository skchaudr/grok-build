# Build: Traces page status line (2026-11-16)

Builder: sonnet-5.5. No cargo run; every line was reviewed from the code. The lead's single test run has not happened yet, so nothing below is compiled or passed.

## Files touched (all under `crates/codegen/`)

- `xai-grok-pager/src/views/turn_status.rs`: new `TurnStatusArgs.phase_timer_min_width` replaces the hard 60-column cut (decision 3); `PHASE_TIMER_MIN_WIDTH` is now `pub`. One new test.
- `xai-grok-pager-minimal/src/live.rs`: passes `PHASE_TIMER_MIN_WIDTH`, so minimal mode is unchanged.
- `xai-grok-pager/src/app/agent_view/session.rs`: `turn_status_activity()` (the state and activity the chat row draws: wake display state, send-now gap, `resolve_turn_activity()`), and `arm_activity_clock()` (the phase-transition arming moved out of the chat's draw).
- `xai-grok-pager/src/app/agent_view/render.rs`: `draw()` calls `arm_activity_clock()` every frame before the split; `draw_chat` takes `status_state` and the activity from `turn_status_activity()`; the arming block is gone from it.
- `xai-grok-pager/src/app/agent_view/side_panel.rs`: `side_panel_turn_row()` fills the new `Info.turn`.
- `xai-grok-pager/src/views/side_panel/mod.rs`: `TurnRow` and `Info.turn`; the Traces page reserves its lowest body row (above the footer) and draws the row in all three branches (Following, note, Failed). 11 tests.
- `xai-grok-pager/src/app/agent_view/side_panel_tests.rs`: 3 agent-level tests.

## How the panel reaches the turn activity

`resolve_turn_activity()` (via `turn_status_activity()`) -> `Info.turn` (`TurnRow`: state, activity, `activity_started_at`, animation tick, bash/pending-input/goal flags) -> `TurnRow::draw` calls the chat's own `render_turn_status` with `turn_elapsed`/`total_tokens` = `None`, `buttons` = `None` (no `[stop]`/`[↓]`, no mouse rect returned), no watchers, no queue, `phase_timer_min_width` 0. Label, colours, truncation, spinner and timer are the chat's code, not a copy. The row shows when the state is not idle and the chat is not in its parked cue (`renders_parked` unless the dock covers cues).

## Tests written (not run)

Panel (`views/side_panel/mod.rs`, inner widths 43 and 21 where it matters): no turn / full height and rows above and footer do not move; model wait; narrow label cut with `…` and timer whole; tool description, tool without description, long description at 21; Thinking, Responding, Compacting, Cancelling; ask tool hides the timer; paused follow keeps its own line above the row and stays paused; unfocused and zoomed width; note and Failed branches at both widths; height 4 (status row wins); other pages draw no row.
Agent level: `Info.turn` follows idle / running / cancelling and equals `resolve_turn_activity()`; both rows on screen at 110x24; at 60x20 with the panel focused the chat is not drawn and the clock is still armed.
turn_status: `zero_min_width_keeps_the_phase_timer_on_a_narrow_row`.

## Not done as written

- "Snapshot test": the tests assert exact row text instead of `insta` snapshots, because no cargo run means no `.snap` files could be generated and the spinner frame and timer depend on the clock. They pin the same widths.
- Every-shipped-theme check (`GROK_THEME`) and the live half-width screen check are the lead's; I did not build or look at a screen.
- `arm_activity_clock` runs every frame, so `last_activity` now returns to `None` when a turn ends (it used to stay stale until the next shown row), and the debug log `turn.phase_transition` can now record `to: "idle"`.
