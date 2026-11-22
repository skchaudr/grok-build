=== REPOSITORY CONTEXT (15 complete files) ===

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` (complete file, repository grok-build-acpx)
````
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

    /// `node_state`, except that a gate which has been answered is done: the
    /// checkpoint is no longer waiting on anyone.
    pub fn node_state_answered(&self, node_id: &str, answered: bool) -> NodeState {
        match self.node_state(node_id) {
            NodeState::Waiting if answered => NodeState::Done(Outcome::Ok),
            state => state,
        }
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

    let mut summary = summary;
    restore_question(&mut summary, &flow);
    Ok(Run {
        summary,
        flow,
        steps,
        events,
        input,
        sessions,
    })
}

/// acpx reports a checkpoint that has its own `run` with only its node id as the detail. Show the checkpoint's summary.
fn restore_question(summary: &mut RunSummary, flow: &FlowDef) {
    if summary.status != RunStatus::Waiting || summary.status_detail.as_deref() != summary.waiting_on.as_deref() {
        return;
    }
    let node = flow.nodes.iter().find(|n| Some(n.id.as_str()) == summary.waiting_on.as_deref());
    if let Some(question) = node.and_then(|n| n.summary.clone()) {
        summary.status_detail = Some(question);
    }
}

/// Re-read a run in place. Each file that fails to parse (acpx rewrites them non-atomically) keeps its previous value.
/// Returns true when anything changed.
pub fn refresh(run: &mut Run) -> bool {
    let dir = run.summary.dir.clone();
    let mut changed = false;

    let live_val = read_json_file(&dir.join("projections/live.json"));
    let manifest_val = read_json_file(&dir.join("manifest.json"));
    let run_proj_val = read_json_file(&dir.join("projections/run.json"));

    if let Some(flow_val) = read_json_file(&dir.join("flow.json")) {
        let new_flow = parse_flow_def(&flow_val);
        if new_flow != run.flow {
            run.flow = new_flow;
            changed = true;
        }
    }

    if live_val.is_some() || manifest_val.is_some() {
        let mut new_summary = build_summary(&dir, live_val.as_ref(), manifest_val.as_ref());
        restore_question(&mut new_summary, &run.flow);
        if new_summary != run.summary {
            run.summary = new_summary;
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
    // A run that itself came from a gate carries the first run's input one level down: keep handing that on.
    let came_from_gate = run.input.get("gate").is_some() && run.input.get("input").is_some();
    let input = if came_from_gate { &run.input["input"] } else { &run.input };
    serde_json::json!({
        "input": input,
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
    fn test_refresh_keeps_the_restored_gate_question() {
        let tmp = tempdir().ok().unwrap_or_else(|| unreachable!());
        write_bundle(tmp.path());
        // acpx reports only the checkpoint's node id as the detail of a waiting run.
        let live = SAMPLE_LIVE_WAITING.replace(
            r#""statusDetail": "human review of the description""#,
            r#""statusDetail": "review""#,
        );
        fs::write(tmp.path().join("projections/live.json"), live).ok();

        let mut run = load_run(tmp.path()).ok().unwrap_or_else(|| unreachable!());
        assert_eq!(run.summary.status_detail.as_deref(), Some("human review of the description"));
        assert!(!refresh(&mut run));
        assert_eq!(run.summary.status_detail.as_deref(), Some("human review of the description"));
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

````

### `crates/codegen/xai-grok-pager/src/acpx/sessions.rs` (complete file, repository grok-build-acpx)
````
use std::fs;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub record_id: String,
    pub acp_session_id: String,
    pub name: String,
    pub title: String,
    pub agent: String,
    pub cwd: String,
    pub created_ms: i64,
    pub last_used_ms: i64,
    pub closed: bool,
    pub messages: usize,
    pub tokens: u64,
    pub cost_usd: Option<f64>,
    pub stream_path: PathBuf,
}

pub fn sessions_base() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".acpx").join("sessions"))
}

pub fn list_sessions(base: &Path) -> Vec<SessionSummary> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let is_json = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        if !is_json {
            continue;
        }
        if let Some(summary) = parse_session_file(base, &path) {
            out.push(summary);
        }
    }

    out.sort_by(|a, b| {
        b.last_used_ms
            .cmp(&a.last_used_ms)
            .then_with(|| b.created_ms.cmp(&a.created_ms))
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    out
}

pub fn agent_label(agent_command: &str) -> String {
    let mut words = agent_command.split_whitespace().peekable();
    let Some(first) = words.peek().copied() else {
        return "?".to_string();
    };

    let first_base = basename_str(first);
    if first_base == "env" {
        words.next();
    }

    while let Some(&word) = words.peek() {
        if word == "-u" || word == "--unset" {
            words.next();
            words.next();
            continue;
        }
        if word.starts_with("-u") && word.len() > 2 {
            words.next();
            continue;
        }
        if let Some(rest) = word.strip_prefix("--unset=") {
            let _ = rest;
            words.next();
            continue;
        }
        if word == "-i" || word == "-" || word == "--ignore-environment" {
            words.next();
            continue;
        }
        if is_env_assignment(word) {
            words.next();
            continue;
        }
        break;
    }

    let Some(cmd) = words.next() else {
        return "?".to_string();
    };

    let base = basename_str(cmd);
    if base.is_empty() {
        return "?".to_string();
    }

    let stripped = base
        .strip_suffix("-agent-acp")
        .or_else(|| base.strip_suffix("-code-acp"))
        .or_else(|| base.strip_suffix("-acp"))
        .unwrap_or(base);

    if stripped.is_empty() {
        "?".to_string()
    } else {
        stripped.to_string()
    }
}

fn basename_str(s: &str) -> &str {
    let trimmed = s.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(idx) => &trimmed[idx + 1..],
        None => trimmed,
    }
}

fn is_env_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    if name.is_empty() || name.contains(['/', '\\']) {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_session_file(base: &Path, path: &Path) -> Option<SessionSummary> {
    let bytes = fs::read(path).ok()?;
    let val: Value = serde_json::from_slice(&bytes).ok()?;
    let obj = val.as_object()?;

    let record_id = get_non_empty_str(obj, &["acpx_record_id", "acpxRecordId"])?;
    let acp_session_id =
        get_str(obj, &["acp_session_id", "acpSessionId"]).unwrap_or_default();
    let name = get_str(obj, &["name"]).unwrap_or_default();
    let raw_title = get_str(obj, &["title"]).unwrap_or_default();

    let title = if !raw_title.is_empty() {
        raw_title
    } else if !name.is_empty() {
        name.clone()
    } else {
        record_id.clone()
    };

    let agent_cmd = get_str(obj, &["agent_command", "agentCommand"])
        .filter(|s| !s.is_empty())
        .or_else(|| {
            get_field(obj, &["agent_argv", "agentArgv"])
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.is_empty())
        })
        .or_else(|| get_str(obj, &["agent_name", "agentName"]).filter(|s| !s.is_empty()))
        .unwrap_or_default();

    let agent = agent_label(&agent_cmd);
    let cwd = get_str(obj, &["cwd"]).unwrap_or_default();

    let created_ms = get_str(obj, &["created_at", "createdAt"])
        .map(|s| parse_ts_ms(&s))
        .unwrap_or(0);

    let last_used_ms = get_str(
        obj,
        &["last_used_at", "lastUsedAt", "updated_at", "updatedAt"],
    )
    .map(|s| parse_ts_ms(&s))
    .unwrap_or(created_ms);

    let closed = get_field(obj, &["closed"])
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let messages = get_field(obj, &["messages"])
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let tokens = get_field(obj, &["cumulative_token_usage", "cumulativeTokenUsage"])
        .map(extract_tokens)
        .unwrap_or(0);

    let cost_usd =
        get_field(obj, &["cumulative_cost", "cumulativeCost"]).and_then(extract_cost);

    let stream_path = base.join(format!("{record_id}.stream.ndjson"));

    Some(SessionSummary {
        record_id,
        acp_session_id,
        name,
        title,
        agent,
        cwd,
        created_ms,
        last_used_ms,
        closed,
        messages,
        tokens,
        cost_usd,
        stream_path,
    })
}

fn get_field<'a>(
    obj: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    for &k in keys {
        if let Some(v) = obj.get(k) {
            if !v.is_null() {
                return Some(v);
            }
        }
    }
    None
}

fn get_str(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Some(s) = obj.get(k).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

fn get_non_empty_str(
    obj: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    for &k in keys {
        if let Some(s) = obj.get(k).and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_ts_ms(s: &str) -> i64 {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn value_as_u64(v: &Value) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        Some(u)
    } else if let Some(i) = v.as_i64() {
        u64::try_from(i).ok()
    } else if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 {
            Some(f as u64)
        } else {
            None
        }
    } else {
        None
    }
}

fn extract_tokens(val: &Value) -> u64 {
    if let Some(n) = value_as_u64(val) {
        return n;
    }
    if let Some(map) = val.as_object() {
        for key in ["total", "total_tokens", "totalTokens"] {
            if let Some(v) = map.get(key).and_then(value_as_u64) {
                return v;
            }
        }
        let mut sum: u64 = 0;
        for v in map.values() {
            if let Some(n) = value_as_u64(v) {
                sum = sum.saturating_add(n);
            }
        }
        return sum;
    }
    0
}

fn extract_cost(val: &Value) -> Option<f64> {
    if let Some(f) = val.as_f64().filter(|f| f.is_finite()) {
        return Some(f);
    }
    if let Some(map) = val.as_object() {
        for key in ["amount", "usd", "total"] {
            if let Some(f) = map
                .get(key)
                .and_then(Value::as_f64)
                .filter(|f| f.is_finite())
            {
                return Some(f);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_json(dir: &Path, filename: &str, value: &Value) {
        let path = dir.join(filename);
        let text = serde_json::to_string(value).unwrap_or_default();
        let _ = fs::write(path, text);
    }

    #[test]
    fn test_ordering_by_last_use() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "older.json",
            &json!({
                "acpx_record_id": "rec-older",
                "acp_session_id": "sess-1",
                "name": "older",
                "created_at": "2026-11-18T19:00:00Z",
                "last_used_at": "2026-11-18T19:10:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "newer.json",
            &json!({
                "acpx_record_id": "rec-newer",
                "acp_session_id": "sess-2",
                "name": "newer",
                "created_at": "2026-11-18T18:00:00Z",
                "last_used_at": "2026-11-18T19:20:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "middle.json",
            &json!({
                "acpx_record_id": "rec-middle",
                "acp_session_id": "sess-3",
                "name": "middle",
                "created_at": "2026-11-18T19:05:00Z",
                "last_used_at": "2026-11-18T19:15:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].record_id, "rec-newer");
        assert_eq!(list[1].record_id, "rec-middle");
        assert_eq!(list[2].record_id, "rec-older");
        assert_eq!(
            list[0].stream_path,
            tmp.path().join("rec-newer.stream.ndjson")
        );
    }

    #[test]
    fn test_title_fallbacks() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "s1.json",
            &json!({
                "acpx_record_id": "id-1",
                "name": "my-name",
                "title": "Explicit Title",
                "last_used_at": "2026-11-18T19:03:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "s2.json",
            &json!({
                "acpx_record_id": "id-2",
                "name": "fallback-name",
                "title": "",
                "last_used_at": "2026-11-18T19:02:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "s3.json",
            &json!({
                "acpx_record_id": "id-3",
                "last_used_at": "2026-11-18T19:01:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].title, "Explicit Title");
        assert_eq!(list[1].title, "fallback-name");
        assert_eq!(list[2].title, "id-3");
    }

    #[test]
    fn test_both_casings_and_bundle_sample() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "snake.json",
            &json!({
                "schema": "acpx.session.v1",
                "acpx_record_id": "00000000-0000-4000-8000-000000000002",
                "acp_session_id": "00000000-0000-4000-8000-000000000000",
                "agent_command": "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/Users/me/.local/bin/claude /Users/me/.local/bin/claude-agent-acp",
                "cwd": "/private/tmp/acpx-feel",
                "name": "feel-main-c20d0796-4832e848",
                "title": "Directory size classification",
                "created_at": "2026-11-18T19:12:15.617Z",
                "last_used_at": "2026-11-18T19:12:25.549Z",
                "closed": false,
                "messages": [{"User": {}}, {"Agent": {}}],
                "cumulative_token_usage": {
                    "input_tokens": 6,
                    "output_tokens": 233,
                    "cache_creation_input_tokens": 9220,
                    "cache_read_input_tokens": 34694,
                    "total_tokens": 44153
                },
                "cumulative_cost": {"amount": 0.2960819, "currency": "USD"}
            }),
        );
        write_json(
            tmp.path(),
            "camel.json",
            &json!({
                "acpxRecordId": "rec-camel",
                "acpSessionId": "sess-camel",
                "agentCommand": "codex-acp --model o3",
                "cwd": "/work/dir",
                "name": "camel-session",
                "createdAt": "2026-11-18T10:00:00Z",
                "lastUsedAt": "2026-11-18T11:00:00Z",
                "closed": true,
                "messages": [{"a": 1}, {"b": 2}, {"c": 3}],
                "cumulativeTokenUsage": {"totalTokens": 999},
                "cumulativeCost": {"usd": 1.25}
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 2);

        let snake = &list[0];
        assert_eq!(snake.record_id, "00000000-0000-4000-8000-000000000002");
        assert_eq!(snake.acp_session_id, "00000000-0000-4000-8000-000000000000");
        assert_eq!(snake.agent, "claude");
        assert_eq!(snake.cwd, "/private/tmp/acpx-feel");
        assert_eq!(snake.title, "Directory size classification");
        assert!(!snake.closed);
        assert_eq!(snake.messages, 2);
        assert_eq!(snake.tokens, 44153);
        assert_eq!(snake.cost_usd, Some(0.2960819));

        let camel = &list[1];
        assert_eq!(camel.record_id, "rec-camel");
        assert_eq!(camel.acp_session_id, "sess-camel");
        assert_eq!(camel.agent, "codex");
        assert_eq!(camel.cwd, "/work/dir");
        assert_eq!(camel.title, "camel-session");
        assert!(camel.closed);
        assert_eq!(camel.messages, 3);
        assert_eq!(camel.tokens, 999);
        assert_eq!(camel.cost_usd, Some(1.25));
    }

    #[test]
    fn test_token_object_vs_number() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "num.json",
            &json!({
                "acpx_record_id": "t-num",
                "last_used_at": "2026-11-18T19:04:00Z",
                "cumulative_token_usage": 12345
            }),
        );
        write_json(
            tmp.path(),
            "total_field.json",
            &json!({
                "acpx_record_id": "t-total",
                "last_used_at": "2026-11-18T19:03:00Z",
                "cumulative_token_usage": {
                    "input_tokens": 10,
                    "output_tokens": 20,
                    "total": 500
                }
            }),
        );
        write_json(
            tmp.path(),
            "sum_fields.json",
            &json!({
                "acpx_record_id": "t-sum",
                "last_used_at": "2026-11-18T19:02:00Z",
                "cumulative_token_usage": {
                    "input_tokens": 100,
                    "output_tokens": 250,
                    "cache_read": 50
                }
            }),
        );
        write_json(
            tmp.path(),
            "absent.json",
            &json!({
                "acpx_record_id": "t-absent",
                "last_used_at": "2026-11-18T19:01:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].tokens, 12345);
        assert_eq!(list[1].tokens, 500);
        assert_eq!(list[2].tokens, 400);
        assert_eq!(list[3].tokens, 0);
    }

    #[test]
    fn test_cost_shapes() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "c1.json",
            &json!({
                "acpx_record_id": "c-num",
                "last_used_at": "2026-11-18T19:05:00Z",
                "cumulative_cost": 0.42
            }),
        );
        write_json(
            tmp.path(),
            "c2.json",
            &json!({
                "acpx_record_id": "c-amount",
                "last_used_at": "2026-11-18T19:04:00Z",
                "cumulative_cost": {"amount": 0.29, "currency": "USD"}
            }),
        );
        write_json(
            tmp.path(),
            "c3.json",
            &json!({
                "acpx_record_id": "c-usd",
                "last_used_at": "2026-11-18T19:03:00Z",
                "cumulative_cost": {"usd": 1.75}
            }),
        );
        write_json(
            tmp.path(),
            "c4.json",
            &json!({
                "acpx_record_id": "c-total",
                "last_used_at": "2026-11-18T19:02:00Z",
                "cumulative_cost": {"total": 3.5}
            }),
        );
        write_json(
            tmp.path(),
            "c5.json",
            &json!({
                "acpx_record_id": "c-none",
                "last_used_at": "2026-11-18T19:01:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].cost_usd, Some(0.42));
        assert_eq!(list[1].cost_usd, Some(0.29));
        assert_eq!(list[2].cost_usd, Some(1.75));
        assert_eq!(list[3].cost_usd, Some(3.5));
        assert_eq!(list[4].cost_usd, None);
    }

    #[test]
    fn test_agent_label_variants() {
        assert_eq!(
            agent_label(
                "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/x/claude /Users/me/.local/bin/claude-agent-acp"
            ),
            "claude"
        );
        assert_eq!(agent_label("codex-acp --foo"), "codex");
        assert_eq!(agent_label("/opt/x/grok-pi-acp"), "grok-pi");
        assert_eq!(agent_label("/usr/local/bin/claude-code-acp --bar"), "claude");
        assert_eq!(agent_label(""), "?");
        assert_eq!(agent_label("   "), "?");
        assert_eq!(agent_label("env -u FOO BAR=1"), "?");
    }

    #[test]
    fn test_skips_invalid_and_non_json_files() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "valid.json",
            &json!({
                "acpx_record_id": "valid-1",
                "created_at": "not-a-timestamp",
                "last_used_at": "also-bad"
            }),
        );
        // Non-json file
        let _ = fs::write(
            tmp.path().join("valid-1.stream.ndjson"),
            "{\"acpx_record_id\": \"should-ignore\"}\n",
        );
        // Broken json file
        let _ = fs::write(tmp.path().join("broken.json"), "{not valid json");
        // Missing acpx_record_id
        write_json(
            tmp.path(),
            "no_id.json",
            &json!({
                "name": "missing-id"
            }),
        );
        // Subdirectory named *.json or containing json files
        let subdir = tmp.path().join("nested.json");
        let _ = fs::create_dir_all(&subdir);
        write_json(
            &subdir,
            "inner.json",
            &json!({
                "acpx_record_id": "inner-should-skip"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].record_id, "valid-1");
        assert_eq!(list[0].created_ms, 0);
        assert_eq!(list[0].last_used_ms, 0);
    }

    #[test]
    fn test_missing_base_directory_returns_empty() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        let nonexistent = tmp.path().join("does-not-exist");
        let list = list_sessions(&nonexistent);
        assert!(list.is_empty());
    }
}

````

### `crates/codegen/xai-grok-pager/src/acpx/graph.rs` (complete file, repository grok-build-acpx)
````
use std::collections::HashMap;
use crate::acpx::bundle::{FlowEdge, NodeKind, NodeState, Run};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LeftRight,
    TopDown,
}

fn escape_label(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' => '\'',
            '[' | ']' | '{' | '}' | '|' | '<' | '>' | '`' => ' ',
            _ => c,
        })
        .collect()
}

/// Mermaid `flowchart` source for a run: one node per flow node, labelled "<state glyph> <id>".
/// `answered` draws a waiting gate as done.
pub fn mermaid_source(run: &Run, direction: Direction, answered: bool) -> String {
    let mut lines = Vec::new();
    let dir_str = match direction {
        Direction::LeftRight => "LR",
        Direction::TopDown => "TD",
    };
    lines.push(format!("flowchart {dir_str}"));

    let mut id_map: HashMap<&str, String> = HashMap::with_capacity(run.flow.nodes.len());

    for (idx, node) in run.flow.nodes.iter().enumerate() {
        let mermaid_id = format!("n{idx}");
        id_map.insert(node.id.as_str(), mermaid_id.clone());

        let state: NodeState = run.node_state_answered(&node.id, answered);
        let raw_label = format!("{} {}", state.glyph(), node.id);
        let label = escape_label(&raw_label);

        let node_line = match node.kind {
            NodeKind::Acp => format!("  {mermaid_id}(\"{label}\")"),
            NodeKind::Checkpoint => format!("  {mermaid_id}{{\"{label}\"}}"),
            NodeKind::Action | NodeKind::Compute | NodeKind::Unknown => {
                format!("  {mermaid_id}[\"{label}\"]")
            }
        };
        lines.push(node_line);
    }

    for edge in &run.flow.edges {
        match edge {
            FlowEdge::Simple { from, to } => {
                let (Some(from_m), Some(to_m)) =
                    (id_map.get(from.as_str()), id_map.get(to.as_str()))
                else {
                    continue;
                };
                let arrow = edge_arrow(run, from, to);
                lines.push(format!("  {from_m} {arrow} {to_m}"));
            }
            FlowEdge::Switch { from, cases, .. } => {
                let Some(from_m) = id_map.get(from.as_str()) else {
                    continue;
                };
                for (case_val, to) in cases {
                    let Some(to_m) = id_map.get(to.as_str()) else {
                        continue;
                    };
                    let arrow = edge_arrow(run, from, to);
                    let escaped_case = escape_label(case_val);
                    lines.push(format!("  {from_m} {arrow}|\"{escaped_case}\"| {to_m}"));
                }
            }
        }
    }

    lines.join("\n")
}

fn edge_arrow(run: &Run, from: &str, to: &str) -> &'static str {
    if run.edge_taken(from, to) {
        "==>"
    } else if run.steps.iter().any(|s| s.node_id == from) {
        "-.->"
    } else {
        "-->"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        FlowDef, FlowEdge, FlowNode, NodeKind, Outcome, Run, RunStatus, RunSummary, Step,
    };
    use serde_json::Value;
    use std::path::PathBuf;

    fn make_node(id: &str, kind: NodeKind) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            kind,
            summary: None,
            is_shell: false,
        }
    }

    fn make_step(node_id: &str, attempt: usize, kind: NodeKind, outcome: Outcome) -> Step {
        Step {
            attempt_id: format!("{node_id}#{attempt}"),
            node_id: node_id.to_string(),
            kind,
            outcome,
            started_ms: 1000,
            finished_ms: 2000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn make_run(
        status: RunStatus,
        current_node: Option<&str>,
        waiting_on: Option<&str>,
        nodes: Vec<FlowNode>,
        edges: Vec<FlowEdge>,
        steps: Vec<Step>,
    ) -> Run {
        let start_at = nodes.first().map(|n| n.id.clone()).unwrap_or_default();
        Run {
            summary: RunSummary {
                run_id: "run-1".to_string(),
                dir: PathBuf::from("/tmp/run-1"),
                flow_name: "test-flow".to_string(),
                title: "Test Flow".to_string(),
                status,
                started_ms: 1000,
                updated_ms: 2000,
                finished_ms: None,
                current_node: current_node.map(str::to_string),
                waiting_on: waiting_on.map(str::to_string),
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "test-flow".to_string(),
                start_at,
                nodes,
                edges,
            },
            steps,
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn test_linear_flow_all_done() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("init", NodeKind::Action),
                make_node("plan", NodeKind::Acp),
                make_node("finish", NodeKind::Compute),
            ],
            vec![
                FlowEdge::Simple {
                    from: "init".to_string(),
                    to: "plan".to_string(),
                },
                FlowEdge::Simple {
                    from: "plan".to_string(),
                    to: "finish".to_string(),
                },
            ],
            vec![
                make_step("init", 1, NodeKind::Action, Outcome::Ok),
                make_step("plan", 1, NodeKind::Acp, Outcome::Ok),
                make_step("finish", 1, NodeKind::Compute, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ init\"]",
                "  n1(\"✓ plan\")",
                "  n2[\"✓ finish\"]",
                "  n0 ==> n1",
                "  n1 ==> n2",
            ]
        );
    }

    #[test]
    fn test_switch_with_one_case_taken() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("size", NodeKind::Compute),
                make_node("fast", NodeKind::Action),
                make_node("deep", NodeKind::Acp),
            ],
            vec![FlowEdge::Switch {
                from: "size".to_string(),
                on: "bucket".to_string(),
                cases: vec![
                    ("large".to_string(), "deep".to_string()),
                    ("small".to_string(), "fast".to_string()),
                ],
            }],
            vec![
                make_step("size", 1, NodeKind::Compute, Outcome::Ok),
                make_step("fast", 1, NodeKind::Action, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ size\"]",
                "  n1[\"✓ fast\"]",
                "  n2(\"○ deep\")",
                "  n0 -.->|\"large\"| n2",
                "  n0 ==>|\"small\"| n1",
            ]
        );
    }

    #[test]
    fn test_node_currently_running() {
        let run = make_run(
            RunStatus::Running,
            Some("work"),
            None,
            vec![
                make_node("prep", NodeKind::Action),
                make_node("work", NodeKind::Acp),
            ],
            vec![FlowEdge::Simple {
                from: "prep".to_string(),
                to: "work".to_string(),
            }],
            vec![make_step("prep", 1, NodeKind::Action, Outcome::Ok)],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ prep\"]",
                "  n1(\"● work\")",
                "  n0 -.-> n1",
            ]
        );
    }

    #[test]
    fn test_waiting_checkpoint() {
        let run = make_run(
            RunStatus::Waiting,
            Some("gate"),
            Some("gate"),
            vec![
                make_node("build", NodeKind::Action),
                make_node("gate", NodeKind::Checkpoint),
                make_node("deploy", NodeKind::Action),
            ],
            vec![
                FlowEdge::Simple {
                    from: "build".to_string(),
                    to: "gate".to_string(),
                },
                FlowEdge::Simple {
                    from: "gate".to_string(),
                    to: "deploy".to_string(),
                },
            ],
            vec![make_step("build", 1, NodeKind::Action, Outcome::Ok)],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ build\"]",
                "  n1{\"◐ gate\"}",
                "  n2[\"○ deploy\"]",
                "  n0 -.-> n1",
                "  n1 --> n2",
            ]
        );
    }

    #[test]
    fn test_answered_gate_is_drawn_done() {
        let run = make_run(
            RunStatus::Waiting,
            Some("gate"),
            Some("gate"),
            vec![
                make_node("build", NodeKind::Action),
                make_node("gate", NodeKind::Checkpoint),
            ],
            vec![FlowEdge::Simple {
                from: "build".to_string(),
                to: "gate".to_string(),
            }],
            vec![make_step("build", 1, NodeKind::Action, Outcome::Ok)],
        );
        let open = mermaid_source(&run, Direction::LeftRight, false);
        let answered = mermaid_source(&run, Direction::LeftRight, true);
        assert!(open.contains("◐ gate") && !open.contains("✓ gate"));
        assert!(answered.contains("✓ gate") && !answered.contains('◐'));
    }

    #[test]
    fn test_nothing_run_yet() {
        let run = make_run(
            RunStatus::Running,
            None,
            None,
            vec![
                make_node("a", NodeKind::Action),
                make_node("b", NodeKind::Unknown),
                make_node("c", NodeKind::Acp),
            ],
            vec![
                FlowEdge::Simple {
                    from: "a".to_string(),
                    to: "b".to_string(),
                },
                FlowEdge::Switch {
                    from: "b".to_string(),
                    on: "res".to_string(),
                    cases: vec![("ok".to_string(), "c".to_string())],
                },
            ],
            vec![],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"○ a\"]",
                "  n1[\"○ b\"]",
                "  n2(\"○ c\")",
                "  n0 --> n1",
                "  n1 -->|\"ok\"| n2",
            ]
        );
    }

    #[test]
    fn test_label_escaping_special_characters() {
        let tricky_id = "a\"b[c]d{e}f|g<h>i`j";
        let tricky_case = "x\"y[z]{w}|<v>`u";
        let run = make_run(
            RunStatus::Failed,
            None,
            None,
            vec![
                make_node(tricky_id, NodeKind::Action),
                make_node("end", NodeKind::Action),
            ],
            vec![FlowEdge::Switch {
                from: tricky_id.to_string(),
                on: "k".to_string(),
                cases: vec![(tricky_case.to_string(), "end".to_string())],
            }],
            vec![make_step(tricky_id, 1, NodeKind::Action, Outcome::Failed)],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✗ a'b c d e f g h i j\"]",
                "  n1[\"○ end\"]",
                "  n0 -.->|\"x'y z  w   v  u\"| n1",
            ]
        );
    }

    #[test]
    fn test_top_down_header() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![make_node("only", NodeKind::Compute)],
            vec![],
            vec![make_step("only", 1, NodeKind::Compute, Outcome::Ok)],
        );

        let src = mermaid_source(&run, Direction::TopDown, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec!["flowchart TD", "  n0[\"✓ only\"]"]
        );
    }

    #[test]
    fn test_edge_to_unknown_node_skipped() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("a", NodeKind::Action),
                make_node("b", NodeKind::Action),
            ],
            vec![
                FlowEdge::Simple {
                    from: "a".to_string(),
                    to: "ghost".to_string(),
                },
                FlowEdge::Simple {
                    from: "ghost".to_string(),
                    to: "b".to_string(),
                },
                FlowEdge::Switch {
                    from: "a".to_string(),
                    on: "v".to_string(),
                    cases: vec![
                        ("missing".to_string(), "ghost".to_string()),
                        ("present".to_string(), "b".to_string()),
                    ],
                },
                FlowEdge::Switch {
                    from: "ghost".to_string(),
                    on: "v".to_string(),
                    cases: vec![("any".to_string(), "b".to_string())],
                },
            ],
            vec![
                make_step("a", 1, NodeKind::Action, Outcome::Ok),
                make_step("b", 1, NodeKind::Action, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ a\"]",
                "  n1[\"✓ b\"]",
                "  n0 ==>|\"present\"| n1",
            ]
        );
    }

    #[test]
    fn test_node_ran_twice_loop_yields_one_node_line() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("try", NodeKind::Acp),
                make_node("check", NodeKind::Compute),
            ],
            vec![
                FlowEdge::Simple {
                    from: "try".to_string(),
                    to: "check".to_string(),
                },
                FlowEdge::Switch {
                    from: "check".to_string(),
                    on: "ok".to_string(),
                    cases: vec![("retry".to_string(), "try".to_string())],
                },
            ],
            vec![
                make_step("try", 1, NodeKind::Acp, Outcome::Failed),
                make_step("check", 1, NodeKind::Compute, Outcome::Ok),
                make_step("try", 2, NodeKind::Acp, Outcome::Ok),
                make_step("check", 2, NodeKind::Compute, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight, false);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0(\"✓ try\")",
                "  n1[\"✓ check\"]",
                "  n0 ==> n1",
                "  n1 ==>|\"retry\"| n0",
            ]
        );
    }
}

````

### `crates/codegen/xai-grok-pager/src/acpx/mod.rs` (complete file, repository grok-build-acpx)
````
//! acpx flow-run bundles, sessions, graph source and launching.

pub mod bundle;
pub mod graph;
pub mod launch;
pub mod sessions;

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` (complete file, repository grok-build-acpx)
````
//! Screen for one ACPX flow run: header, status banner, flow graph, and a
//! split below with the foldable stage tree on the left and the reserved
//! detail panel `Rect` on the right.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::acpx::bundle::{
    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, TraceEvent,
    fmt_duration, value_text,
};
use crate::acpx::graph::{Direction, mermaid_source};
use crate::render::line_utils::truncate_str;
use crate::scrollback::blocks::markdown_content::MarkdownContent;
use crate::theme::Theme;
use crate::views::acpx_modal::detail::{DetailTarget, value_inline_summary};
use crate::views::traces_fold::{FoldNode, FoldState, FoldTree};
use crate::views::traces_modal::fmt_age;

/// Outcome of handling a key in the run screen.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    Unchanged,
    Changed,
    Back,
    OpenStep(usize),
    Replay(usize),
    Copy(String),
    ScrollDetail(isize),
    Gate { choice: String, flow_file: String },
}

#[derive(Debug, Clone)]
enum NodePayload {
    StepStage {
        step_idx: usize,
        stage_num: usize,
        state: NodeState,
    },
    LiveStage {
        node_id: String,
        stage_num: usize,
        state: NodeState,
    },
    SkippedHeader,
    SkippedNode {
        node_id: String,
        kind: NodeKind,
    },
    Event {
        owner: StageOwner,
        rel_ms: i64,
        kind: String,
        detail: String,
    },
    Result {
        step_idx: usize,
    },
    GateChoice {
        owner: StageOwner,
        choice_idx: usize,
        label: String,
        flow_file: String,
    },
    GateEmpty {
        owner: StageOwner,
    },
}

#[derive(Debug, Clone)]
enum StageOwner {
    Step(usize),
    Live(String),
}

#[derive(Debug, Clone)]
struct GraphCache {
    width: u16,
    lr_src: String,
    td_src: String,
    lr_lines: Vec<Line<'static>>,
    td_lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone)]
struct GateConfirm {
    choice: String,
    flow_file: String,
}

/// Interactive state for the single-run screen.
pub struct RunViewState {
    run: Run,
    node_settings: HashMap<String, AgentSettings>,
    step_settings: HashMap<String, AgentSettings>,
    tree: FoldTree<String>,
    payloads: Vec<NodePayload>,
    fold: FoldState<String>,
    /// Index into `visible` (the output of `tree.visible(&fold)`).
    selected: usize,
    scroll: usize,
    last_pane_height: usize,
    show_graph: bool,
    show_noise: bool,
    detail_rect: Option<Rect>,
    gate_confirm: Option<GateConfirm>,
    graph_cache: Option<GraphCache>,
    answered: Option<String>,
}

impl RunViewState {
    pub fn new(run: Run) -> Self {
        let mut st = Self {
            run,
            node_settings: HashMap::new(),
            step_settings: HashMap::new(),
            tree: FoldTree { nodes: Vec::new() },
            payloads: Vec::new(),
            fold: FoldState::new(),
            selected: 0,
            scroll: 0,
            last_pane_height: 20,
            show_graph: true,
            show_noise: false,
            detail_rect: None,
            gate_confirm: None,
            graph_cache: None,
            answered: None,
        };
        st.rebuild_tree();
        st.select_initial_stage();
        st
    }

    pub fn set_settings(
        &mut self,
        nodes: HashMap<String, AgentSettings>,
        steps: HashMap<String, AgentSettings>,
    ) {
        self.node_settings = nodes;
        self.step_settings = steps;
    }

    pub fn set_answered(&mut self, choice: Option<String>) {
        // A confirm prompt opened before the answer arrived must not stay live.
        if choice.is_some() {
            self.gate_confirm = None;
        }
        let changed = self.answered != choice;
        self.answered = choice;
        if changed {
            // Stage glyphs and the graph are drawn from the answer.
            self.sync();
        }
    }

    pub fn answered(&self) -> Option<&str> {
        self.answered.as_deref()
    }

    pub fn detail_target(&self) -> DetailTarget {
        let vis = self.visible();
        let Some(&node_idx) = vis.get(self.selected) else {
            return DetailTarget::Run;
        };
        let Some(payload) = self.payloads.get(node_idx) else {
            return DetailTarget::Run;
        };
        match payload {
            NodePayload::StepStage { step_idx, .. } | NodePayload::Result { step_idx } => {
                DetailTarget::Step(*step_idx)
            }
            NodePayload::LiveStage { node_id, .. } | NodePayload::SkippedNode { node_id, .. } => {
                DetailTarget::Node(node_id.clone())
            }
            NodePayload::Event { owner, .. }
            | NodePayload::GateChoice { owner, .. }
            | NodePayload::GateEmpty { owner } => match owner {
                StageOwner::Step(idx) => DetailTarget::Step(*idx),
                StageOwner::Live(id) => DetailTarget::Node(id.clone()),
            },
            NodePayload::SkippedHeader => DetailTarget::Run,
        }
    }

    pub fn detail_area(&self) -> Option<Rect> {
        self.detail_rect
    }

    pub fn captures_keys(&self) -> bool {
        self.gate_confirm.is_some()
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn run_mut(&mut self) -> &mut Run {
        &mut self.run
    }

    pub fn sync(&mut self) {
        let vis = self.visible();
        let prev_key = vis
            .get(self.selected)
            .and_then(|&i| self.tree.nodes.get(i))
            .map(|n| n.key.clone());

        // Find the nearest earlier root key as fallback.
        let prev_root_keys: Vec<String> = vis
            .iter()
            .take(self.selected + 1)
            .filter_map(|&i| {
                let n = self.tree.nodes.get(i)?;
                (n.depth == 0).then(|| n.key.clone())
            })
            .collect();

        self.rebuild_tree();
        self.fold.retain(&self.tree);
        self.graph_cache = None;

        let new_vis = self.visible();
        if new_vis.is_empty() {
            self.selected = 0;
            return;
        }

        if let Some(ref key) = prev_key
            && let Some(pos) = new_vis
                .iter()
                .position(|&i| self.tree.nodes.get(i).is_some_and(|n| &n.key == key))
        {
            self.selected = pos;
            return;
        }

        for rkey in prev_root_keys.iter().rev() {
            if let Some(pos) = new_vis
                .iter()
                .position(|&i| self.tree.nodes.get(i).is_some_and(|n| &n.key == rkey))
            {
                self.selected = pos;
                return;
            }
        }

        self.selected = self.selected.min(new_vis.len().saturating_sub(1));
    }

    pub fn selected_step(&self) -> Option<usize> {
        match self.detail_target() {
            DetailTarget::Step(idx) => Some(idx),
            DetailTarget::Run | DetailTarget::Node(_) => None,
        }
    }

    pub fn select_step(&mut self, index: usize) {
        let Some(step) = self.run.steps.get(index) else {
            return;
        };
        let target_key = step.attempt_id.clone();
        let vis = self.visible();
        if let Some(pos) = vis
            .iter()
            .position(|&i| self.tree.nodes.get(i).is_some_and(|n| n.key == target_key))
        {
            self.selected = pos;
        }
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut out = vec!["Enter step", "r replay"];
        if self.run.summary.status == RunStatus::Waiting
            && self.answered.is_none()
            && !bundle::gate_choices(&self.run).is_empty()
        {
            out.push("1-9 gate");
        }
        out.extend([
            "Space fold",
            "J/K detail",
            "v graph",
            "n noise",
            "y copy",
            "Esc back",
            "? keys",
        ]);
        out
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        let vis = self.visible();
        if vis.is_empty() || delta == 0 {
            return false;
        }
        let max_idx = vis.len().saturating_sub(1);
        let next = if delta > 0 {
            self.selected.saturating_add(delta as usize).min(max_idx)
        } else {
            self.selected.saturating_sub(delta.unsigned_abs())
        };
        if next != self.selected {
            self.selected = next;
            true
        } else {
            false
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> RunOutcome {
        if key.kind == KeyEventKind::Release {
            return RunOutcome::Unchanged;
        }

        if let Some(confirm) = self.gate_confirm.clone() {
            return match key.code {
                KeyCode::Enter => {
                    self.gate_confirm = None;
                    RunOutcome::Gate {
                        choice: confirm.choice,
                        flow_file: confirm.flow_file,
                    }
                }
                KeyCode::Esc => {
                    self.gate_confirm = None;
                    RunOutcome::Changed
                }
                _ => RunOutcome::Unchanged,
            };
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => RunOutcome::Back,
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.scroll(1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.scroll(-1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.selected != 0 {
                    self.selected = 0;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                let vis = self.visible();
                let last = vis.len().saturating_sub(1);
                if self.selected != last {
                    self.selected = last;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                if let Some(parent_idx) = self.fold.collapse(&self.tree, node_idx) {
                    let new_vis = self.visible();
                    if let Some(pos) = new_vis.iter().position(|&i| i == parent_idx) {
                        self.selected = pos;
                    }
                } else {
                    self.clamp_selected();
                }
                RunOutcome::Changed
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                self.fold.expand(&self.tree, node_idx);
                RunOutcome::Changed
            }
            KeyCode::Char(' ') => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                self.fold.toggle(&self.tree, node_idx);
                self.clamp_selected();
                RunOutcome::Changed
            }
            KeyCode::Char('E') => {
                let vis = self.visible();
                let cur_node = vis.get(self.selected).copied();
                let cur_root = cur_node.map(|i| self.root_of(i));
                self.fold.toggle_all(&self.tree);
                let new_vis = self.visible();
                if let Some(n) = cur_node
                    && let Some(pos) = new_vis.iter().position(|&i| i == n)
                {
                    self.selected = pos;
                } else if let Some(r) = cur_root
                    && let Some(pos) = new_vis.iter().position(|&i| i == r)
                {
                    self.selected = pos;
                } else {
                    self.clamp_selected();
                }
                RunOutcome::Changed
            }
            KeyCode::Char('n') => {
                self.show_noise = !self.show_noise;
                self.sync();
                RunOutcome::Changed
            }
            KeyCode::Char('v') => {
                self.show_graph = !self.show_graph;
                RunOutcome::Changed
            }
            KeyCode::Char('!') => {
                if self.jump_next_failed() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('c') => {
                if self.jump_current_or_waiting() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('[') => {
                if self.jump_stage(-1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(']') => {
                if self.jump_stage(1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('J') => RunOutcome::ScrollDetail(3),
            KeyCode::Char('K') => RunOutcome::ScrollDetail(-3),
            KeyCode::Char('D') => RunOutcome::ScrollDetail(15),
            KeyCode::Char('U') => RunOutcome::ScrollDetail(-15),
            KeyCode::F(5) => {
                let vis = self.visible();
                if let Some(pos) = vis.iter().position(|&i| {
                    matches!(
                        self.payloads.get(i),
                        Some(NodePayload::GateChoice { .. } | NodePayload::GateEmpty { .. })
                    )
                }) {
                    self.selected = pos;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(ch @ '1'..='9') => {
                if self.run.summary.status != RunStatus::Waiting || self.answered.is_some() {
                    return RunOutcome::Unchanged;
                }
                let idx = (ch as usize) - ('1' as usize);
                let choices = bundle::gate_choices(&self.run);
                if let Some((label, flow_file)) = choices.get(idx) {
                    self.gate_confirm = Some(GateConfirm {
                        choice: label.clone(),
                        flow_file: flow_file.clone(),
                    });
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Enter => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                if let Some(NodePayload::GateChoice {
                    label, flow_file, ..
                }) = self.payloads.get(node_idx)
                {
                    if self.answered.is_some() {
                        return RunOutcome::Unchanged;
                    }
                    self.gate_confirm = Some(GateConfirm {
                        choice: label.clone(),
                        flow_file: flow_file.clone(),
                    });
                    return RunOutcome::Changed;
                }
                if let Some(step_idx) = self.selected_step() {
                    RunOutcome::OpenStep(step_idx)
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('r') => {
                if let Some(step_idx) = self.selected_step()
                    && let Some(step) = self.run.steps.get(step_idx)
                    && step.conversation.is_some()
                {
                    RunOutcome::Replay(step_idx)
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('y') => {
                if let Some(step_idx) = self.selected_step()
                    && let Some(step) = self.run.steps.get(step_idx)
                {
                    RunOutcome::Copy(value_text(&step.output))
                } else {
                    RunOutcome::Unchanged
                }
            }
            _ => RunOutcome::Unchanged,
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        self.detail_rect = None;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut y = area.y;
        let max_y = area.y.saturating_add(area.height);

        // 1. Header row
        if y < max_y {
            self.render_header(buf, Rect::new(area.x, y, area.width, 1), theme, now_ms);
            y = y.saturating_add(1);
        }

        // 2. Banner row (when needed)
        if y < max_y {
            let banner_lines = self.build_banner(area.width as usize, theme, now_ms);
            let max_banner = (max_y.saturating_sub(y) as usize).saturating_sub(1);
            for banner_line in banner_lines.into_iter().take(max_banner) {
                buf.set_line(area.x, y, &banner_line, area.width);
                y = y.saturating_add(1);
            }
        }

        // 3. Flow graph (when toggled on and area height >= 16)
        if self.show_graph && area.height >= 16 && y < max_y {
            let max_graph_rows = (area.height as usize / 3).max(1);
            // Leave at least 2 rows for separator + stages pane
            let avail_for_graph = (max_y.saturating_sub(y) as usize).saturating_sub(2);
            let budget = max_graph_rows.min(avail_for_graph);
            if budget > 0 {
                let graph_lines = self.prepare_graph_lines(area.width, budget, theme);
                for gline in graph_lines {
                    if y >= max_y {
                        break;
                    }
                    buf.set_line(area.x, y, &gline, area.width);
                    y = y.saturating_add(1);
                }
            }
        }

        // 4. Separator row
        if y < max_y {
            let rule: String = "─".repeat(area.width as usize);
            buf.set_stringn(
                area.x,
                y,
                &rule,
                area.width as usize,
                Style::default().fg(theme.gray_dim),
            );
            y = y.saturating_add(1);
        }

        if y >= max_y {
            return;
        }

        // 5. Split pane below
        let body_h = max_y.saturating_sub(y);
        let stages_rect = if area.width >= 110 {
            let left_w = ((area.width as u32 * 2 / 5) as u16).clamp(46, 76);
            let sep_x = area.x.saturating_add(left_w);
            let detail_x = sep_x.saturating_add(1);
            let detail_w = area.x.saturating_add(area.width).saturating_sub(detail_x);
            let sep_style = Style::default().fg(theme.gray_dim);
            for row_y in y..max_y {
                buf.set_stringn(sep_x, row_y, "│", 1, sep_style);
            }
            self.detail_rect = Some(Rect::new(detail_x, y, detail_w, body_h));
            Rect::new(area.x, y, left_w, body_h)
        } else {
            self.detail_rect = None;
            Rect::new(area.x, y, area.width, body_h)
        };

        self.render_stages_pane(buf, stages_rect, theme, now_ms);

        if self.gate_confirm.is_some() {
            self.render_gate_confirm(buf, stages_rect, theme);
        }
    }

    // ---- Internal tree construction ----

    fn rebuild_tree(&mut self) {
        let mut nodes = Vec::new();
        let mut payloads = Vec::new();

        let is_waiting = self.run.summary.status == RunStatus::Waiting;
        let is_running = self.run.summary.status == RunStatus::Running;
        let waiting_node = self.run.summary.waiting_on.as_deref();
        let current_node = self.run.summary.current_node.as_deref();

        // Find if the waiting node already has a step in `run.steps` (pick the last one).
        let waiting_step_idx: Option<usize> = if is_waiting {
            waiting_node.and_then(|wn| self.run.steps.iter().rposition(|s| s.node_id == wn))
        } else {
            None
        };

        // Determine if we need a live/<node> root:
        // - Running node that has no step record, or is currently active without a finished step for this run
        //   Spec: "a root for a node that is Running but has no step record (key = `"live/<node id>"`).
        //   The WAITING stage is the last root whose node id equals `run.summary.waiting_on`: acpx records
        //   the checkpoint step BEFORE the run goes waiting, so it is normally a step root; use a `live/…`
        //   root for it only when no such step exists"
        let live_root_node: Option<(String, NodeState)> = if is_waiting {
            if waiting_step_idx.is_none() {
                waiting_node.map(|wn| (wn.to_string(), self.run.node_state(wn)))
            } else {
                None
            }
        } else if is_running {
            current_node.and_then(|cn| {
                let has_step = self.run.steps.iter().any(|s| s.node_id == cn);
                if !has_step {
                    Some((cn.to_string(), self.run.node_state(cn)))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let mut stage_num = 0usize;

        // a. One root per step in `run.steps` order
        for (step_idx, step) in self.run.steps.iter().enumerate() {
            stage_num += 1;
            let is_waiting_stage = waiting_step_idx == Some(step_idx);
            let state = if is_waiting_stage && self.answered.is_none() {
                NodeState::Waiting
            } else if is_waiting_stage {
                NodeState::Done(Outcome::Ok)
            } else {
                NodeState::Done(step.outcome)
            };

            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: step.attempt_id.clone(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::StepStage {
                step_idx,
                stage_num,
                state,
            });

            // Trace event children
            let evs = self.run.step_events(&step.attempt_id);
            let start_ms = if step.started_ms > 0 {
                step.started_ms
            } else {
                evs.first().map_or(0, |e| e.at_ms)
            };
            for ev in evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("{}/{}", step.attempt_id, ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Step(step_idx),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            // Result row
            nodes.push(FoldNode {
                key: format!("{}/result", step.attempt_id),
                parent: Some(root_idx),
                depth: 1,
            });
            payloads.push(NodePayload::Result { step_idx });

            // Gate children if this step is the waiting stage
            if is_waiting_stage {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Step(step_idx),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // b. Live root (Running with no step record, or Waiting with no step record)
        if let Some((live_node_id, state)) = live_root_node {
            stage_num += 1;
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: format!("live/{live_node_id}"),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::LiveStage {
                node_id: live_node_id.clone(),
                stage_num,
                state: if state == NodeState::Waiting && self.answered.is_some() {
                    NodeState::Done(Outcome::Ok)
                } else {
                    state
                },
            });

            let step_attempts: HashSet<&str> = self
                .run
                .steps
                .iter()
                .map(|s| s.attempt_id.as_str())
                .collect();
            let live_evs: Vec<&TraceEvent> = self
                .run
                .events
                .iter()
                .filter(|ev| {
                    ev.node_id.as_deref() == Some(live_node_id.as_str())
                        && ev
                            .attempt_id
                            .as_deref()
                            .is_none_or(|a| !step_attempts.contains(a))
                })
                .collect();
            let start_ms = live_evs
                .iter()
                .find(|e| e.kind == "node_started")
                .or_else(|| live_evs.first())
                .map_or(self.run.summary.started_ms, |e| e.at_ms);

            for ev in live_evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("live/{live_node_id}/{}", ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Live(live_node_id.clone()),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            if state == NodeState::Waiting {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Live(live_node_id),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // c. Skipped / NotRun flow nodes
        let skipped: Vec<(String, NodeKind)> = self
            .run
            .flow
            .nodes
            .iter()
            .filter(|n| self.run.node_state(&n.id) == NodeState::NotRun)
            .map(|n| (n.id.clone(), n.kind))
            .collect();
        if !skipped.is_empty() {
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: "skipped".to_string(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::SkippedHeader);
            for (nid, kind) in skipped {
                nodes.push(FoldNode {
                    key: format!("skip/{nid}"),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::SkippedNode { node_id: nid, kind });
            }
        }

        self.tree = FoldTree { nodes };
        self.payloads = payloads;
    }

    fn push_gate_children(
        run: &Run,
        root_idx: usize,
        owner: StageOwner,
        nodes: &mut Vec<FoldNode<String>>,
        payloads: &mut Vec<NodePayload>,
    ) {
        let choices = bundle::gate_choices(run);
        if choices.is_empty() {
            nodes.push(FoldNode {
                key: "gate/empty".to_string(),
                parent: Some(root_idx),
                depth: 1,
            });
            payloads.push(NodePayload::GateEmpty { owner });
        } else {
            for (idx, (label, flow_file)) in choices.into_iter().enumerate() {
                let choice_idx = idx + 1;
                nodes.push(FoldNode {
                    key: format!("gate/{choice_idx}"),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::GateChoice {
                    owner: owner.clone(),
                    choice_idx,
                    label,
                    flow_file,
                });
            }
        }
    }

    fn visible(&self) -> Vec<usize> {
        self.tree.visible(&self.fold)
    }

    fn clamp_selected(&mut self) {
        let vis = self.visible();
        if vis.is_empty() {
            self.selected = 0;
        } else if self.selected >= vis.len() {
            self.selected = vis.len() - 1;
        }
    }

    fn root_of(&self, mut idx: usize) -> usize {
        while let Some(node) = self.tree.nodes.get(idx) {
            if let Some(p) = node.parent {
                idx = p;
            } else {
                break;
            }
        }
        idx
    }

    fn select_initial_stage(&mut self) {
        if self.jump_current_or_waiting() {
            return;
        }
        // Fallback: last stage that ran (a + b roots)
        let vis = self.visible();
        if let Some(pos) = vis.iter().rposition(|&i| {
            matches!(
                self.payloads.get(i),
                Some(NodePayload::StepStage { .. } | NodePayload::LiveStage { .. })
            )
        }) {
            self.selected = pos;
        } else {
            self.selected = 0;
        }
    }

    fn jump_current_or_waiting(&mut self) -> bool {
        let vis = self.visible();
        // Waiting stage first
        if self.run.summary.status == RunStatus::Waiting {
            if let Some(pos) = vis.iter().rposition(|&i| {
                matches!(
                    self.payloads.get(i),
                    Some(
                        NodePayload::StepStage {
                            state: NodeState::Waiting,
                            ..
                        } | NodePayload::LiveStage {
                            state: NodeState::Waiting,
                            ..
                        }
                    )
                )
            }) {
                self.selected = pos;
                return true;
            }
        }
        // Running stage
        if let Some(pos) = vis.iter().rposition(|&i| {
            matches!(
                self.payloads.get(i),
                Some(
                    NodePayload::StepStage {
                        state: NodeState::Running,
                        ..
                    } | NodePayload::LiveStage {
                        state: NodeState::Running,
                        ..
                    }
                )
            )
        }) {
            self.selected = pos;
            return true;
        }
        // Otherwise match current_node or waiting_on against the last matching stage root
        let target_node = self
            .run
            .summary
            .waiting_on
            .as_deref()
            .or(self.run.summary.current_node.as_deref());
        if let Some(tn) = target_node
            && let Some(pos) = vis.iter().rposition(|&i| match self.payloads.get(i) {
                Some(NodePayload::StepStage { step_idx, .. }) => self
                    .run
                    .steps
                    .get(*step_idx)
                    .is_some_and(|s| s.node_id == tn),
                Some(NodePayload::LiveStage { node_id, .. }) => node_id == tn,
                _ => false,
            })
        {
            self.selected = pos;
            return true;
        }
        // Else last stage that ran
        if let Some(pos) = vis.iter().rposition(|&i| {
            matches!(
                self.payloads.get(i),
                Some(NodePayload::StepStage { .. } | NodePayload::LiveStage { .. })
            )
        }) {
            self.selected = pos;
            return true;
        }
        false
    }

    fn jump_next_failed(&mut self) -> bool {
        let vis = self.visible();
        let failed_positions: Vec<usize> = vis
            .iter()
            .enumerate()
            .filter_map(|(pos, &i)| match self.payloads.get(i) {
                Some(NodePayload::StepStage { step_idx, .. }) => {
                    let s = self.run.steps.get(*step_idx)?;
                    matches!(
                        s.outcome,
                        Outcome::Failed | Outcome::TimedOut | Outcome::Cancelled
                    )
                    .then_some(pos)
                }
                _ => None,
            })
            .collect();
        if failed_positions.is_empty() {
            return false;
        }
        let next = failed_positions
            .iter()
            .copied()
            .find(|&p| p > self.selected)
            .unwrap_or(failed_positions[0]);
        self.selected = next;
        true
    }

    fn jump_stage(&mut self, dir: isize) -> bool {
        let vis = self.visible();
        let stage_positions: Vec<usize> = vis
            .iter()
            .enumerate()
            .filter_map(|(pos, &i)| {
                matches!(
                    self.payloads.get(i),
                    Some(
                        NodePayload::StepStage { .. }
                            | NodePayload::LiveStage { .. }
                            | NodePayload::SkippedHeader
                    )
                )
                .then_some(pos)
            })
            .collect();
        if stage_positions.is_empty() {
            return false;
        }
        let target = if dir > 0 {
            stage_positions.iter().copied().find(|&p| p > self.selected)
        } else {
            stage_positions
                .iter()
                .rev()
                .copied()
                .find(|&p| p < self.selected)
        };
        if let Some(pos) = target
            && pos != self.selected
        {
            self.selected = pos;
            true
        } else {
            false
        }
    }

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
        }

        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    }

    fn build_banner(&self, width: usize, theme: &Theme, now_ms: i64) -> Vec<Line<'static>> {
        let sum = &self.run.summary;
        let (text, style) = if bundle::is_stalled(sum, now_ms) {
            let node = sum.current_node.as_deref().unwrap_or("?");
            (
                format!("⊘ stalled at {node}: the acpx process is gone"),
                Style::default().fg(theme.gray).add_modifier(Modifier::BOLD),
            )
        } else {
            match sum.status {
                RunStatus::Waiting => {
                    let node = sum
                        .waiting_on
                        .as_deref()
                        .or(sum.current_node.as_deref())
                        .unwrap_or("?");
                    if let Some(ref choice) = self.answered {
                        let msg = if choice.is_empty() {
                            format!("✓ answered at {node}")
                        } else {
                            format!("✓ answered at {node}: {choice}")
                        };
                        (msg, Style::default().fg(theme.gray))
                    } else {
                        // The question, when there is one; a detail that only repeats
                        // the node name is no question.
                        let msg = match sum.status_detail.as_deref() {
                            Some(q) if !q.is_empty() && q != node => {
                                format!("◐ NEEDS YOU at {node}: {q}")
                            }
                            _ => format!("◐ NEEDS YOU at {node}"),
                        };
                        (
                            msg,
                            Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
                        )
                    }
                }
                RunStatus::Failed | RunStatus::TimedOut => {
                    let err = sum
                        .error
                        .as_deref()
                        .or(sum.status_detail.as_deref())
                        .unwrap_or(sum.status.label());
                    (
                        format!("✗ {err}"),
                        Style::default()
                            .fg(theme.accent_error)
                            .add_modifier(Modifier::BOLD),
                    )
                }
                RunStatus::Running => {
                    let node = sum.current_node.as_deref().unwrap_or("?");
                    let msg = match sum.status_detail.as_deref() {
                        Some(d) if !d.is_empty() => format!("● {node} · {d}"),
                        _ => format!("● {node}"),
                    };
                    (
                        msg,
                        Style::default()
                            .fg(theme.accent_running)
                            .add_modifier(Modifier::BOLD),
                    )
                }
                RunStatus::Completed | RunStatus::Unknown => return Vec::new(),
            }
        };

        wrap_banner(&text, width)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, style)))
            .collect()
    }

    fn prepare_graph_lines(
        &mut self,
        width: u16,
        max_rows: usize,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let w = width as usize;
        let lr_src = mermaid_source(&self.run, Direction::LeftRight, self.answered.is_some());
        let td_src = mermaid_source(&self.run, Direction::TopDown, self.answered.is_some());

        let cache_valid = self.graph_cache.as_ref().is_some_and(|c| {
            c.width == width && c.lr_src == lr_src && c.td_src == td_src
        });

        if !cache_valid {
            let lr_lines = render_mermaid_lines(&lr_src, w);
            let td_lines = render_mermaid_lines(&td_src, w);
            self.graph_cache = Some(GraphCache {
                width,
                lr_src,
                td_src,
                lr_lines,
                td_lines,
            });
        }

        let Some(cache) = self.graph_cache.as_ref() else {
            return Vec::new();
        };

        let lr_bad = is_source_listing(&cache.lr_lines)
            || cache.lr_lines.len() > max_rows
            || any_line_wider(&cache.lr_lines, w);

        if !lr_bad {
            return cache.lr_lines.clone();
        }

        if is_source_listing(&cache.td_lines) || any_line_wider(&cache.td_lines, w) {
            return vec![Line::from(Span::styled(
                truncate_str("graph does not fit at this width (v hides this line)", w),
                Style::default().fg(theme.gray_dim),
            ))];
        }

        if cache.td_lines.len() <= max_rows {
            return cache.td_lines.clone();
        }

        let mut out: Vec<Line<'static>> = cache
            .td_lines
            .iter()
            .take(max_rows.saturating_sub(1))
            .cloned()
            .collect();
        out.push(Line::from(Span::styled(
            truncate_str("… graph cut (v hides it)", w),
            Style::default().fg(theme.gray_dim),
        )));
        out
    }

    fn render_stages_pane(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let h = area.height as usize;
        let w = area.width as usize;
        self.last_pane_height = h;

        let vis = self.visible();
        if vis.is_empty() {
            let msg = truncate_str("no stages recorded", w);
            let msg_w = UnicodeWidthStr::width(msg.as_str()) as u16;
            let x = area.x + area.width.saturating_sub(msg_w) / 2;
            let y = area.y + area.height / 2;
            buf.set_stringn(x, y, &msg, w, Style::default().fg(theme.gray_dim));
            return;
        }

        self.clamp_selected();

        // Count root stage transitions to see if blank separator rows fit when pane is taller than content.
        let num_roots = vis
            .iter()
            .filter(|&&i| self.tree.nodes.get(i).is_some_and(|n| n.depth == 0))
            .count();
        let blank_gaps = num_roots.saturating_sub(1);
        let use_blank_separators = vis.len() + blank_gaps <= h;

        if use_blank_separators {
            self.scroll = 0;
        } else {
            if self.selected < self.scroll {
                self.scroll = self.selected;
            } else if self.selected >= self.scroll + h {
                self.scroll = self.selected + 1 - h;
            }
            let max_scroll = vis.len().saturating_sub(h);
            if self.scroll > max_scroll {
                self.scroll = max_scroll;
            }
        }

        let mut row_y = area.y;
        let max_y = area.y.saturating_add(area.height);

        for (vis_pos, &node_idx) in vis.iter().enumerate().skip(self.scroll) {
            if row_y >= max_y {
                break;
            }
            let is_root = self
                .tree
                .nodes
                .get(node_idx)
                .is_some_and(|n| n.depth == 0);
            if use_blank_separators && is_root && vis_pos > 0 {
                row_y = row_y.saturating_add(1);
                if row_y >= max_y {
                    break;
                }
            }

            let is_selected = vis_pos == self.selected;
            let is_last_child = !is_root && self.is_last_visible_child(&vis, vis_pos, node_idx);

            let row_rect = Rect::new(area.x, row_y, area.width, 1);
            if is_selected {
                buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
            }

            let line = self.render_tree_row(
                node_idx,
                is_selected,
                is_last_child,
                w,
                theme,
                now_ms,
            );
            buf.set_line(area.x, row_y, &line, area.width);
            row_y = row_y.saturating_add(1);
        }

        // Right-aligned position counter when rows overflow
        if vis.len() > h {
            let counter = format!(" {}/{} ", self.selected + 1, vis.len());
            let cw = UnicodeWidthStr::width(counter.as_str()) as u16;
            if cw <= area.width {
                let cx = area.x + area.width - cw;
                buf.set_stringn(
                    cx,
                    area.y,
                    &counter,
                    cw as usize,
                    Style::default().fg(theme.gray_dim),
                );
            }
        }
    }

    fn is_last_visible_child(&self, vis: &[usize], vis_pos: usize, node_idx: usize) -> bool {
        let Some(parent) = self.tree.nodes.get(node_idx).and_then(|n| n.parent) else {
            return false;
        };
        match vis.get(vis_pos + 1) {
            Some(&next_idx) => self
                .tree
                .nodes
                .get(next_idx)
                .is_none_or(|n| n.parent != Some(parent)),
            None => true,
        }
    }

    fn render_tree_row(
        &self,
        node_idx: usize,
        selected: bool,
        is_last_child: bool,
        width: usize,
        theme: &Theme,
        now_ms: i64,
    ) -> Line<'static> {
        let Some(node) = self.tree.nodes.get(node_idx) else {
            return Line::default();
        };
        let Some(payload) = self.payloads.get(node_idx) else {
            return Line::default();
        };

        let bg = selected.then_some(theme.bg_highlight);
        let style_with = |fg: Color, bold: bool| {
            let mut s = Style::default().fg(fg);
            if let Some(b) = bg {
                s = s.bg(b);
            }
            if bold || selected {
                s = s.add_modifier(Modifier::BOLD);
            }
            s
        };

        match payload {
            NodePayload::StepStage {
                step_idx,
                stage_num,
                state,
            } => {
                let Some(step) = self.run.steps.get(*step_idx) else {
                    return Line::default();
                };
                let arrow = self.fold_arrow(node_idx, &node.key);
                let label = self.stage_agent_or_kind_label(Some(step), &step.node_id, step.kind);
                let dur = fmt_duration(step.duration_ms());
                self.format_stage_header_line(
                    arrow,
                    *stage_num,
                    *state,
                    &step.node_id,
                    &label,
                    &dur,
                    selected,
                    width,
                    theme,
                )
            }
            NodePayload::LiveStage {
                node_id,
                stage_num,
                state,
            } => {
                let arrow = self.fold_arrow(node_idx, &node.key);
                let kind = self
                    .run
                    .flow
                    .nodes
                    .iter()
                    .find(|n| &n.id == node_id)
                    .map_or(NodeKind::Unknown, |n| n.kind);
                let label = self.stage_agent_or_kind_label(None, node_id, kind);
                let dur_ms = self.live_stage_duration_ms(node_id, now_ms);
                let dur = fmt_duration(dur_ms);
                self.format_stage_header_line(
                    arrow,
                    *stage_num,
                    *state,
                    node_id,
                    &label,
                    &dur,
                    selected,
                    width,
                    theme,
                )
            }
            NodePayload::SkippedHeader => {
                let arrow = self.fold_arrow(node_idx, &node.key);
                let text = truncate_str(&format!("{arrow} ── not run ──"), width);
                Line::from(Span::styled(text, style_with(theme.gray_dim, false)))
            }
            NodePayload::SkippedNode { node_id, kind } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let rest = format!("○ {node_id}  {}", kind.label());
                let avail = width.saturating_sub(4);
                let cut = truncate_str(&rest, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.gray_dim, false)),
                ])
            }
            NodePayload::Event {
                rel_ms,
                kind,
                detail,
                ..
            } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let body = if detail.is_empty() {
                    format!("+{}  {}", fmt_duration(*rel_ms as u64), kind)
                } else {
                    format!("+{}  {}  {}", fmt_duration(*rel_ms as u64), kind, detail)
                };
                let avail = width.saturating_sub(4);
                let cut = truncate_str(&body, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.gray, false)),
                ])
            }
            NodePayload::Result { step_idx } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let avail = width.saturating_sub(4);
                let (text, fg) = self
                    .run
                    .steps
                    .get(*step_idx)
                    .map_or_else(|| (String::new(), theme.gray), |s| step_result_summary(s, theme));
                let cut = truncate_str(&text, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(fg, false)),
                ])
            }
            NodePayload::GateChoice {
                choice_idx,
                label,
                flow_file,
                ..
            } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let avail = width.saturating_sub(4);
                // An answered gate has nothing left to pick: no key numbers, the picked one is ticked.
                let body = match self.answered.as_deref() {
                    Some(picked) if picked.eq_ignore_ascii_case(label) => {
                        format!("✓ {label} → {flow_file}")
                    }
                    Some(_) => format!("  {label} → {flow_file}"),
                    None => format!("[{choice_idx}] {label} → {flow_file}"),
                };
                let cut = truncate_str(&body, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.warning, true)),
                ])
            }
            NodePayload::GateEmpty { .. } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let avail = width.saturating_sub(4);
                let cut = truncate_str(
                    "no next step declared",
                    avail,
                );
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.gray_dim, false)),
                ])
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn format_stage_header_line(
        &self,
        arrow: &'static str,
        stage_num: usize,
        state: NodeState,
        node_id: &str,
        agent_label: &str,
        dur: &str,
        selected: bool,
        width: usize,
        theme: &Theme,
    ) -> Line<'static> {
        let bg = selected.then_some(theme.bg_highlight);
        let mk_style = |fg: Color, bold: bool| {
            let mut s = Style::default().fg(fg);
            if let Some(b) = bg {
                s = s.bg(b);
            }
            if bold || selected {
                s = s.add_modifier(Modifier::BOLD);
            }
            s
        };

        let glyph = state.glyph();
        let glyph_col = node_state_color(state, theme);

        let dur_part = format!(" {dur}");
        let dur_w = UnicodeWidthStr::width(dur_part.as_str());
        let left_budget = width.saturating_sub(dur_w);

        let prefix = format!("{arrow} {stage_num} ");
        let glyph_part = format!("{glyph} ");
        let id_part = node_id.to_string();
        let label_part = if agent_label.is_empty() {
            String::new()
        } else {
            format!("  {agent_label}")
        };

        let p_w = UnicodeWidthStr::width(prefix.as_str());
        let g_w = UnicodeWidthStr::width(glyph_part.as_str());
        let id_budget = left_budget.saturating_sub(p_w + g_w);
        let id_cut = truncate_str(&id_part, id_budget);
        let id_w = UnicodeWidthStr::width(id_cut.as_str());

        let lbl_budget = left_budget.saturating_sub(p_w + g_w + id_w);
        let lbl_cut = truncate_str(&label_part, lbl_budget);
        let lbl_w = UnicodeWidthStr::width(lbl_cut.as_str());

        let used_left = p_w + g_w + id_w + lbl_w;
        let pad = width.saturating_sub(used_left + dur_w);

        let mut spans = vec![
            Span::styled(prefix, mk_style(theme.text_primary, true)),
            Span::styled(glyph_part, mk_style(glyph_col, true)),
            Span::styled(id_cut, mk_style(theme.text_primary, true)),
        ];
        if !lbl_cut.is_empty() {
            spans.push(Span::styled(lbl_cut, mk_style(theme.gray, false)));
        }
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), mk_style(theme.gray, false)));
        }
        if dur_w <= width {
            spans.push(Span::styled(dur_part, mk_style(theme.gray, false)));
        }
        Line::from(spans)
    }

    fn fold_arrow(&self, node_idx: usize, key: &String) -> &'static str {
        if !self.tree.has_children(node_idx) {
            " "
        } else if self.fold.is_collapsed(key) {
            "▸"
        } else {
            "▾"
        }
    }

    fn stage_agent_or_kind_label(
        &self,
        step: Option<&Step>,
        node_id: &str,
        kind: NodeKind,
    ) -> String {
        if let Some(s) = step
            && let Some(st) = self.step_settings.get(&s.attempt_id)
        {
            let lbl = st.label();
            if !lbl.is_empty() {
                return lbl;
            }
        }
        if let Some(st) = self.node_settings.get(node_id) {
            let lbl = st.label();
            if !lbl.is_empty() {
                return lbl;
            }
        }
        if let Some(s) = step
            && let Some(ref agent_name) = s.agent_name
            && !agent_name.is_empty()
        {
            return agent_name.clone();
        }
        let is_shell_node = self
            .run
            .flow
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .is_some_and(|n| n.is_shell);
        let is_shell_step = step
            .and_then(|s| s.action.as_ref())
            .is_some_and(|a| a.action_type == "shell" || a.command.is_some());
        if kind == NodeKind::Action && (is_shell_node || is_shell_step) {
            return "shell".to_string();
        }
        kind.label().to_string()
    }

    fn live_stage_duration_ms(&self, node_id: &str, now_ms: i64) -> u64 {
        let step_attempts: HashSet<&str> = self
            .run
            .steps
            .iter()
            .map(|s| s.attempt_id.as_str())
            .collect();
        let started = self
            .run
            .events
            .iter()
            .rev()
            .find(|ev| {
                ev.node_id.as_deref() == Some(node_id)
                    && ev.kind == "node_started"
                    && ev
                        .attempt_id
                        .as_deref()
                        .is_none_or(|a| !step_attempts.contains(a))
            })
            .map_or(self.run.summary.started_ms, |ev| ev.at_ms);
        if started > 0 && now_ms > started {
            (now_ms - started) as u64
        } else {
            0
        }
    }

    fn render_gate_confirm(&self, buf: &mut Buffer, pane: Rect, theme: &Theme) {
        let Some(ref confirm) = self.gate_confirm else {
            return;
        };
        if pane.width < 12 || pane.height < 5 {
            return;
        }
        let line1 = format!("Start {}?", confirm.flow_file);
        let line2 = format!("choice \"{}\" · runs with approve-all", confirm.choice);
        let line3 = "Enter start · Esc cancel";

        let content_w = [
            UnicodeWidthStr::width(line1.as_str()),
            UnicodeWidthStr::width(line2.as_str()),
            UnicodeWidthStr::width(line3),
        ]
        .into_iter()
        .max()
        .unwrap_or(24);
        let box_w = ((content_w + 4) as u16).min(pane.width);
        let box_h = 5u16.min(pane.height);
        let bx = pane.x + (pane.width.saturating_sub(box_w)) / 2;
        let by = pane.y + (pane.height.saturating_sub(box_h)) / 2;
        let box_rect = Rect::new(bx, by, box_w, box_h);

        let bg_style = Style::default().bg(theme.bg_dark).fg(theme.text_primary);
        for ry in box_rect.y..box_rect.y + box_rect.height {
            for rx in box_rect.x..box_rect.x + box_rect.width {
                buf.set_stringn(rx, ry, " ", 1, bg_style);
            }
        }

        let border_style = Style::default().fg(theme.warning).bg(theme.bg_dark);
        let inner_w = box_w.saturating_sub(2) as usize;
        let top = format!("┌{}┐", "─".repeat(inner_w));
        let bot = format!("└{}┘", "─".repeat(inner_w));
        buf.set_stringn(bx, by, &top, box_w as usize, border_style);
        buf.set_stringn(
            bx,
            by + box_h - 1,
            &bot,
            box_w as usize,
            border_style,
        );
        for ry in (by + 1)..(by + box_h - 1) {
            buf.set_stringn(bx, ry, "│", 1, border_style);
            buf.set_stringn(bx + box_w - 1, ry, "│", 1, border_style);
        }

        let text_w = inner_w.saturating_sub(2);
        let tx = bx.saturating_add(2);
        if box_h >= 3 {
            buf.set_stringn(
                tx,
                by + 1,
                &truncate_str(&line1, text_w),
                text_w,
                Style::default()
                    .fg(theme.text_primary)
                    .bg(theme.bg_dark)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if box_h >= 4 {
            buf.set_stringn(
                tx,
                by + 2,
                &truncate_str(&line2, text_w),
                text_w,
                Style::default().fg(theme.gray).bg(theme.bg_dark),
            );
        }
        if box_h >= 5 {
            buf.set_stringn(
                tx,
                by + 3,
                &truncate_str(line3, text_w),
                text_w,
                Style::default().fg(theme.warning).bg(theme.bg_dark),
            );
        }
    }
}

fn status_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.accent_success,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray,
    }
}

fn node_state_color(state: NodeState, theme: &Theme) -> Color {
    match state {
        NodeState::NotRun => theme.gray_dim,
        NodeState::Running => theme.accent_running,
        NodeState::Waiting => theme.warning,
        NodeState::Done(Outcome::Ok) => theme.accent_success,
        NodeState::Done(Outcome::Failed | Outcome::TimedOut) => theme.accent_error,
        NodeState::Done(Outcome::Cancelled | Outcome::Unknown) => theme.gray,
    }
}

fn step_result_summary(step: &Step, theme: &Theme) -> (String, Color) {
    if matches!(
        step.outcome,
        Outcome::Failed | Outcome::TimedOut | Outcome::Cancelled
    ) {
        let msg = step
            .error
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(match step.outcome {
                Outcome::TimedOut => "timed out",
                Outcome::Cancelled => "cancelled",
                _ => "failed",
            });
        return (first_line(msg), theme.accent_error);
    }
    if let Some( ref err) = step.error
        && !err.is_empty()
    {
        return (first_line(err), theme.accent_error);
    }
    if let Some(route) = step
        .output
        .as_object()
        .and_then(|o| o.get("route"))
        .and_then(Value::as_str)
    {
        return (format!("→ {route}"), theme.text_secondary);
    }
    if let Some(ref action) = step.action
        && (action.action_type == "shell" || action.command.is_some())
        && let Some(code) = action.exit_code
    {
        return (format!("exit {code}"), theme.gray);
    }
    let text = match &step.output {
        Value::Null => step
            .raw_text
            .as_deref()
            .map(first_line)
            .unwrap_or_else(|| "ok".to_string()),
        Value::String(s) => first_line(s),
        other => match value_inline_summary(other) {
            s if s.is_empty() => "ok".to_string(),
            s => s,
        },
    };
    (text, theme.gray)
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn wrap_banner(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    for word in text.split(' ') {
        let word_w = UnicodeWidthStr::width(word);
        if cur.is_empty() {
            if word_w <= width {
                cur.push_str(word);
                cur_w = word_w;
            } else {
                for ch in word.chars() {
                    let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if cur_w + ch_w > width && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += ch_w;
                }
            }
        } else if cur_w + 1 + word_w <= width {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + word_w;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
            if word_w <= width {
                cur.push_str(word);
                cur_w = word_w;
            } else {
                for ch in word.chars() {
                    let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if cur_w + ch_w > width && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += ch_w;
                }
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn render_mermaid_lines(src: &str, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let md = format!("```mermaid\n{src}\n```");
    let out = MarkdownContent::new_with_table_width(md, Some(width)).output(width);
    out.lines.into_iter().map(|bl| bl.content).collect()
}

fn line_plain_text(line: &Line<'_>) -> String {
    let mut s = String::new();
    for span in &line.spans {
        s.push_str(span.content.as_ref());
    }
    s
}

fn is_source_listing(lines: &[Line<'static>]) -> bool {
    lines.iter().any(|l| {
        let t = line_plain_text(l);
        t.contains("mermaid: flowchart") || t.contains("too wide to display")
    })
}

fn any_line_wider(lines: &[Line<'static>], width: usize) -> bool {
    lines
        .iter()
        .any(|l| UnicodeWidthStr::width(line_plain_text(l).as_str()) > width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        ActionReceipt, Conversation, FlowDef, FlowEdge, FlowNode, RunSummary,
    };
    use serde_json::json;
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn buf_text(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    fn make_step(
        attempt_id: &str,
        node_id: &str,
        kind: NodeKind,
        outcome: Outcome,
        output: Value,
    ) -> Step {
        Step {
            attempt_id: attempt_id.to_string(),
            node_id: node_id.to_string(),
            kind,
            outcome,
            started_ms: 1_000,
            finished_ms: 2_500,
            prompt_text: None,
            raw_text: None,
            output,
            error: (outcome == Outcome::Failed).then(|| "boom: command failed".to_string()),
            agent_name: (kind == NodeKind::Acp).then(|| "grok-build".to_string()),
            action: (kind == NodeKind::Action).then(|| ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("cargo test".to_string()),
                args: vec![],
                cwd: None,
                exit_code: Some(0),
                duration_ms: Some(1500),
            }),
            conversation: (kind == NodeKind::Acp).then(|| Conversation {
                session_id: "sess-1".to_string(),
                event_start_seq: 1,
                event_end_seq: 5,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn sample_waiting_run(gate_output: Value) -> Run {
        let flow = FlowDef {
            name: "review-flow".to_string(),
            start_at: "plan".to_string(),
            nodes: vec![
                FlowNode {
                    id: "plan".to_string(),
                    kind: NodeKind::Acp,
                    summary: None,
                    is_shell: false,
                },
                FlowNode {
                    id: "test".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                },
                FlowNode {
                    id: "gate".to_string(),
                    kind: NodeKind::Checkpoint,
                    summary: None,
                    is_shell: false,
                },
                FlowNode {
                    id: "deploy".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                },
            ],
            edges: vec![
                FlowEdge::Simple {
                    from: "plan".to_string(),
                    to: "test".to_string(),
                },
                FlowEdge::Simple {
                    from: "test".to_string(),
                    to: "gate".to_string(),
                },
                FlowEdge::Simple {
                    from: "gate".to_string(),
                    to: "deploy".to_string(),
                },
            ],
        };

        let steps = vec![
            make_step(
                "plan#1",
                "plan",
                NodeKind::Acp,
                Outcome::Ok,
                json!({"route": "ok"}),
            ),
            make_step(
                "test#1",
                "test",
                NodeKind::Action,
                Outcome::Failed,
                Value::Null,
            ),
            make_step(
                "gate#1",
                "gate",
                NodeKind::Checkpoint,
                Outcome::Ok,
                gate_output,
            ),
        ];

        let events = vec![
            TraceEvent {
                seq: 1,
                at_ms: 1_000,
                scope: "node".to_string(),
                kind: "node_started".to_string(),
                node_id: Some("plan".to_string()),
                attempt_id: Some("plan#1".to_string()),
                detail: "starting plan".to_string(),
            },
            TraceEvent {
                seq: 2,
                at_ms: 1_500,
                scope: "node".to_string(),
                kind: "node_heartbeat".to_string(),
                node_id: Some("plan".to_string()),
                attempt_id: Some("plan#1".to_string()),
                detail: "alive".to_string(),
            },
            TraceEvent {
                seq: 3,
                at_ms: 2_500,
                scope: "node".to_string(),
                kind: "node_completed".to_string(),
                node_id: Some("plan".to_string()),
                attempt_id: Some("plan#1".to_string()),
                detail: "ok".to_string(),
            },
        ];

        Run {
            summary: RunSummary {
                run_id: "run-001".to_string(),
                dir: PathBuf::from("/tmp/run-001"),
                flow_name: "review-flow".to_string(),
                title: "PR #42 Review".to_string(),
                status: RunStatus::Waiting,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: Some("gate".to_string()),
                waiting_on: Some("gate".to_string()),
                status_detail: Some("awaiting human approval".to_string()),
                error: None,
            },
            flow,
            steps,
            events,
            input: json!({}),
            sessions: vec![],
        }
    }

    #[test]
    fn initial_selection_starts_on_waiting_stage() {
        let run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
        let st = RunViewState::new(run);
        assert_eq!(st.selected_step(), Some(2));
        assert_eq!(st.detail_target(), DetailTarget::Step(2));
    }

    #[test]
    fn stages_start_expanded_and_hide_heartbeats_until_noise_toggled() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let keys_before: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_before.contains(&"plan#1/1".to_string()));
        assert!(!keys_before.contains(&"plan#1/2".to_string()));
        assert!(keys_before.contains(&"plan#1/3".to_string()));

        assert_eq!(st.handle_key(&key(KeyCode::Char('n'))), RunOutcome::Changed);
        let keys_after: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_after.contains(&"plan#1/2".to_string()));
    }

    #[test]
    fn collapse_on_child_jumps_to_parent() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0);
        assert_eq!(st.selected, 0);
        // Move down onto the first child of plan#1
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.selected, 1);
        assert_eq!(st.detail_target(), DetailTarget::Step(0));
        // Left on a leaf child jumps back to parent root (index 0)
        st.handle_key(&key(KeyCode::Left));
        assert_eq!(st.selected, 0);
        // Left again collapses the root
        st.handle_key(&key(KeyCode::Char('h')));
        assert!(st.fold.is_collapsed(&"plan#1".to_string()));
    }

    #[test]
    fn shift_e_toggles_all_folds() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let expanded_len = st.visible().len();
        st.handle_key(&key(KeyCode::Char('E')));
        let collapsed_len = st.visible().len();
        assert!(collapsed_len < expanded_len);
        // Only root headers remain visible (3 steps + 1 skipped root = 4)
        assert_eq!(collapsed_len, 4);
        st.handle_key(&key(KeyCode::Char('E')));
        assert_eq!(st.visible().len(), expanded_len);
    }

    #[test]
    fn bang_jumps_to_failed_step_and_wraps() {
        let mut run = sample_waiting_run(json!({}));
        run.steps[0].outcome = Outcome::Failed;
        run.steps[1].outcome = Outcome::Failed;
        let mut st = RunViewState::new(run);
        // Starts on step 2 (gate#1)
        assert_eq!(st.selected_step(), Some(2));
        // '!' wraps to step 0
        assert_eq!(st.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(st.selected_step(), Some(0));
        // '!' advances to step 1
        assert_eq!(st.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(st.selected_step(), Some(1));
        // '!' wraps back to step 0
        assert_eq!(st.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(st.selected_step(), Some(0));
    }

    #[test]
    fn c_and_brackets_navigate_stage_headers() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.handle_key(&key(KeyCode::Home));
        assert_eq!(st.selected_step(), Some(0));

        st.handle_key(&key(KeyCode::Char(']')));
        assert_eq!(st.selected_step(), Some(1));

        st.handle_key(&key(KeyCode::Char('[')));
        assert_eq!(st.selected_step(), Some(0));

        st.handle_key(&key(KeyCode::Char('c')));
        assert_eq!(st.selected_step(), Some(2));
    }

    #[test]
    fn enter_and_r_outcomes_for_acp_and_non_acp_steps() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0); // ACP step with conversation
        assert_eq!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::OpenStep(0)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('r'))),
            RunOutcome::Replay(0)
        );

        st.select_step(1); // Action step without conversation
        assert_eq!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::OpenStep(1)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('r'))),
            RunOutcome::Unchanged
        );
    }

    #[test]
    fn y_esc_and_detail_scroll_outcomes() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0);
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('y'))),
            RunOutcome::Copy("{\n  \"route\": \"ok\"\n}".to_string())
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('J'))),
            RunOutcome::ScrollDetail(3)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('K'))),
            RunOutcome::ScrollDetail(-3)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('D'))),
            RunOutcome::ScrollDetail(15)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('U'))),
            RunOutcome::ScrollDetail(-15)
        );
        assert_eq!(st.handle_key(&key(KeyCode::Esc)), RunOutcome::Back);
    }

    #[test]
    fn detail_target_for_step_child_live_root_and_skipped_node() {
        let mut run = sample_waiting_run(json!({}));
        run.summary.status = RunStatus::Running;
        run.summary.waiting_on = None;
        run.summary.current_node = Some("deploy".to_string());
        run.events.push(TraceEvent {
            seq: 99,
            at_ms: 4_000,
            scope: "node".to_string(),
            kind: "node_started".to_string(),
            node_id: Some("deploy".to_string()),
            attempt_id: Some("deploy#1".to_string()),
            detail: "deploying".to_string(),
        });
        // Add an extra unreached node so skipped section also exists
        run.flow.nodes.push(FlowNode {
            id: "cleanup".to_string(),
            kind: NodeKind::Action,
            summary: None,
            is_shell: true,
        });

        let mut st = RunViewState::new(run);
        // Initial selection on running live/deploy stage
        assert_eq!(st.detail_target(), DetailTarget::Node("deploy".to_string()));
        // Move to child event of live/deploy
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.detail_target(), DetailTarget::Node("deploy".to_string()));
        // Move to skipped header
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.detail_target(), DetailTarget::Run);
        // Move to skip/cleanup row
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(
            st.detail_target(),
            DetailTarget::Node("cleanup".to_string())
        );
    }

    #[test]
    fn gate_choices_confirmation_flow() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        let vis_keys: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(vis_keys.contains(&"gate/1".to_string()));
        assert!(vis_keys.contains(&"gate/2".to_string()));
        assert!(!st.captures_keys());

        // Press '2' to open confirmation for "reject"
        assert_eq!(st.handle_key(&key(KeyCode::Char('2'))), RunOutcome::Changed);
        assert!(st.captures_keys());

        // Esc closes confirmation box
        assert_eq!(st.handle_key(&key(KeyCode::Esc)), RunOutcome::Changed);
        assert!(!st.captures_keys());

        // Press '2' then Enter confirms the gate
        assert_eq!(st.handle_key(&key(KeyCode::Char('2'))), RunOutcome::Changed);
        assert!(st.captures_keys());
        assert_eq!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::Gate {
                choice: "reject".to_string(),
                flow_file: "fix.flow.ts".to_string(),
            }
        );
        assert!(!st.captures_keys());
    }

    #[test]
    fn waiting_run_without_next_shows_empty_gate_row() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 35));
        let area_ = buf.area;
        st.render(&mut buf, area_, &theme, 10_000);
        let text = buf_text(&buf);
        assert!(text.contains("no next step declared"));
    }

    #[test]
    fn sync_keeps_selection_when_step_is_appended() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(1);
        assert_eq!(st.selected_step(), Some(1));

        st.run_mut().steps.push(make_step(
            "deploy#1",
            "deploy",
            NodeKind::Action,
            Outcome::Ok,
            Value::Null,
        ));
        st.sync();
        assert_eq!(st.selected_step(), Some(1));
    }

    #[test]
    fn banner_shows_the_gate_question_and_falls_back_to_the_node_name() {
        let theme = Theme::default();
        let banner = |detail: Option<&str>| {
            let mut run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
            run.summary.status_detail = detail.map(str::to_string);
            let st = RunViewState::new(run);
            st.build_banner(120, &theme, 10_000)
                .iter()
                .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
                .collect::<String>()
        };
        assert!(banner(Some("land it, or send it back?"))
            .contains("◐ NEEDS YOU at gate: land it, or send it back?"));
        // acpx reports the node id when the checkpoint has no question: no repeat.
        let repeated = banner(Some("gate"));
        assert!(repeated.contains("◐ NEEDS YOU at gate"));
        assert!(!repeated.contains("gate: gate"));
        let none = banner(None);
        assert!(none.contains("◐ NEEDS YOU at gate"));
        assert!(!none.contains("waiting"));
    }

    #[test]
    fn render_wide_160x40_shows_banner_stages_separator_and_clean_detail_area() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 160, 40);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);

        let text = buf_text(&buf);
        assert!(text.contains("PR #42 Review"));
        assert!(text.contains("◐ NEEDS YOU at gate: awaiting human approval"));
        assert!(text.contains("✓ plan"));
        assert!(text.contains("✗ test"));
        assert!(text.contains("◐ gate"));
        assert!(text.contains("│"));
        // Skipped section lists NotRun node 'deploy'
        assert!(text.contains("── not run ──"));
        assert!(text.contains("○ deploy"));

        let detail = st.detail_area();
        assert!(detail.is_some());
        let drect = detail.unwrap_or_default();
        assert!(drect.x > 46);
        for y in drect.y..drect.y + drect.height {
            for x in drect.x..drect.x + drect.width {
                assert_eq!(buf[(x, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn step_tree_result_line_is_plain_text_not_raw_json() {
        let mut run = sample_waiting_run(json!({}));
        run.steps[0].output = json!({"verdict": "approve", "findings": []});
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);

        let text = buf_text(&buf);
        let line = text
            .lines()
            .find(|l| l.contains("verdict approve"))
            .unwrap_or_else(|| panic!("no result line in:\n{text}"));
        assert!(line.contains("verdict approve, findings (empty)"), "{line}");
        assert!(!line.contains('{') && !line.contains('"'), "{line}");
    }

    #[test]
    fn step_result_summary_keeps_route_exit_and_error_forms() {
        let theme = Theme::default();
        let routed = make_step("a#1", "a", NodeKind::Acp, Outcome::Ok, json!({"route": "pass"}));
        assert_eq!(step_result_summary(&routed, &theme).0, "→ pass");
        let shell = make_step("b#1", "b", NodeKind::Action, Outcome::Ok, Value::Null);
        assert_eq!(step_result_summary(&shell, &theme).0, "exit 0");
        let failed = make_step("c#1", "c", NodeKind::Action, Outcome::Failed, Value::Null);
        assert_eq!(step_result_summary(&failed, &theme).0, "boom: command failed");
    }

    #[test]
    fn render_narrow_100x30_has_no_detail_area() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
        assert_eq!(st.detail_area(), None);
    }

    #[test]
    fn render_with_v_off_hides_graph_rows() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 140, 35);

        let mut buf_on = Buffer::empty(area);
        st.render(&mut buf_on, area, &theme, 10_000);
        let d_on = st.detail_area().unwrap_or_default();

        st.handle_key(&key(KeyCode::Char('v')));
        let mut buf_off = Buffer::empty(area);
        st.render(&mut buf_off, area, &theme, 10_000);
        let d_off = st.detail_area().unwrap_or_default();

        // With graph off, the separator + split panes start right after header + banner (row 3)
        assert_eq!(d_off.y, 3);
        assert!(d_on.y > d_off.y);
    }

    #[test]
    fn render_tiny_40x8_and_0x0_does_not_panic() {
        let run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();

        let mut buf_zero = Buffer::empty(Rect::new(0, 0, 0, 0));
        st.render(&mut buf_zero, Rect::new(0, 0, 0, 0), &theme, 10_000);

        let small = Rect::new(0, 0, 40, 8);
        let mut buf_small = Buffer::empty(small);
        st.render(&mut buf_small, small, &theme, 10_000);

        // Also open the confirmation box and render at small & zero sizes
        st.handle_key(&key(KeyCode::Char('1')));
        st.render(&mut buf_small, small, &theme, 10_000);
        st.render(&mut buf_zero, Rect::new(0, 0, 0, 0), &theme, 10_000);
    }

    #[test]
    fn waiting_long_question_wraps_at_80_columns_and_shows_every_word() {
        let mut run = sample_waiting_run(json!({}));
        let question = "Should we proceed with deploying the database migration to production now or wait until the maintenance window tonight?";
        assert_eq!(question.len(), 119);
        let question = format!("{question}!");
        assert_eq!(question.len(), 120);
        run.summary.status_detail = Some(question.clone());
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
        let text = buf_text(&buf);
        for word in question.split_whitespace() {
            assert!(text.contains(word), "missing word {word:?} in:\n{text}");
        }
    }

    #[test]
    fn waiting_short_question_takes_one_row() {
        let mut run = sample_waiting_run(json!({}));
        run.summary.status_detail = Some("ready?".to_string());
        let mut st = RunViewState::new(run);
        st.handle_key(&key(KeyCode::Char('v')));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
        assert_eq!(st.detail_area().map(|r| r.y), Some(3));
    }

    #[test]
    fn answered_waiting_run_updates_banner_choices_shortcuts_and_keys() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        assert!(st.shortcuts().contains(&"1-9 gate"));
        assert_eq!(st.handle_key(&key(KeyCode::Char('1'))), RunOutcome::Changed);
        assert_eq!(st.handle_key(&key(KeyCode::Esc)), RunOutcome::Changed);

        st.set_answered(Some("approve".to_string()));
        assert_eq!(st.answered(), Some("approve"));
        assert!(!st.shortcuts().contains(&"1-9 gate"));
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('1'))),
            RunOutcome::Unchanged
        );

        let theme = Theme::default();
        let area = Rect::new(0, 0, 120, 35);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
        let text = buf_text(&buf);
        assert!(text.contains("answered at"));
        assert!(text.contains("approve"));
        assert!(!text.contains("NEEDS YOU"));
        assert!(text.contains("✓ approve → land.flow.ts"));
        assert!(text.contains("reject → fix.flow.ts"));
    }

    #[test]
    fn step_tree_gate_rows_drop_the_key_numbers_once_answered() {
        let run = sample_waiting_run(json!({
            "next": {"approve": "land.flow.ts", "reject": "fix.flow.ts"}
        }));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        // Wide enough for the detail pane to take the right side; look at the left (tree) half only.
        let area = Rect::new(0, 0, 160, 40);
        let tree_text = |st: &mut RunViewState| {
            let mut buf = Buffer::empty(area);
            st.render(&mut buf, area, &theme, 10_000);
            let tree_w = st.detail_area().map_or(area.width, |d| d.x);
            (0..area.height)
                .map(|y| {
                    (0..tree_w)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let open = tree_text(&mut st);
        assert!(open.contains("[1] approve → land.flow.ts"), "{open}");
        assert!(open.contains("[2] reject → fix.flow.ts"), "{open}");

        st.set_answered(Some("approve".to_string()));
        let done = tree_text(&mut st);
        assert!(done.contains("✓ approve → land.flow.ts"), "{done}");
        assert!(done.contains("  reject → fix.flow.ts"), "{done}");
        assert!(!done.contains("[1]") && !done.contains("[2]"), "{done}");
    }

    #[test]
    fn answered_gate_is_drawn_done_in_graph_stages_and_detail_pane() {
        let run = sample_waiting_run(json!({
            "next": {"approve": "land.flow.ts", "reject": "fix.flow.ts"}
        }));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 160, 40);
        let draw = |st: &mut RunViewState| {
            let mut buf = Buffer::empty(area);
            st.render(&mut buf, area, &theme, 10_000);
            buf_text(&buf)
        };
        assert!(draw(&mut st).contains("◐ gate"));

        st.set_answered(Some("approve".to_string()));
        let text = draw(&mut st);
        assert!(text.contains("✓ gate"), "{text}");
        assert!(!text.contains('◐'), "{text}");
    }

    #[test]
    fn answer_arriving_while_confirm_is_open_closes_the_confirm() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        assert_eq!(st.handle_key(&key(KeyCode::Char('1'))), RunOutcome::Changed);
        assert!(st.gate_confirm.is_some());

        st.set_answered(Some("approve".to_string()));
        assert!(st.gate_confirm.is_none());
        assert!(!matches!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::Gate { .. }
        ));
    }
}

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` (complete file, repository grok-build-acpx)
````
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::acpx::bundle::{
    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

/// What the right-hand panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum DetailTarget {
    Run,          // overview of the whole run
    Step(usize),  // index into run.steps: one finished attempt
    Node(String), // a node id with no step record: running, or not run (rarely: waiting without a step)
}

#[derive(Debug, Clone, PartialEq)]
struct CacheKey {
    run_dir: PathBuf,
    target: DetailTarget,
    width: u16,
    updated_ms: i64,
    steps_len: usize,
    events_len: usize,
    events_file_len: Option<u64>,
    answered: Option<String>,
}

pub struct DetailPanel {
    cache_key: Option<CacheKey>,
    /// The picked choice of an answered gate; `Some("")` when it was answered
    /// but the choice is unknown. `None` while the gate is open.
    answered: Option<String>,
    last_target: Option<(PathBuf, DetailTarget)>,
    lines: Vec<Line<'static>>,
    scroll: usize,
    tail_follow: bool,
    last_height: usize,
}

impl Default for DetailPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailPanel {
    pub fn new() -> Self {
        Self {
            cache_key: None,
            answered: None,
            last_target: None,
            lines: Vec::new(),
            scroll: 0,
            tail_follow: false,
            last_height: 0,
        }
    }

    /// Tell the panel whether the run's gate has been answered, and with what.
    pub fn set_answered(&mut self, answered: Option<&str>) {
        if self.answered.as_deref() != answered {
            self.answered = answered.map(str::to_string);
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        let max_scroll = self.max_scroll(self.last_height);
        let cur = self.scroll.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_scroll)
        };
        if delta < 0 && next < cur {
            self.tail_follow = false;
        }
        if next >= max_scroll && delta > 0 {
            self.tail_follow = true;
        }
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn max_scroll(&self, height: usize) -> usize {
        if height == 0 {
            self.lines.len().saturating_sub(1)
        } else {
            self.lines.len().saturating_sub(height)
        }
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        run: &Run,
        target: &DetailTarget,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width < 10 || area.height == 0 {
            return;
        }
        self.last_height = area.height as usize;

        let target_pair = (run.summary.dir.clone(), target.clone());
        let target_changed = self.last_target.as_ref() != Some(&target_pair);
        if target_changed {
            self.last_target = Some(target_pair);
            let is_running_node = matches!(
                target,
                DetailTarget::Node(id) if run.node_state(id) == NodeState::Running
            );
            self.tail_follow = is_running_node;
            self.scroll = 0;
        }

        let events_file_len = conversation_file_len(run, target);
        let key = CacheKey {
            run_dir: run.summary.dir.clone(),
            target: target.clone(),
            width: area.width,
            updated_ms: run.summary.updated_ms,
            steps_len: run.steps.len(),
            events_len: run.events.len(),
            events_file_len,
            answered: self.answered.clone(),
        };

        if self.cache_key.as_ref() != Some(&key) {
            self.lines = build_lines(
                run,
                target,
                self.answered.as_deref(),
                (area.width as usize).saturating_sub(2),
                theme,
                now_ms,
            );
            self.cache_key = Some(key);
        }

        let max_scroll = self.max_scroll(area.height as usize);
        if self.tail_follow {
            self.scroll = max_scroll;
        } else if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let height = area.height as usize;
        let total = self.lines.len();
        let needs_scrollbar = total > height;
        let text_width = area.width;

        for row in 0..height {
            let line_idx = self.scroll + row;
            if let Some(line) = self.lines.get(line_idx) {
                buf.set_line(area.x, area.y + row as u16, line, text_width);
            }
        }

        if needs_scrollbar && area.width >= 1 {
            let sb_x = area.x + area.width - 1;
            let thumb_h = ((height * height) / total).max(1).min(height);
            let max_top = height.saturating_sub(thumb_h);
            let thumb_top = if max_scroll == 0 {
                0
            } else {
                (self.scroll * max_top) / max_scroll
            };
            let sb_style = Style::default().fg(theme.gray_dim);
            for r in 0..thumb_h {
                let y = area.y + (thumb_top + r) as u16;
                if y < area.y + area.height {
                    buf.set_stringn(sb_x, y, "█", 1, sb_style);
                }
            }
        }
    }
}

fn conversation_file_len(run: &Run, target: &DetailTarget) -> Option<u64> {
    match target {
        DetailTarget::Step(i) => {
            let step = run.steps.get(*i)?;
            let conv = step.conversation.as_ref()?;
            let path = run.session_events_path(&conv.session_id)?;
            std::fs::metadata(path).ok().map(|m| m.len())
        }
        DetailTarget::Node(id) => {
            if run.node_state(id) != NodeState::Running {
                return None;
            }
            let settings = bundle::node_settings(&run.summary.dir);
            let handle = settings
                .get(id)
                .and_then(|s| s.handle.as_deref())
                .unwrap_or("main");
            let session = run.sessions.iter().find(|s| s.handle == handle)?;
            std::fs::metadata(&session.events_path).ok().map(|m| m.len())
        }
        DetailTarget::Run => None,
    }
}

fn build_lines(
    run: &Run,
    target: &DetailTarget,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    if width < 10 {
        return Vec::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines(run, *i, step, answered, width, theme)
            } else {
                vec![dim_line("step not found", theme)]
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}

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
        Span::styled(
            head.strip_prefix(&format!("{glyph} ")).unwrap_or(&head).to_string(),
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Facts row
    let (done, total) = run.progress();
    let dur = fmt_duration(run.duration_ms(now_ms));
    let mut facts = Vec::new();
    if !run.summary.flow_name.is_empty() {
        facts.push(run.summary.flow_name.clone());
    }
    facts.push(format!("{done}/{total} nodes"));
    facts.push(dur);
    if !status.is_live() {
        let ref_ms = run.summary.finished_ms.unwrap_or(run.summary.updated_ms);
        let ago = now_ms.saturating_sub(ref_ms).max(0);
        facts.push(fmt_age(ago));
    }
    let stalled = bundle::is_stalled(&run.summary, now_ms);
    let facts_text = facts.join(" · ");
    let mut fact_spans = vec![Span::styled(
        truncate_str(&facts_text, width),
        Style::default().fg(theme.gray),
    )];
    if stalled {
        let used = UnicodeWidthStr::width(facts_text.as_str());
        if used + 10 <= width {
            fact_spans.push(Span::styled(" · ", Style::default().fg(theme.gray_dim)));
            fact_spans.push(Span::styled(
                "stalled",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    out.push(Line::from(fact_spans));

    if status == RunStatus::Waiting {
        if let Some(detail) = &run.summary.status_detail {
            if !detail.is_empty() {
                let color = if answered.is_some() { theme.gray } else { theme.warning };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    if matches!(status, RunStatus::Failed | RunStatus::TimedOut) {
        if let Some(err) = &run.summary.error {
            if !err.is_empty() {
                push_wrapped(
                    &mut out,
                    err,
                    width,
                    Style::default().fg(theme.accent_error),
                );
            }
        }
    }

    // Stages section
    if !run.flow.nodes.is_empty() {
        push_section_rule(&mut out, "Stages", width, theme);
        let node_settings = bundle::node_settings(&run.summary.dir);
        let step_settings = bundle::step_settings(&run.summary.dir);
        let max_id_w = run
            .flow
            .nodes
            .iter()
            .map(|n| UnicodeWidthStr::width(n.id.as_str()))
            .max()
            .unwrap_or(4);

        for node in &run.flow.nodes {
            let state = run.node_state_answered(&node.id, answered.is_some());
            let s_glyph = state.glyph();
            let s_color = node_state_color(state, theme);
            let last_step = run.steps.iter().rev().find(|s| s.node_id == node.id);

            let agent_or_kind = last_step
                .and_then(|s| step_settings.get(&s.attempt_id))
                .map(AgentSettings::label)
                .filter(|l| !l.is_empty())
                .or_else(|| {
                    node_settings
                        .get(&node.id)
                        .map(AgentSettings::label)
                        .filter(|l| !l.is_empty())
                })
                .unwrap_or_else(|| node.kind.label().to_string());

            let dur_str = last_step
                .map(|s| fmt_duration(s.duration_ms()))
                .unwrap_or_default();

            let one_line = last_step.map(step_one_line_summary).unwrap_or_default();

            let id_w = UnicodeWidthStr::width(node.id.as_str());
            let pad = " ".repeat(max_id_w.saturating_sub(id_w) + 2);

            let mut spans = Vec::new();
            spans.push(Span::styled(
                format!("{s_glyph} "),
                Style::default().fg(s_color),
            ));
            spans.push(Span::styled(
                format!("{}{pad}", node.id),
                Style::default().fg(theme.text_primary),
            ));
            spans.push(Span::styled(
                agent_or_kind.clone(),
                Style::default().fg(theme.gray),
            ));
            if !dur_str.is_empty() {
                spans.push(Span::styled(
                    format!("  {dur_str}"),
                    Style::default().fg(theme.gray_dim),
                ));
            }
            let prefix_w: usize = spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if !one_line.is_empty() && prefix_w + 3 < width {
                let rem = width - prefix_w - 2;
                let cut = truncate_str(&one_line, rem);
                let res_color = if last_step.and_then(|s| s.error.as_ref()).is_some() {
                    theme.accent_error
                } else {
                    theme.text_secondary
                };
                spans.push(Span::styled("  ", Style::default()));
                spans.push(Span::styled(cut, Style::default().fg(res_color)));
            }
            out.push(truncate_line(Line::from(spans), width));
        }
    }

    // Last result
    if let Some(last_step) = run.steps.last() {
        let lines = format_value_lines(&last_step.output, width, theme);
        if !lines.is_empty() {
            push_section_rule(&mut out, "Last result", width, theme);
            out.extend(lines);
        }
    }

    // Input
    let input_lines = format_value_lines(&run.input, width, theme);
    if !input_lines.is_empty() {
        push_section_rule(&mut out, "Input", width, theme);
        out.extend(
            input_lines,
        );
    }

    out
}

fn step_one_line_summary(step: &Step) -> String {
    if let Some(err) = &step.error {
        if !err.is_empty() {
            return first_non_empty_line(err);
        }
    }
    if let Some(route) = step
        .output
        .as_object()
        .and_then(|o| o.get("route"))
        .and_then(Value::as_str)
    {
        return format!("→ {route}");
    }
    if let Some(act) = &step.action {
        if let Some(code) = act.exit_code {
            return format!("exit {code}");
        }
    }
    match &step.output {
        Value::Null => String::new(),
        Value::String(s) => first_non_empty_line(s),
        other => value_inline_summary(other),
    }
}

/// What an empty list or object shows as a field value (drawn in the same colour as the keys beside it).
const EMPTY: &str = "(empty)";

fn is_empty_container(val: &Value) -> bool {
    match val {
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

pub(super) fn value_inline_summary(val: &Value) -> String {
    match val {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => first_non_empty_line(s),
        Value::Array(arr) => arr
            .iter()
            .map(value_inline_summary)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let vs = if is_empty_container(v) {
                    EMPTY.to_string()
                } else {
                    value_inline_summary(v)
                };
                if vs.is_empty() {
                    k.clone()
                } else {
                    format!("{k} {vs}")
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The key/value rows as plain text, for views that hold text rather than styled lines (the step inspector).
/// Same layout as the rows: one `key  value` line per field, nested values indented, lists as `- `, `(empty)` for
/// empty lists and objects, no braces or quotes. Lines are never wrapped here; the viewer wraps.
pub(super) fn value_plain_text(val: &Value) -> String {
    if is_empty_container(val) {
        return EMPTY.to_string();
    }
    let theme = Theme::current();
    format_value_lines(val, usize::MAX / 4, &theme)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if matches!(val, Value::Null) {
        return out;
    }
    if let Value::String(s) = val {
        if s.trim().is_empty() {
            return out;
        }
    }
    append_value_lines(&mut out, val, 0, width, theme);
    out
}

fn append_value_lines(
    out: &mut Vec<Line<'static>>,
    val: &Value,
    indent: usize,
    width: usize,
    theme: &Theme,
) {
    let val_style = Style::default().fg(theme.text_primary);
    let key_style = Style::default().fg(theme.gray);
    let pad_indent = " ".repeat(indent);
    let body_w = width.saturating_sub(indent).max(1);

    match val {
        Value::Null => {
            if indent > 0 {
                out.push(Line::from(vec![
                    Span::raw(pad_indent),
                    Span::styled("null", val_style),
                ]));
            }
        }
        Value::Bool(b) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(b.to_string(), val_style),
            ]));
        }
        Value::Number(n) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(n.to_string(), val_style),
            ]));
        }
        Value::String(s) => {
            for line in wrap(s, body_w) {
                if indent == 0 {
                    out.push(Line::from(Span::styled(line, val_style)));
                } else {
                    out.push(Line::from(vec![
                        Span::raw(pad_indent.clone()),
                        Span::styled(line, val_style),
                    ]));
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                let bullet_prefix = format!("{pad_indent}- ");
                let item_w = width.saturating_sub(indent + 2).max(1);
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        let mut sub = Vec::new();
                        append_value_lines(&mut sub, item, 0, item_w, theme);
                        if sub.is_empty() {
                            sub.push(Line::from(Span::styled(
                                EMPTY,
                                Style::default().fg(theme.gray),
                            )));
                        }
                        for (idx, sub_line) in sub.into_iter().enumerate() {
                            let mut spans = vec![Span::styled(
                                if idx == 0 {
                                    bullet_prefix.clone()
                                } else {
                                    format!("{pad_indent}  ")
                                },
                                key_style,
                            )];
                            spans.extend(sub_line.spans);
                            out.push(Line::from(spans));
                        }
                    }
                    _ => {
                        let s = match item {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let wrapped = wrap(&s, item_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            let pfx = if idx == 0 {
                                bullet_prefix.clone()
                            } else {
                                format!("{pad_indent}  ")
                            };
                            out.push(Line::from(vec![
                                Span::styled(pfx, key_style),
                                Span::styled(line, val_style),
                            ]));
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            let max_key_w = map
                .keys()
                .map(|k| UnicodeWidthStr::width(k.as_str()))
                .max()
                .unwrap_or(0);
            for (k, v) in map {
                let kw = UnicodeWidthStr::width(k.as_str());
                let key_pad = " ".repeat(max_key_w.saturating_sub(kw) + 2);
                match v {
                    _ if is_empty_container(v) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(format!("{k}{key_pad}"), key_style),
                            Span::styled(EMPTY, Style::default().fg(theme.gray)),
                        ]));
                    }
                    Value::Object(_) | Value::Array(_) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(k.clone(), key_style),
                        ]));
                        append_value_lines(out, v, indent + 2, width, theme);
                    }
                    _ => {
                        let s = match v {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let prefix_w = indent + max_key_w + 2;
                        let val_w = width.saturating_sub(prefix_w).max(1);
                        let wrapped = wrap(&s, val_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            if idx == 0 {
                                out.push(Line::from(vec![
                                    Span::raw(pad_indent.clone()),
                                    Span::styled(format!("{k}{key_pad}"), key_style),
                                    Span::styled(line, val_style),
                                ]));
                            } else {
                                out.push(Line::from(vec![
                                    Span::raw(" ".repeat(prefix_w)),
                                    Span::styled(line, val_style),
                                ]));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn first_non_empty_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn build_step_lines(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    let is_last_of_node = run
        .steps
        .iter()
        .rposition(|s| s.node_id == step.node_id)
        == Some(step_idx);
    let is_gate = run.summary.status == RunStatus::Waiting
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && is_last_of_node;

    let (glyph, glyph_color) = if is_gate && answered.is_none() {
        (NodeState::Waiting.glyph(), theme.warning)
    } else if is_gate {
        // The gate has been answered: done, like the graph and the run header.
        let state = NodeState::Done(Outcome::Ok);
        (state.glyph(), node_state_color(state, theme))
    } else {
        let state = NodeState::Done(step.outcome);
        (state.glyph(), node_state_color(state, theme))
    };

    let dur = fmt_duration(step.duration_ms());
    out.push(title_row_right(
        glyph,
        glyph_color,
        &step.node_id,
        &dur,
        width,
        theme,
    ));

    // Facts row
    let settings_map = bundle::step_settings(&run.summary.dir);
    let agent_lbl = settings_map
        .get(&step.attempt_id)
        .map(AgentSettings::label)
        .filter(|s| !s.is_empty())
        .or_else(|| step.agent_name.clone());

    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        if !lbl.is_empty() {
            facts.push(lbl);
        }
    }
    facts.push(step.kind.label().to_string());
    facts.push(step.attempt_id.clone());
    if let Some(act) = &step.action {
        if let Some(code) = act.exit_code {
            facts.push(format!("exit {code}"));
        }
    }
    let facts_str = truncate_str(&facts.join(" · "), width);
    out.push(Line::from(Span::styled(
        facts_str,
        Style::default().fg(theme.gray),
    )));

    if let Some(err) = &step.error {
        if !err.is_empty() {
            push_wrapped(
                &mut out,
                err,
                width,
                Style::default().fg(theme.accent_error),
            );
        }
    }

    if is_gate {
        if let Some(detail) = &run.summary.status_detail {
            if !detail.is_empty() {
                let color = if answered.is_some() { theme.gray } else { theme.warning };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    // Prompt
    if let Some(prompt) = &step.prompt_text {
        if !prompt.trim().is_empty() {
            push_section_rule(&mut out, "Prompt", width, theme);
            push_wrapped_capped(
                &mut out,
                prompt,
                width,
                14,
                Style::default().fg(theme.text_secondary),
                theme,
            );
        }
    }

    // Command (for actions)
    if let Some(action) = &step.action {
        let mut cmd_parts = Vec::new();
        if let Some(cmd) = &action.command {
            if !cmd.is_empty() {
                cmd_parts.push(cmd.clone());
            }
        }
        for arg in &action.args {
            cmd_parts.push(arg.clone());
        }
        let has_cmd = !cmd_parts.is_empty();
        let has_cwd = action.cwd.as_deref().is_some_and(|c| !c.is_empty());
        if has_cmd || has_cwd {
            push_section_rule(&mut out, "Command", width, theme);
            if has_cmd {
                let cmd_line = cmd_parts.join(" ");
                push_wrapped(
                    &mut out,
                    &cmd_line,
                    width,
                    Style::default().fg(theme.command),
                );
            }
            if let Some(cwd) = &action.cwd {
                if !cwd.is_empty() {
                    push_wrapped(&mut out, cwd, width, Style::default().fg(theme.path));
                }
            }
        }
    }

    // Conversation
    if let Some(conv) = &step.conversation {
        push_section_rule(&mut out, "Conversation", width, theme);
        match run.session_events_path(&conv.session_id) {
            Some(path) => {
                append_conversation(
                    &mut out,
                    path,
                    Some((conv.event_start_seq, conv.event_end_seq)),
                    None,
                    width,
                    theme,
                );
            }
            None => {
                out.push(dim_line("session events file not found", theme));
            }
        }
    }

    // Result
    let result_lines = format_value_lines(&step.output, width, theme);
    if !result_lines.is_empty() {
        push_section_rule(&mut out, "Result", width, theme);
        out.extend(result_lines);
    }

    // stdout / stderr for actions
    if let Some(art) = &step.stdout {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stdout", &run.summary.dir, art, width, theme);
        }
    }
    if let Some(art) = &step.stderr {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stderr", &run.summary.dir, art, width, theme);
        }
    }

    // Events
    let evs: Vec<_> = run
        .step_events(&step.attempt_id)
        .into_iter()
        .filter(|e| e.kind != "node_heartbeat")
        .collect();
    if !evs.is_empty() {
        push_section_rule(&mut out, "Events", width, theme);
        let base_ms = if step.started_ms > 0 {
            step.started_ms
        } else {
            evs.first().map(|e| e.at_ms).unwrap_or(0)
        };
        for ev in evs {
            out.push(format_trace_event(ev, base_ms, width, theme));
        }
    }

    out
}

fn append_artifact_section(
    out: &mut Vec<Line<'static>>,
    title: &str,
    run_dir: &Path,
    artifact: &bundle::ArtifactRef,
    width: usize,
    theme: &Theme,
) {
    const MAX_BYTES: usize = 64 * 1024;
    match bundle::read_artifact(run_dir, artifact, MAX_BYTES) {
        Ok(text) => {
            if text.is_empty() {
                return;
            }
            push_section_rule(out, title, width, theme);
            let raw_lines: Vec<&str> = text.lines().collect();
            let start = raw_lines.len().saturating_sub(30);
            for raw in &raw_lines[start..] {
                push_wrapped(out, raw, width, Style::default().fg(theme.text_primary));
            }
            if artifact.bytes > MAX_BYTES as u64 {
                out.push(dim_line("… output cut at 64 KiB", theme));
            }
        }
        Err(_) => {
            push_section_rule(out, title, width, theme);
            out.push(dim_line("artifact unreadable", theme));
        }
    }
}

fn build_node_lines(
    run: &Run,
    node_id: &str,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let state = run.node_state(node_id);
    let shown = run.node_state_answered(node_id, answered.is_some());
    let glyph = shown.glyph();
    let color = node_state_color(shown, theme);

    out.push(title_row_right(glyph, color, node_id, "", width, theme));

    let settings = bundle::node_settings(&run.summary.dir);
    let agent_lbl = settings
        .get(node_id)
        .map(AgentSettings::label)
        .filter(|s| !s.is_empty());
    let flow_node = run.flow.nodes.iter().find(|n| n.id == node_id);
    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        facts.push(lbl);
    }
    if let Some(n) = flow_node {
        facts.push(n.kind.label().to_string());
    }
    if !facts.is_empty() {
        out.push(Line::from(Span::styled(
            truncate_str(&facts.join(" · "), width),
            Style::default().fg(theme.gray),
        )));
    }

    match state {
        NodeState::Running => {
            push_section_rule(&mut out, "Conversation (live)", width, theme);
            let handle = settings
                .get(node_id)
                .and_then(|s| s.handle.as_deref())
                .unwrap_or("main");
            if let Some(sess) = run.sessions.iter().find(|s| s.handle == handle) {
                let min_exclusive = run
                    .steps
                    .iter()
                    .filter_map(|s| s.conversation.as_ref())
                    .filter(|c| c.session_id == sess.id)
                    .map(|c| c.event_end_seq)
                    .max();
                append_conversation(
                    &mut out,
                    &sess.events_path,
                    None,
                    min_exclusive,
                    width,
                    theme,
                );
            } else {
                out.push(dim_line("no session bound yet", theme));
            }

            let finished_attempts: std::collections::HashSet<&str> =
                run.steps.iter().map(|s| s.attempt_id.as_str()).collect();
            let evs: Vec<_> = run
                .events
                .iter()
                .filter(|e| {
                    e.node_id.as_deref() == Some(node_id)
                        && e.attempt_id
                            .as_deref()
                            .map(|a| !finished_attempts.contains(a))
                            .unwrap_or(true)
                        && e.kind != "node_heartbeat"
                })
                .collect();
            if !evs.is_empty() {
                push_section_rule(&mut out, "Events", width, theme);
                let base_ms = evs.first().map(|e| e.at_ms).unwrap_or(run.summary.started_ms);
                for ev in evs {
                    out.push(format_trace_event(ev, base_ms, width, theme));
                }
            }
        }
        NodeState::Waiting => {
            if let Some(detail) = &run.summary.status_detail {
                if !detail.is_empty() {
                    let color = if answered.is_some() { theme.gray } else { theme.warning };
                    push_wrapped(&mut out, detail, width, Style::default().fg(color));
                }
            }
            append_gate_section(&mut out, run, answered, width, theme);
        }
        NodeState::NotRun => {
            let msg = if matches!(
                run.summary.status,
                RunStatus::Running | RunStatus::Waiting
            ) {
                "not run yet"
            } else {
                "not run"
            };
            out.push(dim_line(msg, theme));
        }
        NodeState::Done(_) => {
            out.push(dim_line("completed", theme));
        }
    }

    out
}

fn append_gate_section(
    out: &mut Vec<Line<'static>>,
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) {
    let choices = bundle::gate_choices(run);
    if choices.is_empty() && answered.is_some() {
        return;
    }
    push_section_rule(out, "Gate", width, theme);
    if choices.is_empty() {
        out.push(dim_line(
            "no next step declared: re-run the flow to continue",
            theme,
        ));
        return;
    }
    for (idx, (label, flow_file)) in choices.iter().enumerate() {
        let spans = match answered {
            // Answered: nothing to pick any more, so no key numbers; the picked one is ticked.
            Some(picked) => {
                let chosen = picked.eq_ignore_ascii_case(label);
                let (mark, mark_color, label_style) = if chosen {
                    (
                        "✓ ",
                        theme.accent_success,
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", theme.gray_dim, Style::default().fg(theme.gray))
                };
                vec![
                    Span::styled(mark, Style::default().fg(mark_color)),
                    Span::styled(format!("{label} "), label_style),
                    Span::styled("→ ", Style::default().fg(theme.gray_dim)),
                    Span::styled(flow_file.clone(), Style::default().fg(theme.gray)),
                ]
            }
            None => {
                let n = idx + 1;
                vec![
                    Span::styled(
                        format!("[{n}] "),
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{label} "),
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("→ ", Style::default().fg(theme.gray_dim)),
                    Span::styled(flow_file.clone(), Style::default().fg(theme.path)),
                ]
            }
        };
        out.push(truncate_line(Line::from(spans), width));
    }
}

#[derive(Debug, Clone)]
enum ConvItem {
    Prompt(String),
    Thought(String),
    ToolCall {
        id: String,
        title: String,
        status: String,
    },
    Message(String),
}

fn append_conversation(
    out: &mut Vec<Line<'static>>,
    path: &Path,
    seq_range: Option<(u64, u64)>,
    min_exclusive_seq: Option<u64>,
    width: usize,
    theme: &Theme,
) {
    let raw = match read_last_512k(path) {
        Ok(s) => s,
        Err(_) => {
            out.push(dim_line("session events file unavailable", theme));
            return;
        }
    };

    let items = parse_conversation_items(&raw, seq_range, min_exclusive_seq);
    if items.is_empty() {
        out.push(dim_line("no conversation events", theme));
        return;
    }

    let mut prev_kind: Option<u8> = None;
    for item in items {
        let kind_tag = match &item {
            ConvItem::Prompt(_) => 1,
            ConvItem::Thought(_) => 2,
            ConvItem::ToolCall { .. } => 3,
            ConvItem::Message(_) => 4,
        };
        if let Some(prev) = prev_kind {
            if prev != kind_tag {
                out.push(Line::default());
            }
        }
        prev_kind = Some(kind_tag);

        match item {
            ConvItem::Prompt(text) => {
                let wrapped = wrap(&text, width);
                let style = Style::default().fg(theme.gray_dim);
                for line in wrapped.into_iter().take(3) {
                    out.push(Line::from(Span::styled(line, style)));
                }
            }
            ConvItem::Thought(text) => {
                let body_w = width.saturating_sub(2).max(1);
                let wrapped = wrap(&text, body_w);
                let thought_style = Style::default()
                    .fg(theme.gray)
                    .add_modifier(Modifier::ITALIC);
                for (idx, line) in wrapped.into_iter().enumerate() {
                    let prefix = if idx == 0 { "∴ " } else { "  " };
                    out.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
                        Span::styled(line, thought_style),
                    ]));
                }
            }
            ConvItem::ToolCall { title, status, .. } => {
                let status_color = match status.as_str() {
                    "completed" => theme.accent_success,
                    "failed" => theme.accent_error,
                    _ => theme.accent_running,
                };
                let status_w = UnicodeWidthStr::width(status.as_str());
                let max_title_w = width.saturating_sub(status_w + 4).max(1);
                let clean_title = first_non_empty_line(&title);
                let cut_title = truncate_str(
                    if clean_title.is_empty() {
                        "tool"
                    } else {
                        &clean_title
                    },
                    max_title_w,
                );
                out.push(Line::from(vec![
                    Span::styled("▸ ", Style::default().fg(theme.accent_tool)),
                    Span::styled(cut_title, Style::default().fg(theme.text_secondary)),
                    Span::styled("  ", Style::default()),
                    Span::styled(status, Style::default().fg(status_color)),
                ]));
            }
            ConvItem::Message(text) => {
                push_wrapped(
                    out,
                    &text,
                    width,
                    Style::default().fg(theme.text_primary),
                );
            }
        }
    }
}

fn read_last_512k(path: &Path) -> std::io::Result<String> {
    const LIMIT: u64 = 512 * 1024;
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let cut = len > LIMIT;
    if cut {
        f.seek(SeekFrom::End(-(LIMIT as i64)))?;
    }
    let mut bytes = Vec::with_capacity(len.min(LIMIT) as usize);
    f.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    if cut {
        if let Some(pos) = text.find('\n') {
            return Ok(text[pos + 1..].to_string());
        }
        return Ok(String::new());
    }
    Ok(text.into_owned())
}

fn parse_conversation_items(
    raw: &str,
    seq_range: Option<(u64, u64)>,
    min_exclusive_seq: Option<u64>,
) -> Vec<ConvItem> {
    let mut items: Vec<ConvItem> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let seq = val.get("seq").and_then(Value::as_u64).unwrap_or(0);
        if let Some((start, end)) = seq_range {
            if seq < start || seq > end {
                continue;
            }
        }
        if let Some(min_seq) = min_exclusive_seq {
            if seq <= min_seq {
                continue;
            }
        }
        let Some(msg) = val.get("message") else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "_auth/status_update" {
            continue;
        }
        let direction = val.get("direction").and_then(Value::as_str).unwrap_or("");

        if direction == "outbound" && method == "session/prompt" {
            if let Some(prompt_arr) = msg
                .get("params")
                .and_then(|p| p.get("prompt"))
                .and_then(Value::as_array)
            {
                let mut parts = Vec::new();
                for part in prompt_arr {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        if !t.is_empty() {
                            parts.push(t);
                        }
                    }
                }
                if !parts.is_empty() {
                    items.push(ConvItem::Prompt(parts.join("\n")));
                }
            }
            continue;
        }

        if direction == "inbound" && method == "session/update" {
            let Some(update) = msg.get("params").and_then(|p| p.get("update")) else {
                continue;
            };
            let kind = update
                .get("sessionUpdate")
                .and_then(Value::as_str)
                .unwrap_or("");
            match kind {
                "agent_thought_chunk" => {
                    if let Some(text) = update
                        .get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(Value::as_str)
                    {
                        if let Some(ConvItem::Thought(prev)) = items.last_mut() {
                            prev.push_str(text);
                        } else {
                            items.push(ConvItem::Thought(text.to_string()));
                        }
                    }
                }
                "agent_message_chunk" => {
                    if let Some(text) = update
                        .get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(Value::as_str)
                    {
                        if let Some(ConvItem::Message(prev)) = items.last_mut() {
                            prev.push_str(text);
                        } else {
                            items.push(ConvItem::Message(text.to_string()));
                        }
                    }
                }
                "tool_call" => {
                    let id = update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let title = update
                        .get("title")
                        .and_then(Value::as_str)
                        .or_else(|| update.get("kind").and_then(Value::as_str))
                        .unwrap_or("tool")
                        .to_string();
                    let status = update
                        .get("status")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("running")
                        .to_string();
                    items.push(ConvItem::ToolCall { id, title, status });
                }
                "tool_call_update" => {
                    let target_id = update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let new_title = update
                        .get("title")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    let new_status = update
                        .get("status")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());

                    if let Some(ConvItem::ToolCall { title, status, .. }) = items
                        .iter_mut()
                        .rev()
                        .find(|it| matches!(it, ConvItem::ToolCall { id, .. } if id == target_id))
                    {
                        if let Some(t) = new_title {
                            *title = t.to_string();
                        }
                        if let Some(s) = new_status {
                            *status = s.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    items
}

fn format_trace_event(
    ev: &bundle::TraceEvent,
    base_ms: i64,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let delta_ms = ev.at_ms.saturating_sub(base_ms).max(0) as u64;
    let prefix = format!("+{delta_ms}ms  ");
    let kind_str = format!("{}", ev.kind);
    let mut spans = vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(kind_str, Style::default().fg(theme.gray)),
    ];
    if !ev.detail.is_empty() {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            ev.detail.clone(),
            Style::default().fg(theme.text_secondary),
        ));
    }
    truncate_line(Line::from(spans), width)
}

fn push_section_rule(
    out: &mut Vec<Line<'static>>,
    name: &str,
    width: usize,
    theme: &Theme,
) {
    out.push(Line::default());
    let prefix = "── ";
    let mid = format!("{name} ");
    let used = UnicodeWidthStr::width(prefix) + UnicodeWidthStr::width(mid.as_str());
    let rem = width.saturating_sub(used);
    let trail = "─".repeat(rem);
    out.push(Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(
            mid,
            Style::default()
                .fg(theme.gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(trail, Style::default().fg(theme.gray_dim)),
    ]));
}

fn title_row_right(
    glyph: &str,
    glyph_color: Color,
    title: &str,
    right: &str,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let right_w = UnicodeWidthStr::width(right);
    let left_max = if right_w > 0 {
        width.saturating_sub(right_w + 2)
    } else {
        width
    };
    let glyph_part = format!("{glyph} ");
    let glyph_w = UnicodeWidthStr::width(glyph_part.as_str());
    let title_cut = truncate_str(title, left_max.saturating_sub(glyph_w));
    let left_used = glyph_w + UnicodeWidthStr::width(title_cut.as_str());

    let mut spans = vec![
        Span::styled(
            glyph_part,
            Style::default().fg(glyph_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title_cut,
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if right_w > 0 && left_used + 1 + right_w <= width {
        let pad = " ".repeat(width - left_used - right_w);
        spans.push(Span::raw(pad));
        spans.push(Span::styled(
            right.to_string(),
            Style::default().fg(theme.gray),
        ));
    }
    Line::from(spans)
}

fn push_wrapped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    style: Style,
) {
    for line in wrap(text, width) {
        out.push(Line::from(Span::styled(line, style)));
    }
}

fn push_wrapped_capped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    max_rows: usize,
    style: Style,
    theme: &Theme,
) {
    let wrapped = wrap(text, width);
    let total = wrapped.len();
    for line in wrapped.into_iter().take(max_rows) {
        out.push(Line::from(Span::styled(line, style)));
    }
    if total > max_rows {
        let more = total - max_rows;
        out.push(dim_line(&format!("… +{more} more rows"), theme));
    }
}

fn dim_line(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(theme.gray_dim),
    ))
}

fn truncate_line(line: Line<'static>, width: usize) -> Line<'static> {
    let mut remaining = width;
    let mut spans = Vec::new();
    for span in line.spans {
        if remaining == 0 {
            break;
        }
        let w = UnicodeWidthStr::width(span.content.as_ref());
        if w <= remaining {
            remaining -= w;
            spans.push(span);
        } else {
            let cut = truncate_str(span.content.as_ref(), remaining);
            spans.push(Span::styled(cut, span.style));
            break;
        }
    }
    Line::from(spans)
}

fn run_status_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.accent_success,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray,
    }
}

fn node_state_color(state: NodeState, theme: &Theme) -> Color {
    match state {
        NodeState::NotRun => theme.gray_dim,
        NodeState::Running => theme.accent_running,
        NodeState::Waiting => theme.warning,
        NodeState::Done(Outcome::Ok) => theme.accent_success,
        NodeState::Done(Outcome::Failed | Outcome::TimedOut) => theme.accent_error,
        NodeState::Done(Outcome::Cancelled | Outcome::Unknown) => theme.gray,
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        let mut clean = String::with_capacity(raw_line.len());
        for ch in raw_line.chars() {
            if ch == '\t' {
                clean.push_str("    ");
            } else if !ch.is_control() {
                clean.push(ch);
            }
        }
        if clean.is_empty() {
            out.push(String::new());
            continue;
        }

        let mut cur = String::new();
        let mut cur_w = 0usize;

        let mut chars = clean.chars().peekable();
        while let Some(&ch) = chars.peek() {
            if ch == ' ' {
                let mut sp_count = 0usize;
                while chars.peek() == Some(&' ') {
                    chars.next();
                    sp_count += 1;
                }
                if cur.is_empty() {
                    for _ in 0..sp_count {
                        if cur_w + 1 > width {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        cur.push(' ');
                        cur_w += 1;
                    }
                } else if chars.peek().is_some() {
                    if cur_w + sp_count <= width {
                        for _ in 0..sp_count {
                            cur.push(' ');
                        }
                        cur_w += sp_count;
                    } else {
                        out.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                }
            } else {
                let mut word = String::new();
                let mut word_w = 0usize;
                while let Some(&c) = chars.peek() {
                    if c == ' ' {
                        break;
                    }
                    chars.next();
                    word.push(c);
                    word_w += UnicodeWidthChar::width(c).unwrap_or(0);
                }

                if word_w <= width {
                    if cur_w + word_w <= width {
                        cur.push_str(&word);
                        cur_w += word_w;
                    } else {
                        let trimmed = cur.trim_end_matches(' ').to_string();
                        out.push(trimmed);
                        cur = word;
                        cur_w = word_w;
                    }
                } else {
                    if !cur.is_empty() {
                        let trimmed = cur.trim_end_matches(' ').to_string();
                        if !trimmed.is_empty() {
                            out.push(trimmed);
                        }
                        cur.clear();
                        cur_w = 0;
                    }
                    for c in word.chars() {
                        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                        if cur_w + cw > width && !cur.is_empty() {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        cur.push(c);
                        cur_w += cw;
                    }
                }
            }
        }

        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        ActionReceipt, ArtifactRef, Conversation, FlowDef, FlowEdge, FlowNode, RunSummary,
        SessionRef, TraceEvent,
    };
    use serde_json::json;

    fn make_test_run(dir: &Path) -> Run {
        Run {
            summary: RunSummary {
                run_id: "run-1".to_string(),
                dir: dir.to_path_buf(),
                flow_name: "feel".to_string(),
                title: "feel: /tmp/test".to_string(),
                status: RunStatus::Running,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: Some("work".to_string()),
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "feel".to_string(),
                start_at: "list".to_string(),
                nodes: vec![
                    FlowNode {
                        id: "list".to_string(),
                        kind: NodeKind::Action,
                        summary: None,
                        is_shell: true,
                    },
                    FlowNode {
                        id: "work".to_string(),
                        kind: NodeKind::Acp,
                        summary: None,
                        is_shell: false,
                    },
                    FlowNode {
                        id: "review".to_string(),
                        kind: NodeKind::Checkpoint,
                        summary: Some("human review".to_string()),
                        is_shell: false,
                    },
                ],
                edges: vec![
                    FlowEdge::Simple {
                        from: "list".to_string(),
                        to: "work".to_string(),
                    },
                    FlowEdge::Simple {
                        from: "work".to_string(),
                        to: "review".to_string(),
                    },
                ],
            },
            steps: Vec::new(),
            events: Vec::new(),
            input: json!({"target": "/tmp/test"}),
            sessions: Vec::new(),
        }
    }

    fn buffer_lines(buf: &Buffer) -> Vec<String> {
        let area = buf.area;
        let mut rows = Vec::new();
        for y in area.y..area.y + area.height {
            let mut s = String::new();
            for x in area.x..area.x + area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            rows.push(s.trim_end().to_string());
        }
        rows
    }

    #[test]
    fn test_wrap_breaks_on_spaces_and_keeps_blank_lines() {
        let wrapped = wrap("hello world\n\nfoo bar baz", 10);
        assert_eq!(wrapped, vec!["hello", "world", "", "foo bar", "baz"]);
    }

    #[test]
    fn test_wrap_hard_breaks_long_words_and_tabs() {
        let wrapped = wrap("abcdefghijkl\t\tx", 5);
        assert_eq!(wrapped[0], "abcde");
        assert_eq!(wrapped[1], "fghij");
        assert!(wrapped.iter().any(|l| l.contains("kl")));
    }

    #[test]
    fn test_wrap_wide_characters() {
        // Each CJK char has display width 2
        let wrapped = wrap("你好世界测试", 6);
        assert_eq!(wrapped, vec!["你好世", "界测试"]);
    }

    #[test]
    fn test_step_sections_order_prompt_result_events() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_500,
            prompt_text: Some("Inspect the repository".to_string()),
            raw_text: None,
            output: json!({"answer": "all good"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.events.push(TraceEvent {
            seq: 1,
            at_ms: 1_100,
            scope: "node".to_string(),
            kind: "node_started".to_string(),
            node_id: Some("work".to_string()),
            attempt_id: Some("work#1".to_string()),
            detail: "Running acp node".to_string(),
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 40),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let text = buffer_lines(&buf).join("\n");
        let p_idx = text.find("── Prompt").expect("Prompt section");
        let r_idx = text.find("── Result").expect("Result section");
        let e_idx = text.find("── Events").expect("Events section");
        assert!(p_idx < r_idx && r_idx < e_idx);
    }

    #[test]
    fn test_action_step_shows_command_exit_code_and_stdout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let art_dir = tmp.path().join("artifacts");
        std::fs::create_dir_all(&art_dir).expect("mkdir");
        let stdout_rel = "artifacts/out.txt";
        std::fs::write(tmp.path().join(stdout_rel), "alpha\nbeta\n").expect("write");

        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_050,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: Some(ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("ls".to_string()),
                args: vec!["-la".to_string(), "/tmp/test".to_string()],
                cwd: Some("/private/tmp/test".to_string()),
                exit_code: Some(0),
                duration_ms: Some(12),
            }),
            conversation: None,
            stdout: Some(ArtifactRef {
                path: stdout_rel.to_string(),
                media_type: "text/plain".to_string(),
                bytes: 11,
            }),
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 30),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let text = buffer_lines(&buf).join("\n");
        assert!(text.contains("exit 0"));
        assert!(text.contains("── Command"));
        assert!(text.contains("ls -la /tmp/test"));
        assert!(text.contains("── stdout"));
        assert!(text.contains("alpha"));
        assert!(text.contains("beta"));
    }

    #[test]
    fn test_conversation_thought_tool_call_and_message() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let lines = [
            json!({"seq": 1, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_thought_chunk", "content": {"text": "Thinking part 1 "}}}}}),
            json!({"seq": 2, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_thought_chunk", "content": {"text": "and part 2."}}}}}),
            json!({"seq": 3, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "tool_call", "toolCallId": "tc1", "title": "Bash", "kind": "execute"}}}}),
            json!({"seq": 4, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "tool_call_update", "toolCallId": "tc1", "title": "ls -la", "status": "completed"}}}}),
            json!({"seq": 5, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "Hello "}}}}}),
            json!({"seq": 6, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "world!"}}}}}),
        ];
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&events_path, body).expect("write events");

        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path,
            agent_name: Some("claude".to_string()),
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 1,
                event_end_seq: 6,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 30),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let rendered = buffer_lines(&buf);
        let full = rendered.join("\n");
        assert!(full.contains("∴ Thinking part 1 and part 2."));
        let tool_rows: Vec<_> = rendered.iter().filter(|l| l.contains('▸')).collect();
        assert_eq!(tool_rows.len(), 1);
        assert!(tool_rows[0].contains("ls -la"));
        assert!(tool_rows[0].contains("completed"));
        assert!(full.contains("Hello world!"));
    }

    #[test]
    fn test_conversation_filters_seq_range_and_skips_auth_status_update() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let lines = [
            json!({"seq": 1, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "OUTSIDE_BEFORE"}}}}}),
            json!({"seq": 2, "direction": "inbound", "message": {"method": "_auth/status_update", "params": {"secret": "SUPER_SECRET_TOKEN_123", "update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "SUPER_SECRET_TOKEN_123"}}}}}),
            json!({"seq": 3, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "INSIDE_RANGE"}}}}}),
            json!({"seq": 4, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "OUTSIDE_AFTER"}}}}}),
        ];
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&events_path, body).expect("write");

        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path,
            agent_name: None,
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 2,
                event_end_seq: 3,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 25));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 25),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("INSIDE_RANGE"));
        assert!(!full.contains("OUTSIDE_BEFORE"));
        assert!(!full.contains("OUTSIDE_AFTER"));
        assert!(!full.contains("SUPER_SECRET_TOKEN_123"));
    }

    #[test]
    fn test_missing_events_file_renders_dim_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path: tmp.path().join("does-not-exist.ndjson"),
            agent_name: None,
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 1,
                event_end_seq: 5,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 20),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("── Conversation"));
        assert!(full.contains("unavailable"));
    }

    #[test]
    fn test_run_target_lists_stages_and_waiting_gate_choices() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.summary.status = RunStatus::Waiting;
        run.summary.waiting_on = Some("review".to_string());
        run.summary.status_detail = Some("awaiting approval".to_string());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_100,
            prompt_text: None,
            raw_text: None,
            output: json!({"ok": true}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.steps.push(Step {
            attempt_id: "review#1".to_string(),
            node_id: "review".to_string(),
            kind: NodeKind::Checkpoint,
            outcome: Outcome::Ok,
            started_ms: 1_200,
            finished_ms: 1_210,
            prompt_text: None,
            raw_text: None,
            output: json!({
                "checkpoint": "review",
                "next": {
                    "ship": "deploy.flow.ts",
                    "abort": "rollback.flow.ts"
                }
            }),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 35));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 90, 35),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("awaiting approval"));
        assert!(full.contains("── Gate"));
        assert!(full.contains("[1] abort → rollback.flow.ts"));
        assert!(full.contains("[2] ship → deploy.flow.ts"));
        assert!(full.contains("── Stages"));
        assert!(full.contains("✓ list"));
        assert!(full.contains("◐ review"));
    }

    /// A run waiting at checkpoint `review`, whose last step offers two choices.
    fn waiting_gate_run(dir: &Path) -> Run {
        let mut run = make_test_run(dir);
        run.summary.status = RunStatus::Waiting;
        run.summary.waiting_on = Some("review".to_string());
        run.summary.status_detail = Some("land it, or send it back?".to_string());
        for (id, node, kind, output) in [
            ("list#1", "list", NodeKind::Action, json!({"ok": true})),
            (
                "review#1",
                "review",
                NodeKind::Checkpoint,
                json!({"next": {"ship": "deploy.flow.ts", "abort": "rollback.flow.ts"}}),
            ),
        ] {
            run.steps.push(Step {
                attempt_id: id.to_string(),
                node_id: node.to_string(),
                kind,
                outcome: Outcome::Ok,
                started_ms: 1_000,
                finished_ms: 1_100,
                prompt_text: None,
                raw_text: None,
                output,
                error: None,
                agent_name: None,
                action: None,
                conversation: None,
                stdout: None,
                stderr: None,
                output_artifact: None,
            });
        }
        run
    }

    fn panel_text(
        run: &Run,
        target: &DetailTarget,
        answered: Option<&str>,
    ) -> String {
        let mut panel = DetailPanel::new();
        panel.set_answered(answered);
        let area = Rect::new(0, 0, 90, 35);
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, run, target, &Theme::default(), 5_000);
        buffer_lines(&buf).join("\n")
    }

    #[test]
    fn answered_gate_is_drawn_done_without_pickable_keys() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());

        // Open gate, for contrast: live glyph and key numbers.
        let open_step = panel_text(&run, &DetailTarget::Step(1), None);
        assert!(open_step.contains("◐ review"));
        assert!(open_step.contains("[1] abort → rollback.flow.ts"));

        for target in [DetailTarget::Step(1), DetailTarget::Run] {
            let text = panel_text(&run, &target, Some("ship"));
            assert!(text.contains("── Gate"), "{target:?}\n{text}");
            assert!(text.contains("✓ ship → deploy.flow.ts"), "{target:?}\n{text}");
            assert!(text.contains("  abort → rollback.flow.ts"), "{target:?}\n{text}");
            assert!(!text.contains("[1]") && !text.contains("[2]"), "{target:?}\n{text}");
            assert!(!text.contains('◐'), "{target:?}\n{text}");
            assert!(text.contains("✓ review"), "{target:?}\n{text}");
        }
    }

    #[test]
    fn answered_gate_with_unknown_choice_ticks_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());
        let text = panel_text(&run, &DetailTarget::Step(1), Some(""));
        assert!(text.contains("✓ review"));
        assert!(text.contains("  ship → deploy.flow.ts"));
        assert!(!text.contains("✓ ship") && !text.contains("✓ abort"));
        assert!(!text.contains("[1]"));
    }

    #[test]
    fn answer_arriving_redraws_a_cached_panel() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());
        let mut panel = DetailPanel::new();
        let area = Rect::new(0, 0, 90, 35);
        let theme = Theme::default();
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Step(1), &theme, 5_000);
        assert!(buffer_lines(&buf).join("\n").contains("[1] abort"));

        panel.set_answered(Some("abort"));
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Step(1), &theme, 5_000);
        let text = buffer_lines(&buf).join("\n");
        assert!(text.contains("✓ abort → rollback.flow.ts"));
        assert!(!text.contains("[1]"));
    }

    #[test]
    fn empty_list_and_object_values_read_as_empty_in_dim_style() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!({"verdict": "approve", "findings": [], "meta": {}, "items": [[], {}]}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        let theme = Theme::default();

        // Key/value rows: key, then (empty) in the same colour as the key.
        let lines = build_step_lines(&run, 0, &run.steps[0], None, 60, &theme);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert!(text.iter().any(|r| r.starts_with("findings") && r.ends_with("(empty)")), "{text:?}");
        assert!(text.iter().any(|r| r.starts_with("meta") && r.ends_with("(empty)")), "{text:?}");
        assert_eq!(text.iter().filter(|r| r.contains("- (empty)")).count(), 2, "{text:?}");
        let empties: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|sp| sp.content.as_ref() == "(empty)")
            .collect();
        assert!(!empties.is_empty());
        // The same colour as the keys next to it, not fainter.
        assert!(empties.iter().all(|sp| sp.style.fg == Some(theme.gray)));
        let key = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|sp| sp.content.trim() == "findings")
            .expect("findings key");
        assert_eq!(key.style.fg, empties[0].style.fg);

        // Stages one-line summary.
        let run_lines = build_run_lines(&run, None, 90, &theme, 5_000);
        let run_text: String = run_lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(run_text.contains("verdict approve, findings (empty), meta (empty)"), "{run_text}");
    }

    #[test]
    fn test_node_target_not_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.summary.status = RunStatus::Completed;
        run.summary.current_node = None;

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 15));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 15),
            &run,
            &DetailTarget::Node("review".to_string()),
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("○ review"));
        assert!(full.contains("not run"));
    }

    #[test]
    fn test_scroll_clamps_and_returns_false_at_limits() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        let long_prompt = (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some(long_prompt),
            raw_text: None,
            output: json!({"a": 1, "b": 2, "c": 3, "d": 4, "e": 5}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 8));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 8),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );

        assert!(!panel.scroll(-1));
        assert!(panel.scroll(2));
        assert!(panel.scroll(1_000));
        assert!(!panel.scroll(1));
        assert!(panel.scroll(-1));
    }

    #[test]
    fn test_target_change_resets_scroll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm".to_string()),
            raw_text: None,
            output: json!({"res": "1\n2\n3\n4\n5\n6\n7\n8"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll(5));
        assert!(panel.scroll > 0);

        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, 0);
    }

    #[test]
    fn test_running_node_tail_follows_and_stops_on_scroll_up() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let mut lines = Vec::new();
        for seq in 1..=15 {
            lines.push(
                serde_json::to_string(&json!({
                    "seq": seq,
                    "direction": "inbound",
                    "message": {
                        "method": "session/update",
                        "params": {
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {"text": format!("msg line {seq}\n")}
                            }
                        }
                    }
                }))
                .expect("json"),
            );
        }
        std::fs::write(&events_path, lines.join("\n")).expect("write");

        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path: events_path.clone(),
            agent_name: Some("claude".to_string()),
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        let target = DetailTarget::Node("work".to_string());
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 6),
            &run,
            &target,
            &Theme::default(),
            5_000,
        );
        let initial_scroll = panel.scroll;
        assert!(initial_scroll > 0);
        assert!(buffer_lines(&buf).join("\n").contains("msg line 15"));

        // Append more events; tail_follow keeps us at the bottom
        for seq in 16..=20 {
            lines.push(
                serde_json::to_string(&json!({
                    "seq": seq,
                    "direction": "inbound",
                    "message": {
                        "method": "session/update",
                        "params": {
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {"text": format!("msg line {seq}\n")}
                            }
                        }
                    }
                }))
                .expect("json"),
            );
        }
        std::fs::write(&events_path, lines.join("\n")).expect("write");
        buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 6),
            &run,
            &target,
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll > initial_scroll);
        assert!(buffer_lines(&buf).join("\n").contains("msg line 20"));

        // Scroll up disables tail-following
        assert!(panel.scroll(-3));
        let fixed_scroll = panel.scroll;
        lines.push(
            serde_json::to_string(&json!({
                "seq": 21,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"text": "msg line 21\n"}
                        }
                    }
                }
            }))
            .expect("json"),
        );
        std::fs::write(&events_path, lines.join("\n")).expect("write");
        buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 6),
            &run,
            &target,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, fixed_scroll);
    }

    #[test]
    fn test_last_result_object_and_array_renders_clean_without_json_punctuation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_100,
            prompt_text: None,
            raw_text: None,
            output: json!({
                "summary": "all good",
                "files": ["a.rs", "b.rs"]
            }),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 35));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 90, 35),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        let rows = buffer_lines(&buf);
        let stage_row = rows
            .iter()
            .find(|r| r.contains("✓ list"))
            .expect("list stage row");
        assert!(stage_row.contains("summary all good"));
        assert!(!stage_row.contains('{'));
        assert!(!stage_row.contains('"'));

        let lr_idx = rows
            .iter()
            .position(|r| r.starts_with("── Last result"))
            .expect("Last result rule");
        let inp_idx = rows
            .iter()
            .position(|r| r.starts_with("── Input"))
            .expect("Input rule");
        let section = &rows[lr_idx + 1..inp_idx];
        assert!(section.iter().any(|r| r.contains("summary") && r.contains("all good")));
        assert!(section.iter().any(|r| r.contains("a.rs")));
        assert!(section.iter().any(|r| r.contains("b.rs")));
        for line in section {
            assert!(!line.contains('{'));
            assert!(!line.contains('}'));
            assert!(!line.contains('['));
            assert!(!line.contains("\"summary\":"));
        }
    }

    #[test]
    fn test_string_output_wraps_and_empty_or_null_hides_section() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.input = json!({});
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!("plain wrapped text"),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let lines = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let joined = text.join("\n");
        assert!(joined.contains("── Last result"));
        assert!(joined.contains("plain wrapped text"));
        assert!(!joined.contains("── Input"));

        run.steps[0].output = Value::Null;
        let lines_null = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let joined_null: String = lines_null
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined_null.contains("── Last result"));

        run.steps[0].output = json!({});
        let lines_empty = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let joined_empty: String = lines_empty
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined_empty.contains("── Last result"));
    }

    #[test]
    fn test_array_output_of_40_items_renders_all_without_cap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        let items: Vec<String> = (0..40).map(|i| format!("item-{i}")).collect();
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!(items),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let lines = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        for i in 0..40 {
            assert!(text.iter().any(|r| r.contains(&format!("item-{i}"))));
        }
        assert!(!text.iter().any(|r| r.contains("more rows")));
    }

    #[test]
    fn test_small_and_zero_rects_do_not_panic_and_stay_inside_area() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = make_test_run(tmp.path());
        let mut panel = DetailPanel::new();

        let mut buf0 = Buffer::empty(Rect::new(0, 0, 0, 0));
        panel.render(
            &mut buf0,
            Rect::new(0, 0, 0, 0),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );

        let mut buf_narrow = Buffer::empty(Rect::new(0, 0, 8, 3));
        panel.render(
            &mut buf_narrow,
            Rect::new(0, 0, 8, 3),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        for cell in &buf_narrow.content {
            assert_eq!(cell.symbol(), " ");
        }

        // Sub-rect bounds check
        let outer = Rect::new(0, 0, 60, 20);
        let sub = Rect::new(5, 3, 40, 10);
        let mut big = Buffer::empty(outer);
        panel.render(
            &mut big,
            sub,
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        for y in 0..outer.height {
            for x in 0..outer.width {
                let inside =
                    x >= sub.x && x < sub.x + sub.width && y >= sub.y && y < sub.y + sub.height;
                if !inside {
                    assert_eq!(
                        big[(x, y)].symbol(),
                        " ",
                        "cell ({x},{y}) modified outside area"
                    );
                }
            }
        }
    }
}

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` (complete file, repository grok-build-acpx)
````
use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{NodeState, Outcome, RunStatus, RunSummary, fmt_duration};
use crate::acpx::sessions::SessionSummary;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerTab {
    Flows,
    Sessions,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LedgerOutcome {
    Unchanged,
    Changed,
    OpenRun(PathBuf),
    OpenSession(SessionSummary),
    Copy(String),
    Rescan,
    Close,
}

/// The gate this run answered: read from this run's own input, written by `bundle::gate_input` when a gate answer
/// starts the next flow file. `projections/run.json` -> `input.gate = { "runId": <the waiting run>, "node": .., "choice": .. }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateAnswer {
    pub run_id: String,   // input.gate.runId: the run that was waiting at the gate
    pub choice: String,   // input.gate.choice: the label that was picked
}

/// Per-run facts the parent loads (cached) so the ledger does no I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
    pub answers: Option<GateAnswer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowOrder {
    Grouped,
    Recent,
    Name,
}

impl FlowOrder {
    fn label(self) -> &'static str {
        match self {
            Self::Grouped => "grouped",
            Self::Recent => "recent",
            Self::Name => "name",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Grouped => Self::Recent,
            Self::Recent => Self::Name,
            Self::Name => Self::Grouped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionOrder {
    Recent,
    Name,
}

impl SessionOrder {
    fn label(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Name => "name",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Recent => Self::Name,
            Self::Name => Self::Recent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    NeedsYou,
    Running,
    Finished,
}

impl SectionKind {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "NEEDS YOU",
            Self::Running => "RUNNING",
            Self::Finished => "FINISHED",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::NeedsYou => theme.warning,
            Self::Running => theme.accent_running,
            Self::Finished => theme.gray_dim,
        }
    }
}

#[derive(Debug, Clone)]
enum FlowListRow {
    Header { kind: SectionKind, count: usize },
    Blank,
    Run(usize), // index in `runs`
    Continuation { indent: usize, text: String },
}

impl FlowListRow {
    fn is_selectable(&self) -> bool {
        matches!(self, Self::Run(_))
    }
}

pub struct LedgerState {
    tab: LedgerTab,
    runs: Vec<RunSummary>,
    extras: HashMap<String, RowExtra>,
    answered: HashMap<String, String>,
    sessions: Vec<SessionSummary>,

    // Per-tab filter, order, active-only, selection & scroll
    flow_filter: String,
    flow_active_only: bool,
    flow_order: FlowOrder,
    flow_selected_run_id: Option<String>,
    flow_scroll: usize,

    session_filter: String,
    session_active_only: bool,
    session_order: SessionOrder,
    session_selected_id: Option<String>,
    session_scroll: usize,

    filter_open: bool,
    last_render_height: usize,
    last_render_width: usize,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerState {
    pub fn new() -> Self {
        Self {
            tab: LedgerTab::Flows,
            runs: Vec::new(),
            extras: HashMap::new(),
            answered: HashMap::new(),
            sessions: Vec::new(),

            flow_filter: String::new(),
            flow_active_only: false,
            flow_order: FlowOrder::Grouped,
            flow_selected_run_id: None,
            flow_scroll: 0,

            session_filter: String::new(),
            session_active_only: false,
            session_order: SessionOrder::Recent,
            session_selected_id: None,
            session_scroll: 0,

            filter_open: false,
            last_render_height: 14,
            last_render_width: 100,
        }
    }

    pub fn set_runs(&mut self, runs: Vec<RunSummary>) {
        self.runs = runs;
        self.rebuild_answered();
        self.reconcile_flow_selection();
    }

    pub fn set_extras(&mut self, extras: HashMap<String, RowExtra>) {
        self.extras = extras;
        self.rebuild_answered();
        self.reconcile_flow_selection();
    }

    pub fn answered_choice(&self, run_id: &str) -> Option<&str> {
        if self
            .runs
            .iter()
            .find(|r| r.run_id == run_id)
            .is_some_and(|r| r.status != RunStatus::Waiting)
        {
            return None;
        }
        self.answered.get(run_id).map(String::as_str)
    }

    fn rebuild_answered(&mut self) {
        let mut winners: HashMap<String, (&str, Option<i64>, &str)> = HashMap::new();
        let mut keys: Vec<&String> = self.extras.keys().collect();
        keys.sort();
        for answering_id in keys {
            if let Some(extra) = self.extras.get(answering_id)
                && let Some(ref ans) = extra.answers
            {
                let started = self
                    .runs
                    .iter()
                    .find(|r| &r.run_id == answering_id)
                    .map(|r| r.started_ms);
                let replace = match winners.get(&ans.run_id) {
                    None => true,
                    Some(&(prev_id, prev_started, _)) => match (prev_started, started) {
                        (Some(ps), Some(ns)) => (ns, answering_id.as_str()) > (ps, prev_id),
                        _ => answering_id.as_str() > prev_id,
                    },
                };
                if replace {
                    winners.insert(
                        ans.run_id.clone(),
                        (answering_id.as_str(), started, ans.choice.as_str()),
                    );
                }
            }
        }
        self.answered = winners
            .into_iter()
            .map(|(k, (_, _, choice))| (k, choice.to_string()))
            .collect();
    }

    pub fn selected_run(&self) -> Option<&RunSummary> {
        if self.tab != LedgerTab::Flows {
            return None;
        }
        let id = self.flow_selected_run_id.as_deref()?;
        let indices = self.filtered_flow_indices();
        indices
            .into_iter()
            .find_map(|idx| self.runs.get(idx).filter(|r| r.run_id == id))
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.sessions = sessions;
        self.reconcile_session_selection();
    }

    pub fn tab(&self) -> LedgerTab {
        self.tab
    }

    pub fn any_live(&self) -> bool {
        self.runs
            .iter()
            .any(|r| r.status.is_live() && !self.is_stalled(r))
    }

    pub fn captures_text(&self) -> bool {
        self.filter_open
    }

    pub fn esc_closes(&self) -> bool {
        !self.filter_open && self.active_filter().is_empty()
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        if self.filter_open {
            vec!["Enter apply", "Esc clear"]
        } else {
            vec![
                "Enter open",
                "Tab switch",
                "/ filter",
                "a active",
                "s sort",
                "y copy",
                "R rescan",
                "Esc close",
            ]
        }
    }

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
        let mut indices: Vec<usize> = self
            .runs
            .iter()
            .enumerate()
            .filter_map(|(i, r)| self.matches_flow_filter(r).then_some(i))
            .collect();

        match self.flow_order {
            FlowOrder::Grouped => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    let sa = match self.run_section(ra) {
                        SectionKind::NeedsYou => 0,
                        SectionKind::Running => 1,
                        SectionKind::Finished => 2,
                    };
                    let sb = match self.run_section(rb) {
                        SectionKind::NeedsYou => 0,
                        SectionKind::Running => 1,
                        SectionKind::Finished => 2,
                    };
                    sa.cmp(&sb)
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Recent => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    rb.updated_ms
                        .cmp(&ra.updated_ms)
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Name => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    ra.title
                        .to_lowercase()
                        .cmp(&rb.title.to_lowercase())
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
        }
        indices
    }

    /// Build visual rows for the Flows tab. `list_height` is used to decide whether blank rows
    /// separate sections when the list is shorter than the area.
    fn build_flow_rows(&self, list_height: usize, now_ms: i64) -> Vec<FlowListRow> {
        let indices = self.filtered_flow_indices();
        if indices.is_empty() {
            return Vec::new();
        }
        if self.flow_order != FlowOrder::Grouped {
            let mut out = Vec::new();
            for idx in indices {
                out.push(FlowListRow::Run(idx));
                self.append_continuations(&mut out, idx, now_ms);
            }
            return out;
        }

        let mut needs_you = Vec::new();
        let mut running = Vec::new();
        let mut finished = Vec::new();
        for idx in indices {
            if let Some(r) = self.runs.get(idx) {
                match self.run_section(r) {
                    SectionKind::NeedsYou => needs_you.push(idx),
                    SectionKind::Running => running.push(idx),
                    SectionKind::Finished => finished.push(idx),
                }
            }
        }

        let sections: Vec<(SectionKind, Vec<usize>)> = [
            (SectionKind::NeedsYou, needs_you),
            (SectionKind::Running, running),
            (SectionKind::Finished, finished),
        ]
        .into_iter()
        .filter(|(_, items)| !items.is_empty())
        .collect();

        let extra_rows: usize = sections
            .iter()
            .flat_map(|(_, items)| items.iter())
            .map(|&idx| self.continuation_lines_for(idx, now_ms).len())
            .sum();
        let non_blank_rows: usize = sections
            .iter()
            .map(|(_, items)| 1 + items.len())
            .sum::<usize>()
            + extra_rows;
        let blank_count = sections.len().saturating_sub(1);
        let insert_blanks = non_blank_rows + blank_count <= list_height;

        let mut out = Vec::with_capacity(non_blank_rows + blank_count);
        for (i, (kind, items)) in sections.into_iter().enumerate() {
            if i > 0 && insert_blanks {
                out.push(FlowListRow::Blank);
            }
            out.push(FlowListRow::Header {
                kind,
                count: items.len(),
            });
            for idx in items {
                out.push(FlowListRow::Run(idx));
                self.append_continuations(&mut out, idx, now_ms);
            }
        }
        out
    }

    fn append_continuations(&self, out: &mut Vec<FlowListRow>, idx: usize, now_ms: i64) {
        for (indent, text) in self.continuation_lines_for(idx, now_ms) {
            out.push(FlowListRow::Continuation { indent, text });
        }
    }

    fn continuation_lines_for(&self, idx: usize, now_ms: i64) -> Vec<(usize, String)> {
        let Some(run) = self.runs.get(idx) else {
            return Vec::new();
        };
        let is_selected = self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
        if !is_selected || run.status != RunStatus::Waiting || self.is_answered(run) {
            return Vec::new();
        }
        let (_, indent, lines) = self.flow_question_lines(run, self.last_render_width, now_ms);
        lines.into_iter().skip(1).map(|t| (indent, t)).collect()
    }

    fn filtered_session_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| self.matches_session_filter(s).then_some(i))
            .collect();

        match self.session_order {
            SessionOrder::Recent => {
                indices.sort_by(|&a, &b| {
                    let sa = &self.sessions[a];
                    let sb = &self.sessions[b];
                    sb.last_used_ms
                        .cmp(&sa.last_used_ms)
                        .then_with(|| sb.created_ms.cmp(&sa.created_ms))
                        .then_with(|| sa.record_id.cmp(&sb.record_id))
                });
            }
            SessionOrder::Name => {
                indices.sort_by(|&a, &b| {
                    let sa = &self.sessions[a];
                    let sb = &self.sessions[b];
                    sa.title
                        .to_lowercase()
                        .cmp(&sb.title.to_lowercase())
                        .then_with(|| sb.last_used_ms.cmp(&sa.last_used_ms))
                        .then_with(|| sa.record_id.cmp(&sb.record_id))
                });
            }
        }
        indices
    }

    fn reconcile_flow_selection(&mut self) {
        let indices = self.filtered_flow_indices();
        if indices.is_empty() {
            self.flow_selected_run_id = None;
            self.flow_scroll = 0;
            return;
        }
        if let Some(ref cur_id) = self.flow_selected_run_id
            && indices
                .iter()
                .any(|&i| self.runs.get(i).map(|r| &r.run_id) == Some(cur_id))
        {
            return;
        }
        if let Some(&first_idx) = indices.first()
            && let Some(r) = self.runs.get(first_idx)
        {
            self.flow_selected_run_id = Some(r.run_id.clone());
        }
    }

    fn reconcile_session_selection(&mut self) {
        let indices = self.filtered_session_indices();
        if indices.is_empty() {
            self.session_selected_id = None;
            self.session_scroll = 0;
            return;
        }
        if let Some(ref cur_id) = self.session_selected_id
            && indices
                .iter()
                .any(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(cur_id))
        {
            return;
        }
        if let Some(&first_idx) = indices.first()
            && let Some(s) = self.sessions.get(first_idx)
        {
            self.session_selected_id = Some(s.record_id.clone());
        }
    }

    fn move_selection(&mut self, delta: isize) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let indices = self.filtered_flow_indices();
                if indices.is_empty() {
                    return false;
                }
                let cur_pos = self
                    .flow_selected_run_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.runs.get(i).map(|r| &r.run_id) == Some(id))
                    })
                    .unwrap_or(0);
                let max_pos = indices.len().saturating_sub(1);
                let new_pos = (cur_pos as isize + delta).clamp(0, max_pos as isize) as usize;
                let new_id = self.runs.get(indices[new_pos]).map(|r| r.run_id.clone());
                if new_id != self.flow_selected_run_id {
                    self.flow_selected_run_id = new_id;
                    true
                } else {
                    false
                }
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                if indices.is_empty() {
                    return false;
                }
                let cur_pos = self
                    .session_selected_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(id))
                    })
                    .unwrap_or(0);
                let max_pos = indices.len().saturating_sub(1);
                let new_pos = (cur_pos as isize + delta).clamp(0, max_pos as isize) as usize;
                let new_id = self
                    .sessions
                    .get(indices[new_pos])
                    .map(|s| s.record_id.clone());
                if new_id != self.session_selected_id {
                    self.session_selected_id = new_id;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn jump_start(&mut self) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let indices = self.filtered_flow_indices();
                let first_id = indices
                    .first()
                    .and_then(|&i| self.runs.get(i))
                    .map(|r| r.run_id.clone());
                if first_id.is_some() && first_id != self.flow_selected_run_id {
                    self.flow_selected_run_id = first_id;
                    true
                } else {
                    false
                }
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                let first_id = indices
                    .first()
                    .and_then(|&i| self.sessions.get(i))
                    .map(|s| s.record_id.clone());
                if first_id.is_some() && first_id != self.session_selected_id {
                    self.session_selected_id = first_id;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn jump_end(&mut self) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let indices = self.filtered_flow_indices();
                let last_id = indices
                    .last()
                    .and_then(|&i| self.runs.get(i))
                    .map(|r| r.run_id.clone());
                if last_id.is_some() && last_id != self.flow_selected_run_id {
                    self.flow_selected_run_id = last_id;
                    true
                } else {
                    false
                }
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                let last_id = indices
                    .last()
                    .and_then(|&i| self.sessions.get(i))
                    .map(|s| s.record_id.clone());
                if last_id.is_some() && last_id != self.session_selected_id {
                    self.session_selected_id = last_id;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        self.move_selection(delta)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        if key.kind == KeyEventKind::Release {
            return LedgerOutcome::Unchanged;
        }

        if self.filter_open {
            return self.handle_filter_key(key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half_page = (self.last_render_height.saturating_sub(1) / 2).max(1) as isize;

        match key.code {
            KeyCode::Char('u') if ctrl => {
                if self.move_selection(-half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                if self.move_selection(half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.move_selection(1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.move_selection(-1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                if self.move_selection(half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                if self.move_selection(-half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.jump_start() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.jump_end() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.tab = match self.tab {
                    LedgerTab::Flows => LedgerTab::Sessions,
                    LedgerTab::Sessions => LedgerTab::Flows,
                };
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('/') => {
                self.filter_open = true;
                LedgerOutcome::Changed
            }
            KeyCode::Char('a') => {
                match self.tab {
                    LedgerTab::Flows => {
                        self.flow_active_only = !self.flow_active_only;
                        self.reconcile_flow_selection();
                    }
                    LedgerTab::Sessions => {
                        self.session_active_only = !self.session_active_only;
                        self.reconcile_session_selection();
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('s') => {
                match self.tab {
                    LedgerTab::Flows => {
                        self.flow_order = self.flow_order.next();
                        self.reconcile_flow_selection();
                    }
                    LedgerTab::Sessions => {
                        self.session_order = self.session_order.next();
                        self.reconcile_session_selection();
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Enter => match self.tab {
                LedgerTab::Flows => {
                    if let Some(run) = self.selected_run() {
                        LedgerOutcome::OpenRun(run.dir.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
                LedgerTab::Sessions => {
                    if let Some(sess) = self.selected_session() {
                        LedgerOutcome::OpenSession(sess.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('y') => match self.tab {
                LedgerTab::Flows => {
                    if let Some(run) = self.selected_run() {
                        LedgerOutcome::Copy(run.dir.to_string_lossy().into_owned())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
                LedgerTab::Sessions => {
                    if let Some(sess) = self.selected_session() {
                        LedgerOutcome::Copy(sess.record_id.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('R') => LedgerOutcome::Rescan,
            KeyCode::Esc => {
                if !self.active_filter().is_empty() {
                    self.active_filter_mut().clear();
                    match self.tab {
                        LedgerTab::Flows => self.reconcile_flow_selection(),
                        LedgerTab::Sessions => self.reconcile_session_selection(),
                    }
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Close
                }
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn handle_filter_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        match key.code {
            KeyCode::Enter => {
                self.filter_open = false;
                LedgerOutcome::Changed
            }
            KeyCode::Esc => {
                self.filter_open = false;
                self.active_filter_mut().clear();
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Backspace => {
                self.active_filter_mut().pop();
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.active_filter_mut().push(c);
                    match self.tab {
                        LedgerTab::Flows => self.reconcile_flow_selection(),
                        LedgerTab::Sessions => self.reconcile_session_selection(),
                    }
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn selected_session(&self) -> Option<&SessionSummary> {
        if self.tab != LedgerTab::Sessions {
            return None;
        }
        let id = self.session_selected_id.as_deref()?;
        let indices = self.filtered_session_indices();
        indices
            .into_iter()
            .find_map(|idx| self.sessions.get(idx).filter(|s| s.record_id == id))
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.last_render_height = area.height as usize;
        self.last_render_width = area.width as usize;

        // Row 0: Tab header
        let list_area = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        };

        let overflow_counter = self.compute_overflow_counter(list_area.height as usize, now_ms);
        self.render_tab_row(
            buf,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            theme,
            overflow_counter.as_deref(),
        );

        if list_area.height == 0 {
            return;
        }

        match self.tab {
            LedgerTab::Flows => self.render_flows(buf, list_area, theme, now_ms),
            LedgerTab::Sessions => self.render_sessions(buf, list_area, theme, now_ms),
        }
    }

    fn compute_overflow_counter(&self, list_height: usize, now_ms: i64) -> Option<String> {
        if list_height == 0 {
            return None;
        }
        match self.tab {
            LedgerTab::Flows => {
                let rows = self.build_flow_rows(list_height, now_ms);
                if rows.len() <= list_height {
                    return None;
                }
                let indices = self.filtered_flow_indices();
                let total = indices.len();
                if total == 0 {
                    return None;
                }
                let pos = self
                    .flow_selected_run_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.runs.get(i).map(|r| &r.run_id) == Some(id))
                    })
                    .unwrap_or(0)
                    + 1;
                Some(format!("{pos}/{total}"))
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                // 1 header row + indices.len() rows
                let total_rows = if indices.is_empty() {
                    0
                } else {
                    1 + indices.len()
                };
                if total_rows <= list_height {
                    return None;
                }
                let total = indices.len();
                let pos = self
                    .session_selected_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(id))
                    })
                    .unwrap_or(0)
                    + 1;
                Some(format!("{pos}/{total}"))
            }
        }
    }

    fn render_tab_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        theme: &Theme,
        overflow_counter: Option<&str>,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }

        let flow_count = self.runs.len();
        let sess_count = self.sessions.len();

        let flows_style = if self.tab == LedgerTab::Flows {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        let sess_style = if self.tab == LedgerTab::Sessions {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        let sep_style = Style::default().fg(theme.gray_dim);

        let left_flows = format!(" Flows {flow_count} ");
        let left_sep = "│";
        let left_sess = format!(" Sessions {sess_count} ");
        let left_width = left_flows.width() + left_sep.width() + left_sess.width();

        let left_line = Line::from(vec![
            Span::styled(left_flows, flows_style),
            Span::styled(left_sep, sep_style),
            Span::styled(left_sess, sess_style),
        ]);
        buf.set_line(row.x, row.y, &left_line, row.width);

        // Build right-aligned components:
        // Summary, Filter/active, [order], overflow_counter
        // Drop order when overlapping tabs: [order], then filter text, then summary.
        let summary_spans = self.build_summary_spans(theme);
        let filter_spans = self.build_filter_spans(theme);
        let order_label = match self.tab {
            LedgerTab::Flows => self.flow_order.label(),
            LedgerTab::Sessions => self.session_order.label(),
        };
        let order_spans = vec![Span::styled(
            format!("[{order_label}]"),
            Style::default().fg(theme.gray_dim),
        )];
        let counter_spans: Vec<Span<'static>> = overflow_counter
            .map(|c| vec![Span::styled(c.to_string(), Style::default().fg(theme.gray))])
            .unwrap_or_default();

        // Try combinations in priority order:
        // 1. summary + filter + order + counter
        // 2. summary + filter + counter (drop [order])
        // 3. summary + counter (drop filter)
        // 4. counter only (drop summary)
        let candidates: [&[&Vec<Span<'static>>]; 4] = [
            &[&summary_spans, &filter_spans, &order_spans, &counter_spans],
            &[&summary_spans, &filter_spans, &counter_spans],
            &[&summary_spans, &counter_spans],
            &[&counter_spans],
        ];

        let avail = w.saturating_sub(left_width + 1);
        for group in candidates {
            let joined = join_span_groups(group, theme);
            let jw: usize = joined.iter().map(|s| s.content.width()).sum();
            if jw > 0 && jw <= avail {
                let rx = row.x.saturating_add((w - jw) as u16);
                let line = Line::from(joined);
                buf.set_line(rx, row.y, &line, jw as u16);
                break;
            }
        }
    }

    fn build_summary_spans(&self, theme: &Theme) -> Vec<Span<'static>> {
        let waiting = self
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Waiting && !self.is_answered(r))
            .count();
        let running = self
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Running && !self.is_stalled(r))
            .count();

        let mut spans = Vec::new();
        if waiting > 0 {
            spans.push(Span::styled(
                format!("{waiting} needs you"),
                Style::default().fg(theme.warning),
            ));
        }
        if running > 0 {
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", Style::default().fg(theme.gray_dim)));
            }
            spans.push(Span::styled(
                format!("{running} running"),
                Style::default().fg(theme.accent_running),
            ));
        }
        spans
    }

    fn build_filter_spans(&self, theme: &Theme) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let filter = self.active_filter();
        if self.filter_open || !filter.is_empty() {
            let text = if self.filter_open {
                format!("/{filter}▏")
            } else {
                format!("/{filter}")
            };
            spans.push(Span::styled(text, Style::default().fg(theme.command)));
        }
        let active_only = match self.tab {
            LedgerTab::Flows => self.flow_active_only,
            LedgerTab::Sessions => self.session_active_only,
        };
        if active_only {
            if !spans.is_empty() {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                "active",
                Style::default().fg(theme.accent_running),
            ));
        }
        spans
    }

    fn render_flows(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        let list_h = area.height as usize;
        let rows = self.build_flow_rows(list_h, now_ms);

        if rows.is_empty() {
            let msg = if !self.flow_filter.is_empty() || self.flow_active_only {
                "No matching flow runs"
            } else {
                "No acpx flow runs found"
            };
            render_empty_centered(buf, area, msg, theme);
            return;
        }

        // Find visual row index of the selected run
        let selected_visual_idx = self.flow_selected_run_id.as_ref().and_then(|sel_id| {
            rows.iter().position(|row| match row {
                FlowListRow::Run(idx) => self.runs.get(*idx).map(|r| &r.run_id) == Some(sel_id),
                _ => false,
            })
        });

        // Adjust scroll to keep selection visible (and include header row immediately above if at top)
        if let Some(sel_v) = selected_visual_idx {
            let mut sel_end = sel_v + 1;
            while matches!(rows.get(sel_end), Some(FlowListRow::Continuation { .. })) {
                sel_end += 1;
            }
            let sel_h = sel_end - sel_v;
            if sel_v < self.flow_scroll {
                self.flow_scroll = sel_v;
                // If the row right above `sel_v` is a section header, show it too when possible
                if sel_v > 0
                    && matches!(rows.get(sel_v - 1), Some(FlowListRow::Header { .. }))
                    && list_h > sel_h
                {
                    self.flow_scroll = sel_v - 1;
                }
            } else if sel_end > self.flow_scroll + list_h {
                self.flow_scroll = sel_end.saturating_sub(list_h).min(sel_v);
            }
        }
        let max_scroll = rows.len().saturating_sub(list_h);
        if self.flow_scroll > max_scroll {
            self.flow_scroll = max_scroll;
        }

        for (line_offset, row) in rows
            .iter()
            .skip(self.flow_scroll)
            .take(list_h)
            .enumerate()
        {
            let y = area.y.saturating_add(line_offset as u16);
            let row_rect = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            match row {
                FlowListRow::Blank => {}
                FlowListRow::Header { kind, count } => {
                    self.render_section_header(buf, row_rect, *kind, *count, theme);
                }
                FlowListRow::Run(idx) => {
                    if let Some(run) = self.runs.get(*idx) {
                        let is_selected =
                            self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
                        self.render_flow_row(buf, row_rect, run, is_selected, theme, now_ms);
                    }
                }
                FlowListRow::Continuation { indent, text } => {
                    buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
                    let content = format!("{}{}", " ".repeat(*indent), text);
                    let line = Line::from(Span::styled(
                        truncate_str(&content, row_rect.width as usize),
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ));
                    buf.set_line(row_rect.x, row_rect.y, &line, row_rect.width);
                }
            }
        }
    }

    fn render_section_header(
        &self,
        buf: &mut Buffer,
        row: Rect,
        kind: SectionKind,
        count: usize,
        theme: &Theme,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }
        let label = format!("{} {} ", kind.label(), count);
        let label_cut = truncate_str(&label, w);
        let lw = label_cut.width();
        let rule_len = w.saturating_sub(lw);
        let rule = "─".repeat(rule_len);

        let line = Line::from(vec![
            Span::styled(
                label_cut,
                Style::default()
                    .fg(kind.color(theme))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rule, Style::default().fg(theme.gray_dim)),
        ]);
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn render_flow_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        run: &RunSummary,
        selected: bool,
        theme: &Theme,
        now_ms: i64,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }

        let base_mod = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        if selected {
            buf.set_style(row, Style::default().bg(theme.bg_highlight));
        }

        let extra = self.extras.get(&run.run_id);
        let stalled = extra.map(|e| e.stalled).unwrap_or(false);
        let answered = self.is_answered(run);

        // Status glyph
        let (glyph, glyph_color) = if stalled {
            ("⊘", theme.gray)
        } else if answered {
            (RunStatus::Completed.glyph(), theme.accent_success)
        } else {
            let c = match run.status {
                RunStatus::Running => theme.accent_running,
                RunStatus::Waiting => theme.warning,
                RunStatus::Completed => theme.accent_success,
                RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
                RunStatus::Unknown => theme.gray_dim,
            };
            (run.status.glyph(), c)
        };

        // Optional right/middle columns dropped by width thresholds:
        // agents below 120, duration below 96, <done>/<total> below 80, age below 64, rail below 48.
        let show_rail = w >= 48;
        let show_age = w >= 64;
        let show_rail_count = w >= 80;
        let show_duration = w >= 96;
        let show_agents = w >= 120;

        // Build rail spans + width
        let mut rail_spans: Vec<Span<'static>> = Vec::new();
        let mut rail_width: usize = 0;
        if show_rail
            && let Some(ex) = extra
            && !ex.rail.is_empty()
        {
            // An answered gate is done, so the ticks and the count below read the same states.
            let rail: Vec<NodeState> = ex
                .rail
                .iter()
                .map(|&ns| {
                    if answered && ns == NodeState::Waiting {
                        NodeState::Done(Outcome::Ok)
                    } else {
                        ns
                    }
                })
                .collect();
            for &ns in &rail {
                let g = ns.glyph();
                let color = match ns {
                    NodeState::Running => theme.accent_running,
                    NodeState::Waiting => theme.warning,
                    NodeState::Done(Outcome::Ok) => theme.accent_success,
                    NodeState::Done(Outcome::Failed | Outcome::TimedOut) => theme.accent_error,
                    _ => theme.gray_dim,
                };
                rail_width += g.width();
                rail_spans.push(Span::styled(
                    g.to_string(),
                    Style::default().fg(color).add_modifier(base_mod),
                ));
            }
            if show_rail_count {
                let done = rail
                    .iter()
                    .filter(|s| matches!(s, NodeState::Done(_)))
                    .count();
                let total = rail.len();
                let cnt = format!(" {done}/{total}");
                rail_width += cnt.width();
                rail_spans.push(Span::styled(
                    cnt,
                    Style::default().fg(theme.gray).add_modifier(base_mod),
                ));
            }
        }

        // Pad the rail to the longest one shown, so the state text starts in one column.
        if show_rail {
            let count_w = |n: usize| if show_rail_count { format!(" {n}/{n}").width() } else { 0 };
            let widest = self
                .extras
                .values()
                .map(|e| if e.rail.is_empty() { 0 } else { e.rail.len() + count_w(e.rail.len()) })
                .max()
                .unwrap_or(0);
            if widest > rail_width {
                rail_spans.push(Span::raw(" ".repeat(widest - rail_width)));
                rail_width = widest;
            }
        }

        let agents_str = if show_agents {
            extra
                .map(|e| e.agents.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| truncate_str(s, 26))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let age_str = if show_age {
            fmt_age(now_ms.saturating_sub(run.updated_ms))
        } else {
            String::new()
        };

        let dur_str = if show_duration {
            let end_ms = if run.status.is_live() && !stalled {
                now_ms
            } else {
                run.finished_ms.unwrap_or(run.updated_ms)
            };
            let dur_ms = (end_ms - run.started_ms).max(0) as u64;
            fmt_duration(dur_ms)
        } else {
            String::new()
        };

        // Raw state text before cutting
        let (raw_state_text, state_color) = self.flow_state_text(run, extra, stalled, theme);

        // Calculate fixed columns width:
        // "<glyph> " (2 cols) + optional gaps & columns
        let prefix_w = glyph.width() + 1;
        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
        let agents_col_w = if !agents_str.is_empty() {
            1 + agents_str.width()
        } else {
            0
        };
        let age_col_w = if !age_str.is_empty() {
            1 + age_str.width()
        } else {
            0
        };
        let dur_col_w = if !dur_str.is_empty() {
            1 + dur_str.width()
        } else {
            0
        };

        let fixed_w = prefix_w + rail_col_w + agents_col_w + age_col_w + dur_col_w;
        let flex_budget = w.saturating_sub(fixed_w);

        // Title and state text share what is left. Every row uses the same title width, so the columns line up.
        let longest_title = self.runs.iter().map(|r| r.title.width()).max().unwrap_or(0);
        let wrap_selected_waiting =
            selected && run.status == RunStatus::Waiting && !answered && !raw_state_text.is_empty();
        let (title_str, state_str) = if flex_budget <= 17 || raw_state_text.is_empty() && rail_width == 0 {
            let first = if wrap_selected_waiting {
                let (first, _, _) = self.flow_question_lines(run, w, now_ms);
                first
            } else {
                String::new()
            };
            (truncate_str(&run.title, flex_budget), first)
        } else {
            let half = (flex_budget - 1).div_euclid(2);
            let title_col = longest_title.clamp(16, half.max(16));
            let title = truncate_str(&run.title, title_col);
            let pad = title_col.saturating_sub(title.width());
            let state = if wrap_selected_waiting {
                let (first, _, _) = self.flow_question_lines(run, w, now_ms);
                first
            } else {
                truncate_str(&raw_state_text, flex_budget - 1 - title_col)
            };
            (format!("{title}{}", " ".repeat(pad)), state)
        };

        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled(
            format!("{glyph} "),
            Style::default().fg(glyph_color).add_modifier(base_mod),
        ));
        if !title_str.is_empty() {
            spans.push(Span::styled(
                title_str.clone(),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(base_mod),
            ));
        }
        if ! rail_spans.is_empty() {
            spans.push(Span::raw(" "));
            spans.extend(rail_spans);
        }
        if !state_str.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                state_str,
                Style::default().fg(state_color).add_modifier(base_mod),
            ));
        }

        // Right-aligned metadata columns (agents, age, duration) so rows align cleanly
        let mut right_spans: Vec<Span<'static>> = Vec::new();
        if !agents_str.is_empty() {
            right_spans.push(Span::raw(" "));
            right_spans.push(Span::styled(
                agents_str,
                Style::default().fg(theme.gray).add_modifier(base_mod),
            ));
        }
        if !age_str.is_empty() {
            right_spans.push(Span::raw(" "));
            right_spans.push(Span::styled(
                age_str,
                Style::default().fg(theme.gray).add_modifier(base_mod),
            ));
        }
        if !dur_str.is_empty() {
            right_spans.push(Span::raw(" "));
            right_spans.push(Span::styled(
                dur_str,
                Style::default().fg(theme.gray_dim).add_modifier(base_mod),
            ));
        }

        let left_w: usize = spans.iter().map(|s| s.content.width()).sum();
        let right_w: usize = right_spans.iter().map(|s| s.content.width()).sum();
        if right_w > 0 && left_w + right_w < w {
            spans.push(Span::raw(" ".repeat(w - left_w - right_w)));
        }
        spans.extend(right_spans);

        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn flow_question_lines(
        &self,
        run: &RunSummary,
        w: usize,
        now_ms: i64,
    ) -> (String, usize, Vec<String>) {
        let extra = self.extras.get(&run.run_id);
        let stalled = extra.map(|e| e.stalled).unwrap_or(false);
        let (raw_state_text, _) = self.flow_state_text(run, extra, stalled, &Theme::default());
        if raw_state_text.is_empty() || w == 0 {
            return (String::new(), 0, Vec::new());
        }

        let show_rail = w >= 48;
        let show_age = w >= 64;
        let show_rail_count = w >= 80;
        let show_duration = w >= 96;
        let show_agents = w >= 120;

        let rail_width = if show_rail {
            let count_w = |n: usize| {
                if show_rail_count {
                    format!(" {n}/{n}").width()
                } else {
                    0
                }
            };
            self.extras
                .values()
                .map(|e| {
                    if e.rail.is_empty() {
                        0
                    } else {
                        e.rail.len() + count_w(e.rail.len())
                    }
                })
                .max()
                .unwrap_or(0)
        } else {
            0
        };

        let agents_w = if show_agents {
            extra
                .map(|e| e.agents.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| 1 + truncate_str(s, 26).width())
                .unwrap_or(0)
        } else {
            0
        };
        let age_w = if show_age {
            1 + fmt_age(now_ms.saturating_sub(run.updated_ms)).width()
        } else {
            0
        };
        let dur_w = if show_duration {
            let end_ms = if run.status.is_live() && !stalled {
                now_ms
            } else {
                run.finished_ms.unwrap_or(run.updated_ms)
            };
            let dur_ms = (end_ms - run.started_ms).max(0) as u64;
            1 + fmt_duration(dur_ms).width()
        } else {
            0
        };

        let prefix_w = run.status.glyph().width() + 1;
        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
        let fixed_w = prefix_w + rail_col_w + agents_w + age_w + dur_w;
        let flex_budget = w.saturating_sub(fixed_w);
        let longest_title = self.runs.iter().map(|r| r.title.width()).max().unwrap_or(0);

        if flex_budget <= 17 {
            let indent = prefix_w;
            let wrap_w = w.saturating_sub(indent).max(1);
            let mut lines = vec![String::new()];
            lines.extend(wrap_words(&raw_state_text, wrap_w));
            (String::new(), indent, lines)
        } else {
            let half = (flex_budget - 1).div_euclid(2);
            let title_col = longest_title.clamp(16, half.max(16));
            let indent = prefix_w + title_col + rail_col_w + 1;
            let state_w = (flex_budget - 1 - title_col).max(1);
            let lines = wrap_words(&raw_state_text, state_w);
            let first = lines.first().cloned().unwrap_or_default();
            (first, indent, lines)
        }
    }

    fn flow_state_text(
        &self,
        run: &RunSummary,
        extra: Option<&RowExtra>,
        stalled: bool,
        theme: &Theme,
    ) -> (String, Color) {
        if run.status == RunStatus::Waiting {
            if let Some(choice) = self.answered.get(&run.run_id) {
                let text = if choice.is_empty() {
                    "answered".to_string()
                } else {
                    format!("answered: {choice}")
                };
                return (text, theme.gray);
            }
            if let Some(q) = extra.and_then(|e| e.question.as_deref()).filter(|q| !q.is_empty()) {
                return (q.to_string(), theme.warning);
            }
            let wait_target = run.waiting_on.as_deref().unwrap_or("checkpoint");
            return (format!("waiting: {wait_target}"), theme.warning);
        }
        if stalled {
            let node = run.current_node.as_deref().unwrap_or("unknown");
            return (format!("stalled at {node}"), theme.gray);
        }
        match run.status {
            RunStatus::Running => {
                let node = run.current_node.as_deref().unwrap_or("running");
                let text = match run.status_detail.as_deref().filter(|d| !d.is_empty()) {
                    Some(detail) => format!("{node} · {detail}"),
                    None => node.to_string(),
                };
                (text, theme.accent_running)
            }
            RunStatus::Failed | RunStatus::TimedOut => {
                let err = run
                    .error
                    .as_deref()
                    .or(run.status_detail.as_deref())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| run.status.label());
                (err.to_string(), theme.accent_error)
            }
            RunStatus::Completed => ("done".to_string(), theme.gray),
            RunStatus::Waiting => unreachable!(),
            RunStatus::Unknown => (run.status.label().to_string(), theme.gray_dim),
        }
    }

    fn render_sessions(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        let indices = self.filtered_session_indices();
        if indices.is_empty() {
            let msg = if !self.session_filter.is_empty() || self.session_active_only {
                "No matching sessions"
            } else {
                "No acpx sessions found"
            };
            render_empty_centered(buf, area, msg, theme);
            return;
        }

        // Row 0 of `area` is the column header row in `theme.gray_dim`
        self.render_session_header(
            buf,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            theme,
        );

        let rows_h = area.height.saturating_sub(1) as usize;
        if rows_h == 0 {
            return;
        }

        let selected_pos = self
            .session_selected_id
            .as_ref()
            .and_then(|id| {
                indices
                    .iter()
                    .position(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(id))
            })
            .unwrap_or(0);

        if selected_pos < self.session_scroll {
            self.session_scroll = selected_pos;
        } else if selected_pos >= self.session_scroll + rows_h {
            self.session_scroll = (selected_pos + 1).saturating_sub(rows_h);
        }
        let max_scroll = indices.len().saturating_sub(rows_h);
        if self.session_scroll > max_scroll {
            self.session_scroll = max_scroll;
        }

        for (line_offset, &sess_idx) in indices
            .iter()
            .skip(self.session_scroll)
            .take(rows_h)
            .enumerate()
        {
            if let Some(sess) = self.sessions.get(sess_idx) {
                let y = area.y.saturating_add(1 + line_offset as u16);
                let row_rect = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                };
                let is_selected =
                    self.session_selected_id.as_deref() == Some(sess.record_id.as_str());
                self.render_session_row(buf, row_rect, sess, is_selected, theme, now_ms);
            }
        }
    }

    fn render_session_header(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }
        let layout = SessionColumns::for_width(w);
        let style = Style::default().fg(theme.gray_dim);
        let line_str = layout.format_row(" ", "SESSION", "AGENT", "CWD", "MSGS", "TOKENS", "COST", "AGE");
        let line = Line::from(Span::styled(truncate_str(&line_str, w), style));
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn render_session_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        sess: &SessionSummary,
        selected: bool,
        theme: &Theme,
        now_ms: i64,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }
        let base_mod = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        if selected {
            buf.set_style(row, Style::default().bg(theme.bg_highlight));
        }

        let layout = SessionColumns::for_width(w);
        let (glyph, glyph_color) = if sess.closed {
            ("○", theme.gray_dim)
        } else {
            ("●", theme.accent_running)
        };

        let cwd_last = last_path_component(&sess.cwd);
        let msgs = sess.messages.to_string();
        let tokens = fmt_tokens(sess.tokens);
        let cost = sess
            .cost_usd
            .map(|c| format!("${c:.2}"))
            .unwrap_or_default();
        let age = fmt_age(now_ms.saturating_sub(sess.last_used_ms));

        let spans = layout.build_spans(
            glyph,
            glyph_color,
            &sess.title,
            &sess.agent,
            cwd_last,
            &msgs,
            &tokens,
            &cost,
            &age,
            base_mod,
            theme,
        );
        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, row.width);
    }
}

struct SessionColumns {
    width: usize,
    show_agent: bool,
    show_cwd: bool,
    show_msgs: bool,
    show_tokens: bool,
    show_cost: bool,
    show_age: bool,
}

impl SessionColumns {
    const AGENT_W: usize = 12;
    const CWD_W: usize = 14;
    const MSGS_W: usize = 5;
    const TOKENS_W: usize = 7;
    const COST_W: usize = 7;
    const AGE_W: usize = 5;

    fn for_width(width: usize) -> Self {
        // Drop columns from the right as width shrinks:
        // Order from left to right: glyph(2) + title + agent + cwd + msgs + tokens + cost + age
        // Drop from right: age (<56), cost (<110), tokens (<96), msgs (<84), cwd (<70), agent (<54)
        let show_agent = width >= 46;
        let show_cwd = width >= 64;
        let show_msgs = width >= 74;
        let show_tokens = width >= 86;
        let show_cost = width >= 98;
        let show_age = width >= 108;
        Self {
            width,
            show_agent,
            show_cwd,
            show_msgs,
            show_tokens,
            show_cost,
            show_age,
        }
    }

    fn title_width(&self) -> usize {
        let mut fixed = 2; // glyph + space
        if self.show_agent {
            fixed += 1 + Self::AGENT_W;
        }
        if self.show_cwd {
            fixed += 1 + Self::CWD_W;
        }
        if self.show_msgs {
            fixed += 1 + Self::MSGS_W;
        }
        if self.show_tokens {
            fixed += 1 + Self::TOKENS_W;
        }
        if self.show_cost {
            fixed += 1 + Self::COST_W;
        }
        if self.show_age {
            fixed += 1 + Self::AGE_W;
        }
        self.width.saturating_sub(fixed).max(1)
    }

    #[allow(clippy::too_many_arguments)]
    fn format_row(
        &self,
        glyph: &str,
        title: &str,
        agent: &str,
        cwd: &str,
        msgs: &str,
        tokens: &str,
        cost: &str,
        age: &str,
    ) -> String {
        let tw = self.title_width();
        let mut out = format!("{} {}", glyph, pad_right(&truncate_str(title, tw), tw));
        if self.show_agent {
            out.push(' ');
            out.push_str(&pad_right(&truncate_str(agent, Self::AGENT_W), Self::AGENT_W));
        }
        if self.show_cwd {
            out.push(' ');
            out.push_str(&pad_right(&truncate_str(cwd, Self::CWD_W), Self::CWD_W));
        }
        if self.show_msgs {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(msgs, Self::MSGS_W), Self::MSGS_W));
        }
        if self.show_tokens {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(tokens, Self::TOKENS_W), Self::TOKENS_W));
        }
        if self.show_cost {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(cost, Self::COST_W), Self::COST_W));
        }
        if self.show_age {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(age, Self::AGE_W), Self::AGE_W));
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn build_spans(
        &self,
        glyph: &str,
        glyph_color: Color,
        title: &str,
        agent: &str,
        cwd: &str,
        msgs: &str,
        tokens: &str,
        cost: &str,
        age: &str,
        base_mod: Modifier,
        theme: &Theme,
    ) -> Vec<Span<'static>> {
        let tw = self.title_width();
        let mut spans = Vec::new();
        spans.push(Span::styled(
            format!("{glyph} "),
            Style::default().fg(glyph_color).add_modifier(base_mod),
        ));
        spans.push(Span::styled(
            pad_right(&truncate_str(title, tw), tw),
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(base_mod),
        ));
        let sec_style = Style::default().fg(theme.gray).add_modifier(base_mod);
        if self.show_agent {
            spans.push(Span::styled(
                format!(" {}", pad_right(&truncate_str(agent, Self::AGENT_W), Self::AGENT_W)),
                sec_style,
            ));
        }
        if self.show_cwd {
            spans.push(Span::styled(
                format!(" {}", pad_right(&truncate_str(cwd, Self::CWD_W), Self::CWD_W)),
                sec_style,
            ));
        }
        if self.show_msgs {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(msgs, Self::MSGS_W), Self::MSGS_W)),
                sec_style,
            ));
        }
        if self.show_tokens {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(tokens, Self::TOKENS_W), Self::TOKENS_W)),
                sec_style,
            ));
        }
        if self.show_cost {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(cost, Self::COST_W), Self::COST_W)),
                sec_style,
            ));
        }
        if self.show_age {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(age, Self::AGE_W), Self::AGE_W)),
                sec_style,
            ));
        }
        spans
    }
}

fn wrap_words(text: &str, max_w: usize) -> Vec<String> {
    let max_w = max_w.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for word in text.split_whitespace() {
        let ww = word.width();
        if cur.is_empty() {
            if ww <= max_w {
                cur.push_str(word);
                cur_w = ww;
            } else {
                for ch in word.chars() {
                    let cw = ch.to_string().width();
                    if cur_w + cw > max_w && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
            }
        } else if cur_w + 1 + ww <= max_w {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.clear();
            cur_w = 0;
            if ww <= max_w {
                cur.push_str(word);
                cur_w = ww;
            } else {
                for ch in word.chars() {
                    let cw = ch.to_string().width();
                    if cur_w + cw > max_w && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn pad_right(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - w))
    }
}

fn last_path_component(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return path;
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
}

fn fmt_tokens(tokens: u64) -> String {
    if tokens == 0 {
        return "0".to_string();
    }
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn join_span_groups(groups: &[&Vec<Span<'static>>], theme: &Theme) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for g in groups {
        if g.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(Span::styled("  ", Style::default().fg(theme.gray_dim)));
        }
        out.extend((*g).iter().cloned());
    }
    out
}

fn render_empty_centered(buf: &mut Buffer, area: Rect, msg: &str, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let y = area.y.saturating_add(area.height / 2);
    let cut = truncate_str(msg, area.width as usize);
    let mw = cut.width() as u16;
    let x = area.x.saturating_add(area.width.saturating_sub(mw) / 2);
    let line = Line::from(Span::styled(cut, Style::default().fg(theme.gray_dim)));
    buf.set_line(x, y, &line, area.width.saturating_sub(x - area.x));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_run(id: &str, title: &str, status: RunStatus, updated_ms: i64) -> RunSummary {
        RunSummary {
            run_id: id.to_string(),
            dir: PathBuf::from(format!("/tmp/runs/{id}")),
            flow_name: format!("flow-{id}"),
            title: title.to_string(),
            status,
            started_ms: updated_ms - 5_000,
            updated_ms,
            finished_ms: matches!(
                status,
                RunStatus::Completed | RunStatus::Failed | RunStatus::TimedOut
            )
            .then_some(updated_ms),
            current_node: Some("build_node".to_string()),
            waiting_on: (status == RunStatus::Waiting).then(|| "approve_step".to_string()),
            status_detail: None,
            error: (status == RunStatus::Failed).then(|| "exit code 1".to_string()),
        }
    }

    fn make_session(id: &str, title: &str, closed: bool, last_used_ms: i64) -> SessionSummary {
        SessionSummary {
            record_id: id.to_string(),
            acp_session_id: format!("acp-{id}"),
            name: format!("name-{id}"),
            title: title.to_string(),
            agent: "grok".to_string(),
            cwd: "/workspace/my-repo".to_string(),
            created_ms: last_used_ms - 10_000,
            last_used_ms,
            closed,
            messages: 12,
            tokens: 12_300,
            cost_usd: Some(0.12),
            stream_path: PathBuf::from(format!("/tmp/sessions/{id}.stream.ndjson")),
        }
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        let w = buf.area.width;
        let mut s = String::new();
        for x in 0..w {
            s.push_str(buf[(x, y)].symbol());
        }
        s.trim_end().to_string()
    }

    fn all_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn grouped_order_and_navigation_skips_headers_and_clamps() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-done", "Done Run", RunStatus::Completed, 3000),
            make_run("r-wait", "Waiting Run", RunStatus::Waiting, 1000),
            make_run("r-live", "Running Run", RunStatus::Running, 2000),
        ]);

        // Waiting run is selected first even though Completed is newer
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Moving up at top clamps
        assert_eq!(state.handle_key(&key(KeyCode::Up)), LedgerOutcome::Unchanged);
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Moving down skips RUNNING header and lands on r-live
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('j'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-live")
        );

        // Moving down again skips FINISHED header and lands on r-done
        assert_eq!(
            state.handle_key(&key(KeyCode::Down)),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );

        // Clamps at bottom
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('j'))),
            LedgerOutcome::Unchanged
        );

        // Jump start and end
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('g'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('G'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );
    }

    #[test]
    fn stalled_running_run_is_placed_under_finished_with_stalled_at() {
        let mut state = LedgerState::new();
        let mut stalled_run = make_run("r-stalled", "Stalled Run", RunStatus::Running, 5000);
        stalled_run.current_node = Some("deploy_step".to_string());
        let done_run = make_run("r-done", "Older Done", RunStatus::Completed, 2000);
        state.set_runs(vec![done_run, stalled_run]);

        let mut extras = HashMap::new();
        extras.insert(
            "r-stalled".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: true,
                question: None,
                answers: None,
            },
        );
        state.set_extras(extras);

        // Stalled run is not considered live
        assert!(!state.any_live());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);
        let text = all_text(&buf);
        assert!(!text.contains("RUNNING 1"));
        assert!(text.contains("FINISHED 2"));
        assert!(text.contains("⊘ Stalled Run"));
        assert!(text.contains("stalled at deploy_step"));
    }

    #[test]
    fn tab_switching_keeps_per_tab_selection_and_selected_run_behavior() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "First Run", RunStatus::Completed, 2000),
            make_run("r2", "Second Run", RunStatus::Completed, 1000),
        ]);
        state.set_sessions(vec![
            make_session("s1", "First Session", false, 2000),
            make_session("s2", "Second Session", false, 1000),
        ]);

        // Select second run on Flows tab
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Switch to Sessions tab
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), LedgerOutcome::Changed);
        assert_eq!(state.tab(), LedgerTab::Sessions);
        assert!(state.selected_run().is_none());

        // Move to second session
        state.handle_key(&key(KeyCode::Down));

        // Switch back to Flows tab: r2 is still selected
        state.handle_key(&key(KeyCode::Char('h')));
        assert_eq!(state.tab(), LedgerTab::Flows);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Switch back to Sessions tab: s2 is still selected
        state.handle_key(&key(KeyCode::Right));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s2", "Second Session", false, 1000))
        );
    }

    #[test]
    fn filter_typing_enter_and_esc_ordering() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Alpha deploy", RunStatus::Completed, 2000),
            make_run("r2", "Beta test", RunStatus::Completed, 1000),
        ]);

        assert!(state.esc_closes());
        assert!(!state.captures_text());

        // Open filter with `/`
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('/'))),
            LedgerOutcome::Changed
        );
        assert!(state.captures_text());
        assert!(!state.esc_closes());

        // Type "beta" (note: 'a' would toggle active-only if filter weren't capturing)
        for ch in ['b', 'e', 't', 'a', 'x'] {
            state.handle_key(&key(KeyCode::Char(ch)));
        }
        state.handle_key(&key(KeyCode::Backspace));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Enter closes input but keeps filter
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::Changed
        );
        assert!(!state.captures_text());
        assert!(!state.esc_closes());
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // First Esc clears the non-empty filter (Changed)
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), LedgerOutcome::Changed);
        assert!(state.esc_closes());
        // Clearing the filter keeps the selected run selected.
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Second Esc returns Close
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), LedgerOutcome::Close);
    }

    #[test]
    fn active_only_toggle_filters_runs_and_sessions() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-done", "Done", RunStatus::Completed, 3000),
            make_run("r-live", "Live", RunStatus::Running, 1000),
        ]);
        state.set_sessions(vec![
            make_session("s-closed", "Closed Sess", true, 3000),
            make_session("s-open", "Open Sess", false, 1000),
        ]);

        // Toggle active-only on Flows
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('a'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-live")
        );
        // Moving down clamps because r-done is filtered out
        assert_eq!(
            state.handle_key(&key(KeyCode::Down)),
            LedgerOutcome::Unchanged
        );

        // Switch to Sessions and toggle active-only
        state.handle_key(&key(KeyCode::Tab));
        state.handle_key(&key(KeyCode::Char('a')));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s-open", "Open Sess", false, 1000))
        );
    }

    #[test]
    fn order_cycling_on_flows_and_sessions() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait", "Zeta Waiting", RunStatus::Waiting, 1000),
            make_run("r-done", "Alpha Done", RunStatus::Completed, 3000),
        ]);
        state.set_sessions(vec![
            make_session("s1", "Zeta Session", false, 3000),
            make_session("s2", "Alpha Session", false, 1000),
        ]);

        // Grouped: Waiting first
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Cycle to Recent: r-done (3000) is above r-wait (1000), jump_start selects r-done
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );

        // Cycle to Name: Alpha Done first
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );

        // Cycle back to Grouped: Waiting first
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Sessions tab cycles Recent -> Name -> Recent
        state.handle_key(&key(KeyCode::Tab));
        state.handle_key(&key(KeyCode::Char('s'))); // Name
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s2", "Alpha Session", false, 1000))
        );
    }

    #[test]
    fn enter_y_and_rescan_outcomes_on_both_tabs() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run("r1", "Run 1", RunStatus::Completed, 1000)]);
        state.set_sessions(vec![make_session("s1", "Sess 1", false, 1000)]);

        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenRun(PathBuf::from("/tmp/runs/r1"))
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("/tmp/runs/r1".to_string())
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('R'))),
            LedgerOutcome::Rescan
        );

        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s1", "Sess 1", false, 1000))
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("s1".to_string())
        );
    }

    #[test]
    fn set_runs_preserves_selected_run_id_and_any_live() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Run 1", RunStatus::Completed, 2000),
            make_run("r2", "Run 2", RunStatus::Completed, 1000),
        ]);
        assert!(!state.any_live());

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Refresh runs with a new live run at the top; r2 stays selected
        state.set_runs(vec![
            make_run("r3", "Run 3", RunStatus::Running, 3000),
            make_run("r1", "Run 1", RunStatus::Completed, 2000),
            make_run("r2", "Run 2", RunStatus::Completed, 1000),
        ]);
        assert!(state.any_live());
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
    }

    #[test]
    fn render_at_120x14_shows_tabs_needs_you_question_rail_and_count() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait", "Release pipeline", RunStatus::Waiting, 9_000),
            make_run("r-run", "Nightly bench", RunStatus::Running, 8_000),
            make_run("r-done", "Unit suite", RunStatus::Completed, 5_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Failed),
                    NodeState::Waiting,
                    NodeState::NotRun,
                ],
                agents: "grok-build · grok-4".to_string(),
                stalled: false,
                question: Some("Ship to prod?".to_string()),
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);

        let r0 = row_text(&buf, 0);
        assert!(r0.contains("Flows 3"));
        assert!(r0.contains("Sessions 0"));
        assert!(r0.contains("1 needs you · 1 running"));
        assert!(r0.contains("[grouped]"));

        let full = all_text(&buf);
        assert!(full.contains("NEEDS YOU 1"));
        assert!(full.contains("Release pipeline"));
        assert!(full.contains("✓✓✓✗◐○ 4/6"));
        assert!(full.contains("Ship to prod?"));
        assert!(full.contains("grok-build · grok-4"));
    }

    #[test]
    fn answered_gate_row_count_includes_the_answered_gate_and_matches_the_rail() {
        let rail = vec![
            NodeState::Done(Outcome::Ok),
            NodeState::Done(Outcome::Ok),
            NodeState::Done(Outcome::Ok),
            NodeState::Waiting,
            NodeState::NotRun,
            NodeState::NotRun,
        ];
        let render = |answered: bool| {
            let mut state = LedgerState::new();
            state.set_runs(vec![
                make_run("r-wait", "Ship flow", RunStatus::Waiting, 9_000),
                make_run("r-next", "Fix flow", RunStatus::Completed, 8_000),
            ]);
            let mut extras = HashMap::new();
            extras.insert(
                "r-wait".to_string(),
                RowExtra {
                    rail: rail.clone(),
                    agents: String::new(),
                    stalled: false,
                    question: Some("Ship it?".to_string()),
                    answers: None,
                },
            );
            if answered {
                extras.insert(
                    "r-next".to_string(),
                    RowExtra {
                        rail: vec![],
                        agents: String::new(),
                        stalled: false,
                        question: None,
                        answers: Some(GateAnswer {
                            run_id: "r-wait".to_string(),
                            choice: "Accept".to_string(),
                        }),
                    },
                );
            }
            state.set_extras(extras);
            let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
            let area_ = buf.area;
            state.render(&mut buf, area_, &Theme::default(), 10_000);
            all_text(&buf)
        };

        let answered = render(true);
        assert!(answered.contains("✓✓✓✓○○ 4/6"), "answered row: {answered}");
        assert!(!answered.contains("3/6"));

        let waiting = render(false);
        assert!(waiting.contains("✓✓✓◐○○ 3/6"), "waiting row: {waiting}");
    }

    #[test]
    fn render_at_50x8_keeps_title_and_does_not_panic() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run(
            "r-wait",
            "Compact Title Check",
            RunStatus::Waiting,
            9_000,
        )]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Done(Outcome::Ok), NodeState::Waiting],
                agents: "grok-agent".to_string(),
                stalled: false,
                question: Some("Approve?".to_string()),
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 8));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);

        let full = all_text(&buf);
        assert!(full.contains("Compact Title"));
        // Below 80 columns, <done>/<total> is dropped
        assert!(!full.contains("1/2"));
    }

    #[test]
    fn render_at_0x0_and_empty_state_do_not_panic() {
        let mut state = LedgerState::new();
        let theme = Theme::default();

        let mut zero_buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        let area_ = zero_buf.area;
        state.render(&mut zero_buf, area_, &theme, 10_000);

        let mut empty_buf = Buffer::empty(Rect::new(0, 0, 80, 10));
        let area_ = empty_buf.area;
        state.render(&mut empty_buf, area_, &theme, 10_000);
        let full = all_text(&empty_buf);
        assert!(full.contains("No acpx flow runs found"));
    }

    #[test]
    fn answered_gate_counts_only_unanswered_and_moves_answered_to_finished() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-old-wait", "Old Gate", RunStatus::Waiting, 1_000),
            make_run("r-new-wait", "New Gate", RunStatus::Waiting, 2_000),
            make_run("r-next", "Next Flow", RunStatus::Completed, 3_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-old-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Continue old?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-new-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Continue new?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-next".to_string(),
            RowExtra {
                rail: vec![NodeState::Done(Outcome::Ok)],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-old-wait".to_string(),
                    choice: "Approve".to_string(),
                }),
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 14));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let full = all_text(&buf);
        assert!(full.contains("1 needs you"));
        assert!(full.contains("NEEDS YOU 1"));
        assert!(full.contains("FINISHED 2"));
        let fin_pos = full.find("FINISHED 2").unwrap_or(0);
        let old_pos = full.find("Old Gate").unwrap_or(0);
        assert!(old_pos > fin_pos);
    }

    #[test]
    fn unanswered_waiting_run_remains_in_needs_you_and_reverts_when_answer_removed() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait", "Gate Run", RunStatus::Waiting, 1_000),
            make_run("r-ans", "Answering Run", RunStatus::Failed, 2_000),
        ]);
        let mut extras_unanswered = HashMap::new();
        extras_unanswered.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Proceed?".to_string()),
                answers: None,
            },
        );
        state.set_extras(extras_unanswered.clone());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 12));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let full = all_text(&buf);
        assert!(full.contains("1 needs you"));
        assert!(full.contains("NEEDS YOU 1"));

        let mut extras_answered = extras_unanswered.clone();
        extras_answered.insert(
            "r-ans".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-wait".to_string(),
                    choice: "Yes".to_string(),
                }),
            },
        );
        state.set_extras(extras_answered);
        let mut buf2 = Buffer::empty(Rect::new(0, 0, 100, 12));
        state.render(&mut buf2, area, &theme, 10_000);
        assert!(!all_text(&buf2).contains("needs you"));

        // Replace extras without the answering run: counts as needing you again
        state.set_extras(extras_unanswered);
        let mut buf3 = Buffer::empty(Rect::new(0, 0, 100, 12));
        state.render(&mut buf3, area, &theme, 10_000);
        let full3 = all_text(&buf3);
        assert!(full3.contains("1 needs you"));
        assert!(full3.contains("NEEDS YOU 1"));
    }

    #[test]
    fn set_extras_before_set_runs_matches_reverse_and_latest_started_wins() {
        let runs = vec![
            make_run("r-wait", "Waiting Gate", RunStatus::Waiting, 1_000),
            make_run("r-ans-1", "Answer 1", RunStatus::Completed, 2_000),
            make_run("r-ans-2", "Answer 2", RunStatus::Completed, 5_000),
        ];
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Which?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-ans-1".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-wait".to_string(),
                    choice: "OldChoice".to_string(),
                }),
            },
        );
        extras.insert(
            "r-ans-2".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-wait".to_string(),
                    choice: "NewChoice".to_string(),
                }),
            },
        );

        let mut s1 = LedgerState::new();
        s1.set_extras(extras.clone());
        s1.set_runs(runs.clone());

        let mut s2 = LedgerState::new();
        s2.set_runs(runs);
        s2.set_extras(extras);

        assert_eq!(s1.answered_choice("r-wait"), Some("NewChoice"));
        assert_eq!(s2.answered_choice("r-wait"), Some("NewChoice"));
        assert_eq!(s1.answered_choice("r-ans-1"), None);
        assert_eq!(s1.answered_choice("nonexistent"), None);

        let theme = Theme::default();
        let mut b1 = Buffer::empty(Rect::new(0, 0, 100, 12));
        let mut b2 = Buffer::empty(Rect::new(0, 0, 100, 12));
        let area = b1.area;
        s1.render(&mut b1, area, &theme, 10_000);
        s2.render(&mut b2, area, &theme, 10_000);
        let t1 = all_text(&b1);
        assert_eq!(t1, all_text(&b2));
        assert!(t1.contains("answered: NewChoice"));
        assert!(!t1.contains("Which?"));
    }

    #[test]
    fn selected_unanswered_waiting_run_wraps_200_char_question_and_unselected_stays_one_row() {
        let words: Vec<String> = (1..=28).map(|i| format!("word{i:02}")).collect();
        let question = format!("{} tailend", words.join(" "));
        assert_eq!(question.len(), 203);

        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait-1", "First Gate", RunStatus::Waiting, 2_000),
            make_run("r-wait-2", "Second Gate", RunStatus::Waiting, 1_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait-1".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some(question.clone()),
                answers: None,
            },
        );
        extras.insert(
            "r-wait-2".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Short?".to_string()),
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let text = all_text(&buf);
        for w in question.split_whitespace() {
            assert!(text.contains(w), "missing word {w} in:\n{text}");
        }

        // Move selection to r-wait-2 so r-wait-1 is unselected and takes a single row (row 2)
        state.handle_key(&key(KeyCode::Down));
        let mut buf2 = Buffer::empty(Rect::new(0, 0, 100, 20));
        state.render(&mut buf2, area, &theme, 10_000);
        assert!(row_text(&buf2, 2).contains("First Gate"));
        assert!(row_text(&buf2, 3).contains("Second Gate"));
    }
}

````

### `crates/codegen/xai-grok-pager/src/views/side_panel/flows.rs` (complete file, repository grok-build-acpx)
````
//! Flows page: the step rail of the featured acpx flow run, then recent runs. Enter opens one in the `/acpx` modal.

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{Ctx, Row, note_rows};
use crate::acpx::bundle::{self, NodeState, Outcome, Run, RunStatus, RunSummary, Step, fmt_duration, value_text};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

/// Prefix of `Row::open` for a row that opens a run in the /acpx modal: "acpx-run:<absolute run dir>".
pub(crate) const OPEN_PREFIX: &str = "acpx-run:";

const LIMIT: usize = 30;

#[derive(Default)]
pub(super) struct Flows {
    runs: Vec<RunSummary>,
    featured: Option<Run>,
}

impl Flows {
    /// The 1 s tick while this page is showing. True when the rows changed.
    pub(super) fn poll(&mut self, now_ms: i64) -> bool {
        match bundle::runs_base() {
            Some(base) => self.poll_at(&base, now_ms),
            None => {
                let changed = !self.runs.is_empty() || self.featured.is_some();
                self.runs.clear();
                self.featured = None;
                changed
            }
        }
    }

    fn poll_at(&mut self, base: &Path, _now_ms: i64) -> bool {
        let mut runs = bundle::list_runs(base);
        runs.truncate(LIMIT);

        let mut changed = self.runs != runs;
        self.runs = runs;

        let target = pick_featured(&self.runs);
        match target {
            None => {
                if self.featured.is_some() {
                    self.featured = None;
                    changed = true;
                }
            }
            Some(summary) => {
                let same_run = self.featured.as_ref().is_some_and(|r| r.summary.dir == summary.dir);
                if same_run {
                    if let Some(run) = &mut self.featured {
                        let was_running = run.summary.status == RunStatus::Running;
                        let summary_changed = run.summary != *summary;
                        if was_running || summary_changed {
                            if bundle::refresh(run) {
                                changed = true;
                            } else if summary_changed {
                                run.summary = summary.clone();
                                changed = true;
                            }
                        }
                    }
                } else {
                    match bundle::load_run(&summary.dir) {
                        Ok(run) => {
                            self.featured = Some(run);
                            changed = true;
                        }
                        Err(_) => {
                            if self.featured.is_some() {
                                self.featured = None;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        changed
    }

    pub(super) fn rows(&self, c: &Ctx) -> Vec<Row> {
        if self.runs.is_empty() {
            return note_rows("no acpx flow runs yet", c);
        }
        let mut out = Vec::new();
        let featured_dir = match (&self.featured, pick_featured(&self.runs)) {
            (Some(run), _) => {
                push_featured_run(&mut out, run, c);
                Some(run.summary.dir.as_path())
            }
            (None, Some(summary)) => {
                push_featured_summary(&mut out, summary, c);
                Some(summary.dir.as_path())
            }
            (None, None) => None,
        };

        let others: Vec<&RunSummary> =
            self.runs.iter().filter(|r| Some(r.dir.as_path()) != featured_dir).collect();

        if !others.is_empty() {
            out.push(Row::plain(Line::raw("")));
            let muted = Style::default().fg(c.theme.text_secondary);
            out.push(Row::plain(Line::from(Span::styled(truncate_str("recent", c.width), muted))));
            for r in others {
                out.push(recent_row(r, c));
            }
        }

        out
    }
}

fn pick_featured(runs: &[RunSummary]) -> Option<&RunSummary> {
    runs.iter()
        .find(|r| r.status == RunStatus::Running)
        .or_else(|| runs.iter().find(|r| r.status == RunStatus::Waiting))
        .or_else(|| runs.first())
}

fn open_target(dir: &Path) -> String {
    format!("{OPEN_PREFIX}{}", dir.display())
}

fn status_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.accent_success,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray_dim,
    }
}

fn node_state_color(state: NodeState, theme: &Theme) -> Color {
    match state {
        NodeState::Running => theme.accent_running,
        NodeState::Waiting => theme.warning,
        NodeState::Done(Outcome::Ok) => theme.accent_success,
        NodeState::Done(_) => theme.accent_error,
        NodeState::NotRun => theme.gray_dim,
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_featured_run(out: &mut Vec<Row>, run: &Run, c: &Ctx) {
    let s = &run.summary;
    let target = open_target(&s.dir);
    let muted = Style::default().fg(c.theme.text_secondary);

    out.push(title_row(s, c, &target));

    let (done, total) = run.progress();
    let dur = fmt_duration(run.duration_ms(c.now_ms));
    let age = fmt_age(c.now_ms.saturating_sub(s.updated_ms));
    let meta = format!("{} · {done}/{total} · {dur} · {age}", s.flow_name);
    out.push(
        Row::new(Line::from(Span::styled(truncate_str(&meta, c.width), muted)), meta).opens(target.clone()),
    );

    push_status_rows(out, s, c, &target);

    for node in &run.flow.nodes {
        let state = run.node_state(&node.id);
        let last_step = run.steps.iter().rev().find(|step| step.node_id == node.id);
        out.push(step_rail_row(&node.id, state, last_step, c, &target));
    }
}

fn push_featured_summary(out: &mut Vec<Row>, s: &RunSummary, c: &Ctx) {
    let target = open_target(&s.dir);
    let muted = Style::default().fg(c.theme.text_secondary);

    out.push(title_row(s, c, &target));

    let end = s.finished_ms.unwrap_or(if s.status.is_live() { c.now_ms } else { s.updated_ms });
    let dur_ms = u64::try_from(end.saturating_sub(s.started_ms).max(0)).unwrap_or(0);
    let dur = fmt_duration(dur_ms);
    let age = fmt_age(c.now_ms.saturating_sub(s.updated_ms));
    let meta = format!("{} · 0/0 · {dur} · {age}", s.flow_name);
    out.push(
        Row::new(Line::from(Span::styled(truncate_str(&meta, c.width), muted)), meta).opens(target.clone()),
    );

    push_status_rows(out, s, c, &target);
}

fn title_row(s: &RunSummary, c: &Ctx, target: &str) -> Row {
    let glyph = s.status.glyph();
    let title = one_line(&s.title);
    let full = format!("{glyph} {title}");
    let glyph_style = Style::default().fg(status_color(s.status, c.theme)).add_modifier(Modifier::BOLD);
    let title_style = Style::default().fg(c.theme.text_primary).add_modifier(Modifier::BOLD);

    let prefix = format!("{glyph} ");
    let prefix_w = prefix.width();
    let line = if c.width <= prefix_w {
        Line::from(Span::styled(truncate_str(&full, c.width), glyph_style))
    } else {
        let cut_title = truncate_str(&title, c.width.saturating_sub(prefix_w));
        Line::from(vec![Span::styled(prefix, glyph_style), Span::styled(cut_title, title_style)])
    };
    Row::new(line, full).opens(target.to_string())
}

fn push_status_rows(out: &mut Vec<Row>, s: &RunSummary, c: &Ctx, target: &str) {
    let muted = Style::default().fg(c.theme.text_secondary);
    match s.status {
        RunStatus::Waiting => {
            let node = s
                .waiting_on
                .as_deref()
                .or(s.current_node.as_deref())
                .filter(|n| !n.is_empty())
                .unwrap_or("checkpoint");
            let wait_text = format!("◐ waiting at {node}");
            let wait_style = Style::default().fg(c.theme.warning);
            out.push(
                Row::new(
                    Line::from(Span::styled(truncate_str(&wait_text, c.width), wait_style)),
                    wait_text,
                )
                .opens(target.to_string()),
            );
            if let Some(detail) = s.status_detail.as_deref().map(one_line).filter(|d| !d.is_empty()) {
                out.push(
                    Row::new(Line::from(Span::styled(truncate_str(&detail, c.width), muted)), detail)
                        .opens(target.to_string()),
                );
            }
        }
        RunStatus::Failed | RunStatus::TimedOut => {
            let err = s
                .error
                .as_deref()
                .or(s.status_detail.as_deref())
                .map(one_line)
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| s.status.label().to_string());
            let err_text = format!("✗ {err}");
            let err_style = Style::default().fg(c.theme.accent_error);
            out.push(
                Row::new(Line::from(Span::styled(truncate_str(&err_text, c.width), err_style)), err_text)
                    .opens(target.to_string()),
            );
        }
        _ => {}
    }
}

fn step_rail_row(
    node_id: &str,
    state: NodeState,
    last_step: Option<&Step>,
    c: &Ctx,
    target: &str,
) -> Row {
    let glyph = state.glyph();
    let glyph_style = Style::default().fg(node_state_color(state, c.theme));
    let name_style = Style::default().fg(c.theme.text_primary);
    let muted = Style::default().fg(c.theme.text_secondary);

    let dur = last_step.map(|st| fmt_duration(st.duration_ms())).unwrap_or_default();
    let prefix = format!("  {glyph} ");
    let prefix_w = prefix.width();
    let dur_w = dur.width();

    let line = if dur.is_empty() {
        if c.width <= prefix_w {
            Line::from(Span::styled(truncate_str(&format!("{prefix}{node_id}"), c.width), glyph_style))
        } else {
            let name = truncate_str(node_id, c.width.saturating_sub(prefix_w));
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{glyph} "), glyph_style),
                Span::styled(name, name_style),
            ])
        }
    } else if c.width <= prefix_w + dur_w + 1 {
        let combined = format!("{prefix}{node_id} {dur}");
        Line::from(Span::styled(truncate_str(&combined, c.width), name_style))
    } else {
        let avail_name = c.width.saturating_sub(prefix_w + dur_w + 1);
        let name = truncate_str(node_id, avail_name);
        let pad = c.width.saturating_sub(prefix_w + name.width() + dur_w);
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(name, name_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(dur, muted),
        ])
    };

    let result = node_result_line(state, last_step);
    let copy = format!("{node_id}: {result}");
    Row::new(line, copy).opens(target.to_string())
}

fn node_result_line(state: NodeState, last_step: Option<&Step>) -> String {
    if let Some(step) = last_step {
        if let Some(err) = step.error.as_deref().map(one_line).filter(|s| !s.is_empty()) {
            return err;
        }
        let out = one_line(&value_text(&step.output));
        if !out.is_empty() {
            return out;
        }
        if let Some(raw) = step.raw_text.as_deref().map(one_line).filter(|s| !s.is_empty()) {
            return raw;
        }
    }
    match state {
        NodeState::NotRun => "not run".into(),
        NodeState::Running => "running".into(),
        NodeState::Waiting => "waiting".into(),
        NodeState::Done(Outcome::Ok) => "ok".into(),
        NodeState::Done(Outcome::Failed) => "failed".into(),
        NodeState::Done(Outcome::TimedOut) => "timed out".into(),
        NodeState::Done(Outcome::Cancelled) => "cancelled".into(),
        NodeState::Done(Outcome::Unknown) => "done".into(),
    }
}

fn recent_row(s: &RunSummary, c: &Ctx) -> Row {
    let glyph = s.status.glyph();
    let glyph_style = Style::default().fg(status_color(s.status, c.theme));
    let title_style = Style::default().fg(c.theme.text_primary);
    let muted = Style::default().fg(c.theme.text_secondary);

    let title = one_line(&s.title);
    let age = fmt_age(c.now_ms.saturating_sub(s.updated_ms));
    let prefix = format!("  {glyph} ");
    let prefix_w = prefix.width();
    let age_w = age.width();

    let line = if c.width <= prefix_w + age_w + 1 {
        let combined = format!("{prefix}{title} {age}");
        Line::from(Span::styled(truncate_str(&combined, c.width), title_style))
    } else {
        let avail_title = c.width.saturating_sub(prefix_w + age_w + 1);
        let cut_title = truncate_str(&title, avail_title);
        let pad = c.width.saturating_sub(prefix_w + cut_title.width() + age_w);
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(cut_title, title_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(age, muted),
        ])
    };

    let target = open_target(&s.dir);
    Row::new(line, s.dir.display().to_string()).opens(target)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::super::{Info, paint};
    use super::*;

    const NOW: i64 = 1_775_655_600_000; // 2026-04-08T13:40:00Z

    struct BundleSpec<'a> {
        id: &'a str,
        flow_name: &'a str,
        title: &'a str,
        status: &'a str,
        started_iso: &'a str,
        updated_iso: &'a str,
        finished_iso: Option<&'a str>,
        current_node: Option<&'a str>,
        waiting_on: Option<&'a str>,
        status_detail: Option<&'a str>,
        error: Option<&'a str>,
        nodes: Vec<(&'a str, &'a str)>,
        edges: Vec<(&'a str, &'a str)>,
        steps: Vec<Value>,
    }

    impl<'a> BundleSpec<'a> {
        fn new(id: &'a str, status: &'a str, started_iso: &'a str) -> Self {
            Self {
                id,
                flow_name: "pr-triage",
                title: id,
                status,
                started_iso,
                updated_iso: started_iso,
                finished_iso: None,
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
                nodes: vec![("extract", "action"), ("triage", "acp"), ("apply", "action")],
                edges: vec![("extract", "triage"), ("triage", "apply")],
                steps: Vec::new(),
            }
        }
    }

    fn write_bundle(base: &Path, spec: &BundleSpec<'_>) -> PathBuf {
        let dir = base.join(spec.id);
        let proj = dir.join("projections");
        fs::create_dir_all(&proj).expect("create projections dir");

        let mut nodes_map = serde_json::Map::new();
        for (nid, nkind) in &spec.nodes {
            nodes_map.insert((*nid).to_string(), json!({ "nodeType": nkind }));
        }
        let edges_vec: Vec<Value> =
            spec.edges.iter().map(|(from, to)| json!({ "from": from, "to": to })).collect();
        let start_at = spec.nodes.first().map(|(n, _)| *n).unwrap_or("start");
        let flow_json = json!({
            "name": spec.flow_name,
            "startAt": start_at,
            "nodes": nodes_map,
            "edges": edges_vec,
        });
        fs::write(dir.join("flow.json"), flow_json.to_string()).expect("write flow.json");

        let live_json = json!({
            "runId": spec.id,
            "flowName": spec.flow_name,
            "runTitle": spec.title,
            "status": spec.status,
            "startedAt": spec.started_iso,
            "updatedAt": spec.updated_iso,
            "finishedAt": spec.finished_iso,
            "currentNode": spec.current_node,
            "waitingOn": spec.waiting_on,
            "statusDetail": spec.status_detail,
            "error": spec.error,
        });
        fs::write(proj.join("live.json"), live_json.to_string()).expect("write live.json");
        fs::write(proj.join("steps.json"), Value::Array(spec.steps.clone()).to_string())
            .expect("write steps.json");

        dir
    }

    fn render_rows(flows: &Flows, width: usize) -> Vec<Row> {
        let info = Info::default();
        let theme = Theme::default();
        let ctx = Ctx { info: &info, theme: &theme, width, now_ms: NOW };
        flows.rows(&ctx)
    }

    #[test]
    fn empty_base_renders_note_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut flows = Flows::default();
        assert!(!flows.poll_at(tmp.path(), NOW));
        let rows = render_rows(&flows, 36);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].copy, "no acpx flow runs yet");
        assert!(rows[0].open.is_none());
    }

    #[test]
    fn featured_prefers_running_over_waiting_over_newest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut completed = BundleSpec::new("run-newest", "completed", "2026-04-08T13:39:00Z");
        completed.title = "Newest Completed";
        write_bundle(tmp.path(), &completed);

        let mut waiting = BundleSpec::new("run-waiting", "waiting", "2026-04-08T13:35:00Z");
        waiting.title = "Older Waiting";
        waiting.waiting_on = Some("triage");
        let waiting_dir = write_bundle(tmp.path(), &waiting);

        let mut flows = Flows::default();
        assert!(flows.poll_at(tmp.path(), NOW));
        let rows = render_rows(&flows, 40);
        assert!(rows[0].copy.contains("Older Waiting"), "waiting beats newer completed");
        assert_eq!(rows[0].open.as_deref(), Some(open_target(&waiting_dir).as_str()));

        let mut running = BundleSpec::new("run-running", "running", "2026-04-08T13:30:00Z");
        running.title = "Oldest Running";
        running.current_node = Some("extract");
        let running_dir = write_bundle(tmp.path(), &running);

        assert!(flows.poll_at(tmp.path(), NOW));
        let rows = render_rows(&flows, 40);
        assert!(rows[0].copy.contains("Oldest Running"), "running beats waiting and completed");
        assert_eq!(rows[0].open.as_deref(), Some(open_target(&running_dir).as_str()));
    }

    #[test]
    fn step_rail_orders_nodes_and_renders_state_glyphs_and_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-1", "running", "2026-04-08T13:38:00Z");
        spec.title = "Triage PR #42";
        spec.current_node = Some("triage");
        spec.steps = vec![json!({
            "attemptId": "extract#1",
            "nodeId": "extract",
            "nodeType": "action",
            "outcome": "ok",
            "startedAt": "2026-04-08T13:38:00.000Z",
            "finishedAt": "2026-04-08T13:38:03.900Z",
            "output": "extracted\n  diff   summary",
        })];
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 42);
        let drawn = paint(&rows, 42);
        let lines: Vec<&str> = drawn.lines().collect();

        assert!(lines[0].contains("● Triage PR #42"));
        assert!(lines[1].contains("pr-triage · 1/3 ·"));
        assert!(lines[2].contains("✓ extract") && lines[2].ends_with("3.9s"));
        assert!(lines[3].contains("● triage"));
        assert!(lines[4].contains("○ apply"));

        assert_eq!(rows[2].copy, "extract: extracted diff summary");
        assert_eq!(rows[3].copy, "triage: running");
        assert_eq!(rows[4].copy, "apply: not run");
        for row in &rows[..5] {
            assert_eq!(row.open.as_deref(), Some(open_target(&dir).as_str()));
        }
    }

    #[test]
    fn waiting_run_includes_waiting_node_and_status_detail_rows() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-wait", "waiting", "2026-04-08T13:30:00Z");
        spec.title = "Deploy Staging";
        spec.waiting_on = Some("approve");
        spec.status_detail = Some("awaiting human approval on slack");
        spec.nodes = vec![("build", "action"), ("approve", "checkpoint")];
        spec.edges = vec![("build", "approve")];
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 44);
        assert_eq!(rows[2].copy, "◐ waiting at approve");
        assert_eq!(rows[3].copy, "awaiting human approval on slack");
        assert_eq!(rows[2].open.as_deref(), Some(open_target(&dir).as_str()));
        assert_eq!(rows[3].open.as_deref(), Some(open_target(&dir).as_str()));
    }

    #[test]
    fn failed_run_includes_error_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-fail", "failed", "2026-04-08T13:30:00Z");
        spec.title = "Nightly Bench";
        spec.error = Some("exit code 101:\ncompilation failed");
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 44);
        assert_eq!(rows[2].copy, "✗ exit code 101: compilation failed");
        assert_eq!(rows[2].open.as_deref(), Some(open_target(&dir).as_str()));
    }

    #[test]
    fn recent_list_excludes_featured_and_opens_each_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut r1 = BundleSpec::new("run-1", "running", "2026-04-08T13:35:00Z");
        r1.title = "Featured Run";
        let d1 = write_bundle(tmp.path(), &r1);

        let mut r2 = BundleSpec::new("run-2", "completed", "2026-04-08T13:38:00Z");
        r2.title = "Second Run";
        let d2 = write_bundle(tmp.path(), &r2);

        let mut r3 = BundleSpec::new("run-3", "failed", "2026-04-08T12:00:00Z");
        r3.title = "Third Run";
        let d3 = write_bundle(tmp.path(), &r3);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 40);

        let recent_idx = rows.iter().position(|r| r.copy == "recent").expect("recent header");
        assert_eq!(rows[recent_idx - 1].copy, "");
        let recent_rows = &rows[recent_idx + 1..];
        assert_eq!(recent_rows.len(), 2);

        assert_eq!(recent_rows[0].copy, d2.display().to_string());
        assert_eq!(recent_rows[0].open.as_deref(), Some(open_target(&d2).as_str()));
        assert_eq!(recent_rows[1].copy, d3.display().to_string());
        assert_eq!(recent_rows[1].open.as_deref(), Some(open_target(&d3).as_str()));
        assert!(recent_rows.iter().all(|r| r.open.as_deref() != Some(open_target(&d1).as_str())));
    }

    #[test]
    fn every_row_fits_within_28_columns() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut r1 = BundleSpec::new("run-long-featured", "waiting", "2026-04-08T13:35:00Z");
        r1.title = "A very long featured run title that exceeds twenty-eight columns easily";
        r1.waiting_on = Some("extremely_long_checkpoint_node_identifier");
        r1.status_detail = Some("waiting for a very verbose status detail explanation string");
        r1.nodes = vec![
            ("very_long_first_step_node_name", "action"),
            ("extremely_long_checkpoint_node_identifier", "checkpoint"),
        ];
        r1.edges = vec![("very_long_first_step_node_name", "extremely_long_checkpoint_node_identifier")];
        r1.steps = vec![json!({
            "attemptId": "very_long_first_step_node_name#1",
            "nodeId": "very_long_first_step_node_name",
            "nodeType": "action",
            "outcome": "ok",
            "startedAt": "2026-04-08T13:35:00.000Z",
            "finishedAt": "2026-04-08T13:37:05.000Z",
        })];
        write_bundle(tmp.path(), &r1);

        let mut r2 = BundleSpec::new("run-long-other", "completed", "2026-04-08T11:00:00Z");
        r2.title = "Another super long run title in the recent list section";
        write_bundle(tmp.path(), &r2);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 28);
        for row in &rows {
            let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                text.width() <= 28,
                "row width {} exceeds 28: {:?}",
                text.width(),
                text
            );
        }
        let painted = paint(&rows, 28);
        for line in painted.lines() {
            assert!(line.width() <= 28);
        }
    }

    #[test]
    fn poll_at_tracks_changes_and_refreshes_running_bundle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-live", "running", "2026-04-08T13:35:00Z");
        spec.current_node = Some("extract");
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        assert!(flows.poll_at(tmp.path(), NOW), "first load is a change");
        assert!(!flows.poll_at(tmp.path(), NOW), "unchanged files report false");

        // Update live.json to complete the run.
        spec.status = "completed";
        spec.updated_iso = "2026-04-08T13:36:00Z";
        spec.finished_iso = Some("2026-04-08T13:36:00Z");
        spec.current_node = None;
        let _ = dir;
        write_bundle(tmp.path(), &spec);

        assert!(flows.poll_at(tmp.path(), NOW), "updated live.json reports true");
        assert!(!flows.poll_at(tmp.path(), NOW), "subsequent poll on completed run is false");
    }

    #[test]
    fn open_targets_always_start_with_open_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r1 = BundleSpec::new("run-a", "running", "2026-04-08T13:35:00Z");
        write_bundle(tmp.path(), &r1);
        let r2 = BundleSpec::new("run-b", "completed", "2026-04-08T13:30:00Z");
        write_bundle(tmp.path(), &r2);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 36);
        let openables: Vec<&str> = rows.iter().filter_map(|r| r.open.as_deref()).collect();
        assert!(!openables.is_empty());
        for target in openables {
            assert!(
                target.starts_with(OPEN_PREFIX),
                "expected {target:?} to start with {OPEN_PREFIX:?}"
            );
        }
    }
}

````

### `crates/codegen/xai-grok-pager/src/app/dispatch/acpx_run.rs` (complete file, repository grok-build-acpx)
````
//! `/acpx run`: one flow per agent view, started as a child process and followed in a transcript card.
//! acpx prints nothing until the run ends, so the card is fed from the run bundle on the one-second tick.

use std::path::PathBuf;
use std::time::Instant;

use crate::acpx::bundle::{self, NodeState, Outcome, Run, RunStatus};
use crate::acpx::launch::{Launch, LaunchSpec, LaunchState};
use crate::app::app_view::AppView;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::{AcpxFlowBlock, AcpxFlowStatus};
use crate::scrollback::entry::EntryId;

pub(crate) struct AcpxRunning {
    launch: Launch,
    entry: EntryId,
    started: Instant,
    run: Option<Run>,
}

pub(crate) fn start(app: &mut AppView, flow_file: PathBuf, input_json: Option<String>) {
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let mut block = AcpxFlowBlock::started(flow_file.display().to_string());
    let launch = if agent.acpx_run.is_some() {
        Err("a flow is already running in this view; /acpx cancel stops it".to_string())
    } else {
        match bundle::runs_base() {
            None => Err("HOME is not set, so acpx has no runs directory".to_string()),
            Some(base) => {
                let spec = LaunchSpec {
                    flow_file,
                    input_json,
                    default_agent: None,
                    // Nothing can answer a permission prompt from a child with no terminal.
                    approve_all: true,
                    cwd: Some(PathBuf::from(&agent.session.cwd)),
                    acpx_bin: None,
                };
                Launch::start(&spec, &base).map_err(|e| format!("could not start acpx: {e}"))
            }
        }
    };
    match launch {
        Ok(launch) => {
            let entry = agent.scrollback.push_block(RenderBlock::AcpxFlow(block));
            agent.scrollback.set_entry_running(entry, true);
            agent.acpx_run = Some(AcpxRunning { launch, entry, started: Instant::now(), run: None });
        }
        Err(message) => {
            block.set_status(AcpxFlowStatus::Failed(message));
            agent.scrollback.push_block(RenderBlock::AcpxFlow(block));
        }
    }
}

pub(crate) fn cancel(app: &mut AppView) {
    if let Some(running) = super::ctx::get_active_agent_mut(app).and_then(|agent| agent.acpx_run.as_mut()) {
        running.launch.cancel();
    }
}

pub(crate) fn any_running(app: &AppView) -> bool {
    app.agents.values().any(|agent| agent.acpx_run.is_some())
}

/// The one-second tick: every view's flow, not only the one on screen. True when a card changed.
pub(crate) fn tick(app: &mut AppView) -> bool {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut dirty = false;
    for agent in app.agents.values_mut() {
        let Some(running) = agent.acpx_run.as_mut() else { continue };
        let state = running.launch.poll();
        if let Some(dir) = running.launch.run_dir().map(|d| d.to_path_buf()) {
            match running.run.as_mut() {
                Some(run) => {
                    bundle::refresh(run);
                }
                None => running.run = bundle::load_run(&dir).ok(),
            }
        }
        let entry = running.entry;
        let done = !matches!(state, LaunchState::Starting | LaunchState::Running { .. });
        if let Some(item) = agent.scrollback.get_by_id_mut(entry)
            && let RenderBlock::AcpxFlow(block) = &mut item.block
        {
            match &running.run {
                Some(run) => block.update_from_run(run, now_ms),
                None => block.elapsed = running.started.elapsed(),
            }
            match state {
                LaunchState::Starting | LaunchState::Running { .. } => {}
                LaunchState::Cancelled => {
                    // The bundle still says the killed node is running.
                    for row in &mut block.rows {
                        if row.state == NodeState::Running {
                            row.state = NodeState::Done(Outcome::Cancelled);
                        }
                    }
                    block.set_status(AcpxFlowStatus::Cancelled);
                }
                LaunchState::Failed(message) => {
                    block.note = running.launch.stderr_tail().lines().last().map(str::to_string).filter(|l| !message.contains(l.as_str()));
                    block.set_status(AcpxFlowStatus::Failed(message));
                }
                // The bundle already set the status; the result line only matters when the bundle could not be read.
                LaunchState::Finished(result) if running.run.is_none() => block.set_status(match result.status {
                    RunStatus::Waiting => AcpxFlowStatus::Waiting {
                        node: result.waiting_on.unwrap_or_default(),
                        detail: result.status_detail.unwrap_or_default(),
                    },
                    RunStatus::Failed | RunStatus::TimedOut => AcpxFlowStatus::Failed(result.error.unwrap_or_else(|| "failed".into())),
                    _ => AcpxFlowStatus::Completed,
                }),
                LaunchState::Finished(_) => {}
            }
            item.invalidate_cache();
        }
        if done {
            agent.scrollback.finish_running(entry);
            agent.acpx_run = None;
        }
        dirty = true;
    }
    dirty
}

````

### `task-scripts/acpx-mode.py` (complete file, repository grok-build-acpx)
````
#!/usr/bin/env python3
"""
acpx-mode.py: slices for "acpx mode": a stdio ACP agent, `grok-acpx-acp`, that the stock pager runs like Grok or pi
(`--agent-cmd grok-acpx-acp` or an `[[agent_choice]]`). acpx is the engine; the pager's own transcript is the interface.

  user message / /flow  ->  agent.rs starts `acpx flow run`  ->  driver.rs follows the run folder live
  ->  project.rs maps each run event to ACP session updates  ->  the pager renders them with its stock blocks.

Each agent step of a flow is a subagent (one capped, live block in the parent transcript; Enter opens its full view).
Shell steps are execute tool calls. A checkpoint with choices is an `x.ai/ask_user_question`. The pager change
(`subagent_preview.rs`) gives the one-line subagent block a capped live tail of the step's transcript.

Pure text generation, no tools. Every slice owns ONE new file and returns the complete file. Wiring is done by hand.

  python3 task-scripts/acpx-mode.py --dry-run          sizes + estimate, no API call
  python3 task-scripts/acpx-mode.py --only driver      fire one slice
  python3 task-scripts/acpx-mode.py                    fire all

Files land in task-scripts/acpx-mode_out/<output_file>; raw returns in acpx-mode_out/_returns/<id>.md.
"""

MODEL = "google/gemini-4-argon"  # OpenRouter id. Override with --model
CTX = "task-scripts/ctx"
MCTX = "task-scripts/ctx/acpx-mode"
BIN = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp"
SRC = "crates/codegen/xai-grok-pager/src"
MAX_TOKENS = 64000

# ── Contract A: driver.rs (implemented by slice `driver`, used by `project` and `agent`) ─────────────────────────
DRIVER_API = """
// crate path: `crate::driver` (module of the binary grok-acpx-acp; main.rs declares `mod driver; mod project; mod agent;`)
use std::path::{Path, PathBuf};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind { Acp, Shell, Compute, Checkpoint, Other }   // flow.json node "nodeType": acp|action|compute|checkpoint. Shell = nodeType "action" with "actionExecution" "shell"; an action with actionExecution "function" is Compute; anything else Other

#[derive(Debug, Clone, PartialEq)]
pub struct SketchNode {
    pub id: String,
    pub kind: StepKind,
    pub summary: Option<String>,   // checkpoint question (flow.json node "summary")
    pub profile: Option<String>,   // acp: flow.json node "profile" (agent name, e.g. "grok-build")
    pub model: Option<String>,     // acp: flow.json node "modelId"
    pub effort: Option<String>,    // acp: flow.json node "configOptions" entry whose configId is "reasoning_effort" -> its "value"
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchEdge { pub from: String, pub to: String, pub label: Option<String> }   // a switch edge yields one SketchEdge per case, label = case key

#[derive(Debug, Clone, PartialEq)]
pub struct FlowSketch { pub name: String, pub start_at: String, pub nodes: Vec<SketchNode>, pub edges: Vec<SketchEdge> }

#[derive(Debug, Clone, PartialEq)]
pub struct GateChoice { pub label: String, pub flow_file: String }

#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    RunStarted { run_id: String, run_dir: PathBuf, title: String, flow_path: String, flow: FlowSketch, at_ms: i64 },
    StepStarted { node: String, attempt: String, kind: StepKind, at_ms: i64 },
    /// The prompt text acpx sent to the step's agent (outbound session/prompt, text blocks joined by "\\n").
    AgentPrompt { node: String, attempt: String, text: String, at_ms: i64 },
    /// One inbound `session/update` of the step's agent: `update` is the JSON object at message.params.update, verbatim.
    AgentUpdate { node: String, attempt: String, update: Value, at_ms: i64 },
    ShellPrepared { node: String, attempt: String, command_line: String, cwd: Option<String>, at_ms: i64 },
    StepFinished {
        node: String, attempt: String, kind: StepKind,
        outcome: String,              // "ok" | "failed" | "timed_out" | other acpx strings verbatim
        duration_ms: u64, error: Option<String>,
        output: Value,                // node_outcome payload.outputInline, or Value::Null
        stdout: Option<String>,       // shell: stdout artifact text (first 64 KiB, lossy UTF-8); None otherwise
        stderr: Option<String>,       // shell: stderr artifact text (first 16 KiB); None otherwise
        exit_code: Option<i64>,       // shell: payload.action.exitCode
        at_ms: i64,
    },
    /// A checkpoint the run now waits on. `question` = the flow node summary, else the node id.
    Waiting { node: String, attempt: String, question: String, choices: Vec<GateChoice>, at_ms: i64 },
    RunFinished { status: String, error: Option<String>, at_ms: i64 },   // completed|failed|timed_out|waiting (see the RunFinished rule)
}

#[derive(Debug, Clone)]
pub struct DriverConfig {
    pub acpx_bin: PathBuf,         // e.g. "acpx" (resolved on PATH) or an absolute path
    pub runs_dir: PathBuf,         // ~/.acpx/flows/runs
    pub cwd: PathBuf,              // working directory for `acpx flow run`
    pub extra_path: Vec<PathBuf>,  // prepended to PATH for the acpx child (e.g. ~/.grok/bin so `grok agent stdio` resolves)
    pub poll_ms: u64,              // tail interval, default 100
}

/// A live run started by `start`. Dropping it does not stop the run.
pub struct RunHandle { /* private fields */ }
impl RunHandle {
    pub fn cancel(&self);                       // SIGTERM to the acpx child pid (acpx then stops its agents/shells and writes run_failed); SIGKILL the process group 5 s later if it is still alive; idempotent
    pub fn run_dir(&self) -> Option<PathBuf>;   // known once RunStarted was sent
}

/// Write input_json to a temp file and spawn `acpx --approve-all --format json flow run <flow_file> --input-file <tmp>`
/// in cfg.cwd (a relative flow_file resolves against cfg.cwd), and follow
/// its run folder, sending every DriverEvent in order on `tx`. Must be called inside a tokio LocalSet (uses
/// tokio::task::spawn_local). The last event is always RunFinished (also when acpx exits early or fails to start,
/// with status "failed" and the error / stderr tail).
pub fn start(cfg: &DriverConfig, flow_file: &Path, input_json: &str,
             tx: tokio::sync::mpsc::UnboundedSender<DriverEvent>) -> anyhow::Result<RunHandle>;

/// Every DriverEvent of an existing run folder (finished, waiting or still running), synchronously, same order and
/// same content `start` would have produced. Used for replay (session/load, /open) and in tests.
pub fn replay(run_dir: &Path) -> anyhow::Result<Vec<DriverEvent>>;

/// Newest-first run folders: (run_id, flow name, title, status, started_ms). Reads manifest.json / projections/live.json.
pub fn list_runs(runs_dir: &Path, limit: usize) -> Vec<(String, String, String, String, i64)>;

pub use xai_grok_pager::acpx::bundle::{parse_ts, fmt_duration};   // re-export, do not reimplement:
// parse_ts("2026-11-20T01:14:34.620Z") -> Some(unix ms); fmt_duration(ms) -> "400ms", "27.1s", "2m14s", ...
"""

# ── Contract B: project.rs (implemented by slice `project`, used by `agent`) ──────────────────────────────────────
PROJECT_API = """
// crate path: `crate::project`
use agent_client_protocol as acp;
use serde_json::Value;
use crate::driver::{DriverEvent, FlowSketch};

/// One thing to send to the pager. The agent adds `_meta` (timing, promptId, isReplay, eventId) when it sends.
#[derive(Debug, Clone)]
pub enum Out {
    /// A standard `session/update` notification for `session_id` (the parent session or a step's child session).
    Update { session_id: String, update: acp::SessionUpdate },
    /// An xAI extension session update: sent as ext notification `x.ai/session_notification` (live) or
    /// `x.ai/session/update` (replay) with params `{ "sessionId": session_id, "update": update, "_meta": {...} }`.
    /// `update` is the JSON object with its "sessionUpdate" tag, e.g. subagent_spawned / subagent_finished.
    XaiUpdate { session_id: String, update: Value },
}

pub struct Projector { /* private */ }
impl Projector {
    /// `parent_session_id`: the pager session this run is shown in.
    pub fn new(parent_session_id: &str) -> Self;
    /// Map one driver event to the updates that show it. Deterministic: same events -> same Outs (ids derive from
    /// run id + attempt id, never from clocks or randomness).
    pub fn on_event(&mut self, ev: &DriverEvent) -> Vec<Out>;
    /// Child session id used for an acp step attempt ("acpx-step:<run_id>:<attempt>"), once StepStarted was seen.
    pub fn child_session_id(&self, attempt: &str) -> Option<String>;
    /// Final agent message text of an acp attempt (concatenated agent_message_chunk text), once seen.
    pub fn final_message(&self, attempt: &str) -> Option<String>;
}

/// The flow as a mermaid `flowchart LR` source (no fences). Node labels: "<glyph> <id>" where the glyph is
/// ✓ done, ✗ failed, ◐ waiting, ● running, ○ not reached; `states` maps node id -> glyph char (missing = ○).
pub fn mermaid(flow: &FlowSketch, states: &std::collections::HashMap<String, char>) -> String;
"""

# ── Contract C: subagent_preview.rs (pager crate, slice `preview`) ─────────────────────────────────────────────────
PREVIEW_API = """
// crate path: `crate::scrollback::blocks::subagent_preview` (crate xai-grok-pager)
use agent_client_protocol as acp;
use ratatui::text::Line;
use crate::theme::Theme;

/// A capped live tail of a subagent's transcript, fed with the same session updates the child view receives.
#[derive(Debug, Clone, Default)]
pub struct SubagentPreview { /* private */ }
impl SubagentPreview {
    pub fn new() -> Self;
    /// Fold one child session update into the preview. Handles AgentThoughtChunk, AgentMessageChunk,
    /// UserMessageChunk, ToolCall, ToolCallUpdate, Plan; ignores the rest. Never panics on any input.
    pub fn ingest(&mut self, update: &acp::SessionUpdate);
    /// The last `max_rows` display rows at `width` columns (wrapped, never wider than width), oldest first.
    pub fn render(&self, width: u16, max_rows: usize, theme: &Theme) -> Vec<Line<'static>>;
    pub fn is_empty(&self) -> bool;
    /// A cheap counter that changes whenever ingest changed what render would draw (for cache invalidation).
    pub fn revision(&self) -> u64;
}
"""

RULES = """
Hard rules: Rust 2024 edition, toolchain 1.94. No `unsafe`, except `libc::kill` calls in driver.rs under
`#[cfg(unix)]` (libc is available on unix only).
No `unwrap()` / `expect()` / indexing that can panic on anything read from disk, the network or the user.
`use agent_client_protocol as acp;` (version 0.10.4, feature "unstable"); only use acp types and builder methods that
appear in the context files. Crates available: agent_client_protocol, xai_acp_lib, xai_grok_pager (the pager library: only the
items named in these prompts), tokio (full), serde_json, anyhow, async_trait, tracing, libc (cfg(unix)), tempfile
(dev-dependency only). No serde derive, no chrono, nothing else.
Names in the "Public surface" block are fixed: other files are being written against them right now. You may add
private helpers and extra public items. Do not redefine types owned by another contract; import them by path.
Tests: a `#[cfg(test)] mod tests` at the bottom; no network, no real acpx, no real agent. File fixtures go in
`tempfile::tempdir()` (dev-dependency). Edition 2024: `gen` is reserved; `std::env::set_var` is unsafe (do not use).
Return the complete file in ONE ```rust block and nothing else.
"""

RUN_FOLDER = """
acpx run folder facts (verified on acpx 0.22; field names exact):
- `~/.acpx/flows/runs/<runId>/` holds manifest.json {runId, flowName, runTitle?, flowPath, startedAt, status}
  (runTitle is optional: title = manifest.runTitle, else live.json runTitle, else flowName),
  flow.json {name, startAt, nodes:{<id>:{nodeType, actionExecution?, profile?, modelId?,
  configOptions?:[{configId,value}], summary?}},
  edges:[{from,to} | {from, switch:{on, cases:{<label>:<to>}}}]}, trace.ndjson, projections/live.json
  {status, statusDetail, waitingOn, currentNode, updatedAt, finishedAt?, error?}, sessions/<sessionId>/events.ndjson,
  artifacts/. Skip a new run folder until its manifest.json exists and parses.
- acpx prints NOTHING until the run ends (then one JSON object with runId). The run folder appears within ~1 s.
  Discover it: snapshot the folder names in runs_dir before spawning; poll every poll_ms for a NEW folder whose
  manifest.json `flowPath` equals the canonicalized flow file path (compare canonicalized; macOS /tmp is /private/tmp).
- trace.ndjson lines: {"seq","at","runId","scope","type","nodeId"?,"attemptId"?,"sessionId"?,"payload"}. Types:
  run_started {flowName, runTitle, flowPath}; node_started {nodeType, statusDetail}; node_heartbeat (every 5 s,
  ignore); artifact_written (ignore); session_bound {sessionId, handle} (scope "session"; the session folder is
  sessions/<sessionId>/); acp_prompt_prepared {sessionId} (nodeId, attemptId set); acp_response_parsed;
  action_prepared {action:{actionType:"shell", command, args[], cwd}}; action_completed {action:{..., exitCode,
  durationMs}, stdoutArtifact:{path}, stderrArtifact:{path}} (paths relative to the run folder);
  node_outcome {nodeType, outcome, durationMs, error, outputInline?}. A checkpoint's node_outcome carries
  outputInline; when outputInline.next is an object {label: flowFile}, those are the gate choices (sorted by label).
  A waiting checkpoint is recorded as node_outcome outcome "ok" and then live.json status becomes "waiting" with
  waitingOn = node id; acpx then exits. A completed run ends with trace `run_completed {status}`; a failed one with
  `run_failed {status:"failed"|"timed_out", error}`; a waiting run has neither. A shell that exits non-zero fails the
  node (node_outcome outcome "failed", error set, possibly no action_completed). An action node with actionExecution
  "function" emits no action_prepared and its node_outcome payload.action.actionType is "function".
- sessions/<sessionId>/events.ndjson lines: {"seq","at","direction":"outbound"|"inbound","message":<JSON-RPC>}.
  A session (handle) can serve several steps. Attribution rule: the k-th outbound message with method
  "session/prompt" in a session file belongs to the k-th acp_prompt_prepared trace event with that sessionId; every
  inbound message with method "session/update" after that prompt and before the inbound response with the same
  JSON-RPC id (a message with direction "inbound", "result" or "error", that id, and seq after the prompt line)
  belongs to that attempt. JSON-RPC ids repeat across prompts, so only match the response AFTER its prompt. Lines may arrive in the
  session file before the matching trace line: buffer them until attribution is known.
  NEVER forward or log messages whose method starts with "_auth" (they carry the account email). Ignore every
  other method (fs/*, _x.ai/*, results) except session/prompt and session/update.
- All files are appended while the run is live; a last line without a trailing newline is a write in progress:
  keep the bytes and retry next poll. Skip lines that do not parse.
- Timestamps: "2026-11-20T01:14:34.620Z".
"""

TASKS = [
    {
        "custom_id": "driver",
        "output_file": f"{BIN}/driver.rs",
        "context_files": [],
        "prompt": f"""
Write `driver.rs` for the binary grok-acpx-acp: it runs acpx flows and turns a run folder into an ordered stream of
`DriverEvent`s, live (`start`) or after the fact (`replay`). No ACP types in this file.

Public surface (implement exactly):
{DRIVER_API}
{RUN_FOLDER}
Design:
- One private `Tailer` that consumes run-folder lines and emits DriverEvents; `start` feeds it incrementally from a
  polling task, `replay` feeds it everything at once, so both paths produce identical events. Each tick reads every session
  file BEFORE the trace file (acpx writes an agent's response before the trace's node_outcome). Trace-derived events
  are emitted in seq order. Session lines are buffered per session until attributed; AgentPrompt/AgentUpdate of an
  attempt are emitted after its StepStarted (and after acp_prompt_prepared). On node_outcome of an acp attempt, first
  flush every AgentUpdate already attributed to it, then emit StepFinished immediately; never wait for a response
  line (a failed or timed-out step has none). Later lines for a finished attempt are dropped.
- RunStarted comes first: built from manifest.json + flow.json (FlowSketch: nodes in BFS order from startAt,
  unreachable nodes appended in key order; edges in file order) and the run_started `at`.
- StepStarted on node_started. ShellPrepared on action_prepared (command_line = command + args, args containing
  spaces or quotes shell-quoted with single quotes). StepFinished on node_outcome (kind from payload.nodeType;
  for shell, read stdout from the action_completed stdoutArtifact, first 64 KiB, path must stay inside the run
  folder: reject absolute paths and `..`; same for stderr, first 16 KiB). A Compute/Shell StepFinished may arrive
  without a prior ShellPrepared. Waiting right after the StepFinished of a checkpoint when outputInline.next is an
  object, or when the run ends with live.json status "waiting" (then choices may be empty and the attempt is the
  node_outcome attempt of live.json waitingOn). Never emit Waiting twice for the same attempt.
- RunFinished last: after the acpx child exited (start) or at the end of replay. status/error: the run_completed /
  run_failed trace event if present, else live.json status and error; "waiting" when live.json says waiting; if
  live.json is still "running" or missing after the child exited, status "failed", error "acpx exited without
  finishing" (in start, acpx's final stdout JSON {{status, error}} is a fallback before that). In replay of a folder
  whose live.json says "running", the last event is RunFinished {{status:"running"}} (still live). In `start`, if no run folder appears within 15 s, or acpx exits before one appears, send RunFinished
  {{status:"failed", error: stderr tail (last 2 KiB) or "acpx did not create a run folder"}}.
- start: tokio::process::Command, stdin null, stdout/stderr piped and drained (stderr tail kept), PATH = extra_path
  joined before the inherited PATH, process_group(0) so cancel can signal the group. Never
  kill with SIGKILL first: a SIGKILLed acpx leaves live.json "running" forever. Keep polling 300 ms after the
  child exits to catch the last lines, then finish.
- list_runs: newest first by startedAt, at most `limit`.
Tests (at least 12), using a run folder written into a tempdir from string literals that follow the facts above:
replay of an acp step (two session/update lines -> two AgentUpdate with the exact `update` JSON; `_auth/...` line
is skipped); a session used by two steps (attribution by order); session lines that appear before their trace line
(feed in two halves through the Tailer and check identical events to one-shot replay); shell step with stdout
artifact and exit code; an artifact path with `..` is not read; checkpoint with next -> Waiting with sorted choices;
waiting run without next; torn last line ignored then completed on the next feed; switch edges -> one SketchEdge per
case with labels; BFS order with an unreachable node; list_runs order; a failed acp step (no response line)
  still yields StepFinished right after its updates; repeated JSON-RPC ids across two prompts; run title fallback to
  flowName; an action node with actionExecution "function" is Compute; run_failed gives status/error.
{RULES}""",
    },
    {
        "custom_id": "project",
        "output_file": f"{BIN}/project.rs",
        "context_files": [f"{MCTX}/notifications.md", f"{MCTX}/subagents.md", f"{MCTX}/tool_calls.md"],
        "prompt": f"""
Write `project.rs` for grok-acpx-acp: a pure mapping from `DriverEvent`s (a running acpx flow) to the ACP updates that
make the stock pager show it as one transcript. No I/O, no async.

Driver types (from crate::driver, already written; do not redefine):
{DRIVER_API}
Public surface (implement exactly):
{PROJECT_API}
How a run must look in the pager (the user's spec):
- The run reads top to bottom in the parent session like a chat turn. Nothing is pinned or snapped.
- RunStarted -> parent AgentMessageChunk with markdown: a heading "**▶ <title>**", a dim line "`<flow file name>` ·
  <run id>", then a ```mermaid fence with `mermaid(flow, states)` (all ○), then a blank line. Also a parent `Plan`
  update listing the steps in FlowSketch order: entry content "<id> · <profile> · <model> · <effort>" for acp steps
  (omit missing parts), "<id> · shell" for shell, "<id> · checkpoint" for checkpoints, "<id>" otherwise; status
  pending. Every later step change re-sends the whole Plan with updated statuses (in_progress / completed; a failed
  step is completed and its content gets " ✗"). Plan priority: medium for all entries.
- An acp step is a SUBAGENT so it is one capped live block in the parent with a full view behind Enter. Follow the
  exact order and JSON shapes in subagents.md: parent ToolCall with tool call id "acpx-tool:<run_id>:<attempt>",
  title exactly "Task", kind Other, status in_progress, and raw_input {{"variant":"Task","task_id":<subagent id>,
  "run_in_background":false,"reconcile":false}} set ON CREATION (the pager hides a Task card only when it is created
  with title "Task" or raw_input.variant "Task"), then XaiUpdate subagent_spawned with subagent_id "acpx-sub:<run_id>:<attempt>",
  parent_session_id, child_session_id (Projector::child_session_id), subagent_type "acpx step", description = the
  step label, model = the model if known. AgentPrompt -> child UserMessageChunk with the prompt text. AgentUpdate ->
  deserialize `update` with `serde_json::from_value::<acp::SessionUpdate>(update.clone())`; on success send it as
  Out::Update for the child session, except AvailableCommandsUpdate, SessionInfoUpdate, CurrentModeUpdate and any
  usage/config update, which are dropped; on failure (unknown variant) drop it. Keep the concatenated agent message text
  per attempt for `final_message`. StepFinished (acp) -> XaiUpdate subagent_finished with EXACTLY these keys:
  "sessionUpdate":"subagent_finished", subagent_id, parent_session_id, child_session_id, status ("completed" for ok,
  else "failed"), error (only when failed), tool_calls (u32: ToolCall updates forwarded for this attempt), turns (1),
  duration_ms (u64), tokens_used (0), output (final message cut to 4000 chars; omitted when empty), will_wake (false).
  tool_calls and turns are REQUIRED by the pager (no serde default: without them the update is dropped). Then a parent
  ToolCallUpdate status completed (also for a failed step: failure shows through subagent_finished; a Failed update
  makes the pager push a red generic card).
- A shell step is an execute tool call in the parent, shown like the pager's bash card (see tool_calls.md for the
  fields the execute card reads): ShellPrepared -> ToolCall id "acpx-tool:<run_id>:<attempt>", kind Execute, title
  = command line, status in_progress, raw_input {{"command": command_line, "cwd": cwd}}. StepFinished -> ToolCallUpdate
  status completed when outcome is "ok" (failed otherwise), content = one text block with stdout (or the error and
  stderr for a failed step, or "(no output)"), raw_output = the pager's Bash output shape: {{"type":"Bash",
  "output":<stdout bytes as a JSON array of numbers>, "output_for_prompt":<stdout>, "exit_code":<exit code or
  null>, "command":<command line>, "truncated":false, "signal":null, "timed_out":<outcome == "timed_out">,
  "description":null, "current_dir":"", "output_file":"", "total_bytes":<stdout byte length>}}.
- Compute steps (and Other): one parent ToolCall kind Think, title "<id>", created completed at StepFinished with
  content = a short text of `output` (a string as is; an object as compact JSON cut to 300 chars).
- Checkpoint: StepStarted/StepFinished show nothing by themselves. Waiting -> parent AgentMessageChunk
  "\\n**◐ <node>** · <question>\\n" plus, when choices exist, one line per choice "`<n>` **<label>** → `<flow_file>`"
  (n from 1). (The agent asks the actual question; do not emit the question request here.) Plan entry in_progress.
- RunFinished -> parent AgentMessageChunk on its own paragraph: "✓ **<title>** finished in <duration>" (completed),
  "✗ **<title>** failed: <error>" (failed / timed_out show the status word), nothing for "waiting".
  Duration = RunFinished.at_ms - RunStarted.at_ms via crate::driver::fmt_duration.
- Before any parent AgentMessageChunk that follows a tool call or subagent block, start with "\\n" so it renders as a
  new paragraph.
- Use the acp 0.10 builder APIs shown in the context files (e.g. acp::ToolCall::new(...).kind(..).status(..),
  acp::ToolCallUpdate::new(id, acp::ToolCallUpdateFields::new().status(..)...), acp::Plan::new(entries),
  acp::PlanEntry::new(content, priority, status), acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(t)))).
  If a builder you need is not shown, build the value with serde_json::from_value from its wire JSON instead
  (wire names: "sessionUpdate":"tool_call", "toolCallId", "title", "kind", "status", "rawInput", "rawOutput",
  "content":[{{"type":"content","content":{{"type":"text","text":...}}}}]).
- acp 0.10 names (verified): acp::PlanEntryPriority::Medium; acp::PlanEntryStatus::{{Pending, InProgress,
  Completed}}; acp::ToolKind::{{Other, Think, Execute}}; acp::ToolCallStatus::{{InProgress, Completed, Failed}};
  acp::SessionUpdate::{{Plan, ToolCall, ToolCallUpdate, AgentMessageChunk, UserMessageChunk, AgentThoughtChunk,
  AvailableCommandsUpdate, SessionInfoUpdate, CurrentModeUpdate}}. SessionUpdate is non_exhaustive: matches need `_`.
- mermaid(): `flowchart LR`, node ids sanitized to [A-Za-z0-9_], labels quoted ["✓ implement"], edges `a --> b` or
  `a -->|label| b`.
Tests (at least 12): RunStarted gives a message containing a mermaid fence and a Plan with one entry per node;
acp step order spawned-before-child-updates; AgentUpdate with an agent_thought_chunk JSON becomes a child
Out::Update; an unknown sessionUpdate is dropped; final_message concatenation; subagent_finished output cut; shell
tool call ids stable and completed/failed status; checkpoint Waiting text lists numbered choices; RunFinished text for
completed and failed; Plan statuses after a failed step; mermaid label glyphs and edge labels; determinism (two
projectors fed the same events give equal Debug output).
{RULES}""",
    },
    {
        "custom_id": "agent",
        "output_file": f"{BIN}/agent.rs",
        "context_files": [f"{MCTX}/acp_agent_skeleton.md", f"{MCTX}/notifications.md", f"{MCTX}/ask_user_question.md",
                          f"{MCTX}/turns_and_queue.md"],
        "prompt": f"""
Write `agent.rs` for grok-acpx-acp: the `acp::Agent` implementation that the pager talks to over stdio. It turns user
messages into acpx flow runs, streams them with crate::driver + crate::project, asks checkpoint questions, and chains
the next flow on an answer.

Driver and projector (already written; import from crate::driver and crate::project, do not redefine):
{DRIVER_API}
{PROJECT_API}
Public surface (implement exactly; main.rs is written by hand against it):
```rust
pub struct AgentConfig {{
    pub driver: crate::driver::DriverConfig,
    pub default_flow: Option<std::path::PathBuf>,   // runs for a plain message when no gate is waiting
    // relative /flow paths and the acpx working directory are the session's cwd (NewSessionRequest.cwd)
}}
pub struct AcpxAgent {{ /* private: client_tx, Rc<RefCell<State>> */ }}
impl AcpxAgent {{
    pub fn new(cfg: AgentConfig, client_tx: tokio::sync::mpsc::UnboundedSender<xai_acp_lib::AcpClientMessage>) -> Self;
}}
#[async_trait::async_trait(?Send)]
impl acp::Agent for AcpxAgent {{ /* initialize, authenticate, new_session, load_session, prompt, cancel,
   set_session_mode, ext_method, ext_notification: signatures exactly as in acp_agent_skeleton.md */ }}

/// What a user message means. Pure; tested.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {{
    Flow {{ file: String, input: serde_json::Value }},  // "/flow <file> [rest]": <file> may be double-quoted (spaces); rest parsed as a JSON object if it is one, else {{"task": rest}} ({{}} when empty)
    Runs,                                            // "/runs"
    Open {{ run_id: String }},                        // "/open <run id>"
    Answer {{ label: String }},                       // "/answer <label>", or a plain message equal (case-insensitive, trimmed) to a waiting gate label
    Task {{ text: String }},                          // any other plain message
    Help,                                            // "/acpx-help" or an unknown "/..." command (the pager owns "/help")
}}
pub fn parse_command(text: &str, waiting_labels: &[String]) -> Command;
```
Behaviour:
- initialize: protocol V1, load_session(true), session_capabilities(acp::SessionCapabilities::new()
  .list(acp::SessionListCapabilities::new())), agent_info Implementation::new("acpx", env!("CARGO_PKG_VERSION"))
  .title("acpx"). authenticate: Ok. set_session_mode: Ok with no change. ext_method: Err MethodNotFound.
  ext_notification: Ok(()).
- Sessions: the pager creates several (a prewarmed one, /new, ...). State is a HashMap<session id, SessionState
  {{cwd, waiting gate, running RunHandle, cancelled flag}}>; every notification and run uses the PromptRequest's
  session_id. new_session: id "acpx-<unix ms>-<counter>", stores NewSessionRequest.cwd (the DriverConfig for its runs
  is cfg.driver cloned with cwd = that cwd); after responding, send an AvailableCommandsUpdate built as
  `acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(vec![acp::AvailableCommand::new(name,
  description).input(acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(hint)))]))` with
  commands flow (hint "<flow file> [task or JSON input]"), runs, open (hint "<run id>"), answer (hint "<choice>"),
  acpx-help (no input). Send it from a spawn_local task so it follows the response.
- list_sessions (ACP session/list; the pager's resume picker uses it): one
  `acp::SessionInfo::new(run_id, cwd).title(title).updated_at(<ISO 8601 of started_ms>)` per run from
  driver::list_runs(runs_dir, 50) (cwd = the session request's cwd if given, else cfg.driver.cwd);
  return `acp::ListSessionsResponse::new(infos)`.
- prompt (one turn = one user message): join the text blocks of the prompt; parse_command with the labels of the
  waiting gate (if any); then:
  * Flow / Task: resolve the flow file against the session cwd (Task uses default_flow; if none, reply with the
    help text and end). Start
    the run with driver::start, create a Projector for this session, and for every DriverEvent: send each Out
    (Update -> acp::SessionNotification via acp_send with _meta as below; XaiUpdate -> ext notification
    "x.ai/session_notification" with params {{"sessionId","update","_meta"}}). When a Waiting event with choices
    arrives, remember it, and ask the question (below) only after that run's RunFinished (status "waiting") has been
    drained (acpx must have finished writing the run folder before load_run/gate_input). The prompt call returns only when the run chain ends: RunFinished not followed
    by a chained run, the user cancelled, or a Waiting the user did not answer. Return
    PromptResponse::new(StopReason::EndTurn) (Cancelled after cancel), stamped with the request's promptId as in
    prompt_response() in the skeleton.
  * Gate question: send `x.ai/ask_user_question` exactly as ask_user_question.md shows (an acp::ExtRequest via
    acp_send, awaiting the response): params include "sessionId" (the session) and "mode":"default", one question (the
    Waiting question), options = the choices (label = choice label, description = "starts <flow_file>", id = label),
    multiSelect false, noFreeform true, toolCallId = "acpx-gate:<run_id>:<attempt>". The chosen label is the first
    string of the first value of the response's "answers" object (unwrap an outer "result" key if present);
    outcome "cancelled" or anything unparsable means not answered. Accepted answer -> chain: the next flow file resolves relative to the directory
    of the current run's flow_path (RunStarted.flow_path); its input is
    `xai_grok_pager::acpx::bundle::gate_input(&run, label)` where `run = xai_grok_pager::acpx::bundle::load_run(&run_dir)?`
    (signatures: `pub fn load_run(dir: &Path) -> io::Result<Run>`, `pub fn gate_input(run: &Run, choice: &str) -> Value`);
    send a parent AgentMessageChunk "\\n→ **<label>**: starting `<file>`\\n" and continue streaming the new run in the
    same turn with a new Projector. Cancelled or error answer -> send "\\n◐ Waiting at <node>. Answer later with
    `/answer <label>`.\\n" and end the turn, remembering the waiting gate (run_dir, node, choices, flow_path).
  * Answer: if a gate is remembered and the label matches one of its choices, chain as above in this turn; else reply
    "No gate is waiting." / "Choices: a, b".
  * Runs: reply with a markdown table of driver::list_runs(runs_dir, 20): | run | flow | status | started |.
  * Open: replay that run into this session in this turn: driver::replay + a new Projector; send the Outs as
    live updates (not isReplay) so they render normally; remember a waiting gate if the run is waiting. Opening
    the same run twice in one session reuses child session ids: reply "Already shown above." instead.
  * Help: a short markdown list of the commands and what a plain message does.
  Replies are parent AgentMessageChunk text.
- _meta on every parent/child session/update during a turn: turnStartMs (prompt start), streamStartMs (prompt start),
  promptId (from the PromptRequest _meta "promptId" when present), agentTimestampMs (now). Child updates also carry
  eventId = "<child session id>:<counter>". See send_update / send_update_for_session in notifications.md.
- cancel (for the session id, or a child "acpx-step:" session id of its run): cancel the running RunHandle (if
  any), mark the session cancelled so the event loop stops after RunFinished; if a gate question is pending, send ext
  notification "pi/ui/cancel_interaction" {{"toolCallId":"acpx-gate:<run_id>:<attempt>"}} and treat it as not
  answered. The prompt then returns StopReason::Cancelled.
- load_session: register the session (cwd from the request). If the id is a run folder under runs_dir: run
  driver::replay, project every event with Projector::new(session id), and send each Out with _meta.isReplay = true
  (XaiUpdate via method "x.ai/session/update"), all BEFORE returning LoadSessionResponse::new() (the pager expects
  replay, then the response); remember a waiting gate. Any other id (e.g. an "acpx-..." session the pager created)
  returns LoadSessionResponse::new() with no replay, not an error.
- One run at a time per session: a prompt that arrives while that session's run is streaming is answered "A run is in progress; press Esc to
  cancel it." and ends immediately (EndTurn).
- State lives in Rc<RefCell<..>>; never hold a borrow across an .await.
Tests (at least 10): parse_command for every variant, including "/flow ship.flow.ts" (empty object input),
"/flow ship.flow.ts {{\\"name\\":\\"x\\"}}" (object input), "/flow f.ts fix the bug" ({{"task":"fix the bug"}}), a plain
message equal to a waiting label with different case and spaces -> Answer, an unknown slash command -> Help; a double-quoted
flow path with spaces; flow path resolution relative to the session cwd and absolute paths; next-flow resolution relative to the declaring flow's
directory; the AvailableCommands list contents; _meta building (pure helper) includes promptId only when present;
answer-label extraction from {{"outcome":"accepted","answers":{{"q":["approve"]}}}}, from the same wrapped in
{{"result": ...}}, and from {{"outcome":"cancelled"}}.
{RULES}""",
    },
    {
        "custom_id": "preview",
        "output_file": f"{SRC}/scrollback/blocks/subagent_preview.rs",
        "context_files": [f"{MCTX}/pager_subagent_block.md", f"{CTX}/theme.rs.txt"],
        "prompt": f"""
Write `subagent_preview.rs` in the pager crate xai-grok-pager: a capped live tail of a subagent's transcript, drawn
inside the parent transcript's one-line SubagentBlock so a running step shows its last 20-30 rows in place (Enter
still opens the full subagent view; that wiring exists). The wiring (feeding updates from the child routing site,
calling render from SubagentBlock, a setting for the row cap) is done by hand afterwards.

Public surface (implement exactly):
{PREVIEW_API}
Rendering (match the main transcript's look in miniature, theme fields from theme.rs.txt only):
- Keep a bounded list of entries (at most 400; drop the oldest): Thought(text), Message(text), User(text),
  Tool{{id, title, status}}, Plan(done, total).
- Consecutive AgentThoughtChunk text appends to the last Thought entry; same for AgentMessageChunk -> Message and
  UserMessageChunk -> User. Any other entry kind in between starts a new entry. ToolCall adds Tool; ToolCallUpdate
  with the same id updates its title (when present) and status in place. Plan replaces/updates one Plan entry.
- Rows: Thought rows prefixed "∴ " in theme.gray, italic; Message rows plain theme.text_primary; User rows prefixed
  "› " in theme.gray_dim; Tool rows "▸ <title>  <status word>" (status word: running /
  done / failed / pending; colour accent_running / accent_success / accent_error / gray_dim; glyph and word always
  present, never colour alone); Plan row "☐ <done>/<total> steps" in gray. Text is wrapped at word boundaries to
  `width` display columns (unicode-width), continuation rows indented by 2 to align under the text; a single word
  wider than width is hard-cut. Collapse runs of blank lines to one; trim trailing whitespace.
- render returns only the last max_rows rows; width 0 or max_rows 0 -> empty Vec. Lines own their strings ('static).
- revision increments on every ingest that changed the entries.
Tests (at least 10): thought chunks coalesce; message after thought starts a new entry; tool call then update changes
status word; wrapping at width 20 never exceeds 20 columns (check with unicode-width) including CJK text; hard-cut
of a long word; max_rows keeps the newest rows; width 0; 400-entry cap; revision changes only on effective change;
an unknown update kind is ignored. Build acp updates in tests with serde_json::from_value from wire JSON
({{"sessionUpdate":"agent_thought_chunk","content":{{"type":"text","text":"..."}}}}, {{"sessionUpdate":"tool_call",
"toolCallId":"t1","title":"read x","kind":"read","status":"in_progress"}}, {{"sessionUpdate":"tool_call_update",
"toolCallId":"t1","status":"completed"}}).
Rules for this file: it compiles inside crate xai-grok-pager (ratatui 0.29, unicode-width 0.2, agent_client_protocol
0.10.4 as `acp`, `crate::theme::Theme` is a Copy struct; get one in tests with `Theme::default()`). No unwrap/expect
on input, no unsafe, no new crates. Return the complete file in ONE ```rust block and nothing else.
""",
    },
]

# ═══════════════════════════════════════════════════════════════════════
# Engine (from templates/parallel-slices-with-context.py; patched: max_tokens + truncation check,
# raw returns kept in _returns/, per-task "raw" output, --only, --dry-run). Nothing below needs editing.
# ═══════════════════════════════════════════════════════════════════════
import argparse, json, os, re, sys, time, urllib.error, urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any, Dict, List

HERE = Path(__file__).resolve().parent
CHEAPER_ENDPOINT = "https://api.cheaperinference.com/v1/chat/completions"
OPENROUTER_ENDPOINT = "https://openrouter.ai/api/v1/chat/completions"
TIMEOUT, MAX_RETRIES, RETRY_DELAY = 600, 3, 2.0
MAX_CONTEXT_CHARS = 32 * 1024  # per context file; anything cut is warned about
PRICE_IN_PER_M, PRICE_OUT_PER_M = 2.00, 10.00  # google/gemini-4-argon on OpenRouter (checked 2026-11-16); estimate only, update with MODEL
TaskSpec = Dict[str, Any]

def make_http_request(url: str, headers: dict, data: dict = None, method: str = "GET") -> dict:
    """Zero-dependency HTTP JSON client with retry on transient errors."""
    body_bytes = json.dumps(data).encode("utf-8") if data is not None else None
    for attempt in range(MAX_RETRIES):
        req = urllib.request.Request(url, data=body_bytes, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
                resp_body = resp.read().decode("utf-8", errors="replace")
                try:
                    return json.loads(resp_body)
                except json.JSONDecodeError as e:
                    raise RuntimeError(f"Invalid JSON response: {resp_body[:200]}") from e
        except urllib.error.HTTPError as e:
            err_body = e.read().decode("utf-8", errors="replace")
            if e.code in (408, 429, 500, 502, 503, 504) and attempt < MAX_RETRIES - 1:
                delay = RETRY_DELAY * (attempt + 1)
                print(f"    [!] HTTP {e.code}, retrying in {delay:.0f}s...", file=sys.stderr)
                time.sleep(delay)
                continue
            raise RuntimeError(f"HTTP {e.code}: {err_body[:500]}")
        except (urllib.error.URLError, TimeoutError) as e:
            if attempt < MAX_RETRIES - 1:
                delay = RETRY_DELAY * (attempt + 1)
                print(f"    [!] Network error ({e}), retrying in {delay:.0f}s...", file=sys.stderr)
                time.sleep(delay)
                continue
            raise RuntimeError(f"Network error: {e}")
    raise RuntimeError("Request failed after all retries")

def extract_code_from_markdown(content: str) -> str:
    content = re.sub(r"^<think>[\s\S]*?</think>\s*", "", (content or "").replace("\r\n", "\n"))
    m = re.search(r"```[a-zA-Z0-9_-]*[ \t]*\n([\s\S]*)\n```", content)  # greedy: generated code may itself contain ``` fences
    return m.group(1).strip() if m else content.strip()

def resolve_api_config(cli_key=None, cli_endpoint=None):
    key = cli_key or os.environ.get("OPENROUTER_API_KEY") or os.environ.get("CHEAPERINFERENCE_API_KEY") or os.environ.get("CHEAPER_INFERENCE_API_KEY")
    if not key and sys.platform == "darwin":
        for svc in ("cheaper-inference-skchadur-2-api-key", "cheaper-inference-api-key"):
            try:
                import subprocess
                out = subprocess.check_output(["security", "find-generic-password", "-s", svc, "-w"], stderr=subprocess.DEVNULL)
                key = out.decode().strip()
                if key: break
            except Exception: pass
    key = key or os.environ.get("OPENROUTER_API_KEY")
    endpoint = cli_endpoint or (CHEAPER_ENDPOINT if key and key.startswith("ci_") else OPENROUTER_ENDPOINT)
    return key, endpoint

def build_prompt_with_context(repo_root: Path, task: TaskSpec) -> str:
    context_blocks = []
    repo_resolved = repo_root.resolve()
    for rel_path in task.get("context_files", []):
        file_path = (repo_root / rel_path).resolve()
        if not file_path.is_relative_to(repo_resolved):
            print(f"    [!] Security: Skipping out-of-repo path: {rel_path}", file=sys.stderr)
            continue
        if not file_path.is_file():
            print(f"    [!] Warning: Context file not found: {file_path}", file=sys.stderr)
            continue
        content = file_path.read_text(encoding="utf-8", errors="replace")
        if len(content) > MAX_CONTEXT_CHARS:
            print(f"    [!] {task['custom_id']}: {rel_path} is {len(content):,} chars, cut to {MAX_CONTEXT_CHARS:,}", file=sys.stderr)
            content = content[:MAX_CONTEXT_CHARS] + "\n... [truncated]"
        context_blocks.append(
            f"### Existing File Context: `{rel_path}`\n```\n{content}\n```"
        )

    context_str = "\n\n".join(context_blocks)
    if context_str:
        return (
            f"=== REPOSITORY CONTEXT ===\n\n"
            f"{context_str}\n\n"
            f"=== TASK SPECIFICATION ===\n\n"
            f"Target output file: `{task['output_file']}`\n\n"
            f"{task['prompt']}"
        )
    return f"Target output file: `{task['output_file']}`\n\n{task['prompt']}"

def execute_single_task(api_key: str, endpoint: str, model: str, task: TaskSpec, repo_root: Path, output_base: Path) -> Dict[str, Any]:
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json",
    }
    full_prompt = build_prompt_with_context(repo_root, task)
    payload = {
        "model": model,
        "temperature": 0.0,
        "max_tokens": MAX_TOKENS,
        "messages": [
            {
                "role": "system",
                "content": (
                    "You are a principal software engineer. Write production-ready code with complete types "
                    "and comprehensive tests where requested. Adhere strictly to the existing codebase conventions "
                    "provided in the context. Return code blocks only."
                ),
            },
            {
                "role": "user",
                "content": full_prompt,
            },
        ],
    }
    start = time.time()
    try:
        res = make_http_request(endpoint, headers=headers, data=payload, method="POST")
        choices = res.get("choices", [])
        if not choices:
            return {"task": task, "success": False, "error": "No response choices returned"}

        raw_content = choices[0].get("message", {}).get("content", "")
        returns = output_base / "_returns" / f"{task['custom_id']}.md"
        returns.parent.mkdir(parents=True, exist_ok=True)
        returns.write_text(raw_content or "", encoding="utf-8")
        finish = choices[0].get("finish_reason") or choices[0].get("native_finish_reason")
        if str(finish).lower() in ("length", "max_tokens"):
            return {"task": task, "success": False, "error": f"TRUNCATED at max_tokens={MAX_TOKENS}; partial return kept in {returns}"}
        clean_code = raw_content.strip() if task.get("raw") else extract_code_from_markdown(raw_content)
        if not clean_code:
            return {"task": task, "success": False, "error": f"EMPTY response; raw return in {returns}"}

        target_path = output_base / task["output_file"]
        target_path.parent.mkdir(parents=True, exist_ok=True)
        tmp_path = target_path.with_suffix(target_path.suffix + ".tmp")
        with open(tmp_path, "w", encoding="utf-8") as f:
            f.write(clean_code + "\n")
        tmp_path.replace(target_path)

        elapsed = time.time() - start
        return {"task": task, "success": True, "path": target_path, "elapsed": elapsed, "chars": len(clean_code)}
    except Exception as e:
        return {"task": task, "success": False, "error": str(e)}

def run_pipeline(api_key: str, endpoint: str, model: str, repo_root: Path, output_base: Path, tasks: List[TaskSpec]):
    print(f"\n[🚀] Launching parallel generation for {len(tasks)} tasks on {model} ({endpoint})...")
    print(f"    Repo root: {repo_root}")
    print(f"    Output base: {output_base}")
    start_total = time.time()
    results = []

    with ThreadPoolExecutor(max_workers=min(len(tasks), 16)) as executor:
        future_to_task = {
            executor.submit(execute_single_task, api_key, endpoint, model, t, repo_root, output_base): t
            for t in tasks
        }
        for future in as_completed(future_to_task):
            res = future.result()
            task = res["task"]
            if res["success"]:
                print(f"    [✓] Generated ({res['elapsed']:.1f}s): {res['path']} ({res['chars']} chars)")
            else:
                print(f"    [✗] Failed {task['custom_id']}: {res['error']}")
            results.append(res)

    total_time = time.time() - start_total
    success_count = sum(1 for r in results if r["success"])
    print(f"\n[🎉] Complete! {success_count}/{len(tasks)} files generated in {total_time:.1f}s!")

def main():
    parser = argparse.ArgumentParser(description="Parallel AI Code Generator with Context Injection")
    parser.add_argument("--key", default=None, help="API key (CHEAPERINFERENCE_API_KEY or OPENROUTER_API_KEY)")
    parser.add_argument("--endpoint", default=None, help="Completions endpoint")
    parser.add_argument("--model", default=MODEL, help=f"Model identifier (default: {MODEL})")
    parser.add_argument("--repo", default=str(HERE.parent), help="Repository root for context files (default: the repo holding this script)")
    parser.add_argument("--output", default=str(HERE / "acpx-mode_out"), help="Base output directory")
    parser.add_argument("--only", help="Comma-separated slice ids to fire")
    parser.add_argument("--dry-run", action="store_true", help="Show prompt sizes and cost estimate, call nothing")
    args = parser.parse_args()

    key, endpoint = (None, None) if args.dry_run else resolve_api_config(args.key, args.endpoint)
    if not key and not args.dry_run:
        print("Error: API key required via --key, CHEAPERINFERENCE_API_KEY, or OPENROUTER_API_KEY.", file=sys.stderr)
        sys.exit(1)

    tasks = [t for t in TASKS if not args.only or t["custom_id"] in args.only.split(",")]
    if not tasks:
        sys.exit(f"--only matched nothing; ids: {', '.join(t['custom_id'] for t in TASKS)}")
    if args.dry_run:
        total_in = 0
        for t in tasks:
            n = len(build_prompt_with_context(Path(args.repo).resolve(), t))
            total_in += n
            print(f"    {t['custom_id']:<16} {n:>7,} chars in  -> {t['output_file']}")
        est = total_in / 4 * PRICE_IN_PER_M / 1e6 + len(tasks) * 6000 * PRICE_OUT_PER_M / 1e6
        print(f"\n[dry run] {len(tasks)} slice(s), ~{total_in // 4:,} tokens in, ~$ {est:.3f} at PRICE_* rates (assumes ~6k tokens out per slice). No API call made.")
        return
    run_pipeline(
        api_key=key,
        endpoint=endpoint,
        model=args.model,
        repo_root=Path(args.repo).resolve(),
        output_base=Path(args.output).resolve(),
        tasks=tasks,
    )

if __name__ == "__main__":
    main()

````

### `src/client/view.ts` (complete file, repository pi-workflows)
````
import type { JsonValue } from "../state/json.js";
import type {
  WorkflowMessageKind,
  WorkflowMessageStatus,
  WorkflowTurn,
} from "../state/workflow-messages.js";

export const RUN_VIEW_SCHEMA = "pi-workflows.run-view.v1" as const;
export const SESSION_VIEW_SCHEMA = "pi-workflows.session-view.v1" as const;
export const SESSION_RUN_VIEW_SCHEMA = "pi-workflows.session-run-view.v1" as const;
export const SESSION_MESSAGE_SCHEMA = "pi-workflows.session-message.v1" as const;
export const BRANCH_REPORT_RECEIPT_SCHEMA = "pi-workflows.branch-report-receipt.v1" as const;
export const WORKFLOW_TURN_REPORT_RECEIPT_SCHEMA =
  "pi-workflows.workflow-turn-report-receipt.v1" as const;

export type WorkflowDisplayStatus =
  | "queued"
  | "running"
  | "waiting"
  | "paused"
  | "completed"
  | "failed"
  | "timed_out"
  | "cancelled"
  | "ambiguous";

export const WORKFLOW_DISPLAY_CONTROLS = [
  "pause",
  "resume",
  "cancel",
  "answer",
  "human-answer",
  "update",
  "submit",
  "review",
] as const;

export type WorkflowDisplayControl = (typeof WORKFLOW_DISPLAY_CONTROLS)[number];

export type WorkflowDisplay = {
  status: WorkflowDisplayStatus;
  activity: "supervised_runner" | "origin_turn" | null;
  controls: WorkflowDisplayControl[];
  reason: string | null;
  reasonContent?: JsonValue;
};

export type WorkflowRunQueueView = {
  runId: string;
  workflowName: string;
  workflowSourceRef: string;
  initialized: boolean;
  definitionDigest: string;
  status: "queued" | "starting" | "running" | "parked" | "done" | "failed" | "cancelled";
  originSessionId: string | null;
  executionMode: "interactive" | "headless";
  parentRunId: string | null;
  rootRunId: string;
  lineageKind: "restart" | null;
  restartNumber: number;
  parentRunRevision: number | null;
  errorCode: string | null;
  createdAt: string;
  updatedAt: string;
  startedAt: string | null;
  finishedAt: string | null;
};

export type WorkflowRunView = {
  schema: typeof RUN_VIEW_SCHEMA;
  runId: string;
  /** Presentation cursor for viewer snapshots and deltas. */
  revision: number;
  /** Execution resource revision for exact state-changing commands. */
  runRevision: number;
  display: WorkflowDisplay;
  manifest: JsonValue;
  state: JsonValue;
  workflow: JsonValue;
  queue: WorkflowRunQueueView;
  updates: JsonValue[];
  graphSteps: JsonValue[];
  graphStepStart: number;
  graphStepTotal: number;
  takenTransitions: string[];
  graphHistory: JsonValue;
  takenTransitionStart: number;
  takenTransitionTotal: number;
  graphCursor: number;
  stepStart: number;
  stepTotal: number;
  tracePage: JsonValue;
  session: JsonValue;
  settingsScopes: JsonValue[];
  settingsStart: number;
  settingsTotal: number;
  followUpQueue: JsonValue;
  followUpStart: number;
  followUpTotal: number;
  updateStart: number;
  updateTotal: number;
  workflowMessages: JsonValue[];
  workflowMessageStart: number;
  workflowMessageTotal: number;
  live: boolean;
  possiblyInterrupted: boolean;
};

export type WorkflowRunListPage = {
  schema: "pi-workflows.run-list-page.v1";
  revision: string;
  start: number;
  total: number;
  items: WorkflowRunSummary[];
};

export type WorkflowRunSummary = {
  runId: string;
  workflowName: string;
  originSessionId: string | null;
  createdAt: string;
  updatedAt: string;
  display: WorkflowDisplay;
  manifest: JsonValue;
  live: boolean;
  possiblyInterrupted: boolean;
};

export type ClientInteractiveRequest = {
  requestId: string;
  runId: string;
  attemptId: string;
  targetSessionId: string;
  kind: "agent" | "assistant" | "checkpoint" | "decision";
  contract: JsonValue;
  revision: number;
  status: "pending" | "settled" | "cancelled";
  acceptedSubmissionId: string | null;
  createdAt: string;
  updatedAt: string;
  settledAt: string | null;
  consumedAt: string | null;
};

/**
 * One node row of the Pi widget list. The server sends the semantic facts and
 * the extension applies terminal colors, glyphs, and layout.
 */
export type WorkflowSessionNodeRow = {
  nodeId: string;
  nodeType: string;
  /** The node runs an action command rather than a model step. */
  actionExecution: "function" | "shell" | null;
  state: "pending" | "running" | "waiting" | "ok" | "failed";
  /** Completed attempts plus the current attempt. */
  attempts: number;
  settingsChangeNumber: number | null;
  /** Present for the current node. */
  statusDetail: string | null;
  /** Present for the current node. */
  startedAt: string | null;
  /** Duration of the last completed attempt in milliseconds. */
  durationMs: number | null;
  /** Bounded failure text of the last attempt. */
  error: string | null;
  /** Present for a human-decision node. */
  humanDecision: {
    audience: string;
    summary: string | null;
    choices: { value: string; label: string }[];
    /** The accepted choice value, when the run recorded one. */
    choiceValue: string | null;
    presentationDigest: string | null;
  } | null;
  /** Present for a waiting node without a human decision. */
  summary: string | null;
  assistantResponse: boolean;
  /** Durable node outcome, when the run recorded one. */
  outcome: "ok" | "timed_out" | "failed" | "cancelled" | null;
};

/** One latest progress update for the widget progress line. */
export type WorkflowSessionProgressUpdate = {
  key: string;
  at: string;
  data: JsonValue;
};

/**
 * A bounded projection of the current run. Pi uses it for commands, the status
 * line, and the widget. Complete history stays behind the detailed view.
 */
export type WorkflowSessionRunView = {
  schema: typeof SESSION_RUN_VIEW_SCHEMA;
  runId: string;
  /** Presentation cursor for the detailed viewer. */
  revision: number;
  /** Execution resource revision for exact state-changing commands. */
  runRevision: number;
  queue: WorkflowRunQueueView;
  display: WorkflowDisplay;
  workflowName: string;
  runTitle: string | null;
  paused: boolean;
  currentNode: string | null;
  waitingOn: string | null;
  error: string | null;
  nodes: WorkflowSessionNodeRow[];
  nodeStart: number;
  nodeTotal: number;
  /** Latest progress update per key, bounded. */
  progressUpdates: WorkflowSessionProgressUpdate[];
  /** Monitor estimate tracks, when the run publishes them. */
  monitorEstimate: JsonValue | null;
  /** Monitor schedule facts, when the run records one. */
  monitorSchedule: { nextCheckAt: string; recordedAt: string } | null;
  live: boolean;
  possiblyInterrupted: boolean;
};

/**
 * The one workflow message Pi must inspect, add, finish, or confirm next. Large
 * content travels as a content reference that the client verifies on read.
 */
export type WorkflowSessionMessage = {
  schema: typeof SESSION_MESSAGE_SCHEMA;
  workflowMessageId: string;
  runId: string;
  targetSessionId: string;
  kind: WorkflowMessageKind;
  sourceId: string;
  order: number;
  status: WorkflowMessageStatus;
  piSessionEntryId: string | null;
  createdAt: string;
  updatedAt: string;
  /** Whether adding this message starts a model turn. */
  triggerTurn: boolean;
  customType: string;
  display: boolean;
  /** Small content stays inline; large content becomes a content reference. */
  content: JsonValue;
  contentDigest: string;
  /** The source request was cancelled; delivery can still be sent. */
  deliveryCancelled: boolean;
};

export type WorkflowSessionView = {
  schema: typeof SESSION_VIEW_SCHEMA;
  sessionId: string;
  /** The current session-owned run, bounded. */
  run: WorkflowSessionRunView | null;
  /** The one interactive request Pi must answer, when one exists. */
  interaction: ClientInteractiveRequest | null;
  /** The one workflow message Pi must handle next, when one exists. */
  workflowMessage: WorkflowSessionMessage | null;
  /** The one open model turn, when one exists. */
  openWorkflowTurn: WorkflowTurn | null;
  coordinatorEpoch: string | null;
  coordinatorActive: boolean;
  branchReportRequired: boolean;
};

export type WorkflowBranchReport = {
  targetSessionId: string;
  coordinatorEpoch: string;
  /**
   * The one current workflow message the server asked Pi to confirm, or null
   * when the session holds no workflow message yet.
   */
  workflowMessageId: string | null;
  /** The Pi session entry for that message, or null when Pi does not hold it. */
  piSessionEntryId: string | null;
  isIdle: boolean;
  hasPendingMessages: boolean;
};

export type WorkflowBranchReportReceipt = {
  schema: typeof BRANCH_REPORT_RECEIPT_SCHEMA;
  /** Whether Pi holds the reported message, and what the server did with it. */
  outcome: "present" | "absent";
  workflowMessageId: string | null;
};

export type WorkflowTurnReportReceipt = {
  schema: typeof WORKFLOW_TURN_REPORT_RECEIPT_SCHEMA;
  ownership: "active" | "settled" | "absent";
  turn: WorkflowTurn | null;
};

export type WorkflowTurnReport =
  | {
      state: "started";
      workflowMessageId: string;
      workflowTurnId: string;
      runId: string;
      targetSessionId: string;
      coordinatorEpoch: string;
    }
  | {
      state: "ended";
      workflowMessageId: string;
      workflowTurnId: string;
      runId: string;
      targetSessionId: string;
      coordinatorEpoch: string;
      stopReason: "completed" | "aborted" | "error" | "lost";
      responseSessionEntryId: string | null;
    };

````

### `src/server/view.ts` (complete file, repository pi-workflows)
````
import { createHash } from "node:crypto";
import {
  RUN_VIEW_SCHEMA,
  SESSION_MESSAGE_SCHEMA,
  SESSION_RUN_VIEW_SCHEMA,
  SESSION_VIEW_SCHEMA,
  type ClientInteractiveRequest,
  type WorkflowDisplay,
  type WorkflowDisplayStatus,
  type WorkflowRunListPage,
  type WorkflowRunQueueView,
  type WorkflowRunSummary,
  type WorkflowRunView,
  type WorkflowSessionMessage,
  type WorkflowSessionNodeRow,
  type WorkflowSessionProgressUpdate,
  type WorkflowSessionRunView,
  type WorkflowSessionView,
} from "../client/view.js";
import type { StateDatabase } from "../state/database.js";
import { canonicalJson, parseJson, type JsonValue } from "../state/json.js";
import {
  WorkflowMessageStore,
  type SessionSummaryFilter,
  type WorkflowMessage,
  type WorkflowMessageSummary,
} from "../state/workflow-messages.js";
import type {
  WorkflowRunQueueStore,
  WorkflowRunQueueRecord,
  WorkflowRunQueueViewRecord,
} from "../workflows/queue.js";
import { NODE_ID_MAX_BYTES } from "../workflows/schema.js";
import type { WorkflowRunDisplayState, WorkflowRunStore } from "../workflows/store.js";
import type {
  WorkflowRunState,
  WorkflowSessionEntryRecord,
  WorkflowSessionEventRecord,
  WorkflowStepRecord,
  WorkflowTraceEvent,
  WorkflowUpdateRecord,
} from "../workflows/types.js";
import { recoveryStopped } from "./recovery.js";
import type { ServerStateStore } from "./state.js";

export const WORKFLOW_PAGE_KINDS = [
  "steps",
  "trace",
  "trace_at_step",
  "session_entries",
  "session_events",
  "settings",
  "follow_ups",
  "updates",
  "workflow_messages",
] as const;

export type WorkflowPageKind = (typeof WORKFLOW_PAGE_KINDS)[number];

type RunPageRequest = {
  kind: WorkflowPageKind;
  cursor: number;
};

type ContentRecord = {
  runId: string;
  path: string;
  mediaType: "application/json" | "text/plain";
  bytes: Buffer;
  sha256: string;
};

const INLINE_CONTENT_BYTES = 16 * 1024;
const VIEW_PAGE_BYTES = 64 * 1024;
const VIEW_PAGE_ITEMS = 256;
const CONTENT_CHUNK_BYTES = 192 * 1024;
const CONTENT_CACHE_BYTES = 64 * 1024 * 1024;
const VIEW_CACHE_ITEMS = 64;
const TERMINAL_VIEW_RETENTION_MS = 60_000;
/**
 * Rows of context the widget window keeps above the node it follows. The widget
 * is about ten lines, so a few earlier rows are enough for orientation.
 */
const SESSION_NODE_LEAD = 4;
/**
 * Text one session field may carry: one node row, the workflow name, the run
 * title, the run error, or one decision choice label. The widget renders each on
 * one line it truncates to the terminal width, and the 1 MiB client frame budget
 * is the external limit that requires this bound. Complete text stays available
 * through the detailed run view and its node history.
 */
const SESSION_TEXT_BYTES = 4 * 1024;
const SESSION_MESSAGE_BATCH = 32;
/**
 * JSON one session detail may carry, such as a monitor estimate, one progress
 * payload, or the choice labels of one decision. A larger value is reported as
 * null, left out, or cut at a choice boundary so the single session frame stays
 * bounded; the detailed run view carries the complete value.
 */
const SESSION_DETAIL_JSON_BYTES = 8 * 1024;

export class ServerViewStore {
  private readonly contentRecords = new Map<string, ContentRecord>();
  private readonly listCache = new Map<string, { revision: string; page: WorkflowRunListPage }>();
  private readonly runCache = new Map<string, { version: string; view: WorkflowRunView | null }>();
  private readonly sessionCache = new Map<string, { version: string; view: WorkflowSessionView }>();
  // The selection walk reads message metadata in the durable order of each step, and
  // a session whose messages did not change keeps the same answer. The memo below
  // holds that answer under a key of cheap indexed facts, so a periodic view call
  // over a long history costs the key instead of the history. It carries the same
  // item bound as the view caches, so its size follows the viewed sessions and not
  // the number of sessions the server has ever seen.
  private readonly selectionCache = new Map<
    string,
    {
      key: string;
      message: WorkflowMessageSummary | undefined;
      retentionKnown: boolean;
      retainedRunId: string | undefined;
      retainedExpiresAt: number | null;
    }
  >();
  private contentBytes = 0;
  private activityRevision = 0;
  private readonly workflowMessages: WorkflowMessageStore;

  constructor(
    private readonly state: StateDatabase,
    private readonly queue: WorkflowRunQueueStore,
    private readonly serverState: ServerStateStore,
    private readonly runs: WorkflowRunStore,
    private readonly hasLiveRunner: (runId: string) => boolean,
    private readonly hasActiveSessionTurn: (targetSessionId: string) => boolean,
  ) {
    this.workflowMessages = serverState.workflowMessages;
  }

  noteWorkflowActivityChange(): void {
    this.activityRevision += 1;
  }

  list(cursor = 0, limit?: number): WorkflowRunListPage {
    return this.state.readTransaction(() => {
      const pageSize = Math.min(limit ?? VIEW_PAGE_ITEMS, VIEW_PAGE_ITEMS);
      const current = this.queue.workflowRunListRevision();
      const revision = `${current.revision}:${this.workflowActivityRevision()}`;
      const cacheKey = `${cursor}:${pageSize}`;
      const cached = this.listCache.get(cacheKey);
      if (cached?.revision === revision) {
        refreshCacheEntry(this.listCache, cacheKey, cached);
        return cached.page;
      }
      const loaded = this.queue.listWorkflowRunViews({ offset: cursor, limit: pageSize });
      const summaries = loaded.runs.map((run) => {
        const display = this.projectDisplay(
          run.runId,
          this.display(run, {
            status: run.runStateStatus as WorkflowRunState["status"],
            paused: run.paused,
            error: run.errorMessage,
          }),
        );
        return {
          runId: run.runId,
          // One free-form value must not push a list frame past the client limit.
          // The complete name stays in the run definition.
          workflowName: boundSessionText(run.workflowName),
          originSessionId: run.originSessionId,
          createdAt: run.createdAt,
          updatedAt: run.updatedAt,
          display,
          manifest: manifest(run, display.status),
          live: display.status === "running" || display.status === "waiting",
          possiblyInterrupted: run.status === "parked" && display.status !== "paused",
        } satisfies WorkflowRunSummary;
      });
      const items = byteBoundedForwardPage(summaries, (summary) => toJson(summary)).map(
        (item) => item as WorkflowRunSummary,
      );
      const page: WorkflowRunListPage = {
        schema: "pi-workflows.run-list-page.v1",
        revision,
        start: cursor,
        total: loaded.total,
        items,
      };
      rememberCacheEntry(this.listCache, cacheKey, { revision, page });
      return page;
    });
  }

  run(runId: string): WorkflowRunView | null {
    return this.state.readTransaction(() => {
      const version = this.runVersion(runId);
      const cached = this.runCache.get(runId);
      if (cached?.version === version) {
        refreshCacheEntry(this.runCache, runId, cached);
        return cached.view;
      }
      const view = this.readRun(runId);
      rememberCacheEntry(this.runCache, runId, { version, view });
      return view;
    });
  }

  page(runId: string, request: RunPageRequest): WorkflowRunView | null {
    return this.state.readTransaction(() => this.readRun(runId, request));
  }

  private readRun(runId: string, page?: RunPageRequest): WorkflowRunView | null {
    const queue = this.queue.getWorkflowRunView(runId);
    const counts = this.runs.readRunViewCounts(runId);
    if (queue === undefined || counts === null) return null;
    const graphCursor =
      page?.kind === "steps"
        ? clampCursor(page.cursor, counts.steps)
        : Math.max(0, counts.steps - 1);
    const traceCursor =
      page?.kind === "trace_at_step"
        ? this.runs.traceCursorForStep(runId, page.cursor, counts.trace)
        : page?.kind === "trace"
          ? page.cursor
          : undefined;
    const stepRange = viewRange(counts.steps, page?.kind === "steps" ? page.cursor : undefined);
    const traceRange = viewRange(counts.trace, traceCursor);
    const entryRange = viewRange(
      counts.sessionEntries,
      page?.kind === "session_entries" ? page.cursor : undefined,
    );
    const eventRange = viewRange(
      counts.sessionEvents,
      page?.kind === "session_events" ? page.cursor : undefined,
    );
    const settingsRange = viewRange(
      counts.settings,
      page?.kind === "settings" ? page.cursor : undefined,
    );
    const followUpRange = viewRange(
      counts.followUps,
      page?.kind === "follow_ups" ? page.cursor : undefined,
    );
    const updateRange = viewRange(
      counts.updates,
      page?.kind === "updates" ? page.cursor : undefined,
    );
    const messageRange = viewRange(
      counts.workflowMessages,
      page?.kind === "workflow_messages" ? page.cursor : undefined,
    );
    const loaded = this.runs.readRunView(runId, {
      steps: stepRange,
      trace: traceRange,
      sessionEntries: entryRange,
      sessionEvents: eventRange,
      settings: settingsRange,
      followUps: followUpRange,
      updates: updateRange,
      graphCursor,
    });
    if (loaded === null) return null;
    const stepPage = byteBoundedCandidatePage(
      loaded.state.steps,
      stepRange.start,
      counts.steps,
      page?.kind === "steps" ? page.cursor : undefined,
      (step) => this.projectStep(runId, step),
    );
    const tracePage = byteBoundedCandidatePage(
      loaded.traceEvents,
      traceRange.start,
      counts.trace,
      traceCursor,
      (event) => this.projectTraceEvent(runId, event),
    );
    const entryPage = byteBoundedCandidatePage(
      loaded.sessionEntries,
      entryRange.start,
      counts.sessionEntries,
      page?.kind === "session_entries" ? page.cursor : undefined,
      (entry) => this.projectSessionEntry(runId, entry),
    );
    const eventPage = byteBoundedCandidatePage(
      loaded.sessionEvents,
      eventRange.start,
      counts.sessionEvents,
      page?.kind === "session_events" ? page.cursor : undefined,
      (event) => this.projectSessionEvent(runId, event),
    );
    const settingsPage = byteBoundedCandidatePage(
      loaded.settingsScopes,
      settingsRange.start,
      counts.settings,
      page?.kind === "settings" ? page.cursor : undefined,
      (scope) => this.projectRecordField(runId, scope, "settings"),
    );
    const followUps = loaded.followUpQueue?.followUps ?? [];
    const followUpPage = byteBoundedCandidatePage(
      followUps,
      followUpRange.start,
      counts.followUps,
      page?.kind === "follow_ups" ? page.cursor : undefined,
      (followUp) => this.projectRecordField(runId, followUp, "prompt"),
    );
    const updates = loaded.state.updates ?? [];
    const updatePage = byteBoundedCandidatePage(
      updates,
      updateRange.start,
      counts.updates,
      page?.kind === "updates" ? page.cursor : undefined,
      (update) => this.projectUpdate(runId, update),
    );
    // Message content loads only for the rows the byte-bounded page shows.
    const messageSummaries = this.workflowMessages.listRunSummaryPage(runId, messageRange);
    const messagePage = byteBoundedCandidatePage(
      messageSummaries,
      messageRange.start,
      counts.workflowMessages,
      page?.kind === "workflow_messages" ? page.cursor : undefined,
      (summary) =>
        this.projectRecordField(runId, this.workflowMessages.materialize(summary), "content"),
    );
    const followUpQueue =
      loaded.followUpQueue === null
        ? null
        : projectFollowUpQueue(loaded.followUpQueue, followUpPage.items);
    const completeGraphSteps = loaded.graphSteps.map((step) => toCompactStepJson(step));
    const completeTakenTransitions = loaded.takenTransitions.map((transition) =>
      toJson(transition),
    );
    const graphSteps = byteBoundedForwardPage(completeGraphSteps, (step) => step);
    const takenTransitions = byteBoundedForwardPage(
      completeTakenTransitions,
      (transition) => transition,
    ).filter((transition): transition is string => typeof transition === "string");
    const graphHistory = this.projectValue(runId, {
      steps: completeGraphSteps,
      transitions: completeTakenTransitions,
    });
    const revision = this.presentationRevision(runId);
    const display = this.projectDisplay(runId, this.display(queue, loaded.state));
    return {
      schema: RUN_VIEW_SCHEMA,
      runId,
      revision,
      runRevision: this.runs.runRevision(runId),
      display,
      manifest: manifest(queue, display.status),
      state: this.projectState(runId, loaded.state, stepPage.items, updatePage.items),
      workflow: this.projectWorkflow(runId, loaded.snapshot),
      queue: projectQueue(queue),
      updates: updatePage.items,
      graphSteps,
      graphStepStart: 0,
      graphStepTotal: loaded.graphSteps.length,
      takenTransitions,
      graphHistory,
      takenTransitionStart: 0,
      takenTransitionTotal: loaded.takenTransitions.length,
      graphCursor,
      stepStart: stepPage.start,
      stepTotal: stepPage.total,
      tracePage,
      session: {
        binding: toJson(loaded.sessionBinding),
        entryPage,
        eventPage,
        capture: toJson(loaded.sessionCapture),
        integrity: toJson(loaded.sessionIntegrity),
        replayCheckpoint: this.projectReplayCheckpoint(
          runId,
          this.runs.readSessionReplayCheckpoint(runId, eventPage.start),
        ),
      },
      settingsScopes: settingsPage.items,
      settingsStart: settingsPage.start,
      settingsTotal: settingsPage.total,
      followUpQueue: toJson(followUpQueue),
      followUpStart: followUpPage.start,
      followUpTotal: followUpPage.total,
      updateStart: updatePage.start,
      updateTotal: updatePage.total,
      workflowMessages: messagePage.items,
      workflowMessageStart: messagePage.start,
      workflowMessageTotal: messagePage.total,
      live: display.status === "running" || display.status === "waiting",
      possiblyInterrupted: queue.status === "parked" && display.status !== "paused",
    };
  }

  session(
    sessionId: string,
    coordinator: { epoch: string; active: boolean; branchReportRequired: boolean } | null = null,
    nodeCursor?: number,
  ): WorkflowSessionView {
    return this.state.readTransaction(() => {
      const activeQueue = this.queue.findSessionReservationView(sessionId);
      const retainedRunId =
        activeQueue === undefined ? this.retainedTerminalRunIdForView(sessionId) : undefined;
      const runId = activeQueue?.runId ?? retainedRunId;
      // Selection is one metadata read. The key must name the exact message,
      // because a status or entry change can land in the same millisecond as the
      // write that made it current, and then the aggregate facts stay equal. The
      // message alone identifies the selection, including a session that holds
      // no live or retained run.
      const selected = this.currentWorkflowMessageSummary(sessionId);
      const version = [
        runId ?? "-",
        runId === undefined ? "-" : this.runVersion(runId),
        this.pendingSessionRevision(sessionId),
        this.sessionMessageRevision(sessionId),
        selected === undefined
          ? "-"
          : [selected.workflowMessageId, selected.status, selected.piSessionEntryId ?? "-"].join(
              ":",
            ),
        this.openTurnRevision(sessionId),
        coordinator?.epoch ?? "-",
        coordinator?.active === true ? "active" : "idle",
        coordinator?.branchReportRequired === true ? "report" : "reported",
        nodeCursor === undefined ? "-" : `${nodeCursor}`,
      ].join("|");
      const cached = this.sessionCache.get(sessionId);
      if (cached?.version === version) {
        refreshCacheEntry(this.sessionCache, sessionId, cached);
        return cached.view;
      }
      const message =
        selected === undefined ? undefined : this.workflowMessages.materialize(selected);
      const openTurn = this.workflowMessages.openTurnsForSession(sessionId)[0];
      const view: WorkflowSessionView = {
        schema: SESSION_VIEW_SCHEMA,
        sessionId,
        run: runId === undefined ? null : this.sessionRun(runId, nodeCursor),
        interaction: this.currentInteraction(sessionId),
        workflowMessage:
          message === undefined
            ? null
            : this.sessionMessage(message, this.hasCancelledSource(message)),
        openWorkflowTurn: openTurn === undefined ? null : openTurn,
        coordinatorEpoch: coordinator?.epoch ?? null,
        coordinatorActive: coordinator?.active ?? false,
        branchReportRequired: coordinator?.branchReportRequired ?? false,
      };
      rememberCacheEntry(this.sessionCache, sessionId, { version, view });
      return view;
    });
  }

  /** The one workflow message Pi must inspect, add, finish, or confirm next. */
  currentWorkflowMessage(sessionId: string): WorkflowMessage | undefined {
    const selected = this.currentWorkflowMessageSummary(sessionId);
    return selected === undefined ? undefined : this.workflowMessages.materialize(selected);
  }

  /**
   * Selection reads message metadata only. Content loads once, for the message
   * the session must act on.
   */
  /** The one workflow message Pi must inspect, add, finish, or confirm next. */
  private currentWorkflowMessageSummary(sessionId: string): WorkflowMessageSummary | undefined {
    return this.sessionSelection(sessionId, false).message;
  }

  /** The retained terminal run the session view shows, or undefined. */
  private retainedTerminalRunIdForView(sessionId: string): string | undefined {
    return this.sessionSelection(sessionId, true).retainedRunId;
  }

  /**
   * The selection and the retained terminal run of one session, held under the
   * indexed facts both depend on. The held result follows the item bound of the
   * view caches, so viewing many sessions cannot grow this map without limit, and a
   * repeat call over a session whose messages did not change reads no message row.
   */
  private sessionSelection(
    sessionId: string,
    wantRetention: boolean,
  ): { message: WorkflowMessageSummary | undefined; retainedRunId: string | undefined } {
    const now = Date.now();
    const key = this.selectionKey(sessionId);
    const cached = this.selectionCache.get(sessionId);
    if (
      cached !== undefined &&
      cached.key === key &&
      this.selectionHolds(cached, now, wantRetention)
    ) {
      refreshCacheEntry(this.selectionCache, sessionId, cached);
      return cached;
    }
    // The retention walk runs at most once per computation, and only when the view
    // needs it or the selection falls through to its retained terminal step.
    let retained: { runId: string; expiresAt: number | null } | undefined;
    let retentionKnown = false;
    const retention = () => {
      if (!retentionKnown) {
        retentionKnown = true;
        retained = this.retainedTerminalForView(sessionId, now);
      }
      return retained;
    };
    const message = this.selectWorkflowMessageSummary(sessionId, retention);
    if (wantRetention) retention();
    const entry = {
      key,
      message,
      retentionKnown,
      retainedRunId: retained?.runId,
      retainedExpiresAt: retained?.expiresAt ?? null,
    };
    rememberCacheEntry(this.selectionCache, sessionId, entry);
    return entry;
  }

  /**
   * Whether a held selection still answers this session. A message inside the
   * terminal retention window and a retained run both expire with the clock, not
   * with a stored write, so both are checked before the result is reused.
   */
  private selectionHolds(
    entry: {
      message: WorkflowMessageSummary | undefined;
      retentionKnown: boolean;
      retainedRunId: string | undefined;
      retainedExpiresAt: number | null;
    },
    now: number,
    wantRetention: boolean,
  ): boolean {
    if (wantRetention && !entry.retentionKnown) return false;
    if (entry.retainedRunId !== undefined && entry.retainedExpiresAt !== null) {
      if (entry.retainedExpiresAt <= now) return false;
    }
    return entry.message === undefined || this.retentionStillHolds(entry.message, now);
  }

  /**
   * Whether a delivered terminal message is still inside its retention window. The
   * window closes with time and not with a stored write, so the held selection is
   * checked against the clock before it is reused.
   */
  private retentionStillHolds(message: WorkflowMessageSummary, now: number = Date.now()): boolean {
    if (message.kind !== "terminal" || message.status !== "sent") return true;
    const turn = this.workflowMessages.latestTurnForMessage(message.workflowMessageId);
    return Date.parse(turn?.endedAt ?? message.updatedAt) + TERMINAL_VIEW_RETENTION_MS > now;
  }

  /**
   * The indexed facts the selection depends on: the message revision the database
   * maintains, the run and request revisions, the state of the session runs, and the
   * open turns of the session.
   */
  private selectionKey(sessionId: string): string {
    return [
      this.sessionMessageRevision(sessionId),
      this.workflowActivityRevision(),
      this.pendingSessionRevision(sessionId),
      this.openTurnRevision(sessionId),
      this.sessionRunsRevision(sessionId),
    ].join("|");
  }

  /**
   * The count and state of the session runs. Eligibility also depends on whether a
   * run is paused and whether its status is terminal, so the selection key covers
   * those facts without reading any message.
   */
  private sessionRunsRevision(sessionId: string): string {
    const row = this.state.connection
      .prepare(
        `SELECT count(*) AS runs,
                coalesce(sum(CASE WHEN r.paused = 1 THEN 1 ELSE 0 END), 0) AS paused,
                coalesce(sum(CASE WHEN r.status IN ('queued', 'running', 'waiting') THEN 1 ELSE 0 END), 0) AS live
         FROM run_bindings b JOIN runs r ON r.run_id = b.run_id
         WHERE b.origin_session_id = ?`,
      )
      .get(sessionId);
    return isObjectRecord(row) &&
      typeof row.runs === "number" &&
      typeof row.paused === "number" &&
      typeof row.live === "number"
      ? `${row.runs}:${row.paused}:${row.live}`
      : "-";
  }

  /** Walk the session message metadata for the one message Pi must act on. */
  private selectWorkflowMessageSummary(
    sessionId: string,
    retention: () => { runId: string } | undefined,
  ): WorkflowMessageSummary | undefined {
    const openTurn = this.workflowMessages.openTurnsForSession(sessionId)[0];
    if (openTurn !== undefined) {
      const open = this.workflowMessages.readSessionSummary(sessionId, openTurn.workflowMessageId);
      if (open !== undefined) return open;
    }
    // Selection walks one bounded batch of metadata at a time, in the order each
    // step needs. Each step names the candidate filter it can use, so it never
    // reads a row it must skip and it stops at the first message it needs.
    for (const message of this.walkSessionSummaries(sessionId, {
      status: "pending",
      newestFirst: false,
      filter: "eligiblePending",
    })) {
      if (this.isMessageEligible(message)) return message;
    }
    for (const message of this.walkSessionSummaries(sessionId, {
      status: "sent",
      newestFirst: true,
      filter: "piWork",
    })) {
      if (!this.needsPiWork(message)) continue;
      return message;
    }
    for (const message of this.walkSessionSummaries(sessionId, {
      status: "sent",
      newestFirst: true,
      filter: "cancelledStep",
    })) {
      if (message.kind !== "step" || !this.hasCancelledSource(message)) continue;
      if (this.workflowMessages.latestTurnForMessage(message.workflowMessageId) !== undefined) {
        continue;
      }
      return message;
    }
    // A retained terminal message stays current until its delivery or first turn finishes.
    const retainedRunId = retention()?.runId;
    if (retainedRunId !== undefined) {
      return this.workflowMessages.readSessionSummaryByRun(sessionId, retainedRunId, "terminal");
    }
    return undefined;
  }

  /**
   * Walk one session's message metadata in bounded batches, in durable order or
   * newest first, with the named candidate filter for that step. The caller stops
   * at the first message it needs.
   */
  private *walkSessionSummaries(
    sessionId: string,
    options: {
      status: string | null;
      newestFirst: boolean;
      kind?: string;
      filter?: SessionSummaryFilter;
      cutoff?: number;
    },
  ): Generator<WorkflowMessageSummary> {
    let lastOrder: number | undefined;
    for (;;) {
      const batch = this.workflowMessages.listSessionSummaryBatch(sessionId, {
        status: options.status,
        newestFirst: options.newestFirst,
        ...(options.kind === undefined ? {} : { kind: options.kind }),
        ...(options.filter === undefined ? {} : { filter: options.filter }),
        ...(options.cutoff === undefined ? {} : { cutoff: options.cutoff }),
        ...(lastOrder === undefined ? {} : { lastOrder }),
        limit: SESSION_MESSAGE_BATCH,
      });
      for (const message of batch) yield message;
      if (batch.length < SESSION_MESSAGE_BATCH) return;
      lastOrder = batch[batch.length - 1]?.order;
      if (lastOrder === undefined) return;
    }
  }

  /** Whether Pi still owes delivery confirmation or a model turn for this message. */
  private needsPiWork(message: WorkflowMessageSummary): boolean {
    return this.openWorkflowMessage([message]) !== undefined;
  }

  private currentInteraction(sessionId: string): ClientInteractiveRequest | null {
    const pending = this.serverState.listPendingInteractions(sessionId);
    const first = pending[0];
    if (first === undefined) return null;
    return this.projectRecordField(
      first.runId,
      first,
      "contract",
    ) as unknown as ClientInteractiveRequest;
  }

  private sessionMessage(
    message: WorkflowMessage,
    deliveryCancelled: boolean,
  ): WorkflowSessionMessage {
    return {
      schema: SESSION_MESSAGE_SCHEMA,
      workflowMessageId: message.workflowMessageId,
      runId: message.runId,
      targetSessionId: message.targetSessionId,
      kind: message.kind,
      sourceId: message.sourceId,
      order: message.order,
      status: message.status,
      piSessionEntryId: message.piSessionEntryId,
      createdAt: message.createdAt,
      updatedAt: message.updatedAt,
      triggerTurn: message.content.triggerTurn,
      customType: message.content.customType,
      display: message.content.display,
      content: this.projectValue(message.runId, toJson(message.content)),
      contentDigest: message.contentDigest,
      deliveryCancelled,
    };
  }

  /**
   * The number of message writes this session has seen. The database maintains it,
   * so the session view cache key notices every insert, update, and delete without
   * reading the session's stored messages.
   */
  private sessionMessageRevision(sessionId: string): string {
    const row = this.state.connection
      .prepare("SELECT revision FROM session_message_revisions WHERE target_session_id = ?")
      .get(sessionId);
    if (row === undefined) return "0";
    if (!isObjectRecord(row) || typeof row.revision !== "number") {
      throw new Error("Session message revision is invalid");
    }
    return `${row.revision}`;
  }

  private openTurnRevision(sessionId: string): string {
    const row = this.state.connection
      .prepare(
        `SELECT count(*) AS count, COALESCE(max(started_at), 0) AS startedAt
         FROM workflow_turns WHERE target_session_id = ? AND state = 'started'`,
      )
      .get(sessionId);
    if (!isObjectRecord(row)) throw new Error("Session turn revision is invalid");
    return `${row.count}:${row.startedAt}`;
  }

  /**
   * Bounded current run projection for Pi. It carries the semantic facts for the
   * status line and widget and leaves complete history to the detailed view.
   */
  sessionRun(runId: string, nodeCursor?: number): WorkflowSessionRunView | null {
    const queue = this.queue.getWorkflowRunView(runId);
    const counts = this.runs.readRunViewCounts(runId);
    if (queue === undefined || counts === null) return null;
    const empty = { start: 0, limit: 0 };
    const loaded = this.runs.readRunView(runId, {
      steps: viewRange(counts.steps),
      trace: empty,
      sessionEntries: empty,
      sessionEvents: empty,
      settings: empty,
      followUps: empty,
      // Progress and monitor facts come from their own bounded tail reads, so the
      // compact run never loads the head of a long update set.
      updates: empty,
      graphCursor: 0,
    });
    if (loaded === null) return null;
    const state = loaded.state;
    const failureNodeId = this.sessionFailureNodeId(state);
    const display = this.projectDisplay(runId, this.display(queue, state));
    const rows = this.sessionNodeRows(runId, loaded.snapshot, state, failureNodeId);
    const window = boundedNodeWindow(
      rows,
      nodeCursor ?? defaultNodeWindowStart(rows, state, failureNodeId),
    );
    return {
      schema: SESSION_RUN_VIEW_SCHEMA,
      runId,
      revision: this.presentationRevision(runId),
      runRevision: this.runs.runRevision(runId),
      queue: projectQueue(queue),
      display,
      workflowName: boundSessionText(state.workflowName),
      runTitle: state.runTitle === undefined ? null : boundSessionText(state.runTitle),
      paused: state.paused === true,
      currentNode: boundSessionNodeId(state.currentNode),
      waitingOn: boundSessionNodeId(state.waitingOn),
      error: state.error === undefined ? null : boundSessionText(state.error),
      nodes: window.items,
      nodeStart: window.start,
      nodeTotal: window.total,
      progressUpdates: sessionProgressUpdates(
        this.runs.readCurrentUpdateTail(runId, {
          type: "progress",
          limit: MAX_SESSION_PROGRESS_UPDATES,
        }),
      ),
      monitorEstimate: boundedSessionJson(toJson(state.outputs.estimate ?? null)),
      monitorSchedule: sessionMonitorSchedule(
        this.runs.readCurrentUpdateTail(runId, {
          type: "monitor.schedule",
          key: "next-check",
          limit: 1,
        }),
      ),
      live: display.status === "running" || display.status === "waiting",
      possiblyInterrupted: queue.status === "parked" && display.status !== "paused",
    };
  }

  /**
   * One row per definition node, in definition order. Attempt facts come from
   * durable attempts so the rows stay correct for long histories. Detailed text
   * travels only for the current, waiting, and most recent failed node.
   */
  private sessionNodeRows(
    runId: string,
    snapshot: unknown,
    state: WorkflowRunState,
    failureNodeId: string | undefined,
  ): WorkflowSessionNodeRow[] {
    const attempts = this.state.connection
      .prepare(
        `SELECT node_id AS nodeId, status, attempt_number AS attemptNumber,
                started_at AS startedAt, finished_at AS finishedAt,
                settings_change_number AS settingsChangeNumber, error_hash AS errorHash
         FROM node_attempts WHERE run_id = ? ORDER BY attempt_number`,
      )
      .all(runId)
      .filter(isNodeAttemptRow);
    const byNode = new Map<string, NodeAttemptFacts>();
    for (const attempt of attempts) {
      const facts = byNode.get(attempt.nodeId) ?? {
        attempts: 0,
        activeStatus: null,
        lastStatus: null,
        lastSettingsChangeNumber: null,
        startedAt: null,
        durationMs: null,
        errorHash: null,
      };
      facts.attempts += 1;
      if (attempt.startedAt !== null && attempt.finishedAt === null) {
        facts.startedAt = new Date(attempt.startedAt).toISOString();
      }
      if (attempt.startedAt !== null && attempt.finishedAt !== null) {
        facts.durationMs = Math.max(0, attempt.finishedAt - attempt.startedAt);
      }
      if (attempt.settingsChangeNumber !== null) {
        facts.lastSettingsChangeNumber = attempt.settingsChangeNumber;
      }
      if (isActiveAttemptStatus(attempt.status)) facts.activeStatus = attempt.status;
      else {
        // Attempts are ordered by number, so a finished attempt means no earlier
        // attempt is still active. A leftover unfinished row from a superseded
        // attempt must not report the node as working after a later attempt
        // succeeded.
        facts.activeStatus = null;
        facts.startedAt = null;
        facts.lastStatus = attempt.status;
        facts.errorHash = attempt.errorHash;
      }
      byNode.set(attempt.nodeId, facts);
    }
    const records = nodeRecords(snapshot);
    const failure = failureNodeId;
    const rows: WorkflowSessionNodeRow[] = [];
    for (const [nodeId, node] of Object.entries(records)) {
      const facts = byNode.get(nodeId);
      const detail =
        nodeId === state.currentNode || nodeId === state.waitingOn || nodeId === failure;
      rows.push({
        nodeId,
        nodeType: typeof node.nodeType === "string" ? node.nodeType : "unknown",
        actionExecution: sessionActionExecution(node),
        state: nodeRowState(state, nodeId, facts),
        attempts: facts?.attempts ?? 0,
        settingsChangeNumber:
          nodeId === state.currentNode
            ? (state.currentSettingsChangeNumber ?? facts?.lastSettingsChangeNumber ?? null)
            : (facts?.lastSettingsChangeNumber ?? null),
        statusDetail:
          nodeId === state.currentNode && typeof state.statusDetail === "string"
            ? boundNodeText(state.statusDetail)
            : null,
        // The widget shows the current node, or the waiting node while the run
        // is running, as the node it is working on. Both need their start time
        // so the elapsed segment stays visible.
        startedAt:
          nodeId === state.currentNode || nodeId === state.waitingOn
            ? (facts?.startedAt ?? null)
            : null,
        durationMs: facts?.durationMs ?? null,
        error: detail ? boundNodeText(this.readAttemptError(facts?.errorHash ?? null)) : null,
        humanDecision: sessionHumanDecision(node, state, nodeId),
        summary:
          nodeId === state.waitingOn && typeof node.summary === "string"
            ? boundNodeText(node.summary)
            : null,
        assistantResponse: isAssistantResponseNode(node),
        outcome: state.results[nodeId]?.outcome ?? null,
      });
    }
    return rows;
  }

  private readAttemptError(hash: Buffer | null): string | null {
    if (hash === null) return null;
    const blob = this.state.readBlob(hash);
    if (blob === undefined) return null;
    return blob.content.toString("utf8").slice(0, 512);
  }

  /** The last node that did not finish cleanly, in completion order. */
  private sessionFailureNodeId(state: WorkflowRunState): string | undefined {
    let failed: string | undefined;
    for (const [nodeId, result] of Object.entries(state.results)) {
      if (result.outcome !== "ok") failed = nodeId;
    }
    return failed;
  }

  clearTerminal(sessionId: string, runId?: string, now: number = Date.now()): string | null {
    return this.state.transaction(() => {
      const retained = this.retainedTerminalRunId(sessionId, now);
      if (retained === undefined) return null;
      if (runId !== undefined && retained !== runId) {
        throw new Error(`Retained terminal workflow does not match run ${runId}`);
      }
      this.state.connection
        .prepare(
          `INSERT INTO session_terminal_views(target_session_id, cleared_run_id, cleared_at)
           VALUES (?, ?, ?)
           ON CONFLICT(target_session_id) DO UPDATE SET
             cleared_run_id = excluded.cleared_run_id, cleared_at = excluded.cleared_at`,
        )
        .run(sessionId, retained, now);
      this.sessionCache.delete(sessionId);
      this.selectionCache.delete(sessionId);
      return retained;
    });
  }

  content(runId: string, contentPath: string, offset: number): JsonValue | null {
    const key = contentKey(runId, contentPath);
    const record = this.contentRecords.get(key) ?? this.recoverContent(runId, contentPath);
    if (record === undefined) return null;
    this.contentRecords.delete(key);
    this.contentRecords.set(key, record);
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > record.bytes.byteLength) {
      throw new Error("Workflow content offset is outside the content range");
    }
    const nextOffset = Math.min(record.bytes.byteLength, offset + CONTENT_CHUNK_BYTES);
    return {
      schema: "pi-workflows.content-chunk.v1",
      runId,
      path: record.path,
      mediaType: record.mediaType,
      bytes: record.bytes.byteLength,
      sha256: record.sha256,
      offset,
      nextOffset,
      complete: nextOffset === record.bytes.byteLength,
      data: record.bytes.subarray(offset, nextOffset).toString("base64"),
    };
  }

  private projectStep(runId: string, step: WorkflowStepRecord): JsonValue {
    const projected = toJson(step);
    if (!isJsonObject(projected)) return projected;
    for (const field of ["prompt", "output", "assistantMessage"] as const) {
      const value = projected[field];
      if (value !== undefined) projected[field] = this.projectValue(runId, value);
    }
    return projected;
  }

  private projectTraceEvent(runId: string, event: WorkflowTraceEvent): JsonValue {
    return this.projectRecordField(runId, event, "payload");
  }

  private projectSessionEntry(runId: string, entry: WorkflowSessionEntryRecord): JsonValue {
    return this.projectRecordField(runId, entry, "entry");
  }

  private projectSessionEvent(runId: string, event: WorkflowSessionEventRecord): JsonValue {
    return this.projectRecordField(runId, event, "payload");
  }

  private projectUpdate(runId: string, update: WorkflowUpdateRecord): JsonValue {
    return this.projectRecordField(runId, update, "data");
  }

  private projectReplayCheckpoint(runId: string, value: JsonValue | null): JsonValue {
    return value === null ? null : this.projectValue(runId, value);
  }

  private projectRecordField(runId: string, value: unknown, field: string): JsonValue {
    const projected = toJson(value);
    if (isJsonObject(projected)) {
      const fieldValue = projected[field];
      if (fieldValue !== undefined) projected[field] = this.projectValue(runId, fieldValue);
    }
    return projected;
  }

  private projectState(
    runId: string,
    stateValue: WorkflowRunState,
    steps: JsonValue[],
    updates: JsonValue[],
  ): JsonValue {
    const state = toJson(stateValue);
    if (!isJsonObject(state)) return state;
    for (const field of ["input", "outputs", "results", "humanDecision", "finalOutput"] as const) {
      const value = state[field];
      if (value !== undefined) state[field] = this.projectValue(runId, value);
    }
    state.steps = steps;
    state.updates = updates;
    return state;
  }

  private projectWorkflow(runId: string, value: unknown): JsonValue {
    const original = toJson(value);
    const workflow = escapeArtifactSentinels(original);
    if (Buffer.byteLength(canonicalJson(workflow)) <= VIEW_PAGE_BYTES * 2) return workflow;
    if (
      !isJsonObject(workflow) ||
      !isJsonObject(workflow.nodes) ||
      typeof workflow.schema !== "string" ||
      typeof workflow.name !== "string" ||
      typeof workflow.startAt !== "string" ||
      !Array.isArray(workflow.edges)
    ) {
      return this.registerContent(runId, original, "application/json");
    }
    const allNodeEntries = Object.entries(workflow.nodes);
    // A node identity larger than one bounded value is left out, never cut, so one
    // node entry cannot exceed the page budget on its own. The complete definition
    // stays reachable through the content reference below.
    const nodeEntries = allNodeEntries.filter(
      ([nodeId]) => Buffer.byteLength(nodeId, "utf8") <= NODE_ID_MAX_BYTES,
    );
    const boundedNodeEntries = byteBoundedForwardPage(nodeEntries, ([nodeId, node]) => [
      nodeId,
      this.projectWorkflowNode(runId, node),
    ]);
    const nodes = Object.fromEntries(
      boundedNodeEntries.flatMap((entry) =>
        Array.isArray(entry) && typeof entry[0] === "string" && entry[1] !== undefined
          ? [[entry[0], entry[1]]]
          : [],
      ),
    );
    const edges = byteBoundedForwardPage(workflow.edges, (edge) => this.projectValue(runId, edge));
    return {
      schema: workflow.schema,
      // One free-form value must not push a run frame past the client limit. The
      // complete definition stays in the run content, reachable by its digest.
      name: boundSessionText(workflow.name),
      startAt: workflow.startAt,
      nodes,
      nodeStart: 0,
      nodeTotal: allNodeEntries.length,
      edges,
      edgeStart: 0,
      edgeTotal: workflow.edges.length,
      content: this.registerContent(runId, original, "application/json"),
    };
  }

  private projectWorkflowNode(runId: string, value: JsonValue): JsonValue {
    if (!isJsonObject(value)) return this.projectValue(runId, value);
    const projected: Record<string, JsonValue> = {};
    for (const field of [
      "nodeType",
      "timeoutMs",
      "statusDetail",
      "actionExecution",
      "settingsRoute",
      "effect",
      "mountPath",
      "localNodeId",
      "includeTransition",
    ]) {
      const fieldValue = value[field];
      if (fieldValue !== undefined) projected[field] = this.projectValue(runId, fieldValue);
    }
    return projected;
  }

  private projectDisplay(runId: string, display: WorkflowDisplay): WorkflowDisplay {
    if (
      display.reason === null ||
      Buffer.byteLength(display.reason, "utf8") <= INLINE_CONTENT_BYTES
    ) {
      return display;
    }
    return {
      ...display,
      reason: "Complete workflow failure details are available.",
      reasonContent: this.registerContent(runId, display.reason, "text/plain"),
    };
  }

  private projectValue(runId: string, value: JsonValue): JsonValue {
    const safeValue = escapeArtifactSentinels(value);
    const mediaType = typeof value === "string" ? "text/plain" : "application/json";
    const bytes =
      mediaType === "text/plain"
        ? Buffer.from(value as string, "utf8")
        : Buffer.from(canonicalJson(value), "utf8");
    return bytes.byteLength <= INLINE_CONTENT_BYTES
      ? safeValue
      : this.registerContent(runId, value, mediaType);
  }

  private registerContent(
    runId: string,
    value: JsonValue,
    mediaType: ContentRecord["mediaType"],
  ): JsonValue {
    const bytes = Buffer.from(
      mediaType === "text/plain" ? (value as string) : canonicalJson(value),
      "utf8",
    );
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    const persistedDigest = this.runs.persistViewContent(runId, bytes, mediaType);
    if (persistedDigest !== sha256) throw new Error("Workflow view content digest changed");
    const extension = mediaType === "text/plain" ? "txt" : "json";
    const contentPath = `artifacts/sha256/${sha256}.${extension}`;
    this.rememberContent({
      runId,
      path: contentPath,
      mediaType,
      bytes,
      sha256,
    });
    return {
      $artifact: {
        path: contentPath,
        mediaType,
        bytes: bytes.byteLength,
        sha256,
        opaque: true,
      },
    };
  }

  private rememberContent(record: ContentRecord): ContentRecord {
    const key = contentKey(record.runId, record.path);
    const previous = this.contentRecords.get(key);
    if (previous !== undefined) this.contentBytes -= previous.bytes.byteLength;
    this.contentRecords.delete(key);
    this.contentRecords.set(key, record);
    this.contentBytes += record.bytes.byteLength;
    while (this.contentBytes > CONTENT_CACHE_BYTES && this.contentRecords.size > 1) {
      const oldest = this.contentRecords.entries().next().value as
        | [string, ContentRecord]
        | undefined;
      if (oldest === undefined) break;
      this.contentRecords.delete(oldest[0]);
      this.contentBytes -= oldest[1].bytes.byteLength;
    }
    return record;
  }

  private recoverContent(runId: string, contentPath: string): ContentRecord | undefined {
    const match = /^artifacts\/sha256\/([0-9a-f]{64})\.(json|txt)$/u.exec(contentPath);
    if (match === null) return undefined;
    const mediaType = match[2] === "txt" ? "text/plain" : "application/json";
    const blob = this.runs.readContentBlob(runId, match[1] as string, mediaType);
    if (blob === undefined) return undefined;
    return this.rememberContent({
      runId,
      path: contentPath,
      mediaType,
      bytes: blob.content,
      sha256: match[1] as string,
    });
  }

  clearConnection(_connectionId: string): void {
    // Coordinator fencing is process-local in the server. Durable turn state is
    // reconciled from the Pi branch after the next coordinator connects.
  }

  /** The retained terminal run of one session, if the session view shows one. */
  private retainedTerminalRunId(sessionId: string, now: number = Date.now()): string | undefined {
    return this.retainedTerminalForView(sessionId, now)?.runId;
  }

  /**
   * The retained terminal run of one session, with the moment its retention window
   * closes. A run kept by a waiting message or a triggering turn has no such moment,
   * because its state and not the clock holds it.
   */
  private retainedTerminalForView(
    sessionId: string,
    now: number,
  ): { runId: string; expiresAt: number | null } | undefined {
    // The walk reads bounded batches of retained terminal metadata, newest first,
    // and stops at the first retained run. Its candidate filter excludes every
    // terminal message whose retention window has passed.
    const cutoff = now - TERMINAL_VIEW_RETENTION_MS;
    for (const message of this.walkSessionSummaries(sessionId, {
      status: null,
      newestFirst: true,
      kind: "terminal",
      filter: "retainedTerminal",
      cutoff,
    })) {
      const run = this.state.connection
        .prepare("SELECT status FROM runs WHERE run_id = ?")
        .get(message.runId);
      if (!isObjectRecord(run) || !isTerminalStatus(run.status)) continue;
      const clear = this.state.connection
        .prepare(
          `SELECT cleared_run_id AS clearedRunId, cleared_at AS clearedAt
           FROM session_terminal_views WHERE target_session_id = ?`,
        )
        .get(sessionId);
      if (
        isObjectRecord(clear) &&
        clear.clearedRunId === message.runId &&
        typeof clear.clearedAt === "number" &&
        clear.clearedAt >= Date.parse(message.createdAt)
      ) {
        continue;
      }
      if (message.status === "pending") return { runId: message.runId, expiresAt: null };
      if (message.status !== "sent") continue;
      const turn = this.workflowMessages.latestTurnForMessage(message.workflowMessageId);
      if (
        message.triggerTurn &&
        !recoveryStopped(this.state, message.workflowMessageId) &&
        (turn === undefined || turn.state === "started")
      ) {
        return { runId: message.runId, expiresAt: null };
      }
      const expiresAt = Date.parse(turn?.endedAt ?? message.updatedAt) + TERMINAL_VIEW_RETENTION_MS;
      if (expiresAt > now) return { runId: message.runId, expiresAt };
    }
    return undefined;
  }

  hasCancelledSource(message: WorkflowMessageSummary): boolean {
    if (message.kind === "terminal")
      return message.triggerTurn && recoveryStopped(this.state, message.workflowMessageId);
    if (message.kind === "step") {
      const request = this.state.connection
        .prepare("SELECT status FROM interactive_requests WHERE request_id = ? AND run_id = ?")
        .get(message.sourceId, message.runId);
      return isObjectRecord(request) && request.status === "cancelled";
    }
    if (message.kind === "followUp") {
      const source = this.state.connection
        .prepare("SELECT status FROM workflow_follow_ups WHERE follow_up_id = ? AND run_id = ?")
        .get(message.sourceId, message.runId);
      return (
        isObjectRecord(source) && (source.status === "cancelled" || source.status === "removed")
      );
    }
    return false;
  }

  private isMessageEligible(message: WorkflowMessageSummary): boolean {
    if (message.status !== "pending") return false;
    if (message.kind === "step" || message.kind === "decision") {
      const request = this.state.connection
        .prepare(
          `SELECT i.status, r.paused FROM interactive_requests i
           JOIN runs r ON r.run_id = i.run_id
           WHERE i.request_id = ? AND i.run_id = ?`,
        )
        .get(message.sourceId, message.runId);
      if (!isObjectRecord(request) || request.status !== "pending") return false;
      return message.kind === "decision" || request.paused === 0;
    }
    if (message.kind === "notification") return true;
    if (message.kind === "terminal") {
      const reservation = this.state.connection
        .prepare(
          `SELECT 1 FROM run_bindings b JOIN runs r ON r.run_id = b.run_id
         WHERE b.origin_session_id = ? AND r.run_id <> ? AND r.status IN ('queued', 'running', 'waiting') LIMIT 1`,
        )
        .get(message.targetSessionId, message.runId);
      if (reservation !== undefined) return false;
      const run = this.state.connection
        .prepare("SELECT status FROM runs WHERE run_id = ?")
        .get(message.runId);
      return (
        isObjectRecord(run) &&
        isTerminalStatus(run.status) &&
        (!message.triggerTurn || !recoveryStopped(this.state, message.workflowMessageId))
      );
    }
    const source = this.state.connection
      .prepare(
        `SELECT f.run_id AS runId, f.order_number AS orderNumber, f.status
         FROM workflow_follow_ups f WHERE f.follow_up_id = ?`,
      )
      .get(message.sourceId);
    if (
      !isObjectRecord(source) ||
      source.status !== "queued" ||
      typeof source.orderNumber !== "number" ||
      typeof source.runId !== "string"
    ) {
      return false;
    }
    const run = this.state.connection
      .prepare("SELECT status FROM runs WHERE run_id = ?")
      .get(source.runId);
    if (!isObjectRecord(run) || run.status !== "completed") return false;
    const terminal = this.workflowMessages
      .listRunSummaries(source.runId)
      .filter((candidate) => candidate.kind === "terminal" && candidate.status === "sent")
      .at(-1);
    if (terminal === undefined) return false;
    if (terminal.triggerTurn) {
      const turn = this.workflowMessages.latestTurnForMessage(terminal.workflowMessageId);
      if (
        turn?.state !== "ended" ||
        turn.stopReason !== "completed" ||
        recoveryStopped(this.state, terminal.workflowMessageId)
      )
        return false;
    }
    const prior = this.state.connection
      .prepare(
        `SELECT follow_up_id AS followUpId, status FROM workflow_follow_ups
         WHERE run_id = ? AND order_number < ? ORDER BY order_number`,
      )
      .all(source.runId, source.orderNumber);
    for (const item of prior) {
      if (
        !isObjectRecord(item) ||
        typeof item.followUpId !== "string" ||
        typeof item.status !== "string"
      ) {
        return false;
      }
      if (item.status === "removed" || item.status === "cancelled") continue;
      const priorMessage = this.workflowMessages.latestForSource("followUp", item.followUpId);
      if (
        priorMessage === undefined ||
        this.workflowMessages.latestTurnForMessage(priorMessage.workflowMessageId)?.state !==
          "ended"
      ) {
        return false;
      }
    }
    const reservation = this.state.connection
      .prepare(
        `SELECT 1 AS present FROM run_bindings b JOIN runs r ON r.run_id = b.run_id
         WHERE b.origin_session_id = ? AND r.run_id <> ?
           AND r.status IN ('queued', 'running', 'waiting') LIMIT 1`,
      )
      .get(message.targetSessionId, message.runId);
    return reservation === undefined;
  }

  private openWorkflowMessage(
    messages: readonly WorkflowMessageSummary[],
  ): WorkflowMessageSummary | undefined {
    for (const message of [...messages].reverse()) {
      if (message.status !== "sent") continue;
      if (message.kind === "step") {
        const request = this.state.connection
          .prepare(
            `SELECT i.status, r.paused FROM interactive_requests i
             JOIN runs r ON r.run_id = i.run_id WHERE i.request_id = ?`,
          )
          .get(message.sourceId);
        if (isObjectRecord(request) && request.status === "pending" && request.paused === 0) {
          return message;
        }
      } else if (
        message.kind === "followUp" ||
        (message.kind === "terminal" &&
          message.triggerTurn &&
          !recoveryStopped(this.state, message.workflowMessageId))
      ) {
        const turn = this.workflowMessages.latestTurnForMessage(message.workflowMessageId);
        if (turn === undefined || turn.state === "started") return message;
      }
    }
    return undefined;
  }

  private workflowActivityRevision(runId?: string): string {
    const row =
      runId === undefined
        ? this.state.connection
            .prepare(
              `SELECT
                 COALESCE((SELECT max(updated_at) FROM workflow_messages), 0) AS messageUpdatedAt,
                 COALESCE((SELECT max(COALESCE(ended_at, started_at)) FROM workflow_turns), 0) AS turnUpdatedAt`,
            )
            .get()
        : this.state.connection
            .prepare(
              `SELECT
                 COALESCE((SELECT max(updated_at) FROM workflow_messages WHERE run_id = ?), 0) AS messageUpdatedAt,
                 COALESCE((SELECT max(COALESCE(ended_at, started_at)) FROM workflow_turns WHERE run_id = ?), 0) AS turnUpdatedAt`,
            )
            .get(runId, runId);
    return isObjectRecord(row) &&
      typeof row.messageUpdatedAt === "number" &&
      typeof row.turnUpdatedAt === "number"
      ? `${row.messageUpdatedAt}:${row.turnUpdatedAt}${this.activityRevision === 0 ? "" : `-${this.activityRevision}`}`
      : `0:0${this.activityRevision === 0 ? "" : `-${this.activityRevision}`}`;
  }

  private display(
    queue: WorkflowRunQueueRecord | WorkflowRunQueueViewRecord,
    state?: WorkflowRunDisplayState | WorkflowRunState | null,
  ): WorkflowDisplay {
    return reduceWorkflowDisplay({
      queueStatus: queue.status,
      durableStatus: state?.status,
      paused: state?.paused === true,
      ambiguous: this.hasAmbiguousEffect(queue.runId),
      runnerActive: this.hasLiveRunner(queue.runId),
      originTurnActive: this.hasActivity(queue.runId),
      pendingRequestKind: this.pendingRequestKind(queue.runId),
      requestDeliveryConfirmed: this.requestDeliveryConfirmed(queue.runId),
      errorMessage: state?.error ?? queue.errorMessage,
    });
  }

  private hasActivity(runId: string): boolean {
    const row = this.state.connection
      .prepare(
        `SELECT t.target_session_id AS targetSessionId FROM workflow_turns t
         JOIN workflow_messages m ON m.workflow_message_id = t.workflow_message_id
         WHERE t.run_id = ? AND t.state = 'started'
           AND m.kind IN ('step', 'terminal', 'followUp') LIMIT 1`,
      )
      .get(runId);
    return (
      isObjectRecord(row) &&
      typeof row.targetSessionId === "string" &&
      this.hasActiveSessionTurn(row.targetSessionId)
    );
  }

  private pendingRequestKind(runId: string): WorkflowDisplayFacts["pendingRequestKind"] {
    const row = this.state.connection
      .prepare("SELECT kind FROM interactive_requests WHERE run_id = ? AND status = 'pending'")
      .get(runId) as
      | { kind: Exclude<WorkflowDisplayFacts["pendingRequestKind"], null> }
      | undefined;
    return row?.kind ?? null;
  }

  private requestDeliveryConfirmed(runId: string): boolean {
    const row = this.state.connection
      .prepare(
        `SELECT m.status FROM interactive_requests i
       JOIN workflow_messages m ON m.source_id = i.request_id AND m.kind = 'step'
       WHERE i.run_id = ? AND i.status = 'pending'
       ORDER BY m.order_number DESC LIMIT 1`,
      )
      .get(runId) as { status: string } | undefined;
    return row?.status === "sent";
  }

  private hasAmbiguousEffect(runId: string): boolean {
    const row = this.state.connection
      .prepare(
        `SELECT 1 AS present FROM effects e JOIN runs r ON r.resource_id = e.source_resource_id
         WHERE r.run_id = ? AND e.status = 'ambiguous' LIMIT 1`,
      )
      .get(runId);
    return row !== undefined;
  }

  private runVersion(runId: string): string {
    const row = this.state.connection
      .prepare(
        `SELECT res.revision, r.status AS runStatus, r.paused,
                q.updated_at AS updatedAt,
                COALESCE(v.presentation_revision, 0) AS presentationRevision
         FROM runs r JOIN resources res ON res.resource_id = r.resource_id
         JOIN run_queue q ON q.run_id = r.run_id
         LEFT JOIN viewer_runs v ON v.run_id = r.run_id
         WHERE r.run_id = ?`,
      )
      .get(runId);
    if (!isRunVersionRow(row)) return "missing";
    return [
      row.revision,
      row.updatedAt,
      row.presentationRevision,
      row.runStatus,
      row.paused,
      this.workflowActivityRevision(runId),
      this.hasLiveRunner(runId),
      this.hasActivity(runId),
      this.pendingRequestKind(runId),
      this.requestDeliveryConfirmed(runId),
      this.hasAmbiguousEffect(runId),
    ].join(":");
  }

  private pendingSessionRevision(sessionId: string): string {
    const row = this.state.connection
      .prepare(
        `SELECT count(*) AS count, COALESCE(sum(revision), 0) AS revisionSum,
                COALESCE(max(updated_at), 0) AS updatedAt
         FROM interactive_requests
         WHERE target_session_id = ? AND status = 'pending'`,
      )
      .get(sessionId);
    if (!isSessionRevisionRow(row)) throw new Error("Session view revision is invalid");
    return `${row.count}:${row.revisionSum}:${row.updatedAt}`;
  }

  private presentationRevision(runId: string): number {
    const row = this.state.connection
      .prepare(`SELECT presentation_revision AS revision FROM viewer_runs WHERE run_id = ?`)
      .get(runId);
    return isRevisionRow(row) ? row.revision : 0;
  }
}

export type WorkflowDisplayFacts = {
  queueStatus: WorkflowRunQueueRecord["status"];
  durableStatus: WorkflowRunState["status"] | undefined;
  paused: boolean;
  ambiguous: boolean;
  runnerActive: boolean;
  originTurnActive: boolean;
  pendingRequestKind: "agent" | "assistant" | "checkpoint" | "decision" | null;
  requestDeliveryConfirmed: boolean;
  errorMessage: string | null;
};

export function reduceWorkflowDisplay(facts: WorkflowDisplayFacts): WorkflowDisplay {
  let status: WorkflowDisplayStatus;
  const activity: WorkflowDisplay["activity"] = facts.runnerActive
    ? "supervised_runner"
    : facts.originTurnActive
      ? "origin_turn"
      : null;
  let reason: string | null = null;

  if (
    facts.durableStatus === "completed" ||
    facts.durableStatus === "failed" ||
    facts.durableStatus === "timed_out" ||
    facts.durableStatus === "cancelled"
  ) {
    status = facts.durableStatus;
    reason = facts.ambiguous
      ? "The run is terminal, but an external effect still needs explicit recovery."
      : facts.originTurnActive
        ? "Execution has ended. The model is reviewing the result and safe next actions."
        : facts.errorMessage;
  } else if (facts.ambiguous) {
    status = "ambiguous";
    reason = "An external effect needs explicit recovery.";
  } else if (facts.paused) {
    status = "paused";
    reason = "The workflow is durably paused.";
  } else if (activity !== null) {
    status = "running";
    reason = facts.originTurnActive
      ? "The agent is working on the workflow step."
      : "The workflow runner is working.";
  } else if (facts.pendingRequestKind !== null || facts.durableStatus === "waiting") {
    status = "waiting";
    reason =
      facts.pendingRequestKind === "decision"
        ? "The workflow needs a protected human decision."
        : facts.pendingRequestKind === "checkpoint"
          ? "The workflow needs a checkpoint answer."
          : facts.pendingRequestKind === "agent"
            ? "The workflow needs its assigned agent result."
            : facts.pendingRequestKind === "assistant"
              ? "The workflow needs its assigned visible response."
              : "The workflow is waiting.";
    if (facts.pendingRequestKind === "agent" || facts.pendingRequestKind === "assistant") {
      if (!facts.requestDeliveryConfirmed)
        reason = "A step is pending delivery. It starts a new model turn after this turn ends.";
    }
  } else if (facts.queueStatus === "parked" || facts.queueStatus === "queued") {
    status = "queued";
    reason =
      facts.queueStatus === "parked"
        ? (facts.errorMessage ?? "The workflow is ready to resume.")
        : null;
  } else if (facts.queueStatus === "done") {
    status = "completed";
  } else if (facts.queueStatus === "failed" || facts.queueStatus === "cancelled") {
    status = facts.queueStatus;
    reason = facts.errorMessage;
  } else {
    status = "running";
  }

  const controls: WorkflowDisplay["controls"] = [];
  if (status === "running" || status === "waiting") controls.push("pause", "cancel");
  else if (status === "paused") controls.push("resume", "cancel");
  else if (status === "queued") {
    if (facts.queueStatus === "parked") controls.push("resume");
    controls.push("cancel");
  }
  if (status === "waiting" || status === "running") {
    if (facts.pendingRequestKind === "checkpoint") controls.push("answer");
    if (facts.pendingRequestKind === "decision") controls.push("human-answer");
    if (facts.pendingRequestKind === "agent") controls.push("update", "submit");
  }
  if (["completed", "failed", "timed_out"].includes(status) && facts.originTurnActive)
    controls.push("cancel");
  if (facts.ambiguous) controls.push("review");
  return { status, activity, controls, reason };
}

function manifest(
  run: WorkflowRunQueueRecord | WorkflowRunQueueViewRecord,
  status: WorkflowDisplayStatus,
): JsonValue {
  return {
    schema: "pi-workflows.run-manifest.v1",
    runId: run.runId,
    // The complete name stays in the run definition, so the list frame cannot
    // grow with one free-form value.
    workflowName: boundSessionText(run.workflowName),
    workflowSource: workflowRootSource(run.workflowSource),
    startedAt: run.startedAt ?? run.createdAt,
    ...(run.finishedAt === null ? {} : { finishedAt: run.finishedAt }),
    status,
    traceSchema: "pi-workflows.trace-event.v1",
    paths: {
      workflow: "server",
      state: "server",
      trace: "server",
    },
  };
}

// The widget shows a bounded set of progress rows. A run keeps up to 1,024
// current updates, so the newest keys are the ones that matter.
const MAX_SESSION_PROGRESS_UPDATES = 16;

type NodeRecord = {
  nodeType?: unknown;
  actionExecution?: unknown;
  summary?: unknown;
  humanDecision?: unknown;
  expectedOutput?: unknown;
};

type NodeAttemptFacts = {
  attempts: number;
  activeStatus: string | null;
  lastStatus: string | null;
  lastSettingsChangeNumber: number | null;
  startedAt: string | null;
  durationMs: number | null;
  errorHash: Buffer | null;
};

type NodeAttemptRow = {
  nodeId: string;
  status: string;
  attemptNumber: number;
  startedAt: number | null;
  finishedAt: number | null;
  settingsChangeNumber: number | null;
  errorHash: Buffer | null;
};

function isNodeAttemptRow(value: unknown): value is NodeAttemptRow {
  if (!isJsonObject(value)) return false;
  return (
    typeof value.nodeId === "string" &&
    typeof value.status === "string" &&
    typeof value.attemptNumber === "number" &&
    (value.startedAt === null || typeof value.startedAt === "number") &&
    (value.finishedAt === null || typeof value.finishedAt === "number") &&
    (value.settingsChangeNumber === null || typeof value.settingsChangeNumber === "number") &&
    (value.errorHash === null || Buffer.isBuffer(value.errorHash))
  );
}

function isActiveAttemptStatus(status: string): boolean {
  return status === "pending" || status === "running" || status === "waiting";
}

function nodeRecords(snapshot: unknown): Record<string, NodeRecord> {
  if (!isJsonObject(snapshot) || !isJsonObject(snapshot.nodes)) return {};
  const records: Record<string, NodeRecord> = {};
  for (const [nodeId, node] of Object.entries(snapshot.nodes)) {
    if (isJsonObject(node)) records[nodeId] = node as NodeRecord;
  }
  return records;
}

function nodeRowState(
  state: WorkflowRunState,
  nodeId: string,
  facts: NodeAttemptFacts | undefined,
): WorkflowSessionNodeRow["state"] {
  if (facts?.activeStatus === "running") return "running";
  if (facts?.activeStatus === "waiting") return "waiting";
  if (facts?.activeStatus === "pending") return "pending";
  const result = state.results[nodeId];
  if (result !== undefined) return result.outcome === "ok" ? "ok" : "failed";
  if (facts?.lastStatus === "completed") return "ok";
  if (facts !== undefined && facts.lastStatus !== null) return "failed";
  return "pending";
}

function sessionActionExecution(node: NodeRecord): "function" | "shell" | null {
  return node.actionExecution === "shell" || node.actionExecution === "function"
    ? node.actionExecution
    : null;
}

function isAssistantResponseNode(node: NodeRecord): boolean {
  return (
    node.nodeType === "agent" &&
    isJsonObject(node.expectedOutput) &&
    node.expectedOutput.kind === "assistant-message"
  );
}

function sessionHumanDecision(
  node: NodeRecord,
  state: WorkflowRunState,
  nodeId: string,
): WorkflowSessionNodeRow["humanDecision"] {
  const human = isJsonObject(node.humanDecision) ? node.humanDecision : undefined;
  if (human === undefined) return null;
  const request = humanDecisionRequest(state.finalOutput);
  const current = request !== undefined && request.nodeId === nodeId ? request : undefined;
  const audience =
    current?.audience ?? (typeof human.audience === "string" ? human.audience : "human");
  const choices = isJsonObject(human.choices)
    ? sessionDecisionChoices(
        Object.entries(human.choices).flatMap(([value, choice]) =>
          isJsonObject(choice) && typeof choice.label === "string"
            ? [{ value, label: choice.label }]
            : [],
        ),
      )
    : [];
  const choiceValue =
    state.humanDecision !== undefined && state.humanDecision.nodeId === nodeId
      ? state.humanDecision.response.choice
      : null;
  return {
    audience,
    summary: current?.summary === undefined ? null : boundNodeText(current.summary),
    choices,
    choiceValue,
    presentationDigest: current?.presentationDigest ?? null,
  };
}

/**
 * Decision choices one session node row may carry. The widget joins the labels
 * into one line it truncates to the terminal width, and the 1 MiB frame budget
 * requires the bound. The complete decision stays available through the detailed
 * run view and its decision record.
 */
function sessionDecisionChoices(
  entries: readonly { value: string; label: string }[],
): { value: string; label: string }[] {
  const choices: { value: string; label: string }[] = [];
  let bytes = 0;
  for (const entry of entries) {
    const label = boundSessionText(entry.label);
    const size = Buffer.byteLength(label, "utf8") + Buffer.byteLength(entry.value, "utf8");
    // Keep at least one choice so the row still names a choice the human sees.
    if (choices.length > 0 && bytes + size > SESSION_DETAIL_JSON_BYTES) break;
    bytes += size;
    choices.push({ value: entry.value, label });
  }
  return choices;
}

function humanDecisionRequest(
  value: unknown,
):
  | { nodeId: string; audience: string; summary: string | null; presentationDigest: string | null }
  | undefined {
  if (!isJsonObject(value)) return undefined;
  if (value.schema !== "pi-workflows.human-decision-request.v1") return undefined;
  if (typeof value.nodeId !== "string" || typeof value.audience !== "string") return undefined;
  // A node identity and an audience name identify state, so one that cannot travel
  // in a bounded frame is left out and never cut. The summary is free-form display
  // text, which is cut at the shared session bound.
  if (
    Buffer.byteLength(value.nodeId, "utf8") > NODE_ID_MAX_BYTES ||
    Buffer.byteLength(value.audience, "utf8") > SESSION_TEXT_BYTES
  ) {
    return undefined;
  }
  const presentation = isJsonObject(value.presentation) ? value.presentation : undefined;
  return {
    nodeId: value.nodeId,
    audience: value.audience,
    summary:
      typeof presentation?.summary === "string" ? boundSessionText(presentation.summary) : null,
    presentationDigest:
      typeof value.presentationDigest === "string" &&
      Buffer.byteLength(value.presentationDigest, "utf8") <= SESSION_TEXT_BYTES
        ? value.presentationDigest
        : null,
  };
}

function sessionProgressUpdates(
  updates: readonly WorkflowUpdateRecord[],
): WorkflowSessionProgressUpdate[] {
  const progress: WorkflowSessionProgressUpdate[] = [];
  for (const update of updates) {
    if (update.type !== "progress") continue;
    const data = boundedSessionJson(toJson(update.data));
    // A payload larger than one session detail is left out rather than cut, so
    // the compact view never carries a value that looks complete but is not.
    if (data === null) continue;
    progress.push({ key: update.key, at: update.at, data });
  }
  // Updates arrive in run revision order, so a bounded set keeps the newest keys
  // instead of the oldest ones.
  return progress.slice(-MAX_SESSION_PROGRESS_UPDATES);
}

function sessionMonitorSchedule(
  updates: readonly WorkflowUpdateRecord[],
): { nextCheckAt: string; recordedAt: string } | null {
  for (const update of updates) {
    if (update.type !== "monitor.schedule" || update.key !== "next-check") continue;
    if (!isJsonObject(update.data) || typeof update.data.nextCheckAt !== "string") continue;
    // A schedule that cannot travel in a bounded frame is left out rather than cut,
    // because a cut instant is another instant. The recorded update stays available
    // through the run update page.
    if (Buffer.byteLength(update.data.nextCheckAt, "utf8") > SESSION_TEXT_BYTES) return null;
    return { nextCheckAt: update.data.nextCheckAt, recordedAt: update.at };
  }
  return null;
}

function projectQueue(
  run: WorkflowRunQueueRecord | WorkflowRunQueueViewRecord,
): WorkflowRunQueueView {
  return {
    runId: run.runId,
    workflowName: boundSessionText(run.workflowName),
    workflowSourceRef: run.workflowSourceRef,
    initialized: run.initialized,
    definitionDigest: run.definitionDigest,
    status: run.status,
    originSessionId: run.originSessionId,
    executionMode: run.executionMode,
    parentRunId: run.parentRunId,
    rootRunId: run.rootRunId,
    lineageKind: run.lineageKind,
    restartNumber: run.restartNumber,
    parentRunRevision: run.parentRunRevision,
    errorCode: run.errorCode,
    createdAt: run.createdAt,
    updatedAt: run.updatedAt,
    startedAt: run.startedAt,
    finishedAt: run.finishedAt,
  };
}

function workflowRootSource(value: unknown): JsonValue {
  const sourceSet = toJson(value);
  if (!isJsonObject(sourceSet)) throw new Error("Workflow queue source set is invalid");
  const root = sourceSet.root;
  if (root === undefined || !isJsonObject(root)) {
    throw new Error("Workflow queue source set is invalid");
  }
  return root;
}

function isObjectRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isTerminalStatus(
  value: unknown,
): value is "completed" | "failed" | "timed_out" | "cancelled" {
  return (
    value === "completed" || value === "failed" || value === "timed_out" || value === "cancelled"
  );
}

function contentKey(runId: string, contentPath: string): string {
  return `${runId}\u0000${contentPath}`;
}

function escapeArtifactSentinels(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map(escapeArtifactSentinels);
  if (!isJsonObject(value)) return value;
  const escaped = Object.fromEntries(
    Object.entries(value).map(([key, item]) => [key, escapeArtifactSentinels(item)]),
  ) as JsonValue;
  return Object.keys(value).length === 1 &&
    (Object.hasOwn(value, "$artifact") || Object.hasOwn(value, "$escaped"))
    ? { $escaped: escaped }
    : escaped;
}

function projectFollowUpQueue(value: unknown, items: JsonValue[]): JsonValue {
  const queue = toJson(value);
  if (!isJsonObject(queue)) return queue;
  delete queue.followUps;
  queue.items = items;
  return queue;
}

function refreshCacheEntry<K, V>(cache: Map<K, V>, key: K, value: V): void {
  cache.delete(key);
  cache.set(key, value);
}

function rememberCacheEntry<K, V>(cache: Map<K, V>, key: K, value: V): void {
  refreshCacheEntry(cache, key, value);
  while (cache.size > VIEW_CACHE_ITEMS) {
    const oldest = cache.keys().next().value as K | undefined;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

function viewRange(total: number, cursor?: number): { start: number; limit: number } {
  const start = workflowPageStart(total, cursor);
  return { start, limit: Math.min(VIEW_PAGE_ITEMS, Math.max(0, total - start)) };
}

function byteBoundedForwardPage<T>(
  values: readonly T[],
  project: (value: T) => JsonValue,
): JsonValue[] {
  const items: JsonValue[] = [];
  let bytes = 0;
  for (const value of values) {
    if (items.length >= VIEW_PAGE_ITEMS) break;
    const item = project(value);
    const itemBytes = Buffer.byteLength(canonicalJson(item)) + (items.length === 0 ? 0 : 1);
    if (items.length > 0 && bytes + itemBytes > VIEW_PAGE_BYTES) break;
    items.push(item);
    bytes += itemBytes;
  }
  return items;
}

function byteBoundedCandidatePage<T>(
  values: readonly T[],
  candidateStart: number,
  total: number,
  requestedCursor: number | undefined,
  project: (value: T) => JsonValue,
): { start: number; total: number; items: JsonValue[] } {
  if (values.length === 0) return { start: candidateStart, total, items: [] };
  const globalCursor = clampCursor(requestedCursor ?? Math.max(0, total - 1), total);
  const localCursor = Math.min(Math.max(0, globalCursor - candidateStart), values.length - 1);
  const page = byteBoundedPage(values, localCursor, project);
  return { start: candidateStart + page.start, total, items: page.items };
}

function byteBoundedPage<T>(
  values: readonly T[],
  requestedCursor: number | undefined,
  project: (value: T) => JsonValue,
): { start: number; total: number; items: JsonValue[] } {
  const total = values.length;
  if (total === 0) return { start: 0, total: 0, items: [] };
  const cursor = clampCursor(requestedCursor ?? total - 1, total);
  const selected = project(values[cursor] as T);
  const selectedBytes = Buffer.byteLength(canonicalJson(selected));
  const indexed = new Map<number, JsonValue>([[cursor, selected]]);
  let pageBytes = selectedBytes;
  let left = cursor - 1;
  let right = cursor + 1;
  let leftBlocked = false;
  let rightBlocked = false;
  let preferLeft = true;

  while (indexed.size < VIEW_PAGE_ITEMS && (!leftBlocked || !rightBlocked)) {
    const index = preferLeft ? left : right;
    const inRange = index >= 0 && index < total;
    if (!inRange) {
      if (preferLeft) leftBlocked = true;
      else rightBlocked = true;
    } else {
      const item = project(values[index] as T);
      const itemBytes = Buffer.byteLength(canonicalJson(item)) + 1;
      if (pageBytes + itemBytes > VIEW_PAGE_BYTES) {
        if (preferLeft) leftBlocked = true;
        else rightBlocked = true;
      } else {
        indexed.set(index, item);
        pageBytes += itemBytes;
        if (preferLeft) left -= 1;
        else right += 1;
      }
    }
    preferLeft = !preferLeft;
  }

  const ordered = [...indexed.entries()].sort(
    ([leftIndex], [rightIndex]) => leftIndex - rightIndex,
  );
  return {
    start: ordered[0]?.[0] ?? cursor,
    total,
    items: ordered.map(([, item]) => item),
  };
}

export function toCompactStepJson(step: WorkflowStepRecord): JsonValue {
  return toJson({
    attemptId: step.attemptId,
    nodeId: step.nodeId,
    nodeType: step.nodeType,
    outcome: step.outcome,
    startedAt: step.startedAt,
    finishedAt: step.finishedAt,
    prompt: null,
    output: null,
    ...(step.settingsScopeId === undefined ? {} : { settingsScopeId: step.settingsScopeId }),
    ...(step.settingsChangeNumber === undefined
      ? {}
      : { settingsChangeNumber: step.settingsChangeNumber }),
    ...(step.settingsHash === undefined ? {} : { settingsHash: step.settingsHash }),
  });
}

export function workflowPageStart(total: number, cursor?: number): number {
  if (total <= 256) return 0;
  if (cursor === undefined) return total - 256;
  const center = clampCursor(cursor, total);
  return Math.min(Math.max(0, center - 128), total - 256);
}

function clampCursor(cursor: number, total: number): number {
  return total === 0 ? 0 : Math.min(cursor, total - 1);
}

/** Text one session node row may carry, bounded by the client frame budget. */
function boundNodeText(value: string | null): string | null {
  return value === null ? null : boundSessionText(value);
}

/** Free-form session text, bounded for the single frame budget. */
function boundSessionText(value: string): string {
  const bytes = Buffer.from(value, "utf8");
  if (bytes.byteLength <= SESSION_TEXT_BYTES) return value;
  // The cut must fall on a character boundary. A cut inside a multi-byte sequence
  // would end the value with a replacement character the widget shows as text.
  let end = SESSION_TEXT_BYTES;
  while (end > 0 && ((bytes[end] ?? 0) & 0xc0) === 0x80) end -= 1;
  return bytes.subarray(0, end).toString("utf8");
}

/**
 * A node identity the session run reports as the node it works on. The window
 * leaves out a row whose own bytes exceed the frame budget, so an identity of
 * that size is left out here too. A cut identity would match no row and would
 * name a node that does not exist. The complete identity stays in the detailed
 * run view.
 */
function boundSessionNodeId(value: string | null | undefined): string | null {
  if (value === null || value === undefined) return null;
  return Buffer.byteLength(canonicalJson(value), "utf8") > NODE_ID_MAX_BYTES ? null : value;
}

/** One JSON detail, or null when the detail is too large for the session frame. */
function boundedSessionJson(value: JsonValue): JsonValue {
  return Buffer.byteLength(canonicalJson(value), "utf8") <= SESSION_DETAIL_JSON_BYTES
    ? value
    : null;
}

/**
 * The window follows the node the widget shows as working, with a little lead.
 * A run with no working node shows the node that carries the failure the widget
 * highlights, or its end, so the useful rows are in the first window.
 */
function defaultNodeWindowStart(
  rows: readonly WorkflowSessionNodeRow[],
  state: WorkflowRunState,
  failureNodeId?: string,
): number {
  // The widget projects the same rows, so a row that reports work is the row it
  // shows as working even when the run state itself carries no working node.
  let rowFocus: string | undefined;
  for (const row of rows) {
    if (row.state === "running" || row.state === "waiting") rowFocus = row.nodeId;
  }
  const focus =
    state.currentNode ?? state.waitingOn ?? rowFocus ?? failureNodeId ?? rows.at(-1)?.nodeId;
  const index = focus === undefined ? -1 : rows.findIndex((row) => row.nodeId === focus);
  return index < 0 ? 0 : Math.max(0, index - SESSION_NODE_LEAD);
}

/**
 * Byte- and item-bounded node rows that start at the requested row. Widget
 * scrolling asks for the row after the last one it holds, so a window always
 * begins where the caller asked and grows forward.
 */
function boundedNodeWindow(
  rows: readonly WorkflowSessionNodeRow[],
  start: number,
): { start: number; total: number; items: WorkflowSessionNodeRow[] } {
  const total = rows.length;
  if (total === 0) return { start: 0, total: 0, items: [] };
  const first = Math.min(Math.max(0, Math.floor(start)), total - 1);
  const items: WorkflowSessionNodeRow[] = [];
  let windowStart = first;
  let bytes = 0;
  for (let index = first; index < total && items.length < VIEW_PAGE_ITEMS; index += 1) {
    const row = rows[index] as WorkflowSessionNodeRow;
    const rowBytes = Buffer.byteLength(canonicalJson(toJson(row)), "utf8") + 1;
    // A row must fit by itself, or the window would never move past it. A row
    // whose own identity exceeds the budget is left out, and the window starts at
    // the next row instead, so one long node id cannot break the frame the client
    // needs. A window that already holds rows stops here, so its rows stay
    // contiguous and the next cursor is exact. Complete node history stays in the
    // detailed run view.
    if (rowBytes > VIEW_PAGE_BYTES) {
      if (items.length === 0) {
        windowStart = index + 1;
        continue;
      }
      break;
    }
    if (items.length > 0 && bytes + rowBytes > VIEW_PAGE_BYTES) break;
    bytes += rowBytes;
    items.push(row);
  }
  return { start: windowStart, total, items };
}

function toJson(value: unknown): JsonValue {
  return parseJson(canonicalJson(value));
}

function isJsonObject(value: unknown): value is { [key: string]: JsonValue } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isRunVersionRow(value: unknown): value is {
  revision: number;
  updatedAt: number;
  presentationRevision: number;
  runStatus: string;
  paused: number;
} {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { revision?: unknown }).revision === "number" &&
    typeof (value as { updatedAt?: unknown }).updatedAt === "number" &&
    typeof (value as { presentationRevision?: unknown }).presentationRevision === "number" &&
    typeof (value as { runStatus?: unknown }).runStatus === "string" &&
    typeof (value as { paused?: unknown }).paused === "number"
  );
}

function isSessionRevisionRow(value: unknown): value is {
  count: number;
  revisionSum: number;
  updatedAt: number;
} {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { count?: unknown }).count === "number" &&
    typeof (value as { revisionSum?: unknown }).revisionSum === "number" &&
    typeof (value as { updatedAt?: unknown }).updatedAt === "number"
  );
}

function isRevisionRow(value: unknown): value is { revision: number } {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { revision?: unknown }).revision === "number"
  );
}

````

### `src/workflows/types.ts` (complete file, repository pi-workflows)
````
export type MaybePromise<T> = T | Promise<T>;

/**
 * Context passed to node callbacks (prompt builders, compute/action runners,
 * validators). `outputs` maps node ids to their accepted outputs; `results`
 * maps node ids to the full result record of their latest attempt.
 */
export type WorkflowNodeContext<TInput = unknown, TSettings = unknown> = {
  input: TInput;
  outputs: Record<string, unknown>;
  results: Record<string, WorkflowNodeResult>;
  state: WorkflowRunState;
  /** Fixed settings value captured when this node attempt started, when declared. */
  settings?: TSettings;
  /** Stable root or included-workflow settings scope for this attempt. */
  settingsScopeId?: string;
  /** Accepted settings change number captured by this attempt. */
  settingsChangeNumber?: number;
  /**
   * Aborted when the node times out or the run is cancelled. Long-running
   * callbacks should observe it (pass it to fetch/spawn or check
   * `signal.aborted`) so side effects stop when the engine gives up on the
   * node.
   */
  signal: AbortSignal;
};

export type WorkflowUpdateInput = {
  type: string;
  key: string;
  data: Record<string, unknown>;
};

export type WorkflowUpdateRecord = {
  updateId: string;
  seq: number;
  at: string;
  runId: string;
  nodeId: string;
  attemptId: string;
  type: string;
  key: string;
  data: Record<string, unknown>;
};

export type WorkflowUpdateReceipt = Pick<
  WorkflowUpdateRecord,
  "updateId" | "seq" | "at" | "type" | "key"
>;

export type WorkflowEffectRecovery = "idempotent" | "manual";

export type WorkflowManagedEffect = {
  /** Stable external effect category, for example `github.comment`. */
  type: string;
  /** Canonical request data used to reject engine-owned key reuse with another operation. */
  request: unknown | ((context: WorkflowNodeContext) => MaybePromise<unknown>);
  /** `manual` never retries after an uncertain child exit. */
  recovery: WorkflowEffectRecovery;
};

export type WorkflowActionContext<TInput = unknown> = WorkflowNodeContext<TInput> & {
  effect: { type: string; idempotencyKey: string; recovery: WorkflowEffectRecovery };
  publishUpdate(update: WorkflowUpdateInput): Promise<WorkflowUpdateReceipt>;
};

export type WorkflowEffectReservation = {
  effectId: string;
  attemptNumber: number;
  disposition: "execute" | "adopted" | "ambiguous";
  result?: unknown;
};

export type WorkflowProgressStatus =
  | "pending"
  | "running"
  | "waiting"
  | "blocked"
  | "completed"
  | "failed"
  | "cancelled"
  | "unknown";

export type WorkflowProgressData = {
  schema: "pi-workflows.progress.v1";
  status: WorkflowProgressStatus;
  label?: string;
  phase?: string;
  completed?: number;
  total?: number;
  unit?: string;
  sourceUpdatedAt?: string;
  sourceEstimatedFinishAt?: string;
};

export type WorkflowNodeCommon = {
  /**
   * Per-node active-execution timeout or a callback that derives it from the run context.
   * Waiting, paused, disconnected, and server-down origin-session time does not count.
   * Null disables the deadline. Omission falls back to the engine default (15 minutes).
   */
  timeoutMs?: number | null | ((context: WorkflowNodeContext) => MaybePromise<number | null>);
  /** Short human-readable label shown in the viewer while the node runs. */
  statusDetail?: string;
};

/**
 * Edges route between nodes. A node has at most one outgoing edge: either a
 * plain `to` edge or a `switch` edge that routes on a JSON path into the
 * node's output (`$.field`, `$output.field`) or result (`$result.outcome`).
 */
export type WorkflowEdge =
  | {
      from: string;
      to: string;
    }
  | {
      from: string;
      switch: {
        on: string;
        cases: Record<string, string>;
      };
    };

/** A visible assistant response used as the accepted agent-node output. */
export type AssistantMessageOutput = {
  kind: "assistant-message";
  /** Optional author-supplied character limit. Omission adds no workflow limit. */
  maxChars?: number;
};

export type AgentExpectedOutput = string | AssistantMessageOutput;

type AgentNodeBase = WorkflowNodeCommon & {
  nodeType: "agent";
  /** Optional exact tool allowlist. Matching workflow submit/update calls remain allowed. */
  allowedTools?: string[];
  /**
   * Name of the acpx agent that runs this step, for example `grok-build` or `claude`. Omit it to
   * run on the run's own Pi (interactive session or headless child). Not `pi`: acpx's Pi agent
   * cannot receive the workflow tool. Cannot be combined with `allowedTools` or assistantMessage().
   */
  agent?: string;
  /** Model to select on `agent` before the step runs. Needs `agent`. */
  model?: string;
  prompt: (context: WorkflowNodeContext) => MaybePromise<string>;
};

/** The existing workflow-tool submission form. */
export type SubmittedAgentNodeDefinition = AgentNodeBase & {
  expectedOutput?: string;
  validate?: (output: unknown, context: WorkflowNodeContext) => MaybePromise<unknown>;
};

/** A model turn whose normal visible assistant text becomes the node output. */
export type AssistantAgentNodeDefinition = AgentNodeBase & {
  expectedOutput: AssistantMessageOutput;
  validate?: never;
};

/** A model-shaped step with either submitted or visible assistant output. */
export type AgentNodeDefinition = SubmittedAgentNodeDefinition | AssistantAgentNodeDefinition;

/** A pure local function: shape inputs, route, format, derive values. */
export type ComputeNodeDefinition = WorkflowNodeCommon & {
  nodeType: "compute";
  run: (context: WorkflowNodeContext) => MaybePromise<unknown>;
  /** Marks a pure route selector that may rerun before route settlement. */
  settingsRoute?: true;
};

/** A durable user-facing message addressed by the runtime to the run's origin session. */
export type NotifyNodeDefinition = WorkflowNodeCommon & {
  nodeType: "notify";
  message: (context: WorkflowNodeContext) => MaybePromise<string>;
  kind?: "progress" | "final";
};

/** A deterministic runtime-owned step implemented as a local function. */
export type FunctionActionNodeDefinition = WorkflowNodeCommon & {
  nodeType: "action";
  effect?: WorkflowManagedEffect;
  run: (context: WorkflowActionContext) => MaybePromise<unknown>;
};

export type ShellActionExecution = {
  command: string;
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  stdin?: string;
  shell?: boolean | string;
  allowNonZeroExit?: boolean;
  timeoutMs?: number;
  /** Cap on captured stdout/stderr each, default 1,000,000 characters. */
  maxOutputChars?: number;
};

export type ShellActionResult = {
  command: string;
  args: string[];
  cwd: string;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  signal: NodeJS.Signals | null;
  durationMs: number;
};

/** A deterministic runtime-owned step implemented as a shell command. */
export type ShellUpdateLine = {
  stream: "stdout" | "stderr";
  text: string;
};

export type ShellActionUpdates = {
  streams?: Array<"stdout" | "stderr">;
  parseLine: (
    line: ShellUpdateLine,
    context: WorkflowActionContext,
  ) => MaybePromise<WorkflowUpdateInput | WorkflowUpdateInput[] | undefined>;
};

export type ShellActionNodeDefinition = WorkflowNodeCommon & {
  nodeType: "action";
  effect?: WorkflowManagedEffect;
  exec: (context: WorkflowActionContext) => MaybePromise<ShellActionExecution>;
  parse?: (result: ShellActionResult, context: WorkflowActionContext) => MaybePromise<unknown>;
  updates?: ShellActionUpdates;
};

export type ActionNodeDefinition = FunctionActionNodeDefinition | ShellActionNodeDefinition;

export type HumanDecisionTextInput = {
  kind: "text";
  name: string;
  prompt: string;
  minLength: number;
  maxLength: number;
};

export type HumanDecisionChoice = {
  label: string;
  input?: HumanDecisionTextInput;
};

export type HumanDecisionChoiceMap = Record<string, HumanDecisionChoice>;

export type DecisionPresentationParagraph = {
  kind: "paragraph";
  text: string;
};

export type DecisionPresentationSection = {
  kind: "section";
  title: string;
};

export type DecisionPresentationBullets = {
  kind: "bullets";
  items: string[];
};

export type DecisionPresentationFields = {
  kind: "fields";
  items: Array<{ label: string; value: string }>;
};

export type DecisionPresentationPreformatted = {
  kind: "preformatted";
  text: string;
};

export type DecisionPresentationBlock =
  | DecisionPresentationParagraph
  | DecisionPresentationSection
  | DecisionPresentationBullets
  | DecisionPresentationFields
  | DecisionPresentationPreformatted;

export type DecisionPresentation = {
  schema: "pi-workflows.decision-presentation.v1";
  summary: string;
  blocks: DecisionPresentationBlock[];
};

export type HumanDecisionPrompt<TSubject = unknown> = {
  title: string;
  subject: TSubject;
  presentation: DecisionPresentation;
  /** Positive revision of the decision subject and presentation. Defaults to 1. */
  revision?: number;
  /** Optional absolute expiry. An expired request cannot accept an answer. */
  expiresAt?: string;
};

export type HumanDecisionAudience =
  | string
  | ((context: WorkflowNodeContext) => MaybePromise<string>);

export type HumanDecisionTimeout = {
  afterMs: number;
  response: HumanDecisionResponse;
};

export type HumanDecisionTimeoutPolicy =
  | HumanDecisionTimeout
  | ((context: WorkflowNodeContext) => MaybePromise<HumanDecisionTimeout | undefined>);

export type HumanDecisionNodeContract = {
  audience: HumanDecisionAudience;
  choices: HumanDecisionChoiceMap;
  request: (context: WorkflowNodeContext) => MaybePromise<HumanDecisionPrompt>;
  onTimeout?: HumanDecisionTimeoutPolicy;
};

type HumanDecisionRequestCommon = {
  decisionId: string;
  requestDigest: string;
  runId: string;
  workflowName: string;
  nodeId: string;
  attemptId: string;
  audience: string;
  title: string;
  choices: HumanDecisionChoiceMap;
  createdAt: string;
  expiresAt?: string;
  defaultResponse?: HumanDecisionResponse;
};

export type HumanDecisionRequest = HumanDecisionRequestCommon & {
  schema: "pi-workflows.human-decision-request.v1";
  subject: unknown;
  presentation: DecisionPresentation;
  revision: number;
  subjectDigest: string;
  presentationDigest: string;
};

/**
 * Complete operator-facing request passed to a decision channel. It excludes
 * the canonical subject by design.
 */
export type HumanDecisionChannelRequest = HumanDecisionRequestCommon & {
  schema: "pi-workflows.human-decision-channel-request.v1";
  sourceSchema: HumanDecisionRequest["schema"];
  presentation: DecisionPresentation;
  presentationDigest: string;
  revision: number;
};

export type HumanDecisionResponse = {
  choice: string;
  input?: Record<string, string>;
};

export type HumanDecisionAnswerSource = {
  channel: string;
  actorId: string;
  eventId: string;
};

export type HumanDecisionSubmission = HumanDecisionResponse & {
  decisionId: string;
  requestDigest: string;
  source: HumanDecisionAnswerSource;
  idempotencyKey: string;
};

type ResolvedHumanDecisionCommon = {
  decisionId: string;
  requestDigest: string;
  response: HumanDecisionResponse;
  provenance: "human" | "timeout";
  acceptedAt: string;
  answerDigest: string;
};

type AcceptedHumanDecisionCommon = ResolvedHumanDecisionCommon & {
  provenance: "human";
  source: HumanDecisionAnswerSource;
  idempotencyKey: string;
};

type DefaultedHumanDecisionCommon = ResolvedHumanDecisionCommon & {
  provenance: "timeout";
};

export type AcceptedHumanDecision = AcceptedHumanDecisionCommon & {
  schema: "pi-workflows.human-decision-accepted.v1";
  subjectDigest: string;
  presentationDigest: string;
  revision: number;
};

export type DefaultedHumanDecision = DefaultedHumanDecisionCommon & {
  schema: "pi-workflows.human-decision-accepted.v1";
  subjectDigest: string;
  presentationDigest: string;
  revision: number;
};
export type ResolvedHumanDecision = AcceptedHumanDecision | DefaultedHumanDecision;

type HumanDecisionReceiptCommon = {
  decisionId: string;
  requestDigest: string;
  nodeId: string;
  response: HumanDecisionResponse;
  provenance: "human" | "timeout";
  acceptedAt: string;
  answerDigest: string;
};

export type HumanDecisionReceipt = HumanDecisionReceiptCommon & {
  schema: "pi-workflows.human-decision-receipt.v1";
  subjectDigest: string;
  presentationDigest: string;
  revision: number;
};

export type HumanDecisionDeliveryRecord = {
  schema: "pi-workflows.human-decision-delivery.v1";
  attemptId: string;
  decisionId: string;
  requestDigest: string;
  presentationDigest: string;
  channel: string;
  phase: "intent" | "part" | "complete";
  state: "intent" | "confirmed" | "failed" | "unknown";
  createdAt: string;
  finishedAt?: string;
  recipientIndex?: number;
  partIndex?: number;
  partCount?: number;
  contentDigest?: string;
  messageCount?: number;
  errorCode?: string;
};

export type HumanDecisionSettlementRecord = {
  schema: "pi-workflows.human-decision-settlement.v1";
  attemptId: string;
  decisionId: string;
  requestDigest: string;
  channel: string;
  state: "confirmed" | "failed" | "unknown";
  createdAt: string;
  finishedAt: string;
  errorCode?: string;
};

export type HumanDecisionCancellationRecord = {
  schema: "pi-workflows.human-decision-cancellation.v1";
  decisionId: string;
  requestDigest: string;
  cancelledAt: string;
  reason: "cancelled" | "expired";
};

/**
 * A same-run input request. The optional `run` callback produces its prompt.
 * The accepted answer becomes the node output; requesting input does not finish the node.
 */
export type CheckpointNodeDefinition = WorkflowNodeCommon & {
  nodeType: "checkpoint";
  summary?: string;
  run?: (context: WorkflowNodeContext) => MaybePromise<unknown>;
  /** Typed verified-human request. Still executes as a checkpoint node. */
  humanDecision?: HumanDecisionNodeContract;
};

export type WorkflowNodeDefinition =
  | AgentNodeDefinition
  | ComputeNodeDefinition
  | NotifyNodeDefinition
  | ActionNodeDefinition
  | CheckpointNodeDefinition;

/** Runtime parser that also carries its normalized TypeScript result type. */
export type WorkflowValueParser<T> = (value: unknown) => MaybePromise<T>;

export type WorkflowExitDefinition<TOutput = unknown> = {
  /** Successful terminal node whose output leaves through this exit. */
  from: string;
  /** Optional runtime output normalizer and validator. */
  validate?: WorkflowValueParser<TOutput>;
};

export type WorkflowExitMap = Record<string, WorkflowExitDefinition>;

export type WorkflowInputOf<TWorkflow> =
  TWorkflow extends WorkflowDefinition<infer TInput, any, any, any> ? TInput : unknown;

export type WorkflowSettingsOf<TWorkflow> =
  TWorkflow extends WorkflowDefinition<any, any, any, infer TSettings> ? TSettings : unknown;

export type WorkflowExitOutputs<TWorkflow> =
  TWorkflow extends WorkflowDefinition<any, infer TExits, any, any>
    ? {
        [K in keyof TExits]: TExits[K] extends WorkflowExitDefinition<infer TOutput>
          ? TOutput
          : unknown;
      }
    : Record<string, unknown>;

export type WorkflowIncludedResult<TWorkflow> = {
  [K in keyof WorkflowExitOutputs<TWorkflow>]: {
    exit: K;
    output: WorkflowExitOutputs<TWorkflow>[K];
  };
}[keyof WorkflowExitOutputs<TWorkflow>];

export type WorkflowIncludeDefinition<
  TWorkflow extends WorkflowDefinition<any, any, any, any> = WorkflowDefinition<any, any, any, any>,
> = {
  /** Imported child definition or dynamic discovered name/path. */
  workflow: TWorkflow | string;
  /** Pure parent-to-child input mapping, evaluated on every mount entry. */
  input?: (context: WorkflowNodeContext) => MaybePromise<WorkflowInputOf<TWorkflow>>;
  /** Optional explicit initial settings mapping for this child invocation. */
  settings?: (context: WorkflowNodeContext) => MaybePromise<WorkflowSettingsOf<TWorkflow>>;
  /** Optional direct definition that supplies the contract for a dynamic reference. */
  contract?: TWorkflow;
};

export type WorkflowIncludeMap = Record<string, WorkflowIncludeDefinition>;

export type WorkflowIncludeExitReference<TIncludes extends WorkflowIncludeMap> = {
  [K in keyof TIncludes & string]: TIncludes[K] extends WorkflowIncludeDefinition<infer TWorkflow>
    ? `${K}.${Extract<keyof WorkflowExitOutputs<TWorkflow>, string>}`
    : never;
}[keyof TIncludes & string];

export type WorkflowTypedEdge<
  TNodes extends Record<string, WorkflowNodeDefinition>,
  TIncludes extends WorkflowIncludeMap,
> =
  | {
      from: (keyof TNodes & string) | WorkflowIncludeExitReference<TIncludes>;
      to: (keyof TNodes & string) | (keyof TIncludes & string);
    }
  | {
      from: keyof TNodes & string;
      switch: {
        on: string;
        cases: Record<string, (keyof TNodes & string) | (keyof TIncludes & string)>;
      };
    };

export type WorkflowDefinition<
  TInput = any,
  TExits extends WorkflowExitMap = WorkflowExitMap,
  TIncludes extends WorkflowIncludeMap = WorkflowIncludeMap,
  TSettings = any,
> = {
  name: string;
  /** Optional workflow-owned JSON settings that can change during a run. */
  settings?: import("./settings.js").WorkflowSettingsDefinition<TSettings, TInput>;
  /** Module URL used to attest directly imported child workflow files. */
  source?: string;
  /** Stable public input-and-exit contract identity for compatible overrides. */
  contractId?: string;
  /** Optional runtime input normalizer and validator. */
  input?: WorkflowValueParser<TInput>;
  /** Optional human-readable run title (static or derived from input). */
  title?:
    | string
    | ((context: { input: TInput; workflowName: string }) => MaybePromise<string | undefined>);
  startAt: string;
  nodes: Record<string, WorkflowNodeDefinition>;
  includes?: TIncludes;
  exits?: TExits;
  edges: WorkflowEdge[];
  /** Guard against unbounded loops. Defaults to the engine's maxSteps. */
  maxSteps?: number;
};

export type WorkflowNodeOutcome = "ok" | "timed_out" | "failed" | "cancelled";

/** Explicit linkage from a workflow attempt to the Pi conversation slice it produced. */
export type ConversationRange = {
  /** First Pi session entry id of the attempt. */
  firstEntryId: string;
  /** Last Pi session entry id of the attempt, inclusive. */
  lastEntryId: string;
};

export type WorkflowNodeResult = {
  attemptId: string;
  nodeId: string;
  nodeType: WorkflowNodeDefinition["nodeType"];
  outcome: WorkflowNodeOutcome;
  startedAt: string;
  finishedAt: string;
  durationMs: number;
  output?: unknown;
  error?: string;
};

export type WorkflowActionReceipt = {
  actionType: "shell" | "function";
  command?: string;
  args?: string[];
  cwd?: string;
  exitCode?: number | null;
  signal?: NodeJS.Signals | null;
  durationMs?: number;
};

export type WorkflowStepRecord = {
  attemptId: string;
  nodeId: string;
  nodeType: WorkflowNodeDefinition["nodeType"];
  outcome: WorkflowNodeOutcome;
  startedAt: string;
  finishedAt: string;
  /** Full prompt text for agent steps, `null` for other node types. */
  prompt: string | null;
  output: unknown;
  error?: string;
  action?: WorkflowActionReceipt;
  /** Receipt for a visible assistant-message completion. */
  assistantMessage?: AssistantMessageReceipt;
  /** For agent steps recorded inside a Pi conversation. */
  conversation?: ConversationRange;
  /** Fixed settings scope used by this attempt. */
  settingsScopeId?: string;
  /** Fixed accepted-through settings number used by this attempt. */
  settingsChangeNumber?: number;
  /** SHA-256 hash of the fixed settings JSON used by this attempt. */
  settingsHash?: string;
};

export type WorkflowRunStatus =
  | "running"
  | "waiting"
  | "completed"
  | "failed"
  | "timed_out"
  | "cancelled";

export type WorkflowSource =
  | { kind: "builtin"; id: string; revision: string }
  | { kind: "file"; path: string; hash: string };

export type WorkflowMountedSource = {
  mountPath: string[];
  workflowName: string;
  source: WorkflowSource;
};

export type WorkflowMountSnapshot = {
  mountPath: string[];
  workflowName: string;
  entryNode: string;
  exits: Record<string, string>;
  maxSteps?: number;
};

export type WorkflowCompositionSnapshot = {
  mounts: WorkflowMountSnapshot[];
};

export type WorkflowRunState = {
  schema: "pi-workflows.run-state.v1";
  /** Resource revision of the event reflected by this projection. */
  traceSeq: number;
  runId: string;
  workflowName: string;
  /** Set on explicit fresh restarts. Checkpoint answers keep the same run. */
  parentRunId?: string;
  runTitle?: string;
  /** Stable built-in identity or immutable file source used by this run. */
  workflowSource?: WorkflowSource;
  /** Sorted immutable sources used by included workflow mounts. */
  workflowSources?: WorkflowMountedSource[];
  /** SHA-256 of the fully resolved definition snapshot. */
  definitionDigest?: string;
  /** Legacy fields accepted only by the bounded built-in migration. */
  workflowPath?: string;
  workflowHash?: string;
  startedAt: string;
  finishedAt?: string;
  updatedAt: string;
  status: WorkflowRunStatus;
  input: unknown;
  outputs: Record<string, unknown>;
  results: Record<string, WorkflowNodeResult>;
  steps: WorkflowStepRecord[];
  /** Latest update for each `(type, key)` pair, sorted by trace sequence. */
  updates?: WorkflowUpdateRecord[];
  currentNode?: string;
  currentAttemptId?: string;
  currentNodeStartedAt?: string;
  /** Durable active-execution deadline for the current attempt. Null disables the deadline. */
  currentNodeTimeoutMs?: number | null;
  currentNodeElapsedMs?: number;
  currentSettingsScopeId?: string;
  currentSettingsChangeNumber?: number;
  currentSettingsHash?: string;
  statusDetail?: string;
  /** Redacted receipt for the latest accepted human decision in this run. */
  humanDecision?: HumanDecisionReceipt;
  /** True while the run is held at a step boundary by a pause request. */
  paused?: boolean;
  waitingOn?: string;
  finalOutput?: unknown;
  error?: string;
};

export type WorkflowNodeSnapshot = {
  nodeType: WorkflowNodeDefinition["nodeType"];
  timeoutMs?: number | null;
  statusDetail?: string;
  summary?: string;
  expectedOutput?: AgentExpectedOutput;
  allowedTools?: string[];
  agent?: string;
  model?: string;
  settingsRoute?: true;
  actionExecution?: "function" | "shell";
  effect?: { type: string; recovery: WorkflowEffectRecovery };
  mountPath?: string[];
  localNodeId?: string;
  includeTransition?: "entry" | "exit";
  humanDecision?: {
    audience: string;
    dynamicAudience?: boolean;
    choices: HumanDecisionChoiceMap;
    onTimeout?: HumanDecisionTimeout;
    dynamicTimeout?: boolean;
  };
};

export type WorkflowDefinitionSnapshot = {
  schema: "pi-workflows.definition-snapshot.v1";
  name: string;
  contractId?: string;
  startAt: string;
  nodes: Record<string, WorkflowNodeSnapshot>;
  edges: WorkflowEdge[];
  composition?: WorkflowCompositionSnapshot;
  settings?: WorkflowSettingsSnapshot;
  settingsScopes?: Array<{
    mountPath: string[];
    settings: WorkflowSettingsSnapshot;
  }>;
};

export type WorkflowSettingsSnapshot = {
  description?: string;
  paths: Array<{
    path: string;
    permissions: Partial<Record<"read" | "add" | "remove" | "replace", string[]>>;
  }>;
};

export type WorkflowTraceEvent = {
  seq: number;
  at: string;
  scope: "run" | "node" | "agent" | "action" | "session";
  type: string;
  runId: string;
  nodeId?: string;
  attemptId?: string;
  payload: Record<string, unknown>;
};

/** `session/binding.json`: written once when a run binds to a conversation. */
export type WorkflowSessionBinding = {
  schema: "pi-workflows.session-binding.v1";
  runId: string;
  /** Pi session UUID. */
  piSessionId: string;
  /**
   * Absolute path of the Pi session file; provenance only, never read back.
   * Absent for in-memory sessions.
   */
  piSessionFile?: string;
  /** Working directory of the conversation. */
  cwd: string;
  boundAt: string;
};

/**
 * One line of `session/entries.ndjson`: a verbatim Pi session entry recorded
 * while the run was active. The inner entry shape is owned by Pi.
 */
export type WorkflowSessionEntryRecord = {
  /** Starts at 1, increases by exactly 1 within the file. */
  seq: number;
  /** When the entry was recorded in workflow state. */
  at: string;
  /** Verbatim Pi session entry (has its own id/parentId/timestamp). */
  entry: Record<string, unknown>;
};

export type WorkflowSessionEventType =
  | "turn_started"
  | "turn_finished"
  | "message_started"
  | "assistant_event"
  | "message_finished"
  | "tool_execution_started"
  | "tool_execution_finished";

/** One normalized temporal Pi event in `session/events.ndjson`. */
export type WorkflowSessionEventRecord = {
  /** Starts at 1 and increases by exactly 1 within the file. */
  seq: number;
  /** Time when the extension received the public Pi event. */
  at: string;
  nodeId: string;
  attemptId: string;
  turnId?: string;
  messageId?: string;
  toolCallId?: string;
  type: WorkflowSessionEventType;
  payload: Record<string, unknown>;
};

export type WorkflowSessionCaptureFailure = {
  failedAt: string;
  code: string;
  message: string;
};

/** Atomic integrity projection for the temporal session journal. */
export type WorkflowSessionCapture = {
  schema: "pi-workflows.session-capture.v1";
  eventSchema: "pi-workflows.session-event.v1";
  status: "recording" | "complete" | "failed";
  eventCount: number;
  entryCount: number;
  lastEventSeq: number;
  failure?: WorkflowSessionCaptureFailure;
};

export type WorkflowTraceEventDraft = Omit<WorkflowTraceEvent, "seq" | "at" | "runId">;

export type WorkflowRunResult = {
  runId: string;
  state: WorkflowRunState;
};

export type AgentStepCompletion = "submit" | "assistant";

export type AssistantMessageReceipt = {
  sha256: string;
  /** Pi session entry containing the visible response, when available. */
  entryId?: string;
  /** Present only when the workflow author supplied a limit. */
  maxChars?: number;
  /** True when recovery adopted an already visible response. */
  recovered?: boolean;
};

/** The step contract handed to the executor alongside the prompt. */
export type AgentStepContract = {
  requestId: string;
  runId: string;
  workflowName: string;
  nodeId: string;
  attemptId: string;
  completion: AgentStepCompletion;
  allowedTools?: string[];
  expectedOutput?: string;
  maxOutputChars?: number;
};

/** Optional human-facing labels for an agent step. They never affect execution. */
export type AgentStepPresentation = {
  runTitle?: string;
  statusDetail?: string;
};

export type AgentStepRequest = {
  contract: AgentStepContract;
  prompt: string;
  presentation?: AgentStepPresentation;
  /** The step's `agent`: an executor that can run acpx agents runs the step on it. */
  agent?: string;
  /** The step's `model`, applied to `agent`. */
  model?: string;
  /**
   * Validate a submission from the model. Returns the normalized output or an
   * error message the executor should surface to the model for retry.
   */
  accept: (output: unknown) => Promise<{ ok: true; value: unknown } | { ok: false; error: string }>;
  /** Publish a non-completing update from a headless executor. */
  publishUpdate?: (
    update: WorkflowUpdateInput,
    idempotencyKey?: string,
  ) => Promise<WorkflowUpdateReceipt>;
};

export type AgentStepSubmission = {
  output: unknown;
  /** Receipt for a visible assistant response. */
  assistantMessage?: AssistantMessageReceipt;
  /**
   * The Pi conversation slice this step produced, when the executor records
   * one. Persisted verbatim into the step record and terminal node event.
   */
  conversation?: ConversationRange;
};

/**
 * Runs one agent step to completion. Implementations deliver the prompt to
 * the model and resolve once the configured output has been accepted.
 * Must reject with an `AbortError`-like error when `signal` aborts.
 */
export interface AgentStepExecutor {
  /**
   * `visible` supports assistant-message output. `park` asks the engine to
   * leave the run claimable for an origin session. Omission is unsupported.
   */
  readonly assistantMessageMode?: "visible" | "park" | "unsupported";
  /** True when the workflow server records active time while this executor parks and resumes an attempt. */
  readonly preservesActiveTimeBudget?: boolean;
  /** Must enforce the exact contract allowlist before every tool executes. */
  readonly enforcesToolAllowlist?: boolean;
  runAgentStep(request: AgentStepRequest, signal: AbortSignal): Promise<AgentStepSubmission>;
}

export type WorkflowNotificationRequest = {
  runId: string;
  workflowName: string;
  nodeId: string;
  attemptId: string;
  /** Stable one-based occurrence of this notify node within the run. */
  notificationIndex: number;
  kind: "progress" | "final";
  content: string;
};

export type WorkflowNotificationReceipt = {
  notificationId: string;
  targetSessionId: string;
};

export interface WorkflowNotificationSink {
  notify(request: WorkflowNotificationRequest): MaybePromise<WorkflowNotificationReceipt>;
}

export type WorkflowEngineOptions = {
  executor: AgentStepExecutor;
  /** Durable destination for notify nodes. Required when a workflow uses one. */
  notificationSink?: WorkflowNotificationSink;
  /** Canonical SQLite database. Defaults to `~/.pi/agent/workflows/state.sqlite`. */
  databasePath?: string;
  /** Durable execution store. Runners use a server-backed implementation. */
  store?: import("./store.js").WorkflowExecutionStore;
  /**
   * Awaited after `run_started` is persisted, before any node executes.
   */
  onRunStarted?: (runId: string, state: WorkflowRunState) => MaybePromise<void>;
  /** Default per-node timeout. Defaults to 15 minutes. */
  defaultNodeTimeoutMs?: number;
  /** Guard against unbounded graph loops. Defaults to 100 executed steps. */
  maxSteps?: number;
  /** Observer invoked after every persisted trace event. */
  onEvent?: (event: WorkflowTraceEvent, state: WorkflowRunState) => void;
};

````

### `tui/src/state/types.rs` (complete file, repository pi-workflows)
````
//! Serde types for the workflow documents carried in server-owned run views.
//! Unknown document fields are tolerated, but the client envelope is strict.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const RUN_BUNDLE_SCHEMA: &str = "pi-workflows.run-run.v1";
pub const RUN_STATE_SCHEMA: &str = "pi-workflows.run-state.v1";
pub const DEFINITION_SNAPSHOT_SCHEMA: &str = "pi-workflows.definition-snapshot.v1";
pub const SESSION_BINDING_SCHEMA: &str = "pi-workflows.session-binding.v1";
pub const SESSION_EVENT_SCHEMA: &str = "pi-workflows.session-event.v1";
pub const SESSION_CAPTURE_SCHEMA: &str = "pi-workflows.session-capture.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Waiting,
    Paused,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    Ambiguous,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunStatus::Completed | RunStatus::Failed | RunStatus::TimedOut | RunStatus::Cancelled
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Waiting => "waiting",
            RunStatus::Paused => "paused",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::TimedOut => "timed_out",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Ambiguous => "ambiguous",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowActivity {
    SupervisedRunner,
    OriginTurn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDisplay {
    pub status: RunStatus,
    pub activity: Option<WorkflowActivity>,
    /// Advisory control names are owned by the Workflow Server.
    /// Executable client operations remain closed protocol types.
    pub controls: Vec<String>,
    pub reason: Option<String>,
    #[serde(rename = "reasonContent", skip_serializing_if = "Option::is_none")]
    pub reason_content: Option<Value>,
}

impl WorkflowDisplay {
    pub fn active_node<'a>(&self, state: &'a RunState) -> Option<&'a str> {
        if self.status != RunStatus::Running {
            return None;
        }
        state.current_node.as_deref().or_else(|| {
            (self.activity == Some(WorkflowActivity::OriginTurn))
                .then_some(state.waiting_on.as_deref())
                .flatten()
        })
    }
}

#[cfg(test)]
mod workflow_display_tests {
    use super::{RunState, RunStatus, WorkflowActivity, WorkflowDisplay};
    use serde_json::{json, Value};
    use std::path::PathBuf;

    #[test]
    fn preserves_the_server_display_and_uses_only_a_running_origin_turn_as_the_active_wait() {
        let state: RunState = serde_json::from_value(json!({
            "schema":"pi-workflows.run-state.v1",
            "traceSeq":1,
            "runId":"run-1",
            "workflowName":"smoke",
            "startedAt":"2026-01-01T00:00:00.000Z",
            "updatedAt":"2026-01-01T00:00:01.000Z",
            "status":"waiting",
            "input":{},
            "outputs":{},
            "results":{},
            "steps":[],
            "waitingOn":"work"
        }))
        .unwrap();
        let running: WorkflowDisplay = serde_json::from_value(json!({
            "status":"running",
            "activity":"origin_turn",
            "controls":["pause","cancel"],
            "reason":null,
            "reasonContent":{"turn":"active"}
        }))
        .unwrap();

        assert_eq!(running.status, RunStatus::Running);
        assert_eq!(running.activity, Some(WorkflowActivity::OriginTurn));
        assert_eq!(running.controls, vec!["pause", "cancel"]);
        assert_eq!(running.active_node(&state), Some("work"));

        let waiting = WorkflowDisplay {
            status: RunStatus::Waiting,
            activity: None,
            controls: vec!["pause".into(), "cancel".into()],
            reason: Some("waiting".into()),
            reason_content: None,
        };
        assert_eq!(waiting.active_node(&state), None);
    }

    #[test]
    fn accepts_server_owned_display_controls_and_rejects_non_strings() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root")
            .join("protocol/fixtures/run-view-controls-v1.json");
        let fixture: Value = serde_json::from_slice(&std::fs::read(path).expect("control fixture"))
            .expect("valid control fixture");
        let displays = fixture["displays"].as_array().expect("fixture displays");

        for case in displays {
            let display: WorkflowDisplay = serde_json::from_value(case["display"].clone())
                .expect("server-owned control strings should decode");
            assert!(!display.controls.is_empty());
        }

        let mut invalid = displays[0]["display"].clone();
        invalid["controls"][0] = json!(42);
        assert!(serde_json::from_value::<WorkflowDisplay>(invalid).is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeOutcome {
    Ok,
    TimedOut,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowSource {
    Builtin { id: String, revision: String },
    File { path: String, hash: String },
}

impl WorkflowSource {
    pub fn display(&self) -> String {
        match self {
            WorkflowSource::Builtin { id, revision } => format!("builtin:{id}@{revision}"),
            WorkflowSource::File { path, .. } => path.clone(),
        }
    }
}

#[cfg(test)]
mod workflow_source_tests {
    use super::WorkflowSource;

    #[test]
    fn parses_and_displays_each_source_kind() {
        let builtin: WorkflowSource =
            serde_json::from_str(r#"{"kind":"builtin","id":"monitor","revision":"1"}"#)
                .expect("built-in source should parse");
        let file: WorkflowSource =
            serde_json::from_str(r#"{"kind":"file","path":"/tmp/demo.workflow.ts","hash":"abc"}"#)
                .expect("file source should parse");

        assert_eq!(builtin.display(), "builtin:monitor@1");
        assert_eq!(file.display(), "/tmp/demo.workflow.ts");
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "workflowName")]
    pub workflow_name: String,
    #[serde(rename = "runTitle", skip_serializing_if = "Option::is_none")]
    pub run_title: Option<String>,
    #[serde(rename = "workflowSource", skip_serializing_if = "Option::is_none")]
    pub workflow_source: Option<WorkflowSource>,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub status: RunStatus,
    #[serde(rename = "traceSchema")]
    pub trace_schema: String,
    pub paths: ManifestPaths,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestPaths {
    pub workflow: String,
    pub state: String,
    pub trace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    pub schema: String,
    #[serde(rename = "traceSeq")]
    pub trace_seq: u64,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "workflowName")]
    pub workflow_name: String,
    #[serde(rename = "runTitle", skip_serializing_if = "Option::is_none")]
    pub run_title: Option<String>,
    #[serde(rename = "workflowSource", skip_serializing_if = "Option::is_none")]
    pub workflow_source: Option<WorkflowSource>,
    #[serde(rename = "parentRunId", skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(rename = "carriedStepCount", skip_serializing_if = "Option::is_none")]
    pub carried_step_count: Option<u64>,
    #[serde(rename = "workflowSources", skip_serializing_if = "Option::is_none")]
    pub workflow_sources: Option<Vec<Value>>,
    #[serde(rename = "definitionDigest", skip_serializing_if = "Option::is_none")]
    pub definition_digest: Option<String>,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub status: RunStatus,
    pub input: Value,
    pub outputs: BTreeMap<String, Value>,
    pub results: BTreeMap<String, Value>,
    pub steps: Vec<StepRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updates: Option<Vec<Value>>,
    #[serde(rename = "currentNode", skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
    #[serde(rename = "currentAttemptId", skip_serializing_if = "Option::is_none")]
    pub current_attempt_id: Option<String>,
    #[serde(
        rename = "currentNodeStartedAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub current_node_started_at: Option<String>,
    #[serde(
        rename = "currentSettingsScopeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub current_settings_scope_id: Option<String>,
    #[serde(
        rename = "currentSettingsChangeNumber",
        skip_serializing_if = "Option::is_none"
    )]
    pub current_settings_change_number: Option<u64>,
    #[serde(
        rename = "currentSettingsHash",
        skip_serializing_if = "Option::is_none"
    )]
    pub current_settings_hash: Option<String>,
    #[serde(rename = "statusDetail", skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    #[serde(rename = "humanDecision", skip_serializing_if = "Option::is_none")]
    pub human_decision: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(rename = "waitingOn", skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(rename = "finalOutput", skip_serializing_if = "Option::is_none")]
    pub final_output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeResult {
    #[serde(rename = "attemptId")]
    pub attempt_id: String,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "nodeType")]
    pub node_type: String,
    pub outcome: NodeOutcome,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "finishedAt")]
    pub finished_at: String,
    #[serde(rename = "durationMs")]
    pub duration_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    #[serde(rename = "attemptId")]
    pub attempt_id: String,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "nodeType")]
    pub node_type: String,
    pub outcome: NodeOutcome,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "finishedAt")]
    pub finished_at: String,
    /// Full prompt for agent steps (`null` otherwise); may be an
    /// externalized `$artifact` object in persisted form.
    pub prompt: Value,
    pub output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionReceipt>,
    #[serde(rename = "assistantMessage", skip_serializing_if = "Option::is_none")]
    pub assistant_message: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationRange>,
    #[serde(rename = "settingsScopeId", skip_serializing_if = "Option::is_none")]
    pub settings_scope_id: Option<String>,
    #[serde(
        rename = "settingsChangeNumber",
        skip_serializing_if = "Option::is_none"
    )]
    pub settings_change_number: Option<u64>,
    #[serde(rename = "settingsHash", skip_serializing_if = "Option::is_none")]
    pub settings_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionReceipt {
    #[serde(rename = "actionType")]
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(rename = "exitCode", skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<Value>,
    #[serde(rename = "durationMs", skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRange {
    #[serde(rename = "firstEntryId")]
    pub first_entry_id: String,
    #[serde(rename = "lastEntryId")]
    pub last_entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub seq: u64,
    pub at: String,
    pub scope: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "nodeId", skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(rename = "attemptId", skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub schema: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "piSessionId")]
    pub pi_session_id: String,
    #[serde(rename = "piSessionFile", skip_serializing_if = "Option::is_none")]
    pub pi_session_file: Option<String>,
    pub cwd: String,
    #[serde(rename = "boundAt")]
    pub bound_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEntryRecord {
    pub seq: u64,
    pub at: String,
    pub entry: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEventRecord {
    pub seq: u64,
    pub at: String,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "attemptId")]
    pub attempt_id: String,
    #[serde(rename = "turnId", skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(rename = "toolCallId", skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCaptureStatus {
    Recording,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCaptureFailure {
    #[serde(rename = "failedAt")]
    pub failed_at: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionCapture {
    pub schema: String,
    #[serde(rename = "eventSchema")]
    pub event_schema: String,
    pub status: SessionCaptureStatus,
    #[serde(rename = "eventCount")]
    pub event_count: u64,
    #[serde(rename = "entryCount")]
    pub entry_count: u64,
    #[serde(rename = "lastEventSeq")]
    pub last_event_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<SessionCaptureFailure>,
}

// --- Definition snapshot ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefinitionSnapshot {
    pub schema: String,
    pub name: String,
    #[serde(rename = "startAt")]
    pub start_at: String,
    /// Insertion order matters (BFS fallback order, display order), so the
    /// map preserves the document order of the JSON object.
    pub nodes: serde_json::Map<String, Value>,
    pub edges: Vec<EdgeDef>,
}

impl DefinitionSnapshot {
    pub fn node_type(&self, node_id: &str) -> Option<&str> {
        self.nodes
            .get(node_id)?
            .get("nodeType")
            .and_then(Value::as_str)
    }

    pub fn node_action_execution(&self, node_id: &str) -> Option<&str> {
        self.nodes
            .get(node_id)?
            .get("actionExecution")
            .and_then(Value::as_str)
    }

    pub fn node_ids(&self) -> impl Iterator<Item = &str> {
        self.nodes.keys().map(String::as_str)
    }
}

/// A workflow edge: either a simple `from -> to` or a labelled switch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EdgeDef {
    Simple { from: String, to: String },
    Switch { from: String, switch: SwitchDef },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchDef {
    pub on: String,
    /// Case order matters for edge expansion; preserve document order.
    pub cases: serde_json::Map<String, Value>,
}

/// An artifact reference extracted from a `{"$artifact": …}` sentinel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub path: String,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Detect the `$artifact` sentinel: an object whose only key is `$artifact`.
pub fn as_artifact_ref(value: &Value) -> Option<ArtifactRef> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    serde_json::from_value(object.get("$artifact")?.clone()).ok()
}

/// Unwrap one level of `{"$escaped": …}` if present.
pub fn as_escaped(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.get("$escaped")
}

````

### `tui/src/client.rs` (complete file, repository pi-workflows)
````
//! Reconnecting client used by local Unix-socket and remote WebSocket modes.

use crate::client_commands::{build_command, CommandIds, CommandOutcome, CommandTracker};
use crate::layout::{layout_graph, GraphLayout};
use crate::protocol::{
    encode_request, parse_server_message, ClientRequest, PageKind, ServerMessage, PROTOCOL_ID,
};
use crate::state::types::{
    as_artifact_ref, ArtifactRef, DefinitionSnapshot, Manifest, RunState, StepRecord,
    WorkflowDisplay,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub struct RemoteView {
    pub revision: u64,
    pub graph_revision: u64,
    generation: u64,
    pub manifest: Manifest,
    pub state: RunState,
    pub display: WorkflowDisplay,
    pub graph_steps: Vec<StepRecord>,
    pub taken_transitions: Vec<String>,
    pub graph_cursor: u64,
    pub step_start: u64,
    pub step_total: u64,
    pub snapshot: Option<DefinitionSnapshot>,
    pub graph_layout: Option<GraphLayout>,
    pub events: Vec<Value>,
    pub trace_start: u64,
    pub trace_total: u64,
    pub session_binding: Option<Value>,
    pub session_entries: Vec<Value>,
    pub session_entry_start: u64,
    pub session_entry_total: u64,
    pub session_events: Vec<Value>,
    pub session_event_start: u64,
    pub session_event_total: u64,
    pub session_events_malformed: bool,
    pub session_events_torn_tail: bool,
    pub session_capture: Option<Value>,
    pub session_replay_checkpoint: Option<Value>,
    pub settings_scopes: Vec<Value>,
    pub settings_start: u64,
    pub settings_total: u64,
    pub follow_up_queue: Option<Value>,
    pub follow_up_start: u64,
    pub follow_up_total: u64,
    pub update_start: u64,
    pub update_total: u64,
    pub live: bool,
    pub possibly_interrupted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ContentState {
    #[default]
    Absent,
    Pending,
    Ready(String),
    Invalid(String),
}

impl ContentState {
    fn as_deref(&self) -> Option<&str> {
        match self {
            Self::Ready(content) => Some(content),
            Self::Absent | Self::Pending | Self::Invalid(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ViewContent {
    definition: ContentState,
    graph_history: ContentState,
    display_reason: ContentState,
}

enum DecodeOutcome {
    Ready(Box<RemoteView>),
    PendingContent,
    Invalid(String),
}

fn decode_view(
    revision: u64,
    generation: u64,
    raw: &Value,
    content: &ViewContent,
) -> DecodeOutcome {
    let graph_revision = raw
        .get("graphRevision")
        .and_then(Value::as_u64)
        .unwrap_or(revision);
    let Some(manifest_value) = raw.get("manifest") else {
        return DecodeOutcome::Invalid("manifest is missing".to_string());
    };
    let Ok(manifest) = serde_json::from_value::<Manifest>(manifest_value.clone()) else {
        return DecodeOutcome::Invalid("manifest has invalid required fields".to_string());
    };
    let Some(state_value) = raw.get("state") else {
        return DecodeOutcome::Invalid("state is missing".to_string());
    };
    let Ok(state) = serde_json::from_value::<RunState>(state_value.clone()) else {
        return DecodeOutcome::Invalid("state has invalid required fields".to_string());
    };
    let Some(display_value) = raw.get("display") else {
        return DecodeOutcome::Invalid("display is missing".to_string());
    };
    let Ok(mut display) = serde_json::from_value::<WorkflowDisplay>(display_value.clone()) else {
        return DecodeOutcome::Invalid("display has invalid required fields".to_string());
    };

    for item in [
        &content.definition,
        &content.graph_history,
        &content.display_reason,
    ] {
        if let ContentState::Invalid(error) = item {
            return DecodeOutcome::Invalid(error.clone());
        }
    }
    if [
        &content.definition,
        &content.graph_history,
        &content.display_reason,
    ]
    .iter()
    .any(|item| matches!(item, ContentState::Pending))
    {
        return DecodeOutcome::PendingContent;
    }
    if display_reason_artifact(raw).is_some() {
        let Some(reason_content) = display_reason_value(raw, content.display_reason.as_deref())
        else {
            return DecodeOutcome::Invalid("display reason content is invalid".to_string());
        };
        display.reason_content = Some(reason_content);
    }
    let graph_history = graph_history_value(raw, content.graph_history.as_deref());
    if graph_history_artifact(raw).is_some() && graph_history.is_none() {
        return DecodeOutcome::Invalid("workflow graph history is invalid".to_string());
    }
    let graph_steps = graph_history
        .as_ref()
        .and_then(|value| value.get("steps"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| {
            raw.get("graphSteps")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
        })
        .unwrap_or_else(|| state.steps.clone());
    let taken_transitions = graph_history
        .as_ref()
        .and_then(|value| value.get("transitions"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| {
            raw.get("takenTransitions")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
        })
        .unwrap_or_default();
    let graph_cursor = raw
        .get("graphCursor")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| state.steps.len().saturating_sub(1) as u64);
    let step_start = raw.get("stepStart").and_then(Value::as_u64).unwrap_or(0);
    let step_total = raw
        .get("stepTotal")
        .and_then(Value::as_u64)
        .unwrap_or(state.steps.len() as u64);
    let Some(workflow_value) = workflow_definition_value(raw, content.definition.as_deref()) else {
        return DecodeOutcome::Invalid("workflow definition is missing or invalid".to_string());
    };
    let Ok(snapshot) = serde_json::from_value::<DefinitionSnapshot>(workflow_value) else {
        return DecodeOutcome::Invalid(
            "workflow definition has invalid required fields".to_string(),
        );
    };
    let snapshot = Some(snapshot);
    let graph_layout = raw
        .get("graphScene")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| snapshot.as_ref().map(layout_graph));
    let events = page_items(raw, "/tracePage/items");
    let trace_start = pointer_u64(raw, "/tracePage/start");
    let trace_total = pointer_u64(raw, "/tracePage/total").max(events.len() as u64);
    let session_binding = raw
        .pointer("/session/binding")
        .cloned()
        .filter(|v| !v.is_null());
    let session_entries = page_items(raw, "/session/entryPage/items");
    let session_entry_start = pointer_u64(raw, "/session/entryPage/start");
    let session_entry_total =
        pointer_u64(raw, "/session/entryPage/total").max(session_entries.len() as u64);
    let session_events = page_items(raw, "/session/eventPage/items");
    let session_event_start = pointer_u64(raw, "/session/eventPage/start");
    let session_event_total =
        pointer_u64(raw, "/session/eventPage/total").max(session_events.len() as u64);
    let settings_scopes = raw
        .get("settingsScopes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let follow_up_queue = raw.get("followUpQueue").cloned().filter(|v| !v.is_null());
    let update_total = raw
        .get("updateTotal")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            state
                .updates
                .as_ref()
                .map_or(0, |updates| updates.len() as u64)
        });
    DecodeOutcome::Ready(Box::new(RemoteView {
        revision,
        graph_revision,
        generation,
        manifest,
        state,
        display,
        graph_steps,
        taken_transitions,
        graph_cursor,
        step_start,
        step_total,
        snapshot,
        graph_layout,
        events,
        trace_start,
        trace_total,
        session_binding,
        session_entries,
        session_entry_start,
        session_entry_total,
        session_events,
        session_event_start,
        session_event_total,
        session_events_malformed: raw
            .pointer("/session/integrity/malformed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        session_events_torn_tail: raw
            .pointer("/session/integrity/tornTail")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        session_capture: raw
            .pointer("/session/capture")
            .cloned()
            .filter(|v| !v.is_null()),
        session_replay_checkpoint: raw
            .pointer("/session/replayCheckpoint")
            .cloned()
            .filter(|v| !v.is_null()),
        settings_start: raw
            .get("settingsStart")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        settings_total: raw
            .get("settingsTotal")
            .and_then(Value::as_u64)
            .unwrap_or(settings_scopes.len() as u64),
        settings_scopes,
        follow_up_start: raw
            .get("followUpStart")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        follow_up_total: raw
            .get("followUpTotal")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        follow_up_queue,
        update_start: raw.get("updateStart").and_then(Value::as_u64).unwrap_or(0),
        update_total,
        live: raw.get("live").and_then(Value::as_bool).unwrap_or(false),
        possibly_interrupted: raw
            .get("possiblyInterrupted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }))
}

fn page_items(raw: &Value, pointer: &str) -> Vec<Value> {
    raw.pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn pointer_u64(raw: &Value, pointer: &str) -> u64 {
    raw.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn workflow_definition_artifact(raw: &Value) -> Option<ArtifactRef> {
    let workflow = raw.get("workflow")?;
    as_artifact_ref(workflow).or_else(|| workflow.get("content").and_then(as_artifact_ref))
}

fn workflow_definition_value(raw: &Value, content: Option<&str>) -> Option<Value> {
    let workflow = raw.get("workflow")?;
    if workflow_definition_artifact(raw).is_some() {
        return serde_json::from_str(content?).ok();
    }
    Some(workflow.clone())
}

fn graph_history_artifact(raw: &Value) -> Option<ArtifactRef> {
    raw.get("graphHistory").and_then(as_artifact_ref)
}

fn graph_history_value(raw: &Value, content: Option<&str>) -> Option<Value> {
    let history = raw.get("graphHistory")?;
    if graph_history_artifact(raw).is_some() {
        return serde_json::from_str(content?).ok();
    }
    Some(history.clone())
}

fn display_reason_artifact(raw: &Value) -> Option<ArtifactRef> {
    raw.pointer("/display/reasonContent")
        .and_then(as_artifact_ref)
}

fn display_reason_value(raw: &Value, content: Option<&str>) -> Option<Value> {
    let reason = raw.pointer("/display/reasonContent")?;
    let Some(artifact) = display_reason_artifact(raw) else {
        return Some(reason.clone());
    };
    match artifact.media_type.as_str() {
        "text/plain" => Some(Value::String(content?.to_string())),
        "application/json" => serde_json::from_str(content?).ok(),
        _ => None,
    }
}

fn workflow_definition_content(
    state: &mut Shared,
    run_id: &str,
    raw: &Value,
) -> (ContentState, bool) {
    let Some(artifact) = workflow_definition_artifact(raw) else {
        return (ContentState::Absent, false);
    };
    referenced_content(
        state,
        run_id,
        artifact,
        &["application/json"],
        "workflow definition",
    )
}

fn graph_history_content(state: &mut Shared, run_id: &str, raw: &Value) -> (ContentState, bool) {
    let Some(artifact) = graph_history_artifact(raw) else {
        return (ContentState::Absent, false);
    };
    referenced_content(
        state,
        run_id,
        artifact,
        &["application/json"],
        "workflow graph history",
    )
}

fn display_reason_content(state: &mut Shared, run_id: &str, raw: &Value) -> (ContentState, bool) {
    let Some(artifact) = display_reason_artifact(raw) else {
        return (ContentState::Absent, false);
    };
    referenced_content(
        state,
        run_id,
        artifact,
        &["text/plain", "application/json"],
        "workflow display reason",
    )
}

fn referenced_content(
    state: &mut Shared,
    run_id: &str,
    artifact: ArtifactRef,
    accepted_media_types: &[&str],
    label: &str,
) -> (ContentState, bool) {
    let key = (run_id.to_string(), artifact.path.clone());
    match state.artifacts.get(&key).cloned() {
        Some(ArtifactEntry::Ready(content)) => {
            let valid = accepted_media_types.contains(&artifact.media_type.as_str())
                && content.len() as u64 == artifact.bytes
                && hex_sha256(content.as_bytes()) == artifact.sha256;
            if valid {
                return (ContentState::Ready(content), false);
            }
            let artifact_error = format!("{label} content does not match its reference");
            state
                .artifacts
                .insert(key, ArtifactEntry::Error(artifact_error));
            (
                ContentState::Invalid(format!("{label} content is invalid")),
                false,
            )
        }
        Some(ArtifactEntry::Error(_)) => (
            ContentState::Invalid(format!("{label} content is invalid")),
            false,
        ),
        Some(ArtifactEntry::Loading(_)) => (ContentState::Pending, false),
        None => {
            state
                .artifacts
                .insert(key.clone(), ArtifactEntry::Loading(Vec::new()));
            state.content_requests.insert(key, 0);
            (ContentState::Pending, true)
        }
    }
}

#[derive(Debug, Clone)]
enum ArtifactEntry {
    Loading(Vec<u8>),
    Ready(String),
    Error(String),
}

struct RunListAssembly {
    revision: String,
    total: u64,
    items: Vec<Value>,
}

#[derive(Default)]
struct Shared {
    connected: bool,
    connecting: bool,
    reconnect_attempt: u32,
    error: Option<String>,
    summaries: Vec<Value>,
    run_list: Option<RunListAssembly>,
    run_list_request: Option<(String, u64)>,
    raw_views: HashMap<String, (u64, u64, u64, Value)>,
    next_view_generation: u64,
    watched: HashSet<String>,
    page_requests: HashMap<(String, PageKind), u64>,
    content_requests: HashMap<(String, String), u64>,
    artifacts: HashMap<(String, String), ArtifactEntry>,
    command_ids: CommandIds,
    commands: CommandTracker,
    command_results: Vec<(ClientRequest, CommandOutcome)>,
}

#[derive(Clone)]
enum Endpoint {
    WebSocket(String),
    Local(PathBuf),
}

fn discard_run_state(shared: &mut Shared, run_id: &str) {
    shared.raw_views.remove(run_id);
    shared
        .page_requests
        .retain(|(candidate, _), _| candidate != run_id);
    shared
        .content_requests
        .retain(|(candidate, _), _| candidate != run_id);
    shared
        .artifacts
        .retain(|(candidate, _), _| candidate != run_id);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CachedDecodeState {
    PendingContent,
    Invalid(String),
}

pub struct RemoteRuns {
    shared: Arc<Mutex<Shared>>,
    wake: Option<mpsc::UnboundedSender<()>>,
    worker: Option<JoinHandle<()>>,
    decoded: HashMap<String, RemoteView>,
    decode_states: HashMap<String, (u64, CachedDecodeState)>,
}

impl RemoteRuns {
    pub fn connect(url: &str) -> Result<Self> {
        Self::start(Endpoint::WebSocket(url.to_string()))
    }

    pub fn connect_local(path: &Path) -> Result<Self> {
        Self::start(Endpoint::Local(path.to_path_buf()))
    }

    fn start(endpoint: Endpoint) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            connecting: true,
            ..Shared::default()
        }));
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        let task_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(run_reconnecting(endpoint, task_shared, wake_rx));
        });
        let _ = wake_tx.send(());
        Ok(Self {
            shared,
            wake: Some(wake_tx),
            worker: Some(worker),
            decoded: HashMap::new(),
            decode_states: HashMap::new(),
        })
    }

    pub fn connected(&self) -> bool {
        self.shared.lock().unwrap().connected
    }

    pub fn status_label(&self) -> &'static str {
        let shared = self.shared.lock().unwrap();
        if shared.connected {
            "connected"
        } else if shared.reconnect_attempt > 0 {
            "reconnecting"
        } else if shared.connecting {
            "connecting"
        } else {
            "disconnected"
        }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }

    pub fn summaries(&self) -> Vec<Value> {
        self.shared.lock().unwrap().summaries.clone()
    }

    pub fn watch(&mut self, run_id: &str) {
        let mut shared = self.shared.lock().unwrap();
        let old: Vec<String> = shared.watched.drain().collect();
        for old_id in old {
            if old_id != run_id {
                discard_run_state(&mut shared, &old_id);
                self.decoded.remove(&old_id);
                self.decode_states.remove(&old_id);
            }
        }
        shared.watched.insert(run_id.to_string());
        drop(shared);
        self.wake();
    }

    pub fn request_page(&self, run_id: &str, kind: PageKind, cursor: u64) {
        self.shared
            .lock()
            .unwrap()
            .page_requests
            .insert((run_id.to_string(), kind), cursor);
        self.wake();
    }

    pub fn request_artifact(&self, run_id: &str, path: &str) {
        let key = (run_id.to_string(), path.to_string());
        let mut shared = self.shared.lock().unwrap();
        if shared.artifacts.contains_key(&key) {
            return;
        }
        shared
            .artifacts
            .insert(key.clone(), ArtifactEntry::Loading(Vec::new()));
        shared.content_requests.insert(key, 0);
        drop(shared);
        self.wake();
    }

    pub fn artifact_snapshot(
        &self,
        run_id: &str,
    ) -> HashMap<String, std::result::Result<String, String>> {
        self.shared
            .lock()
            .unwrap()
            .artifacts
            .iter()
            .filter(|((candidate, _), _)| candidate == run_id)
            .filter_map(|((_, path), entry)| {
                let value = match entry {
                    ArtifactEntry::Loading(_) => return None,
                    ArtifactEntry::Ready(content) => Ok(content.clone()),
                    ArtifactEntry::Error(error) => Err(error.clone()),
                };
                Some((path.clone(), value))
            })
            .collect()
    }

    /// Queue a state-changing command (`run.pause`, `run.resume`,
    /// `run.cancel`); it is sent, and re-sent after a reconnect, until the
    /// server answers it.
    pub fn send_command(&self, operation: &str, run_id: &str) {
        let mut shared = self.shared.lock().unwrap();
        let state = &mut *shared;
        let command = build_command(&mut state.command_ids, operation, run_id);
        state.commands.track(command);
        drop(shared);
        self.wake();
    }

    pub fn command_pending(&self, run_id: &str) -> bool {
        self.shared.lock().unwrap().commands.is_pending(run_id)
    }

    /// Answered commands since the last call, oldest first.
    pub fn take_command_results(&self) -> Vec<(ClientRequest, CommandOutcome)> {
        std::mem::take(&mut self.shared.lock().unwrap().command_results)
    }

    pub fn view(&mut self, run_id: &str) -> Option<&RemoteView> {
        let raw = self.shared.lock().unwrap().raw_views.get(run_id).cloned();
        let Some((_, revision, generation, raw)) = raw else {
            self.decoded.remove(run_id);
            self.decode_states.remove(run_id);
            return None;
        };
        if self
            .decoded
            .get(run_id)
            .is_some_and(|view| view.generation == generation)
        {
            self.decode_states.remove(run_id);
            return self.decoded.get(run_id);
        }
        if self
            .decode_states
            .get(run_id)
            .is_some_and(|(cached_generation, _)| *cached_generation == generation)
        {
            return self.decoded.get(run_id);
        }

        let (content, requested) = {
            let mut shared = self.shared.lock().unwrap();
            let (definition, definition_requested) =
                workflow_definition_content(&mut shared, run_id, &raw);
            let (graph_history, graph_requested) = graph_history_content(&mut shared, run_id, &raw);
            let (display_reason, display_reason_requested) =
                display_reason_content(&mut shared, run_id, &raw);
            (
                ViewContent {
                    definition,
                    graph_history,
                    display_reason,
                },
                definition_requested || graph_requested || display_reason_requested,
            )
        };
        if requested {
            self.wake();
        }
        match decode_view(revision, generation, &raw, &content) {
            DecodeOutcome::Ready(decoded) => {
                self.decoded.insert(run_id.to_string(), *decoded);
                self.decode_states.remove(run_id);
            }
            DecodeOutcome::PendingContent => {
                self.decode_states.insert(
                    run_id.to_string(),
                    (generation, CachedDecodeState::PendingContent),
                );
            }
            DecodeOutcome::Invalid(reason) => {
                self.decoded.remove(run_id);
                self.decode_states.insert(
                    run_id.to_string(),
                    (
                        generation,
                        CachedDecodeState::Invalid(format!(
                            "Workflow run snapshot is invalid: {reason}."
                        )),
                    ),
                );
            }
        }
        self.decoded.get(run_id)
    }

    pub fn view_error(&self, run_id: &str) -> Option<String> {
        self.decode_states
            .get(run_id)
            .and_then(|(_, state)| match state {
                CachedDecodeState::Invalid(error) => Some(error.clone()),
                CachedDecodeState::PendingContent => None,
            })
    }

    fn wake(&self) {
        if let Some(wake) = &self.wake {
            let _ = wake.send(());
        }
    }
}

impl Drop for RemoteRuns {
    fn drop(&mut self) {
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn run_reconnecting(
    endpoint: Endpoint,
    shared: Arc<Mutex<Shared>>,
    mut wake: mpsc::UnboundedReceiver<()>,
) {
    let mut attempt = 0u32;
    loop {
        {
            let mut state = shared.lock().unwrap();
            state.connected = false;
            state.connecting = true;
            state.reconnect_attempt = attempt;
            state.commands.connection_lost();
        }
        let result = match &endpoint {
            Endpoint::WebSocket(url) => run_websocket(url, Arc::clone(&shared), &mut wake).await,
            Endpoint::Local(path) => run_local(path, Arc::clone(&shared), &mut wake).await,
        };
        if wake.is_closed() {
            return;
        }
        {
            let mut state = shared.lock().unwrap();
            state.connected = false;
            state.connecting = false;
            state.error = Some(match result {
                Ok(()) => "connection closed".to_string(),
                Err(error) => format!("{error:#}"),
            });
        }
        attempt = attempt.saturating_add(1);
        let delay = (250u64.saturating_mul(1u64 << attempt.min(5))).min(10_000);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
            message = wake.recv() => if message.is_none() { return; }
        }
    }
}

async fn run_websocket(
    url: &str,
    shared: Arc<Mutex<Shared>>,
    wake: &mut mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("connecting to {url}"))?;
    let (mut sink, mut stream) = socket.split();
    let mut sent = HashSet::new();
    let mut counter = 0u64;
    loop {
        for request in reconcile_requests(&shared, &mut sent, &mut counter) {
            let text = encode_request(&request).map_err(anyhow::Error::msg)?;
            sink.send(Message::Text(text.into())).await?;
        }
        tokio::select! {
            message = stream.next() => {
                let Some(message) = message else { return Ok(()) };
                match message? {
                    Message::Text(text) => handle_server_message(text.as_ref(), &shared)?,
                    Message::Close(_) => return Ok(()),
                    Message::Ping(payload) => sink.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    Message::Binary(_) | Message::Frame(_) => anyhow::bail!("server frame must be text"),
                }
            }
            message = wake.recv() => {
                if message.is_none() { return Ok(()); }
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

async fn run_local(
    path: &Path,
    shared: Arc<Mutex<Shared>>,
    wake: &mut mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    #[cfg(unix)]
    {
        let socket = UnixStream::connect(path)
            .await
            .with_context(|| format!("connecting to workflow server {}", path.display()))?;
        run_local_connection(socket, shared, wake).await
    }
    #[cfg(windows)]
    {
        let socket = ClientOptions::new()
            .open(path)
            .with_context(|| format!("connecting to workflow server {}", path.display()))?;
        run_local_connection(socket, shared, wake).await
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("local workflow server transport is not supported on this platform");
}

async fn run_local_connection<S>(
    socket: S,
    shared: Arc<Mutex<Shared>>,
    wake: &mut mpsc::UnboundedReceiver<()>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(socket);
    let mut lines = BufReader::new(read).lines();
    let mut sent = HashSet::new();
    let mut counter = 0u64;
    loop {
        for request in reconcile_requests(&shared, &mut sent, &mut counter) {
            let text = encode_request(&request).map_err(anyhow::Error::msg)?;
            write.write_all(text.as_bytes()).await?;
            write.write_all(b"\n").await?;
            write.flush().await?;
        }
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { return Ok(()) };
                handle_server_message(&line, &shared)?;
            }
            message = wake.recv() => {
                if message.is_none() { return Ok(()); }
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

fn reconcile_requests(
    shared: &Arc<Mutex<Shared>>,
    sent: &mut HashSet<String>,
    counter: &mut u64,
) -> Vec<ClientRequest> {
    let (watched, pages, contents, revisions, run_list_request, commands) = {
        let mut guard = shared.lock().unwrap();
        let state = &mut *guard;
        let commands = state.commands.take_unsent(&mut state.command_ids);
        (
            state.watched.clone(),
            state.page_requests.clone(),
            state.content_requests.clone(),
            state
                .raw_views
                .iter()
                .map(|(run_id, (_, revision, _, _))| (run_id.clone(), *revision))
                .collect::<HashMap<_, _>>(),
            state.run_list_request.clone(),
            commands,
        )
    };
    let mut requests = Vec::new();
    if !sent.contains("runs") {
        requests.push(request(
            counter,
            "view.runs.watch",
            None,
            json!({"subscriptionId":"runs"}),
        ));
        sent.insert("runs".to_string());
    }
    sent.retain(|key| {
        !key.starts_with("runs-page:")
            || run_list_request
                .as_ref()
                .is_some_and(|(revision, cursor)| key == &format!("runs-page:{revision}:{cursor}"))
    });
    if let Some((revision, cursor)) = run_list_request {
        let key = format!("runs-page:{revision}:{cursor}");
        if sent.insert(key) {
            requests.push(request(
                counter,
                "view.runs.page",
                None,
                json!({"revision":revision,"cursor":cursor}),
            ));
        }
    }
    let removals: Vec<String> = sent
        .iter()
        .filter_map(|id| id.strip_prefix("run:").map(str::to_string))
        .filter(|run_id| !watched.contains(run_id))
        .collect();
    for run_id in removals {
        requests.push(request(
            counter,
            "view.run.unwatch",
            Some(&run_id),
            json!({"subscriptionId":format!("run:{run_id}")}),
        ));
        sent.remove(&format!("run:{run_id}"));
    }
    for run_id in watched {
        let subscription = format!("run:{run_id}");
        if sent.insert(subscription.clone()) {
            requests.push(request(
                counter,
                "view.run.watch",
                Some(&run_id),
                json!({"subscriptionId":subscription}),
            ));
        }
    }
    let desired_page_keys = pages
        .iter()
        .map(|((run_id, kind), cursor)| {
            let revision = revisions.get(run_id).copied().unwrap_or(0);
            format!("page:{run_id}:{}:{cursor}:{revision}", page_name(*kind))
        })
        .collect::<HashSet<_>>();
    sent.retain(|key| !key.starts_with("page:") || desired_page_keys.contains(key));
    for ((run_id, kind), cursor) in pages {
        let revision = revisions.get(&run_id).copied().unwrap_or(0);
        let key = format!("page:{run_id}:{}:{cursor}:{revision}", page_name(kind));
        if sent.insert(key) {
            requests.push(request(
                counter,
                "view.page",
                Some(&run_id),
                json!({"kind":page_name(kind),"cursor":cursor}),
            ));
        }
    }
    let desired_content_keys = contents
        .iter()
        .map(|((run_id, path), offset)| format!("content:{run_id}:{path}:{offset}"))
        .collect::<HashSet<_>>();
    sent.retain(|key| !key.starts_with("content:") || desired_content_keys.contains(key));
    for ((run_id, path), offset) in contents {
        let key = format!("content:{run_id}:{path}:{offset}");
        if sent.insert(key) {
            requests.push(request(
                counter,
                "view.content",
                Some(&run_id),
                json!({"path":path,"offset":offset}),
            ));
        }
    }
    requests.extend(commands);
    requests
}

fn request(
    counter: &mut u64,
    operation: &str,
    run_id: Option<&str>,
    payload: Value,
) -> ClientRequest {
    *counter = counter.wrapping_add(1);
    let id = format!("piw-{}-{counter}", std::process::id());
    ClientRequest {
        schema: PROTOCOL_ID.to_string(),
        message_type: "request".to_string(),
        request_id: id.clone(),
        client_id: format!("piw-{}", std::process::id()),
        operation: operation.to_string(),
        idempotency_key: id,
        run_id: run_id.map(str::to_string),
        expected_revision: None,
        payload,
    }
}

fn handle_server_message(text: &str, shared: &Arc<Mutex<Shared>>) -> Result<()> {
    match parse_server_message(text).map_err(anyhow::Error::msg)? {
        ServerMessage::Hello(hello) => {
            anyhow::ensure!(
                hello.package_version == env!("CARGO_PKG_VERSION"),
                "workflow client version mismatch: server {}, piw {}",
                hello.package_version,
                env!("CARGO_PKG_VERSION")
            );
            let mut state = shared.lock().unwrap();
            state.connected = true;
            state.connecting = false;
            state.reconnect_attempt = 0;
            state.error = None;
        }
        ServerMessage::Event(event) => match event.event.as_str() {
            "runs" => {
                start_run_list(&mut shared.lock().unwrap(), &event.payload)?;
            }
            "run_snapshot" => {
                let Some(run_id) = event.run_id else {
                    return Ok(());
                };
                store_snapshot(
                    &mut shared.lock().unwrap(),
                    run_id,
                    event.revision.unwrap_or(0),
                    event.payload,
                );
            }
            _ => {}
        },
        ServerMessage::Response(response) => {
            {
                // Command outcomes belong to the command, not the connection.
                let mut state = shared.lock().unwrap();
                if state.commands.owns(&response.request_id) {
                    let outcome = CommandOutcome::from_response(&response);
                    if let Some(done) = state.commands.complete(&response.request_id, outcome) {
                        state.command_results.push(done);
                    }
                    return Ok(());
                }
            }
            if response.outcome == "unavailable"
                || response.outcome == "rejected"
                || response.outcome == "notFound"
            {
                shared.lock().unwrap().error = Some(
                    response
                        .error
                        .unwrap_or_else(|| format!("workflow request was {}", response.outcome)),
                );
            } else if let Some(receipt) = response.receipt {
                match receipt.get("schema").and_then(Value::as_str) {
                    Some("pi-workflows.run-list-page.v1") => {
                        if response.outcome == "accepted" {
                            merge_run_list(&mut shared.lock().unwrap(), &receipt)?;
                        } else {
                            shared.lock().unwrap().run_list_request = None;
                        }
                    }
                    Some("pi-workflows.run-page.v1") => {
                        if response.outcome == "accepted" {
                            merge_page(&mut shared.lock().unwrap(), &receipt)?;
                        }
                    }
                    Some("pi-workflows.content-chunk.v1") => {
                        merge_content(&mut shared.lock().unwrap(), &receipt)?;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn start_run_list(state: &mut Shared, page: &Value) -> Result<()> {
    let revision = page
        .get("revision")
        .and_then(Value::as_str)
        .context("run list page has no revision")?
        .to_string();
    let start = page
        .get("start")
        .and_then(Value::as_u64)
        .context("run list page has no start")?;
    let total = page
        .get("total")
        .and_then(Value::as_u64)
        .context("run list page has no total")?;
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .context("run list page items are not an array")?;
    anyhow::ensure!(start == 0, "run list snapshot does not start at zero");
    anyhow::ensure!(
        items.len() as u64 <= total,
        "run list page exceeds its total"
    );
    if items.len() as u64 == total {
        state.summaries = items;
        state.run_list = None;
        state.run_list_request = None;
    } else {
        anyhow::ensure!(!items.is_empty(), "run list page made no progress");
        let cursor = items.len() as u64;
        state.run_list = Some(RunListAssembly {
            revision: revision.clone(),
            total,
            items,
        });
        state.run_list_request = Some((revision, cursor));
    }
    Ok(())
}

fn merge_run_list(state: &mut Shared, page: &Value) -> Result<()> {
    let revision = page
        .get("revision")
        .and_then(Value::as_str)
        .context("run list page has no revision")?;
    let start = page
        .get("start")
        .and_then(Value::as_u64)
        .context("run list page has no start")?;
    let total = page
        .get("total")
        .and_then(Value::as_u64)
        .context("run list page has no total")?;
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .context("run list page items are not an array")?;
    let Some(assembly) = state.run_list.as_mut() else {
        return Ok(());
    };
    if assembly.revision != revision {
        return Ok(());
    }
    let expected = assembly.items.len() as u64;
    anyhow::ensure!(
        start == expected,
        "run list page does not continue the snapshot"
    );
    anyhow::ensure!(
        total == assembly.total,
        "run list total changed while paging"
    );
    anyhow::ensure!(!items.is_empty(), "run list page made no progress");
    assembly.items.extend(items);
    anyhow::ensure!(
        assembly.items.len() as u64 <= assembly.total,
        "run list page exceeds its total"
    );
    if assembly.items.len() as u64 == assembly.total {
        state.summaries = std::mem::take(&mut assembly.items);
        state.run_list = None;
        state.run_list_request = None;
    } else {
        state.run_list_request = Some((revision.to_string(), assembly.items.len() as u64));
    }
    Ok(())
}

fn merge_content(state: &mut Shared, receipt: &Value) -> Result<()> {
    let run_id = receipt
        .get("runId")
        .and_then(Value::as_str)
        .context("content chunk has no runId")?;
    let path = receipt
        .get("path")
        .and_then(Value::as_str)
        .context("content chunk has no path")?;
    let offset = receipt
        .get("offset")
        .and_then(Value::as_u64)
        .context("content chunk has no offset")?;
    let next_offset = receipt
        .get("nextOffset")
        .and_then(Value::as_u64)
        .context("content chunk has no nextOffset")?;
    let total = receipt
        .get("bytes")
        .and_then(Value::as_u64)
        .context("content chunk has no byte total")?;
    let sha256 = receipt
        .get("sha256")
        .and_then(Value::as_str)
        .context("content chunk has no digest")?;
    let complete = receipt
        .get("complete")
        .and_then(Value::as_bool)
        .context("content chunk has no completion marker")?;
    let data = BASE64
        .decode(
            receipt
                .get("data")
                .and_then(Value::as_str)
                .context("content chunk has no data")?,
        )
        .context("content chunk is not valid base64")?;
    let key = (run_id.to_string(), path.to_string());
    let Some(entry) = state.artifacts.remove(&key) else {
        return Ok(());
    };
    let ArtifactEntry::Loading(mut bytes) = entry else {
        state.artifacts.insert(key, entry);
        return Ok(());
    };
    if bytes.len() as u64 != offset || offset.saturating_add(data.len() as u64) != next_offset {
        state.artifacts.insert(
            key.clone(),
            ArtifactEntry::Error("workflow content chunk offset is invalid".to_string()),
        );
        state.content_requests.remove(&key);
        bump_view_generation(state, run_id);
        return Ok(());
    }
    bytes.extend_from_slice(&data);
    if complete {
        let digest = hex_sha256(&bytes);
        if bytes.len() as u64 != total || next_offset != total || digest != sha256 {
            state.artifacts.insert(
                key.clone(),
                ArtifactEntry::Error("workflow content digest does not match".to_string()),
            );
        } else {
            match String::from_utf8(bytes) {
                Ok(content) => {
                    state
                        .artifacts
                        .insert(key.clone(), ArtifactEntry::Ready(content));
                }
                Err(_) => {
                    state.artifacts.insert(
                        key.clone(),
                        ArtifactEntry::Error("workflow content is not UTF-8".to_string()),
                    );
                }
            }
        }
        state.content_requests.remove(&key);
        bump_view_generation(state, run_id);
    } else {
        state
            .artifacts
            .insert(key.clone(), ArtifactEntry::Loading(bytes));
        state.content_requests.insert(key, next_offset);
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn merge_page(state: &mut Shared, receipt: &Value) -> Result<()> {
    let run_id = receipt
        .get("runId")
        .and_then(Value::as_str)
        .context("run page has no runId")?;
    let kind = receipt
        .get("kind")
        .and_then(Value::as_str)
        .context("run page has no kind")?;
    let start = receipt.get("start").and_then(Value::as_u64).unwrap_or(0);
    let total = receipt.get("total").and_then(Value::as_u64).unwrap_or(0);
    let items = receipt
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .context("run page items are not an array")?;
    let page_kind = page_kind(kind).context("run page kind is invalid")?;
    let cursor = receipt
        .get("cursor")
        .and_then(Value::as_u64)
        .context("run page has no cursor")?;
    let revision = receipt
        .get("revision")
        .and_then(Value::as_u64)
        .context("run page has no revision")?;
    if state.page_requests.get(&(run_id.to_string(), page_kind)) != Some(&cursor) {
        return Ok(());
    }
    let Some((_, view_revision, generation, view)) = state.raw_views.get_mut(run_id) else {
        return Ok(());
    };
    if *view_revision != revision {
        return Ok(());
    }

    match kind {
        "steps" => {
            view["graphSteps"] = receipt
                .get("graphSteps")
                .cloned()
                .unwrap_or_else(|| Value::Array(items.clone()));
            view["stepStart"] = json!(start);
            view["stepTotal"] = json!(total);
            view["graphCursor"] = receipt.get("graphCursor").cloned().unwrap_or(json!(0));
            view["takenTransitions"] = receipt
                .get("takenTransitions")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if let Some(workflow_state) = view.get_mut("state").and_then(Value::as_object_mut) {
                workflow_state.insert("steps".to_string(), Value::Array(items));
            }
        }
        "trace" | "trace_at_step" => {
            view["tracePage"] = json!({"start":start,"total":total,"items":items});
        }
        "session_entries" | "session_events" => {
            let session = view
                .get_mut("session")
                .and_then(Value::as_object_mut)
                .context("run view has no session object")?;
            session.insert(
                if kind == "session_entries" {
                    "entryPage".to_string()
                } else {
                    "eventPage".to_string()
                },
                json!({"start":start,"total":total,"items":items}),
            );
            if kind == "session_events" {
                session.insert(
                    "replayCheckpoint".to_string(),
                    receipt
                        .get("replayCheckpoint")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
        }
        "settings" => {
            view["settingsScopes"] = Value::Array(items);
            view["settingsStart"] = json!(start);
            view["settingsTotal"] = json!(total);
        }
        "follow_ups" => {
            if !view.get("followUpQueue").is_some_and(Value::is_object) {
                view["followUpQueue"] = json!({});
            }
            if let Some(queue) = view.get_mut("followUpQueue").and_then(Value::as_object_mut) {
                queue.insert("items".to_string(), Value::Array(items));
            }
            view["followUpStart"] = json!(start);
            view["followUpTotal"] = json!(total);
        }
        "updates" => {
            view["updates"] = Value::Array(items.clone());
            view["updateStart"] = json!(start);
            view["updateTotal"] = json!(total);
            if let Some(workflow_state) = view.get_mut("state").and_then(Value::as_object_mut) {
                workflow_state.insert("updates".to_string(), Value::Array(items));
            }
        }
        _ => anyhow::bail!("unsupported run page kind {kind}"),
    }
    *generation = generation.wrapping_add(1);
    Ok(())
}

fn bump_view_generation(state: &mut Shared, run_id: &str) {
    if let Some((_, _, generation, _)) = state.raw_views.get_mut(run_id) {
        *generation = generation.wrapping_add(1);
    }
}

fn store_snapshot(state: &mut Shared, run_id: String, event_revision: u64, value: Value) {
    state.next_view_generation = state.next_view_generation.wrapping_add(1);
    let generation = state.next_view_generation;
    let view_revision = value
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap_or(event_revision);
    state
        .raw_views
        .insert(run_id, (event_revision, view_revision, generation, value));
}

fn page_kind(value: &str) -> Option<PageKind> {
    match value {
        "steps" => Some(PageKind::Steps),
        "trace" => Some(PageKind::Trace),
        "trace_at_step" => Some(PageKind::TraceAtStep),
        "session_entries" => Some(PageKind::SessionEntries),
        "session_events" => Some(PageKind::SessionEvents),
        "settings" => Some(PageKind::Settings),
        "follow_ups" => Some(PageKind::FollowUps),
        "updates" => Some(PageKind::Updates),
        _ => None,
    }
}

fn page_name(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Steps => "steps",
        PageKind::Trace => "trace",
        PageKind::TraceAtStep => "trace_at_step",
        PageKind::SessionEntries => "session_entries",
        PageKind::SessionEvents => "session_events",
        PageKind::Settings => "settings",
        PageKind::FollowUps => "follow_ups",
        PageKind::Updates => "updates",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{apply_patch, PatchOp};
    use crate::render::{render_graph, render_graph_lines, GraphNodeStyle, GraphView};

    fn ready(outcome: DecodeOutcome) -> RemoteView {
        match outcome {
            DecodeOutcome::Ready(view) => *view,
            DecodeOutcome::PendingContent => panic!("view should not need content"),
            DecodeOutcome::Invalid(error) => panic!("view should decode: {error}"),
        }
    }

    fn control_fixture() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root")
            .join("protocol/fixtures/run-view-controls-v1.json");
        serde_json::from_slice(&std::fs::read(path).expect("control fixture"))
            .expect("valid control fixture")
    }

    fn agent_snapshot() -> Value {
        control_fixture()["agentSnapshot"].clone()
    }

    fn remote_runs(shared: Arc<Mutex<Shared>>) -> RemoteRuns {
        RemoteRuns {
            shared,
            wake: None,
            worker: None,
            decoded: HashMap::new(),
            decode_states: HashMap::new(),
        }
    }

    fn hello_message() -> String {
        crate::protocol::canonical_json(&json!({
            "schema":PROTOCOL_ID,
            "type":"hello",
            "connectionId":"test-connection",
            "packageVersion":env!("CARGO_PKG_VERSION")
        }))
        .unwrap()
    }

    fn snapshot_message(snapshot: Value) -> String {
        crate::protocol::canonical_json(&json!({
            "schema":PROTOCOL_ID,
            "type":"event",
            "subscriptionId":"run:run-agent-controls",
            "event":"run_snapshot",
            "revision":4,
            "runId":"run-agent-controls",
            "payload":snapshot
        }))
        .unwrap()
    }

    #[test]
    fn missing_run_watch_sets_a_visible_client_error() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let message = crate::protocol::canonical_json(&json!({
            "schema": PROTOCOL_ID,
            "type": "response",
            "requestId": "watch-missing",
            "outcome": "notFound",
            "error": "Workflow run not found"
        }))
        .unwrap();
        handle_server_message(&message, &shared).unwrap();
        assert_eq!(
            shared.lock().unwrap().error.as_deref(),
            Some("Workflow run not found")
        );
    }

    #[test]
    fn decodes_the_exact_agent_control_snapshot() {
        let raw = agent_snapshot();
        let view = ready(decode_view(4, 1, &raw, &ViewContent::default()));

        assert_eq!(
            view.display.controls,
            vec!["pause", "cancel", "update", "submit"]
        );
        assert_eq!(view.display.active_node(&view.state), None);
    }

    #[test]
    fn malformed_required_snapshot_sections_are_invalid() {
        for (section, reason) in [
            ("manifest", "manifest has invalid required fields"),
            ("state", "state has invalid required fields"),
            ("display", "display has invalid required fields"),
            (
                "workflow",
                "workflow definition has invalid required fields",
            ),
        ] {
            let mut raw = agent_snapshot();
            raw[section] = json!({});
            assert!(matches!(
                decode_view(4, 1, &raw, &ViewContent::default()),
                DecodeOutcome::Invalid(error) if error == reason
            ));
        }
    }

    #[test]
    fn invalid_inline_snapshot_fields_take_priority_over_pending_content() {
        let mut raw = agent_snapshot();
        raw["display"]["controls"] = json!([42]);
        let content = ViewContent {
            definition: ContentState::Pending,
            ..ViewContent::default()
        };

        assert!(matches!(
            decode_view(4, 1, &raw, &content),
            DecodeOutcome::Invalid(error) if error == "display has invalid required fields"
        ));
    }

    #[test]
    fn caches_invalid_snapshots_by_generation_and_recovers_on_a_new_snapshot() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        store_snapshot(
            &mut shared.lock().unwrap(),
            "run-agent-controls".to_string(),
            4,
            agent_snapshot(),
        );
        let mut remote = remote_runs(Arc::clone(&shared));
        assert!(remote.view("run-agent-controls").is_some());
        assert!(remote.view_error("run-agent-controls").is_none());

        let private_value = "private-session-content".repeat(1_000);
        let mut invalid = agent_snapshot();
        invalid["display"]["controls"] = json!([{"private":private_value}]);
        store_snapshot(
            &mut shared.lock().unwrap(),
            "run-agent-controls".to_string(),
            5,
            invalid,
        );
        assert!(remote.view("run-agent-controls").is_none());
        let error = remote
            .view_error("run-agent-controls")
            .expect("invalid snapshot should have a visible error");
        assert_eq!(
            error,
            "Workflow run snapshot is invalid: display has invalid required fields."
        );
        assert!(error.len() < 128);
        let cached_generation = remote.decode_states["run-agent-controls"].0;
        assert!(remote.view("run-agent-controls").is_none());
        assert_eq!(
            remote.decode_states["run-agent-controls"].0,
            cached_generation
        );

        store_snapshot(
            &mut shared.lock().unwrap(),
            "run-agent-controls".to_string(),
            6,
            agent_snapshot(),
        );
        assert!(remote.view("run-agent-controls").is_some());
        assert!(remote.view_error("run-agent-controls").is_none());
    }

    #[test]
    fn referenced_content_is_pending_until_the_verified_artifact_arrives() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut raw = agent_snapshot();
        let definition = serde_json::to_string(&raw["workflow"]).unwrap();
        let path = "artifacts/sha256/agent-definition.json";
        raw["workflow"]["content"] = json!({
            "$artifact": {
                "path":path,
                "mediaType":"application/json",
                "bytes":definition.len(),
                "sha256":hex_sha256(definition.as_bytes())
            }
        });
        store_snapshot(
            &mut shared.lock().unwrap(),
            "run-agent-controls".to_string(),
            4,
            raw,
        );
        let mut remote = remote_runs(Arc::clone(&shared));

        assert!(remote.view("run-agent-controls").is_none());
        assert!(remote.view_error("run-agent-controls").is_none());
        assert!(matches!(
            remote.decode_states["run-agent-controls"].1,
            CachedDecodeState::PendingContent
        ));
        assert_eq!(
            shared
                .lock()
                .unwrap()
                .content_requests
                .get(&("run-agent-controls".to_string(), path.to_string())),
            Some(&0)
        );

        {
            let mut state = shared.lock().unwrap();
            state.artifacts.insert(
                ("run-agent-controls".to_string(), path.to_string()),
                ArtifactEntry::Ready(definition),
            );
            bump_view_generation(&mut state, "run-agent-controls");
        }
        assert!(remote.view("run-agent-controls").is_some());
        assert!(remote.view_error("run-agent-controls").is_none());
    }

    #[tokio::test]
    async fn local_transport_uses_the_shared_snapshot_decoder() {
        for (snapshot, ready_expected) in [
            (agent_snapshot(), true),
            (
                {
                    let mut invalid = agent_snapshot();
                    invalid.as_object_mut().unwrap().remove("manifest");
                    invalid
                },
                false,
            ),
        ] {
            let shared = Arc::new(Mutex::new(Shared::default()));
            shared
                .lock()
                .unwrap()
                .watched
                .insert("run-agent-controls".to_string());
            let (client, mut server) = tokio::io::duplex(1_048_576);
            let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
            let hello = hello_message();
            let event = snapshot_message(snapshot);
            let server_task = tokio::spawn(async move {
                server.write_all(hello.as_bytes()).await.unwrap();
                server.write_all(b"\n").await.unwrap();
                server.write_all(event.as_bytes()).await.unwrap();
                server.write_all(b"\n").await.unwrap();
                server.shutdown().await.unwrap();
            });

            run_local_connection(client, Arc::clone(&shared), &mut wake_rx)
                .await
                .unwrap();
            drop(wake_tx);
            server_task.await.unwrap();
            let mut remote = remote_runs(shared);
            assert_eq!(remote.view("run-agent-controls").is_some(), ready_expected);
            assert_eq!(
                remote.view_error("run-agent-controls").is_some(),
                !ready_expected
            );
        }
    }

    #[tokio::test]
    async fn websocket_transport_uses_the_shared_snapshot_decoder() {
        for (snapshot, ready_expected) in [
            (agent_snapshot(), true),
            (
                {
                    let mut invalid = agent_snapshot();
                    invalid["state"] = json!({"status":"waiting"});
                    invalid
                },
                false,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let hello = hello_message();
            let event = snapshot_message(snapshot);
            let server_task = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                socket.send(Message::Text(hello.into())).await.unwrap();
                socket.send(Message::Text(event.into())).await.unwrap();
                let _ = socket.next().await;
                socket.close(None).await.unwrap();
            });
            let shared = Arc::new(Mutex::new(Shared::default()));
            shared
                .lock()
                .unwrap()
                .watched
                .insert("run-agent-controls".to_string());
            let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();

            run_websocket(
                &format!("ws://{address}"),
                Arc::clone(&shared),
                &mut wake_rx,
            )
            .await
            .unwrap();
            drop(wake_tx);
            server_task.await.unwrap();
            let mut remote = remote_runs(shared);
            assert_eq!(remote.view("run-agent-controls").is_some(), ready_expected);
            assert_eq!(
                remote.view_error("run-agent-controls").is_some(),
                !ready_expected
            );
        }
    }

    #[test]
    fn decodes_the_server_owned_run_view_contract() {
        let source = json!({"kind":"file","path":"/tmp/smoke.workflow.ts","hash":"abc"});
        let raw = json!({
            "manifest": {
                "schema":"pi-workflows.run-manifest.v1",
                "runId":"run-1",
                "workflowName":"smoke",
                "workflowSource":source,
                "startedAt":"2026-01-01T00:00:00.000Z",
                "finishedAt":"2026-01-01T00:00:01.000Z",
                "status":"completed",
                "traceSchema":"pi-workflows.trace-event.v1",
                "paths":{"workflow":"server","state":"server","trace":"server"}
            },
            "state": {
                "schema":"pi-workflows.run-state.v1",
                "traceSeq":1,
                "runId":"run-1",
                "workflowName":"smoke",
                "workflowSource":source,
                "startedAt":"2026-01-01T00:00:00.000Z",
                "finishedAt":"2026-01-01T00:00:01.000Z",
                "updatedAt":"2026-01-01T00:00:01.000Z",
                "status":"completed",
                "input":{},
                "outputs":{},
                "results":{},
                "steps":[],
                "updates":[{"seq":299}]
            },
            "workflow": {
                "schema":"pi-workflows.definition-snapshot.v1",
                "name":"smoke",
                "startAt":"done",
                "nodes":{"done":{"nodeType":"compute"}},
                "edges":[]
            },
            "display": {
                "status":"completed",
                "activity":null,
                "controls":[],
                "reason":null
            },
            "graphSteps":[],
            "takenTransitions":[],
            "graphCursor":0,
            "stepStart":0,
            "stepTotal":0,
            "tracePage":{"start":0,"total":0,"items":[]},
            "session":{
                "binding":null,
                "entryPage":{"start":0,"total":0,"items":[]},
                "eventPage":{"start":0,"total":0,"items":[]},
                "capture":null,
                "integrity":null
            },
            "settingsScopes":[],
            "settingsStart":0,
            "settingsTotal":0,
            "followUpQueue":null,
            "followUpStart":0,
            "followUpTotal":0,
            "updateStart":299,
            "updateTotal":300,
            "live":false,
            "possiblyInterrupted":false
        });

        let view = ready(decode_view(4, 1, &raw, &ViewContent::default()));
        assert_eq!(view.manifest.workflow_name, "smoke");
        assert_eq!(view.state.status, crate::state::types::RunStatus::Completed);
        assert_eq!(view.update_total, 300);

        let mut missing_display = raw;
        missing_display.as_object_mut().unwrap().remove("display");
        assert!(matches!(
            decode_view(4, 1, &missing_display, &ViewContent::default()),
            DecodeOutcome::Invalid(error) if error == "display is missing"
        ));
    }

    #[test]
    fn uses_the_server_display_status_during_an_origin_turn() {
        let source = json!({"kind":"file","path":"/tmp/smoke.workflow.ts","hash":"abc"});
        let reason_content = "The origin-session model turn is active.";
        let raw = json!({
            "manifest": {
                "schema":"pi-workflows.run-manifest.v1",
                "runId":"run-1",
                "workflowName":"smoke",
                "workflowSource":source,
                "startedAt":"2026-01-01T00:00:00.000Z",
                "finishedAt":null,
                "status":"running",
                "traceSchema":"pi-workflows.trace-event.v1",
                "paths":{"workflow":"server","state":"server","trace":"server"}
            },
            "state": {
                "schema":"pi-workflows.run-state.v1",
                "traceSeq":1,
                "runId":"run-1",
                "workflowName":"smoke",
                "workflowSource":source,
                "startedAt":"2026-01-01T00:00:00.000Z",
                "finishedAt":"2026-01-01T00:00:01.000Z",
                "updatedAt":"2026-01-01T00:00:01.000Z",
                "status":"waiting",
                "input":{},
                "outputs":{},
                "results":{},
                "steps":[],
                "waitingOn":"work"
            },
            "display": {
                "status":"running",
                "activity":"origin_turn",
                "controls":["pause","cancel"],
                "reason":"The model is running.",
                "reasonContent": {
                    "$artifact": {
                        "path":"artifacts/sha256/display-reason.txt",
                        "mediaType":"text/plain",
                        "bytes":reason_content.len(),
                        "sha256":hex_sha256(reason_content.as_bytes())
                    }
                }
            },
            "workflow": {
                "schema":"pi-workflows.definition-snapshot.v1",
                "name":"smoke",
                "startAt":"work",
                "nodes":{"work":{"nodeType":"agent"}},
                "edges":[]
            }
        });

        let view = ready(decode_view(
            4,
            1,
            &raw,
            &ViewContent {
                display_reason: ContentState::Ready(reason_content.to_string()),
                ..ViewContent::default()
            },
        ));

        assert_eq!(view.state.status, crate::state::types::RunStatus::Waiting);
        assert_eq!(view.display.status, crate::state::types::RunStatus::Running);
        assert_eq!(
            view.display.reason_content,
            Some(Value::String(reason_content.to_string()))
        );
        assert_eq!(view.display.active_node(&view.state), Some("work"));

        let graph = GraphView {
            state: &view.state,
            display: &view.display,
            snapshot: view.snapshot.as_ref(),
            graph_steps: Some(&view.graph_steps),
            taken_transitions: Some(&view.taken_transitions),
        };
        let rendered =
            render_graph_lines(&graph, -1, 1_767_225_660_000, GraphNodeStyle::Line).join("\n");
        assert!(rendered.contains("◐ work"), "{rendered}");
        assert!(!rendered.contains("⏸ work"), "{rendered}");

        let replayed = render_graph(&graph, -1, false, 1_767_225_660_000, GraphNodeStyle::Line)
            .expect("graph should render")
            .canvas
            .render_plain()
            .join("\n");
        assert!(!replayed.contains("◐ work"), "{replayed}");
    }

    #[test]
    fn display_only_patch_changes_the_live_status_without_changing_durable_state() {
        let mut raw = json!({
            "display": {
                "status":"waiting",
                "activity":null,
                "controls":["pause","cancel","answer"],
                "reason":"The workflow is waiting for origin-session input."
            },
            "state":{"status":"waiting"}
        });

        apply_patch(
            &mut raw,
            &[
                PatchOp::Replace {
                    path: "/display/status".into(),
                    value: json!("running"),
                },
                PatchOp::Replace {
                    path: "/display/activity".into(),
                    value: json!("origin_turn"),
                },
            ],
        )
        .unwrap();

        assert_eq!(raw["display"]["status"], "running");
        assert_eq!(raw["display"]["activity"], "origin_turn");
        assert_eq!(raw["state"]["status"], "waiting");
    }

    #[test]
    fn large_workflow_definitions_load_before_layout() {
        let nodes = (0..300)
            .map(|index| (format!("node-{index}"), json!({"nodeType":"compute"})))
            .collect::<serde_json::Map<_, _>>();
        let full = json!({
            "schema":"pi-workflows.definition-snapshot.v1",
            "name":"large",
            "startAt":"node-0",
            "nodes":nodes,
            "edges":[]
        });
        let content = serde_json::to_string(&full).unwrap();
        let path = "artifacts/sha256/definition.json";
        let raw = json!({
            "workflow": {
                "schema":"pi-workflows.definition-snapshot.v1",
                "name":"large",
                "startAt":"node-0",
                "nodes":{"node-0":{"nodeType":"compute"}},
                "edges":[],
                "content": {
                    "$artifact": {
                        "path":path,
                        "mediaType":"application/json",
                        "bytes":content.len(),
                        "sha256":hex_sha256(content.as_bytes()),
                        "opaque":true
                    }
                }
            }
        });
        let mut state = Shared::default();
        let (missing, requested) = workflow_definition_content(&mut state, "run-1", &raw);
        assert_eq!(missing, ContentState::Pending);
        assert!(requested);
        assert_eq!(
            state
                .content_requests
                .get(&("run-1".to_string(), path.to_string())),
            Some(&0)
        );
        state.artifacts.insert(
            ("run-1".to_string(), path.to_string()),
            ArtifactEntry::Ready(content),
        );
        let (loaded, requested) = workflow_definition_content(&mut state, "run-1", &raw);
        assert!(!requested);
        let snapshot: DefinitionSnapshot =
            serde_json::from_value(workflow_definition_value(&raw, loaded.as_deref()).unwrap())
                .unwrap();
        assert_eq!(snapshot.nodes.len(), 300);
        assert_eq!(layout_graph(&snapshot).rank_of_node.len(), 300);

        let mut invalid = raw;
        invalid["workflow"]["content"]["$artifact"]["sha256"] = json!("0".repeat(64));
        let (loaded, requested) = workflow_definition_content(&mut state, "run-1", &invalid);
        assert!(matches!(loaded, ContentState::Invalid(_)));
        assert!(!requested);
        assert!(state.error.is_none());
    }

    #[test]
    fn complete_graph_history_uses_the_shared_content_loader() {
        let history = json!({
            "steps":[],
            "transitions":(0..300).map(|index| format!("node-{index}->node-{}", index + 1)).collect::<Vec<_>>()
        });
        let content = serde_json::to_string(&history).unwrap();
        let path = "artifacts/sha256/graph-history.json";
        let raw = json!({
            "graphSteps":[],
            "takenTransitions":["node-0->node-1"],
            "graphHistory": {
                "$artifact": {
                    "path":path,
                    "mediaType":"application/json",
                    "bytes":content.len(),
                    "sha256":hex_sha256(content.as_bytes()),
                    "opaque":true
                }
            }
        });
        let mut state = Shared::default();
        let (missing, requested) = graph_history_content(&mut state, "run-1", &raw);
        assert_eq!(missing, ContentState::Pending);
        assert!(requested);
        state.artifacts.insert(
            ("run-1".to_string(), path.to_string()),
            ArtifactEntry::Ready(content),
        );
        let (loaded, requested) = graph_history_content(&mut state, "run-1", &raw);
        assert!(!requested);
        let complete = graph_history_value(&raw, loaded.as_deref()).unwrap();
        assert_eq!(complete["transitions"].as_array().map(Vec::len), Some(300));
    }

    #[test]
    fn complete_display_reason_uses_the_shared_content_loader() {
        let content = "complete workflow failure reason";
        let path = "artifacts/sha256/display-reason.txt";
        let raw = json!({
            "display": {
                "status":"failed",
                "activity":null,
                "controls":[],
                "reason":"Complete workflow failure details are available.",
                "reasonContent": {
                    "$artifact": {
                        "path":path,
                        "mediaType":"text/plain",
                        "bytes":content.len(),
                        "sha256":hex_sha256(content.as_bytes())
                    }
                }
            }
        });
        let mut state = Shared::default();
        let (missing, requested) = display_reason_content(&mut state, "run-1", &raw);
        assert_eq!(missing, ContentState::Pending);
        assert!(requested);
        state.artifacts.insert(
            ("run-1".to_string(), path.to_string()),
            ArtifactEntry::Ready(content.to_string()),
        );
        let (loaded, requested) = display_reason_content(&mut state, "run-1", &raw);
        assert!(!requested);
        assert_eq!(
            display_reason_value(&raw, loaded.as_deref()),
            Some(Value::String(content.to_string()))
        );
    }

    #[test]
    fn a_run_page_updates_only_its_selected_window() {
        let mut state = Shared::default();
        state.raw_views.insert(
            "run-1".to_string(),
            (
                3,
                3,
                1,
                json!({
                    "state":{"steps":[],"updates":[]},
                    "session":{"entryPage":{"items":[]},"eventPage":{"items":[]}},
                    "settingsScopes":[{"change":1}],
                    "settingsStart":0,
                    "settingsTotal":1
                }),
            ),
        );
        state
            .page_requests
            .insert(("run-1".to_string(), PageKind::Settings), 44);
        state
            .page_requests
            .insert(("run-1".to_string(), PageKind::Steps), 0);
        state
            .page_requests
            .insert(("run-1".to_string(), PageKind::FollowUps), 2);
        state
            .page_requests
            .insert(("run-1".to_string(), PageKind::SessionEvents), 256);
        merge_page(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-page.v1",
                "runId":"run-1",
                "revision":3,
                "kind":"settings",
                "cursor":44,
                "start":44,
                "total":300,
                "items":[{"change":299}]
            }),
        )
        .unwrap();
        merge_page(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-page.v1",
                "runId":"run-1",
                "revision":3,
                "kind":"steps",
                "cursor":0,
                "start":0,
                "total":1,
                "items":[{"step":0}],
                "graphSteps":[{"node":"one"}],
                "graphCursor":0,
                "takenTransitions":[]
            }),
        )
        .unwrap();
        merge_page(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-page.v1",
                "runId":"run-1",
                "revision":3,
                "kind":"follow_ups",
                "cursor":2,
                "start":2,
                "total":3,
                "items":[{"followUpId":"follow-3"}]
            }),
        )
        .unwrap();
        merge_page(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-page.v1",
                "runId":"run-1",
                "revision":3,
                "kind":"session_events",
                "cursor":256,
                "start":256,
                "total":300,
                "items":[{"seq":257}],
                "replayCheckpoint":{"throughSeq":256}
            }),
        )
        .unwrap();
        let (_, _, generation, view) = state.raw_views.get("run-1").unwrap();
        assert_eq!(*generation, 5);
        assert_eq!(view["settingsStart"], 44);
        assert_eq!(view["settingsScopes"], json!([{"change":299}]));
        assert_eq!(view["graphSteps"], json!([{"node":"one"}]));
        assert_eq!(view["state"]["steps"], json!([{"step":0}]));
        assert_eq!(
            view["followUpQueue"]["items"],
            json!([{"followUpId":"follow-3"}])
        );
        assert_eq!(view["session"]["replayCheckpoint"]["throughSeq"], 256);
    }

    #[test]
    fn stale_run_pages_cannot_replace_the_requested_window() {
        let mut state = Shared::default();
        state.raw_views.insert(
            "run-1".to_string(),
            (
                1,
                3,
                1,
                json!({"state":{"steps":[]},"graphSteps":[],"takenTransitions":[]}),
            ),
        );
        state
            .page_requests
            .insert(("run-1".to_string(), PageKind::Steps), 20);
        let receipt = |cursor: u64, revision: u64, item: u64| {
            json!({
                "schema":"pi-workflows.run-page.v1",
                "runId":"run-1",
                "revision":revision,
                "kind":"steps",
                "cursor":cursor,
                "start":cursor,
                "total":100,
                "items":[{"step":item}],
                "graphSteps":[],
                "graphCursor":cursor,
                "takenTransitions":[]
            })
        };
        merge_page(&mut state, &receipt(10, 3, 10)).unwrap();
        merge_page(&mut state, &receipt(20, 2, 2)).unwrap();
        assert_eq!(state.raw_views["run-1"].2, 1);
        merge_page(&mut state, &receipt(20, 3, 20)).unwrap();
        assert_eq!(state.raw_views["run-1"].2, 2);
        assert_eq!(
            state.raw_views["run-1"].3["state"]["steps"],
            json!([{"step":20}])
        );
    }

    #[test]
    fn paged_run_lists_publish_only_complete_matching_revisions() {
        let mut state = Shared::default();
        start_run_list(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-list-page.v1",
                "revision":"3:10:20",
                "start":0,
                "total":3,
                "items":[{"runId":"run-1"},{"runId":"run-2"}]
            }),
        )
        .unwrap();
        assert!(state.summaries.is_empty());
        assert_eq!(state.run_list_request, Some(("3:10:20".to_string(), 2)));
        merge_run_list(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-list-page.v1",
                "revision":"stale",
                "start":2,
                "total":3,
                "items":[{"runId":"stale"}]
            }),
        )
        .unwrap();
        assert!(state.summaries.is_empty());
        merge_run_list(
            &mut state,
            &json!({
                "schema":"pi-workflows.run-list-page.v1",
                "revision":"3:10:20",
                "start":2,
                "total":3,
                "items":[{"runId":"run-3"}]
            }),
        )
        .unwrap();
        assert_eq!(state.summaries.len(), 3);
        assert!(state.run_list_request.is_none());
    }

    #[test]
    fn switching_runs_discards_old_pages_content_and_artifacts() {
        let mut state = Shared::default();
        state
            .raw_views
            .insert("old".to_string(), (1, 1, 1, json!({})));
        state
            .page_requests
            .insert(("old".to_string(), PageKind::Steps), 10);
        state
            .content_requests
            .insert(("old".to_string(), "old.json".to_string()), 0);
        state.artifacts.insert(
            ("old".to_string(), "old.json".to_string()),
            ArtifactEntry::Ready("old".to_string()),
        );
        state
            .page_requests
            .insert(("current".to_string(), PageKind::Steps), 20);

        discard_run_state(&mut state, "old");

        assert!(!state.raw_views.contains_key("old"));
        assert!(state
            .page_requests
            .keys()
            .all(|(run_id, _)| run_id != "old"));
        assert!(state
            .content_requests
            .keys()
            .all(|(run_id, _)| run_id != "old"));
        assert!(state.artifacts.keys().all(|(run_id, _)| run_id != "old"));
        assert!(state
            .page_requests
            .contains_key(&("current".to_string(), PageKind::Steps)));
    }

    #[test]
    fn page_requests_can_return_to_an_earlier_window() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        shared
            .lock()
            .unwrap()
            .raw_views
            .insert("run-1".to_string(), (7, 7, 1, json!({})));
        let mut sent = HashSet::new();
        let mut counter = 0;
        let request_at = |cursor: u64,
                          shared: &Arc<Mutex<Shared>>,
                          sent: &mut HashSet<String>,
                          counter: &mut u64| {
            shared
                .lock()
                .unwrap()
                .page_requests
                .insert(("run-1".to_string(), PageKind::Steps), cursor);
            reconcile_requests(shared, sent, counter)
                .into_iter()
                .filter(|request| request.operation == "view.page")
                .count()
        };
        assert_eq!(request_at(10, &shared, &mut sent, &mut counter), 1);
        assert_eq!(request_at(20, &shared, &mut sent, &mut counter), 1);
        assert_eq!(request_at(10, &shared, &mut sent, &mut counter), 1);
    }

    #[test]
    fn content_chunks_are_reassembled_and_verified() {
        let content = br#"{"complete":true}"#.to_vec();
        let path = "artifacts/sha256/content.json";
        let key = ("run-1".to_string(), path.to_string());
        let mut state = Shared::default();
        state
            .raw_views
            .insert("run-1".to_string(), (1, 1, 1, json!({"workflow":null})));
        state
            .artifacts
            .insert(key.clone(), ArtifactEntry::Loading(Vec::new()));
        state.content_requests.insert(key.clone(), 0);
        merge_content(
            &mut state,
            &json!({
                "schema":"pi-workflows.content-chunk.v1",
                "runId":"run-1",
                "path":path,
                "mediaType":"application/json",
                "bytes":content.len(),
                "sha256":hex_sha256(&content),
                "offset":0,
                "nextOffset":content.len(),
                "complete":true,
                "data":BASE64.encode(&content)
            }),
        )
        .unwrap();
        assert!(
            matches!(state.artifacts.get(&key), Some(ArtifactEntry::Ready(value)) if value == "{\"complete\":true}")
        );
        assert!(!state.content_requests.contains_key(&key));
        assert_eq!(state.raw_views["run-1"].2, 2);
    }

    #[test]
    fn bad_content_offsets_and_digests_become_bounded_run_errors() {
        let content = b"{}";
        for (offset, next_offset, digest) in [(1, 3, hex_sha256(content)), (0, 2, "0".repeat(64))] {
            let path = "artifacts/sha256/content.json";
            let key = ("run-1".to_string(), path.to_string());
            let mut state = Shared::default();
            state
                .raw_views
                .insert("run-1".to_string(), (1, 1, 1, json!({})));
            state
                .artifacts
                .insert(key.clone(), ArtifactEntry::Loading(Vec::new()));
            state.content_requests.insert(key.clone(), 0);

            merge_content(
                &mut state,
                &json!({
                    "schema":"pi-workflows.content-chunk.v1",
                    "runId":"run-1",
                    "path":path,
                    "mediaType":"application/json",
                    "bytes":content.len(),
                    "sha256":digest,
                    "offset":offset,
                    "nextOffset":next_offset,
                    "complete":true,
                    "data":BASE64.encode(content)
                }),
            )
            .unwrap();
            assert!(matches!(
                state.artifacts.get(&key),
                Some(ArtifactEntry::Error(_))
            ));
            let (resolved, requested) = referenced_content(
                &mut state,
                "run-1",
                ArtifactRef {
                    path: path.to_string(),
                    media_type: "application/json".to_string(),
                    bytes: content.len() as u64,
                    sha256: hex_sha256(content),
                },
                &["application/json"],
                "workflow definition",
            );
            assert_eq!(
                resolved,
                ContentState::Invalid("workflow definition content is invalid".to_string())
            );
            assert!(!requested);
            assert!(state.error.is_none());
        }
    }
}

````

=== TASK SPECIFICATION ===

Slice id: mirror
Target file: `crates/codegen/xai-grok-pager/src/piw/mirror.rs` (new, in the Grok Build pager library)
Goal: let Grok Build's existing flow screens (the runs pop-up with its list, run view, step detail and inspector,
the side-panel list of runs) show pi-workflows runs next to acpx runs WITHOUT those screens changing.

Why a mirror. Those screens do not take run data in memory. They read acpx run folders from disk, synchronously, on
a one-second tick: `acpx/bundle.rs` (attached whole) lists and loads runs from a runs directory, step conversations
are read from session event files through `acpx/sessions.rs` (attached), agent settings come from `flow.json` and
`projections/steps.json` under the run folder (`node_settings`, `step_settings`), stdout / stderr / outputs are
artifact files (`read_artifact`), and the screens call these with `run.summary.dir` (`views/acpx_modal/run.rs`,
`detail.rs`, `ledger.rs`, `views/side_panel/flows.rs`, `app/dispatch/acpx_run.rs`, all attached whole). So the only
way to feed them unchanged is to give them a run folder. This slice keeps, for each pi-workflows run, a folder in
acpx's run-folder format under a mirror directory, written and updated from what a `PiwClient` (surface below;
another engineer is writing it now) has cached. The reading code in `bundle.rs` and `sessions.rs` is the exact
specification of the format: every file, field and event name they read, and how. The folder layout is also
described in prose in the `RUN_FOLDER` string of the attached `task-scripts/acpx-mode.py` (ignore the rest of that
file). Read the screens to see which fields actually reach the user (for example the trace event names the timeline
looks for in `run.rs` and `detail.rs`, how a waiting run's choices come from `bundle::gate_choices`, the 90 second
stall rule in `bundle::is_stalled`), and write exactly what makes them show a pi-workflows run correctly.

The source is `WorkflowRunSummary` / `WorkflowRunView` JSON. `piw`'s decoder of the same views
(`tui/src/state/types.rs`, `tui/src/client.rs`) is attached as a worked example.

What a run view does and does not carry. The view is built by `readRun` in `src/server/view.ts` (attached whole;
it, not `server.ts`, is the authority) and typed in `src/client/view.ts`. Read both before relying on any field:
- `state.steps` is ONE byte-bounded page of full step records around the latest step, not every step. The complete
  list of attempts is `graphSteps`: compact records whose `prompt` and `output` are null.
- A step's `prompt` and `output` exist only on its FINISHED step record. The step that is running now is only
  `state.currentNode` / `state.currentAttemptId` / `state.currentNodeStartedAt`; it has no step record yet.
- Any value over 16 KiB (a prompt, an output) is replaced by a `$artifact` content reference; the text is fetched
  separately (`PiwClient::content`).
- For large workflow definitions the nodes in the view keep only a few fields; `agent`, `model` and the
  human-decision details may be missing. Treat every definition field as optional.
- The view has the session the run belongs to (`queue.originSessionId`) and timestamps (`queue.createdAt` /
  `startedAt` / `finishedAt`, each step's `startedAt` / `finishedAt`).
- The view does NOT carry the pending gate: the question's request id and revision are only in the SESSION
  snapshot's `interaction` (`ClientInteractiveRequest` in `src/client/view.ts`), which `PiwClient::watch_session`
  / `session` deliver. Use `gate_from` (below) rather than reading it yourself.

Live step activity on the wire. The coordinator is adding this to pi-workflows (server and `src/client/protocol.ts`);
treat it as existing even though the attached pi-workflows files do not have it yet:
- Request operation "view.run.activity.watch": envelope `runId` = the run; payload
  {"subscriptionId": <string chosen by the client>, "afterSeq": <integer >= 0>}. Answered like the other view watches.
- The server then sends events
  {"schema":"pi-workflows.client.v1","type":"event","subscriptionId":<id>,"event":"step_activity","runId":<run>,
   "payload":{"activities":[<StepActivity>, ...]}}
  first with everything already remembered after `afterSeq`, then live. Activities arrive in `seq` order; `seq` is
  per run, starts at 1 and increases by exactly 1. Each frame is under 1 MiB.
  The payload also carries `"reset": true` on the first event of a watch whose `afterSeq` is greater than the
  newest seq the server holds for the run (the server restarted and numbers again from 1): the client must then
  discard the activity it holds for that run before applying the event. `"reset"` is absent otherwise.
- Request operation "view.run.activity.unwatch": payload {"subscriptionId": <id>}.
- One StepActivity:
  {"schema":"pi-workflows.step-activity.v1","runId":…,"nodeId":…,"attemptId":…,"seq":<int>,"at":<ISO time>,
   "kind":"text"|"thought"|"tool"|"status","text":<string>,
   "tool":{"toolCallId":…,"title":…,"status"?:"pending"|"in_progress"|"completed"|"failed","kind"?:…,
           "input"?:<json>,"output"?:<json>,"truncated"?:<bool>}}      ("tool" present exactly when kind is "tool")
  kind "text": a chunk of the agent's answer, APPENDED to the earlier text chunks of that attempt.
  kind "thought": a chunk of the agent's thinking, appended the same way.
  kind "status": one complete status line; replaces the attempt's previous status line.
  kind "tool": the whole current state of one tool call, sent again each time the call changes.
  Activity is not durable: after a server restart or once a run is over the server may remember none of it.
  A server that does not know these two operations closes the connection without answering (see how
  `src/server/server.ts` treats a frame it cannot parse).

Crates: those of the pager library (serde, serde_json, anyhow, chrono, tracing); dev: tempfile.

Environment: Rust edition 2024, toolchain 1.94. No `unsafe`. No `unwrap()` / `expect()` / indexing that can panic on
anything read from the socket, the disk or the user (a poisoned mutex is recovered with `into_inner`, not unwrapped).
Edition 2024: `gen` is a reserved word; `std::env::set_var` is unsafe (do not use it, not even in tests).
Names in the "Public surface" blocks are fixed: other files are being written against them right now, by other
engineers who see only those blocks. You may add private helpers and extra public items. Do not re-declare a type
another block owns; import it by the path given. Do not edit the attached files: the coordinator adds the `mod`
lines and wires this file into the existing code by hand.
JSON from the server is read defensively as `serde_json::Value`: a missing or wrongly typed field makes that one
thing absent; it never panics and never discards the rest.
Tests: a `#[cfg(test)] mod tests` at the bottom of the file; no real pi-workflows server, no network, no real
agent, nothing outside `tempfile::tempdir()`.
Return format: the complete file, nothing omitted, no prose, as:

=== FILE: <repository-relative path> ===
```rust
<complete file>
```

Types you import, do not re-declare:
- `use crate::piw::client::{PiwClient, RunView, StepActivity, ActivityKind, ToolActivity, Content};`

```rust
// File crates/codegen/xai-grok-pager/src/piw/client.rs. Path inside the pager library: `crate::piw::client`;
// from the grok-acpx-acp binary: `xai_grok_pager::piw::client`.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind { Text, Thought, Tool, Status }

#[derive(Debug, Clone, PartialEq)]
pub struct ToolActivity {
    pub tool_call_id: String,
    pub title: String,
    pub status: Option<String>,   // "pending" | "in_progress" | "completed" | "failed"
    pub kind: Option<String>,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepActivity {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub seq: u64,
    pub at: String,
    pub kind: ActivityKind,
    pub text: String,
    pub tool: Option<ToolActivity>,   // Some exactly when kind == Tool
}
impl StepActivity {
    /// None for anything that is not a well-formed StepActivity.
    pub fn from_json(value: &Value) -> Option<StepActivity>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiwResponse {
    pub request_id: String,
    pub operation: String,
    pub run_id: Option<String>,
    pub outcome: String,          // the response's `outcome` word, verbatim
    pub revision: Option<u64>,
    pub receipt: Option<Value>,   // verbatim
    pub error: Option<Value>,     // verbatim
}
impl PiwResponse {
    /// True when the server accepted the request (see the outcome words in src/client/protocol.ts).
    pub fn ok(&self) -> bool;
    /// A one-line human message for a refusal, taken from `error`; None when ok().
    pub fn error_text(&self) -> Option<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState { Connecting, Connected, Reconnecting, Closed }

/// `PiwResponse::outcome` of a request the client gave up on locally because the connection dropped.
pub const CONNECTION_LOST: &str = "connection_lost";

#[derive(Debug, Clone, PartialEq)]
pub struct RunView {
    pub revision: u64,       // the view payload's `revision` (presentation cursor), 0 if absent
    pub run_revision: u64,   // the view payload's `runRevision` (execution revision), 0 if absent
    pub view: Value,         // the latest `run_snapshot` event payload: one complete WorkflowRunView (src/client/view.ts)
}

/// A value the server replaced by a `$artifact` content reference, fetched in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content { Pending, Ready(String), Invalid(String) }

/// Send + Sync. Share it with `Arc<PiwClient>`. Every method except `call` returns immediately and never does I/O
/// on the calling thread: a worker thread owns the socket, reconnects, and re-sends the watches after a reconnect.
pub struct PiwClient { /* private */ }
impl PiwClient {
    /// The server socket of this machine's default state folder, by the same rule as `src/client/protocol.ts`.
    pub fn default_socket_path() -> Option<PathBuf>;
    /// Returns at once; the worker connects in the background. Err only if the worker thread cannot be started.
    pub fn connect(socket: &Path) -> anyhow::Result<PiwClient>;
    pub fn state(&self) -> ConnState;
    pub fn last_error(&self) -> Option<String>;
    /// The `clientId` this client puts on every request.
    pub fn client_id(&self) -> String;
    /// The server's `packageVersion` from its hello, once connected.
    pub fn server_version(&self) -> Option<String>;
    /// How many times the connection has been established (1 after the first connect). The coordinator role and
    /// its epoch belong to one connection, so users of session commands compare this to notice a reconnect.
    pub fn connection_count(&self) -> u64;
    /// Bumps every time any cached list, view, session snapshot or activity changes. Lets pollers skip work.
    pub fn generation(&self) -> u64;

    pub fn watch_runs(&self);
    /// Latest run summaries (WorkflowRunSummary JSON values), in the server's order. Empty until the first event.
    pub fn runs(&self) -> Vec<Value>;
    pub fn watch_run(&self, run_id: &str);
    pub fn unwatch_run(&self, run_id: &str);
    pub fn run_view(&self, run_id: &str) -> Option<RunView>;
    /// Starts (once) the activity watch of the run from seq 0 and keeps everything received.
    pub fn watch_activity(&self, run_id: &str);
    /// Received activity of the run with seq > after_seq, in seq order.
    pub fn activity(&self, run_id: &str, after_seq: u64) -> Vec<StepActivity>;
    /// Watch a session's snapshot; with `coordinator` true the watch also claims the coordinator role.
    pub fn watch_session(&self, session_id: &str, coordinator: bool);
    /// Latest session snapshot payload (`interaction`, `coordinatorEpoch`, `coordinatorActive`,
    /// `branchReportRequired`, ...), verbatim. After a reconnect it is None until the new snapshot arrives.
    pub fn session(&self, session_id: &str) -> Option<Value>;
    /// End the session watch (and with it this connection's hold on the coordinator role, as the server defines).
    pub fn unwatch_session(&self, session_id: &str);
    /// The text behind a `$artifact` content reference found in a view of `run_id`: starts the fetch
    /// (`view.content`, as `piw` does) on first call and returns Pending until it is complete. Cached per reference.
    /// A value that is not a reference gives Invalid.
    pub fn content(&self, run_id: &str, reference: &Value) -> Content;

    /// Queue a request with a fresh idempotency key; returns its requestId. It is sent when connected (and kept
    /// until then).
    pub fn send(&self, operation: &str, run_id: Option<&str>, expected_revision: Option<u64>, payload: Value) -> String;
    /// The same with the caller's idempotency key, so a caller can repeat a request the server may already have
    /// applied (the server deduplicates on the key).
    pub fn send_keyed(&self, operation: &str, run_id: Option<&str>, expected_revision: Option<u64>, payload: Value, idempotency_key: &str) -> String;
    /// The response to a request, once; None while it has not arrived. A request that was on the wire when the
    /// connection dropped is NOT re-sent: it completes with `outcome == CONNECTION_LOST` (ok() false) and the
    /// caller decides whether to repeat it with `send_keyed`.
    pub fn take_response(&self, request_id: &str) -> Option<PiwResponse>;
    /// `send` and wait. Blocks the calling thread up to `timeout`; never call it on the UI thread.
    pub fn call(&self, operation: &str, run_id: Option<&str>, expected_revision: Option<u64>, payload: Value, timeout: Duration) -> anyhow::Result<PiwResponse>;
    /// Stop the worker and close the socket. Later calls are no-ops; `state()` is Closed.
    pub fn close(&self);
}
```

- `use crate::piw::gate::{gate_from, answer_target, PiwGate};`

```rust
// File crates/codegen/xai-grok-pager/src/piw/gate.rs (`crate::piw::gate`, `xai_grok_pager::piw::gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind { Decision, Checkpoint }

#[derive(Debug, Clone, PartialEq)]
pub struct PiwGateChoice {
    pub key: String,                 // the choice's key in the workflow definition; what the answer names
    pub label: String,               // what the user sees (the choice's label, else its key)
    pub text_input: Option<String>,  // name of the text input this choice declares, when it wants text with it
}

/// A run parked on a question for a human.
#[derive(Debug, Clone, PartialEq)]
pub struct PiwGate {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub session_id: String,   // the session the answer goes through (`targetSessionId` of the request)
    pub kind: GateKind,
    pub request_id: String,   // the pending interaction request the answer must name
    pub revision: u64,        // the INTERACTION's revision (sent as expectedRevision), not the run's
    pub question: String,
    pub choices: Vec<PiwGateChoice>,   // sorted by `key` (the wire is canonical JSON; author order is not kept).
                                       // Empty for a checkpoint, which takes a free JSON/text input instead.
}

/// The gate a run is waiting on: Some only when `session` (a session snapshot from `PiwClient::session`) has a
/// pending `interaction` of kind "decision" or "checkpoint" for the run of `view`.
pub fn gate_from(view: &crate::piw::client::RunView, session: &serde_json::Value) -> Option<PiwGate>;

/// Grok Build's existing gate plumbing carries a "flow file" string per choice. For pi-workflows gates that string
/// is an answer target: `piw-answer://<run id>/<choice key>` (both percent-encoded: every byte outside
/// [A-Za-z0-9._-] as %XX). The coordinator routes such targets to the answer path instead of starting a flow.
pub const ANSWER_SCHEME: &str = "piw-answer://";
pub fn answer_target(run_id: &str, choice_key: &str) -> String;
/// (run id, choice key) of an answer target; None for anything else (for example a real flow file path).
pub fn parse_answer_target(target: &str) -> Option<(String, String)>;
```

Public surface (implement exactly):

```rust
/// Everything known about one run at one moment; the pure input of the writer.
pub struct MirrorInput<'a> {
    pub summary: Option<&'a serde_json::Value>,   // its WorkflowRunSummary, when the runs list has it
    pub view: Option<&'a RunView>,                // its run view, when the run is being watched
    pub activity: &'a [StepActivity],             // all activity received for it (may be empty)
    pub gate: Option<&'a PiwGate>,                // the gate it is parked on
    /// Text of a view value: a JSON string as itself, another inline value as compact JSON, a `$artifact`
    /// reference once fetched (None while pending).
    pub text_of: &'a dyn Fn(&serde_json::Value) -> Option<String>,
}

/// The run folder name used for a pi-workflows run id (safe as one path component, reversible).
pub fn folder_name(run_id: &str) -> String;
/// The run id of a mirror folder, None for a folder this module did not write.
pub fn run_id_of_folder(dir: &std::path::Path) -> Option<String>;
/// True when `dir` is a run folder inside `mirror_root` (so callers can route answers and cancels).
pub fn is_mirror_dir(mirror_root: &std::path::Path, dir: &std::path::Path) -> bool;

/// Write or update the run's folder under `mirror_root` so that `bundle::list_runs(mirror_root)` and
/// `bundle::load_run` return the run as the screens should show it. Idempotent: writing the same input twice
/// changes no file. Each file is replaced atomically (temp file + rename in the same directory), so a reader on
/// another thread never sees a half-written file; append-only files (session events, the trace) are appended to.
pub fn write_run(mirror_root: &std::path::Path, input: &MirrorInput<'_>) -> anyhow::Result<std::path::PathBuf>;

/// Keeps the mirror current on its own thread: watches the runs list, mirrors every run's summary, and for runs
/// that are active or were asked for with `want_run` also watches the view, the activity and the session and
/// mirrors those. Never blocks the caller.
pub struct PiwMirror { /* private */ }
impl PiwMirror {
    pub fn start(client: std::sync::Arc<PiwClient>, mirror_root: std::path::PathBuf) -> anyhow::Result<PiwMirror>;
    pub fn root(&self) -> &std::path::Path;
    /// A screen opened this run: mirror it in full from now on (finished runs included).
    pub fn want_run(&self, run_id: &str);
    /// The screen closed it: a finished run stops being watched.
    pub fn release_run(&self, run_id: &str);
    /// The gate a run is parked on, for the answer path.
    pub fn gate(&self, run_id: &str) -> Option<PiwGate>;
    /// "connected" / "connecting" / "reconnecting" / the last error, for a status line.
    pub fn status_line(&self) -> String;
    pub fn stop(&self);
}
```

Behaviour:
- Fidelity is judged through the reader: whatever `bundle.rs` returns for the mirrored folder is what the user
  sees. Fill every field the pi-workflows data can supply with the meaning `bundle.rs` and the screens give it:
  run status (map the pi-workflows display statuses onto the statuses acpx writes; a cancelled, paused or queued
  run becomes the closest acpx status, with the pi-workflows word in the status detail), title, flow name, started
  / updated / finished times, current node, what it is waiting on, the error; the flow definition with node kinds,
  edges and branch edges; per step attempt its kind, outcome, times, prompt, output, error, agent name, model and
  effort (in the places `node_settings` / `step_settings` read them).
- The step's conversation is written as session event records in the shape `sessions.rs` parses, built from the
  step's StepActivity: text chunks appended in order to the agent's message, thoughts likewise, tool calls with
  their latest state. With no activity for a finished step, its output (via `text_of`) is the agent's message. A
  running step shows its text so far and grows on each write. Activity may be gone after a server restart: never
  delete conversation records already written because a later write has less activity.
- A run parked on a gate is written the way acpx writes a run waiting at a checkpoint, with the question, and with
  the choices where `bundle::gate_choices` reads them: label -> `answer_target(run_id, key)` (the screens start the
  "flow file" of the chosen label; the coordinator routes answer targets to the gate answer). A checkpoint gate
  (no choices) is written as waiting with the question and no choices.
- Stall rule: the screens call a running run stalled when its updated time is older than 90 seconds. A
  pi-workflows agent step can run longer than that without the view changing, so while a run is running set the
  updated time from the newest of: the view's own timestamps, the newest activity `at`, and the time of this write
  if the client is connected.
- Trace events: write the events the run view's timeline and the detail screen look for, in time order, once each
  (a rewrite never duplicates an event).
- Missing or wrongly typed fields never panic and never drop the run: that field is left as acpx leaves it when it
  has nothing.
- `PiwMirror`: one `std::thread`, woken by `PiwClient::generation` changes (poll every 250 ms; write only when
  the generation moved or a running run's stall clock needs refreshing, at most once a second per run). Watches the
  session of each fully mirrored run WITHOUT claiming the coordinator role, to get its gate through `gate_from`.
  While the client is reconnecting, the folders stay as last written.
- Nothing is ever deleted from the mirror directory.

Tests (all through the real reader, in a `tempfile::tempdir()`): write a run, then assert on what
`crate::acpx::bundle::list_runs` / `load_run` (and the `sessions.rs` reader for conversations) return. Cover: a
running run with one finished and one running agent step and activity for the running one (its conversation holds
the joined text and the tool call's latest state); the same run written again with more activity (text grows, no
duplicate trace events, unchanged files untouched); a finished run with no activity (outputs used); less activity
on a later write (nothing lost); a run waiting on a decision gate (`gate_choices` returns the labels with answer
targets); a checkpoint gate; a failed run with its error; a cancelled run; a branching definition's edges; agent
settings read back through `node_settings` / `step_settings`; a running run not stalled after a write and stalled
when its inputs are old and the client is not connected; a summary-only run (no view yet) appearing in the list;
garbage values; `folder_name` / `run_id_of_folder` round trip with a run id containing `/` and spaces;
`is_mirror_dir`.
