# Watch A: running-call visibility, `grok claude` pane 17e421

Watcher: watch-a (Haiku 5.5, read-only). Pane: Rex block 17e421, `grok claude`, Sonnet 5.5 medium, F3 panel open.
Start 14:29. Report-by 15:30. Raw snapshots (15 s cadence) in `/private/tmp/claude-501/watch/watch-a/`. Snapshotter PID 14662.

Interface text is quoted as the screen drew it. Conversation text (user prompts, model replies) is not copied; it is shown as `<text>`.

## Turn 1: four Bash cards, 14:30 to 14:31

Turn start: user message at gutter time 14:30 (`<text>`). The only tool activity in the window.

- 14:31:04 (snap-143104): status row `⠹ Waiting for response… 3.5s`, right side `11s ⇣61.9k [stop]`. One card header visible: `◆ Run git log --oneline -8 --stat | head -40; ls docs | rg -i 'investigate|watch|visib'`. Panel body: Session (`id be1e228d-…`, `status running`, `context 6% 61k/1.0M`, `tool calls   —`, `turns        —`).
- 14:31:19 (snap-143119): status row `⠋ Waiting for response… 4.6s`, right side `26s ⇣71.4k [stop]`. Four card headers visible: `◆ Run git log …`, `◆ Run cat docs/INVESTIGATE-running-tool-visibility-2026-11-16.md; …`, `◆ Run sed -n 30,48p docs/INVESTIGATE-running-tool-visibility-2026-11-16.md | cut -c1-1800; …`, `◆ Run ls docs | rg -i '^WATCH'; git fetch -q && …`. Panel still `status running`, `tool calls   —`.
- 14:31:34 (snap-143134): status row gone. Bottom line `Worked for 30s`. Panel `status idle`. The four cards are out of view; the pane has scrolled to `<text>`.

Measured: the turn ran about 14:30:53 to 14:31:30, `Worked for 30s`. The status-row timer counted the whole turn (11 s at 14:31:04, 26 s at 14:31:19), not each call.

While the cards ran:
- Chat: `◆ Run` header per Bash call, command text cut at the card edge. No elapsed time on any card. No status mark (spinner, ✓, ✗) on any card header in the captured width.
- Status row: spinner, `Waiting for response…`, turn timer, live token count. The label names the model wait, not the tool.
- Panel: `status running` only; `tool calls` and `turns` stayed `—`. The panel did not count the calls.
- Steps list not observed during this turn (the panel was not on it).

Same turn, seen afterwards (14:33:34, snap-143334), in the grouped list in the panel body:
- Turn header `#14 › <text>` with `30s`.
- Rows `1 ◆ Bash git log --oneline -8 --stat… 0.5s`, `2 ◆ Bash cat docs/INVESTIG… ✗ 0.4s`, `3 ◆ Bash sed -n 30,48p docs/INVESTI… 0.4s`, `4 ◆ Bash ls docs | rg -i '^WATCH'; … 1.7s`.
- So each call's real duration was 0.4 to 1.7 s, and the 30 s turn was mostly model wait. The ✗ on row 2 is shown; its cause is not visible on screen.

## Turn 2: model-only, 14:33:34 to 14:33:45, no tool calls

- 14:33:34 (snap-143334): status row `⠴ Waiting for response… 6.0s`, right side `6.0s ⇣75.0k [stop]`. The panel body shows the grouped list; the turn's header `#15 › <text>` reads `0.0s` with no child rows.
- 14:33:49 (snap-143349): status row gone, `Worked for 11s`. No rows under `#15` in the captured lines.

Measured: 11 s, zero tool calls. The `#15` header read `0.0s` while the status row counted 6.0 s, so the header shows summed tool time, not turn time. This is the only turn in the window where that can be checked, and it matches.

A model-only turn draws a status row with a spinner and timer, and no card or row. That is the only visible signal of work in that turn.

## Other observations

- Tab bar: the first line showed `[Agents]` in every snapshot from 14:31:04 to 14:34:05, while the body was the Session page (14:31 to 14:31:34) and the grouped list (14:33). The bracket does not follow the body, so it cannot be used to tell which page is open.
- 14:32:41 to 14:33:11: a block-viewer overlay (`[x]` close, footer `Esc:close  │  Enter:quote  │  /:search …`) was open over the chat, then closed. Who opened it is not visible.
- 14:34:20: a block viewer again (`Esc:close  │  Enter:quote  …`) over the chat. Not a tool call.
- 14:34:05: the composer showed a typed draft (`❯ > <text>`) with footer `Enter:send`. No status row.
- 14:29 to 14:31:04 and 14:31:34 to 14:33:34: no tool call in progress.

## Seen / not seen

Seen:
- Bash tool cards in chat with `◆ Run` header and command text, while a turn runs.
- Status row: spinner, `Waiting for response…`, turn timer, live token count (`⇣61.9k` to `⇣75.0k`).
- Session panel: `status running` during a turn; `tool calls` and `turns` stay `—` throughout.
- No elapsed time or status mark on any running card header.
- Grouped list in the panel: turn headers `#N › <text>` with turn time; call rows `N ◆ Bash <desc> <dur>` with per-call duration and `✗` on a failed call. Completed calls show real durations (0.4 to 2.9 s here).
- Model-only turn: status row only, no card, no row, header `0.0s`.

Not seen:
- The grouped list during a running call. The rows appeared only after their turn finished.
- A call longer than about 20 s. The longest tool call in the window was 2.9 s.
- A backgrounded Bash call.
- A stall or failure.
- Any elapsed time on a running card.

Next: sample until 15:30, update this file by 14:50 and at each 15 minutes after. Watcher sends no input.
