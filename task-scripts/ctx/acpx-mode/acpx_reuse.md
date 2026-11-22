# Reusable acpx code in the pager crate (visibility matters for a separate bin crate)

Visibility (verified): lib.rs declares `pub mod acp;` and `pub mod acpx;`; acpx/mod.rs declares `pub mod bundle; pub mod graph; pub mod launch; pub mod sessions;` so xai_grok_pager::acpx::{bundle,launch,graph,sessions} are usable from grok-acpx-acp. BUT acp/mod.rs declares `pub(crate) mod acpx_feed;` and `pub(crate) mod claude_feed;` (Event, Fed are pub(crate)), so acpx_feed::convert / convert_text are NOT reachable from the bin crate without changing them to pub (a one-line pager change).

- lib.rs: public modules
```rust
// from crates/codegen/xai-grok-pager/src/lib.rs:12-13
pub mod acp;
pub mod acpx;
```

- acpx/mod.rs
```rust
// from crates/codegen/xai-grok-pager/src/acpx/mod.rs:1-6
//! acpx flow-run bundles, sessions, graph source and launching.

pub mod bundle;
pub mod graph;
pub mod launch;
pub mod sessions;
```

- acp/mod.rs: feed modules are pub(crate)
```rust
// from crates/codegen/xai-grok-pager/src/acp/mod.rs:6-12
pub(crate) mod claude_feed;
pub(crate) mod acpx_feed;
pub(crate) mod live_feed;
pub(crate) mod claude_subagents;
// Not opened from the list yet.
#[allow(dead_code)]
pub(crate) mod codex_feed;
```

- claude_feed: Event and Fed (types returned by acpx_feed)
```rust
// from crates/codegen/xai-grok-pager/src/acp/claude_feed.rs:16-26
/// One update for the handler. `Acp` goes in as `session/update`, `Xai` as `x.ai/session/update`.
pub(crate) enum Event {
    Acp(acp::SessionUpdate),
    Xai(XaiSessionUpdate),
}

/// An update and the `_meta` the handler reads: replay flag, event id, prompt id, timestamps.
pub(crate) struct Fed {
    pub event: Event,
    pub meta: Map<String, Value>,
}
```

- acpx_feed: convert (file) and convert_text (NDJSON text), doc comments and parsing of wrapped/bare acpx event lines
```rust
// from crates/codegen/xai-grok-pager/src/acp/acpx_feed.rs:1-125
//! An acpx session event log (`events.ndjson` or `<id>.stream.ndjson`) turned into the events the pager's session handler consumes.

use std::fs;
use std::io;
use std::path::Path;

use agent_client_protocol as acp;
use chrono::DateTime;
use serde_json::{Map, Value, json};

use super::claude_feed::{Event, Fed, chunk};

/// `range` = inclusive `(start_seq, end_seq)` for a flow step; `None` = the whole file.
pub(crate) fn convert(
    events: &Path,
    range: Option<(u64, u64)>,
    session_id: &str,
) -> io::Result<Vec<Fed>> {
    let text = fs::read_to_string(events)?;
    Ok(convert_text(&text, range, session_id))
}

/// Convert raw NDJSON text from an acpx flow bundle `events.ndjson` or bare `<id>.stream.ndjson` into `Fed` events.
pub(crate) fn convert_text(
    text: &str,
    range: Option<(u64, u64)>,
    session_id: &str,
) -> Vec<Fed> {
    let mut state = Builder {
        sid: session_id,
        seq: 0,
        prompt_id: String::new(),
        out: Vec::new(),
    };

    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        // Detect wrapped format (`{"seq":N,"at":"...","direction":"...","message":{...}}`) vs bare format.
        let (line_seq, ts_ms, msg) = if let Some(message) = val.get("message").filter(|m| m.is_object()) {
            let s = val
                .get("seq")
                .and_then(Value::as_u64)
                .unwrap_or((idx + 1) as u64);
            let ts = val
                .get("at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);
            (s, ts, message)
        } else {
            // Bare format: skip journal header lines.
            if val.get("schema").and_then(Value::as_str) == Some("acpx.session.journal.v1")
                || val.get("type").and_then(Value::as_str) == Some("segment")
            {
                continue;
            }
            ((idx + 1) as u64, None, &val)
        };

        if let Some((start, end)) = range
            && (line_seq < start || line_seq > end)
        {
            continue;
        }

        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue;
        };
        if method.starts_with('_') {
            continue;
        }

        match method {
            "session/prompt" => {
                if let Some(id_val) = msg.get("id") {
                    state.prompt_id = render_id(id_val);
                }
                if let Some(prompts) = msg
                    .get("params")
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_array)
                {
                    for block in prompts {
                        if block.get("type").and_then(Value::as_str) == Some("text")
                            && let Some(t) = block.get("text").and_then(Value::as_str)
                        {
                            let meta = state.meta(ts_ms);
                            state.push(
                                Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(t))),
                                meta,
                            );
                        }
                    }
                }
            }
            "session/update" => {
                let Some(update_val) = msg.get("params").and_then(|p| pContent_update(p)) else {
                    continue;
                };
                let Ok(update) = serde_json::from_value::<acp::SessionUpdate>(update_val.clone())
                else {
                    continue;
                };
                if is_allowed_update(&update) {
                    let meta = state.meta(ts_ms);
                    state.push(Event::Acp(update), meta);
                }
            }
            _ => {}
        }
    }

    state.out
}

fn pContent_update(params: &Value) -> Option<&Value> {
    params.get("update")
}

```

- acpx_feed: Builder stamps _meta isReplay, eventId, promptId, agentTimestampMs
```rust
// from crates/codegen/xai-grok-pager/src/acp/acpx_feed.rs:153-180
struct Builder<'a> {
    sid: &'a str,
    seq: usize,
    prompt_id: String,
    out: Vec<Fed>,
}

impl Builder<'_> {
    fn meta(&mut self, ts: Option<i64>) -> Map<String, Value> {
        self.seq += 1;
        let mut m = Map::new();
        m.insert("isReplay".into(), json!(true));
        m.insert("eventId".into(), json!(format!("{}-{}", self.sid, self.seq)));
        if !self.prompt_id.is_empty() {
            m.insert("promptId".into(), json!(self.prompt_id));
        }
        if let Some(ts) = ts {
            m.insert("agentTimestampMs".into(), json!(ts));
        }
        m
    }

    fn push(&mut self, event: Event, meta: Map<String, Value>) {
        self.out.push(Fed { event, meta });
    }
}

#[cfg(test)]
```

- how Fed becomes an ACP message (SessionNotification or x.ai/session/update ext)
```rust
// from crates/codegen/xai-grok-pager/src/app/dispatch/standalone_traces.rs:467-487
pub(super) fn message(session_id: &str, fed: Fed) -> AcpClientMessage {
    let (response_tx, _rx) = tokio::sync::oneshot::channel();
    match fed.event {
        Event::Acp(update) => AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
            request: acp::SessionNotification::new(acp::SessionId::new(session_id), update).meta(Some(fed.meta)),
            response_tx,
        }),
        Event::Xai(update) => {
            let payload = SessionNotification {
                session_id: acp::SessionId::new(session_id),
                update,
                meta: Some(serde_json::Value::Object(fed.meta)),
            };
            let params = serde_json::value::to_raw_value(&payload).expect("session notification serializes");
            AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                request: acp::ExtNotification::new("x.ai/session/update", params.into()),
                response_tx,
            })
        }
    }
}
```

- bundle.rs imports
```rust
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1-8
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use chrono::DateTime;
use serde_json::Value;

```

- public items of acpx/bundle.rs (signatures only; struct and enum definitions verbatim)
```rust
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:9-17
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus {
    Running,
    Waiting,
    Completed,
    Failed,
    TimedOut,
    Unknown,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:19-26
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Acp,
    Action,
    Compute,
    Checkpoint,
    Unknown,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:28-35
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    TimedOut,
    Failed,
    Cancelled,
    Unknown,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:37-44
/// What a node looks like right now, derived from steps + live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    NotRun,
    Running,
    Waiting,
    Done(Outcome),
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:46-46
impl RunStatus {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:47-47
    pub fn from_str_tolerant(s: &str) -> Self { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:58-58
    pub fn label(self) -> &'static str { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:69-69
    pub fn glyph(self) -> &'static str { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:80-80
    pub fn is_live(self) -> bool { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:85-85
impl NodeKind {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:86-86
    pub fn from_str_tolerant(s: &str) -> Self { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:96-96
    pub fn label(self) -> &'static str { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:107-107
impl Outcome {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:108-108
    pub fn from_str_tolerant(s: &str) -> Self { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:119-119
impl NodeState {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:120-120
    pub fn glyph(self) -> &'static str { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:134-139
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactRef {
    pub path: String,
    pub media_type: String,
    pub bytes: u64,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:141-146
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub session_id: String,
    pub event_start_seq: u64,
    pub event_end_seq: u64,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:148-156
#[derive(Debug, Clone, PartialEq)]
pub struct ActionReceipt {
    pub action_type: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:158-177
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
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:179-179
impl Step {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:180-180
    pub fn duration_ms(&self) -> u64 { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:186-192
#[derive(Debug, Clone, PartialEq)]
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    pub summary: Option<String>,
    pub is_shell: bool,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:194-205
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
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:207-215
/// flow.json. `nodes` is ordered breadth-first from `start_at` following edges (Simple first, then Switch cases in
/// order); nodes not reachable are appended sorted by id.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowDef {
    pub name: String,
    pub start_at: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:217-227
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
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:229-235
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRef {
    pub id: String,
    pub handle: String,
    pub events_path: PathBuf,
    pub agent_name: Option<String>,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:237-252
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
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:254-262
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub summary: RunSummary,
    pub flow: FlowDef,
    pub steps: Vec<Step>,
    pub events: Vec<TraceEvent>,
    pub input: Value,
    pub sessions: Vec<SessionRef>,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:264-264
impl Run {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:265-265
    pub fn node_state(&self, node_id: &str) -> NodeState { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:280-280
    pub fn taken_path(&self) -> Vec<String> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:284-284
    pub fn edge_taken(&self, from: &str, to: &str) -> bool { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:290-290
    pub fn step_events(&self, attempt_id: &str) -> Vec<&TraceEvent> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:297-297
    pub fn progress(&self) -> (usize, usize) { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:302-302
    pub fn duration_ms(&self, now_ms: i64) -> u64 { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:314-314
    pub fn session_events_path(&self, session_id: &str) -> Option<&Path> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:322-322
pub fn runs_base() -> Option<PathBuf> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:330-330
pub fn list_runs(base: &Path) -> Vec<RunSummary> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:360-360
pub fn load_run(dir: &Path) -> io::Result<Run> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:423-425
/// Re-read a run in place. Each file that fails to parse (acpx rewrites them non-atomically) keeps its previous value.
/// Returns true when anything changed.
pub fn refresh(run: &mut Run) -> bool { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:502-507
/// Read an artifact as text, at most `max_bytes` (cut on a char boundary). Rejects absolute paths and any `..` component.
pub fn read_artifact(
    run_dir: &Path,
    artifact: &ArtifactRef,
    max_bytes: usize,
) -> io::Result<String> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:543-543
pub fn parse_ts(s: &str) -> Option<i64> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:549-549
pub fn fmt_duration(ms: u64) -> String { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:576-576
pub fn value_text(v: &Value) -> String { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1160-1168
/// Who ran (or will run) an agent node. Every field is None for non-agent nodes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentSettings {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub handle: Option<String>,
    pub acp_session_id: Option<String>,
}
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1170-1170
impl AgentSettings {
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1171-1172
    /// "grok-build · grok-4.7 · low" from the parts that are known; "" when none.
    pub fn label(&self) -> String { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1186-1187
/// Per node id, what the flow file asked for (flow.json). Empty on any error.
pub fn node_settings(run_dir: &Path) -> HashMap<String, AgentSettings> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1205-1206
/// Per attempt id, what the step actually ran with (projections/steps.json). Empty on any error.
pub fn step_settings(run_dir: &Path) -> HashMap<String, AgentSettings> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1233-1234
/// Running on paper, but nothing has written the run for 90 s: the acpx process is gone.
pub fn is_stalled(summary: &RunSummary, now_ms: i64) -> bool { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1238-1239
/// The flow file a run was started from (manifest.json, else projections/live.json).
pub fn flow_path(run_dir: &Path) -> Option<PathBuf> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1246-1248
/// A waiting run's gate choices: the waiting checkpoint step's output `next` object (`{ "<label>": "<flow file>" }`),
/// as (label, flow file) in key order. Empty when the run is not waiting or the checkpoint declares none.
pub fn gate_choices(run: &Run) -> Vec<(String, String)> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/bundle.rs:1261-1262
/// What a gate hands the next flow: the first run's input and every node's last output, plus which gate was answered.
pub fn gate_input(run: &Run, choice: &str) -> Value { .. }
```

- public items of acpx/launch.rs (spawns the acpx CLI for a flow file; prints one JSON line when done or at a checkpoint)
```rust
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:12-20
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub flow_file: PathBuf,
    pub input_json: Option<String>,
    pub default_agent: Option<String>,
    pub approve_all: bool,
    pub cwd: Option<PathBuf>,
    pub acpx_bin: Option<String>,
}
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:22-31
/// The single JSON line acpx prints on stdout when the run ends or stops at a checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowRunResult {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub status: RunStatus,
    pub waiting_on: Option<String>,
    pub status_detail: Option<String>,
    pub error: Option<String>,
}
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:33-40
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchState {
    Starting,
    Running { run_dir: PathBuf },
    Finished(FlowRunResult),
    Failed(String),
    Cancelled,
}
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:42-53
pub struct Launch {
    child: Child,
    stdout_rx: Receiver<String>,
    stderr_rx: Receiver<String>,
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
    runs_base: PathBuf,
    before_dirs: HashSet<PathBuf>,
    flow_file: PathBuf,
    run_dir: Option<PathBuf>,
    terminal_state: Option<LaunchState>,
}
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:55-55
impl Launch {
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:56-56
    pub fn start(spec: &LaunchSpec, runs_base: &Path) -> io::Result<Launch> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:71-71
    pub fn poll(&mut self) -> LaunchState { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:135-135
    pub fn run_dir(&self) -> Option<&Path> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:139-139
    pub fn cancel(&mut self) { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:146-146
    pub fn stderr_tail(&self) -> String { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:251-251
pub fn argv(spec: &LaunchSpec) -> Vec<String> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:272-272
pub fn parse_result(stdout: &str) -> Option<FlowRunResult> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:341-345
pub fn find_new_run(
    runs_base: &Path,
    before: &HashSet<PathBuf>,
    flow_file: &Path,
) -> Option<PathBuf> { .. }
// from crates/codegen/xai-grok-pager/src/acpx/launch.rs:374-374
pub fn parse_run_args(args: &str) -> Result<(PathBuf, Option<String>), String> { .. }
```
