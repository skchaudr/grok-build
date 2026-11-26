use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;
use xai_grok_pager::piw::client::{
    ActivityKind, ConnState, Content, PiwClient, RunView, StepActivity, ToolActivity,
};
use xai_grok_pager::piw::gate::{PiwGate, answer_target, gate_from};

use crate::driver::{
    DriverEvent, FlowSketch, GateChoice, SketchEdge, SketchNode, StepKind, parse_ts,
};

const MAX_PENDING_OUTPUT_VIEWS: u8 = 3;
const PENDING_CONTENT_RETRY_TIMEOUT: Duration = Duration::from_secs(15);

pub struct PiwRunHandle {
    client: Arc<PiwClient>,
    run_id: String,
    detached: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
    /// The gate the run is parked on right now, kept current by the watch
    /// task so the answer path can pick it up (None while it runs).
    gate: Arc<Mutex<Option<PiwGate>>>,
}

impl PiwRunHandle {
    /// Ask the server to cancel the run ("run.cancel") and stop watching once the run reports a final state.
    pub fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self
            .client
            .send("run.cancel", Some(&self.run_id), None, json!({}));
        self.notify.notify_one();
    }

    /// Stop watching without touching the run.
    pub fn detach(&self) {
        self.detached.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    /// The gate the run is parked on now, if any, as last seen by the watch
    /// task. The answer path reads it to build its request.
    pub fn gate(&self) -> Option<PiwGate> {
        match self.gate.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Watch `run_id` and send its events to `tx` until the run is final (RunFinished is the last event) or the handle
/// is detached. Spawns a tokio task with `tokio::task::spawn_local` (the caller runs inside a `LocalSet`, like
/// `driver::start`). `poll_ms` is how often the client's caches are checked (use `PiwClient::generation` to skip
/// idle ticks).
pub fn watch(
    client: Arc<PiwClient>,
    run_id: &str,
    poll_ms: u64,
    tx: UnboundedSender<DriverEvent>,
) -> PiwRunHandle {
    let run_id_owned = run_id.to_string();
    let detached = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(Notify::new());
    let gate_slot = Arc::new(Mutex::new(None::<PiwGate>));

    let handle = PiwRunHandle {
        client: Arc::clone(&client),
        run_id: run_id_owned.clone(),
        detached: Arc::clone(&detached),
        cancelled: Arc::clone(&cancelled),
        notify: Arc::clone(&notify),
        gate: Arc::clone(&gate_slot),
    };

    client.watch_run(&run_id_owned);
    client.watch_activity(&run_id_owned);

    tokio::task::spawn_local(async move {
        let poll_dur = Duration::from_millis(poll_ms.max(10));
        let mut tailer = PiwTailer::new(&run_id_owned);
        let mut watched_session: Option<String> = None;
        let mut last_generation: Option<u64> = None;
        let mut last_connection_count: u64 = 0;
        let mut last_activity_seq: u64 = 0;
        let mut last_view: Option<RunView> = None;
        let mut pending_refs: Vec<(Value, Instant)> = Vec::new();

        loop {
            if detached.load(Ordering::SeqCst) || tx.is_closed() {
                break;
            }

            match client.state() {
                ConnState::Closed => break,
                ConnState::Connecting | ConnState::Reconnecting => {
                    tokio::select! {
                        () = tokio::time::sleep(poll_dur) => {}
                        () = notify.notified() => {}
                    }
                    continue;
                }
                ConnState::Connected => {}
            }

            let conn_count = client.connection_count();
            if conn_count != last_connection_count {
                last_connection_count = conn_count;
                let all = client.activity(&run_id_owned, 0);
                if all.last().is_some_and(|a| a.seq < last_activity_seq) {
                    last_activity_seq = 0;
                }
            }

            let cur_generation = client.generation();
            let content_progressed = check_pending_refs(&client, &run_id_owned, &pending_refs);
            let generation_changed = last_generation != Some(cur_generation);

            if generation_changed || content_progressed {
                last_generation = Some(cur_generation);

                // Ingest activity before the view so any chunks that arrived alongside a step's
                // start or finish are held/emitted before StepFinished.
                let activities = client.activity(&run_id_owned, last_activity_seq);
                if !activities.is_empty() {
                    if let Some(max_seq) = activities.iter().map(|a| a.seq).max() {
                        last_activity_seq = last_activity_seq.max(max_seq);
                    }
                    for ev in tailer.on_activity(&activities) {
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                }

                if let Some(rv) = client.run_view(&run_id_owned) {
                    if let Some(sid) = extract_origin_session_id(&rv.view) {
                        if watched_session.as_deref() != Some(sid.as_str()) {
                            watched_session = Some(sid.clone());
                        }
                        // A cleaned-up gate answer releases the coordinator role
                        // without dropping the snapshot. If the snapshot is gone,
                        // re-subscribe so the next gate stays visible.
                        if client.session(&sid).is_none() {
                            let _ = client.watch_session(&sid, true);
                        }
                    }

                    let view_changed = last_view.as_ref() != Some(&rv);
                    if view_changed || content_progressed {
                        let newly_pending: RefCell<Vec<Value>> = RefCell::new(Vec::new());
                        let text_of = |val: &Value| -> Option<String> {
                            resolve_client_text(&client, &run_id_owned, val, &newly_pending)
                        };
                        let events = tailer.on_view(&rv, &text_of);
                        last_view = Some(rv);

                        let now = Instant::now();
                        let next_pending = newly_pending.into_inner();
                        let mut updated_pending = Vec::with_capacity(next_pending.len());
                        for reference in next_pending {
                            let since = pending_refs
                                .iter()
                                .find(|(prev, _)| prev == &reference)
                                .map_or(now, |(_, t)| *t);
                            if !updated_pending.iter().any(|(r, _)| r == &reference) {
                                updated_pending.push((reference, since));
                            }
                        }
                        pending_refs = updated_pending;

                        for ev in events {
                            if tx.send(ev).is_err() {
                                break;
                            }
                        }
                    }
                }

                if !tailer.finished()
                    && let Some(rv) = &last_view
                    && let Some(sid) = &watched_session
                {
                    if client.session(sid).is_none() {
                        let _ = client.watch_session(sid, true);
                    }
                    if let Some(session_snapshot) = client.session(sid) {
                        let gate = gate_from(rv, &session_snapshot);
                        store_gate(&gate_slot, gate.clone());
                        for ev in tailer.on_gate(gate.as_ref()) {
                            if tx.send(ev).is_err() {
                                break;
                            }
                        }
                    }
                }

                if tailer.finished() || tx.is_closed() {
                    break;
                }
            }

            tokio::select! {
                () = tokio::time::sleep(poll_dur) => {}
                () = notify.notified() => {}
            }
        }

        client.unwatch_run(&run_id_owned);
        if let Some(sid) = watched_session {
            client.unwatch_session(&sid);
        }
    });

    handle
}

fn store_gate(slot: &Mutex<Option<PiwGate>>, gate: Option<PiwGate>) {
    match slot.lock() {
        Ok(mut guard) => *guard = gate,
        Err(poisoned) => *poisoned.into_inner() = gate,
    }
}

fn check_pending_refs(
    client: &PiwClient,
    run_id: &str,
    pending_refs: &[(Value, Instant)],
) -> bool {
    for (reference, since) in pending_refs {
        match client.content(run_id, reference) {
            Content::Ready(_) | Content::Invalid(_) => return true,
            Content::Pending => {
                if since.elapsed() >= PENDING_CONTENT_RETRY_TIMEOUT {
                    return true;
                }
            }
        }
    }
    false
}

fn resolve_client_text(
    client: &PiwClient,
    run_id: &str,
    value: &Value,
    pending_out: &RefCell<Vec<Value>>,
) -> Option<String> {
    if value.is_null() {
        return None;
    }
    if is_artifact_ref(value) {
        return match client.content(run_id, value) {
            Content::Ready(text) => Some(text),
            Content::Pending => {
                pending_out.borrow_mut().push(value.clone());
                None
            }
            Content::Invalid(_) => {
                pending_out.borrow_mut().push(value.clone());
                None
            }
        };
    }
    match unwrap_escaped(value) {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => serde_json::to_string(other).ok(),
    }
}

/// The pure core, for tests and for replaying a finished run: feed it each new view, each batch of activity and
/// the current gate; it returns the events to emit, in order. `watch` is a thin loop around it: it watches the run,
/// its activity and (without claiming the coordinator role) the run's session, and calls `gate_from`.
pub struct PiwTailer {
    run_id: String,
    run_started_emitted: bool,
    finished: bool,
    node_kinds: HashMap<String, StepKind>,
    started_attempts: HashSet<String>,
    finished_attempts: HashSet<String>,
    attempt_nodes: HashMap<String, String>,
    node_attempt_counts: HashMap<String, u64>,
    synthetic_active: Option<(String, String)>,
    attempt_aliases: HashMap<String, String>,
    shell_prepared_attempts: HashSet<String>,
    held_activity: HashMap<String, Vec<StepActivity>>,
    seen_tool_calls: HashMap<String, HashSet<String>>,
    emitted_text_attempts: HashSet<String>,
    pending_output_wait: Option<(String, u8)>,
    emitted_gate_requests: HashSet<String>,
    last_view_at_ms: Option<i64>,
}

impl PiwTailer {
    pub fn new(run_id: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            run_started_emitted: false,
            finished: false,
            node_kinds: HashMap::new(),
            started_attempts: HashSet::new(),
            finished_attempts: HashSet::new(),
            attempt_nodes: HashMap::new(),
            node_attempt_counts: HashMap::new(),
            synthetic_active: None,
            attempt_aliases: HashMap::new(),
            shell_prepared_attempts: HashSet::new(),
            held_activity: HashMap::new(),
            seen_tool_calls: HashMap::new(),
            emitted_text_attempts: HashSet::new(),
            pending_output_wait: None,
            emitted_gate_requests: HashSet::new(),
            last_view_at_ms: None,
        }
    }

    /// `text_of` turns a view value into text: a JSON string as itself, another inline value as compact JSON, a
    /// `$artifact` reference through `PiwClient::content` (None while it is still Pending or when Invalid).
    pub fn on_view(
        &mut self,
        view: &RunView,
        text_of: &dyn Fn(&Value) -> Option<String>,
    ) -> Vec<DriverEvent> {
        if self.finished {
            return Vec::new();
        }

        let raw = &view.view;
        let mut out = Vec::new();

        if let Some(ts) = extract_view_timestamp_ms(raw) {
            self.last_view_at_ms = Some(ts);
        }

        if !self.run_started_emitted {
            self.emit_run_started(raw, text_of, &mut out);
        }

        let state = raw.get("state");
        let full_steps = collect_full_steps_by_attempt(state);
        let finished_records = collect_finished_attempts(raw, &full_steps);

        for rec in finished_records {
            let Some(raw_attempt_id) = rec.attempt_id else {
                continue;
            };
            let Some(node_id) = rec.node_id else {
                continue;
            };

            let attempt_id = self.resolve_attempt_id(&node_id, &raw_attempt_id);
            if self.finished_attempts.contains(&attempt_id) {
                continue;
            }

            let kind = self.resolve_step_kind(
                &node_id,
                rec.node_type.as_deref(),
                rec.full.and_then(|s| s.get("action")),
            );
            let started_at_ms = rec
                .started_at
                .as_deref()
                .and_then(parse_ts)
                .unwrap_or_else(|| self.fallback_at_ms());
            let finished_at_ms = rec
                .finished_at
                .as_deref()
                .and_then(parse_ts)
                .unwrap_or(started_at_ms);

            let newly_started = self.started_attempts.insert(attempt_id.clone());
            if newly_started {
                self.attempt_nodes
                    .insert(attempt_id.clone(), node_id.clone());
                *self.node_attempt_counts.entry(node_id.clone()).or_insert(0) += 1;
                out.push(DriverEvent::StepStarted {
                    node: node_id.clone(),
                    attempt: attempt_id.clone(),
                    kind,
                    at_ms: started_at_ms,
                });

                // Emit AgentPrompt only when prompt text is available at the moment StepStarted is emitted
                // (never after an AgentUpdate, and never holding StepStarted back to wait for it).
                if kind == StepKind::Acp
                    && let Some(prompt_val) = rec.full.and_then(|s| s.get("prompt"))
                    && !prompt_val.is_null()
                    && let Some(prompt_text) = text_of(prompt_val)
                    && !prompt_text.is_empty()
                {
                    out.push(DriverEvent::AgentPrompt {
                        node: node_id.clone(),
                        attempt: attempt_id.clone(),
                        text: prompt_text,
                        at_ms: started_at_ms,
                    });
                }
            }

            if kind == StepKind::Acp {
                self.flush_held_activity(&attempt_id, &mut out);
            } else {
                self.held_activity.remove(&attempt_id);
            }

            let result_fallback = matching_node_result(state, &node_id, &raw_attempt_id);
            let raw_output = rec
                .full
                .and_then(|s| s.get("output"))
                .filter(|v| !v.is_null())
                .or_else(|| {
                    result_fallback
                        .and_then(|r| r.get("output"))
                        .filter(|v| !v.is_null())
                });

            let mut resolved_output_text: Option<String> = None;

            if kind == StepKind::Acp && !self.emitted_text_attempts.contains(&attempt_id) {
                if let Some(output_val) = raw_output {
                    if is_artifact_ref(output_val) {
                        match text_of(output_val) {
                            Some(text) => {
                                if self
                                    .pending_output_wait
                                    .as_ref()
                                    .is_some_and(|(id, _)| id == &attempt_id)
                                {
                                    self.pending_output_wait = None;
                                }
                                if !text.is_empty() {
                                    self.emitted_text_attempts.insert(attempt_id.clone());
                                    out.push(DriverEvent::AgentUpdate {
                                        node: node_id.clone(),
                                        attempt: attempt_id.clone(),
                                        update: agent_message_chunk_json(&text),
                                        at_ms: finished_at_ms,
                                    });
                                }
                                resolved_output_text = Some(text);
                            }
                            None => {
                                let should_hold = match &mut self.pending_output_wait {
                                    Some((id, views_waited)) if id == &attempt_id => {
                                        *views_waited = views_waited.saturating_add(1);
                                        *views_waited < MAX_PENDING_OUTPUT_VIEWS
                                    }
                                    _ => {
                                        self.pending_output_wait = Some((attempt_id.clone(), 0));
                                        true
                                    }
                                };
                                if should_hold {
                                    // Hold this StepFinished and everything after it.
                                    return out;
                                }
                                self.pending_output_wait = None;
                            }
                        }
                    } else if let Some(text) = text_of(output_val) {
                        if !text.is_empty() {
                            self.emitted_text_attempts.insert(attempt_id.clone());
                            out.push(DriverEvent::AgentUpdate {
                                node: node_id.clone(),
                                attempt: attempt_id.clone(),
                                update: agent_message_chunk_json(&text),
                                at_ms: finished_at_ms,
                            });
                        }
                        resolved_output_text = Some(text);
                    }
                }
            }

            if self
                .pending_output_wait
                .as_ref()
                .is_some_and(|(id, _)| id == &attempt_id)
            {
                self.pending_output_wait = None;
            }

            let materialized_output = match raw_output {
                None => Value::Null,
                Some(v) if is_artifact_ref(v) => {
                    let text = resolved_output_text.or_else(|| text_of(v));
                    match text {
                        Some(s) => serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s)),
                        None => Value::Null,
                    }
                }
                Some(v) => unwrap_escaped(v).clone(),
            };

            let (stdout, stderr, exit_code) = if kind == StepKind::Shell {
                let action = rec.full.and_then(|s| s.get("action"));
                if let Some(act) = action
                    && let Some(cmd) = act.get("command").and_then(Value::as_str)
                    && !cmd.is_empty()
                    && self.shell_prepared_attempts.insert(attempt_id.clone())
                {
                    let args: Vec<&str> = act
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    let command_line = format_shell_command(cmd, &args);
                    let cwd = act
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    out.push(DriverEvent::ShellPrepared {
                        node: node_id.clone(),
                        attempt: attempt_id.clone(),
                        command_line,
                        cwd,
                        at_ms: started_at_ms,
                    });
                }

                let stdout_str = materialized_output
                    .get("stdout")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| {
                        if materialized_output.is_null() {
                            None
                        } else if materialized_output.is_object()
                            && materialized_output.get("stdout").is_some()
                        {
                            None
                        } else {
                            raw_output.and_then(text_of)
                        }
                    });
                let stderr_str = materialized_output
                    .get("stderr")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let code = action
                    .and_then(|a| a.get("exitCode"))
                    .and_then(Value::as_i64)
                    .or_else(|| materialized_output.get("exitCode").and_then(Value::as_i64));
                (stdout_str, stderr_str, code)
            } else {
                (None, None, None)
            };

            let duration_ms = extract_duration_ms(
                rec.full,
                result_fallback,
                rec.started_at.as_deref().and_then(parse_ts),
                rec.finished_at.as_deref().and_then(parse_ts),
            );
            let outcome = effective_step_outcome(
                rec.outcome.as_deref().unwrap_or_default(),
                rec.full,
                &materialized_output,
            );
            let error = rec
                .full
                .and_then(|s| extract_non_empty_str(s.get("error")))
                .or_else(|| result_fallback.and_then(|r| extract_non_empty_str(r.get("error"))));

            self.finished_attempts.insert(attempt_id.clone());
            self.held_activity.remove(&attempt_id);
            out.push(DriverEvent::StepFinished {
                node: node_id,
                attempt: attempt_id,
                kind,
                outcome,
                duration_ms,
                error,
                output: materialized_output,
                stdout,
                stderr,
                exit_code,
                at_ms: finished_at_ms,
            });
        }

        let terminal_status = detect_terminal_status(raw);

        // Emit StepStarted for the currently running step (not yet in graphSteps/state.steps).
        if terminal_status.is_none()
            && let Some((curr_node, curr_attempt_opt, curr_started_at)) =
                extract_running_step(raw)
        {
            let attempt_id = match curr_attempt_opt {
                Some(real_attempt) => self.resolve_attempt_id(&curr_node, &real_attempt),
                None => {
                    if let Some((syn_node, syn_att)) = &self.synthetic_active
                        && syn_node == &curr_node
                    {
                        syn_att.clone()
                    } else {
                        let next_num = self
                            .node_attempt_counts
                            .get(&curr_node)
                            .copied()
                            .unwrap_or(0)
                            + 1;
                        let syn = format!("{curr_node}#{next_num}");
                        self.synthetic_active = Some((curr_node.clone(), syn.clone()));
                        syn
                    }
                }
            };

            if !self.finished_attempts.contains(&attempt_id)
                && self.started_attempts.insert(attempt_id.clone())
            {
                self.attempt_nodes
                    .insert(attempt_id.clone(), curr_node.clone());
                *self
                    .node_attempt_counts
                    .entry(curr_node.clone())
                    .or_insert(0) += 1;
                let kind = self.resolve_step_kind(&curr_node, None, None);
                let at_ms = curr_started_at
                    .as_deref()
                    .and_then(parse_ts)
                    .unwrap_or_else(|| self.fallback_at_ms());
                out.push(DriverEvent::StepStarted {
                    node: curr_node,
                    attempt: attempt_id.clone(),
                    kind,
                    at_ms,
                });
                if kind == StepKind::Acp {
                    self.flush_held_activity(&attempt_id, &mut out);
                } else {
                    self.held_activity.remove(&attempt_id);
                }
            }
        }

        if let Some(raw_status) = terminal_status {
            let run_error = extract_run_error(raw, text_of);
            let (status, error) = map_terminal_status(&raw_status, run_error);
            let at_ms = extract_run_finished_at_ms(raw).unwrap_or_else(|| self.fallback_at_ms());
            self.finished = true;
            self.held_activity.clear();
            out.push(DriverEvent::RunFinished {
                status,
                error,
                at_ms,
            });
        }

        out
    }

    pub fn on_activity(&mut self, activity: &[StepActivity]) -> Vec<DriverEvent> {
        if self.finished {
            return Vec::new();
        }
        let mut out = Vec::new();
        for act in activity {
            if !act.run_id.is_empty() && !self.run_id.is_empty() && act.run_id != self.run_id {
                continue;
            }
            if act.attempt_id.is_empty() {
                continue;
            }
            let attempt_id = self
                .attempt_aliases
                .get(&act.attempt_id)
                .cloned()
                .unwrap_or_else(|| act.attempt_id.clone());

            if self.finished_attempts.contains(&attempt_id) {
                continue;
            }

            if self.started_attempts.contains(&attempt_id) {
                if let Some(ev) = self.convert_activity(&attempt_id, act) {
                    out.push(ev);
                }
            } else {
                self.held_activity
                    .entry(attempt_id)
                    .or_default()
                    .push(act.clone());
            }
        }
        out
    }

    /// The gate the run is parked on now, or None. Tests build `PiwGate` values directly.
    pub fn on_gate(&mut self, gate: Option<&PiwGate>) -> Vec<DriverEvent> {
        if self.finished {
            return Vec::new();
        }
        let Some(gate) = gate else {
            return Vec::new();
        };
        if !gate.run_id.is_empty() && !self.run_id.is_empty() && gate.run_id != self.run_id {
            return Vec::new();
        }
        if !self.emitted_gate_requests.insert(gate.request_id.clone()) {
            return Vec::new();
        }

        let target_run_id = if gate.run_id.is_empty() {
            self.run_id.as_str()
        } else {
            gate.run_id.as_str()
        };
        let choices: Vec<GateChoice> = gate
            .choices
            .iter()
            .map(|c| GateChoice {
                label: if c.label.is_empty() {
                    c.key.clone()
                } else {
                    c.label.clone()
                },
                flow_file: answer_target(target_run_id, &c.key),
            })
            .collect();

        let question = if gate.question.is_empty() {
            gate.node_id.clone()
        } else {
            gate.question.clone()
        };

        vec![DriverEvent::Waiting {
            node: gate.node_id.clone(),
            attempt: gate.attempt_id.clone(),
            question,
            choices,
            at_ms: self.fallback_at_ms(),
        }]
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    fn emit_run_started(
        &mut self,
        raw: &Value,
        text_of: &dyn Fn(&Value) -> Option<String>,
        out: &mut Vec<DriverEvent>,
    ) {
        let state = raw.get("state");
        let manifest = raw.get("manifest");
        let queue = raw.get("queue");

        let run_id = extract_non_empty_str(raw.get("runId"))
            .or_else(|| state.and_then(|s| extract_non_empty_str(s.get("runId"))))
            .or_else(|| manifest.and_then(|m| extract_non_empty_str(m.get("runId"))))
            .or_else(|| queue.and_then(|q| extract_non_empty_str(q.get("runId"))))
            .unwrap_or_else(|| self.run_id.clone());
        if self.run_id.is_empty() && !run_id.is_empty() {
            self.run_id = run_id.clone();
        }

        let workflow_doc = resolve_workflow_doc(raw, text_of);
        let fallback_name = state
            .and_then(|s| extract_non_empty_str(s.get("workflowName")))
            .or_else(|| manifest.and_then(|m| extract_non_empty_str(m.get("workflowName"))))
            .or_else(|| queue.and_then(|q| extract_non_empty_str(q.get("workflowName"))))
            .unwrap_or_else(|| run_id.clone());

        let (sketch, kinds) = build_flow_sketch(workflow_doc.as_ref(), &fallback_name);
        self.node_kinds = kinds;

        let title = state
            .and_then(|s| extract_non_empty_str(s.get("runTitle")))
            .or_else(|| manifest.and_then(|m| extract_non_empty_str(m.get("runTitle"))))
            .unwrap_or_else(|| sketch.name.clone());

        let flow_path = extract_flow_path(raw);
        let at_ms = state
            .and_then(|s| s.get("startedAt").and_then(Value::as_str))
            .and_then(parse_ts)
            .or_else(|| {
                manifest
                    .and_then(|m| m.get("startedAt").and_then(Value::as_str))
                    .and_then(parse_ts)
            })
            .or_else(|| {
                queue
                    .and_then(|q| {
                        q.get("startedAt")
                            .or_else(|| q.get("createdAt"))
                            .and_then(Value::as_str)
                    })
                    .and_then(parse_ts)
            })
            .unwrap_or_else(|| self.fallback_at_ms());

        self.run_started_emitted = true;
        out.push(DriverEvent::RunStarted {
            run_id,
            run_dir: PathBuf::new(),
            title,
            flow_path,
            flow: sketch,
            at_ms,
        });
    }

    fn resolve_attempt_id(&mut self, node_id: &str, raw_attempt_id: &str) -> String {
        if let Some(aliased) = self.attempt_aliases.get(raw_attempt_id) {
            return aliased.clone();
        }
        if !self.started_attempts.contains(raw_attempt_id)
            && let Some((syn_node, syn_att)) = &self.synthetic_active
            && syn_node == node_id
        {
            let syn_att = syn_att.clone();
            self.synthetic_active = None;
            self.attempt_aliases
                .insert(raw_attempt_id.to_string(), syn_att.clone());
            return syn_att;
        }
        raw_attempt_id.to_string()
    }

    fn resolve_step_kind(
        &self,
        node_id: &str,
        node_type: Option<&str>,
        action: Option<&Value>,
    ) -> StepKind {
        if let Some(action_type) = action
            .and_then(|a| a.get("actionType"))
            .and_then(Value::as_str)
        {
            match action_type {
                "shell" => return StepKind::Shell,
                "function" => return StepKind::Compute,
                _ => {}
            }
        }
        match node_type {
            Some("agent") => StepKind::Acp,
            Some("compute") => StepKind::Compute,
            Some("checkpoint") => StepKind::Checkpoint,
            Some("action") => self
                .node_kinds
                .get(node_id)
                .copied()
                .unwrap_or(StepKind::Other),
            Some("notify") => StepKind::Other,
            Some(_) => self
                .node_kinds
                .get(node_id)
                .copied()
                .unwrap_or(StepKind::Other),
            None => self
                .node_kinds
                .get(node_id)
                .copied()
                .unwrap_or(StepKind::Other),
        }
    }

    fn flush_held_activity(&mut self, attempt_id: &str, out: &mut Vec<DriverEvent>) {
        let Some(held) = self.held_activity.remove(attempt_id) else {
            return;
        };
        for act in held {
            if let Some(ev) = self.convert_activity(attempt_id, &act) {
                out.push(ev);
            }
        }
    }

    fn convert_activity(&mut self, attempt_id: &str, act: &StepActivity) -> Option<DriverEvent> {
        let node = if !act.node_id.is_empty() {
            act.node_id.clone()
        } else {
            self.attempt_nodes
                .get(attempt_id)
                .cloned()
                .unwrap_or_default()
        };
        let at_ms = parse_ts(&act.at).unwrap_or_else(|| self.fallback_at_ms());

        match act.kind {
            ActivityKind::Text => {
                if act.text.is_empty() {
                    return None;
                }
                self.emitted_text_attempts.insert(attempt_id.to_string());
                Some(DriverEvent::AgentUpdate {
                    node,
                    attempt: attempt_id.to_string(),
                    update: agent_message_chunk_json(&act.text),
                    at_ms,
                })
            }
            ActivityKind::Thought => {
                if act.text.is_empty() {
                    return None;
                }
                Some(DriverEvent::AgentUpdate {
                    node,
                    attempt: attempt_id.to_string(),
                    update: agent_thought_chunk_json(&act.text),
                    at_ms,
                })
            }
            ActivityKind::Tool => {
                let tool = act.tool.as_ref()?;
                if tool.tool_call_id.is_empty() {
                    return None;
                }
                let is_first = self
                    .seen_tool_calls
                    .entry(attempt_id.to_string())
                    .or_default()
                    .insert(tool.tool_call_id.clone());
                let update = if is_first {
                    tool_call_json(tool)
                } else {
                    tool_call_update_json(tool)
                };
                Some(DriverEvent::AgentUpdate {
                    node,
                    attempt: attempt_id.to_string(),
                    update,
                    at_ms,
                })
            }
            ActivityKind::Status => None,
        }
    }

    fn fallback_at_ms(&self) -> i64 {
        self.last_view_at_ms.unwrap_or_else(now_ms)
    }
}

/// Non-zero `action.exitCode` / `output.exitCode`, or `output.ok == false`, is a
/// failure even when `allowFailure` leaves `status`/`outcome` as completed/ok.
fn effective_step_outcome(declared: &str, full: Option<&Value>, output: &Value) -> String {
    let status = full
        .and_then(|s| s.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let action = full.and_then(|s| s.get("action")).filter(|v| !v.is_null());
    let failed = step_reports_failure(action, output, full);
    let normalized = match declared {
        "" | "completed" => "ok",
        other => other,
    };
    if failed && (normalized == "ok" || status == "completed") {
        return "failed".to_string();
    }
    normalized.to_string()
}

fn step_reports_failure(action: Option<&Value>, output: &Value, full: Option<&Value>) -> bool {
    read_exit_code(action, output, full).is_some_and(|code| code != 0) || output_ok_is_false(output, full)
}

fn read_exit_code(action: Option<&Value>, output: &Value, full: Option<&Value>) -> Option<i64> {
    fn code_of(value: Option<&Value>) -> Option<i64> {
        value
            .and_then(|v| v.get("exitCode").or_else(|| v.get("exit_code")))
            .and_then(Value::as_i64)
    }
    code_of(action)
        .or_else(|| code_of(Some(output)))
        .or_else(|| code_of(full.and_then(|s| s.get("output"))))
        .or_else(|| code_of(full.and_then(|s| s.get("action"))))
}

fn output_ok_is_false(output: &Value, full: Option<&Value>) -> bool {
    fn not_ok(value: &Value) -> bool {
        value.get("ok").and_then(Value::as_bool) == Some(false)
    }
    not_ok(output)
        || full
            .and_then(|s| s.get("output"))
            .is_some_and(not_ok)
}

struct FinishedAttempt<'a> {
    attempt_id: Option<String>,
    node_id: Option<String>,
    node_type: Option<String>,
    outcome: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    full: Option<&'a Value>,
}

fn collect_full_steps_by_attempt<'a>(state: Option<&'a Value>) -> HashMap<String, &'a Value> {
    let mut map = HashMap::new();
    let Some(steps) = state
        .and_then(|s| s.get("steps"))
        .and_then(Value::as_array)
    else {
        return map;
    };
    for step in steps {
        if let Some(id) = extract_non_empty_str(step.get("attemptId")) {
            map.insert(id, step);
        }
    }
    map
}

fn collect_finished_attempts<'a>(
    raw: &'a Value,
    full_steps: &HashMap<String, &'a Value>,
) -> Vec<FinishedAttempt<'a>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    if let Some(graph_steps) = raw.get("graphSteps").and_then(Value::as_array) {
        for compact in graph_steps {
            let attempt_id = extract_non_empty_str(compact.get("attemptId"));
            if let Some(ref id) = attempt_id
                && !seen.insert(id.clone())
            {
                continue;
            }
            let full = attempt_id
                .as_deref()
                .and_then(|id| full_steps.get(id).copied());
            let node_id = full
                .and_then(|s| extract_non_empty_str(s.get("nodeId")))
                .or_else(|| extract_non_empty_str(compact.get("nodeId")));
            let node_type = full
                .and_then(|s| extract_non_empty_str(s.get("nodeType")))
                .or_else(|| extract_non_empty_str(compact.get("nodeType")));
            let outcome = full
                .and_then(|s| extract_non_empty_str(s.get("outcome")))
                .or_else(|| extract_non_empty_str(compact.get("outcome")));
            let started_at = full
                .and_then(|s| extract_non_empty_str(s.get("startedAt")))
                .or_else(|| extract_non_empty_str(compact.get("startedAt")));
            let finished_at = full
                .and_then(|s| extract_non_empty_str(s.get("finishedAt")))
                .or_else(|| extract_non_empty_str(compact.get("finishedAt")));

            out.push(FinishedAttempt {
                attempt_id,
                node_id,
                node_type,
                outcome,
                started_at,
                finished_at,
                full,
            });
        }
    }

    if let Some(steps) = raw
        .get("state")
        .and_then(|s| s.get("steps"))
        .and_then(Value::as_array)
    {
        for step in steps {
            let attempt_id = extract_non_empty_str(step.get("attemptId"));
            if let Some(ref id) = attempt_id
                && !seen.insert(id.clone())
            {
                continue;
            }
            out.push(FinishedAttempt {
                attempt_id,
                node_id: extract_non_empty_str(step.get("nodeId")),
                node_type: extract_non_empty_str(step.get("nodeType")),
                outcome: extract_non_empty_str(step.get("outcome")),
                started_at: extract_non_empty_str(step.get("startedAt")),
                finished_at: extract_non_empty_str(step.get("finishedAt")),
                full: Some(step),
            });
        }
    }

    out
}

fn matching_node_result<'a>(
    state: Option<&'a Value>,
    node_id: &str,
    attempt_id: &str,
) -> Option<&'a Value> {
    let res = state
        .and_then(|s| s.get("results"))
        .and_then(|r| r.get(node_id))?;
    if res.get("attemptId").and_then(Value::as_str) == Some(attempt_id) {
        Some(res)
    } else {
        None
    }
}

fn extract_running_step(raw: &Value) -> Option<(String, Option<String>, Option<String>)> {
    let state = raw.get("state")?;
    let current_node = extract_non_empty_str(state.get("currentNode")).or_else(|| {
        let display = raw.get("display")?;
        let is_running = display.get("status").and_then(Value::as_str) == Some("running");
        let is_origin_turn = display.get("activity").and_then(Value::as_str) == Some("origin_turn");
        if is_running && is_origin_turn {
            extract_non_empty_str(state.get("waitingOn"))
        } else {
            None
        }
    })?;
    let attempt_id = extract_non_empty_str(state.get("currentAttemptId"));
    let started_at = extract_non_empty_str(state.get("currentNodeStartedAt"));
    Some((current_node, attempt_id, started_at))
}

fn detect_terminal_status(raw: &Value) -> Option<String> {
    let status = raw
        .get("display")
        .and_then(|d| extract_non_empty_str(d.get("status")))
        .or_else(|| {
            raw.get("state")
                .and_then(|s| extract_non_empty_str(s.get("status")))
        })
        .or_else(|| {
            raw.get("manifest")
                .and_then(|m| extract_non_empty_str(m.get("status")))
        })
        .or_else(|| {
            raw.get("queue")
                .and_then(|q| extract_non_empty_str(q.get("status")))
        })?;

    match status.as_str() {
        "queued" | "starting" | "running" | "waiting" | "paused" | "parked" | "ambiguous" => None,
        _ => Some(status),
    }
}

fn map_terminal_status(raw_status: &str, run_error: Option<String>) -> (String, Option<String>) {
    match raw_status {
        "completed" | "done" => ("completed".to_string(), None),
        "failed" => ("failed".to_string(), run_error),
        "cancelled" => ("cancelled".to_string(), run_error),
        other => {
            let err = match run_error.filter(|s| !s.is_empty()) {
                Some(detail) => format!("{other}: {detail}"),
                None => other.to_string(),
            };
            ("failed".to_string(), Some(err))
        }
    }
}

fn extract_run_error(
    raw: &Value,
    text_of: &dyn Fn(&Value) -> Option<String>,
) -> Option<String> {
    if let Some(err) = raw
        .get("state")
        .and_then(|s| extract_non_empty_str(s.get("error")))
    {
        return Some(err);
    }
    if let Some(display) = raw.get("display") {
        if let Some(rc) = display.get("reasonContent")
            && !rc.is_null()
            && let Some(text) = text_of(rc)
            && !text.is_empty()
        {
            return Some(text);
        }
        if let Some(reason) = extract_non_empty_str(display.get("reason")) {
            return Some(reason);
        }
    }
    raw.get("queue")
        .and_then(|q| extract_non_empty_str(q.get("errorCode")))
}

fn extract_run_finished_at_ms(raw: &Value) -> Option<i64> {
    let candidates = [
        raw.pointer("/state/finishedAt"),
        raw.pointer("/manifest/finishedAt"),
        raw.pointer("/queue/finishedAt"),
        raw.pointer("/state/updatedAt"),
        raw.pointer("/queue/updatedAt"),
    ];
    for candidate in candidates {
        if let Some(ts) = candidate.and_then(Value::as_str).and_then(parse_ts) {
            return Some(ts);
        }
    }
    None
}

fn extract_view_timestamp_ms(raw: &Value) -> Option<i64> {
    let candidates = [
        raw.pointer("/state/updatedAt"),
        raw.pointer("/state/finishedAt"),
        raw.pointer("/queue/updatedAt"),
        raw.pointer("/state/startedAt"),
        raw.pointer("/manifest/startedAt"),
        raw.pointer("/queue/createdAt"),
    ];
    for candidate in candidates {
        if let Some(ts) = candidate.and_then(Value::as_str).and_then(parse_ts) {
            return Some(ts);
        }
    }
    None
}

fn extract_origin_session_id(raw: &Value) -> Option<String> {
    extract_non_empty_str(raw.pointer("/queue/originSessionId"))
        .or_else(|| extract_non_empty_str(raw.pointer("/session/binding/piSessionId")))
        // Headless runs (the CLI `start` command) record their session only here.
        .or_else(|| extract_non_empty_str(raw.pointer("/followUpQueue/originSessionId")))
}

fn extract_flow_path(raw: &Value) -> String {
    for source in [
        raw.pointer("/manifest/workflowSource"),
        raw.pointer("/state/workflowSource"),
    ] {
        if let Some(src) = source.and_then(Value::as_object) {
            match src.get("kind").and_then(Value::as_str) {
                Some("file") => {
                    if let Some(p) = extract_non_empty_str(src.get("path")) {
                        return p;
                    }
                }
                Some("builtin") => {
                    if let Some(id) = extract_non_empty_str(src.get("id")) {
                        if let Some(rev) = extract_non_empty_str(src.get("revision")) {
                            return format!("builtin:{id}@{rev}");
                        }
                        return format!("builtin:{id}");
                    }
                }
                _ => {}
            }
        }
    }
    extract_non_empty_str(raw.pointer("/queue/workflowSourceRef"))
        .or_else(|| extract_non_empty_str(raw.pointer("/state/workflowPath")))
        .unwrap_or_default()
}

fn extract_duration_ms(
    full: Option<&Value>,
    result_fallback: Option<&Value>,
    started_ms: Option<i64>,
    finished_ms: Option<i64>,
) -> u64 {
    if let Some(d) = full
        .and_then(|s| s.pointer("/action/durationMs"))
        .and_then(json_non_negative_u64)
    {
        return d;
    }
    if let Some(d) = result_fallback
        .and_then(|r| r.get("durationMs"))
        .and_then(json_non_negative_u64)
    {
        return d;
    }
    if let (Some(start), Some(end)) = (started_ms, finished_ms) {
        return end.saturating_sub(start).max(0) as u64;
    }
    0
}

fn json_non_negative_u64(v: &Value) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        return Some(u);
    }
    if let Some(f) = v.as_f64()
        && f.is_finite()
        && f >= 0.0
    {
        return Some(f.round() as u64);
    }
    None
}

fn resolve_workflow_doc(
    raw: &Value,
    text_of: &dyn Fn(&Value) -> Option<String>,
) -> Option<Value> {
    let wf = raw.get("workflow")?;
    if is_artifact_ref(wf) {
        return text_of(wf).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    }
    if let Some(content_ref) = wf.get("content")
        && is_artifact_ref(content_ref)
        && let Some(full_text) = text_of(content_ref)
        && let Ok(parsed) = serde_json::from_str::<Value>(&full_text)
    {
        return Some(parsed);
    }
    Some(wf.clone())
}

fn classify_node_kind(node_val: &Value) -> StepKind {
    match node_val.get("nodeType").and_then(Value::as_str) {
        Some("agent") => StepKind::Acp,
        Some("compute") => StepKind::Compute,
        Some("checkpoint") => StepKind::Checkpoint,
        Some("action") => match node_val.get("actionExecution").and_then(Value::as_str) {
            Some("shell") => StepKind::Shell,
            Some("function") => StepKind::Compute,
            _ => StepKind::Other,
        },
        Some("notify") => StepKind::Other,
        _ => {
            if node_val
                .get("humanDecision")
                .is_some_and(|v| !v.is_null())
            {
                StepKind::Checkpoint
            } else {
                StepKind::Other
            }
        }
    }
}

fn build_flow_sketch(
    workflow_val: Option<&Value>,
    fallback_name: &str,
) -> (FlowSketch, HashMap<String, StepKind>) {
    let name = workflow_val
        .and_then(|w| extract_non_empty_str(w.get("name")))
        .unwrap_or_else(|| fallback_name.to_string());
    let start_at = workflow_val
        .and_then(|w| extract_non_empty_str(w.get("startAt")))
        .unwrap_or_default();

    let mut nodes_map: BTreeMap<String, SketchNode> = BTreeMap::new();
    let mut kinds_map: HashMap<String, StepKind> = HashMap::new();

    if let Some(obj) = workflow_val
        .and_then(|w| w.get("nodes"))
        .and_then(Value::as_object)
    {
        for (id, nval) in obj {
            let kind = classify_node_kind(nval);
            let summary = extract_non_empty_str(nval.get("summary"));
            let mut profile = extract_non_empty_str(nval.get("agent"))
                .or_else(|| extract_non_empty_str(nval.get("profile")));
            // Pi agent nodes omit `agent`; the card must still read "pi".
            if profile.is_none() && kind == StepKind::Acp {
                profile = Some("pi".to_string());
            }
            let model = extract_non_empty_str(nval.get("model"))
                .or_else(|| extract_non_empty_str(nval.get("modelId")));
            let effort = nval
                .get("configOptions")
                .and_then(Value::as_array)
                .and_then(|arr| {
                    arr.iter().find_map(|entry| {
                        if entry.get("configId").and_then(Value::as_str)
                            == Some("reasoning_effort")
                        {
                            extract_non_empty_str(entry.get("value"))
                        } else {
                            None
                        }
                    })
                });
            kinds_map.insert(id.clone(), kind);
            nodes_map.insert(
                id.clone(),
                SketchNode {
                    id: id.clone(),
                    kind,
                    summary,
                    profile,
                    model,
                    effort,
                },
            );
        }
    }

    let mut edges: Vec<SketchEdge> = Vec::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    if let Some(edge_arr) = workflow_val
        .and_then(|w| w.get("edges"))
        .and_then(Value::as_array)
    {
        for eval in edge_arr {
            let Some(from) = extract_non_empty_str(eval.get("from")) else {
                continue;
            };
            if let Some(to) = extract_non_empty_str(eval.get("to")) {
                adj.entry(from.clone()).or_default().push(to.clone());
                edges.push(SketchEdge {
                    from,
                    to,
                    label: None,
                });
            } else if let Some(cases) = eval
                .get("switch")
                .and_then(|s| s.get("cases"))
                .and_then(Value::as_object)
            {
                for (label, target_val) in cases {
                    if let Some(to) = target_val.as_str().filter(|s| !s.is_empty()) {
                        adj.entry(from.clone()).or_default().push(to.to_string());
                        edges.push(SketchEdge {
                            from: from.clone(),
                            to: to.to_string(),
                            label: Some(label.clone()),
                        });
                    }
                }
            }
        }
    }

    let mut ordered_nodes = Vec::with_capacity(nodes_map.len());
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    if !start_at.is_empty() && nodes_map.contains_key(&start_at) {
        visited.insert(start_at.clone());
        queue.push_back(start_at.clone());
    }

    while let Some(curr) = queue.pop_front() {
        if let Some(node) = nodes_map.get(&curr) {
            ordered_nodes.push(node.clone());
        }
        if let Some(neighbors) = adj.get(&curr) {
            for nb in neighbors {
                if nodes_map.contains_key(nb) && visited.insert(nb.clone()) {
                    queue.push_back(nb.clone());
                }
            }
        }
    }

    for (id, node) in &nodes_map {
        if !visited.contains(id) {
            ordered_nodes.push(node.clone());
        }
    }

    (
        FlowSketch {
            name,
            start_at,
            nodes: ordered_nodes,
            edges,
        },
        kinds_map,
    )
}

fn agent_message_chunk_json(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": {
            "type": "text",
            "text": text,
        }
    })
}

fn agent_thought_chunk_json(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_thought_chunk",
        "content": {
            "type": "text",
            "text": text,
        }
    })
}

fn tool_call_json(tool: &ToolActivity) -> Value {
    let mut obj = Map::new();
    obj.insert(
        "sessionUpdate".to_string(),
        Value::String("tool_call".to_string()),
    );
    obj.insert(
        "toolCallId".to_string(),
        Value::String(tool.tool_call_id.clone()),
    );
    let title = if tool.title.is_empty() {
        tool.tool_call_id.clone()
    } else {
        tool.title.clone()
    };
    obj.insert("title".to_string(), Value::String(title));
    obj.insert(
        "kind".to_string(),
        Value::String(normalize_tool_kind(tool.kind.as_deref()).to_string()),
    );
    obj.insert(
        "status".to_string(),
        Value::String(
            tool.status
                .as_deref()
                .and_then(normalize_tool_status)
                .unwrap_or("in_progress")
                .to_string(),
        ),
    );
    if let Some(input) = &tool.input {
        obj.insert("rawInput".to_string(), input.clone());
    }
    if let Some(output) = &tool.output {
        obj.insert("rawOutput".to_string(), output.clone());
        if let Some(content) = tool_output_content(output) {
            obj.insert("content".to_string(), content);
        }
    }
    Value::Object(obj)
}

fn tool_call_update_json(tool: &ToolActivity) -> Value {
    let mut obj = Map::new();
    obj.insert(
        "sessionUpdate".to_string(),
        Value::String("tool_call_update".to_string()),
    );
    obj.insert(
        "toolCallId".to_string(),
        Value::String(tool.tool_call_id.clone()),
    );
    if !tool.title.is_empty() {
        obj.insert("title".to_string(), Value::String(tool.title.clone()));
    }
    if let Some(kind) = tool.kind.as_deref() {
        obj.insert(
            "kind".to_string(),
            Value::String(normalize_tool_kind(Some(kind)).to_string()),
        );
    }
    if let Some(status) = tool.status.as_deref().and_then(normalize_tool_status) {
        obj.insert("status".to_string(), Value::String(status.to_string()));
    }
    if let Some(input) = &tool.input {
        obj.insert("rawInput".to_string(), input.clone());
    }
    if let Some(output) = &tool.output {
        obj.insert("rawOutput".to_string(), output.clone());
        if let Some(content) = tool_output_content(output) {
            obj.insert("content".to_string(), content);
        }
    }
    Value::Object(obj)
}

fn tool_output_content(output: &Value) -> Option<Value> {
    let text = match output {
        Value::Null => return None,
        Value::String(s) => {
            if s.is_empty() {
                return None;
            }
            s.clone()
        }
        other => serde_json::to_string(other).ok()?,
    };
    Some(json!([{
        "type": "content",
        "content": {
            "type": "text",
            "text": text,
        }
    }]))
}

fn normalize_tool_kind(kind: Option<&str>) -> &'static str {
    let Some(k) = kind else {
        return "other";
    };
    match k.to_ascii_lowercase().as_str() {
        "read" => "read",
        "edit" | "write" => "edit",
        "delete" | "remove" => "delete",
        "move" | "rename" => "move",
        "search" | "grep" | "find" | "glob" => "search",
        "execute" | "bash" | "shell" | "command" => "execute",
        "think" => "think",
        "fetch" | "http" | "web" => "fetch",
        "switch_mode" => "switch_mode",
        "other" => "other",
        _ => "other",
    }
}

fn normalize_tool_status(status: &str) -> Option<&'static str> {
    match status.to_ascii_lowercase().as_str() {
        "pending" => Some("pending"),
        "in_progress" | "running" => Some("in_progress"),
        "completed" | "ok" | "done" => Some("completed"),
        "failed" | "error" | "timed_out" | "cancelled" => Some("failed"),
        _ => None,
    }
}

fn is_artifact_ref(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.len() != 1 {
        return false;
    }
    obj.get("$artifact")
        .and_then(Value::as_object)
        .and_then(|a| a.get("path"))
        .and_then(Value::as_str)
        .is_some()
}

fn unwrap_escaped(value: &Value) -> &Value {
    let Some(obj) = value.as_object() else {
        return value;
    };
    if obj.len() == 1
        && let Some(inner) = obj.get("$escaped")
    {
        return inner;
    }
    value
}

fn extract_non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn format_shell_command(command: &str, args: &[&str]) -> String {
    let mut parts = Vec::with_capacity(1 + args.len());
    if !command.is_empty() {
        parts.push(shell_quote_if_needed(command));
    }
    for &arg in args {
        parts.push(shell_quote_if_needed(arg));
    }
    parts.join(" ")
}

fn shell_quote_if_needed(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .any(|c| c.is_whitespace() || c == '\'' || c == '"')
    {
        let escaped = s.replace('\'', "'\\''");
        format!("'{escaped}'")
    } else {
        s.to_string()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Out, Projector};
    use xai_grok_pager::piw::gate::{GateKind, PiwGateChoice};

    fn inline_text_of(value: &Value) -> Option<String> {
        if value.is_null() || is_artifact_ref(value) {
            return None;
        }
        match unwrap_escaped(value) {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => serde_json::to_string(other).ok(),
        }
    }

    fn make_view(revision: u64, view: Value) -> RunView {
        RunView {
            revision,
            run_revision: revision,
            view,
        }
    }

    fn three_step_workflow() -> Value {
        json!({
            "schema": "pi-workflows.definition-snapshot.v1",
            "name": "ship-pr",
            "startAt": "extract",
            "nodes": {
                "extract": { "nodeType": "action", "actionExecution": "shell" },
                "review": { "nodeType": "agent", "agent": "grok-build", "model": "grok-4" },
                "summarize": { "nodeType": "compute" }
            },
            "edges": [
                { "from": "extract", "to": "review" },
                { "from": "review", "to": "summarize" }
            ]
        })
    }

    fn base_run_view(
        status: &str,
        graph_steps: Value,
        full_steps: Value,
        current_node: Option<&str>,
        current_attempt: Option<&str>,
        current_started_at: Option<&str>,
    ) -> Value {
        let mut state = json!({
            "schema": "pi-workflows.run-state.v1",
            "traceSeq": 10,
            "runId": "run-1",
            "workflowName": "ship-pr",
            "runTitle": "Ship PR #42",
            "workflowSource": { "kind": "file", "path": "/repo/ship.workflow.ts", "hash": "abc" },
            "startedAt": "2026-11-20T01:00:00.000Z",
            "updatedAt": "2026-11-20T01:00:10.000Z",
            "status": status,
            "input": {},
            "outputs": {},
            "results": {},
            "steps": full_steps
        });
        if let Some(obj) = state.as_object_mut() {
            if let Some(n) = current_node {
                obj.insert("currentNode".to_string(), json!(n));
            }
            if let Some(a) = current_attempt {
                obj.insert("currentAttemptId".to_string(), json!(a));
            }
            if let Some(t) = current_started_at {
                obj.insert("currentNodeStartedAt".to_string(), json!(t));
            }
            if matches!(status, "completed" | "failed" | "cancelled" | "timed_out") {
                obj.insert("finishedAt".to_string(), json!("2026-11-20T01:00:10.000Z"));
            }
        }
        json!({
            "schema": "pi-workflows.run-view.v1",
            "runId": "run-1",
            "revision": 1,
            "runRevision": 1,
            "display": {
                "status": status,
                "activity": if status == "running" { json!("supervised_runner") } else { Value::Null },
                "controls": [],
                "reason": Value::Null
            },
            "manifest": {
                "schema": "pi-workflows.run-manifest.v1",
                "runId": "run-1",
                "workflowName": "ship-pr",
                "workflowSource": { "kind": "file", "path": "/repo/ship.workflow.ts", "hash": "abc" },
                "startedAt": "2026-11-20T01:00:00.000Z",
                "status": status
            },
            "state": state,
            "workflow": three_step_workflow(),
            "queue": {
                "runId": "run-1",
                "workflowName": "ship-pr",
                "workflowSourceRef": "/repo/ship.workflow.ts",
                "originSessionId": "sess-1",
                "status": if status == "completed" { "done" } else { status },
                "createdAt": "2026-11-20T01:00:00.000Z",
                "updatedAt": "2026-11-20T01:00:10.000Z",
                "startedAt": "2026-11-20T01:00:00.000Z"
            },
            "graphSteps": graph_steps
        })
    }

    fn step_compact(
        attempt_id: &str,
        node_id: &str,
        node_type: &str,
        outcome: &str,
        started_at: &str,
        finished_at: &str,
    ) -> Value {
        json!({
            "attemptId": attempt_id,
            "nodeId": node_id,
            "nodeType": node_type,
            "outcome": outcome,
            "startedAt": started_at,
            "finishedAt": finished_at,
            "prompt": Value::Null,
            "output": Value::Null
        })
    }

    fn step_full_shell() -> Value {
        json!({
            "attemptId": "a1",
            "nodeId": "extract",
            "nodeType": "action",
            "outcome": "ok",
            "startedAt": "2026-11-20T01:00:01.000Z",
            "finishedAt": "2026-11-20T01:00:02.000Z",
            "prompt": Value::Null,
            "output": { "stdout": "diff --git\n", "stderr": "", "exitCode": 0 },
            "action": {
                "actionType": "shell",
                "command": "git",
                "args": ["diff", "HEAD~1"],
                "cwd": "/repo",
                "exitCode": 0,
                "durationMs": 1000
            }
        })
    }

    fn step_full_agent() -> Value {
        json!({
            "attemptId": "a2",
            "nodeId": "review",
            "nodeType": "agent",
            "outcome": "ok",
            "startedAt": "2026-11-20T01:00:03.000Z",
            "finishedAt": "2026-11-20T01:00:06.000Z",
            "prompt": "Review this diff carefully.",
            "output": "Looks good to ship."
        })
    }

    fn step_full_compute() -> Value {
        json!({
            "attemptId": "a3",
            "nodeId": "summarize",
            "nodeType": "compute",
            "outcome": "ok",
            "startedAt": "2026-11-20T01:00:07.000Z",
            "finishedAt": "2026-11-20T01:00:08.000Z",
            "prompt": Value::Null,
            "output": { "ready": true }
        })
    }

    #[test]
    fn three_step_run_view_by_view_and_single_final_view() {
        // 1. Delivered view by view (with live activity for the agent step)
        let mut live = PiwTailer::new("run-1");
        let mut live_events = Vec::new();

        // View 1: extract running
        live_events.extend(live.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("extract"),
                    Some("a1"),
                    Some("2026-11-20T01:00:01.000Z"),
                ),
            ),
            &inline_text_of,
        ));
        assert!(!live.finished());

        // View 2: extract finished, review running
        let c1 = step_compact(
            "a1",
            "extract",
            "action",
            "ok",
            "2026-11-20T01:00:01.000Z",
            "2026-11-20T01:00:02.000Z",
        );
        live_events.extend(live.on_view(
            &make_view(
                2,
                base_run_view(
                    "running",
                    json!([c1]),
                    json!([step_full_shell()]),
                    Some("review"),
                    Some("a2"),
                    Some("2026-11-20T01:00:03.000Z"),
                ),
            ),
            &inline_text_of,
        ));

        // Live activity for review
        live_events.extend(live.on_activity(&[StepActivity {
            run_id: "run-1".to_string(),
            node_id: "review".to_string(),
            attempt_id: "a2".to_string(),
            seq: 1,
            at: "2026-11-20T01:00:04.000Z".to_string(),
            kind: ActivityKind::Text,
            text: "Looks good to ship.".to_string(),
            tool: None,
        }]));

        // View 3: review finished, summarize running
        let c2 = step_compact(
            "a2",
            "review",
            "agent",
            "ok",
            "2026-11-20T01:00:03.000Z",
            "2026-11-20T01:00:06.000Z",
        );
        live_events.extend(live.on_view(
            &make_view(
                3,
                base_run_view(
                    "running",
                    json!([c1, c2]),
                    json!([step_full_shell(), step_full_agent()]),
                    Some("summarize"),
                    Some("a3"),
                    Some("2026-11-20T01:00:07.000Z"),
                ),
            ),
            &inline_text_of,
        ));

        // View 4: all finished
        let c3 = step_compact(
            "a3",
            "summarize",
            "compute",
            "ok",
            "2026-11-20T01:00:07.000Z",
            "2026-11-20T01:00:08.000Z",
        );
        let final_view = make_view(
            4,
            base_run_view(
                "completed",
                json!([c1, c2, c3]),
                json!([step_full_shell(), step_full_agent(), step_full_compute()]),
                None,
                None,
                None,
            ),
        );
        live_events.extend(live.on_view(&final_view, &inline_text_of));
        assert!(live.finished());

        // Live run had no AgentPrompt because review was seen while running.
        assert!(!live_events.iter().any(|e| matches!(e, DriverEvent::AgentPrompt { .. })));

        // 2. Same run delivered as one final view
        let mut replay = PiwTailer::new("run-1");
        let replay_events = replay.on_view(&final_view, &inline_text_of);
        assert!(replay.finished());

        // Replay has AgentPrompt for review right after StepStarted(review).
        let prompt_idx = replay_events
            .iter()
            .position(|e| matches!(e, DriverEvent::AgentPrompt { node, text, .. } if node == "review" && text == "Review this diff carefully."));
        assert!(prompt_idx.is_some());
        if let Some(idx) = prompt_idx {
            assert!(matches!(
                replay_events.get(idx.saturating_sub(1)),
                Some(DriverEvent::StepStarted { node, kind: StepKind::Acp, .. }) if node == "review"
            ));
        }

        // Compare the StepStarted / StepFinished / RunFinished skeleton between live and replay.
        let skeleton = |evs: &[DriverEvent]| -> Vec<String> {
            evs.iter()
                .filter_map(|e| match e {
                    DriverEvent::RunStarted { run_id, .. } => Some(format!("run_start:{run_id}")),
                    DriverEvent::StepStarted { node, attempt, kind, .. } => {
                        Some(format!("step_start:{node}:{attempt}:{kind:?}"))
                    }
                    DriverEvent::StepFinished { node, attempt, kind, outcome, .. } => {
                        Some(format!("step_fin:{node}:{attempt}:{kind:?}:{outcome}"))
                    }
                    DriverEvent::RunFinished { status, .. } => Some(format!("run_fin:{status}")),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(skeleton(&live_events), skeleton(&replay_events));
        assert_eq!(
            skeleton(&replay_events),
            vec![
                "run_start:run-1",
                "step_start:extract:a1:Shell",
                "step_fin:extract:a1:Shell:ok",
                "step_start:review:a2:Acp",
                "step_fin:review:a2:Acp:ok",
                "step_start:summarize:a3:Compute",
                "step_fin:summarize:a3:Compute:ok",
                "run_fin:completed",
            ]
        );
    }

    #[test]
    fn running_step_known_only_from_current_node() {
        // Case A: currentNode + currentAttemptId (no graphSteps/state.steps yet)
        let mut t = PiwTailer::new("run-1");
        let evs = t.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    Some("att-live"),
                    Some("2026-11-20T01:00:05.000Z"),
                ),
            ),
            &inline_text_of,
        );
        assert_eq!(evs.len(), 2);
        assert!(matches!(
            evs.get(1),
            Some(DriverEvent::StepStarted { node, attempt, kind: StepKind::Acp, .. })
                if node == "review" && attempt == "att-live"
        ));

        // Case B: currentNode without currentAttemptId, later finishes with a real attemptId
        let mut t2 = PiwTailer::new("run-1");
        let evs1 = t2.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    None,
                    Some("2026-11-20T01:00:05.000Z"),
                ),
            ),
            &inline_text_of,
        );
        let started_attempt = match evs1.get(1) {
            Some(DriverEvent::StepStarted { node, attempt, kind: StepKind::Acp, .. })
                if node == "review" =>
            {
                attempt.clone()
            }
            other => panic!("expected StepStarted for review, got {other:?}"),
        };
        // Repeating the same view emits nothing.
        let evs_dup = t2.on_view(
            &make_view(
                2,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    None,
                    Some("2026-11-20T01:00:05.000Z"),
                ),
            ),
            &inline_text_of,
        );
        assert!(evs_dup.is_empty());

        // Finishing aliases real attemptId back to the started synthetic attempt so it closes cleanly.
        let c2 = step_compact(
            "a2",
            "review",
            "agent",
            "ok",
            "2026-11-20T01:00:05.000Z",
            "2026-11-20T01:00:06.000Z",
        );
        let evs2 = t2.on_view(
            &make_view(
                3,
                base_run_view(
                    "completed",
                    json!([c2]),
                    json!([step_full_agent()]),
                    None,
                    None,
                    None,
                ),
            ),
            &inline_text_of,
        );
        assert!(!evs2.iter().any(|e| matches!(e, DriverEvent::StepStarted { .. })));
        assert!(evs2.iter().any(|e| matches!(
            e,
            DriverEvent::StepFinished { node, attempt, .. }
                if node == "review" && attempt == &started_attempt
        )));
    }

    #[test]
    fn attempt_in_graph_steps_outside_state_steps_page() {
        let mut t = PiwTailer::new("run-1");
        let c1 = step_compact(
            "a1",
            "extract",
            "action",
            "ok",
            "2026-11-20T01:00:01.000Z",
            "2026-11-20T01:00:02.000Z",
        );
        let c2 = step_compact(
            "a2",
            "review",
            "agent",
            "ok",
            "2026-11-20T01:00:03.000Z",
            "2026-11-20T01:00:06.000Z",
        );
        // state.steps only contains a2 (a1 fell off the byte-bounded page).
        let evs = t.on_view(
            &make_view(
                1,
                base_run_view(
                    "completed",
                    json!([c1, c2]),
                    json!([step_full_agent()]),
                    None,
                    None,
                    None,
                ),
            ),
            &inline_text_of,
        );
        assert!(t.finished());
        assert!(evs.iter().any(|e| matches!(
            e,
            DriverEvent::StepStarted { node, attempt, kind: StepKind::Shell, .. }
                if node == "extract" && attempt == "a1"
        )));
        assert!(evs.iter().any(|e| matches!(
            e,
            DriverEvent::StepFinished { node, attempt, kind: StepKind::Shell, output, duration_ms: 1000, .. }
                if node == "extract" && attempt == "a1" && output.is_null()
        )));
        assert!(evs.iter().any(|e| matches!(
            e,
            DriverEvent::AgentPrompt { node, attempt, text, .. }
                if node == "review" && attempt == "a2" && text == "Review this diff carefully."
        )));
    }

    #[test]
    fn activity_before_view_is_held_and_flushed_after_step_started() {
        let mut t = PiwTailer::new("run-1");
        let early = t.on_activity(&[
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 1,
                at: "2026-11-20T01:00:03.100Z".to_string(),
                kind: ActivityKind::Text,
                text: "First ".to_string(),
                tool: None,
            },
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 2,
                at: "2026-11-20T01:00:03.200Z".to_string(),
                kind: ActivityKind::Text,
                text: "Second".to_string(),
                tool: None,
            },
        ]);
        assert!(early.is_empty());

        let evs = t.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    Some("a2"),
                    Some("2026-11-20T01:00:03.000Z"),
                ),
            ),
            &inline_text_of,
        );
        assert_eq!(evs.len(), 4);
        assert!(matches!(evs.first(), Some(DriverEvent::RunStarted { .. })));
        assert!(matches!(
            evs.get(1),
            Some(DriverEvent::StepStarted { node, attempt, .. }) if node == "review" && attempt == "a2"
        ));
        assert!(matches!(
            evs.get(2),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.pointer("/content/text").and_then(Value::as_str) == Some("First ")
        ));
        assert!(matches!(
            evs.get(3),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.pointer("/content/text").and_then(Value::as_str) == Some("Second")
        ));
    }

    #[test]
    fn text_chunks_separate_in_order_thought_forwarded_status_dropped() {
        let mut t = PiwTailer::new("run-1");
        t.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    Some("a2"),
                    Some("2026-11-20T01:00:03.000Z"),
                ),
            ),
            &inline_text_of,
        );

        let evs = t.on_activity(&[
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 1,
                at: "2026-11-20T01:00:03.100Z".to_string(),
                kind: ActivityKind::Thought,
                text: "Checking imports".to_string(),
                tool: None,
            },
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 2,
                at: "2026-11-20T01:00:03.200Z".to_string(),
                kind: ActivityKind::Status,
                text: "Reading files...".to_string(),
                tool: None,
            },
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 3,
                at: "2026-11-20T01:00:03.300Z".to_string(),
                kind: ActivityKind::Text,
                text: "Part 1".to_string(),
                tool: None,
            },
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 4,
                at: "2026-11-20T01:00:03.400Z".to_string(),
                kind: ActivityKind::Text,
                text: " + Part 2".to_string(),
                tool: None,
            },
        ]);

        assert_eq!(evs.len(), 3);
        assert!(matches!(
            evs.first(),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("agent_thought_chunk")
                && update.pointer("/content/text").and_then(Value::as_str) == Some("Checking imports")
        ));
        assert!(matches!(
            evs.get(1),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk")
                && update.pointer("/content/text").and_then(Value::as_str) == Some("Part 1")
        ));
        assert!(matches!(
            evs.get(2),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk")
                && update.pointer("/content/text").and_then(Value::as_str) == Some(" + Part 2")
        ));
    }

    #[test]
    fn tool_call_then_tool_call_update() {
        let mut t = PiwTailer::new("run-1");
        t.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    Some("a2"),
                    Some("2026-11-20T01:00:03.000Z"),
                ),
            ),
            &inline_text_of,
        );

        let evs = t.on_activity(&[
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 1,
                at: "2026-11-20T01:00:03.100Z".to_string(),
                kind: ActivityKind::Tool,
                text: String::new(),
                tool: Some(ToolActivity {
                    tool_call_id: "tc-1".to_string(),
                    title: "read_file".to_string(),
                    status: Some("in_progress".to_string()),
                    kind: Some("read".to_string()),
                    input: Some(json!({ "path": "src/lib.rs" })),
                    output: None,
                    truncated: false,
                }),
            },
            StepActivity {
                run_id: "run-1".to_string(),
                node_id: "review".to_string(),
                attempt_id: "a2".to_string(),
                seq: 2,
                at: "2026-11-20T01:00:03.500Z".to_string(),
                kind: ActivityKind::Tool,
                text: String::new(),
                tool: Some(ToolActivity {
                    tool_call_id: "tc-1".to_string(),
                    title: "read_file".to_string(),
                    status: Some("completed".to_string()),
                    kind: Some("read".to_string()),
                    input: Some(json!({ "path": "src/lib.rs" })),
                    output: Some(json!("fn main() {}")),
                    truncated: false,
                }),
            },
        ]);

        assert_eq!(evs.len(), 2);
        assert!(matches!(
            evs.first(),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call")
                && update.get("toolCallId").and_then(Value::as_str) == Some("tc-1")
                && update.get("status").and_then(Value::as_str) == Some("in_progress")
                && update.pointer("/rawInput/path").and_then(Value::as_str) == Some("src/lib.rs")
        ));
        assert!(matches!(
            evs.get(1),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call_update")
                && update.get("toolCallId").and_then(Value::as_str) == Some("tc-1")
                && update.get("status").and_then(Value::as_str) == Some("completed")
                && update.get("rawOutput") == Some(&json!("fn main() {}"))
                && update.pointer("/content/0/content/text").and_then(Value::as_str) == Some("fn main() {}")
        ));
    }

    #[test]
    fn finished_step_without_activity_falls_back_to_inline_and_artifact_output() {
        // Inline fallback
        let mut t_inline = PiwTailer::new("run-1");
        let c2 = step_compact(
            "a2",
            "review",
            "agent",
            "ok",
            "2026-11-20T01:00:03.000Z",
            "2026-11-20T01:00:06.000Z",
        );
        let evs = t_inline.on_view(
            &make_view(
                1,
                base_run_view(
                    "completed",
                    json!([c2]),
                    json!([step_full_agent()]),
                    None,
                    None,
                    None,
                ),
            ),
            &inline_text_of,
        );
        assert!(evs.iter().any(|e| matches!(
            e,
            DriverEvent::AgentUpdate { node, attempt, update, .. }
                if node == "review"
                && attempt == "a2"
                && update.get("sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk")
                && update.pointer("/content/text").and_then(Value::as_str) == Some("Looks good to ship.")
        )));
        assert!(t_inline.finished());

        // Artifact output that resolves one view later
        let mut t_art = PiwTailer::new("run-1");
        let art_ref = json!({
            "$artifact": {
                "path": "artifacts/sha256/aaaa.txt",
                "mediaType": "text/plain",
                "bytes": 18,
                "sha256": "aaaa"
            }
        });
        let step_art = json!({
            "attemptId": "a2",
            "nodeId": "review",
            "nodeType": "agent",
            "outcome": "ok",
            "startedAt": "2026-11-20T01:00:03.000Z",
            "finishedAt": "2026-11-20T01:00:06.000Z",
            "prompt": "Review this.",
            "output": art_ref
        });
        let art_view = make_view(
            1,
            base_run_view("completed", json!([c2]), json!([step_art]), None, None, None),
        );

        let pending_text_of = |v: &Value| -> Option<String> {
            if is_artifact_ref(v) {
                None
            } else {
                inline_text_of(v)
            }
        };
        let evs1 = t_art.on_view(&art_view, &pending_text_of);
        // RunStarted + StepStarted + AgentPrompt emitted, but StepFinished & RunFinished held!
        assert_eq!(evs1.len(), 3);
        assert!(matches!(evs1.first(), Some(DriverEvent::RunStarted { .. })));
        assert!(matches!(evs1.get(1), Some(DriverEvent::StepStarted { .. })));
        assert!(matches!(evs1.get(2), Some(DriverEvent::AgentPrompt { .. })));
        assert!(!t_art.finished());

        let ready_text_of = |v: &Value| -> Option<String> {
            if is_artifact_ref(v) {
                Some("Long review text!!".to_string())
            } else {
                inline_text_of(v)
            }
        };
        let evs2 = t_art.on_view(&art_view, &ready_text_of);
        assert_eq!(evs2.len(), 3);
        assert!(matches!(
            evs2.first(),
            Some(DriverEvent::AgentUpdate { update, .. })
                if update.pointer("/content/text").and_then(Value::as_str) == Some("Long review text!!")
        ));
        assert!(matches!(evs2.get(1), Some(DriverEvent::StepFinished { .. })));
        assert!(matches!(evs2.get(2), Some(DriverEvent::RunFinished { status, .. }) if status == "completed"));
        assert!(t_art.finished());

        // Artifact output that never resolves gives up after 3 further on_view calls
        let mut t_timeout = PiwTailer::new("run-1");
        let e0 = t_timeout.on_view(&art_view, &pending_text_of);
        assert_eq!(e0.len(), 3);
        assert!(t_timeout.on_view(&art_view, &pending_text_of).is_empty());
        assert!(t_timeout.on_view(&art_view, &pending_text_of).is_empty());
        assert!(!t_timeout.finished());
        let e3 = t_timeout.on_view(&art_view, &pending_text_of);
        assert_eq!(e3.len(), 2);
        assert!(matches!(e3.first(), Some(DriverEvent::StepFinished { .. })));
        assert!(matches!(e3.get(1), Some(DriverEvent::RunFinished { status, .. }) if status == "completed"));
        assert!(t_timeout.finished());
    }

    #[test]
    fn no_agent_prompt_after_updates() {
        let mut t = PiwTailer::new("run-1");
        t.on_view(
            &make_view(
                1,
                base_run_view(
                    "running",
                    json!([]),
                    json!([]),
                    Some("review"),
                    Some("a2"),
                    Some("2026-11-20T01:00:03.000Z"),
                ),
            ),
            &inline_text_of,
        );
        let u = t.on_activity(&[StepActivity {
            run_id: "run-1".to_string(),
            node_id: "review".to_string(),
            attempt_id: "a2".to_string(),
            seq: 1,
            at: "2026-11-20T01:00:04.000Z".to_string(),
            kind: ActivityKind::Text,
            text: "Working on it".to_string(),
            tool: None,
        }]);
        assert_eq!(u.len(), 1);

        let c2 = step_compact(
            "a2",
            "review",
            "agent",
            "ok",
            "2026-11-20T01:00:03.000Z",
            "2026-11-20T01:00:06.000Z",
        );
        let fin = t.on_view(
            &make_view(
                2,
                base_run_view(
                    "completed",
                    json!([c2]),
                    json!([step_full_agent()]),
                    None,
                    None,
                    None,
                ),
            ),
            &inline_text_of,
        );
        assert!(!fin.iter().any(|e| matches!(e, DriverEvent::AgentPrompt { .. })));
        assert!(!fin.iter().any(|e| matches!(e, DriverEvent::AgentUpdate { .. })));
        assert!(matches!(fin.first(), Some(DriverEvent::StepFinished { .. })));
        assert!(matches!(fin.get(1), Some(DriverEvent::RunFinished { .. })));
    }

    #[test]
    fn decision_gate_emitted_once_with_answer_targets_then_run_continues() {
        let mut t = PiwTailer::new("run-1");
        t.on_view(
            &make_view(
                1,
                base_run_view("waiting", json!([]), json!([]), None, None, None),
            ),
            &inline_text_of,
        );

        let gate = PiwGate {
            run_id: "run-1".to_string(),
            node_id: "approve_deploy".to_string(),
            attempt_id: "gate-1".to_string(),
            session_id: "sess-1".to_string(),
            kind: GateKind::Decision,
            request_id: "req-1".to_string(),
            revision: 3,
            question: "Deploy to production?".to_string(),
            choices: vec![
                PiwGateChoice {
                    key: "approve".to_string(),
                    label: "Approve".to_string(),
                    text_input: None,
                },
                PiwGateChoice {
                    key: "reject".to_string(),
                    label: "Reject".to_string(),
                    text_input: Some("reason".to_string()),
                },
            ],
        };

        let g1 = t.on_gate(Some(&gate));
        assert_eq!(g1.len(), 1);
        assert!(matches!(
            g1.first(),
            Some(DriverEvent::Waiting { node, attempt, question, choices, .. })
                if node == "approve_deploy"
                && attempt == "gate-1"
                && question == "Deploy to production?"
                && choices == &vec![
                    GateChoice {
                        label: "Approve".to_string(),
                        flow_file: answer_target("run-1", "approve"),
                    },
                    GateChoice {
                        label: "Reject".to_string(),
                        flow_file: answer_target("run-1", "reject"),
                    },
                ]
        ));

        // Repeat emits nothing.
        assert!(t.on_gate(Some(&gate)).is_empty());
        // Gate clears after answer.
        assert!(t.on_gate(None).is_empty());

        // Run continues and finishes.
        let c3 = step_compact(
            "a3",
            "summarize",
            "compute",
            "ok",
            "2026-11-20T01:00:07.000Z",
            "2026-11-20T01:00:08.000Z",
        );
        let after = t.on_view(
            &make_view(
                2,
                base_run_view(
                    "completed",
                    json!([c3]),
                    json!([step_full_compute()]),
                    None,
                    None,
                    None,
                ),
            ),
            &inline_text_of,
        );
        assert!(after.iter().any(|e| matches!(e, DriverEvent::StepFinished { node, .. } if node == "summarize")));
        assert!(after.iter().any(|e| matches!(e, DriverEvent::RunFinished { status, .. } if status == "completed")));
        assert!(t.finished());
    }

    #[test]
    fn checkpoint_gate_emits_waiting_with_empty_choices() {
        let mut t = PiwTailer::new("run-1");
        let gate = PiwGate {
            run_id: "run-1".to_string(),
            node_id: "manual_input".to_string(),
            attempt_id: "cp-1".to_string(),
            session_id: "sess-1".to_string(),
            kind: GateKind::Checkpoint,
            request_id: "req-cp-1".to_string(),
            revision: 1,
            question: "Provide release notes".to_string(),
            choices: Vec::new(),
        };
        let evs = t.on_gate(Some(&gate));
        assert_eq!(evs.len(), 1);
        assert!(matches!(
            evs.first(),
            Some(DriverEvent::Waiting { node, attempt, question, choices, .. })
                if node == "manual_input"
                && attempt == "cp-1"
                && question == "Provide release notes"
                && choices.is_empty()
        ));
        assert!(t.on_gate(Some(&gate)).is_empty());
    }

    #[test]
    fn failed_cancelled_and_unknown_terminal_runs() {
        // Failed
        let mut t_fail = PiwTailer::new("run-1");
        let mut fail_val = base_run_view("failed", json!([]), json!([]), None, None, None);
        fail_val["state"]["error"] = json!("tests failed");
        let evs = t_fail.on_view(&make_view(1, fail_val), &inline_text_of);
        assert!(matches!(
            evs.last(),
            Some(DriverEvent::RunFinished { status, error: Some(err), .. })
                if status == "failed" && err == "tests failed"
        ));
        assert!(t_fail.finished());

        // Cancelled
        let mut t_cancel = PiwTailer::new("run-1");
        let cancel_val = base_run_view("cancelled", json!([]), json!([]), None, None, None);
        let evs = t_cancel.on_view(&make_view(1, cancel_val), &inline_text_of);
        assert!(matches!(
            evs.last(),
            Some(DriverEvent::RunFinished { status, .. }) if status == "cancelled"
        ));
        assert!(t_cancel.finished());

        // Unknown / timed_out -> failed with server's word in error
        let mut t_unknown = PiwTailer::new("run-1");
        let timed_val = base_run_view("timed_out", json!([]), json!([]), None, None, None);
        let evs = t_unknown.on_view(&make_view(1, timed_val), &inline_text_of);
        assert!(matches!(
            evs.last(),
            Some(DriverEvent::RunFinished { status, error: Some(err), .. })
                if status == "failed" && err.contains("timed_out")
        ));
        assert!(t_unknown.finished());
    }

    #[test]
    fn malformed_or_missing_view_fields_produce_no_panic() {
        for bad in [
            Value::Null,
            json!(42),
            json!({}),
            json!({
                "runId": 123,
                "display": "bad",
                "manifest": [],
                "state": {
                    "status": 999,
                    "steps": "not-an-array",
                    "currentNode": 12,
                    "results": "nope"
                },
                "workflow": {
                    "name": false,
                    "startAt": [],
                    "nodes": { "n1": "not-an-object" },
                    "edges": [123, { "from": "n1", "switch": "bad" }]
                },
                "graphSteps": [null, 5, { "attemptId": "", "nodeId": 7 }]
            }),
        ] {
            let mut t = PiwTailer::new("run-bad");
            let evs = t.on_view(&make_view(1, bad), &inline_text_of);
            assert!(matches!(evs.first(), Some(DriverEvent::RunStarted { .. })));
        }
    }

    #[test]
    fn headless_run_session_id_comes_from_follow_up_queue() {
        // A headless run (the CLI `start` command) has no queue.originSessionId
        // and no session binding; its session id sits in followUpQueue only.
        let headless = json!({
            "queue": { "originSessionId": null },
            "session": { "binding": null },
            "followUpQueue": { "originSessionId": "cli-session-1" }
        });
        assert_eq!(extract_origin_session_id(&headless), Some("cli-session-1".to_string()));

        // A session-mode run still prefers queue.originSessionId.
        let session_run = json!({
            "queue": { "originSessionId": "sess-1" },
            "session": { "binding": { "piSessionId": "sess-2" } },
            "followUpQueue": { "originSessionId": "cli-session-1" }
        });
        assert_eq!(extract_origin_session_id(&session_run), Some("sess-1".to_string()));

        // Nothing recorded at all: no session to watch.
        assert_eq!(extract_origin_session_id(&json!({ "queue": {} })), None);
    }

    #[test]
    fn flow_sketch_branching_node_and_edge_order() {
        let wf = json!({
            "schema": "pi-workflows.definition-snapshot.v1",
            "name": "triage",
            "startAt": "start",
            "nodes": {
                "z_dead": { "nodeType": "compute" },
                "a_dead": { "nodeType": "notify" },
                "end": { "nodeType": "checkpoint", "summary": "Confirm?" },
                "branch_b": { "nodeType": "action", "actionExecution": "function" },
                "branch_a": { "nodeType": "agent", "agent": "grok-build", "model": "grok-4" },
                "start": { "nodeType": "action", "actionExecution": "shell" }
            },
            "edges": [
                {
                    "from": "start",
                    "switch": {
                        "on": "$.ok",
                        "cases": {
                            "yes": "branch_a",
                            "no": "branch_b"
                        }
                    }
                },
                { "from": "branch_a", "to": "end" },
                { "from": "branch_b", "to": "end" }
            ]
        });

        let (sketch, _) = build_flow_sketch(Some(&wf), "fallback");
        let ids: Vec<&str> = sketch.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids.first().copied(), Some("start"));
        assert!(ids.get(1..3).is_some_and(|s| s.contains(&"branch_a") && s.contains(&"branch_b")));
        assert_eq!(ids.get(3).copied(), Some("end"));
        assert_eq!(ids.get(4..), Some(&["a_dead", "z_dead"][..]));

        let find_node = |id: &str| sketch.nodes.iter().find(|n| n.id == id);
        assert_eq!(find_node("start").map(|n| n.kind), Some(StepKind::Shell));
        assert_eq!(find_node("branch_a").map(|n| n.kind), Some(StepKind::Acp));
        assert_eq!(
            find_node("branch_a").and_then(|n| n.profile.as_deref()),
            Some("grok-build")
        );
        assert_eq!(
            find_node("branch_a").and_then(|n| n.model.as_deref()),
            Some("grok-4")
        );
        assert_eq!(
            find_node("branch_b").map(|n| n.kind),
            Some(StepKind::Compute)
        );
        assert_eq!(find_node("end").map(|n| n.kind), Some(StepKind::Checkpoint));
        assert_eq!(
            find_node("end").and_then(|n| n.summary.as_deref()),
            Some("Confirm?")
        );
        assert_eq!(find_node("a_dead").map(|n| n.kind), Some(StepKind::Other));

        assert_eq!(sketch.edges.len(), 4);
        assert!(sketch.edges.contains(&SketchEdge {
            from: "start".to_string(),
            to: "branch_a".to_string(),
            label: Some("yes".to_string()),
        }));
        assert!(sketch.edges.contains(&SketchEdge {
            from: "start".to_string(),
            to: "branch_b".to_string(),
            label: Some("no".to_string()),
        }));
    }

}
