# Watch A: running-call visibility, `grok claude` pane 17e421

Watcher: watch-a (Haiku 5.5, read-only). Pane: Rex block 17e421, `grok claude`, Sonnet 5.5 medium, F3 panel open on Session.
Start 14:29. Report-by 15:30. Raw snapshots (15 s cadence) in `/private/tmp/claude-501/watch/watch-a/`. Snapshotter PID 14662.

Interface text is quoted as the screen drew it. Conversation text is not copied.

## Call 1: four Bash cards in one turn, 14:30 to 14:31

Turn start: user message at gutter time 14:30 (text not copied). The turn was the only tool activity in the window.

- 14:31:04 (snap-143104): status row `⠹ Waiting for response… 3.5s` with right side `11s ⇣61.9k [stop]`. One card header visible: `◆ Run git log --oneline -8 --stat | head -40; ls docs | rg -i 'investigate|watch|visib'`. Panel status `running`, context `6% 61k/1.0M`, `tool calls   —`, `turns        —`.
- 14:31:19 (snap-143119): status row `⠋ Waiting for response… 4.6s` with right side `26s ⇣71.4k [stop]`. Four card headers visible: `◆ Run git log …`, `◆ Run cat docs/INVESTIGATE-running-tool-visibility-2026-11-16.md; echo ======; git show 6add1aad --…`, `◆ Run sed -n 30,48p docs/INVESTIGATE-running-tool-visibility-2026-11-16.md | cut -c1-1800; echo; ec…`, `◆ Run ls docs | rg -i '^WATCH'; git fetch -q && git log origin/main --oneline -3`. Panel still `status running`, `tool calls   —`.
- 14:31:34 (snap-143134): status row gone. Bottom line `Worked for 30s`. Panel `status idle`. The four cards are no longer in view; the pane has scrolled to the reply.

Measured: the turn ran from about 14:30:53 to about 14:31:30, roughly 30 s by `Worked for 30s`. Per-card length is not measurable from the screen.

What the screen showed while the cards ran:
- Chat: a `◆ Run` header per Bash call, with the command text truncated at the card edge. No elapsed time on any card. No status mark (no spinner, no ✓, no ✗) visible on any card header in the captured width.
- Status row: a spinner and a timer, and the label `Waiting for response…`. The label names the model wait, not the tool. The timer ran 11 s, then 26 s, so it counts the whole turn.
- F3 panel (Session page): `status running` only. `tool calls` stayed at `—` and `turns` at `—` during the call. The panel did not show the calls.
- Steps and Agents pages were not open, so their rows for these calls are not observed.

## Other observations

- 14:32 to 14:33 (snap-143219 to snap-143304): pane idle, Session page, `status idle`. A block-viewer overlay (`[x]` close, footer `Esc:close  │  Enter:quote  │  /:search …`) was open at 14:32:41 and closed by 14:33:11. Who opened it is not visible.
- No tool call was in progress at the start of the watch (14:29 to 14:31:04), and none after 14:31:34 up to 14:33.

## Seen / not seen

Seen:
- Bash tool cards appear in the chat with `◆ Run` and command text while a turn runs.
- The status row carries spinner, `Waiting for response…`, a turn timer, and a live token count (`⇣61.9k` to `⇣71.4k`).
- The F3 Session panel shows `status running` during the turn, and `tool calls` and `turns` stay `—`.
- No elapsed time or status mark on the running cards.

Not seen:
- Steps or Agents page during a call, so the "running" state on Steps and the empty Agents page are unverified on screen.
- A call longer than about 20 s with a visible card timer. The one turn was 30 s total and showed no per-card timer.
- A backgrounded Bash call (no call was backgrounded in the window).
- A stall or failure. No run stalled.
- Session page showed no per-call output while the cards ran.

Next: keep sampling until 15:30. The next call will be read on Steps as well, if the pane is switched there by its user. Watcher does not send input.
