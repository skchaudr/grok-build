# Objective: packet 7, inspector body cursor and range

You are writing the task packet (not the implementation) for the packet below. Facts as of 9f61d30d on main. Packet 3b (detail-pane focus and linewise copy) has landed in 92f80438, was used on the installed build, and copied exact lines, wrapped lines and ranges. Its implementation is in views/acpx_modal/detail.rs and is the pattern packet 7 reuses.

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

## Defects the operator hit on the installed build (sab-air, 2026-11-27 23:29), in his words
He opened /acpx, a past run, one step with Enter (the inspector), and copied output. The page looks good and works badly:
1. To read another step he has to press Esc, go down a step and press Enter again; he wants to open one step and use the arrow keys to reach the others.
2. Tab on a step moves between the step's output and its meta (inspector tabs). That is expected; it stays.
3. y copies the whole tab, one line or fifty. He wants the same linewise copy the grok build line viewer has.

## What the code shows about those defects
Step switching already exists as [ and ] (STEP_KEYS: Previous / next step in run), listed only in the ? overlay, and Left/Right are bound to horizontal scroll (also h/l). Up/Down and j/k scroll today and become the line cursor under the contract, so they cannot also change steps. Today y copies the whole active tab (test copy_returns_active_tab_full_text); after this packet Y does that and y copies the cursor line or visual range.

## Decision made for the writer (the coordinator's pick; the operator may overrule)
Left and Right switch to the previous and next step, like [ and ]; h and l keep horizontal scroll. The footer of the inspector shows the step keys and the cursor keys while it is open.
