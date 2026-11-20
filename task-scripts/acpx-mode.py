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
pub enum StepKind { Acp, Shell, Compute, Checkpoint, Decision, Other }   // flow.json node "nodeType": acp|action|compute|checkpoint|decision; an action node is Shell

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
        outcome: String,              // "ok" | "failed" | "timed_out" | "cancelled" | other acpx strings verbatim
        duration_ms: u64, error: Option<String>,
        output: Value,                // node_outcome payload.outputInline, or Value::Null
        stdout: Option<String>,       // shell: stdout artifact text (first 64 KiB, lossy UTF-8); None otherwise
        exit_code: Option<i64>,       // shell: payload.action.exitCode
        at_ms: i64,
    },
    /// A checkpoint the run now waits on. `question` = the flow node summary, else the node id.
    Waiting { node: String, attempt: String, question: String, choices: Vec<GateChoice>, at_ms: i64 },
    RunFinished { status: String, error: Option<String>, at_ms: i64 },   // projections/live.json status: completed|failed|waiting|timed_out|cancelled
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
    pub fn cancel(&self);                       // SIGTERM to the acpx child (and its process group); idempotent
    pub fn run_dir(&self) -> Option<PathBuf>;   // known once RunStarted was sent
}

/// Spawn `acpx --approve-all --format json flow run <flow_file> --input-json <input_json>` in cfg.cwd and follow
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

pub fn parse_ts(s: &str) -> Option<i64>;        // "2026-11-20T01:14:34.620Z" -> unix ms
pub fn fmt_duration(ms: u64) -> String;         // 0.4s, 27.1s, 2m14s, 1h03m
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
Hard rules: Rust 2024 edition, toolchain 1.94. No `unsafe`, except the single `libc::kill` call allowed in driver.rs
if you use libc (prefer `nix`-free std/tokio APIs; `tokio::process::Child::start_kill` is acceptable for cancel).
No `unwrap()` / `expect()` / indexing that can panic on anything read from disk, the network or the user.
`use agent_client_protocol as acp;` (version 0.10.4, feature "unstable"); only use acp types and builder methods that
appear in the context files. Crates available: agent_client_protocol, xai_acp_lib, tokio (rt, macros, process, sync,
time, io-util, fs), serde_json, serde, anyhow, async_trait, tracing, chrono 0.4. Nothing else.
Names in the "Public surface" block are fixed: other files are being written against them right now. You may add
private helpers and extra public items. Do not redefine types owned by another contract; import them by path.
Tests: a `#[cfg(test)] mod tests` at the bottom; no network, no real acpx, no real agent. File fixtures go in
`tempfile::tempdir()` (dev-dependency). Edition 2024: `gen` is reserved; `std::env::set_var` is unsafe (do not use).
Return the complete file in ONE ```rust block and nothing else.
"""

RUN_FOLDER = """
acpx run folder facts (verified on acpx 0.22; field names exact):
- `~/.acpx/flows/runs/<runId>/` holds manifest.json {runId, flowName, runTitle, flowPath, startedAt, status},
  flow.json {name, startAt, nodes:{<id>:{nodeType, profile?, modelId?, configOptions?:[{configId,value}], summary?}},
  edges:[{from,to} | {from, switch:{on, cases:{<label>:<to>}}}]}, trace.ndjson, projections/live.json
  {status, statusDetail, waitingOn, currentNode, updatedAt}, sessions/<sessionId>/events.ndjson, artifacts/.
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
  waitingOn = node id; acpx then exits. There is no run_completed trace event: the final status is live.json.
- sessions/<sessionId>/events.ndjson lines: {"seq","at","direction":"outbound"|"inbound","message":<JSON-RPC>}.
  A session (handle) can serve several steps. Attribution rule: the k-th outbound message with method
  "session/prompt" in a session file belongs to the k-th acp_prompt_prepared trace event with that sessionId; every
  inbound message with method "session/update" after that prompt and before the inbound response with the same
  JSON-RPC id (a message with "result" or "error" and that id) belongs to that attempt. Lines may arrive in the
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
  polling task, `replay` feeds it everything at once (all trace lines first, then each session file, then a final
  flush), so both paths produce identical events. Emission order inside a tick: trace-derived events in seq order,
  with AgentPrompt/AgentUpdate for an attempt emitted after its StepStarted and before its StepFinished (buffer
  StepFinished of an acp attempt until the session file showed the prompt's response, or until the run ended).
- RunStarted comes first: built from manifest.json + flow.json (FlowSketch: nodes in BFS order from startAt,
  unreachable nodes appended in key order; edges in file order) and the run_started `at`.
- StepStarted on node_started. ShellPrepared on action_prepared (command_line = command + args, args containing
  spaces or quotes shell-quoted with single quotes). StepFinished on node_outcome (kind from payload.nodeType;
  for shell, read stdout from the action_completed stdoutArtifact, first 64 KiB, path must stay inside the run
  folder: reject absolute paths and `..`). Waiting right after the StepFinished of a checkpoint when
  outputInline.next is an object, or when the run ends with live.json status "waiting" (then choices may be empty).
  Never emit Waiting twice for the same attempt.
- RunFinished last: after the acpx child exited (start) or at the end of replay, read projections/live.json status
  and error. In `start`, if no run folder appears within 15 s, or acpx exits before one appears, send RunFinished
  {{status:"failed", error: stderr tail (last 2 KiB) or "acpx did not create a run folder"}}.
- start: tokio::process::Command, stdin null, stdout/stderr piped and drained (stderr tail kept), PATH = extra_path
  joined before the inherited PATH, process_group(0) so cancel can signal the group. Keep polling 300 ms after the
  child exits to catch the last lines, then finish.
- list_runs: newest first by startedAt, at most `limit`.
Tests (at least 12), using a run folder written into a tempdir from string literals that follow the facts above:
replay of an acp step (two session/update lines -> two AgentUpdate with the exact `update` JSON; `_auth/...` line
is skipped); a session used by two steps (attribution by order); session lines that appear before their trace line
(feed in two halves through the Tailer and check identical events to one-shot replay); shell step with stdout
artifact and exit code; an artifact path with `..` is not read; checkpoint with next -> Waiting with sorted choices;
waiting run without next; torn last line ignored then completed on the next feed; switch edges -> one SketchEdge per
case with labels; BFS order with an unreachable node; list_runs order; fmt_duration and parse_ts.
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
  exact order and JSON shapes in subagents.md: parent ToolCall (title = the step
  label "<id> · <model> · <effort>", kind Other, status in_progress) with tool call id "acpx-tool:<run_id>:
  <attempt>", then parent ToolCallUpdate raw_input {{"variant":"Task","task_id":<subagent id>,"run_in_background":
  false,"reconcile":false}}, then XaiUpdate subagent_spawned with subagent_id "acpx-sub:<run_id>:<attempt>",
  parent_session_id, child_session_id (Projector::child_session_id), subagent_type "acpx step", description = the
  step label, model = the model if known. AgentPrompt -> child UserMessageChunk with the prompt text. AgentUpdate ->
  deserialize `update` with `serde_json::from_value::<acp::SessionUpdate>(update.clone())`; on success send it as
  Out::Update for the child session; on failure (unknown variant) drop it. Keep the concatenated agent message text
  per attempt for `final_message`. StepFinished (acp) -> XaiUpdate subagent_finished (status "completed" for ok,
  "failed" otherwise; output = final message text, cut to 4000 chars; error when failed; duration_ms) and parent
  ToolCallUpdate status completed / failed.
- A shell step is an execute tool call in the parent, shown like the pager's bash card (see tool_calls.md for the
  fields the execute card reads): ShellPrepared -> ToolCall id "acpx-tool:<run_id>:<attempt>", kind Execute, title
  = command line, status in_progress, raw_input {{"command": command_line, "cwd": cwd}}. StepFinished -> ToolCallUpdate
  status completed when outcome is "ok" (failed otherwise), content = one text block with stdout (or "(no output)"),
  raw_output {{"output": stdout, "exit_code": exit_code}}.
- Compute and decision steps: one parent ToolCall kind Think, title "<id>", created completed at StepFinished with
  content = a short text of `output` (a string as is; an object as compact JSON cut to 300 chars).
- Checkpoint: StepStarted/StepFinished show nothing by themselves. Waiting -> parent AgentMessageChunk
  "\\n**◐ <node>** · <question>\\n" plus, when choices exist, one line per choice "`<n>` **<label>** → `<flow_file>`"
  (n from 1). (The agent asks the actual question; do not emit the question request here.) Plan entry in_progress.
- RunFinished -> parent AgentMessageChunk on its own paragraph: "✓ **<title>** finished in <duration>" (completed),
  "✗ **<title>** failed: <error>" (failed / timed_out / cancelled show the status word), nothing for "waiting".
  Duration = RunFinished.at_ms - RunStarted.at_ms via crate::driver::fmt_duration.
- Before any parent AgentMessageChunk that follows a tool call or subagent block, start with "\\n" so it renders as a
  new paragraph.
- Use the acp 0.10 builder APIs shown in the context files (e.g. acp::ToolCall::new(...).kind(..).status(..),
  acp::ToolCallUpdate::new(id, acp::ToolCallUpdateFields::new().status(..)...), acp::Plan::new(entries),
  acp::PlanEntry::new(content, priority, status), acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(t)))).
  If a builder you need is not shown, build the value with serde_json::from_value from its wire JSON instead
  (wire names: "sessionUpdate":"tool_call", "toolCallId", "title", "kind", "status", "rawInput", "rawOutput",
  "content":[{{"type":"content","content":{{"type":"text","text":...}}}}]).
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
    pub flows_dir: std::path::PathBuf,               // relative /flow paths resolve here (default: cwd)
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
    Flow {{ file: String, input: serde_json::Value }},  // "/flow <file> [rest]": rest parsed as a JSON object if it is one, else {{"task": rest}} ({{}} when empty)
    Runs,                                            // "/runs"
    Open {{ run_id: String }},                        // "/open <run id>"
    Answer {{ label: String }},                       // "/answer <label>", or a plain message equal (case-insensitive, trimmed) to a waiting gate label
    Task {{ text: String }},                          // any other plain message
    Help,                                            // "/help" or an unknown "/..." command
}}
pub fn parse_command(text: &str, waiting_labels: &[String]) -> Command;
```
Behaviour:
- initialize: protocol V1, load_session(true), agent_info Implementation::new("acpx", env!("CARGO_PKG_VERSION"))
  .title("acpx"). authenticate: Ok. set_session_mode: Ok with no change. ext_method: Err MethodNotFound.
  ext_notification: Ok(()).
- new_session: a fresh session id "acpx-<unix ms>"; remember it as the current session; after responding, send an
  AvailableCommandsUpdate with commands flow (input hint "<flow file> [task or JSON input]"), runs, open (hint
  "<run id>"), answer (hint "<choice>"), help. Send that update from a spawn_local task so it follows the response.
- prompt (one turn = one user message): join the text blocks of the prompt; parse_command with the labels of the
  waiting gate (if any); then:
  * Flow / Task: resolve the flow file (Task uses default_flow; if none, reply with the help text and end). Start
    the run with driver::start, create a Projector for this session, and for every DriverEvent: send each Out
    (Update -> acp::SessionNotification via acp_send with _meta as below; XaiUpdate -> ext notification
    "x.ai/session_notification" with params {{"sessionId","update","_meta"}}). When a Waiting event with choices
    arrives, ask the question (below). The prompt call returns only when the run chain ends: RunFinished not followed
    by a chained run, the user cancelled, or a Waiting the user did not answer. Return
    PromptResponse::new(StopReason::EndTurn) (Cancelled after cancel), stamped with the request's promptId as in
    prompt_response() in the skeleton.
  * Gate question: send `x.ai/ask_user_question` exactly as ask_user_question.md shows (an acp::ExtRequest via
    acp_send, awaiting the response): one question (the Waiting question), options = the choices (label = choice
    label, description = "starts <flow_file>", id = label), multiSelect false, noFreeform true, toolCallId =
    "acpx-gate:<run_id>:<attempt>". Accepted answer -> chain: the next flow file resolves relative to the directory
    of the current run's flow_path (RunStarted.flow_path); its input is
    `xai_grok_pager::acpx::bundle::gate_input(&run, label)` where `run = xai_grok_pager::acpx::bundle::load_run(&run_dir)?`
    (signatures: `pub fn load_run(dir: &Path) -> io::Result<Run>`, `pub fn gate_input(run: &Run, choice: &str) -> Value`);
    send a parent AgentMessageChunk "\\n→ **<label>**: starting `<file>`\\n" and continue streaming the new run in the
    same turn with a new Projector. Cancelled or error answer -> send "\\n◐ Waiting at <node>. Answer later with
    `/answer <label>`.\\n" and end the turn, remembering the waiting gate (run_dir, node, choices, flow_path).
  * Answer: if a gate is remembered and the label matches one of its choices, chain as above in this turn; else reply
    "No gate is waiting." / "Choices: a, b".
  * Runs: reply with a markdown table of driver::list_runs(runs_dir, 20): | run | flow | status | started |.
  * Open: replay that run into the current session in this turn: driver::replay + a new Projector; send the Outs as
    live updates (not isReplay) so they render normally; remember a waiting gate if the run is waiting.
  * Help: a short markdown list of the commands and what a plain message does.
  Replies are parent AgentMessageChunk text.
- _meta on every parent/child session/update during a turn: turnStartMs (prompt start), streamStartMs (prompt start),
  promptId (from the PromptRequest _meta "promptId" when present), agentTimestampMs (now). Child updates also carry
  eventId = "<child session id>:<counter>". See send_update / send_update_for_session in notifications.md.
- cancel: cancel the running RunHandle (if any), mark the turn cancelled so the event loop stops after RunFinished,
  and let any pending question resolve (if the pager does not answer it, treat the cancel as "not answered").
- load_session: the session id is a run id (folder name under runs_dir): set it as the current session, run
  driver::replay, project every event with Projector::new(session id), and send each Out with _meta.isReplay = true
  (XaiUpdate via method "x.ai/session/update"); then return LoadSessionResponse::new(). Unknown run id ->
  acp::Error::invalid_params with a message.
- One run at a time: a prompt that arrives while a run is streaming is answered "A run is in progress; press Esc to
  cancel it." and ends immediately (EndTurn).
- State lives in Rc<RefCell<..>>; never hold a borrow across an .await.
Tests (at least 10): parse_command for every variant, including "/flow ship.flow.ts" (empty object input),
"/flow ship.flow.ts {{\\"name\\":\\"x\\"}}" (object input), "/flow f.ts fix the bug" ({{"task":"fix the bug"}}), a plain
message equal to a waiting label with different case and spaces -> Answer, an unknown slash command -> Help; flow
path resolution relative to flows_dir and absolute paths; next-flow resolution relative to the declaring flow's
directory; the AvailableCommands list contents; _meta building (pure helper) includes promptId only when present.
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
