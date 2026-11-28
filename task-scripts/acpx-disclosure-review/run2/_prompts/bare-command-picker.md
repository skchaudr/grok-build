Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, ledger routing, or release behavior.

=== REPOSITORY CONTEXT (13 excerpts) ===

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1-121 of 3126 (repository grok-build)
````
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
use xai_grok_pager::acpx::bundle;
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
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1125-1257 of 3126 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1258-1355 of 3126 (repository grok-build)
````
    async fn replay_run_into_session(
        &self,
        session_id: &str,
        run_dir: &Path,
        source: SavedRunSource,
        is_replay: bool,
        timing: Option<&TurnTiming>,
    ) -> anyhow::Result<()> {
        let events = saved_run_events(run_dir, source)?;
        let mut projector = Projector::new(session_id);
        if source == SavedRunSource::PiwMirror {
            projector.show_gates_read_only();
        }
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
                    if source == SavedRunSource::Native && !choices.is_empty() {
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

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1375-1393 of 3126 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 1825-2023 of 3126 (repository grok-build)
````
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
                let runs = list_saved_runs(&runs_dir, runtime::mirror_root().as_deref(), 20);
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
                    match resolve_saved_run(&runs_dir, runtime::mirror_root().as_deref(), &run_id) {
                        Some((run_dir, source)) => {
                            if let Err(err) = self
                                .replay_run_into_session(&session_id, &run_dir, source, false, Some(&timing))
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

````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 2104-2252 of 3126 (repository grok-build)
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

````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 2293-2350 of 3126 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 2375-2478 of 3126 (repository grok-build)
````
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SavedRunSource {
    Native,
    PiwMirror,
}

impl SavedRunSource {
    fn label(self) -> &'static str {
        match self {
            Self::Native => "ACPX",
            Self::PiwMirror => "PIW mirror",
        }
    }
}

type ListedRun = (String, String, String, String, i64);

fn list_saved_runs(base: &Path, mirror: Option<&Path>, limit: usize) -> Vec<(ListedRun, SavedRunSource)> {
    let mut runs: Vec<_> = driver::list_runs(base, limit)
        .into_iter()
        .map(|run| (run, SavedRunSource::Native))
        .collect();
    if let Some(mirror) = mirror {
        runs.extend(driver::list_runs(mirror, limit).into_iter().map(|run| (run, SavedRunSource::PiwMirror)));
    }
    runs.sort_by(|(a, _), (b, _)| b.4.cmp(&a.4).then_with(|| b.0.cmp(&a.0)));
    runs.truncate(limit);
    runs
}

fn resolve_saved_run(base: &Path, mirror: Option<&Path>, id: &str) -> Option<(PathBuf, SavedRunSource)> {
    resolve_run_dir(base, id)
        .map(|dir| (dir, SavedRunSource::Native))
        .or_else(|| {
            resolve_run_dir(mirror?, id).map(|dir| (dir, SavedRunSource::PiwMirror))
        })
}

fn saved_run_events(dir: &Path, source: SavedRunSource) -> anyhow::Result<Vec<DriverEvent>> {
    let mut events = driver::replay(dir)?;
    if source == SavedRunSource::PiwMirror
        && events.iter().any(|event| matches!(event, DriverEvent::Waiting { .. }))
    {
        // The mirror keeps the authoritative gate question and choices in its
        // projections, even when the trace has no checkpoint outcome to replay.
        let run = bundle::load_run(dir)?;
        for event in &mut events {
            if let DriverEvent::Waiting { node, question, choices, .. } = event
                && run.summary.waiting_on.as_deref() == Some(node.as_str())
            {
                if let Some(detail) = run.summary.status_detail.as_ref() {
                    *question = detail.clone();
                }
                *choices = bundle::gate_choices(&run)
                    .into_iter()
                    .map(|(label, _)| GateChoice { label, flow_file: String::new() })
                    .collect();
            }
        }
    }
    Ok(events)
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
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 2531-2597 of 3126 (repository grok-build)
````
fn format_runs_table(runs: &[(ListedRun, SavedRunSource)]) -> String {
    if runs.is_empty() {
        return "No acpx runs found.".to_string();
    }
    let mut out = String::from("| run | flow | source | status | started |\n| --- | --- | --- | --- | --- |\n");
    for ((run_id, flow_name, _title, status, started_ms), source) in runs {
        let started = format_iso8601_ms(*started_ms);
        let source = source.label();
        out.push_str(&format!("| `{run_id}` | {flow_name} | {source} | {status} | {started} |\n"));
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
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs` lines 2598-2783 of 3126 (repository grok-build)
````
mod tests {
    use super::*;

    #[tokio::test]
    async fn saved_runs_list_both_sources_and_only_native_replay_restores_a_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let native = tmp.path().join("native");
        let mirror = tmp.path().join("mirror");
        let at = "2026-11-20T01:14:34Z";
        for (base, id) in [(&native, "native-run"), (&mirror, "workflow-run")] {
            let dir = base.join(id);
            std::fs::create_dir_all(dir.join("projections")).unwrap();
            let projection = json!({"status":"waiting", "waitingOn":"gate", "statusDetail":"Approve the build?"});
            for (file, data) in [
                ("manifest.json", json!({"runId":id, "flowName":"build", "startedAt":at})),
                ("flow.json", json!({"name":"build", "startAt":"gate", "nodes":{"gate":{"nodeType":"checkpoint"}}, "edges":[]})),
                ("projections/live.json", projection),
                ("projections/steps.json", json!([{"attemptId":"a1","nodeId":"gate","nodeType":"checkpoint","outcome":"ok","output":{"next":{"approve":"piw:choice"}}}])),
            ] {
                std::fs::write(dir.join(file), data.to_string()).unwrap();
            }
            if base == &native {
                let outcome = json!({"seq":1,"at":at,"type":"node_outcome","nodeId":"gate","attemptId":"a1","payload":{"nodeType":"checkpoint","outcome":"ok","outputInline":{"next":{"approve":"land.flow.ts"}}}});
                std::fs::write(dir.join("trace.ndjson"), format!("{outcome}\n")).unwrap();
            }
        }

        let table = format_runs_table(&list_saved_runs(&native, Some(&mirror), 20));
        assert!(table.contains("`native-run` | build | ACPX | waiting"), "{table}");
        assert!(table.contains("`workflow-run` | build | PIW mirror | waiting"), "{table}");

        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = tokio::spawn(async move {
            let mut updates = String::new();
            while let Some(message) = rx.recv().await {
                use std::fmt::Write as _;
                writeln!(&mut updates, "{message:?}").unwrap();
                match message {
                    AcpClientMessage::SessionNotification(args) => args.response_tx.send(Ok(())).unwrap(),
                    AcpClientMessage::ExtNotification(args) => args.response_tx.send(Ok(())).unwrap(),
                    _ => panic!("unexpected ACP request: {message:?}"),
                }
            }
            updates
        });
        let agent = AcpxAgent::new(AgentConfig {
            driver: DriverConfig {
                acpx_bin: PathBuf::from("acpx"),
                runs_dir: native.clone(),
                cwd: tmp.path().to_path_buf(),
                extra_path: Vec::new(),
                poll_ms: 100,
            },
            default_flow: None,
        }, tx);
        for id in ["native-run", "workflow-run"] {
            agent.ensure_session(id, None);
            let (dir, source) = resolve_saved_run(&native, Some(&mirror), id).unwrap();
            agent.replay_run_into_session(id, &dir, source, false, None).await.unwrap();
            let st = agent.state.borrow();
            let session = &st.sessions[id];
            if id == "native-run" {
                let gate = session.waiting_gate.as_ref().expect("native gate restored");
                assert_eq!(gate.choices[0].flow_file, "land.flow.ts");
            } else {
                assert!(session.waiting_gate.is_none());
                assert!(session.waiting_piw.is_none());
                assert!(matches!(parse_command("approve", &[]), Command::Task { .. }));
            }
        }
        drop(agent);
        let updates = client.await.unwrap();
        assert!(updates.contains("Approve the build?"), "{updates}");
        assert!(updates.contains("**approve**"), "{updates}");
        assert!(updates.contains("This replay is read-only"), "{updates}");
    }

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
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs` lines 394-530 of 2341 (repository grok-build)
````
pub fn list_runs(runs_dir: &Path, limit: usize) -> Vec<(String, String, String, String, i64)> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let live = read_json_file(&path.join("projections").join("live.json"));

        let run_id = json_str(&manifest, "runId")
            .or_else(|| entry.file_name().to_str().map(ToString::to_string))
            .unwrap_or_default();
        if run_id.is_empty() {
            continue;
        }
        let flow_name = json_str(&manifest, "flowName").unwrap_or_default();
        let title = json_str(&manifest, "runTitle")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                live.as_ref()
                    .and_then(|l| json_str(l, "runTitle"))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| flow_name.clone());
        let status = live
            .as_ref()
            .and_then(|l| json_str(l, "status"))
            .filter(|s| !s.is_empty())
            .or_else(|| json_str(&manifest, "status"))
            .unwrap_or_else(|| "unknown".to_string());
        let started_ms = json_str(&manifest, "startedAt")
            .and_then(|s| parse_ts(&s))
            .unwrap_or(0);

        runs.push((run_id, flow_name, title, status, started_ms));
    }

    runs.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| b.0.cmp(&a.0)));
    runs.truncate(limit);
    runs
}

fn make_temp_input_file(input_json: &str) -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..100u32 {
        let name = format!("grok-acpx-input-{pid}-{nanos}-{attempt}.json");
        let path = dir.join(name);
        if !path.exists() {
            fs::write(&path, input_json.as_bytes())?;
            return Ok(path);
        }
    }
    anyhow::bail!("failed to create temporary input file")
}

fn find_matching_run_dir(
    runs_dir: &Path,
    existing: &HashSet<String>,
    canon_flow: &Path,
) -> Option<PathBuf> {
    let entries = fs::read_dir(runs_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_str()?.to_string();
        if existing.contains(&name) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let Some(fp_str) = json_str(&manifest, "flowPath") else {
            continue;
        };
        let fp = PathBuf::from(&fp_str);
        let canon_fp = fs::canonicalize(&fp).unwrap_or(fp);
        if canon_fp == canon_flow {
            return Some(path);
        }
    }
    None
}

fn extract_stderr_tail(stderr_buf: &Arc<Mutex<Vec<u8>>>) -> String {
    let Ok(guard) = stderr_buf.lock() else {
        return String::new();
    };
    let len = guard.len();
    let start = len.saturating_sub(2048);
    let slice = guard.get(start..).unwrap_or(&[]);
    String::from_utf8_lossy(slice).trim().to_string()
}

fn parse_stdout_json(stdout_buf: &Arc<Mutex<Vec<u8>>>) -> Option<Value> {
    let guard = stdout_buf.lock().ok()?;
    let text = String::from_utf8_lossy(&guard);
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        {
            return Some(val);
        }
    }
    serde_json::from_str::<Value>(text.trim()).ok()
}

fn read_json_file(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(ToString::to_string)
}

#[derive(Default)]
struct LineReader {
    offset: u64,
    buf: Vec<u8>,
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs` lines 1499-1527 of 2341 (repository grok-build)
````
mod tests {
    use super::*;
    use serde_json::json;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(p, content.as_bytes());
    }

    fn base_manifest(run_id: &str, flow_name: &str, title: Option<&str>) -> String {
        let mut obj = json!({
            "runId": run_id,
            "flowName": flow_name,
            "flowPath": "/tmp/flow.json",
            "startedAt": "2026-11-20T01:14:34.000Z",
            "status": "completed"
        });
        if let Some(t) = title
            && let Some(map) = obj.as_object_mut()
        {
            map.insert("runTitle".to_string(), json!(t));
        }
        obj.to_string()
    }

    #[test]
````

### `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs` lines 2168-2219 of 2341 (repository grok-build)
````

    #[test]
    fn test_list_runs_order_and_title_fallback() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let runs_dir = tmp.path();

        let r1 = runs_dir.join("r1");
        write_file(
            &r1,
            "manifest.json",
            &json!({
                "runId": "r1",
                "flowName": "flow-one",
                "startedAt": "2026-11-20T01:00:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r2 = runs_dir.join("r2");
        write_file(
            &r2,
            "manifest.json",
            &json!({
                "runId": "r2",
                "flowName": "flow-two",
                "runTitle": "Explicit Title",
                "startedAt": "2026-11-20T02:00:00.000Z",
                "status": "running"
            })
            .to_string(),
        );
        write_file(
            &r2,
            "projections/live.json",
            &json!({"status": "failed"}).to_string(),
        );

        let list = list_runs(runs_dir, 10);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "r2");
        assert_eq!(list[0].2, "Explicit Title");
        assert_eq!(list[0].3, "failed");
        assert_eq!(list[1].0, "r1");
        assert_eq!(list[1].2, "flow-one");
        assert_eq!(list[1].3, "completed");

        let limited = list_runs(runs_dir, 1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].0, "r2");
    }

````

=== TASK SPECIFICATION ===

Slice ID: bare-command-picker

Ownership: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs and crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs, including their tests. Edit nothing else.

Goal: in Grok ACPX chat a bare /open or bare /flow shows a pick list instead
of the help text. Today parse_command returns Command::Help for both; the
attached parse_command_unknown_slash_and_acpx_help test asserts that and must
change for these two commands only.

Bare /open: show one question through the existing x.ai/ask_user_question
machinery (ask_question_raw, shaped like ask_gate_question, noFreeform true)
listing the 20 most recent saved runs from list_saved_runs(&runs_dir,
runtime::mirror_root().as_deref(), 20), the same list /runs shows, newest
first, native ACPX and mirrored PIW runs together. Option id and label are the
run ID. The description carries what the operator needs to choose: flow name,
title when it differs from the flow name, source (SavedRunSource::label),
status, and start time formatted with the existing helper. Choosing a run does
exactly what `/open <run_id>` does now, through resolve_saved_run and
replay_run_into_session with the resolved source, so a mirrored run stays
read-only and a native run keeps its gate restore; move the current Open arm
into one helper that both paths call, keeping its "Already shown above." and
not-found replies. Dismissing the question, an unknown choice, or a cancelled
session opens nothing and sends one short line. No runs: reply
"No acpx runs found." without a question. Opening stays inspection only.

Bare /flow: list the configured default_flow() first when it exists, then
the flowPath values recorded in the manifests of the 20 most recent runs from
both sources, runs_dir() and runtime::mirror_root(), newest first, each with
the flow name, source, last status and last start time of its most recent
run. /flow <file>.workflow.ts already routes to PIW (is_workflow_file,
run_piw_flow), so a mirrored run's workflow is a valid candidate. Add one
driver function next to list_runs that reads one runs directory's manifests,
reusing read_json_file, json_str and parse_ts, and call it for each source;
skip manifests without a flowPath. Resolve every candidate, the default flow
included, the way explicit /flow resolves its file (resolve_flow_path against
session_cwd); skip a candidate whose resolved path is not an existing file
(recorded paths under /private/tmp are often gone); deduplicate by resolved
path, keeping the newest run's description. Selecting a candidate does exactly
what typing `/flow <resolved path>` does today: it goes through the existing
Flow arm with input {} and starts a new run, the .workflow.ts route included.
Move that arm's body into one helper both paths call. Dismissing the question,
an unknown choice or a cancelled session starts nothing and sends one short
line. No candidates left after filtering: reply with the help text as today.
Mirrored /open stays read-only.

Keep `/open <id>`, `/flow <file> ...`, /runs, /answer (bare /answer still
Help) and plain-text handling unchanged. Update available_commands and
help_markdown so both commands say the argument is optional. Keep the
question's toolCallId distinct from gate tool call IDs.

Tests: keep the attached saved_runs_list_both_sources_and_only_native_replay_restores_a_gate
test passing unchanged. parse_command returns the new bare variants for "/open" and "/flow"
(and still Help for "/runs extra" and bare "/answer"); build the option lists
through pure functions you add and test them: newest-first order across both
sources, run ID as id and label, description content including the source, title omitted when equal to the flow name,
flow candidates from both sources, a relative flowPath resolved against the
session cwd, a missing file skipped (pass the file check in as a closure so the
pure function needs no real paths), two runs whose paths resolve to the same
file collapsed to the newest, default flow first and not repeated, manifests
without flowPath skipped (driver test with temp dirs like the attached
test_list_runs_order_and_title_fallback), selection mapped to the same
Command::Flow { file, input: {} } that typing /flow <path> produces, and the
empty cases.

Edit list format. The file is Markdown. List the edits in application order;
each edit has exactly this shape:

## Edit <N>: <short purpose>
Path: <repository-relative path>
OLD:
~~~rust
<existing lines copied verbatim from one attached excerpt>
~~~
NEW:
~~~rust
<complete replacement>
~~~

Rules:
- Copy OLD character for character from an attached excerpt, including
  indentation, blank lines and comments. Never retype, reformat, abbreviate or
  elide it. OLD must occur exactly once in the file: include enough lines to be
  unique, normally the whole function, impl item, struct or test.
- Every OLD quotes the original attached source, never the result of an
  earlier edit. No two OLD blocks overlap.
- To insert new code (a field, function or test), quote a nearby existing
  anchor as OLD and give the anchor plus the new code as NEW.
- NEW is complete code with no "...", "unchanged" or elided bodies.
- Prefer replacing whole functions and whole test functions to small fragments.
- Inside the file use only ~~~ fences, never triple backticks.
- Never edit code outside the attached excerpts. If the change needs that, add
  a final "## Coordinator follow-ups" section naming each location and reason.

The coordinator matches each OLD exactly once on the pinned base and stops on
zero or several matches, then applies the edits and runs format, compile and
the targeted tests.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-returns/bare-command-picker.edits.md ===
```markdown
<complete Markdown edit list>
```
