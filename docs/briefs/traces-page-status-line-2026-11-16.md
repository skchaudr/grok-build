# Traces page: the chat's live status line as its last row (2026-11-16)

Read `AGENTS.md`, `.pi/agents/delivery-policy.md` and `docs/EXEC-usage-and-running-calls-2026-11-16.md` section E first; they apply to you.

## Intent (Sab's words, 2026-11-16)

> you were not responding, but there clearly was activity ... I have no clue what's going on

> Oh but waiting for response just hit for 6 seconds but I guess its different

Shown on his own screen: the spinner line above the input box (`⠙ Capture this pane while a call is runnin… 0.4s`, or `Waiting for response… 6s`) and told "draw that same line as the last row of the Traces panel too". His answer: "Yeah thats a worthy fix".

## The change

In the F3 side panel, Traces page (`crates/codegen/xai-grok-pager/src/views/side_panel/mod.rs`, which draws `Follow` from `views/traces_follow.rs`), the last row above the page footer shows the current turn activity of this session: the same spinner, label and per-activity timer the chat status row draws (`views/turn_status.rs`, fed by `TurnActivity` from `acp/tracker.rs`). One source: reuse the status row's own label and timer formatting; do not compute a second label or a second clock.

Nothing else changes. Not in scope: the look of running rows, the bare `◆ Tool` row, the clock format, any other page.

## Every state (each needs a snapshot test at 43 and 21 columns)

1. No turn in progress: no status row; the follow view keeps the full height.
2. Model wait, no tool yet (`Waiting for response…`): spinner, label, timer.
3. Tool running: spinner, the tool's description as the chat shows it, timer.
4. Model replying or thinking (whatever labels the chat row uses): same text as the chat row.
5. Narrow: the label truncates with `…`; the timer is never cut.
6. Follow view paused (cursor moved up, `G live` hint showing): the status row still shows; it does not move the cursor or resume following.
7. Panel zoomed: same row, full panel width.
8. Traces page before the index has the session (the one-line note state): the status row still shows.
9. Panel unfocused vs focused: same row.

The row takes one line from the follow view's height only while a turn is in progress. It is not selectable and `y` does not copy it.

## Builder rules

- Edit and commit only; run no cargo. Review your change from the code. Write the tests; the lead's single test run will run them.
- Colours come from the theme fields the chat status row already uses.
- Never tmux, no kills by name, no subagents, nothing on sab-air.
- Commit and push with the trailer `Co-authored-by: claude-code sonnet-5.5`. Report in `docs/BUILD-traces-page-status-line-2026-11-16.md`: files touched, how the panel reaches the turn activity, tests written, anything in this brief you could not do as written.

## Spec reviewer (runs before the builder; a different agent)

Read this brief and the code it names. Do not edit code. Write `docs/REVIEW-brief-traces-page-status-line-2026-11-16.md`: states missing or ambiguous, whether the panel can reach the turn activity without a second source (file and line), anything here that contradicts Sab's quoted words or grok-build's existing behaviour. Per `.pi/agents/delivery-policy.md`: evidence, not a veto; zero findings is a valid result. Commit and push with the same trailer.
