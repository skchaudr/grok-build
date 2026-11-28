# ACPX opening screen proposal

Draft for review. This proposal covers the Flows screen you see when opening `/acpx`. It does not redesign workflow cards in the main pager, the open-run stage view, or the inspector. Those have specific review and evaluation cases already.

## What the screen should answer

1. What work is running, and what is waiting on a decision?
2. What happened on the run I selected?
3. How do I get to the exact run or step when I need the full evidence?

Open directly into Flows. Make that screen useful before opening anything. Keep the full ledger on the same screen, with groups the operator can expand, rather than adding a separate landing page or another Enter before they can inspect a run. Keep Agent Turns as the other tab.

## First view

Use three sections in the default grouped order: **Waiting for input**, **Running**, and **Recent outcomes**. Omit empty sections. The labels describe what the workflow is doing without guessing what the operator cares about.

- **Waiting for input:** One expandable row for multiple waits from the same workflow, scoped to the project and source when known. A group says how many are waiting; it does not copy one run's question onto the group as if they all share it. Sort groups and their members by most recent update. Age can be shown as context; it never hides, downgrades, or cancels a wait. Show that a run blocks a build only when the evidence says so. An answer in flight or accepted while the engine still reports waiting stays here with distinct status text. A single waiting run stays a direct row. Packet 5 owns how the UI learns gate and answer state.
- **Running:** Keep each run individually selectable. Show the known current stage. A stalled run remains here with a stalled label.
- **Recent outcomes:** Use packet 2's newest-first finished grouping and preserve each historical run. Keep the latest outcome visible on the collapsed row. Show failures and cancellations in plain language. The dot rail, executed-attempt count, and named execution path retain the agreed width behavior in packet 2. They describe the selected or representative run, never the sum of all members.

Illustration using the reported repeated `ship: slugify` and `mixed-talk` entries and the historical `beta8-merge` failure. Counts shown for the waiting groups are the examples reported in the conversation, not a claim about the current run store. The right pane illustrates the kind of evidence to show; it does not invent compiler output or a gate answer.

```text
 Flows  |  Agent Turns

 Waiting for input            │ beta8-merge · Failed
 ▸ ship: slugify    5 waiting  │ Failed at cargo_check
 ▸ mixed-talk      2 waiting  │ Compiler error: <saved diagnostic, if present>
                              │
 Recent outcomes              │ Executed path
 > beta8-merge     Failed     │ <recorded attempts in order>
 ▸ mixed-talk      8 finished │
                              │ Enter  Open run
 Space  Expand group          │
 /  Filter runs               │
```

The example has a failure selected. On a fresh open, select the first row in the current sort order. After inspecting a run or refreshing the list, keep the same selected run by ID while it remains available.

## What gets disclosed on selection and on open

- **Select a group:** Show its count and what it contains. For a finished group, the preview shows the newest run and labels it as such. For a waiting group, show that there are several separate gates and `Space` reveals them. Do not show one member's choices as group actions.
- **Select a run:** At wide widths, the right pane is a compact, read-only summary. Show its real status and one useful piece of evidence: the saved gate question and available choices, the active step, the failed attempt and a short diagnostic, or the result of a completed run. Use the recorded path and outcome rather than unrun nodes or raw JSON wrappers. If the summary has no diagnostic or question, say what is missing. The full output stays in the open run and inspector.
- **Narrow widths:** Drop the side pane. Put a brief status or question below the selected row, plus packet 2's selected-run path where applicable. Keep the workflow name and status readable, and let the selected question wrap. Keep long output behind Enter. A narrow layout requires no additional mode or key.
- **Open:** `Space` expands or folds a group; `Enter` on a waiting group expands it, and `Enter` on an individual run opens it. For a finished group, preserve packet 2's representative-run behavior. Keep `/` filter, existing sort options, and tab navigation. Searching for an older run by ID exposes that run even when its group is folded. `Esc` from an open run returns to the same row and expansion state. Previewing or opening a run never submits, starts, or cancels anything.

## Review against these cases

| Case | What should be observable |
| --- | --- |
| Five `ship: slugify` waits and two `mixed-talk` waits | Two group rows in the default view. Expand `ship: slugify`; every member remains separately selectable and shows only its own question. An older wait still appears. |
| `beta8-merge` failure | Selecting the run shows the failed attempt and the saved diagnostic when available. Enter reaches the full evidence. Esc returns to that run. |
| Completed run with retries | Keep the rail, count, and named path; retries occur in recorded order. Narrow width keeps the selected run's full path reachable without widening the screen. |
| Waiting run updates after an answer | The UI distinguishes the answer status from the engine state. The row moves to outcomes only when the engine reports a terminal state. No input is sent by opening the row. |
| Empty list or no matches | Say which case applies. The other tab and clearing a filter stay reachable. |

## Boundary with the current packets

This proposal replaces packet 4's extra-entry-screen approach if accepted. Packet 2's finished grouping and rail/count/path contract stay intact. This proposal changes how repeated waits appear by default: multiple waits share a folded row, and each remains individually selectable after expansion. The compact preview is also new work. Packet 5 still governs permissions and engine state. The main pager, modal stage view, and inspector need their already identified walkthroughs, not a second blanket design pass. Review whether this arrangement solves the clutter and makes the next action clear. Code, slice boundaries, and dispatch are outside this draft.
