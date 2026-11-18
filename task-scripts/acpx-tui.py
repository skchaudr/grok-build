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
MAX_TOKENS = 32000  # per slice; a return cut off at this is marked failed, not written

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

# ── Contract 2: the view surface. Slices ledger/run_view/inspector implement it; slice `modal` drives it. ─────────
VIEW_API = """
// ==== crate::views::acpx_modal::ledger  (file: src/views/acpx_modal/ledger.rs) ====
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerTab { Flows, Sessions }
#[derive(Debug, Clone, PartialEq)]
pub enum LedgerOutcome { Unchanged, Changed, OpenRun(std::path::PathBuf), OpenSession(SessionSummary), Copy(String), Rescan, Close }
pub struct LedgerState { /* private */ }
impl LedgerState {
    pub fn new() -> Self;
    pub fn set_runs(&mut self, runs: Vec<RunSummary>);            // keeps the selected run_id selected when still present
    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>);
    pub fn tab(&self) -> LedgerTab;
    pub fn any_live(&self) -> bool;                                // some run is Running
    pub fn handle_key(&mut self, key: &KeyEvent) -> LedgerOutcome;
    pub fn scroll(&mut self, delta: isize) -> bool;                // mouse wheel; true when the selection moved
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64);
    pub fn shortcuts(&self) -> Vec<&'static str>;                  // footer labels, e.g. "Enter open"
}

// ==== crate::views::acpx_modal::run  (file: src/views/acpx_modal/run.rs) ====
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome { Unchanged, Changed, Back, OpenStep(usize), Replay(usize), Copy(String) }   // usize = index into run.steps
pub struct RunViewState { /* private */ }
impl RunViewState {
    pub fn new(run: Run) -> Self;                                  // selection starts on the waiting/current step, else the last step
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
Tests: a `#[cfg(test)] mod tests` at the bottom, no terminal, no network, no snapshot files. Render tests draw into
`Buffer::empty(Rect::new(0, 0, w, h))` and assert on the row text. Tests that need files create them under
`std::env::temp_dir()` in a uniquely named directory and remove it afterwards.
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
  `eventsPath`); a session's agent_name comes from the `sessionBindings` value whose `bundleId` equals the session id.
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
Tests (at least 14): build a bundle directory in the temp dir from string literals modelled on the sample and cover:
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
  else the sum of its numeric values. `cost_usd`: `cumulative_cost` as a number, or its `usd` / `total` field.
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
  Checkpoint `n0{{{{"…"}}}}` (hexagon), Unknown `n0["…"]`.
- One line per edge. Simple: arrow only. Switch: one arrow per case, labelled with the case value (same escaping).
- Arrow: `==>` when `run.edge_taken(from, to)`; `-.->` when `from` has a step but this edge was not taken;
  `-->` when `from` has not run yet. With a label: `==>|small|`, `-.->|large|`, `-->|x|`.
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
        "custom_id": "ledger",
        "output_file": f"{SRC}/views/acpx_modal/ledger.rs",
        "context_files": view_ctx(),
        "prompt": f"""
Write `ledger.rs`: the first screen of the `/acpx` modal, a two-tab list of flow runs and plain acpx sessions.
It draws inside a content Rect the modal chrome already framed; the parent draws the title and the footer.

Types you import (`use crate::acpx::bundle::{{RunStatus, RunSummary, fmt_duration}};`
`use crate::acpx::sessions::SessionSummary;`):
{BUNDLE_API}
{SESSIONS_API}
Public surface:
{VIEW_API.split('// ==== crate::views::acpx_modal::run')[0]}
Layout:
- Row 0: the tabs, ` Flows 12 │ Sessions 40 `, active tab bold + `theme.accent_assistant`, the other `theme.gray`;
  right-aligned on the same row a summary such as `2 running · 1 waiting` and, when filtering, `/text`  `active`.
- Row 1: column header in `theme.gray_dim`. Rows from 2: the list.
- Flows columns: status glyph (coloured), title (flexible width), flow name, state text, age of last update
  (`fmt_age(now_ms - updated_ms)`), duration. State text: Waiting -> `waiting: <waiting_on>`; Running ->
  `<current_node>` plus status_detail when it fits; Failed/TimedOut -> the error cut to fit; Completed -> `done`.
  Drop columns from the right as the width shrinks: duration below 100 columns, flow name below 80, age below 60.
- Sessions columns: `●` open / `○` closed, title, agent, last path component of cwd, message count, tokens
  (`12.3k`, `1.2M`), cost (`$0.12`, blank when None), age of last use. Same shrinking idea.
Keys: movement per the house style; Tab / BackTab / Left / Right / h / l switch tab; `/` opens an inline filter
(typed characters and Backspace edit it, Enter keeps it, Esc clears it and closes the input; matching is
case-insensitive substring over title, flow name, status label, run id — or title, agent, cwd, name for sessions);
`a` toggles active-only (Running or Waiting; for sessions: not closed); `s` cycles the sort (recent, status, name)
and the header shows the active one; Enter -> OpenRun(dir) / OpenSession(row); `y` -> Copy(run dir as a string, or
the session record id); `R` -> Rescan; Esc -> clears a non-empty filter first (Changed), otherwise Close. While the
filter input is open every printable key goes to it.
Each tab keeps its own selection, scroll, and filter. `set_runs` / `set_sessions` re-apply filter and sort.
{VIEW_STYLE}
Tests (at least 12): navigation and clamping; tab switching keeps per-tab selection; filter typing, Enter, Esc
ordering (clear, then Close); active-only; sort cycling changes order; Enter outcomes on both tabs; `y`; `R`;
set_runs keeps the selected run_id; any_live; render at 120x12 shows the tab row, a waiting row containing
`waiting: review` and its glyph; render at 50x6 does not panic and keeps the title; render at 0x0 does not panic;
empty state line.
{RULES}""",
    },
    {
        "custom_id": "run_view",
        "output_file": f"{SRC}/views/acpx_modal/run.rs",
        "context_files": view_ctx(f"{CTX}/traces_fold.rs.txt", f"{CTX}/markdown_content.rs.txt"),
        "prompt": f"""
Write `run.rs`: the screen for one flow run: the flow graph on top, a foldable outline of its steps below.

Types you import (`use crate::acpx::bundle::{{NodeKind, NodeState, Outcome, Run, RunStatus, Step, TraceEvent,
fmt_duration, value_text}};` `use crate::acpx::graph::{{Direction, mermaid_source}};`
`use crate::views::traces_fold::{{FoldNode, FoldState, FoldTree}};` (shown in traces_fold.rs.txt; reuse it, do not
write another fold model) `use crate::scrollback::blocks::markdown_content::MarkdownContent;`):
{BUNDLE_API}
{GRAPH_API}
Public surface:
{VIEW_API.split('// ==== crate::views::acpx_modal::run')[1].split('// ==== crate::views::acpx_modal::inspector')[0]}
Layout, top to bottom inside `area`:
1. Header (1 row): run status glyph + title (bold), then dim ` · `-joined facts: flow name, `<done>/<total> nodes`,
   duration (`run.duration_ms(now_ms)`), age when not live.
2. Banner (1 row, only when needed): Waiting -> `◐ WAITING at <node>: <status_detail>` in `theme.warning`, followed by
   dim `acpx cannot resume a run; re-run the flow to continue`; Failed/TimedOut -> `✗ <error>` in
   `theme.accent_error`; Running -> `● <current_node> · <status_detail>` in `theme.accent_running`.
3. Graph (toggle with `v`, on by default): render
   `MarkdownContent::new_with_table_width(format!("```mermaid\\n{{src}}\\n```"), Some(width)).output(width)` and
   paint each `BlockLine.content` (a `Line<'static>`) with `buf.set_line`. Use `Direction::LeftRight`; if that
   produces more than `area.height / 2` lines or any line wider than the area, use `Direction::TopDown`; if that is
   still too tall, show the first rows that fit and a dim `… graph cut (v hides it)` last row. Cache the rendered
   lines keyed on (width, mermaid source) so it is not re-rendered every frame. Hidden automatically when the area
   has fewer than 12 rows.
4. Separator row in `theme.gray_dim`.
5. Outline (the rest): a `FoldTree<String>` whose roots are the steps (key = attempt id) and whose children are that
   attempt's trace events (key = `"<attempt>/<seq>"`), from `run.step_events`. A node that is Running or Waiting but
   has no step record yet gets a root row too (key = `"live/<node id>"`). Step row:
   `▾|▸ <glyph> <node id>  <kind label or agent name>  <duration>  <one-line result>` where the result is the error
   for failures, `→ <route>` when the output object has a string `route`, the exit code for shell actions, else the
   first line of `value_text(&step.output)`, cut to fit. Event row (indented, dim): `+<ms since step start>  <kind>
   <detail>`; `node_heartbeat` rows are hidden unless `n` (noise) is toggled on. Steps start collapsed.
Keys: movement per the house style over visible rows; Left/h collapse (on a child or collapsed root, jump to the
parent, as `FoldState::collapse` reports); Right/l expand; Space toggles; `E` toggles all; `n` noise; `v` graph;
`!` jumps to the next failed step (wrapping); `c` jumps to the current/waiting step; `[` / `]` previous/next step
root; Enter -> OpenStep(step index) (on an event row: its parent step); `r` -> Replay(step index) only for a step
with `conversation` Some, otherwise Unchanged; `y` -> Copy(value_text of the selected step's output); Esc -> Back.
`sync()` rebuilds the tree after `bundle::refresh` changed the run and keeps selection by key, falling back to the
nearest earlier root; fold state is kept (`FoldState::retain`).
{VIEW_STYLE}
Tests (at least 12), building `Run` values by hand with a private helper: initial selection on the waiting step;
expand shows events and hides heartbeats until `n`; collapse on a child jumps to the parent; `E`; `!` wraps; `c`;
`[` `]`; Enter and `r` outcomes including `r` on a non-acp step; `y`; Esc; sync keeps selection when a step is
appended; render at 120x30 contains the title, the WAITING banner and a step row with its glyph; render with `v`
off has no graph rows; render at 40x8 and 0x0 does not panic.
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
{VIEW_API.split('// ==== crate::views::acpx_modal::inspector')[1]}
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
- `?` toggles a key reference drawn over the content (a plain list of every key for the active screen, taken from a
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
    pattern = r"```[a-zA-Z0-9_-]*[ \t]*\r?\n([\s\S]*?)```"
    matches = re.findall(pattern, content, flags=re.IGNORECASE)
    if matches:
        return "\n\n".join(matches).strip()
    return content.strip()

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
