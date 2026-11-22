use crate::piw::client::{CONNECTION_LOST, ConnState, PiwClient, PiwResponse, RunView};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONNECTION_RESTARTS: u32 = 3;
const MAX_REVISION_RETRIES: u32 = 3;
const BLOCKING_POLL_INTERVAL: Duration = Duration::from_millis(20);

const REFUSED_LIVE_PI_SESSION: &str = "This run belongs to a live Pi session; answer it there.";
const REFUSED_ALREADY_ANSWERED: &str = "This question was already answered.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    Decision,
    Checkpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiwGateChoice {
    pub key: String,
    pub label: String,
    pub text_input: Option<String>,
}

/// A run parked on a question for a human.
#[derive(Debug, Clone, PartialEq)]
pub struct PiwGate {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub session_id: String,
    pub kind: GateKind,
    pub request_id: String,
    pub revision: u64,
    pub question: String,
    pub choices: Vec<PiwGateChoice>,
}

/// The gate a run is waiting on: Some only when `session` (a session snapshot from `PiwClient::session`) has a
/// pending `interaction` of kind "decision" or "checkpoint" for the run of `view`.
pub fn gate_from(view: &RunView, session: &Value) -> Option<PiwGate> {
    let view_run_id = run_id_of_view(view)?;
    let interaction = session.get("interaction").and_then(Value::as_object)?;

    if interaction.get("status").and_then(Value::as_str) != Some("pending") {
        return None;
    }

    let kind = match interaction.get("kind").and_then(Value::as_str)? {
        "decision" => GateKind::Decision,
        "checkpoint" => GateKind::Checkpoint,
        _ => return None,
    };

    let run_id = interaction.get("runId").and_then(non_empty_str)?;
    if run_id != view_run_id {
        return None;
    }

    let request_id = interaction.get("requestId").and_then(non_empty_str)?;
    let revision = interaction.get("revision").and_then(Value::as_u64)?;
    let session_id = interaction
        .get("targetSessionId")
        .and_then(non_empty_str)
        .or_else(|| session.get("sessionId").and_then(non_empty_str))?;
    let attempt_id = interaction
        .get("attemptId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let contract = interaction.get("contract").map(unwrap_escaped);

    let node_id = contract
        .and_then(|c| c.get("nodeId").and_then(non_empty_str))
        .or_else(|| {
            contract
                .and_then(|c| c.get("contract"))
                .map(unwrap_escaped)
                .and_then(|c| c.get("nodeId").and_then(non_empty_str))
        })
        .or_else(|| {
            view.view
                .get("state")
                .and_then(|s| s.get("waitingOn"))
                .and_then(non_empty_str)
        })
        .or_else(|| {
            view.view
                .get("state")
                .and_then(|s| s.get("currentNode"))
                .and_then(non_empty_str)
        })
        .or_else(|| attempt_id.split('#').next().filter(|s| !s.is_empty()))
        .unwrap_or("")
        .to_string();

    let node_def = view
        .view
        .get("workflow")
        .map(unwrap_escaped)
        .and_then(|w| w.get("nodes"))
        .map(unwrap_escaped)
        .and_then(|nodes| nodes.get(&node_id))
        .map(unwrap_escaped);

    let question = extract_question(contract, node_def, view, &node_id);

    let choices = match kind {
        GateKind::Checkpoint => Vec::new(),
        GateKind::Decision => extract_decision_choices(contract, node_def),
    };

    Some(PiwGate {
        run_id: run_id.to_string(),
        node_id,
        attempt_id,
        session_id: session_id.to_string(),
        kind,
        request_id: request_id.to_string(),
        revision,
        question,
        choices,
    })
}

/// Grok Build's existing gate plumbing carries a "flow file" string per choice. For pi-workflows gates that string
/// is an answer target: `piw-answer://<run id>/<choice key>` (both percent-encoded: every byte outside
/// [A-Za-z0-9._-] as %XX). The coordinator routes such targets to the answer path instead of starting a flow.
pub const ANSWER_SCHEME: &str = "piw-answer://";

pub fn answer_target(run_id: &str, choice_key: &str) -> String {
    format!(
        "{ANSWER_SCHEME}{}/{}",
        percent_encode(run_id),
        percent_encode(choice_key)
    )
}

/// (run id, choice key) of an answer target; None for anything else (for example a real flow file path).
pub fn parse_answer_target(target: &str) -> Option<(String, String)> {
    let rest = target.strip_prefix(ANSWER_SCHEME)?;
    let (raw_run, raw_choice) = rest.split_once('/')?;
    if raw_choice.contains('/') {
        return None;
    }
    let run_id = percent_decode(raw_run)?;
    let choice_key = percent_decode(raw_choice)?;
    if run_id.is_empty() || choice_key.is_empty() {
        return None;
    }
    Some((run_id, choice_key))
}

/// What the user chose.
#[derive(Debug, Clone, PartialEq)]
pub enum GateAnswer {
    /// A decision choice (index into `PiwGate::choices`), with the text for its declared input when it has one.
    Choice { index: usize, text: Option<String> },
    /// A checkpoint's input: any JSON value.
    Input(Value),
}

/// Match what a user typed against a gate.
/// Decision: a 1-based number, a choice key or label (case-insensitive, surrounding space ignored) or an
/// unambiguous prefix of one; anything after the first whitespace following the choice is the text for that
/// choice's input ("2 because the tests fail"). A choice that declares a text input and gets none is an Err that
/// says so; text given to a choice without an input is an Err.
/// Checkpoint: the whole text, as JSON when it parses as JSON, else as a JSON string.
/// Err is one sentence naming the valid choices.
pub fn parse_answer(gate: &PiwGate, typed: &str) -> Result<GateAnswer, String> {
    match gate.kind {
        GateKind::Checkpoint => {
            let trimmed = typed.trim();
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                Ok(GateAnswer::Input(value))
            } else {
                Ok(GateAnswer::Input(Value::String(trimmed.to_string())))
            }
        }
        GateKind::Decision => parse_decision_answer(&gate.choices, typed),
    }
}

/// The request that answers the gate: (operation, payload, expectedRevision). Pure.
pub fn answer_request(
    gate: &PiwGate,
    answer: &GateAnswer,
    coordinator_epoch: &str,
    submission_id: &str,
) -> Result<(String, Value, u64), String> {
    if coordinator_epoch.trim().is_empty() {
        return Err("A coordinator epoch is required to answer the gate.".to_string());
    }
    if submission_id.trim().is_empty() {
        return Err("A submission id is required to answer the gate.".to_string());
    }

    match (gate.kind, answer) {
        (GateKind::Decision, GateAnswer::Choice { index, text }) => {
            let Some(choice) = gate.choices.get(*index) else {
                return Err(format!(
                    "Choice index {} is out of range for this decision.",
                    index.saturating_add(1)
                ));
            };
            let response = match (&choice.text_input, text) {
                (Some(input_name), Some(value)) => {
                    if value.trim().is_empty() {
                        return Err(format!(
                            "Choice \"{}\" requires non-empty text for \"{}\".",
                            choice.label, input_name
                        ));
                    }
                    json!({
                        "choice": choice.key,
                        "input": { input_name: value },
                    })
                }
                (Some(input_name), None) => {
                    return Err(format!(
                        "Choice \"{}\" requires text for \"{}\".",
                        choice.label, input_name
                    ));
                }
                (None, Some(_)) => {
                    return Err(format!(
                        "Choice \"{}\" does not accept text input.",
                        choice.label
                    ));
                }
                (None, None) => {
                    json!({ "choice": choice.key })
                }
            };
            let payload = json!({
                "targetSessionId": gate.session_id,
                "coordinatorEpoch": coordinator_epoch,
                "requestId": gate.request_id,
                "submissionId": submission_id,
                "response": response,
            });
            Ok(("decision.answer".to_string(), payload, gate.revision))
        }
        (GateKind::Checkpoint, GateAnswer::Input(input)) => {
            let payload = json!({
                "targetSessionId": gate.session_id,
                "coordinatorEpoch": coordinator_epoch,
                "requestId": gate.request_id,
                "submissionId": submission_id,
                "input": input,
            });
            Ok(("checkpoint.answer".to_string(), payload, gate.revision))
        }
        (GateKind::Decision, GateAnswer::Input(_)) => {
            Err("A decision gate requires a choice answer, not free checkpoint input.".to_string())
        }
        (GateKind::Checkpoint, GateAnswer::Choice { .. }) => {
            Err("A checkpoint gate requires an input value, not a decision choice.".to_string())
        }
    }
}

/// The session a run belongs to, read from its view.
pub fn session_of_view(view: &RunView) -> Option<String> {
    view.view
        .get("queue")
        .and_then(|q| q.get("originSessionId"))
        .and_then(non_empty_str)
        .or_else(|| {
            view.view
                .get("session")
                .and_then(|s| s.get("binding"))
                .and_then(|b| b.get("piSessionId"))
                .and_then(non_empty_str)
        })
        .or_else(|| view.view.get("originSessionId").and_then(non_empty_str))
        .map(str::to_string)
}

/// The coordinator fields of a session snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorState {
    pub epoch: Option<String>,
    pub active: bool,
    pub branch_report_required: bool,
}

pub fn coordinator_state(session_snapshot: &Value) -> CoordinatorState {
    let epoch = session_snapshot
        .get("coordinatorEpoch")
        .and_then(non_empty_str)
        .map(str::to_string);
    let active = session_snapshot
        .get("coordinatorActive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let branch_report_required = session_snapshot
        .get("branchReportRequired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    CoordinatorState {
        epoch,
        active,
        branch_report_required,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnswerProgress {
    /// Still working. Call `poll` again on the next tick.
    Pending,
    /// The server accepted the answer; the run resumes.
    Accepted,
    /// The answer was not delivered. The message is one sentence, ready to show to the user.
    Refused(String),
}

/// A non-blocking state machine for one answer, driven from the UI tick.
pub struct GateAnswerer {
    client: Arc<PiwClient>,
    machine: AnswerMachine,
}

impl GateAnswerer {
    pub fn new(client: Arc<PiwClient>, gate: PiwGate, answer: GateAnswer) -> Self {
        Self {
            client,
            machine: AnswerMachine::new(gate, answer),
        }
    }

    pub fn poll(&mut self) -> AnswerProgress {
        self.machine.poll(self.client.as_ref(), Instant::now())
    }
}

impl Drop for GateAnswerer {
    fn drop(&mut self) {
        self.machine.cleanup_if_claimed(self.client.as_ref());
    }
}

/// The same, blocking, for the helper binary's answer command. Never call on the UI thread.
pub fn answer_blocking(
    client: &PiwClient,
    gate: &PiwGate,
    answer: &GateAnswer,
    timeout: Duration,
) -> anyhow::Result<()> {
    run_blocking_with(client, gate, answer, timeout, Instant::now, |dur| {
        std::thread::sleep(dur);
    })
}

// ==== Transport Abstraction & State Machine ====

trait GateTransport {
    fn state(&self) -> ConnState;
    fn connection_count(&self) -> u64;
    fn watch_session(&self, session_id: &str, coordinator: bool);
    fn session(&self, session_id: &str) -> Option<Value>;
    fn unwatch_session(&self, session_id: &str);
    fn send(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
    ) -> String;
    fn send_keyed(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
        idempotency_key: &str,
    ) -> String;
    fn take_response(&self, request_id: &str) -> Option<PiwResponse>;
}

impl GateTransport for PiwClient {
    fn state(&self) -> ConnState {
        PiwClient::state(self)
    }

    fn connection_count(&self) -> u64 {
        PiwClient::connection_count(self)
    }

    fn watch_session(&self, session_id: &str, coordinator: bool) {
        PiwClient::watch_session(self, session_id, coordinator);
    }

    fn session(&self, session_id: &str) -> Option<Value> {
        PiwClient::session(self, session_id)
    }

    fn unwatch_session(&self, session_id: &str) {
        PiwClient::unwatch_session(self, session_id);
    }

    fn send(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
    ) -> String {
        PiwClient::send(self, operation, run_id, expected_revision, payload)
    }

    fn send_keyed(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
        idempotency_key: &str,
    ) -> String {
        PiwClient::send_keyed(
            self,
            operation,
            run_id,
            expected_revision,
            payload,
            idempotency_key,
        )
    }

    fn take_response(&self, request_id: &str) -> Option<PiwResponse> {
        PiwClient::take_response(self, request_id)
    }
}

enum Phase {
    Init,
    WaitSnapshot {
        since: Instant,
    },
    WaitCoordinator {
        since: Instant,
    },
    WaitBranchReport {
        request_id: String,
        epoch: String,
        since: Instant,
    },
    WaitAnswer {
        request_id: String,
        epoch: String,
        since: Instant,
    },
    Done(AnswerProgress),
}

struct AnswerMachine {
    gate: PiwGate,
    answer: GateAnswer,
    phase: Phase,
    claimed_role: bool,
    released_role: bool,
    seen_connection_count: u64,
    awaiting_reconnect_from: Option<u64>,
    connection_restarts: u32,
    revision_retries: u32,
    submission_id: String,
    answer_idempotency_key: String,
}

impl AnswerMachine {
    fn new(gate: PiwGate, answer: GateAnswer) -> Self {
        Self {
            gate,
            answer,
            phase: Phase::Init,
            claimed_role: false,
            released_role: false,
            seen_connection_count: 0,
            awaiting_reconnect_from: None,
            connection_restarts: 0,
            revision_retries: 0,
            submission_id: new_id("piw-sub"),
            answer_idempotency_key: new_id("piw-ans"),
        }
    }

    fn poll(&mut self, client: &dyn GateTransport, now: Instant) -> AnswerProgress {
        if let Phase::Done(progress) = &self.phase {
            return progress.clone();
        }

        if client.state() == ConnState::Closed {
            return self.finish_refused(client, "The workflow server connection was closed.");
        }

        if matches!(self.phase, Phase::Init) {
            if let Err(err) = answer_request(&self.gate, &self.answer, "probe-epoch", "probe-sub") {
                return self.finish_refused(client, &ensure_sentence(&err));
            }
            self.seen_connection_count = client.connection_count();
            client.watch_session(&self.gate.session_id, false);
            self.phase = Phase::WaitSnapshot { since: now };
            return AnswerProgress::Pending;
        }

        let current_conn = client.connection_count();
        if current_conn != self.seen_connection_count {
            if self.awaiting_reconnect_from == Some(self.seen_connection_count) {
                self.seen_connection_count = current_conn;
                self.awaiting_reconnect_from = None;
            } else {
                return self.restart_after_connection_loss(client, now, false);
            }
        }

        match &self.phase {
            Phase::Init | Phase::Done(_) => unreachable!("handled above"),
            Phase::WaitSnapshot { since } => {
                let since = *since;
                let Some(snapshot) = client.session(&self.gate.session_id) else {
                    if now.saturating_duration_since(since) >= SNAPSHOT_TIMEOUT {
                        return self.finish_refused(
                            client,
                            "Timed out waiting for the session snapshot from the workflow server.",
                        );
                    }
                    return AnswerProgress::Pending;
                };
                self.awaiting_reconnect_from = None;
                let coord = coordinator_state(&snapshot);
                if coord.active {
                    let Some(epoch) = coord.epoch else {
                        if now.saturating_duration_since(since) >= SNAPSHOT_TIMEOUT {
                            return self.finish_refused(
                                client,
                                "Timed out waiting for a coordinator epoch from the workflow server.",
                            );
                        }
                        return AnswerProgress::Pending;
                    };
                    if coord.branch_report_required {
                        return self.send_branch_report(client, epoch, now);
                    }
                    return self.send_answer(client, epoch, now);
                }
                if coord.epoch.is_some() {
                    return self.finish_refused(client, REFUSED_LIVE_PI_SESSION);
                }
                self.claimed_role = true;
                tracing::debug!(
                    session_id = %self.gate.session_id,
                    run_id = %self.gate.run_id,
                    "claiming unowned workflow session coordinator role"
                );
                client.watch_session(&self.gate.session_id, true);
                self.phase = Phase::WaitCoordinator { since: now };
                AnswerProgress::Pending
            }
            Phase::WaitCoordinator { since } => {
                let since = *since;
                if let Some(snapshot) = client.session(&self.gate.session_id) {
                    let coord = coordinator_state(&snapshot);
                    if coord.active
                        && let Some(epoch) = coord.epoch
                    {
                        if coord.branch_report_required {
                            return self.send_branch_report(client, epoch, now);
                        }
                        return self.send_answer(client, epoch, now);
                    }
                }
                if now.saturating_duration_since(since) >= SNAPSHOT_TIMEOUT {
                    return self.finish_refused(
                        client,
                        "Timed out waiting for session coordinator authority from the workflow server.",
                    );
                }
                AnswerProgress::Pending
            }
            Phase::WaitBranchReport {
                request_id,
                epoch,
                since,
            } => {
                let request_id = request_id.clone();
                let epoch = epoch.clone();
                let since = *since;
                let Some(resp) = client.take_response(&request_id) else {
                    if now.saturating_duration_since(since) >= RESPONSE_TIMEOUT {
                        return self.finish_refused(
                            client,
                            "Timed out waiting for the workflow server to accept the branch report.",
                        );
                    }
                    return AnswerProgress::Pending;
                };
                if resp.outcome == CONNECTION_LOST {
                    return self.restart_after_connection_loss(client, now, true);
                }
                if resp.ok() {
                    return self.send_answer(client, epoch, now);
                }
                let msg = server_error_sentence(&resp);
                self.finish_refused(client, &msg)
            }
            Phase::WaitAnswer {
                request_id,
                epoch,
                since,
            } => {
                let request_id = request_id.clone();
                let epoch = epoch.clone();
                let since = *since;
                let Some(resp) = client.take_response(&request_id) else {
                    if now.saturating_duration_since(since) >= RESPONSE_TIMEOUT {
                        return self.finish_refused(
                            client,
                            "Timed out waiting for the workflow server to respond to the gate answer.",
                        );
                    }
                    return AnswerProgress::Pending;
                };
                if resp.outcome == CONNECTION_LOST {
                    return self.restart_after_connection_loss(client, now, true);
                }
                if resp.ok() {
                    return self.finish_accepted(client);
                }
                self.handle_answer_refusal(client, &resp, epoch, now)
            }
        }
    }

    fn send_branch_report(
        &mut self,
        client: &dyn GateTransport,
        epoch: String,
        now: Instant,
    ) -> AnswerProgress {
        let payload = json!({
            "targetSessionId": self.gate.session_id,
            "coordinatorEpoch": epoch,
            "workflowMessageId": Value::Null,
            "piSessionEntryId": Value::Null,
            "isIdle": true,
            "hasPendingMessages": false,
        });
        let request_id = client.send("workflowMessage.reportBranch", None, None, payload);
        self.phase = Phase::WaitBranchReport {
            request_id,
            epoch,
            since: now,
        };
        AnswerProgress::Pending
    }

    fn send_answer(
        &mut self,
        client: &dyn GateTransport,
        epoch: String,
        now: Instant,
    ) -> AnswerProgress {
        let (operation, payload, expected_revision) =
            match answer_request(&self.gate, &self.answer, &epoch, &self.submission_id) {
                Ok(built) => built,
                Err(err) => return self.finish_refused(client, &ensure_sentence(&err)),
            };
        let request_id = client.send_keyed(
            &operation,
            Some(&self.gate.run_id),
            Some(expected_revision),
            payload,
            &self.answer_idempotency_key,
        );
        self.phase = Phase::WaitAnswer {
            request_id,
            epoch,
            since: now,
        };
        AnswerProgress::Pending
    }

    fn handle_answer_refusal(
        &mut self,
        client: &dyn GateTransport,
        resp: &PiwResponse,
        epoch: String,
        now: Instant,
    ) -> AnswerProgress {
        if let Some(snapshot) = client.session(&self.gate.session_id) {
            let interaction = snapshot.get("interaction").and_then(Value::as_object);
            match interaction {
                None => {
                    return self.finish_refused(client, REFUSED_ALREADY_ANSWERED);
                }
                Some(inter) => {
                    let req_id = inter.get("requestId").and_then(Value::as_str);
                    let status = inter.get("status").and_then(Value::as_str);
                    if req_id != Some(self.gate.request_id.as_str()) || status != Some("pending") {
                        return self.finish_refused(client, REFUSED_ALREADY_ANSWERED);
                    }
                    if let Some(new_rev) = inter.get("revision").and_then(Value::as_u64)
                        && new_rev != self.gate.revision
                        && self.revision_retries < MAX_REVISION_RETRIES
                    {
                        self.revision_retries += 1;
                        self.gate.revision = new_rev;
                        self.submission_id = new_id("piw-sub");
                        self.answer_idempotency_key = new_id("piw-ans");
                        let coord = coordinator_state(&snapshot);
                        let active_epoch = if coord.active {
                            coord.epoch.unwrap_or(epoch)
                        } else {
                            epoch
                        };
                        if coord.active && coord.branch_report_required {
                            return self.send_branch_report(client, active_epoch, now);
                        }
                        return self.send_answer(client, active_epoch, now);
                    }
                }
            }
        }
        let msg = server_error_sentence(resp);
        self.finish_refused(client, &msg)
    }

    fn restart_after_connection_loss(
        &mut self,
        client: &dyn GateTransport,
        now: Instant,
        from_connection_lost_response: bool,
    ) -> AnswerProgress {
        if self.connection_restarts >= MAX_CONNECTION_RESTARTS {
            return self.finish_refused(
                client,
                "The connection to the workflow server keeps dropping; the answer could not be delivered.",
            );
        }
        self.connection_restarts += 1;
        let current_conn = client.connection_count();
        self.seen_connection_count = current_conn;
        self.awaiting_reconnect_from = if from_connection_lost_response {
            Some(current_conn)
        } else {
            None
        };
        tracing::debug!(
            session_id = %self.gate.session_id,
            restarts = self.connection_restarts,
            "restarting gate answerer after connection loss"
        );
        client.watch_session(&self.gate.session_id, false);
        self.phase = Phase::WaitSnapshot { since: now };
        AnswerProgress::Pending
    }

    fn finish_accepted(&mut self, client: &dyn GateTransport) -> AnswerProgress {
        self.cleanup_if_claimed(client);
        self.phase = Phase::Done(AnswerProgress::Accepted);
        AnswerProgress::Accepted
    }

    fn finish_refused(&mut self, client: &dyn GateTransport, message: &str) -> AnswerProgress {
        self.cleanup_if_claimed(client);
        let progress = AnswerProgress::Refused(ensure_sentence(message));
        self.phase = Phase::Done(progress.clone());
        progress
    }

    fn cleanup_if_claimed(&mut self, client: &dyn GateTransport) {
        if self.claimed_role && !self.released_role {
            self.released_role = true;
            client.unwatch_session(&self.gate.session_id);
        }
    }

    fn abort(&mut self, client: &dyn GateTransport) {
        self.cleanup_if_claimed(client);
        if !matches!(self.phase, Phase::Done(_)) {
            self.phase = Phase::Done(AnswerProgress::Refused(
                "Timed out waiting to answer the workflow gate.".to_string(),
            ));
        }
    }
}

fn run_blocking_with<FNow, FSleep>(
    client: &dyn GateTransport,
    gate: &PiwGate,
    answer: &GateAnswer,
    timeout: Duration,
    mut now: FNow,
    mut sleep: FSleep,
) -> anyhow::Result<()>
where
    FNow: FnMut() -> Instant,
    FSleep: FnMut(Duration),
{
    let start = now();
    let mut machine = AnswerMachine::new(gate.clone(), answer.clone());
    loop {
        let current = now();
        match machine.poll(client, current) {
            AnswerProgress::Accepted => return Ok(()),
            AnswerProgress::Refused(msg) => return Err(anyhow::anyhow!("{msg}")),
            AnswerProgress::Pending => {}
        }
        let elapsed = current.saturating_duration_since(start);
        if elapsed >= timeout {
            machine.abort(client);
            return Err(anyhow::anyhow!(
                "Timed out waiting to answer the workflow gate."
            ));
        }
        let remaining = timeout.saturating_sub(elapsed);
        sleep(remaining.min(BLOCKING_POLL_INTERVAL));
    }
}

// ==== Private Helpers ====

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

fn non_empty_str(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

fn non_empty_trimmed_str(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn unwrap_escaped(mut value: &Value) -> &Value {
    // Unwrap nested `{"$escaped": ...}` sentinels defensively with a hard bound.
    for _ in 0..8 {
        let Some(obj) = value.as_object() else {
            break;
        };
        if obj.len() != 1 {
            break;
        }
        let Some(inner) = obj.get("$escaped") else {
            break;
        };
        value = inner;
    }
    value
}

fn run_id_of_view(view: &RunView) -> Option<&str> {
    view.view
        .get("runId")
        .and_then(non_empty_str)
        .or_else(|| {
            view.view
                .get("queue")
                .and_then(|q| q.get("runId"))
                .and_then(non_empty_str)
        })
        .or_else(|| {
            view.view
                .get("state")
                .and_then(|s| s.get("runId"))
                .and_then(non_empty_str)
        })
        .or_else(|| {
            view.view
                .get("manifest")
                .and_then(|m| m.get("runId"))
                .and_then(non_empty_str)
        })
}

fn extract_question(
    contract: Option<&Value>,
    node_def: Option<&Value>,
    view: &RunView,
    node_id: &str,
) -> String {
    let from_contract = contract.and_then(|c| {
        c.get("presentation")
            .map(unwrap_escaped)
            .and_then(|p| p.get("summary"))
            .and_then(non_empty_trimmed_str)
            .or_else(|| c.get("summary").and_then(non_empty_trimmed_str))
            .or_else(|| c.get("question").and_then(non_empty_trimmed_str))
            .or_else(|| c.get("title").and_then(non_empty_trimmed_str))
            .or_else(|| {
                let prompt = c.get("prompt").map(unwrap_escaped)?;
                non_empty_trimmed_str(prompt)
                    .or_else(|| prompt.get("summary").and_then(non_empty_trimmed_str))
                    .or_else(|| prompt.get("question").and_then(non_empty_trimmed_str))
                    .or_else(|| prompt.get("title").and_then(non_empty_trimmed_str))
            })
    });

    if let Some(q) = from_contract {
        return q.to_string();
    }

    let from_node = node_def.and_then(|n| {
        n.get("summary")
            .and_then(non_empty_trimmed_str)
            .or_else(|| {
                n.get("humanDecision")
                    .map(unwrap_escaped)
                    .and_then(|h| h.get("summary"))
                    .and_then(non_empty_trimmed_str)
            })
            .or_else(|| n.get("statusDetail").and_then(non_empty_trimmed_str))
    });

    if let Some(q) = from_node {
        return q.to_string();
    }

    if let Some(detail) = view
        .view
        .get("state")
        .and_then(|s| s.get("statusDetail"))
        .and_then(non_empty_trimmed_str)
    {
        return detail.to_string();
    }

    node_id.to_string()
}

fn extract_decision_choices(
    contract: Option<&Value>,
    node_def: Option<&Value>,
) -> Vec<PiwGateChoice> {
    let contract_map = contract
        .and_then(|c| c.get("choices"))
        .map(unwrap_escaped)
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty());

    let map = contract_map.or_else(|| {
        node_def
            .and_then(|n| n.get("humanDecision"))
            .map(unwrap_escaped)
            .and_then(|h| h.get("choices"))
            .map(unwrap_escaped)
            .and_then(Value::as_object)
    });

    let Some(map) = map else {
        return Vec::new();
    };

    let mut choices: Vec<PiwGateChoice> = map
        .iter()
        .map(|(key, val)| {
            let val = unwrap_escaped(val);
            let label = val
                .get("label")
                .and_then(non_empty_str)
                .unwrap_or(key.as_str())
                .to_string();
            let text_input = val
                .get("input")
                .map(unwrap_escaped)
                .and_then(Value::as_object)
                .and_then(|input| {
                    if let Some(kind) = input.get("kind").and_then(Value::as_str)
                        && kind != "text"
                    {
                        return None;
                    }
                    input.get("name").and_then(non_empty_str).map(str::to_string)
                });
            PiwGateChoice {
                key: key.clone(),
                label,
                text_input,
            }
        })
        .collect();
    choices.sort_by(|a, b| a.key.cmp(&b.key));
    choices
}

fn percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while let Some(&b) = bytes.get(idx) {
        if b == b'%' {
            let hi = hex_nibble(*bytes.get(idx + 1)?)?;
            let lo = hex_nibble(*bytes.get(idx + 2)?)?;
            out.push((hi << 4) | lo);
            idx += 3;
        } else {
            out.push(b);
            idx += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_decision_answer(choices: &[PiwGateChoice], typed: &str) -> Result<GateAnswer, String> {
    if choices.is_empty() {
        return Err("This decision has no valid choices.".to_string());
    }
    let valid = format_valid_choices(choices);
    let trimmed = typed.trim();
    if trimmed.is_empty() {
        return Err(format!("No choice entered; valid choices are: {valid}."));
    }

    // 1. Exact key or label at a word boundary (supports multi-word labels with trailing text).
    if let Some(matched) = match_exact_word_boundary(choices, trimmed) {
        return match matched {
            ExactMatch::Unique { index, rest } => finalize_choice(choices, index, rest, &valid),
            ExactMatch::Ambiguous => Err(format!(
                "Choice \"{trimmed}\" is ambiguous; valid choices are: {valid}."
            )),
        };
    }

    // 2. First whitespace-delimited token as a 1-based number.
    let (first_token, after_first) = split_first_token(trimmed);
    if !first_token.is_empty() && first_token.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(number) = first_token.parse::<usize>()
            && number >= 1
            && number <= choices.len()
        {
            return finalize_choice(choices, number - 1, after_first, &valid);
        }
        return Err(format!(
            "Choice \"{first_token}\" is out of range; valid choices are: {valid}."
        ));
    }

    // 3. Unambiguous prefix match on the whole trimmed input (supports prefixes of multi-word labels).
    let whole_matches = prefix_matches(choices, trimmed);
    if whole_matches.len() == 1
        && let Some(&index) = whole_matches.first()
    {
        return finalize_choice(choices, index, None, &valid);
    }
    if whole_matches.len() > 1 {
        return Err(format!(
            "Choice \"{trimmed}\" is ambiguous; valid choices are: {valid}."
        ));
    }

    // 4. Unambiguous prefix match on the first token, treating everything after it as text input.
    if after_first.is_some() {
        let token_matches = prefix_matches(choices, first_token);
        if token_matches.len() == 1
            && let Some(&index) = token_matches.first()
        {
            return finalize_choice(choices, index, after_first, &valid);
        }
        if token_matches.len() > 1 {
            return Err(format!(
                "Choice \"{first_token}\" is ambiguous; valid choices are: {valid}."
            ));
        }
    }

    Err(format!(
        "Unknown choice \"{first_token}\"; valid choices are: {valid}."
    ))
}

enum ExactMatch<'a> {
    Unique { index: usize, rest: Option<&'a str> },
    Ambiguous,
}

fn match_exact_word_boundary<'a>(
    choices: &[PiwGateChoice],
    input: &'a str,
) -> Option<ExactMatch<'a>> {
    let mut best_words = 0usize;
    let mut best: Vec<(usize, Option<&'a str>)> = Vec::new();

    for (index, choice) in choices.iter().enumerate() {
        for candidate in [choice.key.as_str(), choice.label.as_str()] {
            let word_count = candidate.split_whitespace().count();
            if word_count == 0 {
                continue;
            }
            if let Some(rest) = strip_prefix_words_ci(input, candidate) {
                let rest = if rest.is_empty() { None } else { Some(rest) };
                if word_count > best_words {
                    best_words = word_count;
                    best.clear();
                    best.push((index, rest));
                } else if word_count == best_words
                    && !best.iter().any(|(existing, _)| *existing == index)
                {
                    best.push((index, rest));
                }
            }
        }
    }

    match best.as_slice() {
        [] => None,
        [(index, rest)] => Some(ExactMatch::Unique {
            index: *index,
            rest: *rest,
        }),
        _ => Some(ExactMatch::Ambiguous),
    }
}

fn strip_prefix_words_ci<'a>(input: &'a str, candidate: &str) -> Option<&'a str> {
    let mut remaining = input;
    let mut matched_any = false;
    for expected in candidate.split_whitespace() {
        remaining = remaining.trim_start();
        let (tok, rest) = split_first_token_raw(remaining);
        if tok.is_empty() || tok.to_lowercase() != expected.to_lowercase() {
            return None;
        }
        matched_any = true;
        remaining = rest;
    }
    if !matched_any {
        return None;
    }
    Some(remaining.trim())
}

fn split_first_token(s: &str) -> (&str, Option<&str>) {
    let (first, rest) = split_first_token_raw(s);
    let rest_trimmed = rest.trim();
    if rest_trimmed.is_empty() {
        (first, None)
    } else {
        (first, Some(rest_trimmed))
    }
}

fn split_first_token_raw(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(idx) => (&s[..idx], &s[idx..]),
        None => (s, ""),
    }
}

fn prefix_matches(choices: &[PiwGateChoice], prefix: &str) -> Vec<usize> {
    let needle = prefix.to_lowercase();
    choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| {
            let key_match = choice.key.to_lowercase().starts_with(&needle);
            let label_match = choice.label.to_lowercase().starts_with(&needle);
            (key_match || label_match).then_some(index)
        })
        .collect()
}

fn finalize_choice(
    choices: &[PiwGateChoice],
    index: usize,
    rest: Option<&str>,
    valid: &str,
) -> Result<GateAnswer, String> {
    let Some(choice) = choices.get(index) else {
        return Err(format!(
            "Choice index {} is out of range; valid choices are: {valid}.",
            index.saturating_add(1)
        ));
    };
    match (&choice.text_input, rest) {
        (Some(_), Some(text)) => Ok(GateAnswer::Choice {
            index,
            text: Some(text.to_string()),
        }),
        (Some(input_name), None) => Err(format!(
            "Choice \"{}\" requires text for \"{}\"; valid choices are: {valid}.",
            choice.label, input_name
        )),
        (None, Some(_)) => Err(format!(
            "Choice \"{}\" does not accept text input; valid choices are: {valid}.",
            choice.label
        )),
        (None, None) => Ok(GateAnswer::Choice { index, text: None }),
    }
}

fn format_valid_choices(choices: &[PiwGateChoice]) -> String {
    choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let num = i + 1;
            let name = if c.label.eq_ignore_ascii_case(&c.key) {
                c.label.clone()
            } else {
                format!("{} ({})", c.label, c.key)
            };
            match &c.text_input {
                Some(input_name) => format!("{num}: {name} [requires {input_name}]"),
                None => format!("{num}: {name}"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn server_error_sentence(resp: &PiwResponse) -> String {
    if let Some(text) = resp.error_text() {
        let cleaned = single_line(&text);
        if !cleaned.is_empty() {
            return ensure_sentence(&cleaned);
        }
    }
    ensure_sentence(&format!(
        "The workflow server refused the request ({})",
        resp.outcome
    ))
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ensure_sentence(text: &str) -> String {
    let cleaned = single_line(text);
    if cleaned.is_empty() {
        return "The workflow request was refused.".to_string();
    }
    if cleaned.ends_with('.') || cleaned.ends_with('!') || cleaned.ends_with('?') {
        cleaned
    } else {
        format!("{cleaned}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    fn sample_run_view(run_id: &str, session_id: &str) -> RunView {
        RunView {
            revision: 5,
            run_revision: 12,
            view: json!({
                "schema": "pi-workflows.run-view.v1",
                "runId": run_id,
                "revision": 5,
                "runRevision": 12,
                "queue": {
                    "runId": run_id,
                    "originSessionId": session_id,
                    "status": "parked"
                },
                "state": {
                    "schema": "pi-workflows.run-state.v1",
                    "runId": run_id,
                    "status": "waiting",
                    "waitingOn": "gate",
                    "statusDetail": "Waiting for operator input"
                },
                "workflow": {
                    "schema": "pi-workflows.definition-snapshot.v1",
                    "name": "deploy",
                    "startAt": "gate",
                    "nodes": {
                        "gate": {
                            "nodeType": "checkpoint",
                            "summary": "Node fallback summary",
                            "humanDecision": {
                                "audience": "operator",
                                "choices": {
                                    "stop": { "label": "Stop" },
                                    "approve": { "label": "Approve" },
                                    "rework": {
                                        "label": "Request changes",
                                        "input": {
                                            "kind": "text",
                                            "name": "reason",
                                            "prompt": "Why?",
                                            "minLength": 1,
                                            "maxLength": 500
                                        }
                                    }
                                }
                            }
                        },
                        "review": {
                            "nodeType": "checkpoint",
                            "summary": "Provide release notes"
                        }
                    },
                    "edges": []
                }
            }),
        }
    }

    fn decision_session_snapshot(
        run_id: &str,
        session_id: &str,
        revision: u64,
        status: &str,
    ) -> Value {
        json!({
            "schema": "pi-workflows.session-view.v1",
            "sessionId": session_id,
            "coordinatorEpoch": Value::Null,
            "coordinatorActive": false,
            "branchReportRequired": false,
            "interaction": {
                "requestId": "request-gate-1",
                "runId": run_id,
                "attemptId": "gate#1",
                "targetSessionId": session_id,
                "kind": "decision",
                "revision": revision,
                "status": status,
                "acceptedSubmissionId": Value::Null,
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:00.000Z",
                "settledAt": Value::Null,
                "consumedAt": Value::Null,
                "contract": {
                    "schema": "pi-workflows.human-decision-request.v1",
                    "decisionId": "request-gate-1",
                    "requestDigest": "digest-1",
                    "runId": run_id,
                    "workflowName": "deploy",
                    "nodeId": "gate",
                    "attemptId": "gate#1",
                    "audience": "operator",
                    "title": "Deploy?",
                    "presentation": {
                        "schema": "pi-workflows.decision-presentation.v1",
                        "summary": "Choose whether to deploy.",
                        "blocks": []
                    },
                    "choices": {
                        "stop": { "label": "Stop" },
                        "rework": {
                            "label": "Request changes",
                            "input": {
                                "kind": "text",
                                "name": "reason",
                                "prompt": "Explain what must change",
                                "minLength": 1,
                                "maxLength": 200
                            }
                        },
                        "approve": { "label": "Approve" }
                    }
                }
            }
        })
    }

    fn sample_decision_gate() -> PiwGate {
        PiwGate {
            run_id: "run-1".to_string(),
            node_id: "gate".to_string(),
            attempt_id: "gate#1".to_string(),
            session_id: "sess-1".to_string(),
            kind: GateKind::Decision,
            request_id: "request-gate-1".to_string(),
            revision: 1,
            question: "Choose whether to deploy.".to_string(),
            choices: vec![
                PiwGateChoice {
                    key: "approve".to_string(),
                    label: "Approve".to_string(),
                    text_input: None,
                },
                PiwGateChoice {
                    key: "rework".to_string(),
                    label: "Request changes".to_string(),
                    text_input: Some("reason".to_string()),
                },
                PiwGateChoice {
                    key: "stop".to_string(),
                    label: "Stop".to_string(),
                    text_input: None,
                },
            ],
        }
    }

    fn sample_checkpoint_gate() -> PiwGate {
        PiwGate {
            run_id: "run-1".to_string(),
            node_id: "review".to_string(),
            attempt_id: "review#1".to_string(),
            session_id: "sess-1".to_string(),
            kind: GateKind::Checkpoint,
            request_id: "request-cp-1".to_string(),
            revision: 3,
            question: "Provide release notes".to_string(),
            choices: Vec::new(),
        }
    }

    // ---- gate_from tests ----

    #[test]
    fn gate_from_decision_with_three_choices_sorted_and_text_input() {
        let view = sample_run_view("run-1", "sess-1");
        let session = decision_session_snapshot("run-1", "sess-1", 4, "pending");

        let Some(gate) = gate_from(&view, &session) else {
            panic!("expected decision gate");
        };
        assert_eq!(gate.run_id, "run-1");
        assert_eq!(gate.node_id, "gate");
        assert_eq!(gate.attempt_id, "gate#1");
        assert_eq!(gate.session_id, "sess-1");
        assert_eq!(gate.kind, GateKind::Decision);
        assert_eq!(gate.request_id, "request-gate-1");
        assert_eq!(gate.revision, 4);
        assert_eq!(gate.question, "Choose whether to deploy.");
        assert_eq!(
            gate.choices,
            vec![
                PiwGateChoice {
                    key: "approve".to_string(),
                    label: "Approve".to_string(),
                    text_input: None,
                },
                PiwGateChoice {
                    key: "rework".to_string(),
                    label: "Request changes".to_string(),
                    text_input: Some("reason".to_string()),
                },
                PiwGateChoice {
                    key: "stop".to_string(),
                    label: "Stop".to_string(),
                    text_input: None,
                },
            ]
        );
    }

    #[test]
    fn gate_from_decision_falls_back_to_workflow_node_when_contract_is_artifact() {
        let view = sample_run_view("run-1", "sess-1");
        let session = json!({
            "sessionId": "sess-1",
            "interaction": {
                "requestId": "request-gate-1",
                "runId": "run-1",
                "attemptId": "gate#1",
                "targetSessionId": "sess-1",
                "kind": "decision",
                "revision": 2,
                "status": "pending",
                "contract": {
                    "$artifact": {
                        "path": "artifacts/sha256/abc.json",
                        "mediaType": "application/json",
                        "bytes": 99999,
                        "sha256": "abc"
                    }
                }
            }
        });

        let Some(gate) = gate_from(&view, &session) else {
            panic!("expected gate from fallback");
        };
        assert_eq!(gate.node_id, "gate");
        assert_eq!(gate.question, "Node fallback summary");
        assert_eq!(gate.choices.len(), 3);
        assert_eq!(gate.choices[0].key, "approve");
        assert_eq!(gate.choices[1].key, "rework");
        assert_eq!(gate.choices[1].text_input.as_deref(), Some("reason"));
        assert_eq!(gate.choices[2].key, "stop");
    }

    #[test]
    fn gate_from_checkpoint_has_empty_choices_and_uses_node_summary_fallback() {
        let view = sample_run_view("run-1", "sess-1");
        let session = json!({
            "sessionId": "sess-1",
            "interaction": {
                "requestId": "request-cp-1",
                "runId": "run-1",
                "attemptId": "review#1",
                "targetSessionId": "sess-1",
                "kind": "checkpoint",
                "revision": 1,
                "status": "pending",
                "contract": {
                    "nodeId": "review"
                }
            }
        });

        let Some(gate) = gate_from(&view, &session) else {
            panic!("expected checkpoint gate");
        };
        assert_eq!(gate.kind, GateKind::Checkpoint);
        assert_eq!(gate.node_id, "review");
        assert_eq!(gate.question, "Provide release notes");
        assert!(gate.choices.is_empty());
    }

    #[test]
    fn gate_from_rejects_settled_other_run_and_agent_interactions() {
        let view = sample_run_view("run-1", "sess-1");

        // Settled interaction
        let settled = decision_session_snapshot("run-1", "sess-1", 2, "settled");
        assert_eq!(gate_from(&view, &settled), None);

        // Cancelled interaction
        let cancelled = decision_session_snapshot("run-1", "sess-1", 2, "cancelled");
        assert_eq!(gate_from(&view, &cancelled), None);

        // Another run's interaction
        let other_run = decision_session_snapshot("run-other", "sess-1", 1, "pending");
        assert_eq!(gate_from(&view, &other_run), None);

        // Agent interaction
        let agent = json!({
            "sessionId": "sess-1",
            "interaction": {
                "requestId": "request-agent-1",
                "runId": "run-1",
                "attemptId": "work#1",
                "targetSessionId": "sess-1",
                "kind": "agent",
                "revision": 1,
                "status": "pending",
                "contract": {}
            }
        });
        assert_eq!(gate_from(&view, &agent), None);

        // Missing or wrong-type fields never panic
        assert_eq!(gate_from(&view, &json!(null)), None);
        assert_eq!(gate_from(&view, &json!({"interaction": 42})), None);
    }

    // ---- parse_answer tests ----

    #[test]
    fn parse_answer_decision_by_number_key_label_and_prefix() {
        let gate = sample_decision_gate();

        // 1-based number
        assert_eq!(
            parse_answer(&gate, " 1 "),
            Ok(GateAnswer::Choice {
                index: 0,
                text: None
            })
        );
        assert_eq!(
            parse_answer(&gate, "3"),
            Ok(GateAnswer::Choice {
                index: 2,
                text: None
            })
        );

        // Key (case-insensitive, surrounding space ignored)
        assert_eq!(
            parse_answer(&gate, "  APPROVE  "),
            Ok(GateAnswer::Choice {
                index: 0,
                text: None
            })
        );
        assert_eq!(
            parse_answer(&gate, "stop"),
            Ok(GateAnswer::Choice {
                index: 2,
                text: None
            })
        );

        // Label
        assert_eq!(
            parse_answer(&gate, "Stop"),
            Ok(GateAnswer::Choice {
                index: 2,
                text: None
            })
        );

        // Unambiguous prefix
        assert_eq!(
            parse_answer(&gate, "app"),
            Ok(GateAnswer::Choice {
                index: 0,
                text: None
            })
        );
        assert_eq!(
            parse_answer(&gate, "st"),
            Ok(GateAnswer::Choice {
                index: 2,
                text: None
            })
        );
    }

    #[test]
    fn parse_answer_decision_with_text_input_and_errors() {
        let gate = sample_decision_gate();

        // Number + text
        assert_eq!(
            parse_answer(&gate, "2 because the tests fail"),
            Ok(GateAnswer::Choice {
                index: 1,
                text: Some("because the tests fail".to_string())
            })
        );

        // Key + text
        assert_eq!(
            parse_answer(&gate, "rework fix lint warnings"),
            Ok(GateAnswer::Choice {
                index: 1,
                text: Some("fix lint warnings".to_string())
            })
        );

        // Multi-word label + text
        assert_eq!(
            parse_answer(&gate, "Request changes fix lint warnings"),
            Ok(GateAnswer::Choice {
                index: 1,
                text: Some("fix lint warnings".to_string())
            })
        );

        // Prefix + text
        assert_eq!(
            parse_answer(&gate, "req missing docs"),
            Ok(GateAnswer::Choice {
                index: 1,
                text: Some("missing docs".to_string())
            })
        );

        // Missing required text
        let err_missing = parse_answer(&gate, "2");
        assert!(err_missing.is_err());
        let msg = err_missing.err().unwrap_or_default();
        assert!(msg.contains("reason"));
        assert!(msg.contains("Approve"));

        let err_missing_label = parse_answer(&gate, "Request changes");
        assert!(err_missing_label.is_err());

        // Text given to a choice without input
        let err_extra = parse_answer(&gate, "1 looks fine");
        assert!(err_extra.is_err());
        let msg_extra = err_extra.err().unwrap_or_default();
        assert!(msg_extra.contains("does not accept text"));
        assert!(msg_extra.contains("Approve"));

        // Out of range
        assert!(parse_answer(&gate, "0").is_err());
        assert!(parse_answer(&gate, "4").is_err());

        // Ambiguous prefix
        let mut ambiguous_gate = sample_decision_gate();
        ambiguous_gate.choices.push(PiwGateChoice {
            key: "apply".to_string(),
            label: "Apply".to_string(),
            text_input: None,
        });
        let err_ambig = parse_answer(&ambiguous_gate, "app");
        assert!(err_ambig.is_err());
        assert!(err_ambig.err().unwrap_or_default().contains("ambiguous"));
    }

    #[test]
    fn parse_answer_checkpoint_json_and_plain_text() {
        let cp = sample_checkpoint_gate();

        assert_eq!(
            parse_answer(&cp, r#"{"release": "v1.2.3", "ok": true}"#),
            Ok(GateAnswer::Input(json!({"release": "v1.2.3", "ok": true})))
        );
        assert_eq!(
            parse_answer(&cp, "42"),
            Ok(GateAnswer::Input(json!(42)))
        );
        assert_eq!(
            parse_answer(&cp, "  ship with release notes  "),
            Ok(GateAnswer::Input(Value::String(
                "ship with release notes".to_string()
            )))
        );
    }

    // ---- answer_request tests ----

    #[test]
    fn answer_request_decision_plain_and_with_input_and_checkpoint() {
        let gate = sample_decision_gate();
        let epoch = "session-epoch-1234";

        // Decision without input
        let res = answer_request(
            &gate,
            &GateAnswer::Choice {
                index: 0,
                text: None,
            },
            epoch,
            "sub-1",
        );
        assert_eq!(
            res,
            Ok((
                "decision.answer".to_string(),
                json!({
                    "targetSessionId": "sess-1",
                    "coordinatorEpoch": "session-epoch-1234",
                    "requestId": "request-gate-1",
                    "submissionId": "sub-1",
                    "response": { "choice": "approve" }
                }),
                1,
            ))
        );

        // Decision with input
        let res_with_input = answer_request(
            &gate,
            &GateAnswer::Choice {
                index: 1,
                text: Some("tests failed".to_string()),
            },
            epoch,
            "sub-2",
        );
        assert_eq!(
            res_with_input,
            Ok((
                "decision.answer".to_string(),
                json!({
                    "targetSessionId": "sess-1",
                    "coordinatorEpoch": "session-epoch-1234",
                    "requestId": "request-gate-1",
                    "submissionId": "sub-2",
                    "response": {
                        "choice": "rework",
                        "input": { "reason": "tests failed" }
                    }
                }),
                1,
            ))
        );

        // Checkpoint
        let cp = sample_checkpoint_gate();
        let res_cp = answer_request(
            &cp,
            &GateAnswer::Input(json!({"notes": "ready"})),
            epoch,
            "sub-3",
        );
        assert_eq!(
            res_cp,
            Ok((
                "checkpoint.answer".to_string(),
                json!({
                    "targetSessionId": "sess-1",
                    "coordinatorEpoch": "session-epoch-1234",
                    "requestId": "request-cp-1",
                    "submissionId": "sub-3",
                    "input": { "notes": "ready" }
                }),
                3,
            ))
        );

        // Guard rails
        assert!(
            answer_request(
                &gate,
                &GateAnswer::Choice {
                    index: 0,
                    text: None
                },
                "",
                "sub-1"
            )
            .is_err()
        );
        assert!(answer_request(&gate, &GateAnswer::Input(json!(1)), epoch, "sub-1").is_err());
        assert!(
            answer_request(
                &cp,
                &GateAnswer::Choice {
                    index: 0,
                    text: None
                },
                epoch,
                "sub-1"
            )
            .is_err()
        );
    }

    // ---- coordinator_state & session_of_view tests ----

    #[test]
    fn coordinator_state_and_session_of_view_parse_defensively() {
        assert_eq!(
            coordinator_state(&json!({
                "coordinatorEpoch": "session-epoch-abc",
                "coordinatorActive": true,
                "branchReportRequired": true
            })),
            CoordinatorState {
                epoch: Some("session-epoch-abc".to_string()),
                active: true,
                branch_report_required: true,
            }
        );

        assert_eq!(
            coordinator_state(&json!({
                "coordinatorEpoch": null,
                "coordinatorActive": false,
                "branchReportRequired": false
            })),
            CoordinatorState {
                epoch: None,
                active: false,
                branch_report_required: false,
            }
        );

        assert_eq!(
            coordinator_state(&json!({
                "coordinatorEpoch": 123,
                "coordinatorActive": "yes",
                "branchReportRequired": null
            })),
            CoordinatorState {
                epoch: None,
                active: false,
                branch_report_required: false,
            }
        );

        let view = sample_run_view("run-1", "sess-origin");
        assert_eq!(session_of_view(&view), Some("sess-origin".to_string()));

        let binding_only = RunView {
            revision: 1,
            run_revision: 1,
            view: json!({
                "runId": "run-2",
                "session": { "binding": { "piSessionId": "sess-bound" } }
            }),
        };
        assert_eq!(
            session_of_view(&binding_only),
            Some("sess-bound".to_string())
        );
    }

    // ---- answer_target / parse_answer_target tests ----

    #[test]
    fn answer_target_round_trip_and_non_targets() {
        let run_id = "run/2026 %20 alpha";
        let choice_key = "yes / 100% ok";
        let encoded = answer_target(run_id, choice_key);
        assert!(encoded.starts_with(ANSWER_SCHEME));
        assert!(!encoded[ANSWER_SCHEME.len()..].contains(' '));
        assert_eq!(
            parse_answer_target(&encoded),
            Some((run_id.to_string(), choice_key.to_string()))
        );

        // Non-targets return None
        assert_eq!(parse_answer_target("flows/ship.flow.ts"), None);
        assert_eq!(parse_answer_target("/tmp/flows/ship.flow.ts"), None);
        assert_eq!(parse_answer_target("piw-answer://only-one-part"), None);
        assert_eq!(parse_answer_target("piw-answer://a/b/c"), None);
        assert_eq!(parse_answer_target("piw-answer://a/%ZZ"), None);
        assert_eq!(parse_answer_target("piw-answer:///choice"), None);
        assert_eq!(parse_answer_target("piw-answer://run/"), None);
    }

    // ---- AnswerMachine tests with MockTransport ----

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        request_id: String,
        operation: String,
        run_id: Option<String>,
        expected_revision: Option<u64>,
        payload: Value,
        idempotency_key: Option<String>,
    }

    struct MockInner {
        conn_state: ConnState,
        conn_count: u64,
        watches: Vec<(String, bool)>,
        unwatches: Vec<String>,
        session_snapshots: HashMap<String, Option<Value>>,
        requests: Vec<RecordedRequest>,
        responses: HashMap<String, PiwResponse>,
        queued_responses: HashMap<String, VecDeque<PiwResponse>>,
        next_req_num: u64,
    }

    struct MockTransport {
        inner: Mutex<MockInner>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                inner: Mutex::new(MockInner {
                    conn_state: ConnState::Connected,
                    conn_count: 1,
                    watches: Vec::new(),
                    unwatches: Vec::new(),
                    session_snapshots: HashMap::new(),
                    requests: Vec::new(),
                    responses: HashMap::new(),
                    queued_responses: HashMap::new(),
                    next_req_num: 1,
                }),
            }
        }

        fn set_snapshot(&self, session_id: &str, snapshot: Option<Value>) {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .session_snapshots
                .insert(session_id.to_string(), snapshot);
        }

        fn set_connection_count(&self, count: u64) {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.conn_count = count;
        }

        fn set_conn_state(&self, state: ConnState) {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.conn_state = state;
        }

        fn respond(
            &self,
            request_id: &str,
            operation: &str,
            outcome: &str,
            error: Option<&str>,
        ) {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.responses.insert(
                request_id.to_string(),
                PiwResponse {
                    request_id: request_id.to_string(),
                    operation: operation.to_string(),
                    run_id: None,
                    outcome: outcome.to_string(),
                    revision: None,
                    receipt: None,
                    error: error.map(|e| Value::String(e.to_string())),
                },
            );
        }

        fn last_request(&self) -> Option<RecordedRequest> {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.requests.last().cloned()
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.requests.clone()
        }

        fn watches(&self) -> Vec<(String, bool)> {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.watches.clone()
        }

        fn unwatches(&self) -> Vec<String> {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.unwatches.clone()
        }
    }

    impl GateTransport for MockTransport {
        fn state(&self) -> ConnState {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).conn_state
        }

        fn connection_count(&self) -> u64 {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).conn_count
        }

        fn watch_session(&self, session_id: &str, coordinator: bool) {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.watches.push((session_id.to_string(), coordinator));
        }

        fn session(&self, session_id: &str) -> Option<Value> {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.session_snapshots.get(session_id).cloned().flatten()
        }

        fn unwatch_session(&self, session_id: &str) {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.unwatches.push(session_id.to_string());
        }

        fn send(
            &self,
            operation: &str,
            run_id: Option<&str>,
            expected_revision: Option<u64>,
            payload: Value,
        ) -> String {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let req_id = format!("req-{}", guard.next_req_num);
            guard.next_req_num += 1;
            guard.requests.push(RecordedRequest {
                request_id: req_id.clone(),
                operation: operation.to_string(),
                run_id: run_id.map(str::to_string),
                expected_revision,
                payload,
                idempotency_key: None,
            });
            if let Some(mut resp) = guard
                .queued_responses
                .get_mut(operation)
                .and_then(VecDeque::pop_front)
            {
                resp.request_id = req_id.clone();
                guard.responses.insert(req_id.clone(), resp);
            }
            req_id
        }

        fn send_keyed(
            &self,
            operation: &str,
            run_id: Option<&str>,
            expected_revision: Option<u64>,
            payload: Value,
            idempotency_key: &str,
        ) -> String {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let req_id = format!("req-{}", guard.next_req_num);
            guard.next_req_num += 1;
            guard.requests.push(RecordedRequest {
                request_id: req_id.clone(),
                operation: operation.to_string(),
                run_id: run_id.map(str::to_string),
                expected_revision,
                payload,
                idempotency_key: Some(idempotency_key.to_string()),
            });
            if let Some(mut resp) = guard
                .queued_responses
                .get_mut(operation)
                .and_then(VecDeque::pop_front)
            {
                resp.request_id = req_id.clone();
                guard.responses.insert(req_id.clone(), resp);
            }
            req_id
        }

        fn take_response(&self, request_id: &str) -> Option<PiwResponse> {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.responses.remove(request_id)
        }
    }

    fn snapshot_with_coord(
        epoch: Option<&str>,
        active: bool,
        branch_report_required: bool,
        interaction: Value,
    ) -> Value {
        json!({
            "schema": "pi-workflows.session-view.v1",
            "sessionId": "sess-1",
            "coordinatorEpoch": epoch,
            "coordinatorActive": active,
            "branchReportRequired": branch_report_required,
            "interaction": interaction
        })
    }

    fn pending_interaction_json(revision: u64) -> Value {
        json!({
            "requestId": "request-gate-1",
            "runId": "run-1",
            "attemptId": "gate#1",
            "targetSessionId": "sess-1",
            "kind": "decision",
            "revision": revision,
            "status": "pending",
            "contract": {}
        })
    }

    #[test]
    fn answerer_unowned_session_full_sequence_and_cleanup() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        // Step 1: watch without claiming
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        assert_eq!(mock.watches(), vec![("sess-1".to_string(), false)]);

        // Step 2: unowned snapshot -> claim coordinator
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                None,
                false,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        assert_eq!(
            mock.watches(),
            vec![
                ("sess-1".to_string(), false),
                ("sess-1".to_string(), true)
            ]
        );
        assert!(mock.requests().is_empty());

        // Step 3: coordinator granted with branchReportRequired -> send reportBranch
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-aaa"),
                true,
                true,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(report_req) = mock.last_request() else {
            panic!("expected reportBranch request");
        };
        assert_eq!(report_req.operation, "workflowMessage.reportBranch");
        assert_eq!(
            report_req.payload,
            json!({
                "targetSessionId": "sess-1",
                "coordinatorEpoch": "session-epoch-aaa",
                "workflowMessageId": null,
                "piSessionEntryId": null,
                "isIdle": true,
                "hasPendingMessages": false
            })
        );

        // Step 4: reportBranch accepted -> send decision.answer
        mock.respond(&report_req.request_id, &report_req.operation, "accepted", None);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(ans_req) = mock.last_request() else {
            panic!("expected answer request");
        };
        assert_eq!(ans_req.operation, "decision.answer");
        assert_eq!(ans_req.run_id.as_deref(), Some("run-1"));
        assert_eq!(ans_req.expected_revision, Some(1));
        assert!(ans_req.idempotency_key.is_some());
        assert_eq!(
            ans_req.payload.get("coordinatorEpoch"),
            Some(&json!("session-epoch-aaa"))
        );

        // Answer accepted -> Accepted and unwatch_session called
        mock.respond(&ans_req.request_id, &ans_req.operation, "accepted", None);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Accepted);
        assert_eq!(mock.unwatches(), vec!["sess-1".to_string()]);

        // Repeated poll stays Accepted and sends nothing more
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Accepted);
        assert_eq!(mock.requests().len(), 2);
        assert_eq!(mock.unwatches().len(), 1);
    }

    #[test]
    fn answerer_refuses_session_held_by_another_connection() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-live-pi"),
                false,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(
            machine.poll(&mock, t0),
            AnswerProgress::Refused(REFUSED_LIVE_PI_SESSION.to_string())
        );
        // Never claimed, never sent any request, never unwatched
        assert_eq!(mock.watches(), vec![("sess-1".to_string(), false)]);
        assert!(mock.requests().is_empty());
        assert!(mock.unwatches().is_empty());
    }

    #[test]
    fn answerer_uses_already_held_role_without_reclaiming_or_unwatching() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 2,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-existing"),
                true,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        assert_eq!(mock.watches(), vec![("sess-1".to_string(), false)]);
        let Some(ans_req) = mock.last_request() else {
            panic!("expected direct answer request");
        };
        assert_eq!(ans_req.operation, "decision.answer");
        mock.respond(&ans_req.request_id, &ans_req.operation, "accepted", None);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Accepted);
        // Must not unwatch a role this answerer did not claim.
        assert!(mock.unwatches().is_empty());
    }

    #[test]
    fn answerer_retries_on_stale_revision_with_new_ids_then_succeeds() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-1"),
                true,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(req1) = mock.last_request() else {
            panic!("expected first answer");
        };
        assert_eq!(req1.expected_revision, Some(1));

        // Refuse with conflict while snapshot now carries revision 2
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-1"),
                true,
                false,
                pending_interaction_json(2),
            )),
        );
        mock.respond(
            &req1.request_id,
            &req1.operation,
            "conflict",
            Some("Interactive request revision conflict"),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(req2) = mock.last_request() else {
            panic!("expected rebuilt answer");
        };
        assert_eq!(req2.expected_revision, Some(2));
        assert_ne!(req2.idempotency_key, req1.idempotency_key);
        assert_ne!(
            req2.payload.get("submissionId"),
            req1.payload.get("submissionId")
        );

        mock.respond(&req2.request_id, &req2.operation, "accepted", None);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Accepted);
    }

    #[test]
    fn answerer_refuses_when_question_was_already_answered() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-1"),
                true,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(req1) = mock.last_request() else {
            panic!("expected answer");
        };

        // Snapshot shows interaction is now gone
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-1"),
                true,
                false,
                Value::Null,
            )),
        );
        mock.respond(
            &req1.request_id,
            &req1.operation,
            "conflict",
            Some("Another human decision answer already won"),
        );
        assert_eq!(
            machine.poll(&mock, t0),
            AnswerProgress::Refused(REFUSED_ALREADY_ANSWERED.to_string())
        );
    }

    #[test]
    fn answerer_retries_after_connection_lost_with_same_idempotency_key() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        // Claim and send answer on connection 1
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                None,
                false,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-1"),
                true,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(req1) = mock.last_request() else {
            panic!("expected first answer");
        };

        // Connection drops while answer was on the wire
        mock.set_conn_state(ConnState::Reconnecting);
        mock.set_snapshot("sess-1", None);
        mock.respond(&req1.request_id, &req1.operation, CONNECTION_LOST, None);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);

        // Worker reconnects (count 1 -> 2): absorbed as same restart, not double-counted
        mock.set_conn_state(ConnState::Connected);
        mock.set_connection_count(2);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);

        // New snapshot arrives (unowned), re-claim, reportBranch, re-send answer
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                None,
                false,
                false,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-2"),
                true,
                true,
                pending_interaction_json(1),
            )),
        );
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(rep2) = mock.last_request() else {
            panic!("expected reportBranch after reconnect");
        };
        assert_eq!(rep2.operation, "workflowMessage.reportBranch");
        mock.respond(&rep2.request_id, &rep2.operation, "accepted", None);

        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        let Some(req2) = mock.last_request() else {
            panic!("expected retried answer");
        };
        assert_eq!(req2.operation, "decision.answer");
        // Must reuse the exact same idempotency key and submission id!
        assert_eq!(req2.idempotency_key, req1.idempotency_key);
        assert_eq!(
            req2.payload.get("submissionId"),
            req1.payload.get("submissionId")
        );
        assert_eq!(
            req2.payload.get("coordinatorEpoch"),
            Some(&json!("session-epoch-2"))
        );

        // Server deduplicates and returns "adopted"
        mock.respond(&req2.request_id, &req2.operation, "adopted", None);
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Accepted);
        assert_eq!(mock.unwatches(), vec!["sess-1".to_string()]);
    }

    #[test]
    fn answerer_refuses_after_three_connection_loss_restarts() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        // Initial start at connection_count = 1
        assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);

        // 3 restarts allowed (counts 2, 3, 4)
        for next_count in 2..=4 {
            mock.set_connection_count(next_count);
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
        }

        // 4th connection loss exceeds the 3-restart cap -> Refused
        mock.set_connection_count(5);
        match machine.poll(&mock, t0) {
            AnswerProgress::Refused(msg) => {
                assert!(msg.contains("keeps dropping"));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn answerer_enforces_timeouts_without_sleeping() {
        let t0 = Instant::now();

        // 1. Snapshot timeout (10s)
        {
            let mock = MockTransport::new();
            let mut machine = AnswerMachine::new(
                sample_decision_gate(),
                GateAnswer::Choice {
                    index: 0,
                    text: None,
                },
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_secs(9)),
                AnswerProgress::Pending
            );
            match machine.poll(&mock, t0 + Duration::from_secs(10)) {
                AnswerProgress::Refused(msg) => assert!(msg.contains("session snapshot")),
                other => panic!("expected snapshot timeout, got {other:?}"),
            }
        }

        // 2. Coordinator grant timeout (10s) -> unwatches claimed session
        {
            let mock = MockTransport::new();
            let mut machine = AnswerMachine::new(
                sample_decision_gate(),
                GateAnswer::Choice {
                    index: 0,
                    text: None,
                },
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            mock.set_snapshot(
                "sess-1",
                Some(snapshot_with_coord(
                    None,
                    false,
                    false,
                    pending_interaction_json(1),
                )),
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_secs(9)),
                AnswerProgress::Pending
            );
            match machine.poll(&mock, t0 + Duration::from_secs(10)) {
                AnswerProgress::Refused(msg) => assert!(msg.contains("coordinator")),
                other => panic!("expected coordinator timeout, got {other:?}"),
            }
            assert_eq!(mock.unwatches(), vec!["sess-1".to_string()]);
        }

        // 3. Branch report response timeout (30s)
        {
            let mock = MockTransport::new();
            let mut machine = AnswerMachine::new(
                sample_decision_gate(),
                GateAnswer::Choice {
                    index: 0,
                    text: None,
                },
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            mock.set_snapshot(
                "sess-1",
                Some(snapshot_with_coord(
                    None,
                    false,
                    false,
                    pending_interaction_json(1),
                )),
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            mock.set_snapshot(
                "sess-1",
                Some(snapshot_with_coord(
                    Some("session-epoch-1"),
                    true,
                    true,
                    pending_interaction_json(1),
                )),
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_secs(29)),
                AnswerProgress::Pending
            );
            match machine.poll(&mock, t0 + Duration::from_secs(30)) {
                AnswerProgress::Refused(msg) => assert!(msg.contains("branch report")),
                other => panic!("expected branch report timeout, got {other:?}"),
            }
            assert_eq!(mock.unwatches(), vec!["sess-1".to_string()]);
        }

        // 4. Answer response timeout (30s)
        {
            let mock = MockTransport::new();
            let mut machine = AnswerMachine::new(
                sample_decision_gate(),
                GateAnswer::Choice {
                    index: 0,
                    text: None,
                },
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            mock.set_snapshot(
                "sess-1",
                Some(snapshot_with_coord(
                    Some("session-epoch-1"),
                    true,
                    false,
                    pending_interaction_json(1),
                )),
            );
            assert_eq!(machine.poll(&mock, t0), AnswerProgress::Pending);
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_secs(29)),
                AnswerProgress::Pending
            );
            match machine.poll(&mock, t0 + Duration::from_secs(30)) {
                AnswerProgress::Refused(msg) => assert!(msg.contains("gate answer")),
                other => panic!("expected answer timeout, got {other:?}"),
            }
        }
    }

    #[test]
    fn answerer_never_double_sends_across_repeated_polls() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        let t0 = Instant::now();
        let mut machine = AnswerMachine::new(gate, answer);

        // WaitSnapshot repeated polls
        for step in 0..5 {
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_millis(step * 100)),
                AnswerProgress::Pending
            );
        }
        assert_eq!(mock.watches().len(), 1);
        assert_eq!(mock.requests().len(), 0);

        // WaitCoordinator repeated polls
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                None,
                false,
                false,
                pending_interaction_json(1),
            )),
        );
        for step in 0..5 {
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_millis(step * 100)),
                AnswerProgress::Pending
            );
        }
        assert_eq!(mock.watches().len(), 2);
        assert_eq!(mock.requests().len(), 0);

        // WaitBranchReport repeated polls
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                Some("session-epoch-1"),
                true,
                true,
                pending_interaction_json(1),
            )),
        );
        for step in 0..5 {
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_millis(step * 100)),
                AnswerProgress::Pending
            );
        }
        assert_eq!(mock.requests().len(), 1);
        let Some(report_req) = mock.last_request() else {
            panic!("expected reportBranch");
        };
        mock.respond(&report_req.request_id, &report_req.operation, "accepted", None);

        // WaitAnswer repeated polls
        for step in 0..5 {
            assert_eq!(
                machine.poll(&mock, t0 + Duration::from_millis(step * 100)),
                AnswerProgress::Pending
            );
        }
        assert_eq!(mock.requests().len(), 2);
    }

    #[test]
    fn blocking_driver_honors_overall_timeout_and_releases_claim() {
        let mock = MockTransport::new();
        let gate = sample_decision_gate();
        let answer = GateAnswer::Choice {
            index: 0,
            text: None,
        };
        mock.set_snapshot(
            "sess-1",
            Some(snapshot_with_coord(
                None,
                false,
                false,
                pending_interaction_json(1),
            )),
        );

        let clock = Mutex::new(Instant::now());
        let result = run_blocking_with(
            &mock,
            &gate,
            &answer,
            Duration::from_millis(50),
            || *clock.lock().unwrap_or_else(|e| e.into_inner()),
            |dur| {
                let mut guard = clock.lock().unwrap_or_else(|e| e.into_inner());
                *guard += dur;
            },
        );
        assert!(result.is_err());
        assert_eq!(mock.unwatches(), vec!["sess-1".to_string()]);
    }
}
