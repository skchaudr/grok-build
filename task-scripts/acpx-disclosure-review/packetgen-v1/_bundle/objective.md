# Objective: packet 7, inspector body cursor and range

You are writing the task packet (not the implementation) for the packet below. Facts as of 8da326fe on main. Packet 3b (detail-pane focus and linewise copy) has landed in 92f80438, was used on the installed build, and copied exact lines, wrapped lines and ranges. Its implementation is in views/acpx_modal/detail.rs and is the pattern packet 7 reuses.

## The packet as the operator wrote it
### 7. Inspector body

The inspector body gets the same cursor and range. Its Tab and Shift+Tab keep switching inspector tabs; `Y` copies the whole tab, which is what `y` does today, and `y` copies the cursor line or range.

## The shared key contract (verbatim, from the focus and copy track)
From the friction logs of 11-26 and 11-27: the modal shows evidence that can't be reached, selected or copied. Three packets share one key contract, taken from the pager's line viewer, so every read-only pane behaves the same:

- Tab and Shift+Tab move focus between the panes of the current view, both directions; the focused pane is marked the way existing views mark focus.
- In a focused text pane: `j`/`k` and Up/Down move a line cursor and keep it visible; `v` starts or ends a visual line range; `y` copies the cursor line or the range; `Y` copies the whole pane.
- Copy is plain text: no styles, no indentation or breaks added by wrapping; a wrapped source line copies whole. The notice reads as a normal copy message.
- Esc first ends visual mode, then returns focus to where the operator came from, then leaves the view, landing on the same row or stage they left.
- Keys on a list or tree keep their current meaning when it has focus.

## Coordinator integration item (verbatim)
- **Disjoint edits in `acpx_modal.rs`.** Packet 2 edits `row_extra` and its tests. Packet 4 edits modal preview/input/render and `LEDGER_KEYS`, including the group Space help. Packet 3b owns Run-mode focus and `RUN_KEYS`; refresh its excerpts after the opening changes land. Match each OLD block by content and check it does not overlap another packet's.

## Facts the operator has not stated but the code shows
The inspector has no Tab-to-pane focus. Tab already switches inspector tabs and stays that way. Today y copies the whole active tab (test copy_returns_active_tab_full_text); after this packet Y does that and y copies the cursor line or visual range.
