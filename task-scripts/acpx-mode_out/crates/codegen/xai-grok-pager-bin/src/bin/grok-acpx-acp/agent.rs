use crate::driver::{self, DriverConfig, DriverEvent, GateChoice, RunHandle};
use crate::project::{Out, Projector};
use agent_client_protocol as acp;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use xai_acp_lib::{AcpClientMessage, acp_send};

pub struct AgentConfig {
    pub driver: DriverConfig,
    pub default_flow: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Flow { file: String, input: Value },
    Runs,
    Open { run_id: String },
    Answer { label: String },
    Task { text: String },
    Help,
}

#[derive(Debug, Clone)]
struct WaitingGate {
    run_id: String,
    run_dir: PathBuf,
    node: String,
    attempt: String,
    question: String,
    choices: Vec<GateChoice>,
    flow_path: String,
}

#[derive(Debug, Clone)]
struct TurnTiming {
    turn_start_ms: i64,
    stream_start_ms: i64,
    prompt_id: Option<String>,
}

struct SessionState {
    cwd: PathBuf,
    waiting_gate: Option<WaitingGate>,
    running_handle: Option<RunHandle>,
    running_run_id: Option<String>,
    pending_question_tool_call_id: Option<String>,
    pending_question_cancel: Option<oneshot::Sender<()>>,
    in_flight: bool,
    cancelled: bool,
    shown_runs: HashSet<String>,
    event_counters: HashMap<String, u64>,
}

impl SessionState {
    fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            waiting_gate: None,
            running_handle: None,
            running_run_id: None,
            pending_question_tool_call_id: None,
            pending_question_cancel: None,
            in_flight: false,
            cancelled: false,
            shown_runs: HashSet::new(),
            event_counters: HashMap::new(),
        }
    }
}

struct State {
    cfg: AgentConfig,
    next_session_counter: u64,
    sessions: HashMap<String, SessionState>,
}

#[derive(Clone)]
pub struct AcpxAgent {
    client_tx: mpsc::UnboundedSender<AcpClientMessage>,
    state: Rc<RefCell<State>>,
}

impl AcpxAgent {
    pub fn new(cfg: AgentConfig, client_tx: mpsc::UnboundedSender<AcpClientMessage>) -> Self {
        Self {
            client_tx,
            state: Rc::new(RefCell::new(State {
                cfg,
                next_session_counter: 1,
                sessions: HashMap::new(),
            })),
        }
    }

    fn ensure_session(&self, session_id: &str, cwd: Option<PathBuf>) -> PathBuf {
        let mut st = self.state.borrow_mut();
        let default_cwd = st.cfg.driver.cwd.clone();
        let entry = st
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState::new(cwd.clone().unwrap_or_else(|| default_cwd.clone())));
        if let Some(c) = cwd {
            entry.cwd = c;
        }
        entry.cwd.clone()
    }

    fn session_driver_config(&self, session_id: &str) -> DriverConfig {
        let st = self.state.borrow();
        let mut drv = st.cfg.driver.clone();
        if let Some(sess) = st.sessions.get(session_id) {
            drv.cwd = sess.cwd.clone();
        }
        drv
    }

    fn next_event_id(&self, parent_session_id: &str, target_session_id: &str) -> Option<String> {
        if target_session_id == parent_session_id {
            return None;
        }
        let mut st = self.state.borrow_mut();
        let sess = st.sessions.get_mut(parent_session_id)?;
        let counter = sess
            .event_counters
            .entry(target_session_id.to_string())
            .or_insert(0);
        *counter = counter.saturating_add(1);
        Some(format!("{target_session_id}:{counter}"))
    }

    async fn send_parent_text(
        &self,
        session_id: &str,
        text: impl Into<String>,
        timing: &TurnTiming,
    ) {
        let update = acp::SessionUpdate::AgentMessageChunk(text_chunk(text));
        self.send_out(
            session_id,
            Out::Update {
                session_id: session_id.to_string(),
                update,
            },
            Some(timing),
            false,
            None,
        )
        .await;
    }

    async fn send_out(
        &self,
        parent_session_id: &str,
        out: Out,
        timing: Option<&TurnTiming>,
        is_replay: bool,
        at_ms: Option<i64>,
    ) {
        let target_session_id = match &out {
            Out::Update { session_id, .. } | Out::XaiUpdate { session_id, .. } => {
                session_id.as_str()
            }
        };
        let event_id = self.next_event_id(parent_session_id, target_session_id);
        let now_ms = at_ms.unwrap_or_else(utc_now_ms);
        let meta = build_notification_meta(timing, is_replay, event_id.as_deref(), now_ms);

        match out {
            Out::Update { session_id, update } => {
                let mut notification =
                    acp::SessionNotification::new(acp::SessionId::new(session_id), update);
                notification = notification.meta(Some(meta));
                if let Err(error) = acp_send(notification, &self.client_tx).await {
                    tracing::debug!(%error, "pager closed while sending acpx session update");
                }
            }
            Out::XaiUpdate { session_id, update } => {
                let method = if is_replay {
                    "x.ai/session/update"
                } else {
                    "x.ai/session_notification"
                };
                let params = json!({
                    "sessionId": session_id,
                    "update": update,
                    "_meta": Value::Object(meta),
                });
                self.send_ext_notification(method, params).await;
            }
        }
    }

    async fn send_ext_notification(&self, method: &str, params: Value) {
        let Ok(raw) = serde_json::value::to_raw_value(&params) else {
            return;
        };
        let notification = acp::ExtNotification::new(method, raw.into());
        if let Err(error) = acp_send(notification, &self.client_tx).await {
            tracing::debug!(%error, method, "pager closed while sending acpx ext notification");
        }
    }

    async fn run_flow_chain(
        &self,
        session_id: &str,
        initial_flow: PathBuf,
        initial_input: Value,
        timing: &TurnTiming,
    ) {
        let mut current_flow = initial_flow;
        let mut current_input = initial_input;

        loop {
            if self.is_session_cancelled(session_id) {
                break;
            }

            let drv_cfg = self.session_driver_config(session_id);
            let input_json =
                serde_json::to_string(&current_input).unwrap_or_else(|_| "{}".to_string());
            let (tx, mut rx) = mpsc::unbounded_channel();

            let handle = match driver::start(&drv_cfg, &current_flow, &input_json, tx) {
                Ok(h) => h,
                Err(err) => {
                    self.send_parent_text(
                        session_id,
                        format!("Failed to start acpx flow `{}`: {err}", current_flow.display()),
                        timing,
                    )
                    .await;
                    break;
                }
            };

            {
                let mut st = self.state.borrow_mut();
                if let Some(sess) = st.sessions.get_mut(session_id) {
                    sess.waiting_gate = None;
                    sess.running_handle = Some(handle);
                    sess.running_run_id = None;
                }
            }

            let mut projector = Projector::new(session_id);
            let mut run_id = String::new();
            let mut run_dir: Option<PathBuf> = None;
            let mut flow_path = current_flow.to_string_lossy().into_owned();
            let mut pending_waiting: Option<(String, String, String, Vec<GateChoice>)> = None;

            while let Some(ev) = rx.recv().await {
                match &ev {
                    DriverEvent::RunStarted {
                        run_id: rid,
                        run_dir: rdir,
                        flow_path: fpath,
                        ..
                    } => {
                        run_id = rid.clone();
                        run_dir = Some(rdir.clone());
                        if !fpath.is_empty() {
                            flow_path = fpath.clone();
                        }
                        let mut st = self.state.borrow_mut();
                        if let Some(sess) = st.sessions.get_mut(session_id) {
                            sess.running_run_id = Some(rid.clone());
                            sess.shown_runs.insert(rid.clone());
                        }
                    }
                    DriverEvent::Waiting {
                        node,
                        attempt,
                        question,
                        choices,
                        ..
                    } => {
                        if !choices.is_empty() {
                            pending_waiting = Some((
                                node.clone(),
                                attempt.clone(),
                                question.clone(),
                                choices.clone(),
                            ));
                        }
                    }
                    _ => {}
                }

                let outs = projector.on_event(&ev);
                for out in outs {
                    self.send_out(session_id, out, Some(timing), false, None)
                        .await;
                }

                if matches!(ev, DriverEvent::RunFinished { .. }) {
                    break;
                }
            }

            let fallback_run_dir = {
                let mut st = self.state.borrow_mut();
                if let Some(sess) = st.sessions.get_mut(session_id) {
                    let dir = sess.running_handle.as_ref().and_then(RunHandle::run_dir);
                    sess.running_handle = None;
                    sess.running_run_id = None;
                    dir
                } else {
                    None
                }
            };
            let resolved_run_dir = run_dir.or(fallback_run_dir);

            if self.is_session_cancelled(session_id) {
                break;
            }

            let Some((node, attempt, question, choices)) = pending_waiting else {
                break;
            };
            let Some(rdir) = resolved_run_dir else {
                break;
            };

            let gate = WaitingGate {
                run_id: if run_id.is_empty() {
                    rdir.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("run")
                        .to_string()
                } else {
                    run_id
                },
                run_dir: rdir,
                node,
                attempt,
                question,
                choices,
                flow_path,
            };

            match self.ask_gate_question(session_id, &gate).await {
                Some(label) => {
                    if self.is_session_cancelled(session_id) {
                        break;
                    }
                    match self.prepare_chained_run(session_id, &gate, &label, timing).await {
                        Some((next_flow, next_input)) => {
                            current_flow = next_flow;
                            current_input = next_input;
                        }
                        None => {
                            self.remember_waiting_gate(session_id, gate);
                            break;
                        }
                    }
                }
                None => {
                    let node_name = gate.node.clone();
                    self.remember_waiting_gate(session_id, gate);
                    if !self.is_session_cancelled(session_id) {
                        self.send_parent_text(
                            session_id,
                            format!(
                                "\n◐ Waiting at {node_name}. Answer later with `/answer <label>`.\n"
                            ),
                            timing,
                        )
                        .await;
                    }
                    break;
                }
            }
        }
    }

    async fn prepare_chained_run(
        &self,
        session_id: &str,
        gate: &WaitingGate,
        chosen_label: &str,
        timing: &TurnTiming,
    ) -> Option<(PathBuf, Value)> {
        let choice = gate.choices.iter().find(|c| {
            c.label.trim().eq_ignore_ascii_case(chosen_label.trim())
        })?;
        let canonical_label = choice.label.clone();
        let next_flow_file = choice.flow_file.clone();

        let run = match xai_grok_pager::acpx::bundle::load_run(&gate.run_dir) {
            Ok(r) => r,
            Err(err) => {
                self.send_parent_text(
                    session_id,
                    format!("Failed to load waiting run at `{}`: {err}", gate.run_dir.display()),
                    timing,
                )
                .await;
                return None;
            }
        };
        let next_input = xai_grok_pager::acpx::bundle::gate_input(&run, &canonical_label);
        let session_cwd = self.session_cwd(session_id);
        let next_flow = resolve_next_flow_path(&session_cwd, &gate.flow_path, &next_flow_file);

        self.send_parent_text(
            session_id,
            format!("\n→ **{canonical_label}**: starting `{next_flow_file}`\n"),
            timing,
        )
        .await;

        Some((next_flow, next_input))
    }

    async fn ask_gate_question(&self, session_id: &str, gate: &WaitingGate) -> Option<String> {
        let tool_call_id = gate_tool_call_id(&gate.run_id, &gate.attempt);
        let options: Vec<Value> = gate
            .choices
            .iter()
            .map(|c| {
                json!({
                    "label": c.label,
                    "description": format!("starts {}", c.flow_file),
                    "preview": Value::Null,
                    "id": c.label,
                })
            })
            .collect();

        let params = json!({
            "sessionId": session_id,
            "toolCallId": tool_call_id,
            "questions": [{
                "question": gate.question,
                "options": options,
                "multiSelect": false,
                "id": "acpx-gate",
            }],
            "mode": "default",
            "noFreeform": true,
        });

        let raw = serde_json::value::to_raw_value(&params).ok()?;
        let request = acp::ExtRequest::new("x.ai/ask_user_question", raw.into());

        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                if sess.cancelled {
                    return None;
                }
                sess.pending_question_tool_call_id = Some(tool_call_id);
                sess.pending_question_cancel = Some(cancel_tx);
            }
        }

        let send_fut = acp_send(request, &self.client_tx);
        let response = tokio::select! {
            res = send_fut => res.ok(),
            _ = cancel_rx => None,
        };

        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                sess.pending_question_tool_call_id = None;
                sess.pending_question_cancel = None;
            }
        }

        let response = response?;
        let outer: Value = serde_json::from_str(response.0.get()).ok()?;
        let chosen = extract_gate_answer_label(&outer)?;
        let matched = gate
            .choices
            .iter()
            .find(|c| c.label.trim().eq_ignore_ascii_case(chosen.trim()))
            .map(|c| c.label.clone());
        matched
    }

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

        Ok(())
    }

    fn remember_waiting_gate(&self, session_id: &str, gate: WaitingGate) {
        let mut st = self.state.borrow_mut();
        if let Some(sess) = st.sessions.get_mut(session_id) {
            sess.waiting_gate = Some(gate);
        }
    }

    fn is_session_cancelled(&self, session_id: &str) -> bool {
        self.state
            .borrow()
            .sessions
            .get(session_id)
            .is_some_and(|s| s.cancelled)
    }

    fn session_cwd(&self, session_id: &str) -> PathBuf {
        let st = self.state.borrow();
        st.sessions
            .get(session_id)
            .map(|s| s.cwd.clone())
            .unwrap_or_else(|| st.cfg.driver.cwd.clone())
    }

    fn default_flow(&self) -> Option<PathBuf> {
        self.state.borrow().cfg.default_flow.clone()
    }

    fn runs_dir(&self) -> PathBuf {
        self.state.borrow().cfg.driver.runs_dir.clone()
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Agent for AcpxAgent {
    async fn initialize(
        &self,
        _arguments: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        Ok(acp::InitializeResponse::new(acp::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new()
                    .load_session(true)
                    .session_capabilities(
                        acp::SessionCapabilities::new().list(acp::SessionListCapabilities::new()),
                    ),
            )
            .agent_info(
                acp::Implementation::new("acpx", env!("CARGO_PKG_VERSION")).title("acpx"),
            ))
    }

    async fn authenticate(
        &self,
        _arguments: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        Ok(acp::AuthenticateResponse::new())
    }

    async fn new_session(
        &self,
        arguments: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        let now = utc_now_ms();
        let session_id = {
            let mut st = self.state.borrow_mut();
            let counter = st.next_session_counter;
            st.next_session_counter = st.next_session_counter.saturating_add(1);
            let id = format!("acpx-{now}-{counter}");
            st.sessions.insert(id.clone(), SessionState::new(arguments.cwd));
            id
        };

        let tx = self.client_tx.clone();
        let sid = session_id.clone();
        tokio::task::spawn_local(async move {
            let update = acp::SessionUpdate::AvailableCommandsUpdate(
                acp::AvailableCommandsUpdate::new(available_commands()),
            );
            let mut notification =
                acp::SessionNotification::new(acp::SessionId::new(sid), update);
            let meta = build_notification_meta(None, false, None, utc_now_ms());
            notification = notification.meta(Some(meta));
            let _ = acp_send(notification, &tx).await;
        });

        Ok(acp::NewSessionResponse::new(session_id))
    }

    async fn load_session(
        &self,
        arguments: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        let session_id = arguments.session_id.0.to_string();
        self.ensure_session(&session_id, Some(arguments.cwd));

        let runs_dir = self.runs_dir();
        if let Some(run_dir) = resolve_run_dir(&runs_dir, &session_id) {
            let _ = self
                .replay_run_into_session(&session_id, &run_dir, true, None)
                .await;
        }

        Ok(acp::LoadSessionResponse::new())
    }

    async fn list_sessions(
        &self,
        arguments: acp::ListSessionsRequest,
    ) -> Result<acp::ListSessionsResponse, acp::Error> {
        let (runs_dir, default_cwd) = {
            let st = self.state.borrow();
            (st.cfg.driver.runs_dir.clone(), st.cfg.driver.cwd.clone())
        };
        let cwd = arguments.cwd.unwrap_or(default_cwd);
        let runs = driver::list_runs(&runs_dir, 50);
        let infos = runs
            .into_iter()
            .map(|(run_id, flow_name, title, _status, started_ms)| {
                let display_title = if title.trim().is_empty() {
                    flow_name
                } else {
                    title
                };
                acp::SessionInfo::new(run_id, cwd.clone())
                    .title(display_title)
                    .updated_at(format_iso8601_ms(started_ms))
            })
            .collect();
        Ok(acp::ListSessionsResponse::new(infos))
    }

    async fn set_session_mode(
        &self,
        _arguments: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        Ok(acp::SetSessionModeResponse::new())
    }

    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let session_id = arguments.session_id.0.to_string();
        self.ensure_session(&session_id, None);

        let client_prompt_id = extract_client_prompt_id(arguments.meta.as_ref());
        let now = utc_now_ms();
        let timing = TurnTiming {
            turn_start_ms: now,
            stream_start_ms: now,
            prompt_id: client_prompt_id.clone(),
        };

        let (already_in_flight, waiting_labels) = {
            let mut st = self.state.borrow_mut();
            let sess = st
                .sessions
                .get_mut(&session_id)
                .expect("session ensured above");
            if sess.in_flight {
                (true, Vec::new())
            } else {
                sess.in_flight = true;
                sess.cancelled = false;
                let labels = sess
                    .waiting_gate
                    .as_ref()
                    .map(|g| g.choices.iter().map(|c| c.label.clone()).collect())
                    .unwrap_or_default();
                (false, labels)
            }
        };

        if already_in_flight {
            self.send_parent_text(
                &session_id,
                "A run is in progress; press Esc to cancel it.",
                &timing,
            )
            .await;
            return Ok(prompt_response(
                acp::StopReason::EndTurn,
                client_prompt_id.as_deref(),
            ));
        }

        let prompt_text = join_prompt_text(&arguments.prompt);
        let cmd = parse_command(&prompt_text, &waiting_labels);

        match cmd {
            Command::Help => {
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
                let waiting_gate = self
                    .state
                    .borrow()
                    .sessions
                    .get(&session_id)
                    .and_then(|s| s.waiting_gate.clone());
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
                            } else {
                                self.remember_waiting_gate(&session_id, gate);
                            }
                        }
                    }
                }
            }
            Command::Flow { file, input } => {
                let cwd = self.session_cwd(&session_id);
                let flow_path = resolve_flow_path(&cwd, &file);
                self.run_flow_chain(&session_id, flow_path, input, &timing)
                    .await;
            }
            Command::Task { text } => {
                if let Some(default_flow) = self.default_flow() {
                    let cwd = self.session_cwd(&session_id);
                    let flow_path = if default_flow.is_absolute() {
                        default_flow
                    } else {
                        cwd.join(default_flow)
                    };
                    let input = if text.trim().is_empty() {
                        json!({})
                    } else {
                        json!({ "task": text })
                    };
                    self.run_flow_chain(&session_id, flow_path, input, &timing)
                        .await;
                } else {
                    self.send_parent_text(&session_id, help_markdown(None), &timing)
                        .await;
                }
            }
        }

        let was_cancelled = {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(&session_id) {
                let c = sess.cancelled;
                sess.in_flight = false;
                sess.cancelled = false;
                sess.running_handle = None;
                sess.running_run_id = None;
                c
            } else {
                false
            }
        };

        let reason = if was_cancelled {
            acp::StopReason::Cancelled
        } else {
            acp::StopReason::EndTurn
        };
        Ok(prompt_response(reason, client_prompt_id.as_deref()))
    }

    async fn cancel(&self, arguments: acp::CancelNotification) -> Result<(), acp::Error> {
        let requested_id = arguments.session_id.0.to_string();
        let target_session_id = {
            let st = self.state.borrow();
            resolve_cancel_target_session(&st.sessions, &requested_id)
        };

        let Some(session_id) = target_session_id else {
            return Ok(());
        };

        let (handle, pending_tool_call_id, pending_cancel_tx) = {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(&session_id) {
                sess.cancelled = true;
                (
                    sess.running_handle.take(),
                    sess.pending_question_tool_call_id.take(),
                    sess.pending_question_cancel.take(),
                )
            } else {
                (None, None, None)
            }
        };

        if let Some(h) = handle {
            h.cancel();
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(&session_id) {
                sess.running_handle = Some(h);
            }
        }

        if let Some(tx) = pending_cancel_tx {
            let _ = tx.send(());
        }

        if let Some(tool_call_id) = pending_tool_call_id {
            self.send_ext_notification(
                "pi/ui/cancel_interaction",
                json!({ "toolCallId": tool_call_id }),
            )
            .await;
        }

        Ok(())
    }

    async fn ext_method(&self, arguments: acp::ExtRequest) -> Result<acp::ExtResponse, acp::Error> {
        let method = arguments.method.as_ref();
        Err(acp::Error::new(
            acp::ErrorCode::MethodNotFound.into(),
            format!("Method not found: {method}"),
        ))
    }

    async fn ext_notification(&self, _arguments: acp::ExtNotification) -> Result<(), acp::Error> {
        Ok(())
    }
}

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
    }
}

fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(idx) => (&s[..idx], s[idx..].trim_start()),
        None => (s, ""),
    }
}

fn parse_flow_file_and_rest(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    if let Some(after_quote) = s.strip_prefix('"') {
        let mut file = String::new();
        let mut chars = after_quote.char_indices();
        while let Some((idx, ch)) = chars.next() {
            if ch == '\\' {
                if let Some((_, next_ch)) = chars.next() {
                    file.push(next_ch);
                }
            } else if ch == '"' {
                let rest = after_quote[idx + 1..].trim_start();
                return Some((file, rest));
            } else {
                file.push(ch);
            }
        }
        Some((file, ""))
    } else {
        let (file, rest) = split_first_word(s);
        Some((file.to_string(), rest))
    }
}

fn unquote_arg(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].trim().to_string()
    } else {
        s.to_string()
    }
}

fn parse_flow_input(rest: &str) -> Value {
    let rest = rest.trim();
    if rest.is_empty() {
        return json!({});
    }
    if rest.starts_with('{')
        && let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(rest)
    {
        return Value::Object(obj);
    }
    json!({ "task": rest })
}

fn available_commands() -> Vec<acp::AvailableCommand> {
    vec![
        acp::AvailableCommand::new("flow", "Run an acpx flow file").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<flow file> [task or JSON input]",
            )),
        ),
        acp::AvailableCommand::new("runs", "List recent acpx flow runs"),
        acp::AvailableCommand::new("open", "Replay an existing acpx run into this session").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<run id>",
            )),
        ),
        acp::AvailableCommand::new("answer", "Answer a waiting checkpoint gate").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<choice>",
            )),
        ),
        acp::AvailableCommand::new("acpx-help", "Show acpx flow commands and usage"),
    ]
}

fn build_notification_meta(
    timing: Option<&TurnTiming>,
    is_replay: bool,
    event_id: Option<&str>,
    agent_timestamp_ms: i64,
) -> acp::Meta {
    let mut meta = acp::Meta::new();
    if is_replay {
        meta.insert("isReplay".into(), Value::Bool(true));
    }
    if let Some(t) = timing {
        meta.insert("turnStartMs".into(), json!(t.turn_start_ms));
        meta.insert("streamStartMs".into(), json!(t.stream_start_ms));
        if let Some(pid) = t.prompt_id.as_deref().filter(|s| !s.is_empty()) {
            meta.insert("promptId".into(), Value::String(pid.to_string()));
        }
    }
    if let Some(eid) = event_id.filter(|s| !s.is_empty()) {
        meta.insert("eventId".into(), Value::String(eid.to_string()));
    }
    meta.insert("agentTimestampMs".into(), json!(agent_timestamp_ms));
    meta
}

fn extract_gate_answer_label(outer: &Value) -> Option<String> {
    let result = outer.get("result").unwrap_or(outer);
    if result.get("outcome").and_then(Value::as_str) != Some("accepted") {
        return None;
    }
    let answers = result.get("answers")?.as_object()?;
    let first_val = answers.values().next()?;
    if let Some(arr) = first_val.as_array() {
        arr.first()?.as_str().map(str::to_string)
    } else {
        first_val.as_str().map(str::to_string)
    }
}

fn resolve_flow_path(session_cwd: &Path, flow_file: &str) -> PathBuf {
    let p = Path::new(flow_file);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        session_cwd.join(p)
    }
}

fn resolve_next_flow_path(
    session_cwd: &Path,
    declaring_flow_path: &str,
    next_flow_file: &str,
) -> PathBuf {
    let next = Path::new(next_flow_file);
    if next.is_absolute() {
        return next.to_path_buf();
    }
    let declaring = resolve_flow_path(session_cwd, declaring_flow_path);
    match declaring.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(next),
        _ => session_cwd.join(next),
    }
}

fn resolve_run_dir(runs_dir: &Path, run_id: &str) -> Option<PathBuf> {
    let trimmed = run_id.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') || trimmed == ".." || trimmed == "." {
        return None;
    }
    let candidate = runs_dir.join(trimmed);
    if candidate.is_dir() && (candidate.join("manifest.json").is_file() || candidate.join("flow.json").is_file()) {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_cancel_target_session(
    sessions: &HashMap<String, SessionState>,
    requested_id: &str,
) -> Option<String> {
    if sessions.contains_key(requested_id) {
        return Some(requested_id.to_string());
    }
    if let Some(rest) = requested_id.strip_prefix("acpx-step:") {
        let run_id = rest.split(':').next().unwrap_or("");
        if !run_id.is_empty() {
            for (sid, sess) in sessions {
                if sess.running_run_id.as_deref() == Some(run_id) {
                    return Some(sid.clone());
                }
            }
        }
    }
    sessions
        .iter()
        .find_map(|(sid, sess)| sess.in_flight.then(|| sid.clone()))
}

fn gate_tool_call_id(run_id: &str, attempt: &str) -> String {
    format!("acpx-gate:{run_id}:{attempt}")
}

fn extract_client_prompt_id(meta: Option<&acp::Meta>) -> Option<String> {
    meta.and_then(|m| m.get("promptId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn join_prompt_text(blocks: &[acp::ContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in blocks {
        if let acp::ContentBlock::Text(t) = block {
            parts.push(t.text.as_str());
        }
    }
    parts.join("\n")
}

fn content_chunk(content: acp::ContentBlock) -> acp::ContentChunk {
    acp::ContentChunk::new(content)
}

fn text_chunk(text: impl Into<String>) -> acp::ContentChunk {
    content_chunk(acp::ContentBlock::Text(acp::TextContent::new(text)))
}

fn prompt_response(
    reason: acp::StopReason,
    client_prompt_id: Option<&str>,
) -> acp::PromptResponse {
    let mut response = acp::PromptResponse::new(reason);
    if let Some(prompt_id) = client_prompt_id.filter(|id| !id.is_empty()) {
        let mut meta = acp::Meta::new();
        meta.insert("promptId".into(), Value::String(prompt_id.to_string()));
        response = response.meta(Some(meta));
    }
    response
}

fn format_runs_table(runs: &[(String, String, String, String, i64)]) -> String {
    if runs.is_empty() {
        return "No acpx runs found.".to_string();
    }
    let mut out = String::from("| run | flow | status | started |\n| --- | --- | --- | --- |\n");
    for (run_id, flow_name, _title, status, started_ms) in runs {
        let started = format_iso8601_ms(*started_ms);
        out.push_str(&format!("| `{run_id}` | {flow_name} | {status} | {started} |\n"));
    }
    out
}

fn help_markdown(default_flow: Option<&Path>) -> String {
    let default_desc = match default_flow {
        Some(path) => format!(
            "A plain message runs the default flow (`{}`) with `{{\"task\": \"<message>\"}}`, or answers a waiting gate if it matches one of the choice labels.",
            path.display()
        ),
        None => "A plain message answers a waiting gate if it matches one of the choice labels, or prints this help when no default flow is configured.".to_string(),
    };
    format!(
        "### acpx commands\n\
        - `/flow <file> [task or JSON]` — start an acpx flow run\n\
        - `/runs` — list recent flow runs\n\
        - `/open <run id>` — replay an existing run into this session\n\
        - `/answer <choice>` — answer a waiting checkpoint gate\n\
        - `/acpx-help` — show this command list\n\n\
        {default_desc}\n"
    )
}

fn utc_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn format_iso8601_ms(ms: i64) -> String {
    let ms = ms.max(0);
    let total_secs = ms / 1000;
    let millis = (ms % 1000) as u32;
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_flow_empty_input() {
        let cmd = parse_command("/flow ship.flow.ts", &[]);
        assert_eq!(
            cmd,
            Command::Flow {
                file: "ship.flow.ts".to_string(),
                input: json!({}),
            }
        );
    }

    #[test]
    fn parse_command_flow_json_object_input() {
        let cmd = parse_command("/flow ship.flow.ts {\"name\":\"x\"}", &[]);
        assert_eq!(
            cmd,
            Command::Flow {
                file: "ship.flow.ts".to_string(),
                input: json!({ "name": "x" }),
            }
        );
    }

    #[test]
    fn parse_command_flow_task_text_input() {
        let cmd = parse_command("/flow f.ts fix the bug", &[]);
        assert_eq!(
            cmd,
            Command::Flow {
                file: "f.ts".to_string(),
                input: json!({ "task": "fix the bug" }),
            }
        );
    }

    #[test]
    fn parse_command_flow_quoted_path_with_spaces() {
        let cmd = parse_command("/flow \"my flows/ship it.flow.ts\" deploy prod", &[]);
        assert_eq!(
            cmd,
            Command::Flow {
                file: "my flows/ship it.flow.ts".to_string(),
                input: json!({ "task": "deploy prod" }),
            }
        );
    }

    #[test]
    fn parse_command_runs_and_open_and_answer() {
        assert_eq!(parse_command("/runs", &[]), Command::Runs);
        assert_eq!(
            parse_command("/open run-2026-01", &[]),
            Command::Open {
                run_id: "run-2026-01".to_string()
            }
        );
        assert_eq!(
            parse_command("/answer approve", &[]),
            Command::Answer {
                label: "approve".to_string()
            }
        );
    }

    #[test]
    fn parse_command_plain_message_matches_waiting_label_case_insensitive() {
        let waiting = vec!["Approve".to_string(), "Rework".to_string()];
        let cmd = parse_command("  approve  ", &waiting);
        assert_eq!(
            cmd,
            Command::Answer {
                label: "Approve".to_string(),
            }
        );
    }

    #[test]
    fn parse_command_plain_message_becomes_task_when_not_matching_gate() {
        let waiting = vec!["Approve".to_string()];
        let cmd = parse_command("add unit tests for driver", &waiting);
        assert_eq!(
            cmd,
            Command::Task {
                text: "add unit tests for driver".to_string(),
            }
        );
    }

    #[test]
    fn parse_command_unknown_slash_and_acpx_help() {
        assert_eq!(parse_command("/acpx-help", &[]), Command::Help);
        assert_eq!(parse_command("/unknown-cmd arg", &[]), Command::Help);
        assert_eq!(parse_command("/flow", &[]), Command::Help);
        assert_eq!(parse_command("/open", &[]), Command::Help);
    }

    #[test]
    fn resolve_flow_paths_relative_and_absolute() {
        let cwd = Path::new("/workspace/project");
        assert_eq!(
            resolve_flow_path(cwd, "flows/pr.flow.ts"),
            PathBuf::from("/workspace/project/flows/pr.flow.ts")
        );
        assert_eq!(
            resolve_flow_path(cwd, "/opt/flows/pr.flow.ts"),
            PathBuf::from("/opt/flows/pr.flow.ts")
        );
    }

    #[test]
    fn resolve_next_flow_relative_to_declaring_flow_directory() {
        let cwd = Path::new("/workspace/project");
        let resolved =
            resolve_next_flow_path(cwd, "sub/flows/triage.flow.ts", "../followup/fix.flow.ts");
        assert_eq!(
            resolved,
            PathBuf::from("/workspace/project/sub/flows/../followup/fix.flow.ts")
        );

        let abs_declaring =
            resolve_next_flow_path(cwd, "/etc/acpx/triage.flow.ts", "ship.flow.ts");
        assert_eq!(abs_declaring, PathBuf::from("/etc/acpx/ship.flow.ts"));

        let abs_next =
            resolve_next_flow_path(cwd, "sub/triage.flow.ts", "/var/flows/absolute.flow.ts");
        assert_eq!(abs_next, PathBuf::from("/var/flows/absolute.flow.ts"));
    }

    #[test]
    fn available_commands_contents() {
        let cmds = available_commands();
        let names: Vec<String> = cmds
            .iter()
            .filter_map(|c| {
                serde_json::to_value(c)
                    .ok()
                    .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string))
            })
            .collect();
        assert_eq!(names, vec!["flow", "runs", "open", "answer", "acpx-help"]);

        let flow_val = serde_json::to_value(&cmds[0]).unwrap();
        assert_eq!(
            flow_val
                .get("input")
                .and_then(|i| i.get("hint"))
                .and_then(Value::as_str),
            Some("<flow file> [task or JSON input]")
        );
        let help_val = serde_json::to_value(&cmds[4]).unwrap();
        assert!(help_val.get("input").is_none() || help_val.get("input") == Some(&Value::Null));
    }

    #[test]
    fn notification_meta_builder_includes_prompt_id_only_when_present() {
        let timing_with_pid = TurnTiming {
            turn_start_ms: 1000,
            stream_start_ms: 1000,
            prompt_id: Some("p-123".to_string()),
        };
        let meta =
            build_notification_meta(Some(&timing_with_pid), false, Some("child:1"), 1250);
        assert_eq!(meta.get("turnStartMs"), Some(&json!(1000)));
        assert_eq!(meta.get("streamStartMs"), Some(&json!(1000)));
        assert_eq!(meta.get("promptId"), Some(&json!("p-123")));
        assert_eq!(meta.get("eventId"), Some(&json!("child:1")));
        assert_eq!(meta.get("agentTimestampMs"), Some(&json!(1250)));
        assert!(meta.get("isReplay").is_none());

        let timing_no_pid = TurnTiming {
            turn_start_ms: 2000,
            stream_start_ms: 2000,
            prompt_id: None,
        };
        let meta_no_pid = build_notification_meta(Some(&timing_no_pid), true, None, 2500);
        assert!(meta_no_pid.get("promptId").is_none());
        assert!(meta_no_pid.get("eventId").is_none());
        assert_eq!(meta_no_pid.get("isReplay"), Some(&Value::Bool(true)));
    }

    #[test]
    fn extract_gate_answer_label_variants() {
        let direct = json!({
            "outcome": "accepted",
            "answers": {
                "q": ["approve"]
            }
        });
        assert_eq!(
            extract_gate_answer_label(&direct),
            Some("approve".to_string())
        );

        let wrapped = json!({
            "result": {
                "outcome": "accepted",
                "answers": {
                    "Which path?": ["rework"]
                }
            }
        });
        assert_eq!(
            extract_gate_answer_label(&wrapped),
            Some("rework".to_string())
        );

        let cancelled = json!({
            "outcome": "cancelled"
        });
        assert_eq!(extract_gate_answer_label(&cancelled), None);

        let wrapped_cancelled = json!({
            "result": {
                "outcome": "cancelled"
            }
        });
        assert_eq!(extract_gate_answer_label(&wrapped_cancelled), None);
    }
}
