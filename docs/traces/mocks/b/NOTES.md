# Prototype B: organization first

Run: `/Volumes/Mini-SSD/offload/repos/grok-build/target-b/release/xai-grok-pager traces` (branch `traces/proto-b`).

## Keys on the board

Tab / Shift+Tab or 1-5: view. `c`: compact or comfortable. `e`: open a session in place (details, then its subagents). Enter: open it in grok's session view (a live one keeps updating). `h`: close, or step to the parent. `/`: search. `H`: machine (all, this Mac, sab-air). Esc: back.

## Definitions

- **Active time:** the sum of gaps between a session's consecutive events, counting a gap only when it is 5 minutes or less. Longer gaps are waiting (on you, or on a subagent). Computed with one SQL query on each machine's own index (sab-air over ssh). Grok and Cursor events carry no timestamps in the index, so their active time shows as `–`.
- **Failed:** the index's own `failed` flag (the session ended on an error with nothing after it, or a finished-with-error sidecar), on the session or any of its subagents.
- **Live:** the index's `active` status (fresh hook receipt or recent writes), on the session or a subagent.
- **Summary:** `claude -p --model haiku` on the last 16 prompts, replies and errors (index previews), for the 30 most recent ended sessions, at most 30 calls per run, cached in `~/.grok/traces/board-summaries.json` and redone only when the session changed. Without one, the row shows the index's last event.

## Left out

- Sessions in the index's trivial filter (fewer than 3 events, no tools) and children beyond the board's roots query (500 local, 200 per remote host).
- sab-air grok and dpi sessions don't open in the session view (their files aren't a single transcript to copy); they show a status line.
- OMP, bb and Cursor rows open the old trajectory view.
- The session view's top line and status line are unchanged (harness · model · folder, alive or ended).

## Subagents in grok's Dashboard

Grok's Dashboard has no subagent rows. They were removed (`views/dashboard/state.rs`: a `sub:` key is "a pinned or reordered subagent row from before those rows were removed"). A parent shows a `Subagents N` chip in its title (`row_title.rs`, `RowBadge::Subagents`), and the subagents are reached inside the session: the Subagents pane under the scrollback and Enter on a subagent row. Here the board lists them under their parent with `e`, running first, and Enter opens one directly.
