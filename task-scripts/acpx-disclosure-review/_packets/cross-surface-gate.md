Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For patch tasks, return a complete Git
unified diff against the attached excerpts with exact context and focused
tests. For design tasks, return only the requested Markdown artifact and
use inline code or indented examples inside it. Do not return placeholder code.
Treat the attached excerpts as evidence; lines outside the excerpts are not
available to you. Rust edition 2024, toolchain 1.94.0. No new dependencies,
unsafe, or new production unwrap/expect. Preserve theme styles and keymaps
unless the task explicitly changes them. Do not change gate authority,
workflow state, ledger routing, or release behavior.

=== REPOSITORY CONTEXT (18 excerpts) ===

### `crates/codegen/xai-grok-pager/src/piw/gate.rs` lines 1-110 of 2796 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager/src/piw/gate.rs` lines 289-373 of 2796 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/piw/gate.rs` lines 585-649 of 2796 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/piw/gate.rs` lines 752-813 of 2796 (repository grok-build)
````
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
            // Release the coordinator claim without dropping the shared session
            // watch. unwatch_session clears the snapshot and hides the next gate.
            let _ = client.watch_session(&self.gate.session_id, false);
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 377-538 of 1571 (repository grok-build)
````
    fn watch_open_mirror_run(&mut self, dir: &Path, bundle_run_id: &str) {
        self.open_mirror_run = None;
        let Some(root) = runtime::mirror().map(|m| m.root().to_path_buf()) else {
            return;
        };
        if !mirror::is_mirror_dir(&root, dir) {
            return;
        }
        let run_id = mirror::run_id_of_folder(dir).unwrap_or_else(|| bundle_run_id.to_string());
        if let Some(m) = runtime::mirror() {
            m.want_run(&run_id);
        }
        self.open_mirror_run = Some(run_id);
    }

    /// The run view left its run (back, another run opened, modal closed): a
    /// finished mirror run stops being watched.
    pub fn release_open_mirror_run(&mut self) {
        if let Some(run_id) = self.open_mirror_run.take() {
            if let Some(m) = runtime::mirror() {
                m.release_run(&run_id);
            }
        }
    }

    /// `/acpx cancel` with no flow of ours running: cancel the mirrored run
    /// the run view is open on, through the server.
    pub fn cancel_open_mirror_run(&mut self) -> bool {
        let Some(rv) = self.run_view.as_ref() else {
            return false;
        };
        let dir = rv.run().summary.dir.clone();
        if !rv.run().summary.status.is_live() {
            return false;
        }
        let Some(m) = runtime::mirror() else {
            return false;
        };
        if !mirror::is_mirror_dir(m.root(), &dir) {
            return false;
        }
        let Some(run_id) = mirror::run_id_of_folder(&dir) else {
            return false;
        };
        let request = m
            .client()
            .send("run.cancel", Some(&run_id), None, serde_json::json!({}));
        let _ = request;
        self.notice = Some(format!("{run_id}: cancel sent"));
        true
    }

    /// Drive every in-flight gate answer one step on the tick. True when one
    /// finished or made progress worth showing.
    fn poll_answerers(&mut self) -> bool {
        let mut changed = false;
        let mut any_done = false;
        for answerer in &mut self.answerers {
            match answerer.poll() {
                piw_gate::AnswerProgress::Pending => {}
                piw_gate::AnswerProgress::Accepted => {
                    self.notice = Some("answer accepted; the run continues".to_string());
                    any_done = true;
                    changed = true;
                }
                piw_gate::AnswerProgress::Refused(message) => {
                    self.notice = Some(message);
                    any_done = true;
                    changed = true;
                }
            }
        }
        if any_done {
            self.answerers.clear();
        }
        changed
    }

    /// A chosen gate label whose "flow file" is a pi-workflows answer target
    /// (`piw-answer://<run>/<choice>`): answer it in place instead of starting
    /// a flow. The answer itself is a `GateAnswerer` polled on the tick.
    fn answer_mirror_gate(&mut self, run_id: &str, choice_label: &str, choice_key: &str) -> AcpxOutcome {
        let Some(m) = runtime::mirror() else {
            self.notice = Some("no workflow server is running; answer there instead".to_string());
            return AcpxOutcome::Changed;
        };
        let Some(gate) = m.gate(run_id) else {
            self.notice = Some("this run is not parked on a question the mirror can see".to_string());
            return AcpxOutcome::Changed;
        };
        let Some(index) = gate.choices.iter().position(|c| c.key == choice_key) else {
            self.notice = Some(format!("choice `{choice_key}` is not one this run waits on"));
            return AcpxOutcome::Changed;
        };
        let answer = PiwGateAnswer::Choice {
            index,
            text: None,
        };
        self.answerers.push(GateAnswerer::new(
            std::sync::Arc::clone(m.client()),
            gate,
            answer,
        ));
        self.notice = Some(format!("{choice_label}: answer sent"));
        AcpxOutcome::Changed
    }

    fn handle_run_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some(rv) = self.run_view.as_mut() else {
            self.mode = Mode::Ledger;
            return AcpxOutcome::Changed;
        };
        match rv.handle_key(key) {
            RunOutcome::Unchanged => AcpxOutcome::Unchanged,
            RunOutcome::Changed => AcpxOutcome::Changed,
            RunOutcome::Back => {
                self.mode = Mode::Ledger;
                self.run_view = None;
                self.inspector = None;
                self.release_open_mirror_run();
                self.rescan_runs();
                AcpxOutcome::Changed
            }
            RunOutcome::OpenStep(idx) => {
                if self.open_step_at(idx) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Replay(idx) => self.replay_step(idx),
            RunOutcome::Copy(text) => AcpxOutcome::Copy(text),
            RunOutcome::ScrollDetail(delta) => {
                if self.detail.scroll(delta) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Gate { choice, flow_file } => {
                // A pi-workflows gate answers in place instead of starting a
                // flow; the target is checked before any relative-path join.
                if let Some((run_id, choice_key)) = piw_gate::parse_answer_target(&flow_file) {
                    return self.answer_mirror_gate(&run_id, &choice, &choice_key);
                }
                let run = rv.run();
                // A relative flow file is beside the flow that declared it.
                let declared = PathBuf::from(&flow_file);
                let flow_file = match bundle::flow_path(&run.summary.dir).as_deref().and_then(Path::parent) {
                    Some(dir) if declared.is_relative() => dir.join(declared),
                    _ => declared,
                };
                let input_json = bundle::gate_input(run, &choice).to_string();
                // Back to the list, where the new run appears.
                self.notice = Some(format!("{choice}: started {}", flow_file.display()));
                self.mode = Mode::Ledger;
                self.run_view = None;
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1464-1563 of 1571 (repository grok-build)
````
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A waiting run whose gate choice carries a pi-workflows answer target as
    /// its "flow file".
    fn write_piw_gate_fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_base = tmp.path().join("runs");
        let run_dir = runs_base.join("run-001");
        let proj = run_dir.join("projections");
        fs::create_dir_all(&proj).expect("create projections");
        fs::write(
            proj.join("live.json"),
            r#"{
                "runId": "run-001",
                "flowName": "piw-demo",
                "runTitle": "mixed demo",
                "status": "waiting",
                "startedAt": "2025-03-01T10:00:00Z",
                "updatedAt": "2025-03-01T10:01:00Z",
                "currentNode": "review",
                "waitingOn": "review"
            }"#,
        )
        .expect("write live.json");
        fs::write(
            run_dir.join("flow.json"),
            r#"{
                "name": "piw-demo",
                "startAt": "plan",
                "nodes": {
                    "plan": { "nodeType": "acp", "summary": "Draft plan" },
                    "review": { "nodeType": "checkpoint", "summary": "Decide" }
                },
                "edges": [ { "from": "plan", "to": "review" } ]
            }"#,
        )
        .expect("write flow.json");
        fs::write(
            proj.join("steps.json"),
            r#"[
                {
                    "attemptId": "review#1",
                    "nodeId": "review",
                    "nodeType": "acp",
                    "outcome": "ok",
                    "startedAt": "2025-03-01T10:00:30Z",
                    "finishedAt": "2025-03-01T10:00:40Z",
                    "output": {
                        "next": {
                            "Approve": "piw-answer://run-001/approve"
                        }
                    }
                }
            ]"#,
        )
        .expect("write steps.json");
        (tmp, runs_base)
    }

    #[test]
    fn answer_target_is_parsed_and_routed_not_launched() {
        let (_tmp, runs_base) = write_piw_gate_fixture();
        let run_dir = runs_base.join("run-001");

        // The target on the fixture's choice is a real answer target.
        assert_eq!(
            crate::piw::gate::parse_answer_target("piw-answer://run-001/approve"),
            Some(("run-001".to_string(), "approve".to_string()))
        );

        let mut state = AcpxModalState::open_at(Some(runs_base), None);
        assert!(state.open_run(&run_dir), "the mirrored-format folder opens");

        // 1 selects the gate choice, Enter confirms it.
        assert_eq!(state.handle_key(&key(KeyCode::Char('1'))), AcpxOutcome::Changed);
        let outcome = state.handle_key(&key(KeyCode::Enter));
        assert!(
            !matches!(outcome, AcpxOutcome::RunFlow { .. }),
            "an answer target must not launch a flow, got {outcome:?}"
        );
        assert_eq!(outcome, AcpxOutcome::Changed);
        // No mirror is running in this test process: the answer is refused
        // locally with a notice, exactly as with no server present.
        assert!(
            state
                .notice()
                .is_some_and(|n| n.contains("no workflow server is running")),
            "unexpected notice: {:?}",
            state.notice()
        );

        // Leaving the run is safe with no mirror running.
        state.release_open_mirror_run();
        assert!(state.answerers.is_empty());
    }

    #[test]
    fn cancel_open_mirror_run_without_mirror_is_a_no_op() {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 1143-1195 of 2758 (repository grok-build)
````

    // ---- Rendering helpers ----

    fn render_header(&self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        let status = if self.run.summary.status == RunStatus::Waiting && self.answered.is_some() {
            RunStatus::Completed
        } else {
            self.run.summary.status
        };
        let status_color = status_color(status, theme);
        let bold = Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(theme.gray);
        let sep = Style::default().fg(theme.gray_dim);

        let mut spans = vec![
            Span::styled(
                format!("{} ", status.glyph()),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(self.run.summary.title.clone(), bold),
        ];

        let mut facts: Vec<String> = Vec::new();
        if !self.run.summary.flow_name.is_empty()
            && !self
                .run
                .summary
                .title
                .starts_with(&self.run.summary.flow_name)
        {
            facts.push(self.run.summary.flow_name.clone());
        }
        let (done, total) = self.run.progress();
        facts.push(format!("{done}/{total} nodes"));
        facts.push(fmt_duration(self.run.duration_ms(now_ms)));
        if !status.is_live() {
            let ref_ms = self
                .run
                .summary
                .finished_ms
                .unwrap_or(self.run.summary.updated_ms);
            if ref_ms > 0 && now_ms >= ref_ms {
                facts.push(fmt_age(now_ms - ref_ms));
            }
        }

        for fact in facts {
            spans.push(Span::styled(" · ", sep));
            spans.push(Span::styled(fact, dim));
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 244-271 of 2698 (repository grok-build)
````
fn build_run_lines(
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let status = run.summary.status;
    // A waiting run whose gate is answered reads as done, like the run header.
    let shown = if status == RunStatus::Waiting && answered.is_some() {
        RunStatus::Completed
    } else {
        status
    };
    let status_color = run_status_color(shown, theme);
    let glyph = shown.glyph();
    let title = if run.summary.title.is_empty() {
        &run.summary.run_id
    } else {
        &run.summary.title
    };
    let head = truncate_str(&format!("{glyph} {title}"), width);
    out.push(Line::from( vec![
        Span::styled(
            format!("{glyph} "),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 310-389 of 2990 (repository grok-build)
````
    fn is_stalled(&self, run: &RunSummary) -> bool {
        self.extras
            .get(&run.run_id)
            .map(|e| e.stalled)
            .unwrap_or(false)
    }

    fn is_answered(&self, run: &RunSummary) -> bool {
        run.status == RunStatus::Waiting && self.answered.contains_key(&run.run_id)
    }

    fn active_filter(&self) -> &str {
        match self.tab {
            LedgerTab::Flows => &self.flow_filter,
            LedgerTab::Sessions => &self.session_filter,
        }
    }

    fn active_filter_mut(&mut self) -> &mut String {
        match self.tab {
            LedgerTab::Flows => &mut self.flow_filter,
            LedgerTab::Sessions => &mut self.session_filter,
        }
    }

    fn is_run_active(&self, run: &RunSummary) -> bool {
        matches!(run.status, RunStatus::Running | RunStatus::Waiting)
            && !self.is_stalled(run)
            && !self.is_answered(run)
    }

    fn matches_flow_filter(&self, run: &RunSummary) -> bool {
        if self.flow_active_only && !self.is_run_active(run) {
            return false;
        }
        let q = self.flow_filter.trim();
        if q.is_empty() {
            return true;
        }
        let q_lower = q.to_lowercase();
        let agents = self
            .extras
            .get(&run.run_id)
            .map(|e| e.agents.as_str())
            .unwrap_or("");
        run.title.to_lowercase().contains(&q_lower)
            || run.flow_name.to_lowercase().contains(&q_lower)
            || run.status.label().to_lowercase().contains(&q_lower)
            || run.run_id.to_lowercase().contains(&q_lower)
            || agents.to_lowercase().contains(&q_lower)
    }

    fn matches_session_filter(&self, s: &SessionSummary) -> bool {
        if self.session_active_only && s.closed {
            return false;
        }
        let q = self.session_filter.trim();
        if q.is_empty() {
            return true;
        }
        let q_lower = q.to_lowercase();
        s.title.to_lowercase().contains(&q_lower)
            || s.agent.to_lowercase().contains(&q_lower)
            || s.cwd.to_lowercase().contains(&q_lower)
            || s.name.to_lowercase().contains(&q_lower)
    }

    fn run_section(&self, run: &RunSummary) -> SectionKind {
        let stalled = self.is_stalled(run);
        if run.status == RunStatus::Waiting && !self.is_answered(run) {
            SectionKind::NeedsYou
        } else if run.status == RunStatus::Running && !stalled {
            SectionKind::Running
        } else {
            SectionKind::Finished
        }
    }

    /// Returns indices into `self.runs` in display order.
    fn filtered_flow_indices(&self) -> Vec<usize> {
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 679-780 of 2983 (repository grok-build)
````
    async fn pump_piw_events(&self, session_id: &str, timing: &TurnTiming) {
        let (mut rx, mut projector) = {
            let mut st = self.state.borrow_mut();
            let Some(watch) = st
                .sessions
                .get_mut(session_id)
                .and_then(|sess| sess.piw.as_mut())
            else {
                return;
            };
            let Some(rx) = watch.rx.take() else {
                return;
            };
            let projector = watch
                .projector
                .take()
                .unwrap_or_else(|| Projector::new(session_id));
            (rx, projector)
        };
        let mut parked = false;
        // Once the turn is cancelled, the run's final state gets this long to
        // arrive so the chat ends on the run cancelled, not on a timer.
        let mut cancel_grace_until: Option<Instant> = None;

        loop {
            let ev = tokio::select! {
                ev = rx.recv() => match ev {
                    Some(ev) => ev,
                    None => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(1000)) => {
                    // A liveness guard: a cancelled turn must end even if the
                    // server never answers. The run's own cancelled state
                    // usually arrives first; give it a moment before giving up.
                    if self.is_session_cancelled(session_id) {
                        let until = *cancel_grace_until.get_or_insert_with(|| {
                            Instant::now() + Duration::from_secs(5)
                        });
                        if Instant::now() >= until {
                            break;
                        }
                    }
                    continue;
                }
            };

            let waiting = if let DriverEvent::Waiting {
                node,
                question,
                choices,
                ..
            } = &ev
            {
                Some((node.clone(), question.clone(), choices.clone()))
            } else {
                None
            };

            let outs = projector.on_event(&ev);
            for out in outs {
                self.send_out(session_id, out, Some(timing), false, None).await;
            }

            if matches!(ev, DriverEvent::RunFinished { .. }) {
                break;
            }

            // Issue 11: park immediately. The inline card is already in the
            // transcript; do not open the auto-pop-up question modal.
            if let Some((node, question, choices)) = waiting {
                let gate = {
                    let st = self.state.borrow();
                    st.sessions
                        .get(session_id)
                        .and_then(|sess| sess.piw.as_ref())
                        .and_then(|watch| watch.handle.gate())
                };
                self.remember_inline_waiting(session_id, gate, node, question, choices);
                parked = true;
                break;
            }
        }

        let mut parked_run_id: Option<String> = None;
        let mut st = self.state.borrow_mut();
        if let Some(sess) = st.sessions.get_mut(session_id) {
            if parked {
                if let Some(watch) = sess.piw.as_mut() {
                    watch.rx = Some(rx);
                    watch.projector = Some(projector);
                    parked_run_id = Some(watch.run_id.clone());
                }
            } else {
                // The run is final; the watch task ends by itself.
                sess.piw = None;
            }
        }
        drop(st);
        // A parked run can still be cancelled or answered from elsewhere (the
        // /acpx modal, another tab). Watch its server state so the chat ends
        // on the run's final state instead of the stale waiting hint.
        if let Some(run_id) = parked_run_id {
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 895-1049 of 2983 (repository grok-build)
````
            .await;

        match ask {
            GateAsk::Withdrawn => {
                // The pager withdrew the question because the turn is being
                // cancelled. Cancel the run now and keep pumping until its
                // final state ends the turn, so the chat shows the run
                // cancelled instead of a stale waiting hint.
                self.piw_gate_withdrawn(session_id)
            }
            GateAsk::Dismissed => {
                // The pager answers an Esc dismissal and a Ctrl+C withdrawal
                // of the turn with the same `outcome: cancelled` response; the
                // session cancel follows a beat later only in the Ctrl+C
                // case. Give it that beat before parking, so a run cancelled
                // with the turn ends on its cancelled state.
                let withdrawn = self.is_session_cancelled(session_id) || {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    self.is_session_cancelled(session_id)
                };
                if withdrawn {
                    return self.piw_gate_withdrawn(session_id);
                }
                // The question was dismissed; the run stays parked and answerable.
                self.remember_waiting_piw(session_id, gate.clone());
                let answer_hint = parked_gate_hint(&node, choices);
                self.send_parent_text(session_id, answer_hint, timing).await;
                WaitingOutcome::Parked
            }
            GateAsk::Answered(typed) => {
                if self.is_session_cancelled(session_id) {
                    self.remember_waiting_piw(session_id, gate);
                    return WaitingOutcome::Parked;
                }
                if self.answer_piw_gate(session_id, gate.clone(), &typed, timing).await {
                    let mut st = self.state.borrow_mut();
                    if let Some(sess) = st.sessions.get_mut(session_id) {
                        sess.waiting_piw = None;
                    }
                    WaitingOutcome::Resumed
                } else {
                    self.remember_waiting_piw(session_id, gate);
                    WaitingOutcome::Parked
                }
            }
        }
    }

    /// The turn is being cancelled with the gate question withdrawn: cancel
    /// the run and keep pumping until its final state ends the turn.
    fn piw_gate_withdrawn(&self, session_id: &str) -> WaitingOutcome {
        {
            let st = self.state.borrow();
            if let Some(sess) = st.sessions.get(session_id)
                && let Some(watch) = sess.piw.as_ref()
            {
                watch.handle.cancel();
            }
        }
        WaitingOutcome::Cancelled
    }

    fn remember_waiting_piw(&self, session_id: &str, gate: Option<PiwGate>) {
        let Some(gate) = gate else { return };
        self.store_waiting_piw(session_id, WaitingPiwGate::from_gate(gate));
    }

    fn remember_inline_waiting(
        &self,
        session_id: &str,
        gate: Option<PiwGate>,
        node: String,
        question: String,
        choices: Vec<GateChoice>,
    ) {
        self.store_waiting_piw(
            session_id,
            WaitingPiwGate::from_event(gate, node, question, choices),
        );
    }

    fn store_waiting_piw(&self, session_id: &str, waiting: WaitingPiwGate) {
        let mut st = self.state.borrow_mut();
        if let Some(sess) = st.sessions.get_mut(session_id) {
            sess.waiting_piw = Some(waiting);
        }
    }

    /// Send the answer through the workflow server, off this task's thread.
    /// True when the server accepted it and the run resumes.
    async fn answer_piw_gate(
        &self,
        session_id: &str,
        gate: Option<PiwGate>,
        typed: &str,
        timing: &TurnTiming,
    ) -> bool {
        let Some(gate) = gate else {
            self.send_parent_text(
                session_id,
                "The workflow server is not showing this question right now; answer it in Pi.",
                timing,
            )
            .await;
            return false;
        };
        // Answering a parked run needs the server; start one only when no
        // server answers, then the shared client (re)connects to it.
        if let Err(message) = self.ensure_piw_server(session_id, timing).await {
            self.send_parent_text(session_id, message, timing)
                .await;
            return false;
        }
        let Some(client) = piw_shared_client() else {
            self.send_parent_text(
                session_id,
                "The pi-workflows server has gone away; answer the question in Pi.",
                timing,
            )
            .await;
            return false;
        };
        let answer = match piw_gate::parse_answer(&gate, typed) {
            Ok(answer) => answer,
            Err(message) => {
                self.send_parent_text(session_id, message, timing).await;
                return false;
            }
        };
        let gate_for_task = gate.clone();
        let answer_for_task = answer.clone();
        let result = tokio::task::spawn_blocking(move || {
            piw_gate::answer_blocking(&client, &gate_for_task, &answer_for_task, Duration::from_secs(60))
        })
        .await;
        match result {
            Ok(Ok(())) => true,
            Ok(Err(message)) => {
                self.send_parent_text(
                    session_id,
                    format!("The answer was not delivered: {message}"),
                    timing,
                )
                .await;
                false
            }
            Err(err) => {
                self.send_parent_text(
                    session_id,
                    format!("Answering the workflow gate failed: {err}"),
                    timing,
                )
                .await;
                false
            }
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1257-1350 of 2983 (repository grok-build)
````
    async fn replay_run_into_session(
        &self,
        session_id: &str,
        run_dir: &Path,
        is_replay: bool,
        timing: Option<&TurnTiming>,
    ) -> anyhow::Result<()> {
        let events = driver::replay(run_dir)?;
        let mut projector = Projector::new(session_id);
        let mut run_id = run_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("run")
            .to_string();
        let mut flow_path = String::new();
        let mut waiting: Option<(String, String, String, Vec<GateChoice>)> = None;
        let mut finished_status = String::new();

        for ev in &events {
            let at_ms = match ev {
                DriverEvent::RunStarted {
                    run_id: rid,
                    flow_path: fp,
                    at_ms,
                    ..
                } => {
                    if !rid.is_empty() {
                        run_id = rid.clone();
                    }
                    if !fp.is_empty() {
                        flow_path = fp.clone();
                    }
                    Some(*at_ms)
                }
                DriverEvent::StepStarted { at_ms, .. }
                | DriverEvent::AgentPrompt { at_ms, .. }
                | DriverEvent::AgentUpdate { at_ms, .. }
                | DriverEvent::ShellPrepared { at_ms, .. }
                | DriverEvent::StepFinished { at_ms, .. } => Some(*at_ms),
                DriverEvent::Waiting {
                    node,
                    attempt,
                    question,
                    choices,
                    at_ms,
                } => {
                    if !choices.is_empty() {
                        waiting = Some((
                            node.clone(),
                            attempt.clone(),
                            question.clone(),
                            choices.clone(),
                        ));
                    }
                    Some(*at_ms)
                }
                DriverEvent::RunFinished { status, at_ms, .. } => {
                    finished_status = status.clone();
                    Some(*at_ms)
                }
            };

            let outs = projector.on_event(ev);
            for out in outs {
                self.send_out(
                    session_id,
                    out,
                    timing,
                    is_replay,
                    if is_replay { at_ms } else { None },
                )
                .await;
            }
        }

        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                sess.shown_runs.insert(run_id.clone());
                if let Some((node, attempt, question, choices)) = waiting
                    && (finished_status.is_empty() || finished_status == "waiting")
                {
                    sess.waiting_gate = Some(WaitingGate {
                        run_id,
                        run_dir: run_dir.to_path_buf(),
                        node,
                        attempt,
                        question,
                        choices,
                        flow_path,
                    });
                }
            }
        }
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1839-1957 of 2983 (repository grok-build)
````
                let default_flow = self.default_flow();
                self.send_parent_text(&session_id, help_markdown(default_flow.as_deref()), &timing)
                    .await;
            }
            Command::Runs => {
                let runs_dir = self.runs_dir();
                let runs = driver::list_runs(&runs_dir, 20);
                self.send_parent_text(&session_id, format_runs_table(&runs), &timing)
                    .await;
            }
            Command::Open { run_id } => {
                let already_shown = self
                    .state
                    .borrow()
                    .sessions
                    .get(&session_id)
                    .is_some_and(|s| s.shown_runs.contains(&run_id));
                if already_shown {
                    self.send_parent_text(&session_id, "Already shown above.", &timing)
                        .await;
                } else {
                    let runs_dir = self.runs_dir();
                    match resolve_run_dir(&runs_dir, &run_id) {
                        Some(run_dir) => {
                            if let Err(err) = self
                                .replay_run_into_session(&session_id, &run_dir, false, Some(&timing))
                                .await
                            {
                                self.send_parent_text(
                                    &session_id,
                                    format!("Failed to open run `{run_id}`: {err}"),
                                    &timing,
                                )
                                .await;
                            }
                        }
                        None => {
                            self.send_parent_text(
                                &session_id,
                                format!("Run `{run_id}` not found."),
                                &timing,
                            )
                            .await;
                        }
                    }
                }
            }
            Command::Answer { label } => {
                let (waiting_gate, waiting_piw) = {
                    let st = self.state.borrow();
                    let sess = st.sessions.get(&session_id);
                    (
                        sess.and_then(|s| s.waiting_gate.clone()),
                        sess.and_then(|s| s.waiting_piw.clone()),
                    )
                };
                if let Some(waiting) = waiting_piw {
                    // A parked pi-workflows run: the answer goes back through
                    // the server and the same run resumes.
                    if self
                        .answer_piw_gate(&session_id, waiting.gate.clone(), &label, &timing)
                        .await
                    {
                        {
                            let mut st = self.state.borrow_mut();
                            if let Some(sess) = st.sessions.get_mut(&session_id) {
                                sess.waiting_piw = None;
                            }
                        }
                        self.pump_piw_events(&session_id, &timing).await;
                        ran_flow = true;
                    }
                } else {
                    match waiting_gate {
                        None => {
                            self.send_parent_text(&session_id, "No gate is waiting.", &timing)
                                .await;
                        }
                        Some(gate) => {
                        let matched = gate
                            .choices
                            .iter()
                            .any(|c| c.label.trim().eq_ignore_ascii_case(label.trim()));
                        if !matched {
                            let list = gate
                                .choices
                                .iter()
                                .map(|c| c.label.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.send_parent_text(
                                &session_id,
                                format!("Choices: {list}"),
                                &timing,
                            )
                            .await;
                        } else {
                            {
                                let mut st = self.state.borrow_mut();
                                if let Some(sess) = st.sessions.get_mut(&session_id) {
                                    sess.waiting_gate = None;
                                }
                            }
                            if let Some((next_flow, next_input)) = self
                                .prepare_chained_run(&session_id, &gate, &label, &timing)
                                .await
                            {
                                self.run_flow_chain(&session_id, next_flow, next_input, &timing)
                                    .await;
                                ran_flow = true;
                            } else {
                                self.remember_waiting_gate(&session_id, gate);
                            }
                        }
                        }
                    }
                }
            }
            Command::Flow { file, input } => {
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 2099-2158 of 2983 (repository grok-build)
````
pub fn parse_command(text: &str, waiting_labels: &[String]) -> Command {
    let trimmed = text.trim();
    if let Some(after_slash) = trimmed.strip_prefix('/') {
        let (cmd_word, rest) = split_first_word(after_slash);
        let cmd_lower = cmd_word.to_ascii_lowercase();
        match cmd_lower.as_str() {
            "flow" => {
                let Some((file, after_file)) = parse_flow_file_and_rest(rest) else {
                    return Command::Help;
                };
                if file.is_empty() {
                    return Command::Help;
                }
                let input = parse_flow_input(after_file);
                Command::Flow { file, input }
            }
            "runs" => {
                if rest.trim().is_empty() {
                    Command::Runs
                } else {
                    Command::Help
                }
            }
            "open" => {
                let run_id = unquote_arg(rest.trim());
                if run_id.is_empty() {
                    Command::Help
                } else {
                    Command::Open { run_id }
                }
            }
            "answer" => {
                let label = unquote_arg(rest.trim());
                if label.is_empty() {
                    Command::Help
                } else {
                    let resolved = waiting_labels
                        .iter()
                        .find(|w| w.trim().eq_ignore_ascii_case(&label))
                        .cloned()
                        .unwrap_or(label);
                    Command::Answer { label: resolved }
                }
            }
            "acpx-help" => Command::Help,
            _ => Command::Help,
        }
    } else {
        if !trimmed.is_empty()
            && let Some(matched) = waiting_labels
                .iter()
                .find(|w| w.trim().eq_ignore_ascii_case(trimmed))
        {
            return Command::Answer {
                label: matched.clone(),
            };
        }
        Command::Task {
            text: trimmed.to_string(),
        }
````

### `tui/src/ui/run_command.rs` lines 1-85 of 351 (repository pi-workflows)
````
use crate::format::sanitize_text;
use crate::theme::Palette;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOp {
    Pause,
    Resume,
    Cancel,
}

impl RunOp {
    pub fn operation(self) -> &'static str {
        match self {
            RunOp::Pause => "run.pause",
            RunOp::Resume => "run.resume",
            RunOp::Cancel => "run.cancel",
        }
    }

    pub fn busy(self) -> &'static str {
        match self {
            RunOp::Pause => "pausing…",
            RunOp::Resume => "resuming…",
            RunOp::Cancel => "cancelling…",
        }
    }

    pub fn done(self) -> &'static str {
        match self {
            RunOp::Pause => "paused",
            RunOp::Resume => "resumed",
            RunOp::Cancel => "cancelled",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            RunOp::Pause => "pause",
            RunOp::Resume => "resume",
            RunOp::Cancel => "cancel",
        }
    }
}

pub fn toggle_op(controls: &[String]) -> Option<RunOp> {
    if controls.iter().any(|c| c == "resume") {
        Some(RunOp::Resume)
    } else if controls.iter().any(|c| c == "pause") {
        Some(RunOp::Pause)
    } else {
        None
    }
}

pub fn no_control_notice(status: &str, controls: &[String]) -> String {
    let clean_status = sanitize_text(status);
    let status_label = if clean_status.is_empty() {
        "in its current state".to_string()
    } else {
        clean_status
    };
    if controls.iter().any(|c| c == "answer") {
        format!(
            "cannot pause or resume ({status_label}); checkpoint must be answered inside the owning Pi session, not from piw"
        )
    } else {
        format!("cannot pause or resume ({status_label})")
    }
}

pub fn outcome_notice(op: RunOp, outcome: &str, error: Option<&str>) -> String {
    let clean_outcome = sanitize_text(outcome);
    if clean_outcome == "accepted" || clean_outcome == "adopted" {
        return op.done().to_string();
    }
    let verb = op.verb();
    let outcome_label = if clean_outcome.is_empty() {
        "failed".to_string()
    } else {
````

### `tui/src/ui/run_command.rs` lines 177-215 of 351 (repository pi-workflows)
````
    fn toggle_op_for_each_server_controls_configuration() {
        // running or waiting
        assert_eq!(
            toggle_op(&vec_str(&["pause", "cancel"])),
            Some(RunOp::Pause)
        );
        // pending checkpoint
        assert_eq!(
            toggle_op(&vec_str(&["pause", "cancel", "answer"])),
            Some(RunOp::Pause)
        );
        // pending decision
        assert_eq!(
            toggle_op(&vec_str(&["pause", "cancel", "human-answer"])),
            Some(RunOp::Pause)
        );
        // pending agent request
        assert_eq!(
            toggle_op(&vec_str(&["pause", "cancel", "update", "submit"])),
            Some(RunOp::Pause)
        );
        // paused or parked queue entry
        assert_eq!(
            toggle_op(&vec_str(&["resume", "cancel"])),
            Some(RunOp::Resume)
        );
        // resume wins over pause if both appear
        assert_eq!(
            toggle_op(&vec_str(&["pause", "resume", "cancel"])),
            Some(RunOp::Resume)
        );
        // queued (unparked) or terminal with active origin turn
        assert_eq!(toggle_op(&vec_str(&["cancel"])), None);
        // completed / failed / timed_out
        assert_eq!(toggle_op(&vec_str(&[])), None);
        // ambiguous run or unknown future controls
        assert_eq!(toggle_op(&vec_str(&["review", "future-op"])), None);
    }

````

### `tui/src/render.rs` lines 271-416 of 1552 (repository pi-workflows)
````
    let Some(local_node_id) = node.get("localNodeId").and_then(Value::as_str) else {
        return sanitize_text(node_id);
    };
    match node.get("includeTransition").and_then(Value::as_str) {
        Some("entry") => format!("{path} · enter"),
        Some("exit") => format!("{path} · {} exit", sanitize_text(local_node_id)),
        _ => format!("{path} › {}", sanitize_text(local_node_id)),
    }
}

fn card_shape(view: &GraphView, node_id: &str) -> CardShape {
    let node = view
        .snapshot
        .and_then(|snapshot| snapshot.nodes.get(node_id));
    let labels = node_branch_labels(view, node_id);
    let branch_rows = labels.len().min(CARD_MAX_BRANCH_ROWS);
    let node_type = node
        .and_then(|value| value.get("nodeType"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let action_execution = node
        .and_then(|value| value.get("actionExecution"))
        .and_then(Value::as_str);
    let type_badge = node_type_badge(node_type, action_execution);
    let mut candidates = vec![
        hierarchical_node_label(node_id, node),
        type_badge.clone(),
        format!("{type_badge} {CARD_WIDEST_STATUS_BADGE}"),
    ];
    if let Some(audience) = node
        .and_then(|value| value.pointer("/humanDecision/audience"))
        .and_then(Value::as_str)
    {
        candidates.push(format!("… human decision · {audience}"));
    }
    let configured_detail = node
        .and_then(|value| value.get("statusDetail").or_else(|| value.get("summary")))
        .and_then(Value::as_str);
    let assistant_detail = (node
        .filter(|value| value.get("nodeType").and_then(Value::as_str) == Some("agent"))
        .and_then(|value| value.pointer("/expectedOutput/kind"))
        .and_then(Value::as_str)
        == Some("assistant-message"))
    .then_some("assistant response");
    let detail = [assistant_detail, configured_detail]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    if !detail.is_empty() {
        candidates.push(format!("… {detail}"));
    }
    candidates.extend(bounded_branch_lines(&labels));
    let content_width = candidates
        .iter()
        .map(|value| display_width(value))
        .max()
        .unwrap_or(CARD_MIN_CONTENT_WIDTH)
        .clamp(CARD_MIN_CONTENT_WIDTH, CARD_MAX_CONTENT_WIDTH);
    CardShape {
        width: content_width + 4,
        height: CARD_CORE_HEIGHT + branch_rows as i64,
    }
}

fn bounded_node_label(node_id: &str, node: Option<&Value>, content_width: i64) -> String {
    let full = hierarchical_node_label(node_id, node);
    if display_width(&full) <= content_width {
        return full;
    }
    let local = node
        .and_then(|value| value.get("localNodeId"))
        .and_then(Value::as_str)
        .map(sanitize_text)
        .unwrap_or_else(|| sanitize_text(node_id));
    let suffix = match node
        .and_then(|value| value.get("includeTransition"))
        .and_then(Value::as_str)
    {
        Some("entry") => "enter".to_string(),
        Some("exit") => format!("{local} exit"),
        _ => local,
    };
    let candidate = format!("… › {suffix}");
    if display_width(&candidate) <= content_width {
        candidate
    } else {
        fit_text(&suffix, content_width as usize)
    }
}

fn bounded_branch_lines(labels: &[String]) -> Vec<String> {
    match labels.len() {
        0..=CARD_MAX_BRANCH_ROWS => labels.iter().map(|label| format!("◇ {label}")).collect(),
        count => vec![
            format!("◇ {}", labels[0]),
            format!("◇ {}", labels[1]),
            format!("+{} branches", count - 2),
        ],
    }
}

fn human_decision_detail(state: &RunState, node_id: &str, node: Option<&Value>) -> Option<String> {
    let human = node?.get("humanDecision")?;
    let choices = human.get("choices")?.as_object()?;
    if state.waiting_on.as_deref() == Some(node_id) {
        let audience = state
            .final_output
            .as_ref()
            .and_then(|request| request.get("audience"))
            .and_then(Value::as_str)
            .or_else(|| human.get("audience").and_then(Value::as_str))
            .unwrap_or("operator");
        return Some(format!("human decision · {}", sanitize_text(audience)));
    }
    state
        .human_decision
        .as_ref()
        .filter(|decision| decision.get("nodeId").and_then(Value::as_str) == Some(node_id))
        .and_then(|decision| decision.pointer("/response/choice"))
        .and_then(Value::as_str)
        .and_then(|choice| choices.get(choice))
        .and_then(|choice| choice.get("label"))
        .and_then(Value::as_str)
        .map(|label| format!("human: {}", sanitize_text(label)))
}

struct RenderedCell {
    cell: GraphCell,
    text: String,
    node_id: String,
    node_type: String,
    type_badge: String,
    status: Option<NodeStatus>,
    attempts: usize,
    elapsed: String,
    detail: String,
    branch_lines: Vec<String>,
    is_start: bool,
    is_end: bool,
    width: i64,
    height: i64,
}

fn render_cell_text(
    view: &GraphView,
````

### `tui/src/protocol.rs` lines 1-34 of 535 (repository pi-workflows)
````
//! Strict version-1 client envelopes shared by local and remote piw transports.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_ID: &str = "pi-workflows.client.v1";

const OPERATIONS: &[&str] = &[
    "run.start",
    "run.pause",
    "run.resume",
    "run.cancel",
    "run.status",
    "run.list",
    "checkpoint.answer",
    "decision.answer",
    "interaction.submit",
    "interaction.update",
    "notification.claim",
    "notification.deliver",
    "turn.claim",
    "turn.resolve",
    "resourceManager.list",
    "resourceManager.get",
    "resourceManager.apply",
    "resourceManager.reconcile",
    "resourceManager.delete",
    "server.status",
    "server.stop",
    "view.runs.watch",
    "view.runs.page",
    "view.run.get",
    "view.run.watch",
    "view.run.unwatch",
````

=== TASK SPECIFICATION ===

Slice ID: cross-surface-gate

Produce an implementation contract and operator walkthrough for the same
existing PIW run in Grok /acpx, standalone PIW, and Grok ACPX chat. No patch
in this packet: cross-repo ownership and the live fixture are review gates.
Treat the engine's request and run state as authoritative. Quote the exact
source functions that support each observed behavior; label anything the
excerpts cannot establish as a verification item. Do not assume standalone
PIW can submit a gate because the protocol lists an operation.

Provide a per-surface read/submit matrix and one transition table covering
pending, submitting, accepted while the engine is still waiting, next live
state, terminal, and stale/rejected. Distinguish checkpoint input from a
protected decision and the PIW route from native ACPX replay; note which
current functions would need changes and which repo owns them. Separate gate
inspection, an explicit answer, and native flow launch. Bare /answer may
reveal choices and must never submit, cancel or start a run. No speculative
cause for the earlier cancellation report. Indicate how to recover after an
uncertain submission by reconciling server state before any resend.

Specify a fresh parked fixture with at least one meaningful next step and an
observable alternate branch. Give a walkthrough that inspects one run on all
three surfaces, compares state before and after inspection and bare /answer,
submits exactly one allowed answer through an authorized surface, and checks
that all three show the engine's branch and next step without reopening if
live watching is supported. Add a separate stale/rejected test. Include
exact expected evidence and mark any commands needing confirmation rather
than guessing. Do not answer a real protected decision or run a workflow.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-returns/cross-surface-gate.md ===
```markdown
<complete Markdown artifact>
```
