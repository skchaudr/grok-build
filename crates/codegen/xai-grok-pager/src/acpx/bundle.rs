use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use chrono::DateTime;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus {
    Running,
    Waiting,
    Completed,
    Failed,
    TimedOut,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Acp,
    Action,
    Compute,
    Checkpoint,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    TimedOut,
    Failed,
    Cancelled,
    Unknown,
}

/// What a node looks like right now, derived from steps + live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    NotRun,
    Running,
    Waiting,
    Done(Outcome),
}

impl RunStatus {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Unknown => "unknown",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Waiting => "◐",
            Self::Completed => "✓",
            Self::Failed => "✗",
            Self::TimedOut => "✗",
            Self::Unknown => "○",
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Running)
    }
}

impl NodeKind {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "acp" => Self::Acp,
            "action" => Self::Action,
            "compute" => Self::Compute,
            "checkpoint" => Self::Checkpoint,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Action => "action",
            Self::Compute => "compute",
            Self::Checkpoint => "checkpoint",
            Self::Unknown => "?",
        }
    }
}

impl Outcome {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "ok" => Self::Ok,
            "timed_out" => Self::TimedOut,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }
}

impl NodeState {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::NotRun => "○",
            Self::Running => "●",
            Self::Waiting => "◐",
            Self::Done(Outcome::Ok) => "✓",
            Self::Done(Outcome::Failed) => "✗",
            Self::Done(Outcome::TimedOut) => "✗",
            Self::Done(Outcome::Cancelled) => "⊘",
            Self::Done(Outcome::Unknown) => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactRef {
    pub path: String,
    pub media_type: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub session_id: String,
    pub event_start_seq: u64,
    pub event_end_seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionReceipt {
    pub action_type: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}

/// One attempt of one node, from projections/steps.json, in execution order.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub attempt_id: String,
    pub node_id: String,
    pub kind: NodeKind,
    pub outcome: Outcome,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub prompt_text: Option<String>,
    pub raw_text: Option<String>,
    pub output: Value,
    pub error: Option<String>,
    pub agent_name: Option<String>,
    pub action: Option<ActionReceipt>,
    pub conversation: Option<Conversation>,
    pub stdout: Option<ArtifactRef>,
    pub stderr: Option<ArtifactRef>,
    pub output_artifact: Option<ArtifactRef>,
}

impl Step {
    pub fn duration_ms(&self) -> u64 {
        let diff = self.finished_ms.saturating_sub(self.started_ms);
        if diff <= 0 { 0 } else { diff as u64 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    pub summary: Option<String>,
    pub is_shell: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlowEdge {
    Simple {
        from: String,
        to: String,
    },
    Switch {
        from: String,
        on: String,
        cases: Vec<(String, String)>,
    },
}

/// flow.json. `nodes` is ordered breadth-first from `start_at` following edges (Simple first, then Switch cases in
/// order); nodes not reachable are appended sorted by id.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowDef {
    pub name: String,
    pub start_at: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

/// One line of trace.ndjson.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    pub seq: u64,
    pub at_ms: i64,
    pub scope: String,
    pub kind: String,
    pub node_id: Option<String>,
    pub attempt_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRef {
    pub id: String,
    pub handle: String,
    pub events_path: PathBuf,
    pub agent_name: Option<String>,
}

/// One row of the runs ledger; cheap to load (projections/live.json, falling back to manifest.json).
#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub dir: PathBuf,
    pub flow_name: String,
    pub title: String,
    pub status: RunStatus,
    pub started_ms: i64,
    pub updated_ms: i64,
    pub finished_ms: Option<i64>,
    pub current_node: Option<String>,
    pub waiting_on: Option<String>,
    pub status_detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub summary: RunSummary,
    pub flow: FlowDef,
    pub steps: Vec<Step>,
    pub events: Vec<TraceEvent>,
    pub input: Value,
    pub sessions: Vec<SessionRef>,
}

impl Run {
    pub fn node_state(&self, node_id: &str) -> NodeState {
        if self.summary.waiting_on.as_deref() == Some(node_id) {
            return NodeState::Waiting;
        }
        if self.summary.status == RunStatus::Running
            && self.summary.current_node.as_deref() == Some(node_id)
        {
            return NodeState::Running;
        }
        if let Some(step) = self.steps.iter().rev().find(|s| s.node_id == node_id) {
            return NodeState::Done(step.outcome);
        }
        NodeState::NotRun
    }

    pub fn taken_path(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.node_id.clone()).collect()
    }

    pub fn edge_taken(&self, from: &str, to: &str) -> bool {
        self.steps
            .windows(2)
            .any(|pair| pair[0].node_id == from && pair[1].node_id == to)
    }

    pub fn step_events(&self, attempt_id: &str) -> Vec<&TraceEvent> {
        self.events
            .iter()
            .filter(|e| e.attempt_id.as_deref() == Some(attempt_id))
            .collect()
    }

    pub fn progress(&self) -> (usize, usize) {
        let distinct: HashSet<&str> = self.steps.iter().map(|s| s.node_id.as_str()).collect();
        (distinct.len(), self.flow.nodes.len())
    }

    pub fn duration_ms(&self, now_ms: i64) -> u64 {
        let end_ms = if let Some(fin) = self.summary.finished_ms {
            fin
        } else if self.summary.status == RunStatus::Running {
            now_ms.max(self.summary.updated_ms)
        } else {
            self.summary.updated_ms
        };
        let diff = end_ms.saturating_sub(self.summary.started_ms);
        if diff <= 0 { 0 } else { diff as u64 }
    }

    pub fn session_events_path(&self, session_id: &str) -> Option<&Path> {
        self.sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.events_path.as_path())
    }
}

pub fn runs_base() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".acpx/flows/runs"))
}

pub fn list_runs(base: &Path) -> Vec<RunSummary> {
    let entries = match fs::read_dir(base) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let live_path = path.join("projections/live.json");
        let manifest_path = path.join("manifest.json");
        if !live_path.is_file() && !manifest_path.is_file() {
            continue;
        }
        if let Some(summary) = read_summary_from_dir(&path) {
            runs.push(summary);
        }
    }

    runs.sort_by(|a, b| {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    runs
}

pub fn load_run(dir: &Path) -> io::Result<Run> {
    let meta = fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "run path is not a directory",
        ));
    }
    // Check directory is readable
    let _ = fs::read_dir(dir)?;

    let live_val = read_json_file(&dir.join("projections/live.json"));
    let manifest_val = read_json_file(&dir.join("manifest.json"));
    let run_proj_val = read_json_file(&dir.join("projections/run.json"));

    let summary = build_summary(dir, live_val.as_ref(), manifest_val.as_ref());

    let flow = read_json_file(&dir.join("flow.json"))
        .map(|v| parse_flow_def(&v))
        .unwrap_or_else(empty_flow_def);

    let steps = read_json_file(&dir.join("projections/steps.json"))
        .and_then(|v| parse_steps(&v))
        .unwrap_or_default();

    let events = read_trace_events(&dir.join("trace.ndjson")).unwrap_or_default();

    let input = run_proj_val
        .as_ref()
        .and_then(|v| v.get("input"))
        .cloned()
        .unwrap_or(Value::Null);

    let sessions = parse_sessions(
        dir,
        manifest_val.as_ref(),
        live_val.as_ref(),
        run_proj_val.as_ref(),
    );

    Ok(Run {
        summary,
        flow,
        steps,
        events,
        input,
        sessions,
    })
}

/// Re-read a run in place. Each file that fails to parse (acpx rewrites them non-atomically) keeps its previous value.
/// Returns true when anything changed.
pub fn refresh(run: &mut Run) -> bool {
    let dir = run.summary.dir.clone();
    let mut changed = false;

    let live_val = read_json_file(&dir.join("projections/live.json"));
    let manifest_val = read_json_file(&dir.join("manifest.json"));
    let run_proj_val = read_json_file(&dir.join("projections/run.json"));

    if live_val.is_some() || manifest_val.is_some() {
        let new_summary = build_summary(&dir, live_val.as_ref(), manifest_val.as_ref());
        if new_summary != run.summary {
            run.summary = new_summary;
            changed = true;
        }
    }

    if let Some(flow_val) = read_json_file(&dir.join("flow.json")) {
        let new_flow = parse_flow_def(&flow_val);
        if new_flow != run.flow {
            run.flow = new_flow;
            changed = true;
        }
    }

    if let Some(steps_val) = read_json_file(&dir.join("projections/steps.json")) {
        if let Some(new_steps) = parse_steps(&steps_val) {
            if new_steps != run.steps {
                run.steps = new_steps;
                changed = true;
            }
        }
    }

    if let Some(new_events) = read_trace_events(&dir.join("trace.ndjson")) {
        if new_events != run.events {
            run.events = new_events;
            changed = true;
        }
    }

    if let Some(ref rp) = run_proj_val {
        let new_input = rp.get("input").cloned().unwrap_or(Value::Null);
        if new_input != run.input {
            run.input = new_input;
            changed = true;
        }
    }

    if manifest_val.is_some() {
        let new_sessions = parse_sessions(
            &dir,
            manifest_val.as_ref(),
            live_val.as_ref(),
            run_proj_val.as_ref(),
        );
        if new_sessions != run.sessions {
            run.sessions = new_sessions;
            changed = true;
        }
    } else if live_val.is_some() || run_proj_val.is_some() {
        // Update agent_name on existing sessions if bindings became available
        let agent_map = extract_agent_map(live_val.as_ref(), run_proj_val.as_ref());
        if !agent_map.is_empty() {
            for s in &mut run.sessions {
                if let Some(agent) = agent_map.get(&s.id) {
                    if s.agent_name.as_ref() != Some(agent) {
                        s.agent_name = Some(agent.clone());
                        changed = true;
                    }
                }
            }
        }
    }

    changed
}

/// Read an artifact as text, at most `max_bytes` (cut on a char boundary). Rejects absolute paths and any `..` component.
pub fn read_artifact(
    run_dir: &Path,
    artifact: &ArtifactRef,
    max_bytes: usize,
) -> io::Result<String> {
    let rel = Path::new(&artifact.path);
    if rel.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact path must be relative",
        ));
    }
    for comp in rel.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "artifact path contains disallowed component",
                ));
            }
        }
    }

    let full_path = run_dir.join(rel);
    let file = fs::File::open(full_path)?;
    let mut buf = Vec::new();
    file.take(max_bytes as u64).read_to_end(&mut buf)?;

    match String::from_utf8(buf) {
        Ok(s) => Ok(s),
        Err(err) => {
            let valid_up_to = err.utf8_error().valid_up_to();
            let mut bytes = err.into_bytes();
            bytes.truncate(valid_up_to);
            Ok(String::from_utf8(bytes).unwrap_or_default())
        }
    }
}

pub fn parse_ts(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

pub fn fmt_duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        let tenths = (ms + 50) / 100;
        let secs = tenths / 10;
        let rem = tenths % 10;
        if secs >= 60 {
            let mins = secs / 60;
            let rem_secs = secs % 60;
            format!("{mins}m{rem_secs:02}s")
        } else {
            format!("{secs}.{rem}s")
        }
    } else if ms < 3_600_000 {
        let total_secs = ms / 1_000;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{mins}m{secs:02}s")
    } else {
        let total_mins = ms / 60_000;
        let hours = total_mins / 60;
        let mins = total_mins % 60;
        format!("{hours}h{mins:02}m")
    }
}

pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

// ==== Private Helpers ====

fn read_json_file(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn string_field(v: &Value, key: &str) -> Option<String> {
    str_field(v, key).map(ToOwned::to_owned)
}

fn u64_field(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn i64_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

fn bool_field(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

fn error_field(v: &Value, key: &str) -> Option<String> {
    let err = v.get(key)?;
    match err {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => {
            if let Some(msg) = map.get("message").and_then(Value::as_str) {
                Some(msg.to_owned())
            } else {
                serde_json::to_string(err).ok()
            }
        }
        other => serde_json::to_string(other).ok(),
    }
}

fn read_summary_from_dir(dir: &Path) -> Option<RunSummary> {
    let live_val = read_json_file(&dir.join("projections/live.json"));
    let manifest_val = read_json_file(&dir.join("manifest.json"));
    if live_val.is_none() && manifest_val.is_none() {
        return None;
    }
    Some(build_summary(dir, live_val.as_ref(), manifest_val.as_ref()))
}

fn build_summary(dir: &Path, live: Option<&Value>, manifest: Option<&Value>) -> RunSummary {
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();

    let pick_str = |key: &str| -> Option<String> {
        live.and_then(|v| string_field(v, key))
            .or_else(|| manifest.and_then(|v| string_field(v, key)))
    };

    let run_id = pick_str("runId").unwrap_or_else(|| dir_name.clone());
    let flow_name = pick_str("flowName").unwrap_or_default();
    let run_title = pick_str("runTitle").filter(|s| !s.is_empty());

    let title = run_title.unwrap_or_else(|| {
        if !flow_name.is_empty() {
            flow_name.clone()
        } else {
            run_id.clone()
        }
    });

    let status = pick_str("status")
        .map(|s| RunStatus::from_str_tolerant(&s))
        .unwrap_or(RunStatus::Unknown);

    let started_ms = pick_str("startedAt")
        .and_then(|s| parse_ts(&s))
        .unwrap_or(0);

    let updated_ms = live
        .and_then(|v| str_field(v, "updatedAt"))
        .and_then(parse_ts)
        .unwrap_or(started_ms);

    let finished_ms = live
        .and_then(|v| str_field(v, "finishedAt"))
        .and_then(parse_ts);

    let current_node = live.and_then(|v| string_field(v, "currentNode"));
    let waiting_on = live.and_then(|v| string_field(v, "waitingOn"));
    let status_detail = live.and_then(|v| string_field(v, "statusDetail"));
    let error = live.and_then(|v| error_field(v, "error"));

    RunSummary {
        run_id,
        dir: dir.to_path_buf(),
        flow_name,
        title,
        status,
        started_ms,
        updated_ms,
        finished_ms,
        current_node,
        waiting_on,
        status_detail,
        error,
    }
}

fn empty_flow_def() -> FlowDef {
    FlowDef {
        name: String::new(),
        start_at: String::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
    }
}

fn parse_flow_def(v: &Value) -> FlowDef {
    let name = string_field(v, "name").unwrap_or_default();
    let start_at = string_field(v, "startAt").unwrap_or_default();

    let mut node_map: HashMap<String, FlowNode> = HashMap::new();
    if let Some(nodes_obj) = v.get("nodes").and_then(Value::as_object) {
        for (id, nv) in nodes_obj {
            let kind = str_field(nv, "nodeType")
                .map(NodeKind::from_str_tolerant)
                .unwrap_or(NodeKind::Unknown);
            let summary = if kind == NodeKind::Checkpoint {
                string_field(nv, "summary")
            } else {
                None
            };
            let is_shell = str_field(nv, "actionExecution") == Some("shell");
            node_map.insert(
                id.clone(),
                FlowNode {
                    id: id.clone(),
                    kind,
                    summary,
                    is_shell,
                },
            );
        }
    }

    let mut edges = Vec::new();
    if let Some(edges_arr) = v.get("edges").and_then(Value::as_array) {
        for ev in edges_arr {
            let Some(from) = string_field(ev, "from") else {
                continue;
            };
            if let Some(sw) = ev.get("switch").and_then(Value::as_object) {
                let on = sw
                    .get("on")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let mut cases = Vec::new();
                if let Some(cases_obj) = sw.get("cases").and_then(Value::as_object) {
                    for (case_val, target_val) in cases_obj {
                        if let Some(target) = target_val.as_str() {
                            cases.push((case_val.clone(), target.to_owned()));
                        }
                    }
                }
                cases.sort_by(|a, b| a.0.cmp(&b.0));
                edges.push(FlowEdge::Switch { from, on, cases });
            } else if let Some(to) = string_field(ev, "to") {
                edges.push(FlowEdge::Simple { from, to });
            }
        }
    }

    // Order nodes breadth-first from `start_at` following edges (Simple first, then Switch cases in order);
    // nodes not reachable are appended sorted by id.
    let mut ordered_nodes = Vec::with_capacity(node_map.len());
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    if !start_at.is_empty() && node_map.contains_key(&start_at) {
        visited.insert(start_at.clone());
        queue.push_back(start_at.clone());
    }

    while let Some(curr) = queue.pop_front() {
        if let Some(node) = node_map.remove(&curr) {
            ordered_nodes.push(node);
        }
        // First follow Simple edges from curr
        for edge in &edges {
            if let FlowEdge::Simple { from, to } = edge {
                if from == &curr && node_map.contains_key(to) && !visited.contains(to) {
                    visited.insert(to.clone());
                    queue.push_back(to.clone());
                }
            }
        }
        // Then follow Switch cases in order from curr
        for edge in &edges {
            if let FlowEdge::Switch { from, cases, .. } = edge {
                if from == &curr {
                    for (_, target) in cases {
                        if node_map.contains_key(target) && !visited.contains(target) {
                            visited.insert(target.clone());
                            queue.push_back(target.clone());
                        }
                    }
                }
            }
        }
    }

    let mut remaining: Vec<FlowNode> = node_map.into_values().collect();
    remaining.sort_by(|a, b| a.id.cmp(&b.id));
    ordered_nodes.extend(remaining);

    FlowDef {
        name,
        start_at,
        nodes: ordered_nodes,
        edges,
    }
}

fn parse_artifact_ref(v: &Value) -> Option<ArtifactRef> {
    let obj = v.as_object()?;
    let path = obj.get("path")?.as_str()?.to_owned();
    let media_type = obj
        .get("mediaType")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let bytes = obj.get("bytes").and_then(Value::as_u64).unwrap_or(0);
    Some(ArtifactRef {
        path,
        media_type,
        bytes,
    })
}

fn parse_action_receipt(v: &Value) -> Option<ActionReceipt> {
    let obj = v.as_object()?;
    let action_type = obj
        .get("actionType")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let command = obj
        .get("command")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let args = obj
        .get("args")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let cwd = obj
        .get("cwd")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let exit_code = obj.get("exitCode").and_then(Value::as_i64);
    let duration_ms = obj.get("durationMs").and_then(Value::as_u64);
    Some(ActionReceipt {
        action_type,
        command,
        args,
        cwd,
        exit_code,
        duration_ms,
    })
}

fn parse_conversation(v: &Value) -> Option<Conversation> {
    let obj = v.as_object()?;
    let session_id = obj.get("sessionId")?.as_str()?.to_owned();
    let event_start_seq = obj.get("eventStartSeq").and_then(Value::as_u64).unwrap_or(0);
    let event_end_seq = obj.get("eventEndSeq").and_then(Value::as_u64).unwrap_or(0);
    Some(Conversation {
        session_id,
        event_start_seq,
        event_end_seq,
    })
}

fn parse_steps(v: &Value) -> Option<Vec<Step>> {
    let arr = v.as_array()?;
    let mut steps = Vec::with_capacity(arr.len());
    for sv in arr {
        let attempt_id = string_field(sv, "attemptId").unwrap_or_default();
        let node_id = string_field(sv, "nodeId").unwrap_or_default();
        let kind = str_field(sv, "nodeType")
            .map(NodeKind::from_str_tolerant)
            .unwrap_or(NodeKind::Unknown);
        let outcome = str_field(sv, "outcome")
            .map(Outcome::from_str_tolerant)
            .unwrap_or(Outcome::Unknown);
        let started_ms = str_field(sv, "startedAt").and_then(parse_ts).unwrap_or(0);
        let finished_ms = str_field(sv, "finishedAt").and_then(parse_ts).unwrap_or(0);
        let prompt_text = string_field(sv, "promptText");
        let raw_text = string_field(sv, "rawText");
        let output = sv.get("output").cloned().unwrap_or(Value::Null);
        let error = error_field(sv, "error");
        let agent_name = sv
            .get("agent")
            .and_then(|a| string_field(a, "agentName"));

        let trace = sv.get("trace");
        let action = trace
            .and_then(|t| t.get("action"))
            .and_then(parse_action_receipt);
        let conversation = trace
            .and_then(|t| t.get("conversation"))
            .and_then(parse_conversation);
        let stdout = trace
            .and_then(|t| t.get("stdoutArtifact"))
            .and_then(parse_artifact_ref);
        let stderr = trace
            .and_then(|t| t.get("stderrArtifact"))
            .and_then(parse_artifact_ref);
        let output_artifact = trace
            .and_then(|t| t.get("outputArtifact"))
            .and_then(parse_artifact_ref);

        steps.push(Step {
            attempt_id,
            node_id,
            kind,
            outcome,
            started_ms,
            finished_ms,
            prompt_text,
            raw_text,
            output,
            error,
            agent_name,
            action,
            conversation,
            stdout,
            stderr,
            output_artifact,
        });
    }
    Some(steps)
}

fn read_trace_events(path: &Path) -> Option<Vec<TraceEvent>> {
    let content = fs::read_to_string(path).ok()?;
    let mut events = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(ev) = parse_trace_event(&v) {
            events.push(ev);
        }
    }
    events.sort_by_key(|e| e.seq);
    Some(events)
}

fn parse_trace_event(v: &Value) -> Option<TraceEvent> {
    let obj = v.as_object()?;
    let seq = obj.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let at_ms = obj
        .get("at")
        .and_then(Value::as_str)
        .and_then(parse_ts)
        .unwrap_or(0);
    let scope = obj
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let kind = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let node_id = obj
        .get("nodeId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let attempt_id = obj
        .get("attemptId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let detail = build_trace_detail(&kind, v);

    Some(TraceEvent {
        seq,
        at_ms,
        scope,
        kind,
        node_id,
        attempt_id,
        detail,
    })
}

fn build_trace_detail(kind: &str, root: &Value) -> String {
    let payload = root.get("payload").unwrap_or(&Value::Null);
    match kind {
        "node_started" => string_field(payload, "nodeType").unwrap_or_default(),
        "node_heartbeat" => string_field(payload, "statusDetail").unwrap_or_default(),
        "action_prepared" => {
            let Some(action) = payload.get("action") else {
                return String::new();
            };
            let mut parts = Vec::new();
            if let Some(cmd) = str_field(action, "command") {
                if !cmd.is_empty() {
                    parts.push(cmd.to_owned());
                }
            }
            if let Some(args) = action.get("args").and_then(Value::as_array) {
                for a in args {
                    if let Some(s) = a.as_str() {
                        parts.push(s.to_owned());
                    }
                }
            }
            parts.join(" ")
        }
        "action_completed" => {
            let Some(action) = payload.get("action") else {
                return String::new();
            };
            let code_str = match i64_field(action, "exitCode") {
                Some(c) => c.to_string(),
                None => str_field(action, "signal").unwrap_or("?").to_owned(),
            };
            let dur = u64_field(action, "durationMs").unwrap_or(0);
            format!("exit {code_str} · {}", fmt_duration(dur))
        }
        "artifact_written" => {
            let art = payload
                .get("artifact")
                .or_else(|| root.get("artifact"))
                .and_then(parse_artifact_ref);
            if let Some(a) = art {
                let file_name = Path::new(&a.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&a.path);
                format!("{file_name} · {} B", a.bytes)
            } else {
                String::new()
            }
        }
        "acp_prompt_prepared" => {
            if let Some(a) = payload
                .get("promptArtifact")
                .and_then(parse_artifact_ref)
            {
                format!("prompt · {} B", a.bytes)
            } else {
                String::new()
            }
        }
        "acp_response_parsed" => {
            if let Some(a) = payload
                .get("rawResponseArtifact")
                .and_then(parse_artifact_ref)
            {
                format!("response · {} B", a.bytes)
            } else {
                String::new()
            }
        }
        "session_bound" => {
            if let Some(handle) = str_field(payload, "handle") {
                format!("session {handle}")
            } else {
                String::new()
            }
        }
        "node_outcome" => {
            let outcome = str_field(payload, "outcome").unwrap_or("?");
            let dur = u64_field(payload, "durationMs").unwrap_or(0);
            let mut s = format!("{outcome} · {}", fmt_duration(dur));
            if let Some(err) = error_field(payload, "error") {
                if !err.is_empty() {
                    s.push_str(" · ");
                    s.push_str(&err);
                }
            }
            s
        }
        "run_started" | "run_completed" | "run_failed" => {
            if let Some(err) = error_field(payload, "error") {
                if !err.is_empty() {
                    return err;
                }
            }
            if let Some(flow) = str_field(payload, "flowName") {
                return flow.to_owned();
            }
            String::new()
        }
        _ => String::new(),
    }
}

fn extract_agent_map(
    live: Option<&Value>,
    run_proj: Option<&Value>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for source in [run_proj, live].into_iter().flatten() {
        if let Some(bindings) = source.get("sessionBindings").and_then(Value::as_object) {
            for (_, bval) in bindings {
                if let (Some(bundle_id), Some(agent_name)) = (
                    str_field(bval, "bundleId"),
                    str_field(bval, "agentName"),
                ) {
                    map.insert(bundle_id.to_owned(), agent_name.to_owned());
                }
            }
        }
    }
    map
}

fn parse_sessions(
    dir: &Path,
    manifest: Option<&Value>,
    live: Option<&Value>,
    run_proj: Option<&Value>,
) -> Vec<SessionRef> {
    let Some(manifest_val) = manifest else {
        return Vec::new();
    };
    let Some(sessions_arr) = manifest_val.get("sessions").and_then(Value::as_array) else {
        return Vec::new();
    };

    let agent_map = extract_agent_map(live, run_proj);
    let mut result = Vec::with_capacity(sessions_arr.len());

    for sv in sessions_arr {
        let Some(id) = string_field(sv, "id") else {
            continue;
        };
        let handle = string_field(sv, "handle").unwrap_or_default();
        let rel_events = string_field(sv, "eventsPath")
            .unwrap_or_else(|| format!("sessions/{id}/events.ndjson"));
        let events_path = dir.join(rel_events);
        let agent_name = agent_map.get(&id).cloned();

        result.push(SessionRef {
            id,
            handle,
            events_path,
            agent_name,
        });
    }

    let _ = bool_field; // silence unused helper if not needed elsewhere
    result
}

/// Who ran (or will run) an agent node. Every field is None for non-agent nodes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentSettings {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub handle: Option<String>,
    pub acp_session_id: Option<String>,
}

impl AgentSettings {
    /// "grok-build · grok-4.7 · low" from the parts that are known; "" when none.
    pub fn label(&self) -> String {
        [&self.agent, &self.model, &self.effort].into_iter().flatten().map(String::as_str).collect::<Vec<_>>().join(" · ")
    }
}

/// A run that says Running but has not been written for this long has lost its acpx process (it heartbeats every 5 s).
const STALLED_AFTER_MS: i64 = 90_000;

fn effort_of(options: Option<&Value>) -> Option<String> {
    options?.as_array()?.iter().find(|o| str_field(o, "configId") == Some("reasoning_effort")).and_then(|o| {
        string_field(o, "acceptedValue").or_else(|| string_field(o, "value"))
    })
}

/// Per node id, what the flow file asked for (flow.json). Empty on any error.
pub fn node_settings(run_dir: &Path) -> HashMap<String, AgentSettings> {
    let Some(flow) = read_json_file(&run_dir.join("flow.json")) else { return HashMap::new() };
    let Some(nodes) = flow.get("nodes").and_then(Value::as_object) else { return HashMap::new() };
    nodes
        .iter()
        .map(|(id, node)| {
            let settings = AgentSettings {
                agent: string_field(node, "profile"),
                model: string_field(node, "modelId"),
                effort: effort_of(node.get("configOptions")),
                handle: node.get("session").and_then(|s| string_field(s, "handle")),
                acp_session_id: None,
            };
            (id.clone(), settings)
        })
        .collect()
}

/// Per attempt id, what the step actually ran with (projections/steps.json). Empty on any error.
pub fn step_settings(run_dir: &Path) -> HashMap<String, AgentSettings> {
    let Some(steps) = read_json_file(&run_dir.join("projections").join("steps.json")) else { return HashMap::new() };
    let Some(steps) = steps.as_array().or_else(|| steps.get("steps").and_then(Value::as_array)) else {
        return HashMap::new();
    };
    steps
        .iter()
        .filter_map(|step| {
            let attempt = string_field(step, "attemptId")?;
            let session = step.get("session").filter(|s| s.is_object());
            let requested = session.and_then(|s| s.get("requestedSettings"));
            let accepted = session.and_then(|s| s.get("acceptedSettings"));
            let settings = AgentSettings {
                agent: session
                    .and_then(|s| string_field(s, "agentName"))
                    .or_else(|| step.get("agent").and_then(|a| string_field(a, "agentName"))),
                model: requested.and_then(|r| string_field(r, "model")),
                effort: effort_of(accepted.and_then(|a| a.get("configOptions")))
                    .or_else(|| effort_of(requested.and_then(|r| r.get("configOptions")))),
                handle: session.and_then(|s| string_field(s, "handle")),
                acp_session_id: session.and_then(|s| string_field(s, "acpSessionId")),
            };
            Some((attempt, settings))
        })
        .collect()
}

/// Running on paper, but nothing has written the run for 90 s: the acpx process is gone.
pub fn is_stalled(summary: &RunSummary, now_ms: i64) -> bool {
    summary.status == RunStatus::Running && now_ms.saturating_sub(summary.updated_ms) > STALLED_AFTER_MS
}

/// The flow file a run was started from (manifest.json, else projections/live.json).
pub fn flow_path(run_dir: &Path) -> Option<PathBuf> {
    [run_dir.join("manifest.json"), run_dir.join("projections").join("live.json")]
        .iter()
        .find_map(|file| read_json_file(file).and_then(|v| string_field(&v, "flowPath")))
        .map(PathBuf::from)
}

/// A waiting run's gate choices: the waiting checkpoint step's output `next` object (`{ "<label>": "<flow file>" }`),
/// as (label, flow file) in key order. Empty when the run is not waiting or the checkpoint declares none.
pub fn gate_choices(run: &Run) -> Vec<(String, String)> {
    if run.summary.status != RunStatus::Waiting {
        return Vec::new();
    }
    let Some(node) = run.summary.waiting_on.as_deref() else { return Vec::new() };
    let Some(step) = run.steps.iter().rev().find(|s| s.node_id == node) else { return Vec::new() };
    let Some(next) = step.output.get("next").and_then(Value::as_object) else { return Vec::new() };
    let mut choices: Vec<(String, String)> =
        next.iter().filter_map(|(label, file)| Some((label.clone(), file.as_str()?.to_string()))).collect();
    choices.sort();
    choices
}

/// What a gate hands the next flow: the first run's input and every node's last output, plus which gate was answered.
pub fn gate_input(run: &Run, choice: &str) -> Value {
    let outputs: serde_json::Map<String, Value> =
        run.steps.iter().map(|step| (step.node_id.clone(), step.output.clone())).collect();
    serde_json::json!({
        "input": run.input,
        "outputs": outputs,
        "gate": { "runId": run.summary.run_id, "node": run.summary.waiting_on, "choice": choice },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const SAMPLE_MANIFEST: &str = r#"{
      "schema": "acpx.flow-run-bundle.v1",
      "runId": "2026-11-18T191214880Z-feel-4832e848",
      "flowName": "feel",
      "runTitle": "feel: /tmp/acpx-feel",
      "flowPath": "/private/tmp/acpx-feel/feel.flow.ts",
      "startedAt": "2026-11-18T19:12:14.893Z",
      "status": "waiting",
      "traceSchema": "acpx.flow-trace-event.v1",
      "paths": {
        "flow": "flow.json",
        "trace": "trace.ndjson",
        "runProjection": "projections/run.json",
        "liveProjection": "projections/live.json",
        "stepsProjection": "projections/steps.json",
        "sessionsDir": "sessions",
        "artifactsDir": "artifacts"
      },
      "sessions": [
        {
          "id": "main-2c2263a9",
          "handle": "main",
          "bindingPath": "sessions/main-2c2263a9/binding.json",
          "recordPath": "sessions/main-2c2263a9/record.json",
          "eventsPath": "sessions/main-2c2263a9/events.ndjson"
        }
      ]
    }"#;

    const SAMPLE_FLOW: &str = r#"{
      "schema": "acpx.flow-definition-snapshot.v1",
      "name": "feel",
      "run": {"hasTitle": true},
      "startAt": "list",
      "nodes": {
        "list": {"nodeType": "action", "actionExecution": "shell", "hasRun": false, "hasExec": true, "hasParse": true},
        "count": {"nodeType": "compute", "hasRun": true},
        "size": {"nodeType": "acp", "session": {}, "cwd": {"mode": "default"}, "hasPrompt": true, "hasParse": true},
        "describe_small": {"nodeType": "acp", "session": {}, "cwd": {"mode": "default"}, "hasPrompt": true, "hasParse": false},
        "describe_large": {"nodeType": "acp", "session": {}, "cwd": {"mode": "default"}, "hasPrompt": true, "hasParse": false},
        "review": {"nodeType": "checkpoint", "summary": "human review of the description", "hasRun": false},
        "orphan_b": {"nodeType": "compute", "hasRun": true},
        "orphan_a": {"nodeType": "action", "actionExecution": "exec"}
      },
      "edges": [
        {"from": "list", "to": "count"},
        {"from": "count", "to": "size"},
        {"from": "size", "switch": {"on": "$.route", "cases": {"small": "describe_small", "large": "describe_large"}}},
        {"from": "describe_small", "to": "review"},
        {"from": "describe_large", "to": "review"}
      ]
    }"#;

    const SAMPLE_LIVE_WAITING: &str = r#"{
      "runId": "2026-11-18T191214880Z-feel-4832e848",
      "flowName": "feel",
      "runTitle": "feel: /tmp/acpx-feel",
      "flowPath": "/private/tmp/acpx-feel/feel.flow.ts",
      "startedAt": "2026-11-18T19:12:14.893Z",
      "updatedAt": "2026-11-18T19:12:26.190Z",
      "status": "waiting",
      "statusDetail": "human review of the description",
      "waitingOn": "review",
      "sessionBindings": {
        "k1": {
          "key": "k1",
          "handle": "main",
          "bundleId": "main-2c2263a9",
          "name": "feel-main-c20d0796-4832e848",
          "agentName": "claude",
          "cwd": "/private/tmp/acpx-feel"
        }
      }
    }"#;

    const SAMPLE_STEPS: &str = r#"[
      {
        "attemptId": "list#1",
        "nodeId": "list",
        "nodeType": "action",
        "outcome": "ok",
        "startedAt": "2026-11-18T19:12:14.903Z",
        "finishedAt": "2026-11-18T19:12:14.916Z",
        "promptText": null,
        "rawText": "err.txt\nfeel.flow.ts\nout.json\n",
        "output": {"files": ["err.txt", "feel.flow.ts", "out.json"]},
        "session": null,
        "agent": null,
        "trace": {
          "action": {"actionType": "shell", "command": "ls", "args": ["-1", "/tmp/acpx-feel"], "cwd": "/private/tmp/acpx-feel", "exitCode": 0, "signal": null, "durationMs": 7},
          "stdoutArtifact": {"path": "artifacts/stdout.txt", "mediaType": "text/plain", "bytes": 30, "sha256": "abc"},
          "stderrArtifact": {"path": "artifacts/stderr.txt", "mediaType": "text/plain", "bytes": 0, "sha256": "def"}
        }
      },
      {
        "attemptId": "count#1",
        "nodeId": "count",
        "nodeType": "compute",
        "outcome": "ok",
        "startedAt": "2026-11-18T19:12:14.918Z",
        "finishedAt": "2026-11-18T19:12:14.920Z",
        "promptText": null,
        "rawText": null,
        "output": {"n": 3},
        "session": null,
        "agent": null,
        "trace": {"outputInline": {"n": 3}}
      },
      {
        "attemptId": "size#1",
        "nodeId": "size",
        "nodeType": "acp",
        "outcome": "ok",
        "startedAt": "2026-11-18T19:12:14.922Z",
        "finishedAt": "2026-11-18T19:12:18.783Z",
        "promptText": "A directory has 3 entries.",
        "rawText": "{\"route\":\"small\"}",
        "output": {"route": "small"},
        "agent": {"agentName": "claude"},
        "trace": {
          "sessionId": "main-2c2263a9",
          "conversation": {"sessionId": "main-2c2263a9", "messageStart": 0, "messageEnd": 1, "eventStartSeq": 1, "eventEndSeq": 16}
        }
      },
      {
        "attemptId": "describe_small#1",
        "nodeId": "describe_small",
        "nodeType": "acp",
        "outcome": "ok",
        "startedAt": "2026-11-18T19:12:18.786Z",
        "finishedAt": "2026-11-18T19:12:26.184Z",
        "promptText": "Describe",
        "rawText": "Description text",
        "output": "Description text",
        "agent": {"agentName": "claude"},
        "trace": {
          "sessionId": "main-2c2263a9",
          "conversation": {"sessionId": "main-2c2263a9", "messageStart": 2, "messageEnd": 3, "eventStartSeq": 17, "eventEndSeq": 62},
          "outputArtifact": {"path": "artifacts/out.txt", "mediaType": "text/plain", "bytes": 16}
        }
      },
      {
        "attemptId": "review#1",
        "nodeId": "review",
        "nodeType": "checkpoint",
        "outcome": "ok",
        "startedAt": "2026-11-18T19:12:26.187Z",
        "finishedAt": "2026-11-18T19:12:26.190Z",
        "promptText": null,
        "rawText": null,
        "output": {"checkpoint": "review", "summary": "human review of the description"},
        "session": null,
        "agent": null,
        "trace": {}
      }
    ]"#;

    const SAMPLE_TRACE: &str = concat!(
        r#"{"seq": 1, "at": "2026-11-18T19:12:14.902Z", "runId": "r1", "scope": "run", "type": "run_started", "payload": {"flowName": "feel"}}"#, "\n",
        r#"{"seq": 2, "at": "2026-11-18T19:12:14.906Z", "runId": "r1", "scope": "node", "type": "node_started", "nodeId": "list", "attemptId": "list#1", "payload": {"nodeType": "action"}}"#, "\n",
        r#"{"seq": 3, "at": "2026-11-18T19:12:14.907Z", "runId": "r1", "scope": "node", "type": "node_heartbeat", "nodeId": "list", "attemptId": "list#1", "payload": {"statusDetail": "shell: ls"}}"#, "\n",
        r#"{"seq": 4, "at": "2026-11-18T19:12:14.907Z", "runId": "r1", "scope": "action", "type": "action_prepared", "nodeId": "list", "attemptId": "list#1", "payload": {"action": {"actionType": "shell", "command": "ls", "args": ["-1", "/tmp/acpx-feel"]}}}"#, "\n",
        r#"{"seq": 5, "at": "2026-11-18T19:12:14.915Z", "runId": "r1", "scope": "artifact", "type": "artifact_written", "nodeId": "list", "attemptId": "list#1", "payload": {"artifact": {"path": "artifacts/sha256-b5d3.txt", "mediaType": "text/plain", "bytes": 30}}}"#, "\n",
        r#"{"seq": 7, "at": "2026-11-18T19:12:14.916Z", "runId": "r1", "scope": "action", "type": "action_completed", "nodeId": "list", "attemptId": "list#1", "payload": {"action": {"actionType": "shell", "command": "ls", "args": ["-1"], "exitCode": 0, "durationMs": 7}}}"#, "\n",
        r#"{"seq": 8, "at": "2026-11-18T19:12:14.918Z", "runId": "r1", "scope": "node", "type": "node_outcome", "nodeId": "list", "attemptId": "list#1", "payload": {"nodeType": "action", "outcome": "ok", "durationMs": 13, "error": null}}"#, "\n",
        r#"{"seq": 14, "at": "2026-11-18T19:12:15.622Z", "runId": "r1", "scope": "session", "type": "session_bound", "sessionId": "main-2c2263a9", "payload": {"sessionId": "main-2c2263a9", "handle": "main"}}"#, "\n",
        r#"{"seq": 15, "at": "2026-11-18T19:12:15.622Z", "runId": "r1", "scope": "acp", "type": "acp_prompt_prepared", "nodeId": "size", "attemptId": "size#1", "payload": {"promptArtifact": {"path": "artifacts/p.txt", "mediaType": "text/plain", "bytes": 248}}}"#, "\n",
        r#"{"seq": 17, "at": "2026-11-18T19:12:18.783Z", "runId": "r1", "scope": "acp", "type": "acp_response_parsed", "nodeId": "size", "attemptId": "size#1", "payload": {"rawResponseArtifact": {"path": "artifacts/r.txt", "mediaType": "text/plain", "bytes": 82}}}"#, "\n"
    );

    fn write_bundle(dir: &Path) {
        fs::create_dir_all(dir.join("projections")).ok();
        fs::create_dir_all(dir.join("artifacts")).ok();
        fs::create_dir_all(dir.join("sessions/main-2c2263a9")).ok();
        fs::write(dir.join("manifest.json"), SAMPLE_MANIFEST).ok();
        fs::write(dir.join("flow.json"), SAMPLE_FLOW).ok();
        fs::write(dir.join("projections/live.json"), SAMPLE_LIVE_WAITING).ok();
        fs::write(
            dir.join("projections/run.json"),
            r#"{"input": {"target": "/tmp/acpx-feel"}}"#,
        )
        .ok();
        fs::write(dir.join("projections/steps.json"), SAMPLE_STEPS).ok();
        fs::write(dir.join("trace.ndjson"), SAMPLE_TRACE).ok();
    }

    #[test]
    fn test_waiting_run_and_sessions() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        assert_eq!(run.summary.status, RunStatus::Waiting);
        assert_eq!(run.summary.waiting_on.as_deref(), Some("review"));
        assert_eq!(run.summary.current_node, None);
        assert_eq!(run.node_state("review"), NodeState::Waiting);
        assert_eq!(run.node_state("list"), NodeState::Done(Outcome::Ok));
        assert_eq!(run.node_state("describe_large"), NodeState::NotRun);

        assert_eq!(run.sessions.len(), 1);
        assert_eq!(run.sessions[0].id, "main-2c2263a9");
        assert_eq!(run.sessions[0].handle, "main");
        assert_eq!(run.sessions[0].agent_name.as_deref(), Some("claude"));
        assert_eq!(
            run.session_events_path("main-2c2263a9"),
            Some(
                tmp.path()
                    .join("sessions/main-2c2263a9/events.ndjson")
                    .as_path()
            )
        );
    }

    #[test]
    fn test_running_run_current_node() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());
        fs::write(
            tmp.path().join("projections/live.json"),
            r#"{
              "runId": "r1",
              "flowName": "feel",
              "startedAt": "2026-11-18T19:12:14.893Z",
              "updatedAt": "2026-11-18T19:12:19.000Z",
              "status": "running",
              "currentNode": "describe_large"
            }"#,
        )
        .ok();

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        assert!(run.summary.status.is_live());
        assert_eq!(run.node_state("describe_large"), NodeState::Running);
        assert_eq!(run.node_state("describe_large").glyph(), "●");
    }

    #[test]
    fn test_failed_step_and_node_state() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());
        fs::write(
            tmp.path().join("projections/steps.json"),
            r#"[{
              "attemptId": "list#1",
              "nodeId": "list",
              "nodeType": "action",
              "outcome": "failed",
              "startedAt": "2026-11-18T19:12:14.000Z",
              "finishedAt": "2026-11-18T19:12:15.500Z",
              "error": "command exited 1"
            }]"#,
        )
        .ok();

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        assert_eq!(run.steps.len(), 1);
        assert_eq!(run.steps[0].outcome, Outcome::Failed);
        assert_eq!(run.steps[0].duration_ms(), 1500);
        assert_eq!(run.steps[0].error.as_deref(), Some("command exited 1"));
        assert_eq!(run.node_state("list"), NodeState::Done(Outcome::Failed));
        assert_eq!(run.node_state("list").glyph(), "✗");
    }

    #[test]
    fn test_flow_switch_and_bfs_order_with_unreachable() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        let ids: Vec<&str> = run.flow.nodes.iter().map(|n| n.id.as_str()).collect();
        // Switch cases sorted by case key ("large" then "small"), then review, then unreachable sorted ("orphan_a", "orphan_b")
        assert_eq!(
            ids,
            vec![
                "list",
                "count",
                "size",
                "describe_large",
                "describe_small",
                "review",
                "orphan_a",
                "orphan_b"
            ]
        );
        assert!(run.flow.nodes[0].is_shell);
        assert!(!run.flow.nodes[6].is_shell);
        assert_eq!(
            run.flow.nodes[5].summary.as_deref(),
            Some("human review of the description")
        );

        let switch_edge = run
            .flow
            .edges
            .iter()
            .find(|e| matches!(e, FlowEdge::Switch { .. }));
        assert_eq!(
            switch_edge,
            Some(&FlowEdge::Switch {
                from: "size".to_owned(),
                on: "$.route".to_owned(),
                cases: vec![
                    ("large".to_owned(), "describe_large".to_owned()),
                    ("small".to_owned(), "describe_small".to_owned()),
                ],
            })
        );
    }

    #[test]
    fn test_taken_path_and_edge_taken() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        assert_eq!(
            run.taken_path(),
            vec!["list", "count", "size", "describe_small", "review"]
        );
        assert!(run.edge_taken("list", "count"));
        assert!(run.edge_taken("size", "describe_small"));
        assert!(!run.edge_taken("size", "describe_large"));
        assert!(!run.edge_taken("describe_large", "review"));
    }

    #[test]
    fn test_progress() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        // 5 distinct nodes executed out of 8 total nodes in flow.json
        assert_eq!(run.progress(), (5, 8));
    }

    #[test]
    fn test_torn_ndjson_and_trace_details() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());

        let mut trace_with_torn = String::from(SAMPLE_TRACE);
        trace_with_torn.push_str("\n   \n{\"seq\": 99, \"at\": \"2026-11-18T19:12:20"); // torn last line
        fs::write(tmp.path().join("trace.ndjson"), trace_with_torn).ok();

        let run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        assert_eq!(run.events.len(), 10);
        assert_eq!(run.events[0].detail, "feel");
        assert_eq!(run.events[1].detail, "action");
        assert_eq!(run.events[2].detail, "shell: ls");
        assert_eq!(run.events[3].detail, "ls -1 /tmp/acpx-feel");
        assert_eq!(run.events[4].detail, "sha256-b5d3.txt · 30 B");
        assert_eq!(run.events[5].detail, "exit 0 · 7ms");
        assert_eq!(run.events[6].detail, "ok · 13ms");
        assert_eq!(run.events[7].detail, "session main");
        assert_eq!(run.events[8].detail, "prompt · 248 B");
        assert_eq!(run.events[9].detail, "response · 82 B");

        let list_ev = run.step_events("list#1");
        assert_eq!(list_ev.len(), 6);
    }

    #[test]
    fn test_refresh_keeps_half_written_and_detects_change() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());

        let mut run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        let orig_steps_len = run.steps.len();
        assert!(!refresh(&mut run));

        // Corrupt steps.json as if half-written; update live.json status to completed
        fs::write(tmp.path().join("projections/steps.json"), "[{\"attemptId\":").ok();
        fs::write(
            tmp.path().join("projections/live.json"),
            r#"{
              "runId": "2026-11-18T191214880Z-feel-4832e848",
              "flowName": "feel",
              "startedAt": "2026-11-18T19:12:14.893Z",
              "updatedAt": "2026-11-18T19:12:30.000Z",
              "finishedAt": "2026-11-18T19:12:30.000Z",
              "status": "completed"
            }"#,
        )
        .ok();

        assert!(refresh(&mut run));
        assert_eq!(run.steps.len(), orig_steps_len);
        assert_eq!(run.summary.status, RunStatus::Completed);
        assert!(!refresh(&mut run));
    }

    #[test]
    fn test_list_runs_ordering_and_manifest_fallback() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        let r_old = tmp.path().join("run-old");
        let r_new = tmp.path().join("run-new");
        let r_junk = tmp.path().join("not-a-run");

        fs::create_dir_all(&r_old).ok();
        fs::create_dir_all(r_new.join("projections")).ok();
        fs::create_dir_all(&r_junk).ok();

        // r_old only has manifest.json (no live.json)
        fs::write(
            r_old.join("manifest.json"),
            r#"{
              "runId": "run-old",
              "flowName": "alpha",
              "startedAt": "2026-11-18T10:00:00.000Z",
              "status": "failed"
            }"#,
        )
        .ok();

        // r_new has live.json
        fs::write(
            r_new.join("projections/live.json"),
            r#"{
              "runId": "run-new",
              "flowName": "beta",
              "runTitle": "Beta Title",
              "startedAt": "2026-11-18T12:00:00.000Z",
              "status": "completed"
            }"#,
        )
        .ok();

        let runs = list_runs(tmp.path());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "run-new");
        assert_eq!(runs[0].title, "Beta Title");
        assert_eq!(runs[0].status, RunStatus::Completed);
        assert_eq!(runs[1].run_id, "run-old");
        assert_eq!(runs[1].title, "alpha");
        assert_eq!(runs[1].status, RunStatus::Failed);
    }

    #[test]
    fn test_read_artifact_security_and_utf8_cap() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        fs::create_dir_all(tmp.path().join("artifacts")).ok();
        // "aé🎉b" -> 'a' (1B), 'é' (2B), '🎉' (4B), 'b' (1B)
        fs::write(tmp.path().join("artifacts/data.txt"), "aé🎉b").ok();

        let parent_ref = ArtifactRef {
            path: "../secret.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            bytes: 10,
        };
        assert!(read_artifact(tmp.path(), &parent_ref, 100).is_err());

        let nested_parent_ref = ArtifactRef {
            path: "artifacts/../../secret.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            bytes: 10,
        };
        assert!(read_artifact(tmp.path(), &nested_parent_ref, 100).is_err());

        let abs_ref = ArtifactRef {
            path: "/etc/passwd".to_owned(),
            media_type: "text/plain".to_owned(),
            bytes: 10,
        };
        assert!(read_artifact(tmp.path(), &abs_ref, 100).is_err());

        let valid_ref = ArtifactRef {
            path: "artifacts/data.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            bytes: 8,
        };
        // Capping at 5 bytes cuts inside the 4-byte emoji '🎉' (bytes 3..7); should truncate cleanly to "aé"
        let s = read_artifact(tmp.path(), &valid_ref, 5)
            .ok()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(s, "aé");
    }

    #[test]
    fn test_parse_ts() {
        let ms = parse_ts("2026-11-18T19:12:14.893Z");
        assert_eq!(ms, Some(1_795_029_134_893));
        assert_eq!(parse_ts("not-a-timestamp"), None);
    }

    #[test]
    fn test_fmt_duration_units() {
        assert_eq!(fmt_duration(0), "0ms");
        assert_eq!(fmt_duration(7), "7ms");
        assert_eq!(fmt_duration(999), "999ms");
        assert_eq!(fmt_duration(3_861), "3.9s");
        assert_eq!(fmt_duration(125_000), "2m05s");
        assert_eq!(fmt_duration(3_840_000), "1h04m");
    }

    #[test]
    fn test_value_text() {
        assert_eq!(value_text(&Value::Null), "");
        assert_eq!(
            value_text(&Value::String("hello\nworld".to_owned())),
            "hello\nworld"
        );
        let obj = serde_json::json!({"a": 1});
        assert_eq!(value_text(&obj), "{\n  \"a\": 1\n}");
    }

    #[test]
    fn test_enums_glyphs_and_labels() {
        assert_eq!(RunStatus::Running.glyph(), "●");
        assert_eq!(RunStatus::Waiting.glyph(), "◐");
        assert_eq!(RunStatus::Completed.glyph(), "✓");
        assert_eq!(RunStatus::Failed.glyph(), "✗");
        assert_eq!(RunStatus::TimedOut.glyph(), "✗");
        assert_eq!(RunStatus::Unknown.glyph(), "○");

        assert_eq!(NodeKind::Acp.label(), "acp");
        assert_eq!(NodeKind::Action.label(), "action");
        assert_eq!(NodeKind::Compute.label(), "compute");
        assert_eq!(NodeKind::Checkpoint.label(), "checkpoint");
        assert_eq!(NodeKind::Unknown.label(), "?");

        assert_eq!(NodeState::NotRun.glyph(), "○");
        assert_eq!(NodeState::Running.glyph(), "●");
        assert_eq!(NodeState::Waiting.glyph(), "◐");
        assert_eq!(NodeState::Done(Outcome::Ok).glyph(), "✓");
        assert_eq!(NodeState::Done(Outcome::Failed).glyph(), "✗");
        assert_eq!(NodeState::Done(Outcome::TimedOut).glyph(), "✗");
        assert_eq!(NodeState::Done(Outcome::Cancelled).glyph(), "⊘");
        assert_eq!(NodeState::Done(Outcome::Unknown).glyph(), "?");
    }
}
