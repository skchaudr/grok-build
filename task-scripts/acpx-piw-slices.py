"""Data-only parallel-slices tasks for Grok Build (grok acpx + /acpx modal) and piw TUI legibility.
v1: model-generated from contracts (7 slices dispatched via `acpx --agent "agy acp --sandbox"` on `gemini-4-neon` at `thought_level=medium`)
v2: built from your workbench design, less prompting

Living plan (Sab + Claude + Pi):
1. v1 Acceptance Test (tied to the real `beta8-merge` run `20261124T063602457Z-beta8-merge-8814563d` and
   `beta8-merge.workflow.ts`):
   - Preflight (`workflow_preflight`) flags `beta8-merge.workflow.ts` for its ungrounded/missing time limits
     (proposing durations from recorded workflow run history) and its `repair_compile -> cargo_check` loop with
     no loop limit.
   - Replaying the Mini run (`20261124T063602457Z-beta8-merge-8814563d`) shows `cargo_check ✗ exit 101` (never `✓`).
   - The final gate (`final_review_gate`) is visible in both `grok acpx` chat and `/acpx` with the preceding
     `cargo_check` compiler failure tail.
   - Typed plain text at the gate answers it (fuzzy match on choice key/label + trailing notes; unmatched text is
     refused with the valid choices and never cancels the run).

2. Coordinator & Grok 4.7 Integration Sequence (after slice generation):
   - Stage 1 (`xai-grok-pager` + C3): Grok 4.7 reproduces and applies C3 in `crates/codegen/xai-grok-pager/src/{acpx/bundle.rs,piw/mirror.rs}`
     and wires the five `xai-grok-pager` slices:
     * `crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs`
     * `crates/codegen/xai-grok-pager/src/acpx/step_digest.rs`
     * `crates/codegen/xai-grok-pager/src/acpx/workflow_preflight.rs`
     * `crates/codegen/xai-grok-pager/src/acpx/run_status_bar.rs`
     * `crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs`
     into `src/acpx/mod.rs` (`pub mod step_evidence; pub mod step_digest; pub mod workflow_preflight; pub mod run_status_bar;`),
     `src/views/acpx_modal.rs` (`pub(crate) mod modal_view;`), `src/views/acpx_modal/run.rs`, and `src/views/acpx_modal/detail.rs`,
     then runs `cargo test -p xai-grok-pager`.
   - Stage 2 (`xai-grok-pager-bin` + C1/C2): Once Stage 1 compiles and passes tests, Grok 4.7 reproduces and applies
     C1 (`gate.rs:804` `unwatch_session` -> `watch_session(sid, false)` + re-watch in `piw_driver.rs` and `mirror.rs` +
     default Pi agent label `"pi"`) and C2 (`agent.rs:633` persistent `Projector` in `PiwWatch`, no auto-pop-up modal
     on `DriverEvent::Waiting`, and `agent.rs:1463` plain-text gate answering + mid-run steering), wires
     * `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/run_path.rs`
     into `main.rs`, `project.rs`, and `agent.rs`, and runs `cargo test -p xai-grok-pager-bin` plus the four
     `beta8-merge` acceptance checks.
   - Stage 3 (`piw` TUI, independent of Stages 1-2): Grok 4.7 wires
     * `tui/src/ui/pane_zoom.rs`
     into `/Users/sab-mini/repos/pi-workflows/tui/src/ui/{mod.rs,help.rs}` and runs `cargo test` in `pi-workflows/tui`.

3. Detailed C3 Specification (`mirror.rs` and `bundle.rs`):
   - (1) Check `action.exitCode != 0` and `output.ok == false` in `piw_driver.rs` and `mirror.rs`; emit visible
     start/finish cards for `Shell` and `Compute` steps in `project.rs`.
   - (2) In `mirror.rs`, resolve `view.workflow.content` via `input.text_of` whenever `view.workflow` was paged
     (`VIEW_PAGE_BYTES * 2`, e.g. `monitor` run `20261124T064326952Z-monitor-cd63bbf0`) so the full
     `WorkflowDefinitionSnapshot` (including `composition.mounts`) is available.
   - (3) In `mirror.rs` `build_flow_json`, copy per-node `timeoutMs`, `mountPath`, `localNodeId`, and
     `includeTransition` into `flow.json` `nodes.<id>`, copy `snapshot.composition.mounts` (`mountPath`,
     `workflowName`, optional `maxSteps`) into top-level `"mounts"` in `flow.json`, and write top-level
     `"maxSteps"` in `flow.json` only when verified for that specific run:
     - from `snapshot.maxSteps` if present,
     - else from `view.state.workflowSource` (or `view.manifest.workflowSource` / `view.queue.workflowSource`):
       - for `{"kind": "builtin", "id": id, "revision": rev}`, look up the pinned `(id, revision)` table from
         `pi-workflows/src/builtins/catalog.ts` (`plain-summary@3 -> 2`, `workspace-preparation@1 -> 5`,
         `sanity-check@6 -> 8`, `autoplan@6 -> 16`, `change-verification@2 -> 30`, `autodoc@3 -> 80`,
         `plan-approval@4 -> 100` [engine default `DEFAULT_MAX_STEPS = 100`], `autoimplement@14 -> 320`,
         `plan-change@1 -> 400`, `monitor@13 -> 200_000`; unknown revision -> `None`),
       - for `{"kind": "file", "path": path, "hash": sha256_hex}`, read `path` on disk and verify
         `sha256(bytes) == sha256_hex` (matching `workflow-runner-entry.ts:561` `resolveVerifiedWorkflow`; an
         edited or missing file is not proof of the historical run's limit and yields `None`). When the hash
         matches, extract top-level `maxSteps` or apply `DEFAULT_MAX_STEPS = 100` (`engine.ts:75, 555`).
     Add the corresponding fields to `bundle::FlowNode`, `bundle::FlowMount`, and `bundle::FlowDef`.
   - (4) In `mirror.rs`, preserve tool inputs, outputs, and truncation flags in `sessions/main/events.ndjson` from
     one authoritative source per run (`input.activity` `StepActivity` when present; `view.session.eventPage.items`
     when `input.activity` is empty on origin-session runs), correlated by `(attempt_id, tool_call_id)`:
     - write `"rawInput"` on the initial `"tool_call"` line (and on any `"tool_call_update"` that supplies a
       non-empty `tool.input`, because ACPX streams `input: {}` on `pending` and the populated arguments on the
       subsequent update),
     - write `"rawOutput"` on `"tool_call"` / `"tool_call_update"` whenever `tool.output` is `Some` (or from
       non-empty `act.text` on a `"completed"` / `"failed"` update when `tool.output` is `None`),
     - write `"truncated": true` when `tool.truncated` is `true`,
     - keep `"title"`, `"status"`, and `"kind"` on `"tool_call"` / `"tool_call_update"` so `detail.rs`
       `parse_conversation_items` continues to work unchanged while `step_evidence` reads `rawInput`, `rawOutput`,
       and `truncated` from the step's `conversation` sequence range `eventStartSeq..=eventEndSeq`.

4. Seven Slices (pilot first: `step_evidence` at `gemini-4-neon` `thought_level=medium` via `slices acp`):
   - `step_evidence` (`crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs`):
     Owns one attempt's transcript range (`StepTraceLink` and `step_trace_link`), the single transcript reader
     (`read_attempt_tool_calls` merging `tool_call` and `tool_call_update` by `toolCallId`), changed-file evidence
     and out-of-scope audit (`StepFileDigest` and `digest_step_files`), and validator rejections and corrections
     (`ValidationDigest` and `extract_step_validations_with_outcome`, receiving the step's effective `Outcome`
     from the caller rather than reimplementing `effective_outcome`).
   - `step_digest` (`crates/codegen/xai-grok-pager/src/acpx/step_digest.rs`):
     Owns structured run interpretation (`effective_outcome`, `non_zero_exit_code`, `trim_failure_tail`,
     `step_failure_reason`, `latest_run_failure`, `active_branch_nodes`, (a) `step_attempt_budget`,
     (c) `extract_step_verdict`, (d) `step_time_budget`, (e) `run_step_budget`, (f) `parse_scoped_node_id` and
     `summarize_included_workflows`), re-exports `step_evidence`'s public types and functions, and provides the
     3-argument `extract_step_validations(run, step, observed_tools)` wrapper passing `effective_outcome(step)`
     into `step_evidence::extract_step_validations_with_outcome`.
   - `workflow_preflight` (`crates/codegen/xai-grok-pager/src/acpx/workflow_preflight.rs`):
     Workflow creator entry point, cross-run per-step duration and failure history (`analyze_workflow_history`),
     and preflight budget/loop audit (`audit_workflow_preflight` + `PreflightOverrides`).
   - `run_path` (`crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/run_path.rs`):
     Top-down branch-pruned Mermaid at `RunStarted`, single-line progress breadcrumb, inline waiting-gate card
     (no auto pop-up, Issue 11), fuzzy plain-text gate answer matcher (Issue 2), and mid-run live steering
     command classifier (`pause`, `resume`, `steer`, `set`, `status`, `preflight`).
   - `modal_view` (`crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs`):
     Untaken switch-branch pruning, full-modal Graph (`m`/`z`) and Detail (`M`/`Z`) zoom with vertical scrolling,
     two-line waiting-gate banner with preceding failure, and per-step structural + trace link annotation.
   - `run_status_bar` (`crates/codegen/xai-grok-pager/src/acpx/run_status_bar.rs`):
     Single-line live status bar and multi-line `/status` report (Issue 12) covering (a)-(f) and trace links.
   - `pane_zoom` (`piw:tui/src/ui/pane_zoom.rs`):
     Full-body pane maximize (`m`/`M`, `Z`, `Esc`), keyboard split resizing (`Alt+j`/`Alt+k`), and 0-column
     sidebar hide (`b`) in `piw` TUI.

Dry-render all 7 ACP packets locally (sends nothing):
  python3 /Users/sab-mini/repos/pi-workflows/skills/parallel-slices/scripts/slices.py acp \
      /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py \
      --model gemini-4-neon --thought-level medium --dry-run

Pilot only `step_evidence` in a named Rex pane with `--keep-open`:
  python3 /Users/sab-mini/repos/pi-workflows/skills/parallel-slices/scripts/slices.py acp \
      /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py \
      --only step_evidence --model gemini-4-neon --thought-level medium --rex
"""

REPOS = {
    "grok": "/Volumes/Mini-SSD/offload/repos/grok-build",
    "piw": "/Users/sab-mini/repos/pi-workflows",
}

SYSTEM = (
    "You are a principal Rust software engineer. Use only this packet; do not use tools, "
    "do not read or write files, and do not run shell commands. Follow the conventions of "
    "the attached repository excerpts. Return only the requested file in the exact "
    "`=== FILE: <path> ===` format given, followed by one fenced `rust` code block, "
    "with no prose before or after."
)

RUST_GROK_ENV = """
Environment:
- Rust 2024 edition, toolchain 1.94.
- No `unsafe`. No `.unwrap()` or `.expect()` in non-test code. Never index slices in a way that can panic on empty or short inputs.
- Use only crates already imported in the attached excerpts (`std`, `serde_json`, `ratatui`, `crossterm`, `unicode_width`). Add no new dependencies.
- Keep functions small, deterministic, and pure. Include comprehensive `#[cfg(test)] mod tests` at the bottom of the file that run with `cargo test` without network, disk outside memory/temp, or a live terminal.
"""

POST_C3_BUNDLE_TYPES_NOTE = """
// ==== Data sources (verified against pi-workflows and grok-build source; read nothing else, guess no keys) ====
// IMPORTANT — Pre-C3 attachment vs Post-C3 bundle types:
// The attached `bundle.rs#L1-345` excerpt shows today's pre-C3 `FlowNode` (4 fields) and `FlowDef` (4 fields).
// Before compiling this slice, the coordinator's C3 change updates `mirror.rs` and `crate::acpx::bundle` so that
// `flow.json` and `sessions/main/events.ndjson` carry the verified server metadata below. Import `FlowNode`,
// `FlowMount`, and `FlowDef` from `crate::acpx::bundle` (do NOT redefine them), and write code and tests against
// these exact post-C3 `crate::acpx::bundle` declarations:
//
//   #[derive(Debug, Clone, Default, PartialEq)]
//   pub struct FlowNode {
//       pub id: String,
//       pub kind: NodeKind,
//       pub summary: Option<String>,
//       pub is_shell: bool,
//       pub timeout_ms: Option<u64>,            // snapshot `timeoutMs`; None when absent or null
//       pub mount_path: Vec<String>,            // snapshot `mountPath`; empty for root-workflow nodes
//       pub local_node_id: Option<String>,      // snapshot `localNodeId`; for includeTransition=="exit", the exit name
//       pub include_transition: Option<String>, // snapshot `includeTransition`: Some("entry" | "exit") or None
//   }
//
//   #[derive(Debug, Clone, Default, PartialEq, Eq)]
//   pub struct FlowMount {
//       pub mount_path: Vec<String>,            // snapshot `composition.mounts[i].mountPath`
//       pub workflow_name: String,              // snapshot `composition.mounts[i].workflowName`
//       pub max_steps: Option<usize>,           // snapshot `composition.mounts[i].maxSteps`; None when child has no cap
//   }
//
//   #[derive(Debug, Clone, Default, PartialEq)]
//   pub struct FlowDef {
//       pub name: String,
//       pub start_at: String,
//       pub nodes: Vec<FlowNode>,
//       pub edges: Vec<FlowEdge>,
//       pub max_steps: Option<usize>,           // verified root `maxSteps` (configured or default 100); None when unknown
//       pub mounts: Vec<FlowMount>,             // snapshot `composition.mounts`
//   }
//
// Verified upstream producers and mirror mappings:
// 1. Node & include metadata (`pi-workflows/src/workflows/store.ts` `createDefinitionSnapshot`, `src/server/view.ts`
//    `projectWorkflowNode`): per node `timeoutMs` (number|null), `mountPath` (string[]), `localNodeId` (string),
//    `includeTransition` ("entry"|"exit"). Included node ids are `<mountPath.join("/")>/<localId>`; an include's exit
//    node has qualified id `<mount>/__piw_exit_<exitName>`, `includeTransition == "exit"`, and `localNodeId` equal to
//    `<exitName>` itself (e.g. `"passed"`). Look up a step's node via `run.flow.nodes.iter().find(|n| n.id == step.node_id)`.
// 2. Root and included-workflow `maxSteps`:
//    - Root limit (`pi-workflows/src/workflows/engine.ts:75, 555`): `workflow.maxSteps ?? DEFAULT_MAX_STEPS (100)`.
//      `mirror.rs` writes top-level `"maxSteps"` into `flow.json` (`run.flow.max_steps`) ONLY when verified against
//      the run's immutable `workflowSource` (`sha256(file) == source.hash` or pinned builtin `(id, revision)`). If a
//      file was edited after the run or the source is unverified, `run.flow.max_steps` is `None` (unknown).
//    - Included-workflow limit (`pi-workflows/src/workflows/composition.ts:232-238`): each mount in
//      `snapshot.composition.mounts` (`run.flow.mounts`) carries `max_steps: Some(n)` only when the child workflow
//      explicitly declared `maxSteps`; otherwise `max_steps: None` (no separate child cap).
//    - Step counting rule (`engine.ts:564-573`, `countExecutableSteps` at `engine.ts:1707-1715`, and
//      `assertInvocationStepLimit` at `engine.ts:1717-1752`): include transition steps (`include_transition ==
//      Some("entry" | "exit")`, or node id matching a mount entry / `<mount>/__piw_exit_<name>`) are NOT counted
//      against `maxSteps`. Root `used_steps` counts all non-transition steps in `run.steps`. Included-workflow
//      `used_steps` counts all non-transition steps inside that mount (`step.node_id.starts_with("<mount>/")`)
//      after the most recent `"entry"` transition step (`step.node_id == "<mount>"`) in `run.steps` (or across all
//      `run.steps` if no `"entry"` step is present).
// 3. Tool inputs/outputs and validator rejections (`pi-workflows/src/server/step-activity.ts`,
//    `src/server/acpx-executor.ts:311-334`, `src/server/rpc-executor.ts:86-105`, `src/server/server.ts:1654-1695`,
//    and `src/piw/mirror.rs`):
//    - In C3, `mirror.rs` writes one authoritative stream per attempt into `sessions/main/events.ndjson` (bounded by
//      `step.conversation` `event_start_seq..=event_end_seq`): from `StepActivity` for ACPX/RPC runs, or from
//      `view.session.eventPage.items` for origin-session runs.
//    - Each tool call is written as `{"seq": N, "direction": "inbound", "message": {"method": "session/update",
//      "params": {"update": {"sessionUpdate": "tool_call" | "tool_call_update", "toolCallId": "<id>",
//      "title": "<title>", "kind": "<kind>", "status": "pending" | "in_progress" | "completed" | "failed",
//      "rawInput": <Value>, "rawOutput": <Value>, "truncated": <bool>}}}}`.
"""

STEP_EVIDENCE_PUBLIC_API = """
// ==== Public contract of `crate::acpx::step_evidence` (re-exported by `crate::acpx::step_digest`) ====
use std::path::PathBuf;
use serde_json::Value;
use crate::acpx::bundle::{Outcome, Run, Step};

/// One merged tool call from a step attempt's `sessions/main/events.ndjson` slice (`event_start_seq..=event_end_seq`).
#[derive(Debug, Clone, PartialEq)]
pub struct MergedToolCall {
    pub tool_call_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub raw_input: Value,
    pub raw_output: Option<Value>,
    pub content: Option<Value>,
    pub truncated: bool,
}

/// File-level progress and scope check for one step attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepFileDigest {
    pub expected_files: Vec<String>,
    pub touched_files: Vec<String>,
    pub out_of_scope_files: Vec<String>,
    pub current_file: Option<String>,
}

impl StepFileDigest {
    pub fn summary_line(&self) -> String;
}

/// (b) Validator rejections and correction attempts for one step attempt, extracted from the step's transcript slice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationDigest {
    pub rejection_count: usize,
    pub correction_attempts: usize,
    pub resolved: bool,
    pub last_error: Option<String>,
    pub last_rejected_output: Option<String>,
}

impl ValidationDigest {
    /// Returns:
    /// - `None` when `self.rejection_count == 0`,
    /// - `Some(format!("1 validation rejected -> corrected ({err})"))` (or `"N validations rejected -> corrected ({err})"`)
    ///   when `self.resolved` is `true` (omitting `" ({err})"` when `self.last_error` is `None`),
    /// - `Some(format!("1 validation rejected ({err})"))` (or `"N validations rejected ({err})"`)
    ///   when `self.resolved` is `false` (omitting `" ({err})"` when `self.last_error` is `None`).
    pub fn badge_label(&self) -> Option<String>;
}

/// Direct link from a step card to its session transcript slice, stdout/stderr artifacts, and live trace filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTraceLink {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub session_events_path: Option<PathBuf>,
    pub event_range: Option<(u64, u64)>,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    pub output_artifact_path: Option<PathBuf>,
}

impl StepTraceLink {
    /// True when at least one transcript range or artifact path is available to open.
    pub fn is_openable(&self) -> bool;
    /// Compact badge showing what can be opened, e.g. `"trace: events 12..48"` or `"trace: stderr"`, or `None` when `!is_openable()`.
    pub fn badge_label(&self) -> Option<String>;
}

pub fn step_trace_link(run: &Run, step: &Step) -> StepTraceLink;
pub fn read_attempt_tool_calls(run: &Run, step: &Step) -> Vec<MergedToolCall>;
pub fn digest_step_files(run: &Run, step: &Step, observed_tools: &[&str]) -> StepFileDigest;
pub fn extract_step_validations_with_outcome(
    run: &Run,
    step: &Step,
    observed_tools: &[&str],
    effective_outcome: Outcome,
) -> ValidationDigest;
"""

# Shared contract exposed by `crate::acpx::step_digest` (which re-exports `step_evidence` and wraps
# `extract_step_validations_with_outcome` into `extract_step_validations(run, step, observed_tools)`).
# Pasted identically into `step_digest`, `workflow_preflight`, `run_path`, `modal_view`, and `run_status_bar`.
STEP_DIGEST_SHARED_API = (
    POST_C3_BUNDLE_TYPES_NOTE
    + STEP_EVIDENCE_PUBLIC_API
    + """
// ==== Additional structured run interpretation in `crate::acpx::step_digest` ====
// (`step_digest` re-exports `MergedToolCall`, `StepFileDigest`, `ValidationDigest`, `StepTraceLink`,
// `step_trace_link`, `read_attempt_tool_calls`, `digest_step_files`, and `extract_step_validations_with_outcome`
// from `crate::acpx::step_evidence`, and adds the types and functions below.)
use std::collections::HashSet;

pub const DEFAULT_FAILURE_MAX_LINES: usize = 6;
pub const DEFAULT_FAILURE_MAX_CHARS: usize = 480;

/// (a) Loop attempt count for one step execution (counted from `run.steps`; the engine has no per-node attempt limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeAttemptBudget {
    pub attempt: usize,
    pub observed_attempts: usize,
}

impl NodeAttemptBudget {
    pub fn badge_label(&self) -> Option<String>;
}

/// (c) Reviewer or router verdict extracted from a step's output and outgoing switch edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepVerdict {
    pub route_key: String,
    pub target_node: Option<String>,
    pub reason: Option<String>,
}

impl StepVerdict {
    pub fn summary_line(&self) -> String;
}

/// (d) Time budget (`elapsed_ms` vs `timeout_ms`) and timeout state for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepTimeBudget {
    pub elapsed_ms: u64,
    pub timeout_ms: Option<u64>,
    pub timed_out: bool,
    pub near_expiry: bool,
}

impl StepTimeBudget {
    pub fn badge_label(&self) -> String;
}

/// (e) Executable (non-transition) steps used and remaining against the root workflow's `maxSteps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepBudgetSummary {
    pub used_steps: usize,
    pub max_steps: Option<usize>,
    pub remaining_steps: Option<usize>,
}

impl StepBudgetSummary {
    /// - When `max_steps == Some(max)`: `Some(format!("steps {}/{max} ({} left)", self.used_steps, self.remaining_steps.unwrap_or(0)))`.
    /// - When `max_steps == None` and `self.used_steps > 0`: `Some(format!("steps {}/?", self.used_steps))`.
    /// - When `max_steps == None` and `self.used_steps == 0`: `None`.
    pub fn badge_label(&self) -> Option<String>;
}

/// (f) Summary of an included child workflow, grouped by `FlowNode::mount_path` / `FlowMount::mount_path` (joined with "/").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeScopeSummary {
    pub mount_path: String,
    pub invocations: usize,
    pub executed_child_nodes: usize,
    pub total_child_nodes: usize,
    pub used_steps: usize,
    pub max_steps: Option<usize>,
    pub remaining_steps: Option<usize>,
    pub active: bool,
    pub named_exit: Option<String>,
}

impl IncludeScopeSummary {
    pub fn summary_line(&self) -> String;
}

pub fn effective_outcome(step: &Step) -> Outcome;
pub fn non_zero_exit_code(step: &Step) -> Option<i64>;
pub fn trim_failure_tail(
    error: Option<&str>,
    stderr: Option<&str>,
    output_tail: Option<&str>,
    stdout: Option<&str>,
    exit_code: Option<i64>,
    max_lines: usize,
    max_chars: usize,
) -> Option<String>;
pub fn step_failure_reason(step: &Step) -> Option<String>;
pub fn latest_run_failure(run: &Run) -> Option<(String, Option<i64>, String)>;
pub fn active_branch_nodes(
    start_at: &str,
    node_ids: &[String],
    edges: &[(String, String)],
    visited: &HashSet<String>,
) -> HashSet<String>;
pub fn extract_step_validations(run: &Run, step: &Step, observed_tools: &[&str]) -> ValidationDigest;
pub fn step_attempt_budget(run: &Run, step: &Step) -> NodeAttemptBudget;
pub fn extract_step_verdict(run: &Run, step_index: usize) -> Option<StepVerdict>;
pub fn step_time_budget(run: &Run, step: &Step, now_ms: i64) -> StepTimeBudget;
pub fn run_step_budget(run: &Run) -> StepBudgetSummary;
pub fn parse_scoped_node_id(node_id: &str) -> (Option<&str>, &str, Option<&str>);
pub fn summarize_included_workflows(run: &Run) -> Vec<IncludeScopeSummary>;
"""
)

RETURN_FORMAT_TEMPLATE = """
Return format:
Return the complete file and nothing else, introduced by the exact header line below followed by one fenced `rust` block. No omitted bodies, no pseudocode, no prose before or after:

=== FILE: {target} ===
```rust
<complete rust source>
```
"""

TASKS = [
    {
        "custom_id": "step_evidence",
        "output_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs",
        ],
        "context_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L1-345",
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L555-590",
            "grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs#L1228-1370",
        ],
        "prompt": f"""
Slice id: step_evidence
Target file: crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs
Goal: Own one step attempt's transcript range and artifact links (`StepTraceLink` and `step_trace_link`), the single transcript reader that merges `tool_call` and `tool_call_update` records in `sessions/main/events.ndjson` (`MergedToolCall` and `read_attempt_tool_calls`), per-step file progress and out-of-scope file audit from preserved tool inputs (`StepFileDigest` and `digest_step_files`), and validator rejections and correction attempts (`ValidationDigest` and `extract_step_validations_with_outcome`).

Ownership:
Return this complete file only. Do not modify other files or redefine `Step`, `Outcome`, `ActionReceipt`, `ArtifactRef`, `ConversationRef`, `NodeKind`, `Run`, or `RunStatus` (import them from `crate::acpx::bundle::{{NodeKind, Outcome, Run, Step}}`). Do NOT import `crate::acpx::step_digest` or reimplement `effective_outcome`: `extract_step_validations_with_outcome` receives `effective_outcome: Outcome` as an argument from the caller. The coordinator owns registering `pub mod step_evidence;` in `src/acpx/mod.rs` and re-exporting its public items from `crate::acpx::step_digest`.
{RUST_GROK_ENV}
Existing interfaces:
- `crate::acpx::bundle::{{ArtifactRef, ConversationRef, NodeKind, Outcome, Run, RunStatus, SessionStream, Step}}` (`bundle.rs#L1-345`, `#L555-590`).
- `detail.rs#L1228-1370` shows how `sessions/main/events.ndjson` lines are structured (`seq`, `direction`, `message.method == "session/update"`, `message.params.update`).

Public surface the coordinator and `step_digest` will call:
```rust
{POST_C3_BUNDLE_TYPES_NOTE}
{STEP_EVIDENCE_PUBLIC_API}
```

Behavior details:
- `step_trace_link(run: &Run, step: &Step) -> StepTraceLink`:
  - Resolve `session_events_path` and `event_range` from `step.conversation`: look up `run.session_events_path(&conv.session_id).map(Path::to_path_buf)` and `Some((conv.event_start_seq, conv.event_end_seq))`. If `step.conversation` is `None` and `step.kind == NodeKind::Acp`, fall back to `run.sessions.first().map(|s| s.events_path.clone())` with `event_range: None`.
  - Resolve `stdout_path`, `stderr_path`, and `output_artifact_path` by joining `run.summary.dir` with `art.path` when `step.stdout`, `step.stderr`, or `step.output_artifact` is `Some(art)` and `!art.path.is_empty()`.
  - `StepTraceLink::is_openable(&self) -> bool`: true when `self.event_range.is_some()` or `self.session_events_path.is_some()` or `self.stdout_path.is_some()` or `self.stderr_path.is_some()` or `self.output_artifact_path.is_some()`.
  - `StepTraceLink::badge_label(&self) -> Option<String>`:
    - If `!self.is_openable()`, return `None`.
    - If let `Some((start, end)) = self.event_range`, return `Some(format!("trace: events {{start}}..{{end}}"))`.
    - Else if `self.stderr_path.is_some()`, return `Some("trace: stderr".to_string())`.
    - Else if `self.stdout_path.is_some()`, return `Some("trace: stdout".to_string())`.
    - Else if `self.output_artifact_path.is_some()`, return `Some("trace: output".to_string())`.
    - Else return `Some("trace: session".to_string())`.
- `read_attempt_tool_calls(run: &Run, step: &Step) -> Vec<MergedToolCall>`:
  - This is the single transcript reader for step evidence. Resolve `link = step_trace_link(run, step)`.
  - If `link.session_events_path` is `None` or the file cannot be read, return `Vec::new()`.
  - When `link.event_range` is `None` and `run.steps.len() > 1`, return `Vec::new()` (do not attribute a whole multi-step session log to one step when `event_range` is missing).
  - Read the NDJSON file line by line. Parse each non-empty line as `serde_json::Value`.
  - If `link.event_range` is `Some((start, end))`, keep only lines where `val.get("seq").and_then(Value::as_u64)` is `Some(seq)` with `seq >= start && seq <= end`.
  - Inspect lines where `val.get("direction").and_then(Value::as_str) == Some("inbound")` and `val.pointer("/message/method").and_then(Value::as_str) == Some("session/update")`.
  - Let `update = val.pointer("/message/params/update")`. Match `update.get("sessionUpdate").and_then(Value::as_str)`:
    - Only handle `"tool_call"` and `"tool_call_update"`.
    - Let `tool_call_id = update.get("toolCallId").or_else(|| update.get("id")).and_then(Value::as_str).unwrap_or("")`.
    - Correlate records by `tool_call_id` (preserving first-seen insertion order across distinct non-empty `tool_call_id`s; if `tool_call_id` is empty, treat the record as its own entry):
      - `title`: update when `update.get("title").and_then(Value::as_str)` is non-empty (falling back to `update.get("name").and_then(Value::as_str)`).
      - `kind`: update when `update.get("kind").and_then(Value::as_str)` is non-empty.
      - `status`: update when `update.get("status").and_then(Value::as_str)` is non-empty (default `"pending"`).
      - `raw_input`: check `update.get("rawInput").or_else(|| update.get("input")).or_else(|| update.get("args"))`. Keep the latest non-empty value (specifically, do not overwrite a populated object/value with `Value::Null` or an empty object `{{}}`, and do overwrite an initial `Value::Null` or empty `{{}}` when a subsequent `"tool_call_update"` supplies the populated arguments).
      - `raw_output`: check `update.get("rawOutput").or_else(|| update.get("output")).or_else(|| update.get("result"))`; if present and non-null, update `raw_output = Some(v.clone())`.
      - `content`: if `update.get("content")` is present and non-null, update `content = Some(v.clone())`.
      - `truncated`: `entry.truncated |= update.get("truncated").and_then(Value::as_bool).unwrap_or(false)`.
- `digest_step_files(run: &Run, step: &Step, observed_tools: &[&str]) -> StepFileDigest`:
  - `expected_files`: extract distinct repo-relative file paths from `step.prompt_text.as_deref().unwrap_or("")` (tokens containing `'/'` and ending with a known source/config extension such as `.rs`, `.ts`, `.js`, `.py`, `.toml`, `.json`, `.md`, `.sh`, `.yml`, `.yaml`, or `Cargo.lock` / `.gitignore`, stripping surrounding backticks/quotes/punctuation and stripping any leading `/.../crates/` or `/.../src/` worktree prefix to start at `crates/`, `src/`, `task-scripts/`, `.pi/`, `docs/`, `tests/`, or `tui/`).
  - `touched_files`: collect distinct normalized paths (in first-seen order) from:
    1. `read_attempt_tool_calls(run, step)` entries that modify or stage files:
       - Edit/Write tool calls (`kind` or `title` case-insensitively containing `"edit"`, `"write"`, `"multiedit"`, `"searchreplace"`, or `raw_input.get("variant")` matching `"SearchReplace"` / `"Write"`, or `raw_input` having `"new_string"` / `"oldText"` / `"edits"` / `"content"` alongside `"file_path"` / `"path"`): extract `raw_input.get("file_path").or_else(|| raw_input.get("path")).and_then(Value::as_str)` (also checking `raw_input.get("tool_input")` if nested).
       - Execute/Bash tool calls (`kind` or `title` case-insensitively containing `"execute"` / `"bash"` or `raw_input.get("command")`): extract file paths passed to `git add` or `git rm` in `raw_input["command"]`.
       (Ignore read-only tools like `read`, `grep`, `find`, `ls`, `search_tool`, `ToolSearch`, and `workflow`.)
    2. `observed_tools` strings (extracting paths from `"Edit ..."` / `"Write ..."` / `"git add ..."`).
  - Normalize every extracted path by stripping leading worktree prefixes up to `crates/`, `src/`, `task-scripts/`, `.pi/`, `docs/`, `tests/`, or `tui/` when present; if a path is outside the repo/worktree (e.g. `/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh` when `expected_files` are under `crates/`), keep its normalized path so it is flagged in `out_of_scope_files`.
  - `out_of_scope_files`: when `expected_files` is non-empty, every path in `touched_files` not present in `expected_files`; when `expected_files` is empty, `Vec::new()`.
  - `current_file`: `touched_files.last().cloned()`.
  - `StepFileDigest::summary_line(&self) -> String`:
    - If `expected_files.is_empty() && touched_files.is_empty()`: `String::new()`.
    - Start with `if !expected_files.is_empty() {{ format!("files {{}}/{{}}", touched_files.len().saturating_sub(out_of_scope_files.len()), expected_files.len()) }} else {{ format!("files {{}}", touched_files.len()) }}`.
    - If `!out_of_scope_files.is_empty()`, append `format!(" (!{{}} out-of-scope)", out_of_scope_files.len())`.
    - If let `Some(curr) = &self.current_file`, append `format!(" · {{curr}}")`.
- `extract_step_validations_with_outcome(run: &Run, step: &Step, observed_tools: &[&str], effective_outcome: Outcome) -> ValidationDigest`:
  - Inspect `read_attempt_tool_calls(run, step)` in chronological order:
    - Identify **workflow submission calls**: tool calls where (`title` or `kind` is `"workflow"`, `"workflow__workflow"`, or `"mcp__workflow__workflow"`, OR `raw_input.get("tool_name")` is one of those, OR `raw_input` has `"action": "submit"` and `"requestId"`), AND the inner input (`raw_input.get("tool_input").unwrap_or(&raw_input)`) has `"action": "submit"` (or `"action"` is omitted on a failed `"workflow"` tool call whose output text contains `"The workflow rejected your submission"`). Ignore `"action": "update"` calls and unrelated tools (`bash`, `edit`, `ToolSearch`, etc.).
    - Extract raw output text from `call.raw_output` (or `call.content`): string directly, or `content[0].text`, or `result.content[0].text`, or `error`, or compact JSON.
    - Skip transport/permission denials that blocked execution before the validator ran (specifically when output text contains `"was not executed: Denied by permission policy"` or `"workflow step channel is unavailable"`).
    - For each remaining workflow submission call `(idx, call)` out of `total_submit_calls` on this attempt:
      - Extract `output_preview` from `inner_input.get("output")` (compact JSON truncated to 120 chars).
      - The submission call is **rejected** if:
        1. Output text contains `"The workflow rejected your submission"` (extract the clean error reason after the first `":\\n"` or `":"` and before `"Call the `workflow` tool again"`, trimming whitespace and collapsing newlines to spaces), OR
        2. `call.status == "failed"` (extract the clean error reason from output text, stripping any `"The workflow rejected your submission for step ...:"` prefix and `"Call the `workflow` tool again with a corrected output."` suffix), OR
        3. `idx + 1 < total_submit_calls` (an earlier `"submit"` call on the same attempt that was followed by a re-submission on the same attempt, as in `RpcStepExecutor`).
      - A submission call is **accepted** if it is not rejected and (`call.status == "completed"` or output text contains `"Submission accepted"` or `"Workflow step output accepted"` or (`idx + 1 == total_submit_calls && effective_outcome == Outcome::Ok && step.finished_ms > step.started_ms`)).
  - Also scan `observed_tools` entries containing `"The workflow rejected your submission"`; and if `rejection_count == 0` while the step is currently waiting (`run.summary.waiting_on.as_deref() == Some(&step.node_id)` and `step.kind != NodeKind::Checkpoint`), check `run.summary.status_detail.as_deref()` when it starts with or contains `"The workflow rejected your submission"`.
  - Set `resolved = rejection_count > 0 && (has_accepted_submit_after_last_rejection || (effective_outcome == Outcome::Ok && step.finished_ms > step.started_ms && run.summary.current_node.as_deref() != Some(&step.node_id) && run.summary.waiting_on.as_deref() != Some(&step.node_id)))`.
  - Set `correction_attempts = if resolved {{ rejection_count }} else {{ total_submit_calls_after_first_rejection }}`.

Tests:
- `step_trace_link` resolves `session_events_path`, `event_range: Some((10, 25))`, and artifact paths under `run.summary.dir`, and formats `badge_label() == Some("trace: events 10..25".into())`.
- `read_attempt_tool_calls` merges an initial `"tool_call"` (`status: "pending"`, `rawInput: {{}}`) with its subsequent `"tool_call_update"` (`status: "completed"`, populated `rawInput`, `rawOutput`, `truncated: true`) by `toolCallId` inside `event_range`, while ignoring events outside `event_range`.
- `digest_step_files` reads a temp `events.ndjson` transcript slice containing preserved `rawInput` payloads for an `edit` tool call on `crates/codegen/xai-grok-shell/src/lib.rs` (in `expected_files` from `step.prompt_text`) and an out-of-scope `edit` tool call on `/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh` (derived from `beta8-merge` seq 1343), verifying `expected_files`, `touched_files`, `out_of_scope_files == vec!["/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh"]`, and `summary_line()` containing `"(!1 out-of-scope)"`.
- `extract_step_validations_with_outcome` reads a temp `events.ndjson` transcript slice where seq 10..11 is a rejected `workflow` `"submit"` call (`rawInput: {{"action": "submit", "requestId": "req-1", "output": {{"resolvedCount": 12}}}}`, `rawOutput: "The workflow rejected your submission for step \\"resolve_group2_shell\\":\\nresolvedCount must equal 35\\n\\nCall the `workflow` tool again with a corrected output."`) and seq 12..13 is an accepted `workflow` `"submit"` call (`rawInput: {{"action": "submit", "requestId": "req-1", "output": {{"resolvedCount": 35}}}}`, `rawOutput: "Submission accepted. This step is complete; do not call the workflow tool again."`), plus an unrelated failed `bash` tool call at seq 9 (`status: "failed"`, `rawOutput: "exit 1"`) and a pre-validator permission denial (`"was not executed: Denied by permission policy"`), and verifies `rejection_count == 1`, `correction_attempts == 1`, `resolved == true`, `last_error == Some("resolvedCount must equal 35".into())`, `last_rejected_output == Some("{{\\"resolvedCount\\":12}}".into())`, and `badge_label() == Some("1 validation rejected -> corrected (resolvedCount must equal 35)".into())`, plus an unresolved case when `effective_outcome` is `Outcome::Failed`.
""" + RETURN_FORMAT_TEMPLATE.format(target="crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs"),
    },
    {
        "custom_id": "step_digest",
        "output_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/step_digest.rs",
        ],
        "context_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L1-345",
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L555-590",
            "grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs#L1-145",
        ],
        "prompt": f"""
Slice id: step_digest
Target file: crates/codegen/xai-grok-pager/src/acpx/step_digest.rs
Goal: Implement structured run interpretation—effective step outcome check, canonical failure-tail trimmer, untaken-branch node pruner, (a) loop attempt counts, (c) reviewer/router verdicts with reasons, (d) step `elapsed vs timeoutMs` time budgets and timeouts, (e) root and per-include `maxSteps` used and remaining, and (f) included child workflows with named exits—while re-exporting `crate::acpx::step_evidence`'s public items and providing the 3-argument `extract_step_validations(run, step, observed_tools)` wrapper that passes `effective_outcome(step)` to `step_evidence::extract_step_validations_with_outcome`.

Ownership:
Return this complete file only. Do not modify other files or redefine `Step`, `Outcome`, `ActionReceipt`, `FlowEdge`, `FlowNode`, `FlowMount`, `FlowDef`, or `Run` (import them from `crate::acpx::bundle::{{fmt_duration, FlowDef, FlowEdge, FlowMount, FlowNode, NodeKind, Outcome, Run, RunStatus, Step}}`). Do NOT implement a second transcript reader in this file: re-export `MergedToolCall`, `StepFileDigest`, `ValidationDigest`, `StepTraceLink`, `step_trace_link`, `read_attempt_tool_calls`, `digest_step_files`, and `extract_step_validations_with_outcome` from `crate::acpx::step_evidence`, and implement `extract_step_validations` as a thin wrapper passing `effective_outcome(step)`. The coordinator owns registering `pub mod step_evidence; pub mod step_digest;` in `src/acpx/mod.rs`.
{RUST_GROK_ENV}
Existing interfaces:
- `crate::acpx::bundle::{{fmt_duration, ActionReceipt, FlowDef, FlowEdge, FlowMount, FlowNode, NodeKind, NodeState, Outcome, Run, RunStatus, Step}}` (`bundle.rs#L1-345` shows the pre-C3 `FlowNode` and `FlowDef`; code against the post-C3 `FlowNode`, `FlowMount`, and `FlowDef` shown in `STEP_DIGEST_SHARED_API` below).
- `crate::acpx::step_evidence::{{digest_step_files, extract_step_validations_with_outcome, read_attempt_tool_calls, step_trace_link, MergedToolCall, StepFileDigest, StepTraceLink, ValidationDigest}}`.

Public surface the coordinator and sibling slices will call:
```rust
{STEP_DIGEST_SHARED_API}
```

Behavior details:
- Re-exports and wrapper:
  - `pub use crate::acpx::step_evidence::{{digest_step_files, extract_step_validations_with_outcome, read_attempt_tool_calls, step_trace_link, MergedToolCall, StepFileDigest, StepTraceLink, ValidationDigest}};`
  - `pub fn extract_step_validations(run: &Run, step: &Step, observed_tools: &[&str]) -> ValidationDigest {{ extract_step_validations_with_outcome(run, step, observed_tools, effective_outcome(step)) }}`
- `effective_outcome`, `non_zero_exit_code`, `trim_failure_tail`, `step_failure_reason`, `latest_run_failure`:
  - `non_zero_exit_code(step)`: checks `step.action.as_ref().and_then(|a| a.exit_code).filter(|&c| c != 0)` first, then `step.output.get("exitCode").and_then(Value::as_i64).filter(|&c| c != 0)`.
  - `effective_outcome(step)`: if `step.outcome != Outcome::Ok`, return `step.outcome`; if `non_zero_exit_code(step).is_some()` or `step.output.get("ok").and_then(Value::as_bool) == Some(false)`, return `Outcome::Failed`; else `Outcome::Ok`.
  - For `Outcome::TimedOut` when `trim_failure_tail` finds no text, `step_failure_reason` returns `Some(format!("timed out after {{}}", fmt_duration(step.duration_ms())))`.
- `active_branch_nodes(start_at, node_ids, edges, visited)`:
  - Walk forward from `start_at`. At a node with several outgoing edges, follow only edges whose target is in `visited` when at least one target is in `visited`; when none is visited yet (decision not made), keep every branch. Return the reachable set.
  - Example: nodes `route_host -> {{resolve_group1_air, resolve_group2_shell}}`, `visited = {{route_host, resolve_group2_shell}}` drops `resolve_group1_air`; with `visited = {{route_host}}` both branches stay.
- (a) `step_attempt_budget(run: &Run, step: &Step) -> NodeAttemptBudget`:
  - `observed_attempts` = number of `run.steps` with `s.node_id == step.node_id`; `attempt` = 1-based position of `step.attempt_id` among them. Attempt ids are UUIDs: never parse them. If `step.attempt_id` is not in `run.steps`, `attempt = observed_attempts + 1`.
  - `badge_label(&self)`: `Some(format!("attempt {{}}", self.attempt))` when `self.attempt > 1 || self.observed_attempts > 1`, else `None`.
- (c) `extract_step_verdict(run: &Run, step_index: usize) -> Option<StepVerdict>`:
  - Match `FlowEdge::Switch {{ from, on, cases }}` for `step.node_id` against `step.output` (resolving `on` stripped of `"$."`) or the next executed step `run.steps.get(step_index + 1)`, or explicit `"verdict"` / `"decision"` string fields in `step.output`.
  - Extract `reason` from keys `["reason", "rationale", "summary", "explanation", "detail", "notes"]` in `step.output` (skipping `"summary"` on `NodeKind::Checkpoint`), collapsed to one line of at most 160 chars.
  - `StepVerdict::summary_line(&self)`:
    - Start with `match &self.target_node {{ Some(t) => format!("verdict: {{}} -> {{t}}", self.route_key), None => format!("verdict: {{}}", self.route_key) }}`.
    - If let `Some(r) = &self.reason`, append `format!(" ({{r}})")`.
- (d) `step_time_budget(run: &Run, step: &Step, now_ms: i64) -> StepTimeBudget`:
  - `elapsed_ms`: if `run.summary.status == RunStatus::Running && run.summary.current_node.as_deref() == Some(&step.node_id) && step.finished_ms <= step.started_ms && now_ms > step.started_ms`, `(now_ms - step.started_ms) as u64`, else `step.duration_ms()`.
  - `timeout_ms`: the step's `FlowNode::timeout_ms`, nothing else.
  - `timed_out`: `effective_outcome(step) == Outcome::TimedOut`.
  - `near_expiry`: `!timed_out && timeout_ms.is_some_and(|t| t > 0 && elapsed_ms.saturating_mul(100) / t >= 75)`.
  - `StepTimeBudget::badge_label(&self) -> String`:
    - Let `e = fmt_duration(self.elapsed_ms)`.
    - Match `(self.timed_out, self.timeout_ms, self.near_expiry)`:
      - `(true, Some(t), _)` -> `format!("TIMED OUT {{e}}/{{}}", fmt_duration(t))`
      - `(true, None, _)` -> `format!("TIMED OUT {{e}}")`
      - `(false, Some(t), true)` -> `format!("⚠ {{e}}/{{}} ({{}}%)", fmt_duration(t), (self.elapsed_ms.saturating_mul(100) / t.max(1)).min(100))`
      - `(false, Some(t), false)` -> `format!("{{e}}/{{}}", fmt_duration(t))`
      - `(false, None, _)` -> `e`
- (e) `run_step_budget(run: &Run) -> StepBudgetSummary`:
  - Uses the engine's `countExecutableSteps` rule (`engine.ts:564-573, 1707-1715`): count `step` in `run.steps` where the node is **not** an include transition step (`run.flow.nodes.iter().find(|n| n.id == step.node_id).and_then(|n| n.include_transition.as_deref())` is neither `Some("entry")` nor `Some("exit")`, and `parse_scoped_node_id(&step.node_id).2.is_none()`, and `!run.flow.mounts.iter().any(|m| m.mount_path.join("/") == step.node_id)`).
  - `max_steps = run.flow.max_steps`.
  - `remaining_steps = max_steps.map(|m| m.saturating_sub(used_steps))`.
- (f) `parse_scoped_node_id(node_id) -> (Option<&str>, &str, Option<&str>)`:
  - Split at the last `'/'` (`mount` is `None` without one); `exit` is `Some(name)` when `local` starts with `__piw_exit_`.
- `summarize_included_workflows(run: &Run) -> Vec<IncludeScopeSummary>`:
  - Group `run.flow.nodes` with non-empty `mount_path` by `mount_path.join("/")` (in BTreeMap order).
  - Look up `max_steps` from `run.flow.mounts.iter().find(|m| m.mount_path.join("/") == mount_key).and_then(|m| m.max_steps)`. Do NOT manufacture a cap when `max_steps` is `None`.
  - A node in the group is a transition node if `node.include_transition.as_deref()` is `Some("entry")` or `Some("exit")` (or `node.id == mount_key` or `parse_scoped_node_id(&node.id).2.is_some()`).
  - `total_child_nodes`: nodes in the group; `executed_child_nodes`: distinct node ids of the group seen in `run.steps`.
  - `invocations`: count of `run.steps` on the group's `include_transition == Some("entry")` node (defaulting to `1` if `executed_child_nodes > 0` and no explicit entry node was in `run.flow.nodes`).
  - `used_steps`: matching `assertInvocationStepLimit` (`engine.ts:1717-1752`), find the last index in `run.steps` of the group's `"entry"` transition step; in the slice of `run.steps` after that entry index (or all `run.steps` if no entry step was recorded), count non-transition steps belonging to the group (`step.node_id.starts_with(&format!("{{mount_key}}/"))` or in the group's node ids).
  - `remaining_steps = max_steps.map(|m| m.saturating_sub(used_steps))`.
  - `active`: `run.summary.current_node.as_deref()` is in the group.
  - `named_exit`: `local_node_id` (or `parse_scoped_node_id` exit name fallback) of the latest executed group node whose `include_transition == Some("exit")`.
  - `summary_line(&self)`:
    - Start with `format!("{{}}: {{}}/{{}} nodes", self.mount_path, self.executed_child_nodes, self.total_child_nodes)`.
    - If `self.max_steps` is `Some(max)`: append `format!(" · {{}}/{{max}} steps ({{}} left)", self.used_steps, self.remaining_steps.unwrap_or(0))`.
    - Else: append `format!(" · {{}} steps", self.used_steps)`.
    - If `self.invocations > 1`: append `format!(" · {{}}x", self.invocations)`.
    - If `self.active`: append `" · active"`.
    - Else if let `Some(exit) = &self.named_exit`: append `format!(" · exit:{{exit}}")`.

Tests:
- `effective_outcome` catches `exit_code: Some(101)` and preserves `Outcome::TimedOut`.
- `active_branch_nodes` drops the untaken branch after the decision and keeps both branches before it (example above).
- `step_attempt_budget` formats `"attempt 2"` for the second of two `cargo_check` steps with UUID attempt ids.
- `extract_step_verdict` extracts `route_key: "needs_work"`, `target_node: Some("repair")`, and `reason`.
- `step_time_budget` reads `FlowNode::timeout_ms`, detects `near_expiry: true` at `90_000 / 120_000 ms`, `timed_out: true` on `Outcome::TimedOut`, and `timeout_ms: None` when the node has none.
- `run_step_budget` and `summarize_included_workflows` test:
  1. a non-default root `FlowDef::max_steps = Some(320)` where `run.steps` includes `"entry"` and `"exit"` transition steps alongside 5 executable steps, verifying `run_step_budget` excludes the transition steps (`used_steps: 5`, `max_steps: Some(320)`, `remaining_steps: Some(315)`),
  2. an included workflow mount WITH its own cap (`FlowMount {{ mount_path: vec!["verify".into()], workflow_name: "change-verification".into(), max_steps: Some(30) }}`) reporting `used_steps: 2`, `max_steps: Some(30)`, `remaining_steps: Some(28)`, and `named_exit: Some("passed")` from `verify/__piw_exit_passed` (`include_transition: Some("exit")`, `local_node_id: Some("passed")`), and
  3. an included workflow mount WITHOUT its own cap (`FlowMount {{ mount_path: vec!["approval".into()], workflow_name: "plan-approval".into(), max_steps: None }}`) reporting `max_steps: None`, `remaining_steps: None`, and no manufactured cap in `summary_line()`.
""" + RETURN_FORMAT_TEMPLATE.format(target="crates/codegen/xai-grok-pager/src/acpx/step_digest.rs"),
    },
    {
        "custom_id": "workflow_preflight",
        "output_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/workflow_preflight.rs",
        ],
        "context_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L1-345",
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L555-590",
        ],
        "prompt": f"""
Slice id: workflow_preflight
Target file: crates/codegen/xai-grok-pager/src/acpx/workflow_preflight.rs
Goal: Provide (1) the workflow creator entry point that indexes existing workflows and past run records into reusable starting templates, (2) cross-run per-step duration and failure analysis (`analyze_workflow_history`) so time limits are proposed from actual past step durations in workflow state rather than guessed, and (3) the preflight budget & loop gate (`audit_workflow_preflight`) that flags `beta8-merge.workflow.ts` for ungrounded/missing time limits and its unbounded `repair_compile -> cargo_check` loop before launch.

Ownership:
Return this complete file only. Do not modify other files, add dependencies, or redefine `Run`, `Step`, `FlowDef`, `FlowMount`, `FlowNode`, `FlowEdge`, or `Outcome` (import them from `crate::acpx::bundle::*` and use `crate::acpx::step_digest::*`). The coordinator owns registering `pub mod workflow_preflight;` in `src/acpx/mod.rs` and wiring it into `/flow` preflight in `grok-acpx-acp` and `/acpx`.
{RUST_GROK_ENV}
Existing interfaces:
- `crate::acpx::bundle::{{fmt_duration, FlowDef, FlowEdge, FlowMount, FlowNode, NodeKind, Outcome, Run, RunStatus, Step}}` (`bundle.rs#L1-345`, `#L555-590`; use the post-C3 `FlowNode`, `FlowMount`, and `FlowDef` fields in `STEP_DIGEST_SHARED_API` below).
- Shared `crate::acpx::step_digest` contract:
```rust
{STEP_DIGEST_SHARED_API}
```

Public surface the coordinator will call:
```rust
use std::collections::BTreeMap;
use std::path::PathBuf;
use serde_json::Value;
use crate::acpx::bundle::{{FlowDef, Run}};

/// Historical duration and failure statistics for one node across recorded runs of the same workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepHistoryStats {{
    pub node_id: String,
    pub executions: usize,
    pub failures: usize,
    pub timeouts: usize,
    pub min_duration_ms: u64,
    pub median_duration_ms: u64,
    pub max_duration_ms: u64,
    /// History-backed recommended `timeoutMs`: `max_duration_ms.saturating_mul(2)` rounded up to the next 30_000ms
    /// (minimum 60_000ms), or `None` when `executions == 0`.
    pub recommended_timeout_ms: Option<u64>,
}}

/// One reusable workflow template entry discovered from recorded runs and workflow files on disk
/// (the v1 entry point for creating or cloning a workflow from past work).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTemplateEntry {{
    pub flow_name: String,
    pub source_path: Option<PathBuf>,
    pub node_count: usize,
    pub has_loops: bool,
    pub recorded_runs: usize,
    pub completed_runs: usize,
    pub failed_runs: usize,
    pub last_run_id: Option<String>,
    pub median_run_duration_ms: Option<u64>,
}}

/// User-supplied or soft-prompted preflight budget & loop overrides applied before launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreflightOverrides {{
    /// Per-node `timeoutMs` overrides (`node_id -> timeout_ms`).
    pub node_timeouts_ms: BTreeMap<String, u64>,
    /// Per-cycle loop attempt limits (`"<from>-><to>"` or `from` node id -> max attempts).
    pub loop_limits: BTreeMap<String, usize>,
    /// Root workflow `maxSteps` override.
    pub max_steps: Option<usize>,
    /// Free-form operator steering notes attached during preflight co-design.
    pub steering_notes: Vec<String>,
}}

/// Specific preflight issue that blocks or warns before launching a workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightIssueKind {{
    /// A back-edge `from -> to` forms a cycle with no loop attempt bound in the definition or `overrides.loop_limits`.
    UnboundedLoop {{
        from: String,
        to: String,
    }},
    /// A node has no `timeoutMs` in the definition or `overrides.node_timeouts_ms`.
    MissingNodeTimeout {{
        node_id: String,
        suggested_ms: Option<u64>,
    }},
    /// A node's configured `timeoutMs` is smaller than or within 10% of its historical `max_duration_ms` (likely a guess).
    UnderbudgetedTimeout {{
        node_id: String,
        configured_ms: u64,
        historical_max_ms: u64,
        suggested_ms: u64,
    }},
    /// Root workflow has no `maxSteps` in the definition or `overrides.max_steps`.
    MissingMaxSteps,
}}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightIssue {{
    /// True when this issue must be filled or explicitly accepted before launch (`UnboundedLoop`, `MissingNodeTimeout`, `MissingMaxSteps`).
    pub blocking: bool,
    pub kind: PreflightIssueKind,
    pub message: String,
}}

/// Complete preflight audit for a workflow definition against past run records and current overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {{
    pub flow_name: String,
    pub node_count: usize,
    pub effective_max_steps: Option<usize>,
    /// Resolved `timeoutMs` per node (`node_id -> Option<u64>`).
    pub node_timeouts_ms: BTreeMap<String, Option<u64>>,
    /// Detected cycle back-edges `(from, to, resolved_limit)`.
    pub loop_edges: Vec<(String, String, Option<usize>)>,
    /// Per-node historical stats across `past_runs` of the same workflow.
    pub history: Vec<StepHistoryStats>,
    pub issues: Vec<PreflightIssue>,
    pub steering_notes: Vec<String>,
}}

impl PreflightReport {{
    /// True when no issue in `self.issues` has `blocking == true`.
    pub fn ready_to_launch(&self) -> bool;
}}

/// Analyze all executed steps across `past_runs` whose `run.summary.flow_name == flow_name` (or `run.flow.name == flow_name`),
/// returning `StepHistoryStats` sorted by `node_id` (or in `past_runs` first-appearance order).
/// Uses `crate::acpx::step_digest::effective_outcome(step)` so `allowFailure` steps with non-zero exit code count as failures!
pub fn analyze_workflow_history(flow_name: &str, past_runs: &[Run]) -> Vec<StepHistoryStats>;

/// Build the catalog of workflow templates from `past_runs` and discovered `workflow_files` on disk.
pub fn discover_workflow_templates(
    past_runs: &[Run],
    workflow_files: &[PathBuf],
) -> Vec<WorkflowTemplateEntry>;

/// Detect directed cycle back-edges `(from, to)` in `flow` using DFS from `flow.start_at` (plus any unvisited nodes in `flow.nodes` order).
pub fn find_cycle_back_edges(flow: &FlowDef) -> Vec<(String, String)>;

/// Audit `flow` (reading `node.timeout_ms`, `flow.max_steps`, and optional `raw_flow_json` for `"maxSteps"`,
/// `"nodes.<id>.timeoutMs"`, `"nodes.<id>.maxAttempts"`) against `past_runs` and `overrides`.
pub fn audit_workflow_preflight(
    flow: &FlowDef,
    raw_flow_json: Option<&Value>,
    past_runs: &[Run],
    overrides: &PreflightOverrides,
) -> PreflightReport;

/// Format `report` as a concise Markdown preflight card for `grok acpx` chat (showing steps, historical vs configured
/// timeouts, loop bounds, `maxSteps`, blocking issues, and how to reply in plain text to set budgets or launch).
pub fn format_preflight_markdown(report: &PreflightReport) -> String;
```

Tests:
- `find_cycle_back_edges` finds `("cargo_check", "repair_compile")` (or `("repair_compile", "cargo_check")`) on a `beta8-merge`-shaped graph with a repair cycle, and returns empty on a DAG.
- `analyze_workflow_history` aggregates durations and counts an `Outcome::Ok` step with `exit_code: Some(101)` as a failure (`failures: 1`) via `step_digest::effective_outcome`, computing `recommended_timeout_ms`.
- `audit_workflow_preflight` on a `beta8-merge`-shaped workflow without loop limits or node timeouts returns `!report.ready_to_launch()` with `UnboundedLoop`, `MissingNodeTimeout`, and `MissingMaxSteps`; when `UnderbudgetedTimeout` is triggered against `past_runs`, it proposes `suggested_ms` from history; and once `PreflightOverrides` supplies the missing values, `report.ready_to_launch()` becomes `true`.
- `discover_workflow_templates` merges `past_runs` and `workflow_files` into template entries with run counts and `has_loops`.
""" + RETURN_FORMAT_TEMPLATE.format(target="crates/codegen/xai-grok-pager/src/acpx/workflow_preflight.rs"),
    },
    {
        "custom_id": "run_path",
        "output_files": [
            "grok:crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/run_path.rs",
        ],
        "context_files": [
            "grok:crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs#L1-130",
            "grok:crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/project.rs#L1-115",
            "grok:crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/project.rs#L510-760",
        ],
        "prompt": f"""
Slice id: run_path
Target file: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/run_path.rs
Goal: Provide (1) top-down/active-branch Mermaid rendering for `RunStarted` using `step_digest::active_branch_nodes`, (2) the single-line taken-path progress breadcrumb for subsequent updates, (3) an inline waiting-gate transcript card with no auto-pop-up modal (`Issue 11`) that embeds the preceding failure tail from `step_digest::trim_failure_tail`, (4) fuzzy plain-text gate choice matching (`Issue 2`) so typing a choice in chat answers the gate while unmatched text is refused with the valid choices without cancelling the run, and (5) mid-run conversational steering classification (`classify_mid_run_input`) so typing during an active run steers (`pause`, `resume`, `queue-follow-up`, `change-settings`, `status`) instead of hitting `"A run is in progress; press Esc to cancel it."`.

Ownership:
Return this complete file only. Do not modify other files, add dependencies, or duplicate branch pruning or failure-tail trimming (call `xai_grok_pager::acpx::step_digest::{{active_branch_nodes, parse_scoped_node_id, trim_failure_tail, DEFAULT_FAILURE_MAX_CHARS, DEFAULT_FAILURE_MAX_LINES}}`). Import `FlowSketch`, `GateChoice`, `SketchEdge`, `SketchNode`, `StepKind` from `crate::driver`. The coordinator owns `mod run_path;` in `main.rs` and calling these functions from `project.rs` and `agent.rs`.
{RUST_GROK_ENV}
Existing interfaces:
- `crate::driver::{{FlowSketch, GateChoice, SketchEdge, SketchNode, StepKind}}` as shown in `driver.rs#L1-130`.
- Shared `xai_grok_pager::acpx::step_digest` contract:
```rust
{STEP_DIGEST_SHARED_API}
```

Public surface the coordinator will call:
```rust
use std::collections::HashMap;
use crate::driver::{{FlowSketch, GateChoice}};

/// Summary of the most recent failed (or non-zero exit) step leading into a gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecedingFailure {{
    pub node_id: String,
    pub exit_code: Option<i64>,
    pub detail: String,
}}

impl PrecedingFailure {{
    pub fn from_raw(
        node_id: &str,
        error: Option<&str>,
        stderr: Option<&str>,
        output_tail: Option<&str>,
        stdout: Option<&str>,
        exit_code: Option<i64>,
    ) -> Self;
}}

pub fn prune_flow_sketch(flow: &FlowSketch, states: &HashMap<String, char>) -> FlowSketch;
pub fn active_mermaid(flow: &FlowSketch, states: &HashMap<String, char>) -> String;
pub fn compact_path_line(
    flow: &FlowSketch,
    states: &HashMap<String, char>,
    exit_codes: &HashMap<String, i64>,
) -> String;
pub fn format_inline_waiting_gate(
    node: &str,
    question: &str,
    choices: &[GateChoice],
    failure: Option<&PrecedingFailure>,
) -> String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateTextMatch {{
    Matched {{
        label: String,
        notes: Option<String>,
    }},
    Refused {{
        refusal_message: String,
    }},
}}

pub fn match_waiting_gate_text(
    node: &str,
    choices: &[GateChoice],
    input: &str,
) -> GateTextMatch;

/// Live operational control action parsed from plain text or slash commands while a workflow run is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidRunCommand {{
    /// Show the live `/status` report for the active run (`"status"`, `"/status"`, `"?"`).
    Status,
    /// Pause the active run (`"pause"`, `"/pause"`).
    Pause,
    /// Resume a paused run (`"resume"`, `"/resume"`, `"continue"`).
    Resume,
    /// Update a scoped setting or budget mid-run (`"set <key>=<value>"` or `"/set <key>=<value>"`).
    ChangeSetting {{
        key: String,
        value: String,
    }},
    /// Queue a steering instruction / follow-up prompt for the run (`"steer: <prompt>"`, `"/steer <prompt>"`, or any non-empty natural-language steering text).
    QueueFollowUp {{
        prompt: String,
    }},
}}

/// Classify user input typed while a workflow run is active (replacing the hard `"A run is in progress; press Esc to cancel it."` block).
pub fn classify_mid_run_input(input: &str) -> Option<MidRunCommand>;
```

Behavior details:
- `prune_flow_sketch`, `active_mermaid`, `compact_path_line`, `format_inline_waiting_gate`, and `match_waiting_gate_text`:
  - Follow the contract above. `active_mermaid` uses `"flowchart LR"` only when the pruned flow has `<= 4` nodes and max out-degree `<= 1`, else `"flowchart TD"`.
- `classify_mid_run_input(input: &str) -> Option<MidRunCommand>`:
  - Trim `input`; return `None` if empty.
  - Strip optional leading `'/'`.
  - Case-insensitive `"status"` or `"?"` -> `Some(MidRunCommand::Status)`.
  - Case-insensitive `"pause"` -> `Some(MidRunCommand::Pause)`.
  - Case-insensitive `"resume"` or `"continue"` -> `Some(MidRunCommand::Resume)`.
  - Prefix `"set "` (case-insensitive) followed by `<key>=<value>` (or `<key> <value>`) with non-empty trimmed `key` and `value` -> `Some(MidRunCommand::ChangeSetting {{ key, value }})`.
  - Prefix `"steer:"` or `"steer "` (case-insensitive) -> `Some(MidRunCommand::QueueFollowUp {{ prompt: rest.trim().to_string() }})` (when `rest.trim()` is non-empty).
  - Any other non-empty text (that does not start with `'/'`) -> `Some(MidRunCommand::QueueFollowUp {{ prompt: trimmed.to_string() }})`.

Tests:
- `prune_flow_sketch` delegates to `step_digest::active_branch_nodes` and drops `"resolve_group1_air"` once `"route_host"` and `"resolve_group2_shell"` are in `states`.
- `active_mermaid` emits `flowchart LR` for a 3-node linear flow, `flowchart TD` for a 5-node flow, and formats `"verify/__piw_exit_passed"` as `"verify ↳ exit:passed"`.
- `compact_path_line` formats `✗ cargo_check (exit 101)` and `✓ verify↳exit:passed`.
- `format_inline_waiting_gate` renders the inline `"WAITING FOR YOUR ANSWER"` card with the preceding failure code block and numbered choices.
- `match_waiting_gate_text` matches `"repair"`, `"REPAIR"`, `"2"`, `"rep"`, and `"repair: fix duplicate key in Cargo.toml"`, and returns `GateTextMatch::Refused` on `"hello world"`.
- `classify_mid_run_input` maps `"/status"`, `"pause"`, `"resume"`, `"set timeoutMs=600000"`, and plain steering text `"focus on Cargo.toml duplicate keys"` to the corresponding `MidRunCommand` variants.
""" + RETURN_FORMAT_TEMPLATE.format(target="crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/run_path.rs"),
    },
    {
        "custom_id": "modal_view",
        "output_files": [
            "grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs",
        ],
        "context_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L1-330",
            "grok:crates/codegen/xai-grok-pager/src/acpx/graph.rs#L1-95",
            "grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs#L1-165",
            "grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs#L565-670",
            "grok:crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs#L1145-1290",
        ],
        "prompt": f"""
Slice id: modal_view
Target file: crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs
Goal: Declutter the `/acpx` modal run view by pruning untaken switch branches via `crate::acpx::step_digest::active_branch_nodes`, adding full-modal graph and detail maximize/zoom modes with vertical graph scrolling, rendering a clear waiting-gate banner with the preceding step's failure reason, and formatting per-step structural and trace-link badges for all six workflow features (a)-(f) plus `StepTraceLink`.

Ownership:
Return this complete file only. Do not modify other files, add dependencies, or duplicate branch-pruning or failure-tail trimming logic (call `crate::acpx::step_digest::*`). The coordinator owns registering `pub(crate) mod modal_view;` in `views/acpx_modal.rs` and wiring it into `views/acpx_modal/run.rs` and `detail.rs`.
{RUST_GROK_ENV}
Existing interfaces:
- `crate::acpx::bundle::{{FlowDef, FlowEdge, FlowMount, FlowNode, NodeKind, NodeState, Outcome, Run, RunStatus, Step}}` (`bundle.rs#L1-330`; use the post-C3 `FlowNode`, `FlowMount`, and `FlowDef` fields in `STEP_DIGEST_SHARED_API` below).
- `crate::render::line_utils::truncate_str` and `crate::theme::Theme`.
- Shared `crate::acpx::step_digest` contract:
```rust
{STEP_DIGEST_SHARED_API}
```

Public surface the coordinator will call:
```rust
use ratatui::layout::Rect;
use ratatui::style::{{Modifier, Style}};
use ratatui::text::{{Line, Span}};
use crate::acpx::bundle::Run;
use crate::theme::Theme;

/// Which pane of the `/acpx` single-run view is zoomed/maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalZoom {{
    #[default]
    Split,
    Graph,
    Detail,
}}

impl ModalZoom {{
    pub fn toggle_graph(self) -> Self;
    pub fn toggle_detail(self) -> Self;
    pub fn header_badge(self) -> Option<&'static str>;
}}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBodyLayout {{
    pub graph_rows: usize,
    pub show_separator: bool,
    pub stages_rect: Option<Rect>,
    pub detail_rect: Option<Rect>,
    pub vertical_sep_x: Option<u16>,
}}

/// Compute the layout below the header and banner at row `y_after_banner` within `area`.
///
/// Rules:
/// - Let `remaining_h = area.y.saturating_add(area.height).saturating_sub(y_after_banner)`. If `remaining_h == 0`, all rects are `None` and `graph_rows == 0`.
/// - When `zoom == ModalZoom::Graph`:
///   - `graph_rows = remaining_h as usize`, `show_separator = false`, `stages_rect = None`, `detail_rect = None`, `vertical_sep_x = None`.
/// - When `zoom == ModalZoom::Detail`:
///   - `graph_rows = 0`, `show_separator = false`, `stages_rect = None`,
///   - `detail_rect = Some(Rect::new(area.x, y_after_banner, area.width, remaining_h))`, `vertical_sep_x = None`.
/// - When `zoom == ModalZoom::Split`:
///   - Max graph budget is `if show_graph && area.height >= 16 {{ ((area.height as usize) * 2 / 5).max(1).min((remaining_h as usize).saturating_sub(3)) }} else {{ 0 }}`.
///   - `graph_rows = actual_graph_lines.min(max_graph_budget)`.
///   - `show_separator = true` when `remaining_h > (graph_rows as u16)`.
///   - Let `body_y = y_after_banner.saturating_add(graph_rows as u16).saturating_add(u16::from(show_separator))`.
///   - Let `body_h = area.y.saturating_add(area.height).saturating_sub(body_y)`.
///   - If `body_h == 0`, `stages_rect = None`, `detail_rect = None`, `vertical_sep_x = None`.
///   - Else if `area.width >= 110`:
///     `left_w = ((area.width as u32 * 2 / 5) as u16).clamp(44, 72)`,
///     `sep_x = area.x.saturating_add(left_w)`,
///     `detail_x = sep_x.saturating_add(1)`,
///     `detail_w = area.x.saturating_add(area.width).saturating_sub(detail_x)`,
///     `stages_rect = Some(Rect::new(area.x, body_y, left_w, body_h))`,
///     `detail_rect = Some(Rect::new(detail_x, body_y, detail_w, body_h))`,
///     `vertical_sep_x = Some(sep_x)`.
///   - Else (`area.width < 110`):
///     `stages_rect = Some(Rect::new(area.x, body_y, area.width, body_h))`, `detail_rect = None`, `vertical_sep_x = None`.
pub fn compute_run_body_layout(
    area: Rect,
    y_after_banner: u16,
    actual_graph_lines: usize,
    show_graph: bool,
    zoom: ModalZoom,
) -> RunBodyLayout;

/// Return a cloned `Run` whose `flow` has untaken branches pruned by calling
/// `crate::acpx::step_digest::active_branch_nodes`.
pub fn prune_run_for_graph(run: &Run) -> Run;

/// Slice a rendered graph (`lines`) to fit `budget_rows` at `scroll_y`, returning `(visible_lines, clamped_scroll_y)`.
pub fn slice_graph_viewport(
    lines: &[Line<'static>],
    budget_rows: usize,
    scroll_y: usize,
    zoom: ModalZoom,
    width: usize,
    theme: &Theme,
) -> (Vec<Line<'static>>, usize);

/// Build the multi-line banner for a waiting gate in `/acpx`, automatically querying
/// `crate::acpx::step_digest::latest_run_failure(run)` for the preceding failure summary.
pub fn build_waiting_gate_banner(
    run: &Run,
    node: &str,
    question: Option<&str>,
    choice_count: usize,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>>;

/// Build a compact structural + trace-link annotation string for `run.steps[step_index]` at `now_ms`, combining:
/// - (f) `format!("exit:{{exit_name}}")` if the step's `FlowNode` has `include_transition.as_deref() == Some("exit")`
///   and `local_node_id.as_deref()` (or `parse_scoped_node_id(&step.node_id).2`) is `Some(exit_name)`,
/// - (a) `step_attempt_budget(run, step).badge_label()`,
/// - (d) `step_time_budget(run, step, now_ms).badge_label()` when `timeout_ms.is_some()` or `timed_out`,
/// - `"exit <code>"` when `non_zero_exit_code(step)` is `Some(code)`,
/// - (b) `extract_step_validations(run, step, &[]).badge_label()`,
/// - (c) `extract_step_verdict(run, step_index).map(|v| v.summary_line())`,
/// - `digest_step_files(run, step, &[]).summary_line()` when non-empty,
/// - `step_trace_link(run, step).badge_label()` when `Some`.
/// Joins non-empty parts with `" · "`.
pub fn format_step_structure_annotation(run: &Run, step_index: usize, now_ms: i64) -> String;
```

Tests:
- `ModalZoom::toggle_graph` and `toggle_detail` transition cleanly between `Split`, `Graph`, and `Detail`.
- `compute_run_body_layout` gives 100% of remaining height to the graph in `ModalZoom::Graph`, 100% of width/height to `detail_rect` in `ModalZoom::Detail`, and a 2/5 + 3/5 split in `ModalZoom::Split` at width 120.
- `prune_run_for_graph` removes the unvisited `"air"` switch case and its unreachable node once `route_host` has transitioned to `"mini"` in `run.steps`.
- `slice_graph_viewport` scrolls vertically in `ModalZoom::Graph` and appends the `"press m to maximize graph"` hint in `ModalZoom::Split`.
- `build_waiting_gate_banner` renders both the `"WAITING FOR YOUR ANSWER"` warning line and the `"✗ Preceding failure in cargo_check (exit 101): ..."` error line.
- `format_step_structure_annotation` combines loop attempt badge, time budget, validator rejection badge (from a temp `events.ndjson` with a rejected-then-accepted workflow submission), reviewer verdict, file progress, and `step_trace_link` badge (`"trace: events 1..5"`).
""" + RETURN_FORMAT_TEMPLATE.format(target="crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs"),
    },
    {
        "custom_id": "run_status_bar",
        "output_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/run_status_bar.rs",
        ],
        "context_files": [
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L1-345",
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L555-590",
            "grok:crates/codegen/xai-grok-pager/src/acpx/bundle.rs#L1255-1275",
        ],
        "prompt": f"""
Slice id: run_status_bar
Target file: crates/codegen/xai-grok-pager/src/acpx/run_status_bar.rs
Goal: Provide a single-line live run status bar and a structured multi-line `/status` report (Issue 12 + all six structural items (a)-(f) + `step_trace_link` transcript/artifact badge) inside Grok Build without querying SQLite or the server CLI.

Ownership:
Return this complete file only. Do not modify other files, add dependencies, or duplicate outcome/failure/file-digest/structure logic (use `crate::acpx::step_digest::*`). The coordinator owns registering `pub mod run_status_bar;` in `src/acpx/mod.rs` and wiring it into `grok-acpx-acp` (`/status` and turn status updates) and `/acpx`.
{RUST_GROK_ENV}
Existing interfaces:
- `crate::acpx::bundle::{{fmt_duration, gate_choices, FlowDef, FlowEdge, FlowMount, FlowNode, NodeKind, NodeState, Outcome, Run, RunStatus, Step}}` (`bundle.rs#L1-345`, `#L555-590`, `#L1255-1275`; use the post-C3 `FlowNode`, `FlowMount`, and `FlowDef` fields in `STEP_DIGEST_SHARED_API` below).
- Shared `crate::acpx::step_digest` contract:
```rust
{STEP_DIGEST_SHARED_API}
```

Public surface the coordinator will call:
```rust
use crate::acpx::bundle::Run;

pub fn format_run_status_line(run: &Run, answered: Option<&str>, now_ms: i64) -> String;
pub fn format_run_status_report(run: &Run, answered: Option<&str>, now_ms: i64) -> String;
```

Behavior details:
- `format_run_status_line(run: &Run, answered: Option<&str>, now_ms: i64) -> String`:
  - Let `title` be `run.summary.title` if non-empty, else `run.summary.flow_name`, else `run.summary.run_id`.
  - Let `(done, total) = run.progress()` and `dur = fmt_duration(run.duration_ms(now_ms))`.
  - Let `step_budget = run_step_budget(run).badge_label()` (`Some("steps X/Y (Z left)")` when `max_steps` is known, or `Some("steps X/?")` when unknown).
  - When `run.summary.status == RunStatus::Waiting` and `answered.is_none()`:
    - Let `gate_node = run.summary.waiting_on.as_deref().or(run.summary.current_node.as_deref()).unwrap_or("gate")`.
    - Collect choice labels from `bundle::gate_choices(run)` as `"1: <c1>, 2: <c2>"`.
    - Start with `format!("◐ {{title}} · {{done}}/{{total}} steps")`, append `format!(" ({{b}})")` if `step_budget` is `Some(b)`,
      then `format!(" · {{dur}} · WAITING FOR YOUR ANSWER at {{gate_node}}")`.
    - If choices are non-empty, append `format!(" [{{choices}}]")`.
    - If `latest_run_failure(run)` is `Some((fail_node, code_opt, _))`:
      match `code_opt` -> `Some(c)` appends `format!(" · last failed: {{fail_node}} (exit {{c}})")`, `None` appends `format!(" · last failed: {{fail_node}}")`.
  - When `run.summary.status == RunStatus::Running`:
    - Let `curr = run.summary.current_node.as_deref().unwrap_or("…")`.
    - Start with `format!("● {{title}} · {{done}}/{{total}} steps")`, append `format!(" ({{b}})")` if `step_budget` is `Some(b)`,
      then `format!(" · {{dur}} · running {{curr}}")`.
    - If `run.steps.iter().rev().find(|s| s.node_id == curr)` is `Some(step)`:
      if let `Some(agent) = step.agent_name.as_deref().filter(|a| !a.is_empty())`, append `format!(" ({{agent}})")`;
      if let `Some(badge) = step_attempt_budget(run, step).badge_label()`, append `format!(" · {{badge}}")`;
      let `tb = step_time_budget(run, step, now_ms)`;
      if `tb.timeout_ms.is_some() || tb.timed_out`, append `format!(" · {{}}", tb.badge_label())`;
      if let `Some(vbadge) = extract_step_validations(run, step, &[]).badge_label()`, append `format!(" · {{vbadge}}")`;
      let `fline = digest_step_files(run, step, &[]).summary_line()`;
      if `!fline.is_empty()`, append `format!(" · {{fline}}")`.
  - Otherwise (Completed, Failed, TimedOut, Unknown, or Waiting with `answered.is_some()`):
    - Effective status is `RunStatus::Completed` when `run.summary.status == RunStatus::Waiting && answered.is_some()`, else `run.summary.status`.
    - Status label is `"TIMED OUT"` when `eff_status == RunStatus::TimedOut`, else `eff_status.label()`.
    - Start with `format!("{{}} {{title}} · {{done}}/{{total}} steps")`, append `format!(" ({{b}})")` if `step_budget` is `Some(b)`,
      then `format!(" · {{dur}} · {{status_label}}")`.
    - If `eff_status` is `Failed` or `TimedOut`, and `latest_run_failure(run)` is `Some((fail_node, code_opt, _))`:
      match `code_opt` -> `Some(c)` appends `format!(" at {{fail_node}} (exit {{c}})")`, `None` appends `format!(" at {{fail_node}}")`.
- `format_run_status_report(run: &Run, answered: Option<&str>, now_ms: i64) -> String`:
  - Line 1: `format!("**{{}}** (`{{}}`)", format_run_status_line(run, answered, now_ms), run.summary.run_id)`
  - If `summarize_included_workflows(run)` is non-empty:
    append a `"\\n**Included workflows:**"` section with one bullet `"- {{inc.summary_line()}}"` per included workflow.
  - Next section: one bullet per executed step `(idx, step)` in `run.steps.iter().enumerate()` including effective glyph (`✓`, `⏱`, `✗`, `⊘`, `?`), display node name (`mount ↳ exit:name` for included exits), attempt badge, agent name, `step_time_budget(run, step, now_ms).badge_label()`, `(exit <code>)` when non-zero, validator rejection badge (`extract_step_validations(run, step, &[]).badge_label()`), reviewer verdict summary line, file progress summary line (`digest_step_files(run, step, &[]).summary_line()`), and `step_trace_link(run, step).badge_label()` when present.
  - Waiting gate section when `run.summary.status == RunStatus::Waiting && answered.is_none()`.

Tests:
- `format_run_status_line` for a running workflow with `FlowDef::max_steps = Some(20)` on the second attempt of a loop step whose `FlowNode::timeout_ms` is `Some(120_000)` shows `"steps 2/20 (18 left)"`, `"attempt 2"`, `"⚠ 1m30s/2m00s (75%)"`, and file progress.
- `format_run_status_line` for a waiting workflow after a failed `cargo_check` (`exitCode: 101`) shows `"WAITING FOR YOUR ANSWER at final_review_gate [1: accept, 2: repair] · last failed: cargo_check (exit 101)"`.
- `format_run_status_report` includes `"Included workflows:"` (with an included mount cap `"5/30 steps (25 left)"`), per-step attempt counts, time budgets, validator rejection badge, reviewer verdicts, `step_trace_link` badges, effective glyphs (`⏱` for `TimedOut`, `✗` for `exitCode: 101`), and waiting gate details.
""" + RETURN_FORMAT_TEMPLATE.format(target="crates/codegen/xai-grok-pager/src/acpx/run_status_bar.rs"),
    },
    {
        "custom_id": "pane_zoom",
        "output_files": [
            "piw:tui/src/ui/pane_zoom.rs",
        ],
        "context_files": [
            "piw:tui/src/ui/mod.rs#L55-70",
            "piw:tui/src/ui/mod.rs#L290-335",
            "piw:tui/src/ui/mod.rs#L1840-1908",
            "piw:tui/src/ui/mod.rs#L2160-2210",
            "piw:tui/src/ui/help.rs#L1-115",
        ],
        "prompt": f"""
Slice id: pane_zoom
Target file: tui/src/ui/pane_zoom.rs
Goal: Add keyboard pane maximize/zoom (`m` / `M` to maximize the focused Graph, Inspector, or Runs pane to 100% of the body area, `Z` to directly maximize the Graph pane, `Esc` to restore split) and keyboard split resizing (`Alt+j` / `Alt+k` or `Ctrl+Down` / `Ctrl+Up` to grow/shrink the graph vs inspector split and full sidebar hide) to the `piw` TUI viewer.

Ownership:
Return this complete file only. Do not modify other files or add dependencies. The coordinator owns registering `mod pane_zoom;` in `tui/src/ui/mod.rs`, wiring `PaneLayoutState` into `App`, and adding the key bindings to `help.rs`.

Environment:
- Rust 2021/2024 edition (`piw` crate in `tui/`).
- Uses only `std`, `crossterm::event::{{KeyCode, KeyEvent, KeyModifiers}}`, and `ratatui::layout::{{Constraint, Direction, Layout, Rect}}`.
- No `.unwrap()` or `.expect()` in non-test code.

Existing interfaces:
- Layout constants and helpers in `tui/src/ui/mod.rs#L55-70`, `#L1840-1908`, `#L2160-2210`.

Public surface the coordinator will call:
```rust
use crossterm::event::{{KeyCode, KeyEvent, KeyModifiers}};
use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {{
    Runs,
    Graph,
    Inspector,
}}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZoomMode {{
    #[default]
    Split,
    Maximized(PaneId),
}}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BodyRects {{
    pub runs_rect: Rect,
    pub main_rect: Rect,
    pub graph_rect: Rect,
    pub inspector_rect: Rect,
    pub sidebar_collapsed: bool,
}}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomKeyAction {{
    ToggleMaximizeFocused,
    ToggleMaximizeGraph,
    RestoreSplit,
    GrowGraph(u16),
    ShrinkGraph(u16),
}}

/// Classify a key event into a `ZoomKeyAction` when applicable.
///
/// Rules:
/// - `KeyCode::Char('m') | KeyCode::Char('M')` (without CONTROL/ALT) -> `Some(ZoomKeyAction::ToggleMaximizeFocused)`
/// - `KeyCode::Char('Z')` (without CONTROL/ALT) -> `Some(ZoomKeyAction::ToggleMaximizeGraph)`
/// - `KeyCode::Esc` when `zoom != ZoomMode::Split` -> `Some(ZoomKeyAction::RestoreSplit)`
/// - `KeyCode::Char('j')` with `ALT`, or `KeyCode::Down` with `CONTROL` or `ALT` -> `Some(ZoomKeyAction::GrowGraph(3))`
/// - `KeyCode::Char('k')` with `ALT`, or `KeyCode::Up` with `CONTROL` or `ALT` -> `Some(ZoomKeyAction::ShrinkGraph(3))`
/// - Otherwise -> `None`
pub fn classify_zoom_key(key: KeyEvent, zoom: ZoomMode) -> Option<ZoomKeyAction>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneLayoutState {{
    pub zoom: ZoomMode,
    pub graph_height_delta: i16,
    pub sidebar_hidden: bool,
}}

impl Default for PaneLayoutState {{
    fn default() -> Self;
}}

impl PaneLayoutState {{
    pub fn apply_action(&mut self, action: ZoomKeyAction, focused: PaneId);
    pub fn toggle_sidebar_hidden(&mut self);
    pub fn status_badge(&self) -> Option<&'static str>;
    pub fn compute_body_rects(&self, body: Rect, runs_width: u16, inspector_height: u16) -> BodyRects;
}}
```

Tests:
- `classify_zoom_key` maps `'m'`, `'Z'`, `Esc` (only when maximized), `Alt+j`, and `Ctrl+Up` as specified.
- `PaneLayoutState::apply_action` toggles `ZoomMode::Maximized(PaneId::Graph)` on and off and restores `ZoomMode::Split`.
- `compute_body_rects` gives 100% of `body` to `graph_rect` (and `Rect::default()` to `runs_rect` and `inspector_rect`) when `ZoomMode::Maximized(PaneId::Graph)`.
- `compute_body_rects` hides the sidebar (`runs_rect.width == 0`, `main_rect.width == body.width`) when `sidebar_hidden == true`.
- `compute_body_rects` grows `graph_rect.height` and shrinks `inspector_rect.height` when `graph_height_delta > 0` while keeping `inspector_rect.height >= 8` and `graph_rect.height >= 8`.
""" + RETURN_FORMAT_TEMPLATE.format(target="tui/src/ui/pane_zoom.rs"),
    },
]
