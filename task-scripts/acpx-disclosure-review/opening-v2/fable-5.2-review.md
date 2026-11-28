# Opening-screen ledger delta: Fable 5.2 review

Packet: `opening-v2/_prompts/acpx-opening-ledger-delta.md` at `3dece007`. Read-only review against product source at that commit and the saved runs on sab-mini (28 native in `~/.acpx/flows/runs`, 47 mirrored in `~/.grok/cache/pi-workflows-mirror`). No worker, build or product edit.

## Verdict

**Do not dispatch this render yet.** Four spec changes are needed, all in the task text; no excerpt has to change. Item 1 needs Sab's decision first.

## Before dispatch

### 1. "Answered Waiting remains Waiting" only ever applies to native ACPX gates, and those never leave Waiting

- **Evidence:** a run counts as answered when another run's `input.gate.runId` names it (`acpx_modal.rs:720`, `ledger.rs:361`). A native gate is answered by starting a new run, so the answered run stays `waiting` on disk for good. A PIW gate is answered in place (`acpx_modal.rs:537`) and never enters that map. Saved runs today: 9 native waits were answered five to seven days ago and still read `waiting` (7 `ship`, 2 `fix`).
- **Consequence:** Waiting for input goes from 10 runs to 19 and never shrinks. The `ship` group reads 12 waiting when 5 need an answer. The header count (`ledger.rs:1355`) still counts unanswered only, so it says 10 beside a section of 19.
- **Repair:** Sab decides. Recommended: drop this sentence from the packet and keep `run_section` filing answered native waits under outcomes, as it does now. What an answered PIW wait looks like stays with packet 5. "Stalled Running stays Running" is unaffected.

### 2. A waiting group and a finished group of the same flow share one fold state

- **Evidence:** fold state is `flow_expanded_groups: HashSet<String>` keyed by `flow_name` (`ledger.rs:186`, `531`, `594`). `mixed-talk` has 2 waiting and 11 finished runs today, so both groups are on screen together. The spec says the groups are separate; it does not say their fold state is, and no listed test covers it.
- **Consequence:** Space on the waiting `mixed-talk` group also opens the 11 finished rows, or the reverse.
- **Repair:** one sentence (waiting and finished groups of one flow fold independently) and one test with a flow that has both.

### 3. No rule for a selected waiting-group header when the list changes

- **Evidence:** selection is `flow_selected_run_id` (`ledger.rs:184`); the header has no run ID. `reconcile_flow_selection` (`ledger.rs:782`) runs on every `set_runs` and `set_extras` and falls back to the first row when the selected ID is not found. The spec covers a selected member across refresh and collapsing onto the header, nothing else.
- **Consequence:** the worker invents it. The likely result is the cursor jumping to the first row on each rescan while the operator sits on a group.
- **Repair:** one sentence: a selected header stays selected by group key across refresh; when its group is no longer shown as a group (one member left, filter, Recent or Name order), select its newest member. Add it to the refresh test.

### 4. Runs past `EXTRAS_LIMIT` have no recorded path, so one workflow splits into two groups

- **Evidence:** `row_extra` runs for the 80 newest runs only; older runs get `answers` and defaults (`acpx_modal.rs:328–347`). The spec puts the path in `RowExtra` via `row_extra` and groups missing paths only with missing paths. There are 75 runs today, newest first, and the waits are 5 to 31 days old.
- **Consequence:** six more runs and the oldest waits lose their path and form a second group with the same label.
- **Repair:** say the path for runs past the limit is read once per run through the cache that already holds their answers (attached at 311–352; `bundle::flow_path` reads `manifest.json` only), and that this is not "widening the cache limit".

## Nonblocking

- **Header row content is unstated.** State the label (`flow_name`, or the newest member's title as the design doc's `ship: slugify` illustration shows), the `N waiting` count, the newest member's age, no rail and no question.
- **Between this packet and the preview packet,** selecting a group header makes `selected_run()` return `None`, `sync_preview` clears the preview and `acpx_modal.rs:838` draws the list full width. At 120 columns and wider the layout switches between split and full width as the cursor crosses a header, and the screen opens that way. Install after both packets land, or accept it.
- **Size.** Estimate only: the functions to replace whole come to about 1,100 production lines (`render_flow_row` 326, `handle_key` 171, `render_flows` 109, grouping and row building about 170, selection and navigation about 230), each quoted as OLD and again as NEW, plus about ten existing tests and nine new ones. Packet 2 returned 3,095 lines in 494 s with 70.9k output tokens; expect more. If the return is cut short or fails matching, the inline preview hook is the part to split off, since nothing calls it until the preview packet.
- **Same file, two recorded paths.** `mixed-demo` is recorded under both `/tmp/…` and `/private/tmp/…`. No waiting run has this today; if one does, it splits a group.

## Checked and fine

- All 18 excerpts equal the source at `3dece007`, line for line.
- Every section-label assertion and every `RowExtra` literal in `ledger.rs` is inside an attached excerpt. The two unattached ranges (2058–2366, sessions rendering; 2786–2985, five tests) have neither, and those tests build runs with unique flow names, so folding waits does not change them. `acpx_modal.rs:346` uses `..RowExtra::default()`.
- Grok 4.7's blockers 1 and 3 are fixed in this render; blocker 2 is correctly held with the preview packet.
- The five unanswered `ship` waits share one recorded flow path, so they fold into one group as the design doc's first case expects.

## Not checked

- The runner's timeout and output limit for this dispatch.
- Any behavior on screen; nothing was built.
