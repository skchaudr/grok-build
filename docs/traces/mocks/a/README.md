# Prototype A: closest to grok

Run: `~/traces-checkpoints/grok-proto-a traces` (built from branch `traces/proto-a`).

## Screens (each at full 200 and half 98 columns)

- `list`: sessions in grok's Dashboard look. Header with live/ended/failed counts, a row of views, sections under a rule, two-line rows (title · harness · folder · machine, then where the session ended up), grok's selection band.
- `compact`: `c` toggles one line per session.
- `subagents`: `e` lists a session's subagents in place under it, running first, then newest.
- `longest`, `failed`, `project`, `harness`: the other views (`Tab` / `Shift+Tab`; `Ctrl+G` too, grok's group key).
- `session`: Enter on an ended session opens grok's own read-only session view.
- `follow`: Enter on a live session opens the same view and keeps feeding it as the transcript grows.
- `subagent-open`: Enter on a subagent row opens that subagent's own session.
- `remote`: a sab-air session, copied over ssh and opened in the same view.

## Definitions

- **Active time**: the gaps between consecutive events in the session, minus every gap that ends at a user prompt (the agent was waiting on you) and every gap over 30 minutes. Computed from the traces index (`ACTIVE_SQL`); sab-air's comes from its own index through `sqlite3` over ssh. Refreshed every minute.
- **Failed**: the index's flag. The session's last event was an error and nothing came after it, or the harness wrote a finished-with-error record.
- **Live**: the index's Active status (a recent hook receipt, or a transcript written in the last 5 minutes), or any of its subagents live.
- **Summaries**: `claude -p --model haiku` with no tools, no saved session and no user settings (so no hooks fire), for the 30 newest ended sessions, at most 30 runs per launch. Cached in `~/.grok/traces/summaries.json` and redone when a session grows. `GROK_TRACES_NO_SUMMARIES=1` turns generation off.

## Subagents in grok's Dashboard

The Dashboard lists top-level agents only; a subagent never gets a row. A parent row shows a gray `Subagents N` chip next to its age, counting only subagents still running (`watchers()` in `app/agent_view/queue.rs`); it shortens to `Sub N`, `N bg`, `bg` as the row narrows. You reach subagents by opening the parent: its Tasks pane (`▾ Subagents N`, `Tab` focuses it, `Enter` opens one, `q`/`Esc` back) and the subagent rows in its scrollback. Here the chip counts every subagent, since history is the point, and `e` shows them in place.

## Left out

- `/` search: it falls back to the old table.
- Mouse clicks (the wheel works) and a `?` sheet for the list.
- Following a live sab-air session: it opens as a snapshot.
- sab-air summaries read only the title and last event, not the event tail.
- Opening an async Claude subagent from inside a session view shows an empty takeover. That's how main behaves today; open it from the list instead.
