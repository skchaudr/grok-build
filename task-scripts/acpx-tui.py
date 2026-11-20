#!/usr/bin/env python3
"""
acpx-tui.py: slices that add an acpx view (`/acpx` modal) to the pager (crates/codegen/xai-grok-pager).

Pure text generation, no tools. Every slice owns ONE new file, gets only the context that file needs, and returns
the complete file. Nothing edits an existing file, so slices cannot conflict. Wiring (mod lines, ActiveModal::Acpx,
key/mouse/draw dispatch, the 1s poll, replay glue) is done afterwards; each prompt states the surface wiring calls.

  python3 task-scripts/acpx-tui.py --dry-run          sizes + estimate, no API call
  python3 task-scripts/acpx-tui.py --only bundle      fire one slice first
  python3 task-scripts/acpx-tui.py                    fire all

Files land in task-scripts/acpx-tui_out/<output_file>; every raw return is kept in acpx-tui_out/_returns/<id>.md.
"""

MODEL = "google/gemini-4-argon"  # OpenRouter id. Override with --model
CTX = "task-scripts/ctx"
SRC = "crates/codegen/xai-grok-pager/src"
MAX_TOKENS = 64000  # per slice; a return cut off at this is marked failed, not written

# ── Contract 1: the data types. Slice `bundle` implements this file; every other slice imports from it. ──────────
BUNDLE_API = """
// ==== crate::acpx::bundle  (file: src/acpx/bundle.rs) ====
use std::path::{Path, PathBuf};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus { Running, Waiting, Completed, Failed, TimedOut, Unknown }   // live.json "status": running|waiting|completed|failed|timed_out
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind { Acp, Action, Compute, Checkpoint, Unknown }                 // "nodeType"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome { Ok, TimedOut, Failed, Cancelled, Unknown }                   // "outcome": ok|timed_out|failed|cancelled
/// What a node looks like right now, derived from steps + live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState { NotRun, Running, Waiting, Done(Outcome) }

impl RunStatus { pub fn label(self) -> &'static str; pub fn glyph(self) -> &'static str; pub fn is_live(self) -> bool /* Running */; }
impl NodeKind  { pub fn label(self) -> &'static str; }       // "acp" "action" "compute" "checkpoint" "?"
impl NodeState { pub fn glyph(self) -> &'static str; }       // NotRun "○", Running "●", Waiting "◐", Done(Ok) "✓", Done(Failed) "✗", Done(TimedOut) "✗", Done(Cancelled) "⊘", Done(Unknown) "?"
// RunStatus::glyph: Running "●", Waiting "◐", Completed "✓", Failed "✗", TimedOut "✗", Unknown "○"

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactRef { pub path: String, pub media_type: String, pub bytes: u64 }     // path is relative to the run dir
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation { pub session_id: String, pub event_start_seq: u64, pub event_end_seq: u64 }
#[derive(Debug, Clone, PartialEq)]
pub struct ActionReceipt { pub action_type: String, pub command: Option<String>, pub args: Vec<String>, pub cwd: Option<String>, pub exit_code: Option<i64>, pub duration_ms: Option<u64> }

/// One attempt of one node, from projections/steps.json, in execution order.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub attempt_id: String,            // "size#1"
    pub node_id: String,
    pub kind: NodeKind,
    pub outcome: Outcome,
    pub started_ms: i64,               // unix ms; 0 when unparseable
    pub finished_ms: i64,
    pub prompt_text: Option<String>,
    pub raw_text: Option<String>,
    pub output: Value,                 // Value::Null when absent; may be a string or an object
    pub error: Option<String>,
    pub agent_name: Option<String>,    // step.agent.agentName
    pub action: Option<ActionReceipt>, // step.trace.action
    pub conversation: Option<Conversation>,   // step.trace.conversation
    pub stdout: Option<ArtifactRef>,   // step.trace.stdoutArtifact
    pub stderr: Option<ArtifactRef>,
    pub output_artifact: Option<ArtifactRef>,
}
impl Step { pub fn duration_ms(&self) -> u64; }   // finished - started, 0 when negative

#[derive(Debug, Clone, PartialEq)]
pub struct FlowNode { pub id: String, pub kind: NodeKind, pub summary: Option<String>, pub is_shell: bool }   // is_shell: actionExecution == "shell"
#[derive(Debug, Clone, PartialEq)]
pub enum FlowEdge {
    Simple { from: String, to: String },
    Switch { from: String, on: String, cases: Vec<(String, String)> },   // (case value, target node id), sorted by case value
}
/// flow.json. `nodes` is ordered breadth-first from `start_at` following edges (Simple first, then Switch cases in
/// order); nodes not reachable are appended sorted by id.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowDef { pub name: String, pub start_at: String, pub nodes: Vec<FlowNode>, pub edges: Vec<FlowEdge> }

/// One line of trace.ndjson.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    pub seq: u64,
    pub at_ms: i64,
    pub scope: String,                 // run | node | action | artifact | session | acp
    pub kind: String,                  // the JSON "type": node_started, node_heartbeat, action_prepared, ...
    pub node_id: Option<String>,
    pub attempt_id: Option<String>,
    pub detail: String,                // one short human line built from the payload ("" when nothing useful), see slice prompt
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRef { pub id: String, pub handle: String, pub events_path: PathBuf, pub agent_name: Option<String> }   // events_path is absolute

/// One row of the runs ledger; cheap to load (projections/live.json, falling back to manifest.json).
#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub dir: PathBuf,
    pub flow_name: String,
    pub title: String,                 // runTitle, else flowName, else run_id
    pub status: RunStatus,
    pub started_ms: i64,
    pub updated_ms: i64,               // updatedAt, else started_ms
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
    pub events: Vec<TraceEvent>,       // ascending seq
    pub input: Value,
    pub sessions: Vec<SessionRef>,
}
impl Run {
    pub fn node_state(&self, node_id: &str) -> NodeState;      // waiting_on == id -> Waiting; current_node == id while status Running -> Running; else last step of that node -> Done(outcome); else NotRun
    pub fn taken_path(&self) -> Vec<String>;                   // node ids of `steps` in order (a node may repeat)
    pub fn edge_taken(&self, from: &str, to: &str) -> bool;    // some consecutive pair in taken_path equals (from, to)
    pub fn step_events(&self, attempt_id: &str) -> Vec<&TraceEvent>;
    pub fn progress(&self) -> (usize, usize);                  // (distinct nodes with a step, flow.nodes.len())
    pub fn duration_ms(&self, now_ms: i64) -> u64;             // finished or updated (or now while Running) minus started
    pub fn session_events_path(&self, session_id: &str) -> Option<&Path>;
}

pub fn runs_base() -> Option<PathBuf>;                         // $HOME/.acpx/flows/runs ; None when HOME is unset
pub fn list_runs(base: &Path) -> Vec<RunSummary>;              // newest started first; unreadable dirs skipped; missing base -> empty
pub fn load_run(dir: &Path) -> std::io::Result<Run>;
/// Re-read a run in place. Each file that fails to parse (acpx rewrites them non-atomically) keeps its previous value.
/// Returns true when anything changed.
pub fn refresh(run: &mut Run) -> bool;
/// Read an artifact as text, at most `max_bytes` (cut on a char boundary). Rejects absolute paths and any `..` component.
pub fn read_artifact(run_dir: &Path, artifact: &ArtifactRef, max_bytes: usize) -> std::io::Result<String>;
pub fn parse_ts(s: &str) -> Option<i64>;                       // RFC 3339 -> unix ms (use chrono)
pub fn fmt_duration(ms: u64) -> String;                        // "7ms" "3.9s" "2m05s" "1h04m"
pub fn value_text(v: &Value) -> String;                        // a JSON string as-is; anything else pretty-printed; Null -> ""
"""

# Hand-written additions to bundle.rs that the second-round slices call. Same module: `crate::acpx::bundle`.
BUNDLE_EXTRA_API = """
// ==== also in crate::acpx::bundle (already written; import what you use) ====
/// Who ran (or will run) an agent node. Every field is None for non-agent nodes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentSettings { pub agent: Option<String>, pub model: Option<String>, pub effort: Option<String>, pub handle: Option<String>, pub acp_session_id: Option<String> }
impl AgentSettings { pub fn label(&self) -> String; }          // the Some parts of agent, model, effort joined by " · ", e.g. "grok-build · grok-4.7 · low"; "" when none
pub fn node_settings(run_dir: &Path) -> std::collections::HashMap<String, AgentSettings>;   // key = node id (from flow.json); empty on any error
pub fn step_settings(run_dir: &Path) -> std::collections::HashMap<String, AgentSettings>;   // key = attempt id (from projections/steps.json); empty on any error
pub fn is_stalled(summary: &RunSummary, now_ms: i64) -> bool;  // status Running but not updated for 90 s: the acpx process is gone
/// A waiting run's gate choices: the waiting checkpoint step's output object `next` ({ "<label>": "<flow file>" }),
/// as (label, flow file) in key order. Empty when the run is not waiting or the checkpoint declares none.
pub fn gate_choices(run: &Run) -> Vec<(String, String)>;
"""

SESSIONS_API = """
// ==== crate::acpx::sessions  (file: src/acpx/sessions.rs) ====
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub record_id: String,             // acpx_record_id
    pub acp_session_id: String,
    pub name: String,
    pub title: String,                 // title, else name, else record_id
    pub agent: String,                 // short agent label, see slice prompt
    pub cwd: String,
    pub created_ms: i64,
    pub last_used_ms: i64,
    pub closed: bool,
    pub messages: usize,
    pub tokens: u64,                   // cumulative_token_usage total (0 when absent)
    pub cost_usd: Option<f64>,
    pub stream_path: std::path::PathBuf,   // <dir>/<record_id>.stream.ndjson (may not exist)
}
pub fn sessions_base() -> Option<std::path::PathBuf>;                  // $HOME/.acpx/sessions
pub fn list_sessions(base: &std::path::Path) -> Vec<SessionSummary>;   // most recently used first
pub fn agent_label(agent_command: &str) -> String;
"""

GRAPH_API = """
// ==== crate::acpx::graph  (file: src/acpx/graph.rs) ====
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { LeftRight, TopDown }
/// Mermaid `flowchart` source for a run: one node per flow node, labelled "<state glyph> <id>".
pub fn mermaid_source(run: &crate::acpx::bundle::Run, direction: Direction) -> String;
"""

DETAIL_API = """
// ==== crate::views::acpx_modal::detail  (file: src/views/acpx_modal/detail.rs) ====
/// What the right-hand panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum DetailTarget {
    Run,             // overview of the whole run
    Step(usize),     // index into run.steps: one finished attempt
    Node(String),    // a node id that has no step record yet (running, waiting, or not run)
}
pub struct DetailPanel { /* private */ }
impl DetailPanel {
    pub fn new() -> Self;
    pub fn scroll(&mut self, delta: isize) -> bool;   // true when the offset moved. Scrolling up turns tail-following off; reaching the bottom turns it on
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, run: &Run, target: &DetailTarget, theme: &Theme, now_ms: i64);
}
"""

# ── Contract 2: the view surface. Slices ledger/run_view/inspector implement it; slice `modal` drives it. ─────────
VIEW_API = """
// ==== crate::views::acpx_modal::ledger  (file: src/views/acpx_modal/ledger.rs) ====
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerTab { Flows, Sessions }
#[derive(Debug, Clone, PartialEq)]
pub enum LedgerOutcome { Unchanged, Changed, OpenRun(std::path::PathBuf), OpenSession(SessionSummary), Copy(String), Rescan, Close }
/// Per-run facts the parent loads (cached) so the ledger does no I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowExtra {
    pub rail: Vec<NodeState>,        // one per flow node, in flow order; empty when unknown
    pub agents: String,              // e.g. "grok-build · grok-4.7 · low"; "" when no agent node
    pub stalled: bool,               // bundle::is_stalled
    pub question: Option<String>,    // what a waiting run is asking (the checkpoint summary)
}
pub struct LedgerState { /* private */ }
impl LedgerState {
    pub fn new() -> Self;
    pub fn set_runs(&mut self, runs: Vec<RunSummary>);            // keeps the selected run_id selected when still present
    pub fn set_extras(&mut self, extras: std::collections::HashMap<String, RowExtra>);   // key = run_id; a run without an entry draws without rail/agents
    pub fn selected_run(&self) -> Option<&RunSummary>;            // the selected row of the Flows tab; None on the Sessions tab or an empty list
    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>);
    pub fn tab(&self) -> LedgerTab;
    pub fn any_live(&self) -> bool;                                // some run is Running
    pub fn captures_text(&self) -> bool;                           // the filter input is open: every key belongs to it
    pub fn esc_closes(&self) -> bool;                              // no input open and the active tab's filter is empty
    pub fn handle_key(&mut self, key: &KeyEvent) -> LedgerOutcome;
    pub fn scroll(&mut self, delta: isize) -> bool;                // mouse wheel; true when the selection moved
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64);
    pub fn shortcuts(&self) -> Vec<&'static str>;                  // footer labels, e.g. "Enter open"
}

// ==== crate::views::acpx_modal::run  (file: src/views/acpx_modal/run.rs) ====
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    Unchanged, Changed, Back, OpenStep(usize), Replay(usize), Copy(String),   // usize = index into run.steps
    ScrollDetail(isize),                                 // the parent scrolls its DetailPanel by this many rows
    Gate { choice: String, flow_file: String },          // the user confirmed a gate choice
}
pub struct RunViewState { /* private */ }
impl RunViewState {
    pub fn new(run: Run) -> Self;                                  // selection starts on the waiting/current stage, else the last stage that ran
    pub fn set_settings(&mut self, nodes: std::collections::HashMap<String, AgentSettings>, steps: std::collections::HashMap<String, AgentSettings>);   // from bundle::node_settings / step_settings
    pub fn detail_target(&self) -> DetailTarget;                   // what the selected row is about (see slice prompt)
    pub fn detail_area(&self) -> Option<Rect>;                     // where the parent draws the DetailPanel, from the last render; None when the area is too narrow for two panes
    pub fn captures_keys(&self) -> bool;                           // the gate confirmation box is open
    pub fn run(&self) -> &Run;
    pub fn run_mut(&mut self) -> &mut Run;                         // for bundle::refresh
    pub fn sync(&mut self);                                        // call after the run changed: rebuild rows, keep selection (by attempt id) and folds
    pub fn selected_step(&self) -> Option<usize>;
    pub fn select_step(&mut self, index: usize);
    pub fn handle_key(&mut self, key: &KeyEvent) -> RunOutcome;
    pub fn scroll(&mut self, delta: isize) -> bool;
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64);
    pub fn shortcuts(&self) -> Vec<&'static str>;
}

// ==== crate::views::acpx_modal::inspector  (file: src/views/acpx_modal/inspector.rs) ====
#[derive(Debug, Clone, PartialEq)]
pub enum InspectorOutcome { Unchanged, Changed, Back, Prev, Next, Replay, Copy(String) }
pub struct InspectorState { /* private */ }
impl InspectorState {
    /// `position` is (1-based index of this step, total steps) for the header.
    pub fn new(run_dir: &Path, step: Step, node: Option<FlowNode>, position: (usize, usize)) -> Self;   // reads stdout/stderr artifacts now (bundle::read_artifact, 256 KiB cap); read errors become the tab's text
    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome;
    pub fn scroll(&mut self, delta: isize) -> bool;
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64);
    pub fn shortcuts(&self) -> Vec<&'static str>;
}
"""

LAUNCH_API = """
// ==== crate::acpx::launch  (file: src/acpx/launch.rs) ====
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub flow_file: std::path::PathBuf,
    pub input_json: Option<String>,        // passed as --input-json
    pub default_agent: Option<String>,     // --default-agent
    pub approve_all: bool,                 // --approve-all (a global option: goes before `flow`)
    pub cwd: Option<std::path::PathBuf>,   // child working directory
    pub acpx_bin: Option<String>,          // default "acpx"
}
/// The single JSON line acpx prints on stdout when the run ends or stops at a checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowRunResult { pub run_id: String, pub run_dir: std::path::PathBuf, pub status: RunStatus, pub waiting_on: Option<String>, pub status_detail: Option<String>, pub error: Option<String> /* acpx never sets it today; parse tolerantly */ }
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchState { Starting, Running { run_dir: std::path::PathBuf }, Finished(FlowRunResult), Failed(String), Cancelled }
pub struct Launch { /* private: the child, reader threads, the set of run dirs that existed before spawn */ }
impl Launch {
    pub fn start(spec: &LaunchSpec, runs_base: &std::path::Path) -> std::io::Result<Launch>;
    pub fn poll(&mut self) -> LaunchState;      // non-blocking; call about once a second
    pub fn run_dir(&self) -> Option<&std::path::Path>;
    pub fn cancel(&mut self);                   // kill the child; later polls return Cancelled
    pub fn stderr_tail(&self) -> String;        // last 20 lines of the child's stderr
}
pub fn argv(spec: &LaunchSpec) -> Vec<String>;                       // without the binary
pub fn parse_result(stdout: &str) -> Option<FlowRunResult>;
pub fn find_new_run(runs_base: &std::path::Path, before: &std::collections::HashSet<std::path::PathBuf>, flow_file: &std::path::Path) -> Option<std::path::PathBuf>;
/// Splits `/acpx run <file> [json]` arguments: first word (or quoted string) is the file, the rest is the input JSON.
pub fn parse_run_args(args: &str) -> Result<(std::path::PathBuf, Option<String>), String>;
"""

CARD_API = """
// ==== crate::scrollback::blocks::acpx_flow  (file: src/scrollback/blocks/acpx_flow.rs) ====
#[derive(Debug, Clone, PartialEq)]
pub enum AcpxFlowStatus {
    Starting,                                  // child spawned, run directory not seen yet
    Running,
    Waiting { node: String, detail: String },  // stopped at a checkpoint
    Completed,
    Failed(String),
    Cancelled,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AcpxFlowRow {
    pub node_id: String,
    pub label: String,              // agent name for acp nodes, else the kind label
    pub state: NodeState,
    pub duration_ms: Option<u64>,   // None until the node has a finished step
    pub result: String,             // one line: error, "→ route", "exit 0", or the output's first line; "" when none
}
#[derive(Debug, Clone)]
pub struct AcpxFlowBlock {
    pub flow_file: String,          // as the user typed it
    pub title: String,              // run title once known, else the flow file name
    pub run_id: Option<String>,
    pub status: AcpxFlowStatus,
    pub rows: Vec<AcpxFlowRow>,     // one per flow node, in flow order
    pub elapsed: std::time::Duration,
    pub note: Option<String>,       // e.g. the last stderr line of a failed start
}
impl AcpxFlowBlock {
    pub fn started(flow_file: impl Into<String>) -> Self;                  // Starting, no rows, title = file name of flow_file
    /// Refresh from a loaded run (called about once a second by the wiring). Does not touch a Cancelled status.
    pub fn update_from_run(&mut self, run: &crate::acpx::bundle::Run, now_ms: i64);
    pub fn set_status(&mut self, status: AcpxFlowStatus);
    pub fn is_running(&self) -> bool;                                      // Starting or Running
}
// plus `impl crate::scrollback::block::BlockContent for AcpxFlowBlock`
"""

FLOWS_API = """
// ==== crate::views::side_panel::flows  (file: src/views/side_panel/flows.rs) ====
/// Prefix of `Row::open` for a row that opens a run in the /acpx modal: "acpx-run:<absolute run dir>".
pub(crate) const OPEN_PREFIX: &str = "acpx-run:";
#[derive(Default)]
pub(super) struct Flows { /* private */ }
impl Flows {
    /// The 1 s tick while this page is showing. True when the rows changed.
    pub(super) fn poll(&mut self, now_ms: i64) -> bool;
    pub(super) fn rows(&self, c: &Ctx) -> Vec<Row>;
}
"""

FEED_API = """
// ==== crate::acp::acpx_feed  (file: src/acp/acpx_feed.rs) ====
/// `range` = inclusive (start_seq, end_seq) for a flow step; None = the whole file.
pub(crate) fn convert(events: &std::path::Path, range: Option<(u64, u64)>, session_id: &str) -> std::io::Result<Vec<Fed>>;
pub(crate) fn convert_text(text: &str, range: Option<(u64, u64)>, session_id: &str) -> Vec<Fed>;
"""

RULES = """
Hard rules: Rust 2024 edition, toolchain 1.94, ratatui 0.29, crossterm 0.28, serde / serde_json, chrono 0.4,
unicode-width 0.2. No new crates, no `unsafe`, no `unwrap()` / `expect()` / indexing that can panic on anything that
comes from disk, the user or the terminal size (a Rect may be 0x0). The file must compile at its output path inside
the existing crate `xai-grok-pager`; `crates/codegen/xai-grok-pager/src/a/b.rs` is `crate::a::b`. Import the contract
types by the `crate::` paths given; do not redefine them. Names in the "Public surface" block are fixed: the wiring
and the other files call exactly them. You may add private helpers and extra public methods.
Imports that exist: `crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers}`, `ratatui::buffer::Buffer`,
`ratatui::layout::Rect`, `ratatui::style::{Color, Modifier, Style}`, `ratatui::text::{Line, Span}`,
`unicode_width::UnicodeWidthStr`, `crate::theme::Theme` (a Copy struct; use only the fields shown in theme.rs.txt),
`crate::render::line_utils::truncate_str(s: &str, max_width: usize) -> String` (cuts to display width),
`crate::views::traces_modal::fmt_age(ago_ms: i64) -> String` ("now", "42s", "3m", "2h", "5d"),
`crossterm::event::{MouseEvent, MouseEventKind}`, `crate::views::modal_window::{self, ModalContentArea, ModalSizing,
ModalWindowConfig, ModalWindowOutcome, ModalWindowState, Shortcut}`.
Drawing: write straight into the `Buffer` (`buf.set_line(x, y, &line, width)`, `buf.set_stringn`, `buf.set_style`);
never draw outside `area`; every row is cut to the area width. Status is never shown by colour alone: a glyph or a
word always carries it (28 themes ship).
`truncate_str` appends `…` when it cuts. Edition 2024: `gen` is a reserved word and `std::env::set_var` is unsafe (do
not use it).
Tests: a `#[cfg(test)] mod tests` at the bottom, no terminal, no network, no snapshot files. Render tests draw into
`Buffer::empty(Rect::new(0, 0, w, h))` and assert on the row text. Tests get a theme from `Theme::default()`; never
write a `Theme` struct literal. Tests that need files use `tempfile::tempdir()` (a dev-dependency of this crate).
Return the complete file in ONE ```rust block and nothing else: no prose before or after.
"""

VIEW_STYLE = """
House style for these views (same as the traces modal): dense rows, one line each; the selected row gets
`theme.bg_highlight` across the full width and bold text; secondary columns in `theme.gray`; headers and separators in
`theme.gray_dim`; running `theme.accent_running`, ok `theme.accent_success`, failed `theme.accent_error`, waiting
`theme.warning`. Vim keys and arrows both work everywhere: j/k and Down/Up move, g/G or Home/End jump, PageUp/PageDown
and Ctrl+U/Ctrl+D move by half the last rendered height. Ignore key events whose `kind` is `KeyEventKind::Release`.
Keep the selection visible by adjusting a scroll offset during `render` (remember the last rendered height). Draw a
right-aligned "12/40" position counter when rows overflow. An empty list draws one dim, centred explanatory line.
"""

def view_ctx(*extra):
    return [f"{CTX}/theme.rs.txt", *extra]

TASKS = [
    {
        "custom_id": "bundle",
        "output_file": f"{SRC}/acpx/bundle.rs",
        "context_files": [f"{CTX}/bundle_sample.md"],
        "prompt": f"""
Write `bundle.rs`: the reader for acpx flow-run bundles on disk. `bundle_sample.md` shows a real bundle; shapes there
are exact, values are shortened. This file is plain data + I/O: no ratatui.

Public surface (implement exactly this; other files are being written against it right now):
{BUNDLE_API}
Details:
- Parse with `serde_json::Value` and tolerant accessors (missing or wrongly typed fields become None / defaults), not
  strict derive structs: acpx adds fields between versions. Unknown enum strings map to `Unknown`.
- `list_runs`: every sub-directory of `base` that has `projections/live.json` or `manifest.json`. live.json wins;
  manifest.json supplies runId/flowName/runTitle/startedAt/status when live.json is missing or does not parse.
- `load_run`: summary as above; `flow.json` -> FlowDef (an edge with a `switch` object is `Switch`; node `summary`
  only exists on checkpoints); `projections/steps.json` (a JSON array) -> steps; `trace.ndjson` -> events;
  `projections/run.json` -> input; `manifest.json` `sessions[]` -> SessionRef (events_path = dir joined with
  `eventsPath`); a session's agent_name comes from the `sessionBindings` value (in live.json, else run.json; NOT in
  the manifest) whose `bundleId` equals the session id.
  Only a missing/unreadable run directory is an `Err`; any single missing file yields empty data for that part.
- ndjson: skip blank lines and lines that do not parse; a final line without a trailing newline that does not parse
  is a write in progress and is skipped silently.
- `refresh`: re-read each part; a part whose file fails to parse keeps the old value; return whether the run differs.
- `TraceEvent.detail` by `kind`: node_started -> the payload nodeType; node_heartbeat -> payload.statusDetail;
  action_prepared -> the command line (command + args joined by spaces); action_completed -> "exit <code> · <dur>";
  artifact_written -> "<file name> · <bytes> B"; acp_prompt_prepared -> "prompt · <bytes> B"; acp_response_parsed ->
  "response · <bytes> B"; session_bound -> "session <handle>"; node_outcome -> "<outcome> · <dur>" plus " · <error>"
  when error is non-null; run_started/run_completed/run_failed -> flow name or error. Anything else -> "".
- Timestamps look like "2026-11-18T19:12:14.893Z".
Tests (at least 14): build a bundle directory in a tempdir from string literals modelled on the sample and cover:
waiting run (waiting_on, node_state Waiting, no current node); running run (currentNode -> Running); failed step;
switch edge parsing and BFS node order with an unreachable node; taken_path / edge_taken for the branch not taken;
progress; torn last ndjson line; refresh keeping old steps when steps.json is half-written and returning true when
live.json changes; list_runs ordering and manifest fallback; read_artifact rejecting `../x` and absolute paths and
capping bytes on a multi-byte char; parse_ts; fmt_duration at each unit; value_text for string / object / null.
{RULES}""",
    },
    {
        "custom_id": "sessions",
        "output_file": f"{SRC}/acpx/sessions.rs",
        "context_files": [f"{CTX}/bundle_sample.md"],
        "prompt": f"""
Write `sessions.rs`: summaries of plain acpx sessions (`~/.acpx/sessions/<record_id>.json`, last section of
`bundle_sample.md`). Plain data + I/O, no ratatui.

Public surface:
{SESSIONS_API}
Details:
- Only `*.json` files directly in `base`; skip files that do not parse or have no `acpx_record_id`.
- Fields are snake_case here, but accept the camelCase spelling of each as a fallback (the same schema appears in
  camelCase inside flow bundles).
- `tokens`: `cumulative_token_usage` may be a number or an object; for an object use its `total` (or `total_tokens`),
  else the sum of its numeric values. `cost_usd`: `cumulative_cost` as a number, or an object's `amount` field (real shape:
  `{{"amount": 0.29, "currency": "USD"}}`; also accept `usd` / `total`).
- `messages` = length of the `messages` array.
- `agent_label("env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/x/claude /Users/me/.local/bin/claude-agent-acp")` is
  "claude": drop a leading `env` with its `-u NAME` pairs and `NAME=value` words, take the file name of the first
  remaining word, strip a trailing "-agent-acp", "-code-acp" or "-acp". Empty input -> "?".
- Timestamps are RFC 3339 (chrono); unparseable -> 0.
Tests (at least 8), using a temp directory: ordering by last use; title fallbacks; both casings; token object vs
number; cost shapes; agent_label on the example above, on "codex-acp --foo", on "/opt/x/grok-pi-acp" and on "";
a non-json file and a broken json file being skipped; missing base directory -> empty.
{RULES}""",
    },
    {
        "custom_id": "graph",
        "output_file": f"{SRC}/acpx/graph.rs",
        "context_files": [],
        "prompt": f"""
Write `graph.rs`: turn a run into Mermaid flowchart source. The pager's markdown renderer draws ```mermaid fences as
box-drawing text; it supports `flowchart LR|TD`, node shapes, `-->`, `==>`, `-.->` and `|label|` on edges, and
IGNORES classDef / style / linkStyle, so state is carried by the label glyph and the arrow type only. Pure, no I/O.

Types you import (`use crate::acpx::bundle::{{FlowEdge, NodeKind, NodeState, Run}};`):
{BUNDLE_API}
Public surface:
{GRAPH_API}
Rules for the output:
- First line `flowchart LR` or `flowchart TD`.
- Node ids in the source are `n0`, `n1`, ... in `run.flow.nodes` order (real ids may contain any character).
- Label = `<NodeState::glyph()> <node id>`, in double quotes, with `"` replaced by `'` and `[`, `]`, `{{`, `}}`, `|`,
  `<`, `>` and backticks replaced by spaces. Shape by kind: Acp `n0("…")` (rounded), Action and Compute `n0["…"]`,
  Checkpoint `n0{{{{"…"}}}}` (drawn as a diamond), Unknown `n0["…"]`.
- One line per edge. Simple: arrow only. Switch: one arrow per case, labelled with the case value (same escaping), in double
  quotes because `;` and `%%` outside quotes end a statement.
- Arrow: `==>` when `run.edge_taken(from, to)`; `-.->` when `from` has a step but this edge was not taken;
  `-->` when `from` has not run yet. With a label: `==>|"small"|`, `-.->|"large"|`, `-->|"x"|`.
- Edges whose endpoints are not in `flow.nodes` are skipped. Deterministic output for identical input.
Tests (at least 9): build `Run` values by hand (write a small private test helper that fills every field) and assert
exact lines: linear flow all done; a switch with one case taken; a node currently Running; a waiting checkpoint;
nothing run yet (all `-->`); label escaping of every special character; TopDown header; an edge to an unknown node
skipped; a node that ran twice (loop) still yields one node line.
{RULES}""",
    },
    {
        "custom_id": "feed",
        "output_file": f"{SRC}/acp/acpx_feed.rs",
        "context_files": [f"{CTX}/claude_feed.rs.txt", f"{CTX}/bundle_sample.md"],
        "prompt": f"""
Write `acpx_feed.rs`: turn an acpx session event log into the events the pager's session handler consumes, so a flow
step's conversation (or a whole acpx session) replays as a normal transcript. `claude_feed.rs.txt` shows the sibling
converters this must match: use `super::claude_feed::{{Event, Fed, chunk}}` and build `meta` the same way
(`isReplay: true`, `eventId: "<session_id>-<n>"`, `promptId` when known, `agentTimestampMs` when the line has a time).
`use agent_client_protocol as acp;`.

Public surface:
{FEED_API}
Two line formats, detected per line:
1. Flow bundle `events.ndjson`: `{{"seq":N,"at":"RFC3339","direction":"inbound"|"outbound","message":{{JSON-RPC}}}}`
   (see `bundle_sample.md`). `range` filters on `seq`.
2. Plain session `<id>.stream.ndjson`: the bare JSON-RPC object per line, no seq or time; the first line is a header
   `{{"schema":"acpx.session.journal.v1","type":"segment",...}}` to skip. Here the 1-based line number stands in for
   seq when a range is given.
Per JSON-RPC message:
- method `session/prompt` (the client's request): for each `params.prompt[]` block with `type == "text"`, emit
  `Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(text)))`; the request `id` rendered as a string becomes the
  current `promptId` for the events that follow.
- method `session/update`: `serde_json::from_value::<acp::SessionUpdate>(params.update)`; on `Err` skip the line.
  Forward only these variants: AgentMessageChunk, AgentThoughtChunk, ToolCall, ToolCallUpdate, Plan. Drop everything
  else (available commands, usage, session info, mode changes): a replay must not change the live view's state.
- Skip everything else: `initialize`, `session/new`, `session/resume`, results and errors, and any method that
  starts with `_` (`_auth/status_update` carries account details and must never be forwarded).
- Blank or unparseable lines are skipped. `convert` = read the file, call `convert_text`.
Tests (at least 9) on `convert_text` with string literals modelled on the sample: prompt -> one user chunk with the
text; agent chunks in order; tool_call then tool_call_update forwarded; usage_update / available_commands_update /
session_info_update dropped; `_auth/status_update` dropped; range keeps only seqs inside it (inclusive both ends);
bare-format input with the journal header; meta has isReplay true, increasing unique eventId values and
agentTimestampMs for the wrapped format; an unparseable line in the middle is skipped. `Fed` and `Event` do not
implement Debug or PartialEq: assert with `matches!` and by reading fields.
{RULES}""",
    },
    {
        "custom_id": "launch",
        "output_file": f"{SRC}/acpx/launch.rs",
        "context_files": [f"{CTX}/bundle_sample.md"],
        "prompt": f"""
Write `launch.rs`: start an acpx flow run as a child process, find the run directory it creates, report its state,
and cancel it. Plain std (std::process, std::thread, std::sync::mpsc); no tokio, no ratatui.
`use crate::acpx::bundle::RunStatus;` (variants: Running, Waiting, Completed, Failed, TimedOut, Unknown).

Public surface:
{LAUNCH_API}
Facts about acpx 0.22 (measured):
- Command line: `acpx [--approve-all] --format json flow run <file> [--input-json <json>] [--default-agent <name>]`.
- stdout stays empty until the run ends or stops at a checkpoint, then carries ONE line:
  `{{"action":"flow_run_result","runId":"…","flowName":"…","status":"waiting","statusDetail":"…","waitingOn":"review","runDir":"/Users/x/.acpx/flows/runs/<runId>","outputs":{{…}},"sessionBindings":{{…}}}}`
  This line is printed ONLY for completed and waiting runs. A failed or timed-out run prints no result line and
  exits non-zero. stderr carries progress lines such as `[acpx] tokens: input=2 output=37 …`.
- The run directory appears under `runs_base` shortly after start; its `manifest.json` has `flowPath` = the flow file
  made absolute against the child's working directory (symlinks not resolved). The run dir cannot be read from
  stdout while the run is live: it is found by listing `runs_base`. `start` therefore joins a relative `flow_file`
  onto `spec.cwd` (else the current directory) before building `argv` and before `find_new_run`.
Behaviour:
- `start`: record the set of directories currently in `runs_base` (missing base = empty set), spawn with stdin null
  and stdout/stderr piped, and drain both pipes on two threads into channels so the child never blocks on a full
  pipe. A spawn error is returned as the `Err`.
- `poll`: drain the channels; if the run dir is not known yet, `find_new_run`; `try_wait` the child. After the child has
  exited, keep receiving from both channels until they disconnect (bounded at 500 ms) BEFORE deciding, so a result
  line still in the pipe is not missed. Exited with a parsable result line -> `Finished`. Exited without one ->
  `Failed(msg)` where msg is the `error` string of `<run_dir>/projections/live.json` when the dir is known and has
  one, else `acpx exited with <status>: <last stderr line>`.
  Still alive -> `Running {{ run_dir }}` once the dir is known, else `Starting`. After `cancel` -> `Cancelled`.
  Once a terminal state is reached every later poll returns the same value.
- `find_new_run`: directories in `runs_base` not in `before` whose manifest `flowPath` equals `flow_file` after
  canonicalising both (fall back to comparing as given when canonicalising fails); the newest by name when several
  match (run ids start with a timestamp). A manifest that is missing or half-written is not a match yet.
- `parse_result`: the last line of `stdout` that parses as JSON with `"action":"flow_run_result"`; other lines ignored.
- `parse_run_args`: `my.flow.ts` -> (path, None); `my.flow.ts {{"a":1}}` -> input kept verbatim; a file in double or
  single quotes may contain spaces; empty args -> Err("usage: /acpx run <flow file> [input json]"); input that is
  not valid JSON -> Err naming the problem.
- Dropping a `Launch` does NOT kill the child (a run outlives the view); only `cancel` does.
Tests (at least 12): argv for every option combination incl. `--approve-all` placed before `flow`; parse_result on
the sample line, on output with noise lines around it, on a line without the optional fields, and on garbage; find_new_run
with a temp runs dir (ignores dirs in `before`, ignores other flows, picks the newest, tolerates a missing
manifest); parse_run_args cases above; and process tests that use `acpx_bin: Some("/bin/sh")`-style stand-ins only
through a private `start_command(std::process::Command, …)` helper that `start` also uses: a child that prints a
result line and exits -> Finished; a child that exits 3 with stderr text -> Failed containing that text; a child
that sleeps -> Starting, then cancel -> Cancelled, and the same on the next poll. Guard the process tests with
`#[cfg(unix)]` and poll in a loop with a 5 second deadline, never a fixed sleep.
{RULES}""",
    },
    {
        "custom_id": "ledger",
        "output_file": f"{SRC}/views/acpx_modal/ledger.rs",
        "context_files": view_ctx(),
        "prompt": f"""
Write `ledger.rs`: the first screen of the `/acpx` modal, a two-tab list of flow runs and plain acpx sessions.
It draws inside a content Rect the modal chrome already framed; the parent draws the title, the footer, and (to the
right of this list, in its own Rect) a preview of the selected run. So `area` is often only 50-80 columns wide on a
200-column terminal: the list must read well at 50 columns and use the room when it has 120.

Types you import (`use crate::acpx::bundle::{{NodeState, Outcome, RunStatus, RunSummary, fmt_duration}};`
`use crate::acpx::sessions::SessionSummary;`):
{BUNDLE_API}
{SESSIONS_API}
Public surface:
{VIEW_API.split('// ==== crate::views::acpx_modal::run')[0]}
Layout:
- Row 0: the tabs, ` Flows 12 │ Sessions 40 `, active tab bold + `theme.accent_assistant`, the other `theme.gray`;
  right-aligned on the same row a summary such as `1 needs you · 2 running` and, when filtering, `/text`  `active`.
- Flows tab, default order `grouped`: three sections, each introduced by a header row that is NOT selectable
  (movement skips it): `NEEDS YOU 2` in `theme.warning` (status Waiting), `RUNNING 1` in `theme.accent_running`
  (Running and not stalled), `FINISHED 9` in `theme.gray_dim` (everything else, stalled runs included). Header text is
  bold and followed by a `─` rule to the right edge in `theme.gray_dim`. Empty sections are omitted. Inside a section:
  most recently updated first. A blank row separates sections when the list is shorter than the area.
- A flow row is ONE line: status glyph (coloured; a stalled run shows `⊘` in `theme.gray`), title (flexible width,
  cut with `truncate_str`), the rail, state text, agents, age of last update (`fmt_age(now_ms - updated_ms)`),
  duration. The rail is `RowExtra.rail` drawn as its glyphs with no spaces, each coloured by state (Running
  accent_running, Waiting warning, Done(Ok) accent_success, Done(Failed|TimedOut) accent_error, others gray_dim),
  followed by ` <done>/<total>` in `theme.gray` where done = entries that are `Done(_)`; nothing when the rail is
  empty. State text: Waiting -> the `question` when Some, else `waiting: <waiting_on>`; stalled -> `stalled at
  <current_node>`; Running -> `<current_node>` plus status_detail when it fits; Failed/TimedOut -> the error cut to
  fit; Completed -> `done`. Drop columns as the width shrinks: agents below 120 columns, duration below 96, the
  `<done>/<total>` count below 80, age below 64, the rail below 48. Title and state text share what is left, title
  first (at least 16 columns), state text cut to fit.
- Sessions tab (unchanged idea): a column header row in `theme.gray_dim`, then rows: `●` open / `○` closed, title,
  agent, last path component of cwd, message count, tokens (`12.3k`, `1.2M`), cost (`$0.12`, blank when None), age of
  last use; drop columns from the right as the width shrinks.
Keys: movement per the house style; Tab / BackTab / Left / Right / h / l switch tab; `/` opens an inline filter
(typed characters and Backspace edit it, Enter keeps it, Esc clears it and closes the input; matching is
case-insensitive substring over title, flow name, status label, run id, agents — or title, agent, cwd, name for
sessions); `a` toggles active-only (Running or Waiting; for sessions: not closed); `s` cycles the order (grouped,
recent, name — the flat orders have no section headers; show the active one right-aligned on the tab row as
`[grouped]`); Enter -> OpenRun(dir) / OpenSession(row); `y` -> Copy(run dir as a string, or the session record id);
`R` -> Rescan; Esc -> clears a non-empty filter first (Changed), otherwise Close. While the filter input is open
every printable key goes to it.
Each tab keeps its own selection, scroll, and filter. `set_runs` / `set_sessions` / `set_extras` re-apply filter and
order and keep the selected run_id / session selected when it is still present.
{VIEW_STYLE}
Tests (at least 14): navigation skips section headers and clamps; grouped order puts a Waiting run above a newer
Completed one; a stalled Running run is listed under FINISHED with `stalled at`; tab switching keeps per-tab
selection; filter typing, Enter, Esc ordering (clear, then Close); active-only; order cycling; Enter outcomes on both
tabs; `y`; `R`; set_runs keeps the selected run_id; selected_run on both tabs; any_live; render at 120x14 shows the
tab row, the `NEEDS YOU` header, a waiting row containing its question text, its rail glyphs and `4/6`; render at
50x8 keeps the title and does not panic; render at 0x0 does not panic; empty state line.
{RULES}""",
    },
    {
        "custom_id": "run_view",
        "output_file": f"{SRC}/views/acpx_modal/run.rs",
        "context_files": view_ctx(f"{CTX}/traces_fold.rs.txt", f"{CTX}/markdown_content.rs.txt"),
        "prompt": f"""
Write `run.rs`: the screen for one flow run. Full width on top: header, banner, flow graph. Below, two panes: on the
left the STAGES of the run, each stage a delineated group with every substep visible; on the right a detail panel
that the PARENT draws (you only reserve its Rect and say what it should show).

Types you import (`use crate::acpx::bundle::{{self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step,
TraceEvent, fmt_duration, value_text}};` `use crate::acpx::graph::{{Direction, mermaid_source}};`
`use crate::views::acpx_modal::detail::DetailTarget;`
`use crate::views::traces_fold::{{FoldNode, FoldState, FoldTree}};` (shown in traces_fold.rs.txt; reuse it, do not
write another fold model) `use crate::scrollback::blocks::markdown_content::MarkdownContent;`):
{BUNDLE_API}
{BUNDLE_EXTRA_API}
{GRAPH_API}
{DETAIL_API}
Public surface:
{'// ==== crate::views::acpx_modal::run' + VIEW_API.split('// ==== crate::views::acpx_modal::run')[1].split('// ==== crate::views::acpx_modal::inspector')[0]}
Layout, top to bottom inside `area`:
1. Header (1 row): run status glyph + title (bold), then dim ` · `-joined facts: flow name (omit it when the title
   already starts with it), `<done>/<total> nodes`, duration (`run.duration_ms(now_ms)`), age when not live.
2. Banner (1 row, only when needed): Waiting -> `◐ NEEDS YOU at <node>: <status_detail>` in `theme.warning`;
   Failed/TimedOut -> `✗ <error>` in `theme.accent_error`; Running -> `● <current_node> · <status_detail>` in
   `theme.accent_running`; `bundle::is_stalled(&run.summary, now_ms)` -> `⊘ stalled at <current_node>: the acpx
   process is gone` in `theme.gray` instead of the Running banner.
3. Graph (toggle with `v`, on by default), full width: render
   `MarkdownContent::new_with_table_width(format!("```mermaid\n{{src}}\n```"), Some(width)).output(width)` and
   paint each `BlockLine.content` (a `Line<'static>`) with `buf.set_line`. When a diagram does not fit, that renderer
   returns a boxed SOURCE LISTING instead of a drawing: detect it by any line whose text contains
   `mermaid: flowchart` or `too wide to display`. Use `Direction::LeftRight`; if that is a source listing, or has
   more than `area.height / 3` lines, or any line wider than the area, use `Direction::TopDown`; if TopDown is a
   source listing show one dim row `graph does not fit at this width (v hides this line)`; if it is only too tall,
   show the first rows that fit and a dim `… graph cut (v hides it)` last row. Cache the rendered lines keyed on
   (width, mermaid source). Hidden automatically when the area has fewer than 16 rows.
4. Separator row in `theme.gray_dim`.
5. The rest is split when `area.width >= 110`: stages pane on the left, width `(area.width * 2 / 5).clamp(46, 76)`,
   then one column holding a `│` rule in `theme.gray_dim`, then the detail Rect (everything to the right), which you
   leave untouched and return from `detail_area()`. Below 110 columns the stages pane takes the full width and
   `detail_area()` is None.
Stages pane: a `FoldTree<String>`. Roots, in this order:
   a. one root per step in `run.steps` order (key = attempt id);
   b. a root for a node that is Running or Waiting but has no step record yet (key = `"live/<node id>"`);
   c. when any flow node has `NodeState::NotRun`: a root `"skipped"` drawn as a dim rule row `── not run ──` whose
      children are those nodes, one per row (key = `"skip/<node id>"`), `○ <node id>  <kind label>` in gray_dim.
   Stage header row: `▾|▸ <n> <glyph> <node id>` bold, then in `theme.gray` the agent label
   (`AgentSettings::label()` from the step settings by attempt id, else the node settings by node id, else the kind
   label; a shell action shows `shell`), then right-aligned the duration (`fmt_duration`; for a Running stage the
   time since its `node_started` event). `<n>` is the 1-based position among a+b. The glyph is coloured by state.
   Children of a stage (indented two columns, a `│` guide in gray_dim, `╰` on the last child):
   - its trace events from `run.step_events`, as `+<ms since stage start>  <kind>  <detail>` in `theme.gray`;
     `node_heartbeat` events are hidden unless `n` (noise) is on (key = `"<attempt>/<seq>"`);
   - a result row (key = `"<attempt>/result"`): the error in accent_error for failures; `→ <route>` when the output
     object has a string `route`; `exit <code>` for shell actions; else the first line of the output, where a JSON
     string is shown as is and objects/arrays as compact one-line JSON (`serde_json::to_string`); cut to fit;
   - for the Waiting stage, the GATE: one row per `bundle::gate_choices(run)` entry, `[<i>] <label> → <flow file>`
     in `theme.warning` (key = `"gate/<i>"`, i from 1); when there are no choices one dim row `no next step declared:
     re-run the flow to continue`.
   Stages are EXPANDED by default (`FoldState` stores collapsed keys, so do nothing special). A blank row separates
   one stage from the next when the pane is taller than the content; never inside a stage.
`detail_target()`: selected row is a step header or one of its children -> `Step(index)`; a `live/…` root or its
children (gate rows included) -> `Node(node id)`; a `skip/…` row -> `Node(node id)`; the `skipped` root or nothing
selected -> `Run`.
Keys (ignored while the confirmation box is open, see below): movement per the house style over visible rows;
Left/h collapse (on a child or collapsed root, jump to the parent, as `FoldState::collapse` reports); Right/l expand;
Space toggles; `E` toggles all; `n` noise; `v` graph; `!` jumps to the next failed step (wrapping); `c` jumps to the
current/waiting stage; `[` / `]` previous/next stage header; `J` / `K` -> ScrollDetail(3) / ScrollDetail(-3) and
`D` / `U` -> ScrollDetail(15) / ScrollDetail(-15); Enter on a gate row opens the confirmation box for that choice,
Enter elsewhere -> OpenStep(step index) (on a child row: its stage; Unchanged for rows with no step); digits `1`-`9`
while the run is Waiting open the confirmation box for that gate choice from anywhere; `F5` (`KeyCode::F(5)`) jumps
the selection to the first gate row; `r` -> Replay(step index) only
for a step with `conversation` Some, otherwise Unchanged; `y` -> Copy(value_text of the selected stage's output);
Esc -> Back.
Confirmation box: a small centred box drawn over the stages pane, `Start <flow file>?` bold, a dim line `choice
"<label>" · runs with approve-all`, and `Enter start · Esc cancel`. While it is open `captures_keys()` is true and
only Enter (-> `Gate {{ choice, flow_file }}` and close the box) and Esc (close, Changed) do anything.
`sync()` rebuilds the tree after `bundle::refresh` changed the run and keeps selection by key, falling back to the
nearest earlier root; fold state is kept (`FoldState::retain`).
`shortcuts()`: `Enter step`, `r replay`, `1-9 gate` (only when the run is waiting and has choices), `Space fold`,
`J/K detail`, `v graph`, `n noise`, `y copy`, `Esc back`, `? keys`.
{VIEW_STYLE}
Tests (at least 16), building `Run` values by hand with a private helper: initial selection on the waiting stage;
stages start expanded and show events but hide heartbeats until `n`; collapse on a child jumps to the parent; `E`;
`!` wraps; `c`; `[` `]`; Enter and `r` outcomes including `r` on a non-acp step; `y`; Esc; `J` and `U` outcomes;
detail_target for a step child, a live root and a skipped node; a waiting run whose checkpoint output is
`{{"next":{{"approve":"land.flow.ts","reject":"fix.flow.ts"}}}}` shows two gate rows, `2` opens the box
(`captures_keys()`), Esc closes it, `2` then Enter yields `Gate {{ choice: "reject", flow_file: "fix.flow.ts" }}`;
a waiting run without `next` shows the `no next step declared` row; sync keeps selection when a step is appended;
render at 160x40 contains the title, the NEEDS YOU banner, a stage header with its glyph, the `│` pane rule, and
`detail_area()` is Some with x greater than 46 and nothing drawn inside it; render at 100x30 has `detail_area()`
None; the skipped section lists a NotRun node; render with `v` off has no graph rows; render at 40x8 and 0x0 does
not panic.
{RULES}""",
    },
    {
        "custom_id": "inspector",
        "output_file": f"{SRC}/views/acpx_modal/inspector.rs",
        "context_files": view_ctx(),
        "prompt": f"""
Write `inspector.rs`: the screen for one step of a flow run: everything acpx recorded about it, in tabs.

Types you import (`use crate::acpx::bundle::{{self, ArtifactRef, FlowNode, NodeKind, Outcome, Step, fmt_duration,
value_text}};`):
{BUNDLE_API}
Public surface:
{'// ==== crate::views::acpx_modal::inspector' + VIEW_API.split('// ==== crate::views::acpx_modal::inspector')[1]}
Layout:
- Row 0: `<outcome glyph> <node id>` bold, then dim ` · ` facts: kind label, agent name, attempt id, duration, and
  right-aligned `step 3/5`.
- Row 1: tabs, only those with content, in this order: Output (value_text of `output`; the output artifact when
  `output` is Null), Prompt (`prompt_text`), Response (`raw_text`), Stdout, Stderr (artifact text; a 0-byte artifact
  still gets its tab with a dim `(empty)`), Error (`error`), Meta. The active tab is bold + `theme.accent_assistant`
  and prefixed with its number (`1 Output  2 Prompt …`). Opens on Error when the step failed, else Output.
- Meta tab: aligned `key  value` rows: node id, kind, checkpoint summary (from `node`), outcome, started / finished
  (as `HH:MM:SS.mmm` UTC), duration, agent, action command line, cwd, exit code, conversation session id and seq
  range, artifact paths with byte sizes.
- Rest: the tab body with a dim line-number gutter; long lines wrap at the area width when wrap is on (default),
  otherwise are cut with `…` and Left/Right (or h/l) scroll horizontally by 8 columns. Wrap by display width, never
  splitting a char. Bottom-right overlay `12-40/312` when the body overflows.
Keys: Tab / BackTab cycle tabs; digits 1-7 pick a tab by its shown number; j/k/arrows scroll one line, PageUp/Down
and Ctrl+U/Ctrl+D half a page, g/G or Home/End; `w` toggles wrap; `y` -> Copy(the active tab's full text);
`r` -> Replay only when the step has `conversation`, else Unchanged; `[` -> Prev, `]` -> Next (the parent swaps the
step); Esc -> Back. Each tab remembers its own scroll.
{VIEW_STYLE}
Tests (at least 12), with artifacts written into a temp directory: tab set for a shell action (Output, Response,
Stdout, Stderr, Meta); empty stderr shows `(empty)`; tab set for an acp step; failed step opens on Error; digit and
Tab selection; per-tab scroll memory; scroll clamps at both ends; wrap on/off changes the line count of a long line;
wrapping a line of wide (CJK) chars never exceeds the width; `y` returns the active tab text; `r` on acp vs
non-acp; `[` `]`; a missing artifact file shows the error text in its tab; render at 100x20 has the header and the
tab row; render at 0x0 does not panic.
{RULES}""",
    },
    {
        "custom_id": "modal",
        "output_file": f"{SRC}/views/acpx_modal.rs",
        "context_files": view_ctx(f"{CTX}/modal_window.rs.txt", f"{CTX}/modal_window_state.rs.txt", f"{CTX}/traces_modal_render.rs.txt"),
        "prompt": f"""
Write `acpx_modal.rs`: the `/acpx` modal's top level. It owns the three screens, moves between them, loads and polls
the data, and draws the shared chrome. `traces_modal_render.rs.txt` shows how the sibling traces modal uses the
chrome in `modal_window.rs.txt`; do it the same way (full-width sizing, `tabs: None`, `fold_info: None`).

This file starts with:
    pub(crate) mod inspector;
    pub(crate) mod ledger;
    pub(crate) mod run;
Those three files are being written separately against this surface (`use` them as `self::ledger::...` etc.):
{VIEW_API}
Data layer (`use crate::acpx::bundle::{{self, Run}};` `use crate::acpx::sessions::{{self, SessionSummary}};`):
{BUNDLE_API}
{SESSIONS_API}
Public surface (the wiring calls exactly this):
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Mode {{ Ledger, Run, Step }}
    /// A conversation to replay as a transcript; the wiring feeds it to `crate::acp::acpx_feed::convert`.
    #[derive(Debug, Clone, PartialEq)]
    pub struct ReplayRequest {{ pub events_path: PathBuf, pub range: Option<(u64, u64)>, pub session_id: String, pub title: String }}
    #[derive(Debug, Clone, PartialEq)]
    pub enum AcpxOutcome {{ Unchanged, Changed, Close, Copy(String), Replay(ReplayRequest) }}
    pub struct AcpxModalState {{ pub window: ModalWindowState, /* rest private */ }}
    impl AcpxModalState {{
        pub fn open() -> Self;                                   // bundle::runs_base() + sessions::sessions_base(), then open_at
        pub fn open_at(runs_base: Option<PathBuf>, sessions_base: Option<PathBuf>) -> Self;   // scans both now
        pub fn open_run(&mut self, dir: &Path) -> bool;           // jump straight to a run (used by `/acpx <run id>`); false + notice when it cannot load
        pub fn mode(&self) -> Mode;
        pub fn wants_poll(&self) -> bool;                         // Ledger: any run live; Run/Step: the open run is Running
        pub fn poll(&mut self) -> bool;                           // called about once a second while wants_poll(); true when a redraw is needed
        pub fn wants_esc(&self) -> bool;                          // true unless Esc would close the modal (the host has its own Esc handling)
        pub fn handle_key(&mut self, key: &KeyEvent) -> AcpxOutcome;
        pub fn handle_mouse(&mut self, mouse: &MouseEvent) -> AcpxOutcome;
        pub fn notice(&self) -> Option<&str>;
    }}
    pub fn render_acpx_modal(buf: &mut Buffer, full_area: Rect, state: &mut AcpxModalState, compact: bool);
Behaviour:
- Title: `acpx` in Ledger; `acpx › <run title>` in Run; `acpx › <run title> › <node id>` in Step (cut to fit).
- Footer shortcuts: the active screen's `shortcuts()` turned into `Shortcut {{ label, clickable: false, id: 0 }}`,
  followed by `? keys`.
- `wants_esc()` = help open, or mode is not Ledger, or `!ledger.esc_closes()`.
- `?` (except in Ledger while `captures_text()`) toggles a key reference drawn over the content (a plain list of every key for the active screen, taken from a
  static table in this file); any key closes it.
- Ledger outcomes: OpenRun(dir) -> `bundle::load_run`, on Ok switch to Run, on Err stay and set the notice
  (`cannot open run: <err>`); OpenSession(s) -> `AcpxOutcome::Replay` with `events_path: s.stream_path`,
  `range: None`, `session_id: s.acp_session_id`, title `acpx · <title>`; Copy -> Copy; Rescan -> rescan both lists;
  Close -> Close.
- Run outcomes: Back -> Ledger (rescan runs so the row is fresh); OpenStep(i) -> Step with
  `InspectorState::new(run dir, step i, the FlowNode with that id, (i + 1, steps.len()))`; Replay(i) -> build the
  ReplayRequest from the step's `conversation` and `run.session_events_path(session_id)`, title
  `<run title> › <node id>`; when the path is unknown set the notice `no event log for this step` and return Changed.
- Step outcomes: Back -> Run with that step selected (`select_step`); Prev / Next -> swap to the adjacent step,
  clamped, and keep the run view's selection in step; Replay -> as above for the open step; Copy -> Copy.
- `poll`: Ledger -> re-list runs (sessions only on Rescan) and return whether the list changed; Run / Step ->
  `bundle::refresh(run_mut())`, then `sync()`; return true when it changed.
- The notice is drawn on the last content row in `theme.warning` and cleared by the next key press.
- Mouse: pass to `modal_window::handle_modal_mouse(&mut self.window, mouse.kind, mouse.column, mouse.row)`;
  CloseRequested -> Close, Handled -> Changed; when Unhandled, ScrollUp / ScrollDown call the active screen's
  `scroll(-3)` / `scroll(3)`.
- Keys go to the active screen first; chrome is only asked (`modal_window::handle_modal_key`) for nothing: Esc is
  decided by the screens (Ledger returns Close, the others Back).
- `render_acpx_modal`: as the traces modal does; return early when the content is under 4 rows or 20 columns;
  `now_ms` from `SystemTime`.
Tests (at least 10), with a bundle written into a temp directory by a private helper (live.json, flow.json,
steps.json, trace.ndjson, manifest.json with one session) and a sessions directory with one record: open_at lists
both; Enter opens the run (mode Run), Enter again the step (mode Step), Esc twice returns to Ledger, Esc again
returns Close; wants_esc per mode; `r` on an acp step returns Replay with the right path and range; opening a
session returns Replay with range None; `]` in Step moves to the next step and Esc lands on it in Run; open_run on
a missing dir returns false and sets the notice, and the next key clears it; poll returns true after live.json's
status changes on disk; wants_poll false for a waiting run; `?` help toggles; render at 120x30 draws the title
`acpx`; render at 10x3 and 0x0 does not panic.
{RULES}""",
    },
    {
        "custom_id": "card",
        "output_file": f"{SRC}/scrollback/blocks/acpx_flow.rs",
        "context_files": [f"{CTX}/workflow_block.rs.txt"],
        "prompt": f"""
Write `acpx_flow.rs`: the transcript card for an acpx flow run started from the pager with `/acpx run <file>`. It is
a sibling of `WorkflowBlock` (shown whole in workflow_block.rs.txt, followed by the `BlockContent` trait): same
imports, same way of building `BlockOutput` from `Line`s (`Line::from(spans).into()`), same `Theme::current()`
helpers (`theme.primary()`, `theme.muted()`, `theme.dim()` return a `Style`; colours such as `theme.accent_success`,
`theme.accent_error`, `theme.accent_running`, `theme.warning` are `Color` fields), same accent/fold methods. Unlike
it, this card has several lines.

Types you import (`use crate::acpx::bundle::{{NodeKind, NodeState, Outcome, Run, RunStatus, fmt_duration,
value_text}};`):
{BUNDLE_API}
Public surface:
{CARD_API}
Output lines:
1. Header: `acpx ` bold, then `<title>`, then by status: Starting `starting…`; Running `running · <done>/<total> ·
   <elapsed>`; Waiting `waiting at <node> · <elapsed>`; Completed `done in <elapsed>`; Failed `failed after
   <elapsed>`; Cancelled `◌ cancelled after <elapsed>` (dim, like the workflow card). Elapsed via
   `crate::util::format_duration(self.elapsed)`.
2. One line per row: two spaces, the `NodeState::glyph()` coloured by state (Running accent_running, Waiting warning,
   Done(Ok) accent_success, Done(Failed|TimedOut) accent_error, others muted), the node id padded to the longest id,
   the label (muted), the duration (`fmt_duration`, muted, blank when None), the result (muted; error results in
   accent_error). No width is known here: do not truncate, the scrollback wraps.
3. Waiting: a line `  ◐ <detail>` in warning, then a dim line `  acpx cannot resume a run; re-run the flow to continue`.
   Failed: a line `  ✗ <message>` in accent_error. `note`, when Some, as a last dim line.
4. A last dim line `  /acpx <run id> opens this run` once `run_id` is known.
In `DisplayMode` terms: when the fold mode in `ctx` is the collapsed one the workflow card uses for its single line,
show only the header; follow the workflow card for `is_foldable`, `default_display_mode`, accent colour by status
(running animated like the workflow card's running state, failed error colour, otherwise as the workflow card does).
`update_from_run`: title = run.summary.title; run_id; rows from `run.flow.nodes` in order with
`run.node_state(id)`, the node's LAST step for duration/result/agent (result rules: step.error; else output object
with a string `route` -> `→ <route>`; else action exit code -> `exit <code>`; else first line of
`value_text(&step.output)`; objects and arrays as compact JSON); status from `run.summary.status` (Running ->
Running, Waiting -> Waiting {{ node: waiting_on, detail: status_detail }}, Completed -> Completed, Failed/TimedOut ->
Failed(error or "failed"), Unknown -> unchanged); elapsed = `run.duration_ms(now_ms)`.
Tests (at least 10), building `Run` by hand with a private helper and reading the text of
`output(&BlockContext::default())` lines (if `BlockContext` has no `Default` in the excerpt, build it the way the
workflow card's own tests do; if those are not shown, test `update_from_run`, `set_status`, `is_running` and a
private `fn lines(&self, selected: bool, collapsed: bool) -> Vec<Line<'static>>` that `output` calls, and assert on
that instead): started state; rows in flow order with glyphs; waiting lines; failed line; completed header;
cancelled survives update_from_run; route result; exit-code result; untaken branch row is `○` with no duration;
run id hint line; collapsed shows one line.
{RULES}""",
    },
    {
        "custom_id": "flows_page",
        "output_file": f"{SRC}/views/side_panel/flows.rs",
        "context_files": [f"{CTX}/side_panel.rs.txt"],
        "prompt": f"""
Write `flows.rs`: the "Flows" page of the pager's F3 side panel: the step rail of the newest acpx flow run, then the
recent runs. side_panel.rs.txt shows the panel's `Row`, `Ctx`, `note_rows`, and the whole sibling page `recent.rs`;
match its style exactly (`use super::{{Ctx, Row, note_rows}};`, rows built from `Line`/`Span`, `truncate_str` to
`c.width`, colours from `c.theme`). The panel is narrow (about 30-50 columns): every row is cut to `c.width`.

Types you import (`use crate::acpx::bundle::{{self, NodeState, Outcome, Run, RunStatus, RunSummary, fmt_duration}};`):
{BUNDLE_API}
Public surface:
{FLOWS_API}
`poll`: `bundle::runs_base()` then `bundle::list_runs` (cheap: one small file per run) on every call; the featured
run is the newest Running run, else the newest Waiting run, else the newest run; `bundle::load_run` it when it
changes and `bundle::refresh` it on later polls while it is Running. Return true when the list or the featured run
changed. Keep at most 30 runs. Provide a private `fn poll_at(&mut self, base: &std::path::Path, now_ms: i64) -> bool`
that `poll` calls, so tests can use a tempdir.
`rows`:
- No runs base or no runs: `note_rows("no acpx flow runs yet", c)`.
- Featured run: a bold title row `<glyph> <title>`; a dim row `<flow name> · <done>/<total> · <duration> · <age>`
  (`crate::views::traces_modal::fmt_age`); when Waiting a `theme.warning` row `◐ waiting at <node>` and a dim row with
  the status detail; when Failed a `theme.accent_error` row `✗ <error>`. All of these rows open the featured run.
- Step rail: one row per flow node in flow order: `<state glyph> <node id>` then right-aligned duration of its last
  step; glyph coloured by state (Running accent_running, Waiting warning, Done(Ok) accent_success, failures
  accent_error, NotRun gray_dim). Each opens the featured run. `copy` is `<node id>: <one-line result>`.
- A blank row, a dim `recent` header, then the other runs: `<glyph> <title>` with right-aligned age; each opens its
  own run; `copy` is the run directory.
A row "opens a run" by `Row::new(line, copy).opens(format!("{{OPEN_PREFIX}}{{}}", dir.display()))`.
Tests (at least 9) with bundles written into a `tempfile::tempdir()` by a private helper, a `Ctx` built the way
`recent.rs` tests build theirs if shown (otherwise test a private `fn rows_for(&self, theme: &Theme, width: usize,
now_ms: i64) -> Vec<Row>` that `rows` calls): empty base; featured prefers running over waiting over newest; rail
order and glyphs; waiting rows; failed row; recent list excludes the featured run and carries its own open target;
every row fits the width at 28 columns (display width); poll_at returns true on first load, false when nothing
changed, true after live.json changes; open target starts with OPEN_PREFIX.
{RULES}""",
    },
    {
        "custom_id": "detail",
        "output_file": f"{SRC}/views/acpx_modal/detail.rs",
        "context_files": view_ctx(f"{CTX}/bundle_sample.md"),
        "prompt": f"""
Write `detail.rs`: the right-hand detail panel of the `/acpx` modal. Beside the runs list it shows an overview of the
selected run; beside a run's stages it shows everything about the selected stage, including the agent's conversation
(thinking, tool calls, messages), and follows a running stage live. It is a scrollable column of wrapped text drawn
into the Rect it is given (often 80-120 columns, 30-50 rows). Legibility is the point: clear section rules, generous
use of the height, nothing truncated that could wrap.

Types you import (`use crate::acpx::bundle::{{self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus,
Step, fmt_duration, value_text}};`):
{BUNDLE_API}
{BUNDLE_EXTRA_API}
Public surface:
{DETAIL_API}
Content (build a `Vec<Line<'static>>` wrapped to the width, cache it, draw the visible window):
- A section rule is a row `── <Name> ─────…` to the right edge, name bold in `theme.gray`, rule in `theme.gray_dim`,
  with one blank row above it.
- `DetailTarget::Step(i)`:
  title row `<glyph> <node id>` bold (glyph coloured by outcome) and right-aligned `<duration>`; a `theme.gray` facts
  row: the agent label (`bundle::step_settings(&run.summary.dir)` by attempt id, `AgentSettings::label()`), the kind
  label, the attempt id, `exit <code>` for actions; an error row in `theme.accent_error` when `step.error` is Some.
  Sections, each only when it has content: `Prompt` (`prompt_text`; at most 14 wrapped rows then a dim
  `… +N more rows`); `Command` for actions (`command` + args on one wrapped paragraph, then `cwd`); `Conversation`
  (see below) for steps with `conversation` Some; `Result` (`value_text(&step.output)`, wrapped, at most 40 rows then
  `… +N more rows`); `stdout` / `stderr` for actions (`bundle::read_artifact(&run.summary.dir, artifact, 64 * 1024)`,
  last 30 lines, a read error shown as one dim row); `Events` (the step's trace events, `+<ms>  <kind>  <detail>`,
  heartbeats left out).
- `DetailTarget::Node(id)`: title row with the node's state glyph and id; facts row from
  `bundle::node_settings(&run.summary.dir)`; then by `run.node_state(id)`:
  Running -> section `Conversation (live)` (see below) and `Events` (events whose node_id is this node and that
  belong to no finished step); Waiting -> the run's `status_detail` in `theme.warning`, then section `Gate` listing
  `bundle::gate_choices(run)` as `[<i>] <label> → <flow file>` or the dim row `no next step declared: re-run the flow
  to continue`; NotRun -> one dim row `not run` (when the run is Running or Waiting: `not run yet`).
- `DetailTarget::Run`: title row `<status glyph> <title>` bold; facts row: flow name, `<done>/<total> nodes`,
  duration, age (`fmt_age`) when not live, `stalled` when `bundle::is_stalled`; for Waiting the `status_detail` in
  `theme.warning` and the gate choices as above; for Failed/TimedOut the error in `theme.accent_error`. Section
  `Stages`: one row per flow node in `run.flow.nodes` order: coloured state glyph, node id padded to the longest id,
  agent label or kind label in `theme.gray`, the duration of its last step, then its one-line result (error; `→
  <route>`; `exit <code>`; else first line of the output, objects as compact JSON) cut to the width. Section
  `Last result`: `value_text` of the last step's output, wrapped, at most 20 rows. Section `Input`: `value_text` of
  `run.input`, at most 12 rows.
Conversation: the session's events file is `events.ndjson` (format in bundle_sample.md): one JSON object per line,
`{{"seq", "at", "direction", "message"}}`, `message` being raw ACP JSON-RPC. Read at most the LAST 512 KiB of the
file (seek; drop the first partial line). Lines whose `message.method` is `_auth/status_update` carry account data:
skip them and never display any part of them. Turn the rest into items:
  outbound `session/prompt` -> the prompt's text parts (shown dim, cut to 3 rows: the Prompt section has it in full);
  inbound `session/update` by `params.update.sessionUpdate`: `agent_thought_chunk` and `agent_message_chunk`
  (`content.text`; concatenate consecutive chunks of the same kind into one paragraph), `tool_call` (`toolCallId`,
  `title`, `kind`, `status`), `tool_call_update` (same `toolCallId`: update its status and title; do not add a row);
  everything else is ignored. Draw: thought as `∴ ` + italic `theme.gray` text; a tool call as one row
  `▸ <title>  <status>` with the status word coloured (completed accent_success, failed accent_error, otherwise
  accent_running); a message as plain `theme.fg`-coloured wrapped text (use the primary text colour shown in
  theme.rs.txt). A blank row between items of different kinds.
  For `Step(i)`: the file is `run.session_events_path(&conversation.session_id)`, and only lines with
  `event_start_seq <= seq <= event_end_seq` count. For a Running `Node(id)`: the session is the `run.sessions` entry
  whose `handle` equals the node's `AgentSettings.handle`; only lines with `seq` greater than the largest
  `event_end_seq` of finished steps on that session id count. A missing or unreadable file is one dim row saying so.
Caching and following: rebuild the lines only when (run dir, target, width, `run.summary.updated_ms`,
`run.steps.len()`, `run.events.len()`, and the events file length for a target that has a conversation) changed.
When the target changes, reset the scroll: to the top, except a Running `Node` starts tail-following (pinned to the
bottom, and stays pinned as content grows until the user scrolls up). Draw a `theme.gray_dim` scroll marker in the
last column (`█` over the visible fraction) when the content is taller than the area.
Wrapping: write a private `wrap(text: &str, width: usize) -> Vec<String>` that breaks on spaces, hard-breaks words
longer than the width, measures by display width (`unicode_width`), keeps blank lines, expands tabs to 4 spaces and
drops other control characters. A width under 10 or a height of 0 draws nothing.
{VIEW_STYLE}
Tests (at least 14), building `Run` by hand with a private helper and writing files into a `tempfile::tempdir()`
used as the run dir: wrap breaks on spaces, hard-breaks a long word, keeps blank lines, handles wide characters;
Step shows Prompt, Result and Events sections in that order; an action step shows Command with its exit code and the
stdout artifact's text; a conversation with two thought chunks, a tool call updated to completed, and two message
chunks renders one thought paragraph, one `▸` row containing `completed`, and one message paragraph; lines outside
the seq range are left out; an `_auth/status_update` line's text never appears; a missing events file renders the
dim row; Run target lists the stages with glyphs and shows the gate choices of a waiting run; Node target for a
NotRun node; scroll clamps at both ends and returns false at the limit; target change resets scroll; a Running node
tail-follows as the events file grows and stops after scrolling up; render at 8x3 and 0x0 does not panic; nothing is
drawn outside `area` (render into a sub-rect of a larger buffer and check the cells outside).
{RULES}""",
    },
    {
        "custom_id": "piw_adapter",
        "output_file": "scripts/acpx-piw.py",
        "context_files": [f"{CTX}/piw_protocol.md", f"{CTX}/bundle_sample.md"],
        "prompt": f"""
Write `acpx-piw.py`: a single-file Python 3 program (standard library only, Python 3.11+, asyncio) that lets the
UNMODIFIED terminal viewer `piw` browse and live-follow acpx flow runs. piw is a client of a socket protocol;
piw_protocol.md is the exact contract, worked out from piw's source: follow it to the letter, especially the
canonical-JSON rule, the hello version gate, and the required keys. bundle_sample.md shows what an acpx run folder
holds (`~/.acpx/flows/runs/<runId>/`: manifest.json, flow.json, trace.ndjson, projections/live.json, run.json,
steps.json, sessions/<bundleId>/events.ndjson). acpx rewrites its projection files non-atomically: a file that fails
to parse keeps its previous value for this poll.

Command line (argparse):
  acpx-piw.py serve [--socket PATH]      serve until interrupted; default socket /tmp/acpx-piw-<uid>.sock
  acpx-piw.py open [RUN_ID] [--once]     start the server on /tmp/acpx-piw-<pid>.sock in this process, run
                                         `piw [RUN_ID] [--once]` as a child with PIW_SOCKET=<that path> and
                                         PIW_NO_AUTOSTART=1 in its environment (stdin/stdout/stderr inherited), exit
                                         with piw's exit status, remove the socket file
  acpx-piw.py selftest                   build the run list and every run's snapshot, assert each message is
                                         canonical (parse then re-serialise gives identical bytes) and under
                                         1 MiB, print one line per run and `selftest ok`; exit 1 on any failure
  common: --runs-dir DIR (default: $ACPX_RUNS_DIR, else ~/.acpx/flows/runs), --piw PATH (default `piw`),
          --piw-version X (default: parsed from `<piw> --version`, whose output is like `piw 0.17.7`)
A socket file left behind by a dead server is removed before binding (try to connect first; only remove when the
connect fails). A connection that closes without sending anything is normal (piw probes).

Server behaviour (one asyncio task per connection, NDJSON lines both ways):
- On accept, write the hello line at once.
- `view.runs.watch`: respond accepted with receipt {{"subscribed": true}}; send a `runs` event with ALL runs in one
  page (newest first, at most 200); poll the runs dir every second and send a new `runs` event when the page changed
  (revision is a decimal string of a counter).
- `view.run.watch`: respond accepted (or notFound with an error text when the run folder does not exist), send a
  `run_snapshot` event, then re-read the run every 500 ms and send a new snapshot when its content changed
  (integer `revision` in the payload and the event, incremented on change). `view.run.unwatch`: respond accepted,
  stop that poll.
- `view.page`: kinds `steps` and `trace_at_step` as section 3 / section 8 item 5 of the contract describe (window
  start 0; all steps; `graphSteps` = for each node its latest step at or before the cursor, with prompt and output
  null; `takenTransitions` for steps up to the cursor; trace items for `trace_at_step` = the trace items whose
  attemptId is that step's); `session_entries` / `session_events` return the same pages the snapshot holds; any other
  kind returns an empty page. Always echo `cursor`, and use the snapshot's current `revision`.
- `run.pause` / `run.resume` / `run.cancel` and every other operation: respond rejected with error
  "acpx runs are read-only here".
Mapping an acpx run to piw's snapshot (keys exactly as the contract requires; acpx timestamps are already RFC 3339):
- status: acpx `running|waiting|completed|failed|timed_out` map to the same strings; anything else `ambiguous`.
- manifest: schema "pi-workflows.run-manifest.v1", runId, workflowName = flowName, runTitle, workflowSource
  {{"kind":"file","path": flowPath, "hash": ""}}, startedAt, finishedAt (null while running; for every other status
  the run's `finishedAt` when present, else its `updatedAt`), status, traceSchema "pi-workflows.trace-event.v1",
  paths {{"workflow":"acpx","state":"acpx","trace":"acpx"}}.
- display: status; activity "supervised_runner" when running else null; controls []; reason = statusDetail or null.
- row extras: live = status running and updated within 90 s; possiblyInterrupted = running and not updated for 90 s.
- workflow: schema "pi-workflows.definition-snapshot.v1", name, startAt, nodes as an object in flow.json order with
  nodeType mapped (`acp` -> `agent`; `action`, `compute`, `checkpoint` unchanged), `actionExecution` and `summary`
  copied when present; edges copied from flow.json keeping only `from`, `to`, `switch` ({{on, cases}}).
- state: schema "pi-workflows.run-state.v1", traceSeq = the last trace seq (0 when none), runId, workflowName,
  runTitle, startedAt, updatedAt, finishedAt, status, input (run.json `input`, else null), outputs and results
  (run.json objects, else {{}}), steps, and when present currentNode, waitingOn, statusDetail, error.
- steps (StepRecord) from projections/steps.json in order: attemptId, nodeId, nodeType (mapped as above), outcome,
  startedAt, finishedAt, prompt = promptText (null when absent), output (null when absent), error when present,
  action = the step's `trace.action` restricted to actionType, command, args, cwd, exitCode, signal, durationMs
  (drop null values), conversation {{firstEntryId, lastEntryId}} when the step has session entries.
- graphSteps omitted (defaults to the steps); takenTransitions = distinct "a->b" for consecutive steps;
  stepStart 0, stepTotal = number of steps; live / possiblyInterrupted as for the row.
- tracePage: every trace.ndjson line except type `node_heartbeat`, as {{seq, type, nodeId, attemptId, payload, at}}
  (omit nodeId / attemptId when absent; payload {{}} when absent).
- session: {{"binding": null, "capture": null, "integrity": null, "eventPage": {{"start":0,"total":0,"items":[]}},
  "entryPage": {{start 0, total = len, items}}}} — "entries mode" of section 5. Build entries from each agent step's
  slice of its session's events.ndjson (the step's `trace.conversation` gives `sessionId` (the folder name under
  sessions/), `eventStartSeq`, `eventEndSeq`), in step order: one `user` message entry with the prompt text; then
  the agent's turn as `assistant` message entries whose content is an array of parts — consecutive
  `agent_thought_chunk` text joined into one {{"type":"thinking","thinking":…}} part, consecutive
  `agent_message_chunk` text joined into one {{"type":"text","text":…}} part, each `tool_call` a
  {{"type":"toolCall","id":toolCallId,"name":title,"arguments":rawInput or {{}}}} part; and for each tool call that
  finished (a later `tool_call_update` with status completed or failed) a `toolResult`-role message with one text
  part (the update's text content when it has any, else the status). Entry shape: {{"seq": n (1-based, contiguous
  over the whole list), "at": the event's `at`, "entry": {{"type":"message","id":"<attemptId>:<k>","message":
  {{"role", "content"}}}}}}. Lines whose `message.method` is `_auth/status_update` carry account data: skip them and
  never copy any part of them. Other sessionUpdate kinds are ignored.
- Size: one line must stay under 1 MiB. Cut any single text to 4000 characters (append "…"), any trace payload whose
  JSON is over 2000 characters to {{"truncated": true}}, and if the snapshot line is still over 900 000 bytes drop the
  session entries of the oldest steps (and their `conversation` ranges) until it fits.
Canonical JSON: write your own serialiser (do not rely on `sort_keys`: keys sort by UTF-16 code units): no
whitespace; strings via `json.dumps(s, ensure_ascii=False)` after replacing lone surrogates; ints as ints; a float
that is integral as an int, any other float via `repr` (and never emit NaN/Infinity: use null); bools and null.
Incoming lines are parsed with `json.loads`; a line that does not parse closes that connection.
Robustness: nothing read from disk may crash the server: every per-run failure becomes a skipped run (list) or a
`notFound` response (watch), logged to stderr as one line. Never print the contents of events files. Handle SIGINT /
SIGTERM by closing the server and removing the socket file.
Structure the file so the mapping is testable without sockets: pure functions `canon(obj) -> str`,
`run_row(run_dir) -> dict | None`, `run_snapshot(run_dir, revision) -> dict | None`,
`page_receipt(snapshot, kind, cursor) -> dict`, `session_entries(run_dir, steps) -> list`. No tests in the file
beyond `selftest`.
Return the complete file in ONE ```python block and nothing else: no prose before or after.
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
    parser.add_argument("--output", default=str(HERE / "acpx-tui_out"), help="Base output directory")
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
