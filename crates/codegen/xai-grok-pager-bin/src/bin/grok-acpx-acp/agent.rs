use crate::driver::{self, DriverConfig, DriverEvent, GateChoice, RunHandle};
use crate::piw_driver::{self, PiwRunHandle};
use crate::project::{Out, Projector};
use agent_client_protocol as acp;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use xai_acp_lib::{AcpClientMessage, acp_send};
use xai_grok_pager::piw::client::PiwClient;
use xai_grok_pager::piw::gate::{self as piw_gate, PiwGate};
use xai_grok_pager::piw::runtime;
use crate::piw_server;

/// One shared workflow-server client for this process, connected lazily to the
/// machine's default socket. `None` while there is no server: everything acpx
/// then behaves exactly as before.
fn piw_shared_client() -> Option<Arc<PiwClient>> {
    runtime::shared_client()
}

/// A workflow file, as opposed to an acpx flow file: `/flow demo.workflow.ts`
/// starts a pi-workflows run instead of an acpx flow.
fn is_workflow_file(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with(".workflow.ts")
}

/// The pi-workflows CLI the driver shells out to, for starting runs and
/// the server: `$GROK_PIWORKFLOWS_CLI` when set (an explicit override for
/// both), else the package Pi's settings resolve (`piw_server::driver_cli`),
/// so the server and the runs always come from the same package.
fn driver_piw_cli() -> std::result::Result<piw_server::DriverCli, String> {
    let Some(agent_dir) = piw_server::pi_agent_dir() else {
        return Err(
            "Pi's agent directory ($HOME/.pi/agent) could not be found; the pi-workflows CLI cannot be found. Set $GROK_PIWORKFLOWS_CLI to a pi-workflows CLI."
                .to_string(),
        );
    };
    piw_server::driver_cli(&agent_dir).map_err(|resolution| {
        piw_server::cli_not_found_text(
            &resolution,
            "Set $GROK_PIWORKFLOWS_CLI to a pi-workflows CLI.",
        )
    })
}

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

/// A pi-workflows run this session is parked on: the gate is answered through
/// the server socket, not by starting another flow.
#[derive(Debug, Clone)]
struct WaitingPiwGate {
    gate: Option<PiwGate>,
    node: String,
    question: String,
    choices: Vec<GateChoice>,
}

impl WaitingPiwGate {
    fn from_gate(gate: PiwGate) -> Self {
        let node = gate.node_id.clone();
        let question = gate.question.clone();
        let choices = piw_choices_as_gate_choices(&gate);
        Self {
            gate: Some(gate),
            node,
            question,
            choices,
        }
    }

    fn from_event(
        gate: Option<PiwGate>,
        node: String,
        question: String,
        choices: Vec<GateChoice>,
    ) -> Self {
        if let Some(gate) = gate {
            let mut waiting = Self::from_gate(gate);
            if !node.is_empty() {
                waiting.node = node;
            }
            if !question.is_empty() {
                waiting.question = question;
            }
            if !choices.is_empty() {
                waiting.choices = choices;
            }
            waiting
        } else {
            Self {
                gate: None,
                node,
                question,
                choices,
            }
        }
    }
}

fn piw_choices_as_gate_choices(gate: &PiwGate) -> Vec<GateChoice> {
    gate.choices
        .iter()
        .map(|choice| GateChoice {
            label: choice.label.clone(),
            flow_file: piw_gate::answer_target(&gate.run_id, &choice.key),
        })
        .collect()
}

/// What happened to a gate question asked in the chat.
#[derive(Debug, Clone, PartialEq)]
enum GateAsk {
    /// A typed answer to send to the server.
    Answered(String),
    /// The user dismissed the question; the run stays parked and answerable.
    Dismissed,
    /// The request never got a response: the pager withdrew it because the
    /// turn is being cancelled. The run must be cancelled with it, and the
    /// turn must end on the run's final state so the chat shows it cancelled.
    Withdrawn,
}

/// What a `Waiting` event did to the turn: keep pumping, park the run and end
/// the turn, or keep pumping until the run reports its cancelled final state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum WaitingOutcome {
    Resumed,
    Parked,
    Cancelled,
}

/// What to do with a prompt that arrived while a run is active or waiting.
enum OccupiedPrompt {
    /// No watched run; handle the prompt as a new command.
    NotOccupied,
    /// Another turn owns `in_flight`; do not clear it and do not cancel.
    LeaveInFlight,
    /// This turn handled the input (gate answer, refusal, or control).
    Handled { ran_flow: bool },
}

/// The response to a gate question, as one of [`GateAsk`]. A response that
/// names no matching label (or an empty checkpoint text) is a dismissal: the
/// run stays parked and answerable through `/answer`.
fn classify_gate_response(outer: &Value, choices: &[GateChoice]) -> GateAsk {
    let notes = extract_gate_answer_notes(outer);
    if choices.is_empty() {
        return match notes.filter(|text| !text.is_empty()) {
            Some(text) => GateAsk::Answered(text),
            None => GateAsk::Dismissed,
        };
    }
    match extract_gate_answer_label(outer)
        .and_then(|label| piw_typed_answer(choices, &label, notes.as_deref()))
    {
        Some(typed) => GateAsk::Answered(typed),
        None => GateAsk::Dismissed,
    }
}

/// A pi-workflows run this session is following. The watch task outlives the
/// turn: while the run is parked the receiver is parked here and events queue
/// up until the answer resumes the run.
struct PiwWatch {
    run_id: String,
    handle: PiwRunHandle,
    rx: Option<mpsc::UnboundedReceiver<DriverEvent>>,
    /// Kept across waiting gates so the transcript is not rebuilt blank.
    projector: Option<Projector>,
}

#[derive(Debug, Clone)]
struct TurnTiming {
    turn_start_ms: i64,
    stream_start_ms: i64,
    prompt_id: Option<String>,
}

impl TurnTiming {
    /// A timing for messages the agent sends outside a turn, such as the
    /// park monitor's final-state report or a server start during a cancel.
    fn fresh() -> TurnTiming {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default();
        TurnTiming {
            turn_start_ms: now_ms,
            stream_start_ms: now_ms,
            prompt_id: None,
        }
    }
}

struct SessionState {
    cwd: PathBuf,
    waiting_gate: Option<WaitingGate>,
    waiting_piw: Option<WaitingPiwGate>,
    running_handle: Option<RunHandle>,
    running_run_id: Option<String>,
    piw: Option<PiwWatch>,
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
            waiting_piw: None,
            running_handle: None,
            running_run_id: None,
            piw: None,
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
        let current_flow = initial_flow;
        let current_input = initial_input;

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

            // Issue 11: park on the inline card. Do not open the question modal.
            self.remember_waiting_gate(session_id, gate);
            break;
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
            format!("\n\n→ **{canonical_label}**: starting `{next_flow_file}`\n\n"),
            timing,
        )
        .await;

        Some((next_flow, next_input))
    }

    /// Make sure a pi-workflows server answers on the machine's default
    /// socket, starting one the way Pi's own client does when the socket is
    /// missing or refuses the connection, and never beside a live server
    /// (the server's own lock stays the authority: the package's own start
    /// command connects first and starts only when nothing answers). While
    /// a start runs, the transcript says one line: `Starting the workflow
    /// server…`. `Err` is the exact failure text for the transcript: the
    /// package path tried, the server's own error text, and the command to
    /// run by hand. No retry: one attempt, the package's own bounded one.
    async fn ensure_piw_server(
        &self,
        session_id: &str,
        timing: &TurnTiming,
    ) -> std::result::Result<(), String> {
        match piw_server::socket_presence() {
            Some(presence) if !presence.needs_start() => return Ok(()),
            Some(_) => {}
            None => {
                return Err(
                    "The default pi-workflows socket path could not be computed; the workflow server cannot be started automatically. Start it by hand: `pi-workflows server start`."
                        .to_string(),
                );
            }
        }
        self.send_parent_text(session_id, "Starting the workflow server…", timing)
            .await;
        // The same CLI the run start uses: `$GROK_PIWORKFLOWS_CLI` when set,
        // else the package Pi's settings resolve.
        match driver_piw_cli() {
            Ok(cli) => {
                let command = cli.entry.server_start();
                match piw_server::run_server_start(&command).await {
                    Ok(()) => Ok(()),
                    Err(detail) => Err(format!(
                        "Starting the workflow server failed (`{}`): {detail}. Start it by hand: `{}`.",
                        cli.package.display(),
                        command.display()
                    )),
                }
            }
            Err(message) => Err(message),
        }
    }

    /// Start a pi-workflows run from a workflow file and follow it through the
    /// same projector loop that renders acpx flows. Cancel maps to
    /// `PiwRunHandle::cancel`, which sends `run.cancel` to the server.
    async fn run_piw_flow(&self, session_id: &str, file: &str, input: Value, timing: &TurnTiming) {
        let cwd = self.session_cwd(session_id);
        let workflow = resolve_flow_path(&cwd, file);
        // Starting a run needs the server; start one only when no server
        // answers (the pop-up's mirror never does this).
        if let Err(message) = self.ensure_piw_server(session_id, timing).await {
            self.send_parent_text(session_id, message, timing)
                .await;
            return;
        }
        let Some(client) = piw_shared_client() else {
            self.send_parent_text(
                session_id,
                "No pi-workflows server is running on this machine; run any `pi-workflows` command first and try again.",
                timing,
            )
            .await;
            return;
        };
        let (run_id, _server_session) = match self.piw_start_run(&cwd, &workflow, &input).await {
            Ok(found) => found,
            Err(message) => {
                self.send_parent_text(
                    session_id,
                    format!("Failed to start workflow `{}`: {message}", workflow.display()),
                    timing,
                )
                .await;
                return;
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = piw_driver::watch(client, &run_id, 100, tx);
        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                // A previous run of this session, parked or finished, stops
                // being watched; its run is not touched.
                if let Some(old) = sess.piw.take() {
                    old.handle.detach();
                }
                sess.waiting_gate = None;
                sess.waiting_piw = None;
                sess.running_run_id = Some(run_id.clone());
                sess.shown_runs.insert(run_id.clone());
                sess.piw = Some(PiwWatch {
                    run_id: run_id.clone(),
                    handle,
                    rx: Some(rx),
                    projector: Some(Projector::new(session_id)),
                });
            }
        }
        self.pump_piw_events(session_id, timing).await;
    }

    /// Run the event loop of the session's pi-workflows run until it parks on
    /// a gate (the receiver is parked back in the session so `/answer` can
    /// resume it) or finishes (the watch is dropped).
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
            self.spawn_piw_park_monitor(session_id, &run_id);
        }
    }

    /// Watch a parked run's server state and report its final state into the
    /// chat. Ends when the run is answered in this chat (`waiting_piw` is
    /// cleared), the session is gone, or the final state was reported.
    fn spawn_piw_park_monitor(&self, session_id: &str, run_id: &str) {
        let Some(client) = piw_shared_client() else {
            return;
        };
        let state = Rc::clone(&self.state);
        let client_tx = self.client_tx.clone();
        let session_id = session_id.to_string();
        let run_id = run_id.to_string();
        tokio::task::spawn_local(async move {
            client.watch_run(&run_id);
            loop {
                tokio::time::sleep(Duration::from_millis(2000)).await;
                let still_parked = {
                    let st = state.borrow();
                    st.sessions
                        .get(&session_id)
                        .is_some_and(|sess| sess.waiting_piw.is_some())
                };
                if !still_parked {
                    return;
                }
                let Some(view) = client.run_view(&run_id) else {
                    continue;
                };
                let v = &view.view;
                let status = v
                    .get("display")
                    .and_then(|d| d.get("status"))
                    .and_then(Value::as_str)
                    .or_else(|| {
                        v.get("manifest")
                            .and_then(|m| m.get("status"))
                            .and_then(Value::as_str)
                    });
                let Some(status) = status else { continue };
                if matches!(
                    status,
                    "queued" | "starting" | "running" | "waiting" | "paused" | "parked" | "ambiguous"
                ) {
                    continue;
                }
                let title = v
                    .get("workflowName")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(run_id.as_str());
                let text = match status {
                    "completed" | "done" => {
                        format!("\n\n✓ **{title}** finished.\n\n")
                    }
                    "cancelled" => {
                        format!("\n\n✗ **{title}** · Workflow run cancelled.\n\n")
                    }
                    other => {
                        format!("\n\n✗ **{title}** failed: {other}.\n\n")
                    }
                };
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or_default();
                let timing = TurnTiming {
                    turn_start_ms: now_ms,
                    stream_start_ms: now_ms,
                    prompt_id: None,
                };
                let agent = AcpxAgent {
                    client_tx: client_tx.clone(),
                    state: Rc::clone(&state),
                };
                agent.send_parent_text(&session_id, text, &timing).await;
                let mut st = state.borrow_mut();
                if let Some(sess) = st.sessions.get_mut(&session_id) {
                    sess.waiting_piw = None;
                    // The run is final; the parked watch is released with it.
                    sess.piw = None;
                }
                return;
            }
        });
    }

    /// A pi-workflows run parked on a human gate: ask the question and answer
    /// it in place, so the same run resumes. False when it stays parked.
    async fn handle_piw_waiting(
        &self,
        session_id: &str,
        node: &str,
        attempt: &str,
        question: &str,
        choices: &[GateChoice],
        timing: &TurnTiming,
    ) -> WaitingOutcome {
        let (run_id, gate) = {
            let st = self.state.borrow();
            st.sessions
                .get(session_id)
                .and_then(|sess| sess.piw.as_ref())
                .map(|watch| (watch.run_id.clone(), watch.handle.gate()))
                .unwrap_or_default()
        };
        if run_id.is_empty() {
            return WaitingOutcome::Parked;
        }

        let ask = self
            .ask_piw_gate_question(session_id, &run_id, attempt, question, choices, gate.as_ref())
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
        }
    }

    /// `pi-workflows start <file> --json`, run with the same CLI the server
    /// start uses (the `$GROK_PIWORKFLOWS_CLI` override, else the package Pi's
    /// settings resolve); returns (run id, server session id) from the JSON
    /// line it prints.
    async fn piw_start_run(
        &self,
        cwd: &Path,
        workflow: &Path,
        input: &Value,
    ) -> std::result::Result<(String, String), String> {
        let input_json = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());
        let cli = driver_piw_cli()?;
        let command = cli.entry.run_start(workflow, &input_json);
        let mut process = tokio::process::Command::new(&command.program);
        process.args(&command.args);
        // An explicit override keeps the session's working directory (the
        // workflow path is absolute); the package's own CLI runs from the
        // package directory so its source entry resolves tsx.
        process.current_dir(command.cwd.as_deref().unwrap_or(cwd));
        let output = match tokio::time::timeout(
            Duration::from_secs(120),
            process
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output(),
        )
        .await
        {
            Ok(output) => output,
            Err(_) => return Err("starting the pi-workflows run timed out".to_string()),
        };
        let output = match output {
            Ok(output) => output,
            Err(err) => {
                return Err(format!(
                    "running `{}` failed: {err}",
                    command.display()
                ))
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(run_id) = value.get("runId").and_then(Value::as_str)
                && !run_id.is_empty()
            {
                let session_id = value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return Ok((run_id.to_string(), session_id));
            }
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_tail = stderr.trim();
        let status_note = if output.status.success() {
            String::new()
        } else {
            format!(" (exit {})", output.status)
        };
        let detail = if stderr_tail.is_empty() {
            stdout.trim()
        } else {
            stderr_tail
        };
        Err(format!(
            "pi-workflows start printed no run id{status_note}: {detail}"
        ))
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

        let outer = self.ask_question_raw(session_id, tool_call_id, params).await?;
        let chosen = extract_gate_answer_label(&outer)?;
        let matched = gate
            .choices
            .iter()
            .find(|c| c.label.trim().eq_ignore_ascii_case(chosen.trim()))
            .map(|c| c.label.clone());
        matched
    }

    /// The same question machinery for a pi-workflows gate: a decision offers
    /// its labels (with a free-form row when a choice wants text with it), a
    /// checkpoint takes typed text. Returns the typed answer, `None` when the
    /// question was dismissed or nothing usable was typed.
    async fn ask_piw_gate_question(
        &self,
        session_id: &str,
        run_id: &str,
        attempt: &str,
        question: &str,
        choices: &[GateChoice],
        gate: Option<&PiwGate>,
    ) -> GateAsk {
        let tool_call_id = gate_tool_call_id(run_id, attempt);
        let needs_text = gate.is_some_and(|g| {
            g.choices.iter().any(|c| c.text_input.as_deref().is_some_and(|t| !t.is_empty()))
        });
        let options: Vec<Value> = choices
            .iter()
            .map(|c| {
                json!({
                    "label": c.label,
                    // No filler: an option shows its own description only when the
                    // workflow gave one (pi-workflows choices carry only a label today).
                    "description": "",
                    "preview": Value::Null,
                    "id": c.label,
                })
            })
            .collect();

        let params = json!({
            "sessionId": session_id,
            "toolCallId": tool_call_id,
            "questions": [{
                "question": question,
                "options": options,
                "multiSelect": false,
                "id": "piw-gate",
            }],
            "mode": "default",
            // A checkpoint answers with typed text; a decision with text
            // inputs can carry notes next to the chosen label.
            "noFreeform": !choices.is_empty() && !needs_text,
        });

        let Some(outer) = self.ask_question_raw(session_id, tool_call_id, params).await else {
            // No response at all: the pager withdrew the request while the
            // turn is being cancelled, not a user dismissal.
            return GateAsk::Withdrawn;
        };
        classify_gate_response(&outer, choices)
    }

    /// Send one `x.ai/ask_user_question` request and return its response
    /// object, wired to the session's cancel plumbing.
    async fn ask_question_raw(
        &self,
        session_id: &str,
        tool_call_id: String,
        params: Value,
    ) -> Option<Value> {
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
        serde_json::from_str(response.0.get()).ok()
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

impl AcpxAgent {
    /// Plain text while a run is active or parked at a gate. Never cancels the run.
    async fn handle_occupied_run_input(
        &self,
        session_id: &str,
        text: &str,
        timing: &TurnTiming,
    ) -> OccupiedPrompt {
        let (in_flight, waiting_gate, waiting_piw) = {
            let st = self.state.borrow();
            let Some(sess) = st.sessions.get(session_id) else {
                return OccupiedPrompt::NotOccupied;
            };
            (
                sess.in_flight,
                sess.waiting_gate.clone(),
                sess.waiting_piw.clone(),
            )
        };
        let waiting = waiting_gate.is_some() || waiting_piw.is_some();
        if !in_flight && !waiting {
            return OccupiedPrompt::NotOccupied;
        }

        if waiting {
            if let Some(cmd) = crate::run_path::classify_mid_run_input(text)
                && !matches!(cmd, crate::run_path::MidRunCommand::QueueFollowUp { .. })
            {
                self.dispatch_mid_run_command(session_id, &cmd, timing).await;
                return if in_flight {
                    OccupiedPrompt::LeaveInFlight
                } else {
                    OccupiedPrompt::Handled { ran_flow: false }
                };
            }
            let (node, choices) = if let Some(waiting) = waiting_piw.as_ref() {
                (waiting.node.clone(), waiting.choices.clone())
            } else {
                let gate = waiting_gate.as_ref().expect("waiting");
                (gate.node.clone(), gate.choices.clone())
            };
            match crate::run_path::match_waiting_gate_text(&node, &choices, text) {
                crate::run_path::GateTextMatch::Matched { label, notes } => {
                    let ran_flow = self
                        .answer_waiting_gate(session_id, waiting_piw, waiting_gate, &label, notes.as_deref(), timing)
                        .await;
                    OccupiedPrompt::Handled { ran_flow }
                }
                crate::run_path::GateTextMatch::Refused { refusal_message } => {
                    self.send_parent_text(session_id, refusal_message, timing).await;
                    if in_flight {
                        OccupiedPrompt::LeaveInFlight
                    } else {
                        OccupiedPrompt::Handled { ran_flow: false }
                    }
                }
            }
        } else {
            match crate::run_path::classify_mid_run_input(text) {
                Some(cmd) => {
                    self.dispatch_mid_run_command(session_id, &cmd, timing).await;
                    OccupiedPrompt::LeaveInFlight
                }
                None => {
                    self.send_parent_text(
                        session_id,
                        "Unrecognized command. The run is still in progress and was not cancelled.",
                        timing,
                    )
                    .await;
                    OccupiedPrompt::LeaveInFlight
                }
            }
        }
    }

    async fn answer_waiting_gate(
        &self,
        session_id: &str,
        waiting_piw: Option<WaitingPiwGate>,
        waiting_gate: Option<WaitingGate>,
        label: &str,
        notes: Option<&str>,
        timing: &TurnTiming,
    ) -> bool {
        let typed = match notes.map(str::trim).filter(|s| !s.is_empty()) {
            Some(notes) => format!("{label} {notes}"),
            None => label.to_string(),
        };
        if let Some(waiting) = waiting_piw {
            let gate = waiting.gate.clone().or_else(|| {
                self.state
                    .borrow()
                    .sessions
                    .get(session_id)
                    .and_then(|sess| sess.piw.as_ref())
                    .and_then(|watch| watch.handle.gate())
            });
            if self
                .answer_piw_gate(session_id, gate, &typed, timing)
                .await
            {
                {
                    let mut st = self.state.borrow_mut();
                    if let Some(sess) = st.sessions.get_mut(session_id) {
                        sess.waiting_piw = None;
                        sess.in_flight = true;
                        sess.cancelled = false;
                    }
                }
                self.pump_piw_events(session_id, timing).await;
                return true;
            }
            return false;
        }
        let Some(gate) = waiting_gate else {
            self.send_parent_text(session_id, "No gate is waiting.", timing)
                .await;
            return false;
        };
        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                sess.waiting_gate = None;
                sess.in_flight = true;
                sess.cancelled = false;
            }
        }
        if let Some((next_flow, next_input)) = self
            .prepare_chained_run(session_id, &gate, label, timing)
            .await
        {
            self.run_flow_chain(session_id, next_flow, next_input, timing)
                .await;
            true
        } else {
            self.remember_waiting_gate(session_id, gate);
            false
        }
    }

    async fn dispatch_mid_run_command(
        &self,
        session_id: &str,
        cmd: &crate::run_path::MidRunCommand,
        timing: &TurnTiming,
    ) {
        match cmd {
            crate::run_path::MidRunCommand::Status => {
                let report = self.mid_run_status_report(session_id);
                self.send_parent_text(session_id, report, timing).await;
            }
            crate::run_path::MidRunCommand::Pause => {
                self.forward_piw_control(session_id, "run.pause", json!({}));
                self.send_parent_text(
                    session_id,
                    "Pause requested. The run was not cancelled.",
                    timing,
                )
                .await;
            }
            crate::run_path::MidRunCommand::Resume => {
                self.forward_piw_control(session_id, "run.resume", json!({}));
                self.send_parent_text(
                    session_id,
                    "Resume requested. The run was not cancelled.",
                    timing,
                )
                .await;
            }
            crate::run_path::MidRunCommand::ChangeSetting { key, value } => {
                self.forward_piw_control(
                    session_id,
                    "run.set",
                    json!({"key": key, "value": value}),
                );
                self.send_parent_text(
                    session_id,
                    format!("Queued setting `{key}`=`{value}`. The run was not cancelled."),
                    timing,
                )
                .await;
            }
            crate::run_path::MidRunCommand::QueueFollowUp { prompt } => {
                self.forward_piw_control(session_id, "run.steer", json!({"prompt": prompt}));
                self.send_parent_text(
                    session_id,
                    format!("Queued steering note: {prompt}\n\nThe run was not cancelled."),
                    timing,
                )
                .await;
            }
        }
    }

    fn forward_piw_control(&self, session_id: &str, operation: &str, payload: Value) {
        let run_id = {
            let st = self.state.borrow();
            st.sessions.get(session_id).and_then(|sess| {
                sess.piw
                    .as_ref()
                    .map(|watch| watch.run_id.clone())
                    .or_else(|| sess.running_run_id.clone())
                    .or_else(|| {
                        sess.waiting_piw
                            .as_ref()
                            .and_then(|waiting| waiting.gate.as_ref().map(|gate| gate.run_id.clone()))
                    })
                    .or_else(|| sess.waiting_gate.as_ref().map(|gate| gate.run_id.clone()))
            })
        };
        let Some(run_id) = run_id else {
            return;
        };
        let Some(client) = piw_shared_client() else {
            return;
        };
        let _ = client.send(operation, Some(&run_id), None, payload);
    }

    fn mid_run_status_report(&self, session_id: &str) -> String {
        let (run_id, run_dir) = {
            let st = self.state.borrow();
            let sess = st.sessions.get(session_id);
            let run_id = sess.and_then(|sess| {
                sess.piw
                    .as_ref()
                    .map(|watch| watch.run_id.clone())
                    .or_else(|| sess.running_run_id.clone())
                    .or_else(|| sess.waiting_gate.as_ref().map(|gate| gate.run_id.clone()))
                    .or_else(|| {
                        sess.waiting_piw
                            .as_ref()
                            .and_then(|waiting| waiting.gate.as_ref().map(|gate| gate.run_id.clone()))
                    })
            });
            let run_dir = sess.and_then(|sess| sess.waiting_gate.as_ref().map(|gate| gate.run_dir.clone()));
            (run_id, run_dir)
        };
        let mut dirs = Vec::new();
        if let Some(dir) = run_dir {
            dirs.push(dir);
        }
        if let Some(id) = run_id.as_deref() {
            if let Some(root) = xai_grok_pager::piw::runtime::mirror_root() {
                dirs.push(root.join(id));
            }
            dirs.push(self.runs_dir().join(id));
        }
        for dir in dirs {
            if let Ok(run) = xai_grok_pager::acpx::bundle::load_run(&dir) {
                return xai_grok_pager::acpx::run_status_bar::format_run_status_report(
                    &run,
                    None,
                    utc_now_ms(),
                );
            }
        }
        match run_id.as_deref() {
            Some(id) => format!("Run `{id}` is in progress. No mirrored bundle is available yet."),
            None => "A run is in progress.".to_string(),
        }
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

        Ok(acp::NewSessionResponse::new(session_id).models(session_model_state()))
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

        Ok(acp::LoadSessionResponse::new().models(session_model_state()))
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

        let prompt_text = join_prompt_text(&arguments.prompt);
        match self
            .handle_occupied_run_input(&session_id, &prompt_text, &timing)
            .await
        {
            OccupiedPrompt::LeaveInFlight => {
                return Ok(prompt_response(
                    acp::StopReason::EndTurn,
                    client_prompt_id.as_deref(),
                    false,
                ));
            }
            OccupiedPrompt::Handled { ran_flow } => {
                let was_cancelled = {
                    let mut st = self.state.borrow_mut();
                    if let Some(sess) = st.sessions.get_mut(&session_id) {
                        let cancelled = sess.cancelled;
                        sess.in_flight = false;
                        sess.cancelled = false;
                        sess.running_handle = None;
                        sess.running_run_id = None;
                        cancelled
                    } else {
                        false
                    }
                };
                let reason = if was_cancelled {
                    acp::StopReason::Cancelled
                } else {
                    acp::StopReason::EndTurn
                };
                return Ok(prompt_response(
                    reason,
                    client_prompt_id.as_deref(),
                    ran_flow,
                ));
            }
            OccupiedPrompt::NotOccupied => {}
        }

        let waiting_labels = {
            let mut st = self.state.borrow_mut();
            let sess = st
                .sessions
                .get_mut(&session_id)
                .expect("session ensured above");
            sess.in_flight = true;
            sess.cancelled = false;
            let mut labels: Vec<String> = sess
                .waiting_gate
                .as_ref()
                .map(|g| g.choices.iter().map(|c| c.label.clone()).collect())
                .unwrap_or_default();
            if let Some(waiting) = sess.waiting_piw.as_ref() {
                labels.extend(waiting.choices.iter().map(|c| c.label.clone()));
            }
            labels
        };

        let cmd = parse_command(&prompt_text, &waiting_labels);
        // True when this turn runs (or resumes) a flow or workflow run: its
        // transcript ends with the run's own duration line.
        let mut ran_flow = false;

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
                if is_workflow_file(&file) {
                    // A `.workflow.ts` file starts a run on the pi-workflows
                    // server instead of an acpx flow.
                    self.run_piw_flow(&session_id, &file, input, &timing).await;
                } else {
                    let cwd = self.session_cwd(&session_id);
                    let flow_path = resolve_flow_path(&cwd, &file);
                    self.run_flow_chain(&session_id, flow_path, input, &timing)
                        .await;
                }
                ran_flow = true;
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
                    ran_flow = true;
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
        Ok(prompt_response(
            reason,
            client_prompt_id.as_deref(),
            ran_flow,
        ))
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

        // A pi-workflows run this session is following: cancel goes to the
        // server, and the watch ends once the run reports a final state.
        // If no server answers anymore, start one first (cancelling a run
        // is an action that needs the server), then cancel through the
        // shared client, which reconnects to it.
        if self.state.borrow().sessions.get(&session_id).is_some_and(|sess| sess.piw.is_some()) {
            if let Err(message) = self.ensure_piw_server(&session_id, &TurnTiming::fresh()).await {
                self.send_parent_text(&session_id, message, &TurnTiming::fresh()).await;
            }
        }
        {
            let st = self.state.borrow();
            if let Some(sess) = st.sessions.get(&session_id)
                && let Some(watch) = sess.piw.as_ref()
            {
                watch.handle.cancel();
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

fn session_model_state() -> acp::SessionModelState {
    acp::SessionModelState::new(
        "acpx",
        vec![acp::ModelInfo::new("acpx", "acpx")],
    )
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

/// The typed answer a question response stands for: the chosen label (matched
/// back to its canonical spelling), with the typed notes after it when there
/// are any. `None` when the label matches no choice.
fn piw_typed_answer(choices: &[GateChoice], label: &str, notes: Option<&str>) -> Option<String> {
    let matched = choices
        .iter()
        .find(|c| c.label.trim().eq_ignore_ascii_case(label.trim()))
        .map(|c| c.label.clone())?;
    Some(match notes.filter(|text| !text.trim().is_empty()) {
        Some(text) => format!("{matched} {}", text.trim()),
        None => matched,
    })
}

/// The typed free-form notes of an accepted question response (the pager
/// puts them in `annotations[question].notes`).
fn extract_gate_answer_notes(outer: &Value) -> Option<String> {
    let result = outer.get("result").unwrap_or(outer);
    if result.get("outcome").and_then(Value::as_str) != Some("accepted") {
        return None;
    }
    let annotations = result.get("annotations")?.as_object()?;
    for annotation in annotations.values() {
        if let Some(notes) = annotation.get("notes").and_then(Value::as_str) {
            let trimmed = notes.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
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

/// The transcript hint after a gate is left for later (Shift+x). It names the real choices
/// built from the gate's labels — `◐ Waiting at gate. Answer with `/answer Continue` or `/answer Stop`.`
/// — instead of a generic `<label>` placeholder; a checkpoint with no choices keeps the free-text hint.
fn parked_gate_hint(node: &str, choices: &[GateChoice]) -> String {
    if choices.is_empty() {
        return format!("\n\n◐ Waiting at {node}. Answer later with `/answer <text>`.\n\n");
    }
    let answers: Vec<String> = choices
        .iter()
        .map(|c| format!("`/answer {}`", c.label))
        .collect();
    let joined = match answers.as_slice() {
        [only] => only.clone(),
        [rest @ .., last] => {
            let head = rest.join(", ");
            if head.is_empty() {
                last.clone()
            } else {
                format!("{head} or {last}")
            }
        }
        [] => String::new(),
    };
    format!("\n\n◐ Waiting at {node}. Answer with {joined}.\n\n")
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
    suppress_duration: bool,
) -> acp::PromptResponse {
    let mut response = acp::PromptResponse::new(reason);
    let mut has_meta = false;
    let mut meta = acp::Meta::new();
    if let Some(prompt_id) = client_prompt_id.filter(|id| !id.is_empty()) {
        meta.insert("promptId".into(), Value::String(prompt_id.to_string()));
        has_meta = true;
    }
    if suppress_duration {
        // The turn's transcript already printed the run's duration
        // ("✓ <run> finished in Xs"); tell the pager not to repeat it in the
        // generic "Worked for Ys" turn marker.
        meta.insert(
            "durationSuppressed".into(),
            Value::Bool(true),
        );
        has_meta = true;
    }
    if has_meta {
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

    #[test]
    fn session_model_state_has_valid_current_model() {
        let state = session_model_state();
        assert!(!state.current_model_id.0.is_empty());
        assert!(
            state.available_models.iter().any(|m| m.model_id == state.current_model_id),
            "current_model_id must be present in available_models"
        );
    }

    #[test]
    fn workflow_files_are_recognized_case_insensitively() {
        assert!(is_workflow_file("demo.workflow.ts"));
        assert!(is_workflow_file("Demo.WORKFLOW.TS"));
        assert!(is_workflow_file("flows/mixed demo.workflow.ts"));
        assert!(!is_workflow_file("ship.flow.ts"));
        assert!(!is_workflow_file("workflow.ts"));
        assert!(!is_workflow_file("demo.workflow.ts.bak"));
    }

    #[test]
    fn extract_gate_answer_notes_variants() {
        let accepted = json!({
            "outcome": "accepted",
            "answers": {"q": ["Other"]},
            "annotations": {"q": {"notes": "  ship it  "}}
        });
        assert_eq!(
            extract_gate_answer_notes(&accepted),
            Some("ship it".to_string())
        );

        let wrapped = json!({
            "result": {
                "outcome": "accepted",
                "answers": {"q": ["Other"]},
                "annotations": {"q": {"notes": "text"}}
            }
        });
        assert_eq!(extract_gate_answer_notes(&wrapped), Some("text".to_string()));

        let no_notes = json!({
            "outcome": "accepted",
            "answers": {"q": ["Other"]}
        });
        assert_eq!(extract_gate_answer_notes(&no_notes), None);

        let empty_notes = json!({
            "outcome": "accepted",
            "answers": {"q": ["Other"]},
            "annotations": {"q": {"notes": "   "}}
        });
        assert_eq!(extract_gate_answer_notes(&empty_notes), None);

        let cancelled = json!({"outcome": "cancelled"});
        assert_eq!(extract_gate_answer_notes(&cancelled), None);
    }

    #[test]
    fn piw_typed_answer_matches_label_and_appends_notes() {
        let choices = vec![
            GateChoice { label: "Approve".into(), flow_file: "piw-answer://run-1/approve".into() },
            GateChoice { label: "Rework".into(), flow_file: "piw-answer://run-1/rework".into() },
        ];
        assert_eq!(
            piw_typed_answer(&choices, " approve ", None),
            Some("Approve".to_string())
        );
        assert_eq!(
            piw_typed_answer(&choices, "approve", Some("because the tests fail")),
            Some("Approve because the tests fail".to_string())
        );
        // Notes that are only whitespace are dropped.
        assert_eq!(
            piw_typed_answer(&choices, "rework", Some("   ")),
            Some("Rework".to_string())
        );
        assert_eq!(piw_typed_answer(&choices, "unknown", None), None);
    }

    #[test]
    fn classify_gate_response_variants() {
        let choices = vec![
            GateChoice { label: "Continue".into(), flow_file: "piw-answer://run-1/continue".into() },
            GateChoice { label: "Stop".into(), flow_file: "piw-answer://run-1/stop".into() },
        ];
        // A decision answer with notes types back label + notes.
        assert_eq!(
            classify_gate_response(
                &json!({
                    "outcome": "accepted",
                    "answers": {"q": ["continue"]},
                    "annotations": {"q": {"notes": "run the rest"}}
                }),
                &choices,
            ),
            GateAsk::Answered("Continue run the rest".to_string())
        );
        // A dismissed (cancelled outcome) question leaves the run parked.
        assert_eq!(
            classify_gate_response(&json!({"outcome": "cancelled"}), &choices),
            GateAsk::Dismissed
        );
        // A label that matches no choice is a dismissal, not an answer.
        assert_eq!(
            classify_gate_response(
                &json!({"outcome": "accepted", "answers": {"q": ["nope"]}}),
                &choices,
            ),
            GateAsk::Dismissed
        );
        // A checkpoint answers with its typed text.
        assert_eq!(
            classify_gate_response(
                &json!({
                    "outcome": "accepted",
                    "answers": {"q": ["Other"]},
                    "annotations": {"q": {"notes": "ship it"}}
                }),
                &[],
            ),
            GateAsk::Answered("ship it".to_string())
        );
        // An empty checkpoint text is a dismissal.
        assert_eq!(
            classify_gate_response(
                &json!({"outcome": "accepted", "answers": {"q": ["Other"]}}),
                &[],
            ),
            GateAsk::Dismissed
        );
    }

    #[test]
    fn piw_typed_answer_feeds_parse_answer_for_decision_and_checkpoint() {
        use xai_grok_pager::piw::gate::{GateAnswer, PiwGateChoice};

        let gate = xai_grok_pager::piw::gate::PiwGate {
            run_id: "run-1".into(),
            node_id: "review".into(),
            attempt_id: "review#1".into(),
            session_id: "sess-1".into(),
            kind: xai_grok_pager::piw::gate::GateKind::Decision,
            request_id: "req-1".into(),
            revision: 3,
            question: "Ship it?".into(),
            choices: vec![
                PiwGateChoice { key: "approve".into(), label: "Approve".into(), text_input: None },
                PiwGateChoice {
                    key: "rework".into(),
                    label: "Rework".into(),
                    text_input: Some("why".into()),
                },
            ],
        };

        // A label typed back through the question UI parses to its choice.
        assert_eq!(
            xai_grok_pager::piw::gate::parse_answer(&gate, "Approve"),
            Ok(GateAnswer::Choice { index: 0, text: None })
        );
        // A label with the question UI's notes parses to the choice and its text.
        assert_eq!(
            xai_grok_pager::piw::gate::parse_answer(
                &gate,
                "Rework because the tests fail"
            ),
            Ok(GateAnswer::Choice {
                index: 1,
                text: Some("because the tests fail".to_string())
            })
        );

        // A checkpoint takes the whole typed text.
        let checkpoint = xai_grok_pager::piw::gate::PiwGate {
            run_id: "run-1".into(),
            node_id: "verify".into(),
            attempt_id: "verify#1".into(),
            session_id: "sess-1".into(),
            kind: xai_grok_pager::piw::gate::GateKind::Checkpoint,
            request_id: "req-2".into(),
            revision: 1,
            question: "Anything to add?".into(),
            choices: Vec::new(),
        };
        assert_eq!(
            xai_grok_pager::piw::gate::parse_answer(&checkpoint, "{\"key\": 1}"),
            Ok(GateAnswer::Input(json!({"key": 1})))
        );
        assert_eq!(
            xai_grok_pager::piw::gate::parse_answer(&checkpoint, "just text"),
            Ok(GateAnswer::Input(json!("just text")))
        );
    }

    #[test]
    fn parked_gate_hint_names_the_real_choices() {
        use crate::driver::GateChoice;

        let two = vec![
            GateChoice {
                label: "Continue".to_string(),
                flow_file: "next.flow.json".to_string(),
            },
            GateChoice {
                label: "Stop".to_string(),
                flow_file: "stop.flow.json".to_string(),
            },
        ];
        assert_eq!(
            parked_gate_hint("gate", &two),
            "\n\n\u{25d0} Waiting at gate. Answer with `/answer Continue` or `/answer Stop`.\n\n"
        );

        let three = vec![
            GateChoice { label: "a".to_string(), flow_file: "a.json".to_string() },
            GateChoice { label: "b".to_string(), flow_file: "b.json".to_string() },
            GateChoice { label: "c".to_string(), flow_file: "c.json".to_string() },
        ];
        assert_eq!(
            parked_gate_hint("review", &three),
            "\n\n\u{25d0} Waiting at review. Answer with `/answer a`, `/answer b` or `/answer c`.\n\n"
        );

        // A checkpoint with no choices keeps the free-text hint.
        assert_eq!(
            parked_gate_hint("notes", &[]),
            "\n\n\u{25d0} Waiting at notes. Answer later with `/answer <text>`.\n\n"
        );
    }
}
