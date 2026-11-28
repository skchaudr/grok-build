# Opening-screen packet review

Pinned base: `73e819e2` (`docs/acpx-modal-opening-screen.md`), packets rendered at `8f7c5ca8`. Current `main` is `bae03973`, which only adds the run2 returns and integrator brief. No product patch.

## Dispatch verdicts

| Packet | Verdict |
|---|---|
| `finished-run-ledger` | **Do not dispatch this render.** The index already requires a rebase before another ledger dispatch. After that rebase, the remaining work is coherent. |
| `acpx-opening-preview` | **Do not dispatch until the ledger interface is in the excerpts it is told to consume.** Its behavior and file ownership are otherwise coherent. |

The earlier run2 ledger return is not, by itself, a blocker. `acpx-disclosure-packets.md` already says: review that return, and if it is accepted and landed, re-render only the remaining ledger changes and do not send both full versions. This render does not do that. It repeats the whole packet 2 assignment against pre-packet-2 excerpts.

## Delivery blockers

### 1. This ledger packet recodes the already-returned packet 2 work

- **Trigger:** dispatching `opening-v1/_prompts/finished-run-ledger.md` while `run2/.../finished-run-ledger.edits.md` is still the candidate to land (return committed at `bae03973`; not yet applied to product code).
- **Evidence:** The opening spec says "Carries forward packet 2" and then restates finished grouping, `RowExtra.executed`, rail/count/path, narrow wrapped path, `/open <run_id>` copy, and Space. Those are the same edits as run2 Edits 1–15, whose OLD blocks quote the same pre-change anchors (`fn row_extra` without `executed`, `RowExtra` without `executed`, `NEEDS YOU`, `5/262`). The working tree already contains that return (`ledger.rs` `build_flow_entries`, `executed`, `"N steps ran"`, and the `5/262` assertion removed). Applying both full versions cannot match OLD blocks twice.
- **Consequence:** Either the second dispatch rewrites accepted grouping, rail, count, and path, or the coordinator stops on zero/several OLD matches. The agreed preservation of that contract is not what a second full rewrite produces.
- **Repair before dispatch:** Review and, if accepted, land the run2 return. Re-render only the opening delta (waiting groups, section labels, shared interface, inline hook) against that base. Do not send this full ledger packet.

### 2. The preview packet cannot see the interface it must call

- **Trigger:** dispatching `acpx-opening-preview` against these excerpts, including after packet 2 alone lands. Packet 2's return does not add `selected_waiting_group`, `selected_run_is_finished_group`, or `set_inline_preview`.
- **Evidence:** The preview spec says "Do not edit ledger.rs" and "the ledger supplies the shared interface below," then requires calls to those three items. Its ledger excerpts stop at `LedgerOutcome`/`RowExtra` (lines 18–56), `selected_run`/`shortcuts` (201–309), `handle_key` (727–891), and `render` (943–980). None of those methods or `InlinePreviewLine` appear. `sync_preview` (excerpt 354–396) drops the preview whenever `selected_run()` is `None`, which is exactly the waiting-group case the spec must preview.
- **Consequence:** A worker that follows the packet cannot name a real method or match an OLD block for it. Wide group preview, "newest run" labeling, and the narrow inline hook then cannot compile against the ledger. Tab/line/range copy in both widths depends on that hook.
- **Repair before dispatch:** Put the three method signatures and `InlinePreviewLine` in the preview packet's ledger excerpts (read-only is enough). Keep the definitions owned by the rebased ledger packet.

### 3. The inline hook has no editable continuation anchor in the ledger packet

- **Trigger:** implementing "None preserves existing continuations; Some, including Some(empty), replaces question/path continuations" from the attached ledger excerpts.
- **Evidence:** `continuation_lines_for` in the attached 1–528 excerpt only emits wrapped question lines for a selected unanswered wait. The executed-path continuation is specified in prose and is not in any attached function; the current tree's path continuation is the unlanded run2 edit. `FlowListRow::Continuation` has no cursor or selected field. `render_flows` (943–1264) paints every continuation in warning/bold and does not scroll a focused wrapped line. Ledger lines 1697–2005, which contain the rest of `render_flow_row`, are outside every excerpt, so cursor painting there cannot be an OLD block.
- **Consequence:** The narrow preview cannot show one copy of the selected path plus a focused, wrappable cursor line. The beta8-merge case (Tab, exact line copy, Esc back to the same row, repeat at narrow width) has no place to land.
- **Repair before dispatch:** On the rebased ledger render, attach the inline-preview replacement to the post-packet-2 continuation and row renderer, including cursor/range display and scroll-into-view.

## Nonblocking

- Section labels: the approved doc says "Waiting for input", "Running", and "Recent outcomes"; attached `SectionKind::label` still returns "NEEDS YOU", "RUNNING", and "FINISHED". The spec does say to update those assertions. Not a dispatch blocker once the rebase excerpts include `SectionKind`.
- List `y` on a concrete run becomes `/open <run_id>`; the doc says the list keeps its existing copy. The index explicitly accepts the command copy. Preview `y` stays exact evidence text. No conflict with gate authority.
- `Tab`/`Shift+Tab` move list/preview focus in the modal, while the ledger packet keeps `Tab` as the tab switch. That split matches the approved doc and the preview spec. The ledger shortcut string can stay "Tab switch" only if the modal consumes Tab before `ledger.handle_key`; the preview packet owns that function.
- Space help: ledger adds the shortcut label; the index assigns the `LEDGER_KEYS` row to the preview packet, which includes `LEDGER_KEYS`. Consistent.
- `row_extra` / `load_extras` ownership is disjoint from the preview's input and render anchors, provided the ledger packet does not also edit `handle_key` or `render_acpx_modal` in `acpx_modal.rs`. It currently does not.
- Waiting groups keyed by `flow_name` plus source root plus absolute `bundle::flow_path`, with missing paths grouped only among themselves, matches equal treatment and project separation. `flow_path` is attached and does not require the file to exist. Age and build interest are explicitly excluded.
- Finished-group Enter keeps the representative; waiting-group Enter expands and does not open. That matches the doc. Run Esc returning to the same row needs no new ledger outcome; selection is already by run ID.
- Diagnostic copy can be built from the attached `Step.error`, `raw_text`, and `step_failure_reason` without a new fetch. Preferring one original line over `trim_failure_tail` is specified. `gate_choices` is attached for evidence-only display. No preview key is given an answer outcome.
- `effective_outcome` is attached to the ledger packet but not its `Step` fields; `row_extra` already has the loaded `Run`. Usable, not blocking.

## Unverified

- Whether the run2 edit list matches current product source exactly once. The working tree contains the resulting code, but this review did not run `check_edits.py`.
- Whether `latest_run_failure` keeps one complete diagnostic line or joins a trimmed tail. The preview spec says to prefer the original line; live clipboard paste is correctly left to the coordinator.
- Packet 3b's interaction with these `handle_key` / `render_acpx_modal` edits. The index already says to re-render 3b after the opening changes land. Not a defect in these two packets.
