# ACPX disclosure: packets for review

This document indexes the five larger changes DeepSeek proposed after Handoff 004, plus the items from `friction-log/11.26.grok.acpx.md`. The worker input definitions, source excerpt references, and artifact contracts are in `acpx-disclosure-slices.py`. The exact rendered prompts for the first review are preserved in `task-scripts/acpx-disclosure-review/_packets/`.

Keep review and run evidence in a named, persistent project directory. Never use `/private/tmp` or the macOS per-user temp directory for rendered prompts, reviews, raw events, receipts, or staged returns. For another dry run, use `slices acp task-scripts/acpx-disclosure-slices.py --dry-run --output <new-repository-local-directory>`. Use a fresh directory to keep earlier review snapshots intact. Before any live dispatch, also set the runner's scratch location to a repository-local directory: `--output` governs artifacts but does not control the empty worker scratch directory.

Code packets (1–4 and 6) return exact old/new edit lists, not diffs: each edit quotes an existing block verbatim and gives its complete replacement. The coordinator matches every OLD block exactly once on the pinned base, stops on zero or several matches, applies the edits, and lets Git produce the diff. Packet 5 returns a design contract. Packet 1 is the first pilot. Packet 7 has a contract but no rendered packet: it waits until packets 1 and 3 land. The reviewed opening-screen design is in `docs/acpx-modal-opening-screen.md`. The exact current opening-screen packets (2 and 4) are in `acpx-disclosure-review/opening-v1/_prompts/`, rendered on `73e819e2`. Earlier ledger and landing prompts remain historical evidence.

Every rendered packet receives independent packet review; returned code is reviewed separately before integration. This index does not authorize dispatch, integration, PIW changes, or release.

## 1. Executed timeline

Runner packet `executed-timeline`. Owns the stage pane in `acpx_modal/run.rs`. Fold finished attempts by default and preserve user expansion, ordered loop attempts, pending choices and the existing key map. An answered gate's stage keeps its expanded display. A live repeated attempt (build → repair → build with the second build running) reads as its own stage. Derive skipped nodes from the same pruned flow as the graph. Accept when `cargo_check → repair_compile → cargo_check` remains visible in order, evidence expands by key, and an untaken branch is absent from Skipped.

## 2. Flows ledger and grouping

Runner packet `finished-run-ledger` retains its ID and carries forward packet 2. It owns `acpx_modal/ledger.rs` and the `row_extra` producer and associated tests in `acpx_modal.rs`. The earlier ledger packet was already dispatched in `run2`. Review that return before another ledger dispatch. If accepted and landed, re-render only the remaining ledger changes against that base; do not apply or send both full ledger versions.

Keep the rail, real-attempt count and named path, including retries and the narrow selected-run path. In the default Flows order, group finished runs under the latest member and group repeated waiting runs separately. Every wait is reachable, including old ones; age, build dependency and operator interest are not display priority rules. Waiting and stalled running runs stay in their actual sections. Preserve selection by run ID, filtering, sort controls and concrete-run copy. A waiting group has no open or copy target until an individual member is selected. Packet 2 owns the selected-group and inline rendering interface used by packet 4.

## 3. Evidence-first detail

Runner packet `attempt-detail` owns the selected-attempt pane in `acpx_modal/detail.rs` and edits no other file. It reuses current evidence helpers, keeps pending choices available and directs operators to the existing inspector, expanded timeline or replay for full evidence. The digest goes first; existing conversation, artifact and event sections may follow, and no function is deleted or renamed. Detail-pane focus and copy are packet 3b, built on this one. Check a nonzero `cargo_check` against `beta8-merge` and a finished agent attempt with recorded review evidence.

## 4. Opening-screen preview and copy

Runner packet `acpx-opening-preview` replaces the older `acpx-landing` design assignment. The modal still opens directly to Flows. This packet owns compact preview evidence, focus and copy, and the Ledger-mode wiring in `acpx_modal.rs`; it does not edit the ledger or the open-run detail panel.

Show an outcome, diagnostic, current step, or recorded gate question and choices as appropriate. Use the same logical preview lines in the wide side pane and narrow inline preview. Tab and Shift+Tab move focus. Reuse the packet 3b linewise copy contract, including exact copy across wraps and Esc returning to the same row. Opening and previewing never perform workflow actions. Packets 2 and 4 have disjoint edit regions and are reviewed and tested together; 4 needs the interface returned by 2 to compile. Render 3b after the opening-screen changes land, because both touch the modal dispatcher and renderer.

## 5. Gate state and answers across the three surfaces

Runner packet `cross-surface-gate`.

**Owns:** The state and action contract for opening, inspecting and answering an existing PIW run in the Grok `/acpx` modal, standalone PIW and Grok ACPX chat. The PIW engine is authoritative for the pending request and run state. Start from `crates/codegen/xai-grok-pager/src/piw/gate.rs`, the modal's run/detail/ledger views, the chat agent's PIW answer and replay paths, and the standalone PIW client's run controls. Read the standalone checkout for coordination; changes there require separate scope approval.

**Behavior:** Inspecting or typing bare `/answer` reveals the gate when applicable and never submits, starts, or cancels work. Keep a submitted request, server acceptance and the engine's next run state distinct. A run still reported as waiting must never display as completed merely because the client sent an answer. A mirrored historical event cannot supply live authority; opening a currently parked PIW run in chat must not turn its answer into a native ACPX continuation. Ask for text or checkpoint data only when the actual gate requires it. Respect protected decisions and each surface's advertised permissions.

**Return and review:** First return a state transition table and a runnable walkthrough for one newly parked PIW fixture. The fixture's gate guards a build or test step, or a result worth checking before going on; Sab: gates matter when the result is uncertain or a build is attached, and a gate guarding neither can be ignored. Start by tracing current behavior, including which surfaces can submit, before proposing implementation. Chat submits through `/answer <label>` and through plain text while a run is occupied; the matrix covers both and keeps what the standalone client does separate from what its protocol lists. Inspect the same run from all three surfaces, compare its state before and after bare `/answer`, send exactly one explicit allowed answer from one supported surface, and verify that each surface displays the engine's accepted branch through the next step. Include a rejected or stale submission case and explain how the UI lets the operator recover without resending an uncertain effect. Review the evidence and ownership split before editing the cross-surface implementation. The earlier cancellation report is untraced; the packet must not assume its cause.

## Before packet 6: mirrored `/runs` and read-only `/open` (one owner)

Packet 6's pick lists must use the same run discovery and open behavior as `/runs` and `/open <id>`, so this lands first, with one owner, and packet 6 is re-rendered from that commit.

- `/runs` lists mirrored PIW runs beside native ACPX runs, each marked with its source.
- `/open <id>` on a native ACPX run keeps restoring its waiting gate (`replay_run_into_session`, `sess.waiting_gate`), as Handoff 004 describes.
- `/open <id>` on a mirrored PIW run is read-only: it replays the run and shows a waiting gate's question and choices, but sets no `waiting_gate`, so typing a choice label cannot answer a PIW gate as a native ACPX continuation. Live PIW answers belong to packet 5.
- Tests: a native waiting run reopened by `/open` still accepts its gate answer; a mirrored waiting run reopened by `/open` shows the question, and a typed choice label answers nothing; `/runs` lists a mirrored run.

The label-only edits land directly, outside the packets: the copy notice uses `delivery.toast_message()` (`app/dispatch/router.rs:1817`), and the Enter help row says "replay agent turn" (`views/acpx_modal.rs:860`).

## 6. Bare `/open` and `/flow` pick lists in ACPX chat

Runner packet `bare-command-picker`. Owns `grok-acpx-acp/agent.rs` and `driver.rs`; no other code packet edits either file (packet 5 only reads `agent.rs`). From the friction log: commands that take arguments are good, a list to choose from when no argument is given is better.

**Behavior:** A bare `/open` shows the existing question pop-up (`x.ai/ask_user_question`, as gates use) listing the 20 newest runs with flow name, title, status and start time; choosing one does exactly what `/open <run_id>` does. A bare `/flow` lists the default flow and the flow files of recent runs, with each one's last status and start time; choosing one asks for the task text, and only a nonempty task starts the run. Dismissing either question starts or opens nothing. Bare `/answer` stays as it is until packet 5's contract settles gate answers.

## Focus and copy track (3b, 7, 8)

From the friction logs of 11-26 and 11-27: the modal shows evidence that can't be reached, selected or copied. Three packets share one key contract, taken from the pager's line viewer, so every read-only pane behaves the same:

- Tab and Shift+Tab move focus between the panes of the current view, both directions; the focused pane is marked the way existing views mark focus.
- In a focused text pane: `j`/`k` and Up/Down move a line cursor and keep it visible; `v` starts or ends a visual line range; `y` copies the cursor line or the range; `Y` copies the whole pane.
- Copy is plain text: no styles, no indentation or breaks added by wrapping; a wrapped source line copies whole. The notice reads as a normal copy message.
- Esc first ends visual mode, then returns focus to where the operator came from, then leaves the view, landing on the same row or stage they left.
- Keys on a list or tree keep their current meaning when it has focus.

Each packet is rendered only after the code it edits has landed, with fresh excerpts and this contract pasted in verbatim. Packet 3b goes first; 7 and 8 reuse its implementation once it has been used and checked.

### 3b. Detail pane focus and copy

Edits `acpx_modal/detail.rs` (cursor and range in `DetailPanel`) and, in `acpx_modal.rs`, only the Run-mode routing: `handle_key`, `handle_run_key`, `wants_esc`, the Run render call and `RUN_KEYS`. It does not touch `run.rs`. Tab and Shift+Tab move between the stage tree and the detail pane; with the tree focused, `v` keeps the noise toggle, `y` keeps copying the stage output and `J`/`K`/`D`/`U` keep scrolling the detail pane. Waits for packet 3. Acceptance: Tab into the detail pane of a failed run, select the compiler-error line, copy it, and return to the same stage.

### 7. Inspector body

The inspector body gets the same cursor and range. Its Tab and Shift+Tab keep switching inspector tabs; `Y` copies the whole tab, which is what `y` does today, and `y` copies the cursor line or range.

### 8. Agent Turns replay view

Enter on an Agent Turns row opens the saved record's stream in the pager's read-only replay view (`app/dispatch/standalone_traces.rs` `open_acpx`, the same view used for Claude and grok trace replays). A record from a one-shot worker holds one prompt and one reply. Acceptance from the 11-27 friction log: open a row, reach the prompt and the reply, copy an exact line or range from either, and press Esc to return to the same list row. Reuse the pager's own controls in that view rather than adding new ones. First step, before the packet is written: drive the installed build in a Rex tab and record which pager keys already work in this view and where routing stops; the packet fixes only what that trace shows broken.

## Coordinator integration items

- **Copy notice.** The fix landed in `c8662887`. Keep the confirmed `Copied!` message and the warning on fallback.
- **Clipboard delivery, live.** A returned `AcpxOutcome::Copy` with the right text does not prove the text reaches the clipboard. On sab-air in the installed build, paste a failed run's exact diagnostic line copied from the wide and narrow opening preview, and later repeat from the open-run detail pane. Verify the Agent Turns record ID and packet 2's `/open <id>` copy separately before shipping those behaviors.
- **Disjoint edits in `acpx_modal.rs`.** Packet 2 edits `row_extra` and its tests. Packet 4 edits modal preview/input/render and `LEDGER_KEYS`, including the group Space help. Packet 3b owns Run-mode focus and `RUN_KEYS`; refresh its excerpts after the opening changes land. Match each OLD block by content and check it does not overlap another packet's.

