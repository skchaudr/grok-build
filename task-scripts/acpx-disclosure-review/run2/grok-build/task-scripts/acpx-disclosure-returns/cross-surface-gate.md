# Cross-Surface PIW Gate Contract and Operator Walkthrough

- **Slice ID:** `cross-surface-gate`
- **Status:** Design contract and operator walkthrough only (no patch in this packet)
- **Review Gates:** Cross-repo ownership (`grok-build` vs `pi-workflows`) and live fixture verification
- **Authority Rule:** The `pi-workflows` engine's pending interaction (`requestId`, `revision`, `status`, `kind`, `targetSessionId`) and run state (`status`, `waitingOn`, `currentNode`, `controls`, `human_decision`) are the sole source of truth across all surfaces.
- **Safety Boundaries:** Never answer a real protected decision or launch a production workflow during verification. This contract records only observed cancel and withdrawal paths from the supplied source excerpts and makes no speculative claims about the root cause of earlier cancellation reports.

---

## 1. Authoritative Engine State, Gate Types, and Protocol Mechanics

### 1.1 How a Gate Is Recognized and Bound

In `grok-build` (`crates/codegen/xai-grok-pager/src/piw/gate.rs`), `gate_from(view, session)` derives a `PiwGate` only when the server session snapshot contains an `interaction` object with:

- `status == "pending"`
- `kind` equal to `"decision"` (`GateKind::Decision`) or `"checkpoint"` (`GateKind::Checkpoint`)
- non-empty `runId` matching `run_id_of_view(view)`
- non-empty `requestId` and numeric `revision` (`u64`)
- `targetSessionId` (or fallback `sessionId`)

`session_of_view(view)` locates a run's session via `queue.originSessionId`, `session.binding.piSessionId`, or `originSessionId`. `coordinator_state(session_snapshot)` extracts the three coordinator authority fields: `coordinatorEpoch`, `coordinatorActive`, and `branchReportRequired`.

### 1.2 Checkpoint Input vs. Protected Decision vs. Native ACPX Gate

| Dimension | PIW Checkpoint (`GateKind::Checkpoint`) | PIW Decision (`GateKind::Decision`, incl. Protected Decisions) | Native ACPX Gate (`WaitingGate`) |
| --- | --- | --- | --- |
| **Wire Operation** | `"checkpoint.answer"` (`answer_request`) | `"decision.answer"` (`answer_request`) | None (local flow launch via `AcpxOutcome::RunFlow` or `run_flow_chain`) |
| **Payload Shape** | `{ targetSessionId, coordinatorEpoch, requestId, submissionId, input }` with `expectedRevision: gate.revision` | `{ targetSessionId, coordinatorEpoch, requestId, submissionId, response: { choice, input? } }` with `expectedRevision: gate.revision` | `bundle::gate_input(run, &choice)` passed to a new chained flow file |
| **Input Parsing (`parse_answer`)** | Trimmed text parsed as JSON `Value`, falling back to `Value::String` | 1-based number, choice `key`/`label` (case-insensitive), or unambiguous prefix; trailing text after first whitespace populates declared `text_input`; missing required `text_input` or unexpected text returns `Err` | Case-insensitive label match against `WaitingGate::choices` (`Command::Answer`, `ask_gate_question`) |
| **Choices in `PiwGate`** | `choices` is empty (`gate_from`) | `choices` extracted via `extract_decision_choices(contract, node_def)` with `key`, `label`, `text_input` | `GateChoice { label, flow_file }` pointing to a flow file on disk |
| **Server `controls` (`pi-workflows/tui/src/ui/run_command.rs`)** | `["pause", "cancel", "answer"]` (`toggle_op_for_each_server_controls_configuration`) | `["pause", "cancel", "human-answer"]` (`toggle_op_for_each_server_controls_configuration`) | N/A (local ACPX bundle) |
| **Coordinator / Live Pi Protection** | Owned by live Pi session when `!coord.active && coord.epoch.is_some()` -> refused (`REFUSED_LIVE_PI_SESSION`); standalone `piw` `no_control_notice` states checkpoint must be answered inside the owning Pi session | Same live Pi protection in `AnswerMachine::poll` (`REFUSED_LIVE_PI_SESSION`); `render.rs` `human_decision_detail` displays `human decision · <audience>` while waiting and `human: <label>` once answered | No PIW coordinator; local bundle state only |
| **Post-Answer Lifecycle** | Same `run_id` resumes on the engine | Same `run_id` resumes on the engine | Old `run_id` stays `Waiting` on disk; a new chained flow run starts; UI masks old run via `self.answered` (`render_header`, `build_run_lines`, `is_answered`, `run_section`) |

### 1.3 Submission, Refusal, Reconnection, and Uncertain-Submission Recovery

Both Grok `/acpx` (`GateAnswerer::poll`, non-blocking per UI tick) and Grok ACPX chat (`answer_piw_gate` calling `piw_gate::answer_blocking` -> `run_blocking_with` inside `tokio::task::spawn_blocking`) execute the same `AnswerMachine` state machine in `grok-build` (`crates/codegen/xai-grok-pager/src/piw/gate.rs`):

1. **Pre-flight Validation (`Phase::Init` -> `Phase::WaitSnapshot`):**
   - `AnswerMachine::poll` probes `answer_request(&self.gate, &self.answer, "probe-epoch", "probe-sub")` before sending any wire request. Invalid choice indices, missing `text_input`, or mismatched `GateKind`/`GateAnswer` fail locally in `Phase::Init` without claiming coordinator authority.
   - Records `seen_connection_count = client.connection_count()` and starts a passive session watch (`client.watch_session(&self.gate.session_id, false)`).

2. **Coordinator Authority Check (`Phase::WaitSnapshot` / `Phase::WaitCoordinator`):**
   - Waits up to `SNAPSHOT_TIMEOUT` (10s) for `client.session(&session_id)`.
   - If `coord.active == true` and `coord.epoch` is `Some(epoch)`: proceeds to `send_branch_report` (when `coord.branch_report_required == true`) or directly to `send_answer`.
   - If `coord.active == false` and `coord.epoch.is_some()`: immediately refuses with `REFUSED_LIVE_PI_SESSION` (`"This run belongs to a live Pi session; answer it there."`). This is the hard guard that prevents non-Pi surfaces from hijacking a live Pi session.
   - If `coord.active == false` and `coord.epoch.is_none()` (unowned session): sets `claimed_role = true`, calls `client.watch_session(&session_id, true)` to claim coordinator authority, and waits up to `SNAPSHOT_TIMEOUT` in `Phase::WaitCoordinator` until `coord.active && coord.epoch.is_some()`.

3. **Optional Branch Report (`AnswerMachine::send_branch_report` / `Phase::WaitBranchReport`):**
   - Sends `"workflowMessage.reportBranch"` with `{ targetSessionId, coordinatorEpoch, workflowMessageId: null, piSessionEntryId: null, isIdle: true, hasPendingMessages: false }`.
   - Waits up to `RESPONSE_TIMEOUT` (30s). On `resp.ok()`, transitions to `send_answer`.

4. **Keyed Submission (`AnswerMachine::send_answer` / `Phase::WaitAnswer`):**
   - Calls `answer_request(&self.gate, &self.answer, &epoch, &self.submission_id)`.
   - Sends via `client.send_keyed(&operation, Some(&self.gate.run_id), Some(expected_revision), payload, &self.answer_idempotency_key)`.
   - Waits up to `RESPONSE_TIMEOUT` (30s). On `resp.ok()`, calls `finish_accepted` -> `cleanup_if_claimed`.

5. **Safe Coordinator Release (`AnswerMachine::cleanup_if_claimed`):**
   - Invoked on `finish_accepted`, `finish_refused`, `abort`, and `GateAnswerer::drop`.
   - Calls `client.watch_session(&self.gate.session_id, false)` rather than `unwatch_session` so the shared session snapshot remains populated for subsequent gates while releasing the coordinator claim.

6. **Refusal Handling (`AnswerMachine::handle_answer_refusal`):**
   - Reads the latest `client.session(&self.gate.session_id)` `interaction` object:
     - Missing `interaction`, `requestId != gate.request_id`, or `status != "pending"` -> `finish_refused(REFUSED_ALREADY_ANSWERED)` (`"This question was already answered."`).
     - Same pending `requestId` with changed `revision` (`new_rev != gate.revision`) and `revision_retries < MAX_REVISION_RETRIES` (3) -> increments `revision_retries`, updates `gate.revision = new_rev`, rotates both `submission_id` and `answer_idempotency_key`, re-reads `coordinator_state`, and retries via `send_branch_report` (if required) or `send_answer`.
     - All other server errors -> `finish_refused(server_error_sentence(resp))`.

7. **Connection-Loss Handling (`AnswerMachine::restart_after_connection_loss`):**
   - Triggered when `client.connection_count()` changes unexpectedly or a response arrives with `resp.outcome == CONNECTION_LOST`.
   - Capped at `MAX_CONNECTION_RESTARTS` (3); exceeding the cap refuses with `"The connection to the workflow server keeps dropping; the answer could not be delivered."`.
   - Re-issues passive `watch_session(session_id, false)` and returns to `Phase::WaitSnapshot`, **preserving the original `submission_id` and `answer_idempotency_key`** so transport retries do not create a second logical submission.
   - If `client.state() == ConnState::Closed`, refuses immediately with `"The workflow server connection was closed."`.

8. **Recovery After an Uncertain Submission (Reconcile Before Any Resend):**
   - An uncertain outcome occurs after `RESPONSE_TIMEOUT` (30s), `GateAnswerer` drop/cancel mid-flight, `answer_blocking` 60s timeout (`agent.rs`), `ConnState::Closed`, or `MAX_CONNECTION_RESTARTS` exhaustion.
   - **Rule:** Never immediately issue a fresh manual submission (which allocates a new `submission_id` and `answer_idempotency_key`). First reconcile against the engine's authoritative state:
     1. Fetch or observe the live session snapshot (`interaction`) and run view (`status`, `waitingOn`, `currentNode`, `human_decision`).
     2. Compare `interaction.requestId`, `interaction.status`, and `interaction.revision` against the originally targeted gate:
        - If `interaction` is absent, `status != "pending"`, `requestId` changed, or `human_decision` already records the node's choice: the earlier submission (or another surface) already landed. Do **not** resend; let all surfaces converge on the next live state.
        - If the same `requestId` is still `"pending"` (with the same or incremented `revision`): verify `coordinator_state` (`active` / `epoch`) to ensure no live Pi session took ownership, then decide explicitly whether to submit once against the current authoritative `revision`.
   - **Verification Item `V-8`:** Server-side deduplication semantics for `submissionId` and `answer_idempotency_key` live in `pi-workflows` server code outside the supplied excerpts and must be verified before relying on cross-reconnect deduplication across server restarts.

---

## 2. Per-Surface Read/Submit Matrix

### 2.1 Protocol Inventory vs. Client Capabilities

`pi-workflows/tui/src/protocol.rs` (`PROTOCOL_ID = "pi-workflows.client.v1"`) lists the following gate- and view-related operations in `OPERATIONS`: `"run.start"`, `"run.pause"`, `"run.resume"`, `"run.cancel"`, `"run.status"`, `"run.list"`, `"checkpoint.answer"`, `"decision.answer"`, `"interaction.submit"`, `"interaction.update"`, `"view.runs.watch"`, `"view.runs.page"`, `"view.run.get"`, `"view.run.watch"`, `"view.run.unwatch"`.

Listing an operation in `protocol.rs` defines the wire vocabulary only. Each surface's actual read and submit capabilities depend strictly on its implemented handlers below.

### 2.2 Per-Surface Matrix (Actual Client Handlers and Requests)

| Surface / Path | Repo & Source Functions | Read / Inspect Path | Live Watch / Refresh | Submit Path & Wire Request | Cancel / Flow Launch Separation | Limitations & Verification Items |
| --- | --- | --- | --- | --- | --- | --- |
| **Grok `/acpx` Modal (PIW mirrored run)** | `grok-build`: `acpx_modal.rs` (`watch_open_mirror_run`, `release_open_mirror_run`, `handle_run_key`, `answer_mirror_gate`, `poll_answerers`, `cancel_open_mirror_run`); `run.rs` (`render_header`); `detail.rs` (`build_run_lines`); `ledger.rs` (`is_answered`, `run_section`); `piw/gate.rs` (`parse_answer_target`, `GateAnswerer`) | Opens mirrored run folder; `watch_open_mirror_run` registers `m.want_run(&run_id)`; choices carry `piw-answer://<run_id>/<choice_key>` (`ANSWER_SCHEME`, `write_piw_gate_fixture`). Ledger places waiting run in `SectionKind::NeedsYou` (`run_section`). | Calls `m.want_run(&run_id)` on open and `m.release_run(&run_id)` on close/back (`V-6` for live projection reload into `RunView`). | Selecting a choice + `Enter` emits `RunOutcome::Gate { choice, flow_file }`. `handle_run_key` checks `parse_answer_target(&flow_file)` first and calls `answer_mirror_gate`, which looks up `m.gate(run_id)`, builds `GateAnswer::Choice { index, text: None }`, and pushes `GateAnswerer::new(...)`. `poll_answerers` drives `AnswerMachine` (`decision.answer` + optional `workflowMessage.reportBranch`). | Strictly separated from native flow launch (`answer_target_is_parsed_and_routed_not_launched` verifies `AcpxOutcome::RunFlow` is never emitted). `cancel_open_mirror_run` sends `"run.cancel"` only on explicit `/acpx cancel`. | `answer_mirror_gate` hardcodes `text: None` and matches `gate.choices` by `key`; it handles plain decision choices only, not decision choices requiring `text_input` or `GateKind::Checkpoint` freeform input (`V-6`, `V-9`). |
| **Grok `/acpx` Modal (Native ACPX run)** | `grok-build`: `acpx_modal.rs` (`handle_run_key`), `run.rs` (`render_header`), `detail.rs` (`build_run_lines`), `ledger.rs` (`is_answered`, `run_section`) | Reads local bundle projections (`live.json`, `flow.json`, `steps.json`). | Local disk rescan on back (`self.rescan_runs()`). | `RunOutcome::Gate` when `parse_answer_target` returns `None`: resolves relative `flow_file` beside `bundle::flow_path`, builds `bundle::gate_input`, returns `AcpxOutcome::RunFlow { flow_file, input_json }` and returns to `Mode::Ledger`. | Launches a **new** chained flow; never sends PIW wire operations. | `answered` map causes waiting native run to display as `Completed` / `Finished` (`render_header`, `build_run_lines`, `is_answered`). |
| **Standalone PIW TUI (`piw`)** | `pi-workflows`: `tui/src/render.rs` (`card_shape`, `bounded_branch_lines`, `human_decision_detail`); `tui/src/ui/run_command.rs` (`RunOp`, `toggle_op`, `no_control_notice`, `outcome_notice`); `tui/src/protocol.rs` (`OPERATIONS`) | Renders node cards with `human decision · <audience>` while `state.waiting_on == Some(node_id)`, up to `CARD_MAX_BRANCH_ROWS` branch lines (`◇ <label>`), and `human: <label>` once `state.human_decision` is recorded (`card_shape`, `human_decision_detail`). | Protocol includes `"view.run.watch"` and `"view.runs.watch"`; `render.rs` consumes live `GraphView` / `RunState` snapshots (`V-2` for event loop wiring). | **No gate submission handler is established by the excerpts.** `RunOp` contains only `Pause` (`"run.pause"`), `Resume` (`"run.resume"`), and `Cancel` (`"run.cancel"`). `no_control_notice` explicitly states checkpoints must be answered inside the owning Pi session, not from `piw`. Presence of `"checkpoint.answer"` / `"decision.answer"` in `protocol.rs` does not imply standalone TUI submission support (`V-1`). | Lifecycle controls (`RunOp::Pause`, `Resume`, `Cancel`) are gated by server `controls` via `toggle_op`; no native ACPX flow launch exists. | `V-1`: verify outside excerpts whether any other `pi-workflows/tui/src` module binds `"human-answer"` to `"decision.answer"`. `V-2`: verify live watch auto-refresh loop. |
| **Grok ACPX Chat — Live PIW Run (Inline Card & Question Path)** | `grok-build`: `grok-acpx-acp/agent.rs` (`pump_piw_events`, `remember_inline_waiting`, `store_waiting_piw`, `ask_piw_gate_question`, `ask_gate_question`, `piw_gate_withdrawn`) | `pump_piw_events` projects `DriverEvent::Waiting` via `Projector::on_event` into an inline card in the transcript and parks immediately (Issue 11 comment: does not auto-open question modal), storing `sess.waiting_piw`. `ask_piw_gate_question` (when invoked) reveals choices via `x.ai/ask_user_question` (`noFreeform: !choices.is_empty() && !needs_text`). | Keeps `sess.piw` (`rx`, `projector`, `handle`) alive while parked; comment at `agent.rs:777` notes watching server state for external answers/cancellations (`V-4`). | If `ask_piw_gate_question` is used, `GateAsk::Answered(typed)` calls `answer_piw_gate` -> `parse_answer` -> `answer_blocking` (`decision.answer` or `checkpoint.answer`) and clears `sess.waiting_piw`. | `GateAsk::Withdrawn` (turn cancel) calls `piw_gate_withdrawn` -> `watch.handle.cancel()`. `GateAsk::Dismissed` waits 400ms to distinguish Esc from Ctrl+C; on Esc, parks without cancelling and sends `parked_gate_hint`. | `V-4`: verify parked external-event watcher after `agent.rs:781`. `V-5`: verify whether any active caller invokes `ask_piw_gate_question`. |
| **Grok ACPX Chat — `/answer <label>` Submit Path** | `grok-build`: `grok-acpx-acp/agent.rs` (`parse_command`, `Command::Answer`, `answer_piw_gate`) | Bare `/answer` (empty rest) currently parses to `Command::Help` (`parse_command`), printing `help_markdown`; it never submits, cancels, or starts a run. | Re-enters `pump_piw_events` after accepted PIW answer. | When `sess.waiting_piw` is `Some(waiting)`, `Command::Answer { label }` calls `answer_piw_gate(session_id, waiting.gate.clone(), &label, &timing)` -> `ensure_piw_server` -> `piw_shared_client` -> `parse_answer` -> `spawn_blocking(answer_blocking)`. On acceptance, clears `sess.waiting_piw` and calls `pump_piw_events`. | For `waiting_gate` (native only), validates choice label (printing `"Choices: ..."` on mismatch) and launches chained flow via `prepare_chained_run` + `run_flow_chain`. When neither gate is waiting, prints `"No gate is waiting."`. | Unlike `answer_waiting_gate`, `Command::Answer` does not fall back to `sess.piw.handle.gate()` when `waiting.gate` is `None`, and does not set `sess.in_flight = true` / `sess.cancelled = false` before `pump_piw_events`. Prompt precedence vs `handle_occupied_run_input` is `V-3`. |
| **Grok ACPX Chat — Plain Text While Occupied** | `grok-build`: `grok-acpx-acp/agent.rs` (`handle_occupied_run_input`, `answer_waiting_gate`, `parse_command`, `answer_piw_gate`) | When `in_flight || waiting_gate.is_some() || waiting_piw.is_some()`, checks `run_path::classify_mid_run_input` (non-`QueueFollowUp`), then matches text via `run_path::match_waiting_gate_text(&node, &choices, text)`. Unmatched text returns `GateTextMatch::Refused { refusal_message }` without submitting. | On accepted PIW answer, sets `in_flight = true`, `cancelled = false`, clears `waiting_piw`, and calls `pump_piw_events`. | `GateTextMatch::Matched { label, notes }` calls `answer_waiting_gate`, composing `typed` as `"{label} {notes}"` (or `label`), resolving `gate` from `waiting.gate` or fallback `sess.piw.handle.gate()`, and calling `answer_piw_gate`. (When not occupied, `parse_command` also maps exact plain-text matches in `waiting_labels` to `Command::Answer`.) | Non-matching text while `in_flight && !waiting` prints `"Unrecognized command. The run is still in progress and was not cancelled."` — never cancels or launches a flow. | `V-3`: `crate::run_path` (`classify_mid_run_input`, `match_waiting_gate_text`) and outer prompt dispatch order are outside the excerpts. |
| **Grok ACPX Chat — `/open <id>` Mirrored PIW Replay** | `grok-build`: `grok-acpx-acp/agent.rs` (`Command::Open`, `resolve_saved_run`, `saved_run_events`, `replay_run_into_session`) | `resolve_saved_run` resolves native runs first, then `SavedRunSource::PiwMirror`. `saved_run_events` overlays `question` (`status_detail`) and `choices` (`bundle::gate_choices`, with `flow_file: String::new()`). `replay_run_into_session` calls `projector.show_gates_read_only()` and inserts `run_id` into `sess.shown_runs`. | **None.** Does not attach `sess.piw`; re-running `/open <id>` returns `"Already shown above."`. | **None (read-only).** `replay_run_into_session` populates `waiting` / `sess.waiting_gate` only when `source == SavedRunSource::Native`, and never populates `sess.waiting_piw`. Submitting `/answer <label>` replies `"No gate is waiting."`. | Strictly inspection-only; cannot submit a PIW answer or launch a native flow. | Core design question in Section 4 below. |
| **Grok ACPX Chat — `/open <id>` Native ACPX Replay** | `grok-build`: `grok-acpx-acp/agent.rs` (`Command::Open`, `resolve_saved_run`, `replay_run_into_session`) | Replays native trace; when `DriverEvent::Waiting` has non-empty choices and `finished_status` is empty or `"waiting"`, restores `sess.waiting_gate = Some(WaitingGate { ... })`. | Static replay; records `run_id` in `sess.shown_runs`. | Restored `sess.waiting_gate` is answerable via `/answer <label>` or occupied plain text, launching the declared chained flow (`prepare_chained_run` -> `run_flow_chain`). | Native flow launch only; never invokes `answer_piw_gate`. | Preserved unchanged by this contract. |

### 2.3 Strict Separation: Gate Inspection, Explicit Answer, and Native Flow Launch

1. **Gate Inspection (Read-Only):** Opening a run in `/acpx`, viewing it in standalone `piw`, running `/runs` or `/open <id>` in chat, or invoking bare `/answer` must produce zero engine mutations (`decision.answer`, `checkpoint.answer`, `run.cancel`, `run.start`) and zero native flow launches (`AcpxOutcome::RunFlow`, `run_flow_chain`).
2. **Bare `/answer` Contract:**
   - **Current observed behavior:** `parse_command` (`agent.rs:2133-2146`) maps bare `/answer` (empty argument) to `Command::Help`, printing `help_markdown` and executing no submission, cancellation, or flow start.
   - **Required contract behavior:** Bare `/answer` may reveal the active waiting gate's question, node, and valid choices (including any required `text_input` field name) as a read-only response, and must **never** submit an answer, cancel a run, or start/launch a flow.
3. **Explicit Answer (PIW Route):** Triggered only by an explicit choice/input action targeting a live pending `PiwGate` (`requestId` + `revision`) on an authorized surface (`answer_mirror_gate` via `piw-answer://` or `answer_piw_gate` via `/answer <label>` / `answer_waiting_gate`), always mediated by `AnswerMachine` and coordinator authority checks.
4. **Native Flow Launch (ACPX Route):** Triggered only when `parse_answer_target` returns `None` (in `acpx_modal::handle_run_key`) or when `sess.waiting_gate` (populated exclusively for `SavedRunSource::Native`) is answered in chat.

---

## 3. Unified Cross-Surface State Transition Table

| State | Authoritative Engine State | Grok `/acpx` Modal | Standalone PIW TUI (`piw`) | Grok ACPX Chat (Live PIW vs. `/open` Mirror) |
| --- | --- | --- | --- | --- |
| **1. `pending`** | Run `status == "waiting"`, `waitingOn == <node_id>`; session `interaction` has `status == "pending"`, `kind == "decision" \| "checkpoint"`, `requestId`, `revision`; `controls` includes `"human-answer"` (decision) or `"answer"` (checkpoint). | Mirrored run appears in `SectionKind::NeedsYou` (`ledger::run_section`). `RunView` shows waiting status (`run::render_header`) and choices backed by `piw-answer://<run_id>/<choice_key>`. `m.gate(run_id)` returns `Some(PiwGate)`. | Node card renders `human decision · <audience>` (`render::human_decision_detail`) and branch lines `◇ <label>` (`bounded_branch_lines`). `toggle_op` offers `RunOp::Pause`; `Cancel` available. No verified gate-submit handler (`V-1`). | **Live PIW:** Inline waiting card projected (`pump_piw_events`), `sess.waiting_piw` armed, parked watch open. **Post-`/open` mirror:** Card rendered read-only (`projector.show_gates_read_only()`), `sess.waiting_piw == None`, `sess.piw == None`. |
| **2. `submitting`** | Engine has not yet committed the answer; `interaction` still `pending` at `expectedRevision`. Client may be in `WaitSnapshot`, `WaitCoordinator` (`watch_session(id, true)`), `WaitBranchReport` (`workflowMessage.reportBranch`), or `WaitAnswer` (`decision.answer` / `checkpoint.answer` with `submissionId` + `answer_idempotency_key`). | `answer_mirror_gate` enqueues `GateAnswerer` in `self.answerers` and shows notice `"{choice_label}: answer sent"`. `poll_answerers` polls non-blocking each tick (`AnswerProgress::Pending`). | Continues showing `pending` state (`human decision · <audience>`) until engine commits. | **Live PIW:** `answer_piw_gate` blocks off-thread in `spawn_blocking(answer_blocking)` (60s timeout) with `AnswerMachine`. **Post-`/open` mirror:** Not enterable today (`Command::Answer` returns `"No gate is waiting."`). |
| **3. `accepted` while the engine is still waiting** | Server returned `resp.ok()` to `decision.answer` / `checkpoint.answer`; coordinator claim is released (`cleanup_if_claimed` -> `watch_session(id, false)`). Brief convergence window before mirrored projections / run snapshot advance off `waitingOn`. | `poll_answerers` receives `AnswerProgress::Accepted`, sets `ModalNotice::warning("answer accepted; the run continues")`, and clears `self.answerers`. Until mirrored `live.json` updates, header/ledger reflect the prior `Waiting` snapshot (`V-6`: verify `self.answered` is not falsely setting `Completed` for PIW runs). | Still renders `human decision · <audience>` until the next `view.run.watch` snapshot arrives with `waitingOn` cleared (`V-2`). | **Live PIW:** `answer_piw_gate` returns `true`; `sess.waiting_piw` is cleared; chat re-enters `pump_piw_events` awaiting the next `DriverEvent` from the engine. **Post-`/open` mirror:** Unchanged (no live watch). |
| **4. `next live state`** | `interaction` for old `requestId` is no longer `pending`; `waitingOn` clears; `status == "running"`; `currentNode` moves to the chosen branch node; `human_decision` records `{ nodeId, response: { choice } }`. | Mirrored `live.json` transitions to `Running`; ledger moves run from `SectionKind::NeedsYou` to `SectionKind::Running` (`run_section`); header/detail show running glyph and updated node progress (`V-6` for live in-view refresh without reopening). | Previous gate card switches from `human decision · <audience>` to `human: <label>` (`human_decision_detail`); next branch node becomes active/running (`V-2` for live auto-refresh). | **Live PIW:** `pump_piw_events` streams `StepStarted` / `AgentUpdate` / `StepFinished` for the chosen branch (even if answered from `/acpx`, per `agent.rs:777`, `V-4`). **Post-`/open` mirror:** Remains frozen on old replay (`shown_runs` blocks `/open <id>` with `"Already shown above."`). |
| **5. `terminal`** | Run reaches `completed`, `failed`, `cancelled`, or `timed_out`; `controls` becomes `[]` (or `["cancel"]` only while origin turn active). | Ledger places run in `SectionKind::Finished` (`run_section`); header renders terminal glyph, duration, and age (`render_header`). `cancel_open_mirror_run` returns `false` (`!status.is_live()`). Leaving run calls `release_open_mirror_run`. | Gate card retains `human: <label>`; `toggle_op(&[])` returns `None`; `no_control_notice` reports `"cannot pause or resume (<status>)"`. | **Live PIW:** `pump_piw_events` receives `DriverEvent::RunFinished`, projects final card, breaks loop, and sets `sess.piw = None`. **Post-`/open` mirror:** Frozen until session ends or design change enables refresh. |
| **6. `stale/rejected`** | Either (a) `interaction` is already answered / `requestId` changed, (b) session belongs to a live Pi coordinator (`!coord.active && coord.epoch.is_some()`), (c) `revision` advanced, or (d) local payload validation failed. Engine state is unchanged by the rejected request. | `GateAnswerer::poll` returns `AnswerProgress::Refused(msg)` (`REFUSED_ALREADY_ANSWERED`, `REFUSED_LIVE_PI_SESSION`, or `server_error_sentence`), releasing any coordinator claim (`cleanup_if_claimed`). `poll_answerers` displays `ModalNotice::warning(msg)` and clears `self.answerers`. (Revision skew retries up to `MAX_REVISION_RETRIES = 3` first.) | Unaffected by the rejected client request; reflects only authoritative engine state (either still `pending` on live Pi session / bad input, or already showing the winning `human: <label>`). | **Live PIW:** `answer_piw_gate` prints `"The answer was not delivered: {message}"` (or `parse_answer` validation message) and returns `false`; `sess.waiting_piw` stays parked unless external-watch reconciles it as already answered (`V-4`). **Post-`/open` mirror:** Refused locally before network (`"No gate is waiting."`). |

---

## 4. Design Question: Live PIW Answers After `/open <id>` and Cross-Repo Ownership

### 4.1 Current Behavior on `/open <id>`

In `grok-build` (`crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs`):

1. `Command::Open { run_id }` checks `sess.shown_runs.contains(&run_id)` and immediately replies `"Already shown above."` if previously opened in the session.
2. Otherwise `resolve_saved_run` checks native runs first (`SavedRunSource::Native`), then the mirror root (`SavedRunSource::PiwMirror`).
3. `replay_run_into_session` calls `saved_run_events(run_dir, source)`. For `SavedRunSource::PiwMirror`:
   - `saved_run_events` overlays `question` (`run.summary.status_detail`) and `choices` (`bundle::gate_choices(&run)` with `flow_file: String::new()`).
   - `replay_run_into_session` calls `projector.show_gates_read_only()`.
   - The `DriverEvent::Waiting` branch captures `waiting` **only** when `source == SavedRunSource::Native && !choices.is_empty()`.
   - Neither `sess.waiting_piw` nor `sess.piw` is populated; no live server watch is started.

### 4.2 Concrete Adaptation Needed (Review-Gated — No Patch in This Packet)

To allow answering a live parked PIW run opened via `/open <id>` in Grok ACPX chat **without** changing gate authority, native replay, or workflow state, the following functions would need coordinated changes:

| Owning Repo | File & Function | Required Adaptation | Authority / Safety Invariant Preserved |
| --- | --- | --- | --- |
| **`grok-build`** | `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` — `replay_run_into_session` | When `source == SavedRunSource::PiwMirror` and replay finishes in `"waiting"` (or empty) status, query the live PIW server via the shared client (`piw_shared_client` / `gate_from` + `coordinator_state`). Only if the server confirms a live pending `PiwGate` for `run_id` **and** the session is not owned by a live Pi session (`!(!coord.active && coord.epoch.is_some())`), attach a live watch handle (`sess.piw`) and arm `sess.waiting_piw` using the live `PiwGate` (not static mirror disk files). Keep `projector.show_gates_read_only()` whenever the run is terminal, server is unreachable, no pending `PiwGate` exists, or a live Pi session owns the coordinator epoch. | Never arms a gate from stale disk projections; preserves `REFUSED_LIVE_PI_SESSION` and `SavedRunSource::Native` replay unchanged. |
| **`grok-build`** | `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` — `Command::Open` | Allow refreshing or re-attaching status for an already-shown `PiwMirror` run when its live state has changed, or surface current live gate status instead of a dead-end `"Already shown above."` without duplicating the full transcript. | Keeps inspection read-only; avoids stale transcript lock-in after external answers. |
| **`grok-build`** | `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` — `Command::Answer` and `parse_command` | (1) Give bare `/answer` a dedicated read-only inspect branch (or command variant) that prints the waiting gate's question, choices, and any `text_input` hint when a gate is waiting, without submitting, cancelling, or launching a flow. (2) Align `Command::Answer { label }` with `answer_waiting_gate` so it refreshes `gate` from `sess.piw.handle.gate()` and updates `sess.in_flight` / `sess.cancelled` consistently before `pump_piw_events`. | Guarantees bare `/answer` is strictly read-only and closes the state-sync gap between `/answer <label>` and occupied plain-text submission. |
| **`grok-build`** | `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` — `handle_occupied_run_input` (and `crate::run_path`, `V-3`) | Verify and ensure `/answer` (bare or with argument) routes cleanly through the inspect/explicit-answer contract when `sess.waiting_piw` is armed, without being misclassified as a cancellation or freeform task. | Preserves occupied-run safety (`"The run is still in progress and was not cancelled."`). |
| **`grok-build`** | `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` — `answer_mirror_gate` (optional scope review) | No change required for plain decision choices. Supporting decision choices with `text_input` or `GateKind::Checkpoint` in `/acpx` would require UI input collection rather than `text: None` and is a separate review decision. Verify `self.answered` is never populated for mirrored PIW runs (`V-6`). | Preserves `piw/gate.rs` `AnswerMachine` and `parse_answer_target` routing untouched. |
| **`pi-workflows`** | `tui/src/ui/run_command.rs`, `tui/src/render.rs`, `tui/src/protocol.rs` | **Review gate owner for engine protocol and standalone `piw` TUI.** No change is required in `pi-workflows` to support `/open <id>` live answering in `grok-build` because `decision.answer`, `checkpoint.answer`, `workflowMessage.reportBranch`, and `view.run.watch` are already in `pi-workflows.client.v1`. Any decision to add gate-submission UI to standalone `piw` TUI (or keep standalone `piw` read + `Pause`/`Resume`/`Cancel` only) is owned exclusively by `pi-workflows`. | Preserves single-owner protocol governance and coordinator authority rules. |

---

## 5. Fresh Parked Fixture Specification

### 5.1 Design Criteria (Why This Gate Is Worth Stopping For)

A gate fixture that guards a no-op echo does not test real operator judgment or observable branching. This fixture guards a real, local, non-destructive build/test step after producing a concrete preview artifact that an operator would inspect before deciding whether to run the test suite or archive the preview report.

- **Fixture Name:** `piw-cross-surface-gate-fixture`
- **Isolation:** Runs entirely inside a disposable temporary workspace (no production repo mutations, no remote deployments, no real protected decision).
- **Coordinator Precondition:** Started in an **unowned** workflow session (`coordinatorActive: false`, `coordinatorEpoch: null`) so an authorized Grok surface can claim coordinator authority via `watch_session(session_id, true)`, submit once, and release via `watch_session(session_id, false)`. (A separate fixture instance bound to a live Pi session is used only for the protected-session refusal test in Section 6.5.)

### 5.2 Workflow Topology and Gate Contract

    prepare_preview  ──►  verify_gate (decision, audience: "operator")
                             ├── [choice: run_tests   / "Run unit checks"]   ──►  run_unit_checks  ──►  finalize_tested
                             └── [choice: report_only / "Archive report only"] ──►  write_report     ──►  finalize_archived

Node definitions (illustrative fixture contract; exact workflow schema fields to be verified against `pi-workflows` schema before execution):

1. **`prepare_preview` (completed before parking):**
   - Runs a fast local check in the temp workspace (for example, inspects a sample crate/module diff or prints candidate build flags and writes `preview-summary.txt` with file count and target check command).
   - Produces output visible in step details on all three surfaces so the operator has a concrete result to review at the gate.

2. **`verify_gate` (`kind: "decision"`, parked waiting node):**
   - `audience`: `"operator"`
   - `question`: `"Preview ready (preview-summary.txt). Run the local verification build/tests now, or archive the report without running checks?"`
   - `choices`:
     - `key: "run_tests"`, `label: "Run unit checks"`, `text_input: null` -> transitions to `run_unit_checks`
     - `key: "report_only"`, `label: "Archive report only"`, `text_input: null` -> transitions to `write_report`
   - Note: `text_input: null` is chosen intentionally for the primary cross-surface fixture so Grok `/acpx` (`answer_mirror_gate` with `text: None`) can submit it cleanly.

3. **Branch A — `run_unit_checks` -> `finalize_tested` (selected in walkthrough):**
   - `run_unit_checks`: executes a real, fast, observable local build/test step in the temp workspace (e.g., `cargo test` on a tiny fixture test or a deterministic verification script that takes ~2-5s so `next live state` / `Running` is directly observable before terminal completion) and writes `verification-result.txt`.
   - `finalize_tested`: records final summary `"branch=run_tests; checks=passed"` and finishes `completed`.

4. **Branch B — `write_report` -> `finalize_archived` (observable alternate branch):**
   - `write_report`: skips the build/test step, writes `archived-report.txt` from `preview-summary.txt`.
   - `finalize_archived`: records final summary `"branch=report_only; checks=skipped"` and finishes `completed`.

---

## 6. Operator Walkthrough

> **Command Confirmation Notice:** The supplied excerpts do not contain CLI startup flags for launching the `pi-workflows` server, starting a fixture run, or invoking standalone `piw`. Every command marked `[NEEDS CONFIRMATION]` must be confirmed against the local repo binaries before execution (`V-7`).

### 6.0 Pre-Flight: Park Fixture and Capture Baseline Engine Snapshot

1. Start the `pi-workflows` server and launch `piw-cross-surface-gate-fixture` in an unowned session `[NEEDS CONFIRMATION]`:

       # [NEEDS CONFIRMATION] exact subcommand and flags for starting fixture run in unowned session
       piw run start --workflow <path/to/piw-cross-surface-gate-fixture>

2. Wait until `prepare_preview` finishes and the run parks at `verify_gate`.
3. Capture Baseline Snapshot `B0` from the engine (`run.status` / `view.run.get` and session snapshot) `[NEEDS CONFIRMATION]`:

       # [NEEDS CONFIRMATION] exact CLI command to dump raw run + session JSON
       piw run status <RUN_ID> --json

4. **Expected Evidence (`B0`):**
   - `status == "waiting"`, `waitingOn == "verify_gate"`, `currentNode == "verify_gate"`
   - `interaction.status == "pending"`, `interaction.kind == "decision"`
   - Record exact `REQ_ID = interaction.requestId` and `REV_0 = interaction.revision`
   - `coordinatorActive == false` and `coordinatorEpoch == null` (confirms unowned session; safe for non-Pi authorized submission)
   - `controls` contains `"pause"`, `"cancel"`, `"human-answer"`

### 6.1 Phase 1 — Read-Only Inspection on All Three Surfaces

Open the **same `<RUN_ID>`** on all three surfaces simultaneously without answering:

1. **Surface 1 — Standalone PIW TUI (`piw`):**
   - Open `<RUN_ID>` in standalone `piw` `[NEEDS CONFIRMATION: piw tui --run <RUN_ID>]`.
   - **Expected Evidence:**
     - `prepare_preview` is complete; `verify_gate` card displays `human decision · operator` (`human_decision_detail`) and branch lines `◇ Run unit checks` and `◇ Archive report only` (`bounded_branch_lines`).
     - No answer is submitted; `toggle_op` maps controls to `RunOp::Pause` only (`V-1`).

2. **Surface 2 — Grok `/acpx` Modal:**
   - Open `/acpx` in Grok pager and open the mirrored folder for `<RUN_ID>` (`watch_open_mirror_run`).
   - **Expected Evidence:**
     - In the Flows ledger, `<RUN_ID>` appears under `SectionKind::NeedsYou` (`ledger::run_section`).
     - In `RunView`, header shows `Waiting` glyph (`run::render_header`), `verify_gate` question is visible, and the two choices carry targets `piw-answer://<RUN_ID>/run_tests` and `piw-answer://<RUN_ID>/report_only` (`ANSWER_SCHEME`).
     - Do **not** press `Enter` yet. Keep the modal open on `<RUN_ID>` to test live convergence in Phase 4.

3. **Surface 3 — Grok ACPX Chat:**
   - In a Grok ACPX chat session, run `/runs`, then `/open <RUN_ID>`.
   - **Expected Evidence:**
     - `/runs` lists `<RUN_ID>` with source label `"PIW mirror"` (`SavedRunSource::PiwMirror::label`).
     - `/open <RUN_ID>` (`Command::Open` -> `resolve_saved_run` -> `saved_run_events` -> `replay_run_into_session`) replays `prepare_preview` and renders the `verify_gate` card with question and both choice labels read-only (`projector.show_gates_read_only()`).
     - No auto-popup question modal opens; `sess.waiting_gate` and `sess.waiting_piw` remain `None`.

### 6.2 Phase 2 — Bare `/answer` Check and Before/After State Comparison

1. In Grok ACPX chat, send bare:

       /answer

2. **Expected Evidence (Current Implementation):**
   - `parse_command` returns `Command::Help`; chat prints `help_markdown`.
   - **Expected Evidence (Target Post-Design Implementation):** Chat prints the waiting gate's question and valid choices read-only.
   - In **both** current and target implementations: zero wire requests (`decision.answer`, `checkpoint.answer`, `run.cancel`, `run.start`) are emitted and no native flow starts.

3. Capture Engine Snapshot `B1` (`[NEEDS CONFIRMATION]` same status command as `B0`) and compare `B1` against `B0`:
   - `B1.status == "waiting"` and `B1.waitingOn == "verify_gate"` (identical to `B0`)
   - `B1.interaction.requestId == REQ_ID`, `B1.interaction.revision == REV_0`, `B1.interaction.status == "pending"` (identical to `B0`)
   - `B1.human_decision` is still absent/null
   - Neither `verification-result.txt` nor `archived-report.txt` exists in the fixture workspace.
   - Standalone `piw` and Grok `/acpx` show unchanged waiting state.

### 6.3 Phase 3 — Submit Exactly One Allowed Answer Through an Authorized Surface

1. **Authorized Surface (Current Code):** Grok `/acpx` modal (already open on `<RUN_ID>` from Phase 1).
2. Press `1` (select `"Run unit checks"`, target `piw-answer://<RUN_ID>/run_tests`) and press `Enter`.
3. **Expected Evidence During Submission & Acceptance:**
   - `handle_run_key` parses `piw-answer://<RUN_ID>/run_tests` via `parse_answer_target` and calls `answer_mirror_gate` (never `AcpxOutcome::RunFlow`).
   - Immediate notice: `"Run unit checks: answer sent"`.
   - `AnswerMachine` claims coordinator (`watch_session(session_id, true)`), submits optional `workflowMessage.reportBranch`, sends `decision.answer` with `expectedRevision == REV_0`, `requestId == REQ_ID`, `response: { "choice": "run_tests" }`, and releases coordinator claim (`watch_session(session_id, false)`).
   - On `AnswerProgress::Accepted`, `poll_answerers` displays notice `"answer accepted; the run continues"` and clears `self.answerers`.

### 6.4 Phase 4 — Cross-Surface Convergence Without Reopening

Observe all three surfaces **without closing or reopening** them, then capture Engine Snapshot `B2`:

1. **Authoritative Engine State (`B2`):**
   - `interaction` for `REQ_ID` is no longer `pending`.
   - `waitingOn` is cleared; run transitions through `currentNode == "run_unit_checks"` (`status == "running"`) -> `finalize_tested` -> `status == "completed"`.
   - `human_decision` records `nodeId == "verify_gate"`, `response.choice == "run_tests"`.
   - Workspace contains `verification-result.txt` and does **not** contain `archived-report.txt`.

2. **Standalone PIW TUI (`piw`) — Without Reopening:**
   - **Expected Evidence (subject to `V-2` live-watch confirmation):** `verify_gate` card updates from `human decision · operator` to `human: Run unit checks` (`human_decision_detail`); `run_unit_checks` transitions to active/running and then terminal complete; `write_report` remains unexecuted.

3. **Grok `/acpx` Modal — Without Reopening:**
   - **Expected Evidence (subject to `V-6` live-mirror refresh confirmation):** Because `watch_open_mirror_run` called `m.want_run(&run_id)`, the open view updates to show `run_unit_checks` running -> `finalize_tested` completed. Returning to the ledger (`Esc`) shows `<RUN_ID>` under `SectionKind::Running` (while active) or `SectionKind::Finished` (once complete), no longer in `SectionKind::NeedsYou`.

4. **Grok ACPX Chat — Current vs. Post-Design Evidence:**
   - **Current `/open <RUN_ID>` behavior:** Chat does **not** auto-update because `replay_run_into_session` attached no live watch (`sess.piw == None`), and re-running `/open <RUN_ID>` replies `"Already shown above."` (`shown_runs` guard). This mismatch is the exact gap addressed by the Section 4 design contract.
   - **Live chat-started run (control comparison):** A PIW run started directly in chat (`pump_piw_events`) retains its parked watch (`agent.rs:777`, `V-4`) and updates to the engine's next live state and final status when answered from `/acpx`.

### 6.5 Phase 5 — Separate Stale / Rejected Tests

Execute these on fresh disposable fixture instances — never against a real protected decision.

#### Test S1 — Already Answered / Stale `requestId` (Cross-Surface Race)
1. Start a fresh unowned `piw-cross-surface-gate-fixture` (`RUN_STALE_1`) and wait for `verify_gate` (`REQ_S1`, `REV_S1`).
2. Open `RUN_STALE_1` in two authorized observers (for example, Grok `/acpx` modal and a second client or CLI answer invocation `[NEEDS CONFIRMATION]`).
3. Submit Choice 1 (`run_tests`) from Observer 1 and wait for engine acceptance.
4. Submit Choice 2 (`report_only`) from Observer 2 against the now-stale `REQ_S1`.
5. **Expected Evidence:**
   - Observer 2's `AnswerMachine::handle_answer_refusal` observes missing or non-pending `interaction` for `REQ_S1` and returns `AnswerProgress::Refused("This question was already answered.")` (`REFUSED_ALREADY_ANSWERED`).
   - Grok `/acpx` displays `ModalNotice::warning("This question was already answered.")`; chat (if used) displays `"The answer was not delivered: This question was already answered."`.
   - Engine state commits only Branch A (`run_unit_checks` / `verification-result.txt`); `write_report` / `archived-report.txt` never runs.

#### Test S2 — Protected Coordinator / Live Pi Session Refusal
1. Start a fresh disposable `piw-cross-surface-gate-fixture` (`RUN_STALE_2`) inside an active Pi session so the session snapshot reports `coordinatorActive: false` (from the external observer's viewpoint) and `coordinatorEpoch: "<non-empty>"`.
2. Inspect `RUN_STALE_2` in Grok `/acpx` modal and attempt to press `2` + `Enter` (`report_only`).
3. **Expected Evidence:**
   - `AnswerMachine::poll` (`Phase::WaitSnapshot`) sees `!coord.active && coord.epoch.is_some()` and refuses without claiming coordinator or sending `decision.answer`: `AnswerProgress::Refused("This run belongs to a live Pi session; answer it there.")` (`REFUSED_LIVE_PI_SESSION`).
   - Engine snapshot remains unchanged (`status == "waiting"`, same `requestId` and `revision`).
   - Clean up by cancelling the disposable fixture run from the owning Pi session `[NEEDS CONFIRMATION]`.

#### Test S3 — Revision Skew and Local Validation Guard
1. **Local validation (no network send):** On a parked decision gate without `text_input`, send extra text via chat (`parse_answer` -> `Err`) or an invalid choice label (`Command::Answer` / `handle_occupied_run_input` -> `GateTextMatch::Refused`).
   - **Expected Evidence:** Single-sentence refusal naming the valid choices or input rule; engine `requestId` and `revision` unchanged; zero wire requests sent.
2. **Revision skew (same `requestId`, incremented `revision`):** When the engine increments `interaction.revision` while `status == "pending"` and `requestId` is unchanged, `handle_answer_refusal` retries up to `MAX_REVISION_RETRIES` (3) with fresh `submission_id` and `answer_idempotency_key`; after exhaustion or permanent server rejection, surfaces `server_error_sentence(resp)` and releases any coordinator claim (`cleanup_if_claimed`).

---

## 7. Verification Items Register

Items below cannot be established from the supplied 19 excerpts and must be verified in the respective repositories before implementing any patch or running the walkthrough:

| ID | Owning Repo | Area | What Must Be Verified |
| --- | --- | --- | --- |
| `V-1` | `pi-workflows` | Standalone `piw` TUI gate submission | Confirm whether any file outside `tui/src/ui/run_command.rs`, `tui/src/render.rs`, and `tui/src/protocol.rs` implements a UI handler sending `"decision.answer"` when `controls` includes `"human-answer"`. |
| `V-2` | `pi-workflows` | Standalone `piw` TUI live watch | Confirm the event loop driving `view.run.watch` updates `GraphView` / `RunState` in place without reopening the run view. |
| `V-3` | `grok-build` | ACPX chat prompt routing & `run_path` | Confirm outer prompt dispatch precedence between `handle_occupied_run_input` (`crate::run_path::classify_mid_run_input`, `match_waiting_gate_text`) and `parse_command`, especially for bare `/answer` and `/answer <label>` while `waiting_piw` is set. |
| `V-4` | `grok-build` | ACPX chat parked external-state watch | Confirm the watcher invoked after `agent.rs:781` propagates external answers and cancellations into the chat transcript and clears `sess.waiting_piw`. |
| `V-5` | `grok-build` | ACPX chat question modal callers | Confirm whether any active code path still calls `ask_piw_gate_question` or `ask_gate_question` after the Issue 11 inline-card parking change in `pump_piw_events`. |
| `V-6` | `grok-build` | `/acpx` modal live mirror refresh & `self.answered` | Confirm how `m.want_run(&run_id)` refreshes an already-open `RunView` without reopening, and confirm `self.answered` is never set for mirrored PIW runs (preventing premature `Completed` rendering while the engine resumes). |
| `V-7` | `pi-workflows` / `grok-build` | Operator CLI commands | Confirm exact binary names, flags, and subcommands for starting the fixture workflow, dumping raw run/session JSON, launching standalone `piw`, and cancelling disposable test runs. |
| `V-8` | `pi-workflows` | Server-side idempotency & branch report | Confirm server deduplication behavior for repeated `submissionId` / `answer_idempotency_key` across `restart_after_connection_loss`, and exact preconditions for `workflowMessage.reportBranch`. |
| `V-9` | `grok-build` | Mirror projection writer for PIW gates | Confirm how the mirror serializer maps `PiwGate` choices (including choices with `text_input` or `GateKind::Checkpoint`) into `steps.json` / `live.json` `piw-answer://` targets. |
