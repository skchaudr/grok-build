# ACPX opening screen proposal

Draft for review. This proposal covers the Flows screen you see when opening `/acpx`. It does not redesign workflow cards in the main pager, the open-run stage view, or the inspector. Those have specific review and evaluation cases already.

## What the screen should answer

1. What work is running, and what is waiting on a decision?
2. What happened on the run I selected?
3. How do I get to the exact run or step when I need the full evidence?

Open directly into Flows. Make that screen useful before opening anything. Keep the full ledger on the same screen, with groups the operator can expand, rather than adding a separate landing page or another Enter before they can inspect a run. Keep Agent Turns as the other tab.

## First view

Use three sections in the default grouped order: **Waiting for input**, **Running**, and **Recent outcomes**. Omit empty sections. The labels describe what the workflow is doing without guessing what the operator cares about.

- **Waiting for input:** Give every recorded wait the same treatment. Group repeated waits by workflow, project, and source when known, and sort groups and their members by most recent update. Each gate remains individually selectable. A group never borrows a member's question or choices. Age is context; it never hides or downgrades a wait. Keep a distinct status for answers in flight or accepted while the engine still reports waiting. Packet 5 owns gate and answer state.
- **Running:** Keep each run individually selectable. Show the known current stage. A stalled run remains here with a stalled label.
- **Recent outcomes:** Use packet 2's newest-first finished grouping and preserve each historical run. Keep the latest outcome visible on the collapsed row. Show failures and cancellations in plain language. The dot rail, executed-attempt count, and named execution path retain the agreed width behavior in packet 2. They describe the selected or representative run, never the sum of all members.

The guidance to place a gate when a result is worth checking or a build depends on it is for workflow authors. The modal applies no priority tier or extra metadata requirement based on that guidance.

Illustration using the reported repeated `ship: slugify` and `mixed-talk` entries and the historical `beta8-merge` failure. The waiting counts are examples from the conversation. The right pane illustrates what to show when recorded evidence is available.

```text
 Flows  |  Agent Turns

 Waiting for input            │ beta8-merge · Failed
 ▸ ship: slugify    5 waiting  │ Failed at cargo_check
 ▸ mixed-talk      2 waiting  │ Compiler error: <saved diagnostic, if present>
                              │
 Recent outcomes              │ Executed path
 > beta8-merge     Failed     │ <recorded attempts in order>
 ▸ mixed-talk      8 finished │
                              │
 Space  Expand group          │ Tab  Focus preview
 /  Filter runs               │ Enter  Open run
```

The example has a failure selected. On a fresh open, select the first row in the current sort order. After inspecting a run or refreshing the list, keep the same selected run by ID while it remains available.

## What gets disclosed on selection and on open

- **Select a group:** Show its count and what it contains. For a finished group, the preview shows the newest run and labels it as such. For a waiting group, show that there are several separate gates and `Space` reveals them. Do not show one member's choices as group actions.
- **Select a run:** At wide widths, the right pane is a compact, read-only summary. Show its real status and one useful piece of evidence: the saved gate question and available choices, the active step, the failed attempt and a short diagnostic, or the result of a completed run. Use the recorded path and outcome rather than unrun nodes or raw JSON wrappers. If the summary has no diagnostic or question, say what is missing. The full output stays in the open run and inspector.
- **Narrow widths:** Put a brief status or question below the selected row, plus packet 2's selected-run path where applicable. Keep that inline preview focusable with the same keys as the side pane. Keep the workflow name and status readable, let the question wrap, and keep long output behind Enter.
- **Focus and copy:** In Flows, `Tab` and `Shift+Tab` move between the list and the selected preview, with a visible focus marker. Use `h`/`l` or Left/Right from the list to switch between Flows and Agent Turns. In the preview, reuse packet 3b's keys: `j`/`k` or arrows move the line cursor, `v` selects a range, `y` copies the line or range, and `Y` copies the preview. Copy original text without line breaks introduced by wrapping. `Esc` clears a range first and then returns focus to the same list row. The list keeps its existing copy behavior.
- **Open:** With the list focused, `Space` expands or folds a group; `Enter` on a waiting group expands it, and `Enter` on an individual run opens it. For a finished group, preserve packet 2's representative-run behavior. Keep `/` filter and existing sort options. Searching by run ID reveals a matching run through folded groups. `Esc` from an open run returns to the same row and expansion state. Previewing or opening a run never submits, starts, or cancels anything.

## Review against these cases

| Case | What should be observable |
| --- | --- |
| Five `ship: slugify` waits and two `mixed-talk` waits | The default view shows two group rows. Expand `ship: slugify`; each member is separately selectable and shows its own question. A five-day-old wait is still reachable. A known build dependency does not change where its gate appears. |
| `beta8-merge` failure | Select the run, Tab into its preview, and copy an exact saved diagnostic line when available. Esc returns focus to the same row; Enter opens the full evidence. Repeat the copy from the inline preview at narrow width. |
| Completed run with retries | Keep the rail, count, and named path; retries occur in recorded order. Narrow width keeps the selected run's full path reachable without widening the screen. |
| Waiting run updates after an answer | The UI distinguishes the answer status from the engine state. The row moves to outcomes only when the engine reports a terminal state. No input is sent by opening the row. |
| Empty list or no matches | Say which case applies. The other tab and clearing a filter stay reachable. |

## Boundary with the current packets

This proposal replaces packet 4's extra-entry-screen approach if accepted. Packet 2's finished grouping and rail/count/path contract stay intact. This proposal changes how repeated waits appear by default: multiple waits share a folded row, and each remains individually selectable after expansion. The compact preview is also new work. Packet 5 still governs permissions and engine state. The main pager, modal stage view, and inspector need their already identified walkthroughs, not a second blanket design pass. Review whether this arrangement solves the clutter and makes the next action clear. Code, slice boundaries, and dispatch are outside this draft.
