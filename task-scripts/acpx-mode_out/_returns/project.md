```rust
use std::collections::HashMap;
use std::path::Path;

use agent_client_protocol as acp;
use serde_json::{Value, json};

use crate::driver::{DriverEvent, FlowSketch, SketchNode, StepKind, fmt_duration};

/// One thing to send to the pager. The agent adds `_meta` (timing, promptId, isReplay, eventId) when it sends.
#[derive(Debug, Clone)]
pub enum Out {
    /// A standard `session/update` notification for `session_id` (the parent session or a step's child session).
    Update {
        session_id: String,
        update: acp::SessionUpdate,
    },
    /// An xAI extension session update: sent as ext notification `x.ai/session_notification` (live) or
    /// `x.ai/session/update` (replay) with params `{ "sessionId": session_id, "update": update, "_meta": {...} }`.
    /// `update` is the JSON object with its "sessionUpdate" tag, e.g. subagent_spawned / subagent_finished.
    XaiUpdate { session_id: String, update: Value },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanNodeState {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Default)]
struct AttemptState {
    child_session_id: String,
    final_message: String,
    tool_calls: u32,
}

#[derive(Debug, Clone, Default)]
struct ShellAttemptState {
    command_line: String,
    cwd: Option<String>,
    prepared: bool,
}

pub struct Projector {
    parent_session_id: String,
    run_id: String,
    title: String,
    started_at_ms: i64,
    flow: Option<FlowSketch>,
    plan_states: HashMap<String, PlanNodeState>,
    attempts: HashMap<String, AttemptState>,
    shell_attempts: HashMap<String, ShellAttemptState>,
    parent_after_block: bool,
}

impl Projector {
    /// `parent_session_id`: the pager session this run is shown in.
    pub fn new(parent_session_id: &str) -> Self {
        Self {
            parent_session_id: parent_session_id.to_string(),
            run_id: String::new(),
            title: String::new(),
            started_at_ms: 0,
            flow: None,
            plan_states: HashMap::new(),
            attempts: HashMap::new(),
            shell_attempts: HashMap::new(),
            parent_after_block: false,
        }
    }

    /// Map one driver event to the updates that show it. Deterministic: same events -> same Outs (ids derive from
    /// run id + attempt id, never from clocks or randomness).
    pub fn on_event(&mut self, ev: &DriverEvent) -> Vec<Out> {
        match ev {
            DriverEvent::RunStarted {
                run_id,
                title,
                flow_path,
                flow,
                at_ms,
                ..
            } => {
                self.run_id = run_id.clone();
                self.title = title.clone();
                self.started_at_ms = *at_ms;
                self.flow = Some(flow.clone());
                self.plan_states.clear();
                for node in &flow.nodes {
                    self.plan_states
                        .insert(node.id.clone(), PlanNodeState::Pending);
                }
                self.parent_after_block = false;

                let flow_file_name = Path::new(flow_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(flow_path.as_str());

                let states = HashMap::new();
                let chart = mermaid(flow, &states);
                let text = format!(
                    "**▶ {title}**\n`{flow_file_name}` · {run_id}\n```mermaid\n{chart}\n```\n\n"
                );

                let mut outs = vec![self.parent_message(&text)];
                if let Some(plan_out) = self.build_plan_out() {
                    outs.push(plan_out);
                }
                outs
            }

            DriverEvent::StepStarted {
                node,
                attempt,
                kind,
                ..
            } => match kind {
                StepKind::Checkpoint => Vec::new(),
                StepKind::Acp => {
                    let mut outs = Vec::new();
                    if self.set_node_state(node, PlanNodeState::InProgress)
                        && let Some(plan_out) = self.build_plan_out()
                    {
                        outs.push(plan_out);
                    }

                    let child_sid = format!("acpx-step:{}:{attempt}", self.run_id);
                    let subagent_id = format!("acpx-sub:{}:{attempt}", self.run_id);
                    let tool_id = format!("acpx-tool:{}:{attempt}", self.run_id);

                    self.attempts.insert(
                        attempt.clone(),
                        AttemptState {
                            child_session_id: child_sid.clone(),
                            final_message: String::new(),
                            tool_calls: 0,
                        },
                    );

                    let raw_input = json!({
                        "variant": "Task",
                        "task_id": subagent_id,
                        "run_in_background": false,
                        "reconcile": false,
                    });
                    let tc = acp::ToolCall::new(acp::ToolCallId::new(tool_id), "Task".to_string())
                        .kind(acp::ToolKind::Other)
                        .status(acp::ToolCallStatus::InProgress)
                        .content(Vec::new())
                        .locations(Vec::new())
                        .raw_input(Some(raw_input));
                    outs.push(Out::Update {
                        session_id: self.parent_session_id.clone(),
                        update: acp::SessionUpdate::ToolCall(tc),
                    });

                    let (description, model) = self.acp_step_desc_and_model(node);
                    let mut spawned = json!({
                        "sessionUpdate": "subagent_spawned",
                        "subagent_id": subagent_id,
                        "parent_session_id": self.parent_session_id,
                        "child_session_id": child_sid,
                        "subagent_type": "acpx step",
                        "description": description,
                    });
                    if let Some(m) = model {
                        spawned["model"] = Value::String(m);
                    }
                    outs.push(Out::XaiUpdate {
                        session_id: self.parent_session_id.clone(),
                        update: spawned,
                    });
                    self.parent_after_block = true;
                    outs
                }
                StepKind::Shell | StepKind::Compute | StepKind::Other => {
                    let mut outs = Vec::new();
                    if self.set_node_state(node, PlanNodeState::InProgress)
                        && let Some(plan_out) = self.build_plan_out()
                    {
                        outs.push(plan_out);
                    }
                    outs
                }
            },

            DriverEvent::AgentPrompt { attempt, text, .. } => {
                let child_sid = self.ensure_child_session_id(attempt);
                vec![Out::Update {
                    session_id: child_sid,
                    update: acp::SessionUpdate::UserMessageChunk(text_chunk(text)),
                }]
            }

            DriverEvent::AgentUpdate {
                attempt, update, ..
            } => {
                let child_sid = self.ensure_child_session_id(attempt);
                if let Some(chunk_text) = extract_agent_message_chunk_text(update)
                    && let Some(state) = self.attempts.get_mut(attempt)
                {
                    state.final_message.push_str(chunk_text);
                }
                if is_dropped_update_json(update) {
                    return Vec::new();
                }
                let Ok(parsed) = serde_json::from_value::<acp::SessionUpdate>(update.clone())
                else {
                    return Vec::new();
                };
                match &parsed {
                    acp::SessionUpdate::AvailableCommandsUpdate(_)
                    | acp::SessionUpdate::SessionInfoUpdate(_)
                    | acp::SessionUpdate::CurrentModeUpdate(_) => return Vec::new(),
                    acp::SessionUpdate::ToolCall(_) => {
                        if let Some(state) = self.attempts.get_mut(attempt) {
                            state.tool_calls = state.tool_calls.saturating_add(1);
                        }
                    }
                    _ => {}
                }
                vec![Out::Update {
                    session_id: child_sid,
                    update: parsed,
                }]
            }

            DriverEvent::ShellPrepared {
                attempt,
                command_line,
                cwd,
                ..
            } => {
                self.shell_attempts.insert(
                    attempt.clone(),
                    ShellAttemptState {
                        command_line: command_line.clone(),
                        cwd: cwd.clone(),
                        prepared: true,
                    },
                );
                let tool_id = format!("acpx-tool:{}:{attempt}", self.run_id);
                let tc = acp::ToolCall::new(acp::ToolCallId::new(tool_id), command_line.clone())
                    .kind(acp::ToolKind::Execute)
                    .status(acp::ToolCallStatus::InProgress)
                    .content(Vec::new())
                    .locations(Vec::new())
                    .raw_input(Some(json!({
                        "command": command_line,
                        "cwd": cwd,
                    })));
                self.parent_after_block = true;
                vec![Out::Update {
                    session_id: self.parent_session_id.clone(),
                    update: acp::SessionUpdate::ToolCall(tc),
                }]
            }

            DriverEvent::StepFinished {
                node,
                attempt,
                kind,
                outcome,
                duration_ms,
                error,
                output,
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                if *kind == StepKind::Checkpoint {
                    return Vec::new();
                }

                let is_ok = outcome == "ok";
                let new_plan_state = if is_ok {
                    PlanNodeState::Completed
                } else {
                    PlanNodeState::Failed
                };
                let plan_changed = self.set_node_state(node, new_plan_state);

                let mut outs = Vec::new();
                match kind {
                    StepKind::Acp => {
                        let child_sid = self.ensure_child_session_id(attempt);
                        let subagent_id = format!("acpx-sub:{}:{attempt}", self.run_id);
                        let tool_id = format!("acpx-tool:{}:{attempt}", self.run_id);

                        let (tool_calls, final_msg) = self
                            .attempts
                            .get(attempt)
                            .map(|s| (s.tool_calls, s.final_message.clone()))
                            .unwrap_or((0, String::new()));

                        let status_str = if is_ok { "completed" } else { "failed" };
                        let mut finished = json!({
                            "sessionUpdate": "subagent_finished",
                            "subagent_id": subagent_id,
                            "parent_session_id": self.parent_session_id,
                            "child_session_id": child_sid,
                            "status": status_str,
                            "tool_calls": tool_calls,
                            "turns": 1_u32,
                            "duration_ms": *duration_ms,
                            "tokens_used": 0_u64,
                            "will_wake": false,
                        });
                        if !is_ok {
                            let err_text = error
                                .clone()
                                .filter(|e| !e.is_empty())
                                .unwrap_or_else(|| outcome.clone());
                            finished["error"] = Value::String(err_text);
                        }
                        if !final_msg.is_empty() {
                            let cut = truncate_chars(&final_msg, 4000);
                            finished["output"] = Value::String(cut);
                        }

                        outs.push(Out::XaiUpdate {
                            session_id: self.parent_session_id.clone(),
                            update: finished,
                        });

                        let fields = acp::ToolCallUpdateFields::new()
                            .status(Some(acp::ToolCallStatus::Completed));
                        let tcu = acp::ToolCallUpdate::new(acp::ToolCallId::new(tool_id), fields);
                        outs.push(Out::Update {
                            session_id: self.parent_session_id.clone(),
                            update: acp::SessionUpdate::ToolCallUpdate(tcu),
                        });
                        self.parent_after_block = true;
                    }

                    StepKind::Shell => {
                        let tool_id = format!("acpx-tool:{}:{attempt}", self.run_id);
                        let shell_state = self
                            .shell_attempts
                            .get(attempt)
                            .cloned()
                            .unwrap_or_default();

                        if !shell_state.prepared {
                            let tc = acp::ToolCall::new(
                                acp::ToolCallId::new(tool_id.clone()),
                                shell_state.command_line.clone(),
                            )
                            .kind(acp::ToolKind::Execute)
                            .status(acp::ToolCallStatus::InProgress)
                            .content(Vec::new())
                            .locations(Vec::new())
                            .raw_input(Some(json!({
                                "command": shell_state.command_line,
                                "cwd": shell_state.cwd,
                            })));
                            outs.push(Out::Update {
                                session_id: self.parent_session_id.clone(),
                                update: acp::SessionUpdate::ToolCall(tc),
                            });
                        }

                        let stdout_str = stdout.as_deref().unwrap_or("");
                        let stderr_str = stderr.as_deref().unwrap_or("");
                        let content_str = if is_ok {
                            if !stdout_str.is_empty() {
                                stdout_str.to_string()
                            } else {
                                "(no output)".to_string()
                            }
                        } else {
                            let mut parts: Vec<&str> = Vec::new();
                            if let Some(err) = error.as_deref().filter(|s| !s.is_empty()) {
                                parts.push(err);
                            }
                            if !stderr_str.is_empty() {
                                parts.push(stderr_str);
                            }
                            if !stdout_str.is_empty() && parts.is_empty() {
                                parts.push(stdout_str);
                            }
                            if parts.is_empty() {
                                "(no output)".to_string()
                            } else {
                                parts.join("\n")
                            }
                        };

                        let stdout_bytes: Vec<u8> = stdout_str.as_bytes().to_vec();
                        let total_bytes = stdout_bytes.len();
                        let raw_output = json!({
                            "type": "Bash",
                            "output": stdout_bytes,
                            "output_for_prompt": stdout_str,
                            "exit_code": exit_code,
                            "command": shell_state.command_line,
                            "truncated": false,
                            "signal": null,
                            "timed_out": outcome == "timed_out",
                            "description": null,
                            "current_dir": "",
                            "output_file": "",
                            "total_bytes": total_bytes,
                        });

                        let status = if is_ok {
                            acp::ToolCallStatus::Completed
                        } else {
                            acp::ToolCallStatus::Failed
                        };
                        let content = vec![acp::ToolCallContent::from(acp::ContentBlock::Text(
                            acp::TextContent::new(content_str),
                        ))];
                        let fields = acp::ToolCallUpdateFields::new()
                            .status(Some(status))
                            .content(Some(content))
                            .raw_output(Some(raw_output));
                        let tcu = acp::ToolCallUpdate::new(acp::ToolCallId::new(tool_id), fields);
                        outs.push(Out::Update {
                            session_id: self.parent_session_id.clone(),
                            update: acp::SessionUpdate::ToolCallUpdate(tcu),
                        });
                        self.parent_after_block = true;
                    }

                    StepKind::Compute | StepKind::Other => {
                        let tool_id = format!("acpx-tool:{}:{attempt}", self.run_id);
                        let short_text = format_compute_output(output, error.as_deref());
                        let content = if short_text.is_empty() {
                            Vec::new()
                        } else {
                            vec![acp::ToolCallContent::from(acp::ContentBlock::Text(
                                acp::TextContent::new(short_text),
                            ))]
                        };
                        let tc = acp::ToolCall::new(acp::ToolCallId::new(tool_id), node.clone())
                            .kind(acp::ToolKind::Think)
                            .status(acp::ToolCallStatus::Completed)
                            .content(content)
                            .locations(Vec::new());
                        outs.push(Out::Update {
                            session_id: self.parent_session_id.clone(),
                            update: acp::SessionUpdate::ToolCall(tc),
                        });
                        self.parent_after_block = true;
                    }

                    StepKind::Checkpoint => {}
                }

                if plan_changed && let Some(plan_out) = self.build_plan_out() {
                    outs.push(plan_out);
                }
                outs
            }

            DriverEvent::Waiting {
                node,
                question,
                choices,
                ..
            } => {
                let mut text = format!("\n**◐ {node}** · {question}\n");
                for (idx, choice) in choices.iter().enumerate() {
                    let n = idx + 1;
                    text.push_str(&format!(
                        "`{n}` **{}** → `{}`\n",
                        choice.label, choice.flow_file
                    ));
                }

                let mut outs = vec![self.parent_message(&text)];
                if self.set_node_state(node, PlanNodeState::InProgress)
                    && let Some(plan_out) = self.build_plan_out()
                {
                    outs.push(plan_out);
                }
                outs
            }

            DriverEvent::RunFinished {
                status,
                error,
                at_ms,
            } => {
                if status == "waiting" {
                    return Vec::new();
                }
                let text = if status == "completed" {
                    let dur_ms = (*at_ms - self.started_at_ms).max(0) as u64;
                    let dur = fmt_duration(dur_ms);
                    format!("✓ **{}** finished in {dur}\n", self.title)
                } else {
                    let detail = match error.as_deref().filter(|s| !s.is_empty()) {
                        Some(err) if status != "failed" => format!("{status} ({err})"),
                        Some(err) => err.to_string(),
                        None => status.clone(),
                    };
                    format!("✗ **{}** failed: {detail}\n", self.title)
                };
                vec![self.parent_message(&text)]
            }
        }
    }

    /// Child session id used for an acp step attempt ("acpx-step:<run_id>:<attempt>"), once StepStarted was seen.
    pub fn child_session_id(&self, attempt: &str) -> Option<String> {
        self.attempts
            .get(attempt)
            .map(|s| s.child_session_id.clone())
    }

    /// Final agent message text of an acp attempt (concatenated agent_message_chunk text), once seen.
    pub fn final_message(&self, attempt: &str) -> Option<String> {
        self.attempts
            .get(attempt)
            .filter(|s| !s.final_message.is_empty())
            .map(|s| s.final_message.clone())
    }

    fn ensure_child_session_id(&mut self, attempt: &str) -> String {
        if let Some(existing) = self.attempts.get(attempt) {
            return existing.child_session_id.clone();
        }
        let child_sid = format!("acpx-step:{}:{attempt}", self.run_id);
        self.attempts.insert(
            attempt.to_string(),
            AttemptState {
                child_session_id: child_sid.clone(),
                final_message: String::new(),
                tool_calls: 0,
            },
        );
        child_sid
    }

    fn set_node_state(&mut self, node: &str, state: PlanNodeState) -> bool {
        let prev = self.plan_states.insert(node.to_string(), state);
        prev != Some(state)
    }

    fn build_plan_out(&self) -> Option<Out> {
        let flow = self.flow.as_ref()?;
        let mut entries = Vec::with_capacity(flow.nodes.len());
        for node in &flow.nodes {
            let state = self
                .plan_states
                .get(&node.id)
                .copied()
                .unwrap_or(PlanNodeState::Pending);
            let mut content = plan_node_label(node);
            let status = match state {
                PlanNodeState::Pending => acp::PlanEntryStatus::Pending,
                PlanNodeState::InProgress => acp::PlanEntryStatus::InProgress,
                PlanNodeState::Completed => acp::PlanEntryStatus::Completed,
                PlanNodeState::Failed => {
                    content.push_str(" ✗");
                    acp::PlanEntryStatus::Completed
                }
            };
            entries.push(acp::PlanEntry::new(
                content,
                acp::PlanEntryPriority::Medium,
                status,
            ));
        }
        Some(Out::Update {
            session_id: self.parent_session_id.clone(),
            update: acp::SessionUpdate::Plan(acp::Plan::new(entries)),
        })
    }

    fn acp_step_desc_and_model(&self, node_id: &str) -> (String, Option<String>) {
        if let Some(flow) = &self.flow
            && let Some(node) = flow.nodes.iter().find(|n| n.id == node_id)
        {
            return (plan_node_label(node), node.model.clone());
        }
        (node_id.to_string(), None)
    }

    fn parent_message(&mut self, text: &str) -> Out {
        let final_text = if self.parent_after_block && !text.starts_with('\n') {
            format!("\n{text}")
        } else {
            text.to_string()
        };
        self.parent_after_block = false;
        Out::Update {
            session_id: self.parent_session_id.clone(),
            update: acp::SessionUpdate::AgentMessageChunk(text_chunk(&final_text)),
        }
    }
}

/// The flow as a mermaid `flowchart LR` source (no fences). Node labels: "<glyph> <id>" where the glyph is
/// ✓ done, ✗ failed, ◐ waiting, ● running, ○ not reached; `states` maps node id -> glyph char (missing = ○).
pub fn mermaid(flow: &FlowSketch, states: &HashMap<String, char>) -> String {
    let mut lines = Vec::new();
    lines.push("flowchart LR".to_string());

    for node in &flow.nodes {
        let sid = sanitize_mermaid_id(&node.id);
        let glyph = states.get(&node.id).copied().unwrap_or('○');
        let label = escape_mermaid_text(&format!("{glyph} {}", node.id));
        lines.push(format!("    {sid}[\"{label}\"]"));
    }

    for edge in &flow.edges {
        let from = sanitize_mermaid_id(&edge.from);
        let to = sanitize_mermaid_id(&edge.to);
        if let Some(label) = edge.label.as_deref().filter(|l| !l.is_empty()) {
            let clean_label = label.replace('|', "/").replace('\n', " ");
            lines.push(format!("    {from} -->|{clean_label}| {to}"));
        } else {
            lines.push(format!("    {from} --> {to}"));
        }
    }

    lines.join("\n")
}

fn sanitize_mermaid_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len().max(1));
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "_".to_string()
    } else if is_mermaid_keyword(&out) {
        format!("n_{out}")
    } else {
        out
    }
}

fn is_mermaid_keyword(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "end" | "subgraph" | "graph" | "flowchart" | "style" | "classdef" | "class" | "click"
    )
}

fn escape_mermaid_text(s: &str) -> String {
    s.replace('"', "'").replace('\n', " ")
}

fn plan_node_label(node: &SketchNode) -> String {
    match node.kind {
        StepKind::Acp => {
            let mut parts = vec![node.id.as_str()];
            if let Some(p) = node.profile.as_deref().filter(|s| !s.is_empty()) {
                parts.push(p);
            }
            if let Some(m) = node.model.as_deref().filter(|s| !s.is_empty()) {
                parts.push(m);
            }
            if let Some(e) = node.effort.as_deref().filter(|s| !s.is_empty()) {
                parts.push(e);
            }
            parts.join(" · ")
        }
        StepKind::Shell => format!("{} · shell", node.id),
        StepKind::Checkpoint => format!("{} · checkpoint", node.id),
        StepKind::Compute | StepKind::Other => node.id.clone(),
    }
}

fn text_chunk(text: &str) -> acp::ContentChunk {
    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
        text.to_string(),
    )))
}

fn extract_agent_message_chunk_text(update: &Value) -> Option<&str> {
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("agent_message_chunk") {
        return None;
    }
    let content = update.get("content")?;
    if content.get("type").and_then(Value::as_str) == Some("text") {
        content.get("text").and_then(Value::as_str)
    } else {
        content.get("text").and_then(Value::as_str)
    }
}

fn is_dropped_update_json(update: &Value) -> bool {
    let Some(tag) = update.get("sessionUpdate").and_then(Value::as_str) else {
        return true;
    };
    let lower = tag.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "available_commands_update"
            | "session_info_update"
            | "current_mode_update"
            | "usage_update"
            | "config_option_update"
            | "config_options_update"
    ) || lower.contains("usage")
        || lower.contains("config")
}

fn format_compute_output(output: &Value, error: Option<&str>) -> String {
    let raw = match output {
        Value::Null => error.unwrap_or("").to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    truncate_chars(&raw, 300)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => s[..byte_idx].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{GateChoice, SketchEdge};
    use std::path::PathBuf;

    fn sample_flow() -> FlowSketch {
        FlowSketch {
            name: "pr-triage".to_string(),
            start_at: "extract".to_string(),
            nodes: vec![
                SketchNode {
                    id: "extract".to_string(),
                    kind: StepKind::Shell,
                    summary: None,
                    profile: None,
                    model: None,
                    effort: None,
                },
                SketchNode {
                    id: "review-code".to_string(),
                    kind: StepKind::Acp,
                    summary: None,
                    profile: Some("grok-build".to_string()),
                    model: Some("grok-4".to_string()),
                    effort: Some("high".to_string()),
                },
                SketchNode {
                    id: "route".to_string(),
                    kind: StepKind::Compute,
                    summary: None,
                    profile: None,
                    model: None,
                    effort: None,
                },
                SketchNode {
                    id: "human_gate".to_string(),
                    kind: StepKind::Checkpoint,
                    summary: Some("Approve changes?".to_string()),
                    profile: None,
                    model: None,
                    effort: None,
                },
            ],
            edges: vec![
                SketchEdge {
                    from: "extract".to_string(),
                    to: "review-code".to_string(),
                    label: None,
                },
                SketchEdge {
                    from: "review-code".to_string(),
                    to: "route".to_string(),
                    label: None,
                },
                SketchEdge {
                    from: "route".to_string(),
                    to: "human_gate".to_string(),
                    label: Some("needs_approval".to_string()),
                },
            ],
        }
    }

    fn chunk_text(update: &acp::SessionUpdate) -> Option<String> {
        let val = serde_json::to_value(update).ok()?;
        val.pointer("/content/text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    #[test]
    fn run_started_emits_mermaid_message_and_initial_plan() {
        let mut proj = Projector::new("parent-1");
        let flow = sample_flow();
        let outs = proj.on_event(&DriverEvent::RunStarted {
            run_id: "run-42".to_string(),
            run_dir: PathBuf::from("/tmp/runs/run-42"),
            title: "Fix bug".to_string(),
            flow_path: "/flows/pr-triage.json".to_string(),
            flow,
            at_ms: 1_000,
        });

        assert_eq!(outs.len(), 2);
        match &outs[0] {
            Out::Update { session_id, update } => {
                assert_eq!(session_id, "parent-1");
                let text = chunk_text(update).unwrap_or_default();
                assert!(text.contains("**▶ Fix bug**"));
                assert!(text.contains("`pr-triage.json` · run-42"));
                assert!(text.contains("```mermaid\nflowchart LR"));
                assert!(text.contains("○ extract"));
            }
            _ => panic!("expected AgentMessageChunk update"),
        }

        match &outs[1] {
            Out::Update {
                session_id,
                update: acp::SessionUpdate::Plan(plan),
            } => {
                assert_eq!(session_id, "parent-1");
                let val = serde_json::to_value(plan).unwrap_or(Value::Null);
                let entries = val
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(entries.len(), 4);
                assert_eq!(
                    entries[0].get("content").and_then(Value::as_str),
                    Some("extract · shell")
                );
                assert_eq!(
                    entries[1].get("content").and_then(Value::as_str),
                    Some("review-code · grok-build · grok-4 · high")
                );
                assert_eq!(
                    entries[2].get("content").and_then(Value::as_str),
                    Some("route")
                );
                assert_eq!(
                    entries[3].get("content").and_then(Value::as_str),
                    Some("human_gate · checkpoint")
                );
                for entry in entries {
                    assert_eq!(entry.get("status").and_then(Value::as_str), Some("pending"));
                    assert_eq!(
                        entry.get("priority").and_then(Value::as_str),
                        Some("medium")
                    );
                }
            }
            _ => panic!("expected Plan update"),
        }
    }

    #[test]
    fn acp_step_order_spawned_before_child_updates() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "run-1".to_string(),
            run_dir: PathBuf::from("/tmp/run-1"),
            title: "Run".to_string(),
            flow_path: "flow.json".to_string(),
            flow: sample_flow(),
            at_ms: 100,
        });

        let start_outs = proj.on_event(&DriverEvent::StepStarted {
            node: "review-code".to_string(),
            attempt: "review-code#1".to_string(),
            kind: StepKind::Acp,
            at_ms: 200,
        });

        assert_eq!(
            proj.child_session_id("review-code#1").as_deref(),
            Some("acpx-step:run-1:review-code#1")
        );
        assert_eq!(start_outs.len(), 3);

        // 1: Plan update
        assert!(matches!(
            start_outs[0],
            Out::Update {
                update: acp::SessionUpdate::Plan(_),
                ..
            }
        ));

        // 2: Parent Task ToolCall with variant Task set on creation
        match &start_outs[1] {
            Out::Update {
                session_id,
                update: acp::SessionUpdate::ToolCall(tc),
            } => {
                assert_eq!(session_id, "parent-1");
                let v = serde_json::to_value(tc).unwrap_or(Value::Null);
                assert_eq!(
                    v.get("toolCallId").and_then(Value::as_str),
                    Some("acpx-tool:run-1:review-code#1")
                );
                assert_eq!(v.get("title").and_then(Value::as_str), Some("Task"));
                assert_eq!(
                    v.pointer("/rawInput/variant").and_then(Value::as_str),
                    Some("Task")
                );
                assert_eq!(
                    v.pointer("/rawInput/task_id").and_then(Value::as_str),
                    Some("acpx-sub:run-1:review-code#1")
                );
            }
            _ => panic!("expected parent Task ToolCall"),
        }

        // 3: XaiUpdate subagent_spawned before any child update
        match &start_outs[2] {
            Out::XaiUpdate { session_id, update } => {
                assert_eq!(session_id, "parent-1");
                assert_eq!(
                    update.get("sessionUpdate").and_then(Value::as_str),
                    Some("subagent_spawned")
                );
                assert_eq!(
                    update.get("subagent_id").and_then(Value::as_str),
                    Some("acpx-sub:run-1:review-code#1")
                );
                assert_eq!(
                    update.get("child_session_id").and_then(Value::as_str),
                    Some("acpx-step:run-1:review-code#1")
                );
                assert_eq!(
                    update.get("subagent_type").and_then(Value::as_str),
                    Some("acpx step")
                );
                assert_eq!(
                    update.get("model").and_then(Value::as_str),
                    Some("grok-4")
                );
            }
            _ => panic!("expected XaiUpdate subagent_spawned"),
        }

        // Subsequent AgentPrompt goes to the child session
        let prompt_outs = proj.on_event(&DriverEvent::AgentPrompt {
            node: "review-code".to_string(),
            attempt: "review-code#1".to_string(),
            text: "Please review this PR".to_string(),
            at_ms: 210,
        });
        assert_eq!(prompt_outs.len(), 1);
        match &prompt_outs[0] {
            Out::Update {
                session_id,
                update: acp::SessionUpdate::UserMessageChunk(_),
            } => {
                assert_eq!(session_id, "acpx-step:run-1:review-code#1");
            }
            _ => panic!("expected child UserMessageChunk"),
        }
    }

    #[test]
    fn agent_update_thought_chunk_forwarded_to_child() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "T".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });
        proj.on_event(&DriverEvent::StepStarted {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            at_ms: 1,
        });

        let outs = proj.on_event(&DriverEvent::AgentUpdate {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            update: json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "Thinking about edge cases..." }
            }),
            at_ms: 2,
        });

        assert_eq!(outs.len(), 1);
        match &outs[0] {
            Out::Update {
                session_id,
                update: acp::SessionUpdate::AgentThoughtChunk(_),
            } => {
                assert_eq!(session_id, "acpx-step:r1:a1");
            }
            _ => panic!("expected child AgentThoughtChunk"),
        }
    }

    #[test]
    fn unknown_and_filtered_session_updates_are_dropped() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "T".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });
        proj.on_event(&DriverEvent::StepStarted {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            at_ms: 1,
        });

        for ignored in [
            json!({ "sessionUpdate": "totally_unknown_future_variant", "foo": 123 }),
            json!({ "sessionUpdate": "available_commands_update", "availableCommands": [] }),
            json!({ "sessionUpdate": "session_info_update", "title": "hi" }),
            json!({ "sessionUpdate": "current_mode_update", "currentModeId": "code" }),
            json!({ "sessionUpdate": "usage_update", "used": 10, "size": 100 }),
            json!({ "sessionUpdate": "config_option_update", "configOptions": [] }),
        ] {
            let outs = proj.on_event(&DriverEvent::AgentUpdate {
                node: "review-code".to_string(),
                attempt: "a1".to_string(),
                update: ignored,
                at_ms: 2,
            });
            assert!(outs.is_empty());
        }
    }

    #[test]
    fn final_message_concatenates_agent_message_chunks() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "T".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });
        proj.on_event(&DriverEvent::StepStarted {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            at_ms: 1,
        });

        assert_eq!(proj.final_message("a1"), None);

        for part in ["Hello", ", ", "world!"] {
            proj.on_event(&DriverEvent::AgentUpdate {
                node: "review-code".to_string(),
                attempt: "a1".to_string(),
                update: json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": part }
                }),
                at_ms: 2,
            });
        }

        assert_eq!(proj.final_message("a1").as_deref(), Some("Hello, world!"));
    }

    #[test]
    fn subagent_finished_truncates_output_to_4000_chars_and_sets_required_fields() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "T".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });
        proj.on_event(&DriverEvent::StepStarted {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            at_ms: 1,
        });

        let long_text = "x".repeat(4500);
        proj.on_event(&DriverEvent::AgentUpdate {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            update: json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": long_text }
            }),
            at_ms: 2,
        });
        proj.on_event(&DriverEvent::AgentUpdate {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            update: json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tc-1",
                "title": "read_file",
                "kind": "read",
                "status": "in_progress"
            }),
            at_ms: 3,
        });

        let outs = proj.on_event(&DriverEvent::StepFinished {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            outcome: "ok".to_string(),
            duration_ms: 1234,
            error: None,
            output: Value::Null,
            stdout: None,
            stderr: None,
            exit_code: None,
            at_ms: 4,
        });

        match &outs[0] {
            Out::XaiUpdate { update, .. } => {
                assert_eq!(
                    update.get("sessionUpdate").and_then(Value::as_str),
                    Some("subagent_finished")
                );
                assert_eq!(
                    update.get("status").and_then(Value::as_str),
                    Some("completed")
                );
                assert_eq!(update.get("tool_calls").and_then(Value::as_u64), Some(1));
                assert_eq!(update.get("turns").and_then(Value::as_u64), Some(1));
                assert_eq!(
                    update.get("duration_ms").and_then(Value::as_u64),
                    Some(1234)
                );
                assert_eq!(update.get("tokens_used").and_then(Value::as_u64), Some(0));
                assert_eq!(
                    update.get("will_wake").and_then(Value::as_bool),
                    Some(false)
                );
                assert!(update.get("error").is_none());
                let out_str = update.get("output").and_then(Value::as_str).unwrap_or("");
                assert_eq!(out_str.chars().count(), 4000);
            }
            _ => panic!("expected subagent_finished XaiUpdate"),
        }
    }

    #[test]
    fn shell_step_tool_call_ids_and_completed_or_failed_status() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "T".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });

        let prep = proj.on_event(&DriverEvent::ShellPrepared {
            node: "extract".to_string(),
            attempt: "extract#1".to_string(),
            command_line: "git status -s".to_string(),
            cwd: Some("/repo".to_string()),
            at_ms: 10,
        });
        assert_eq!(prep.len(), 1);
        match &prep[0] {
            Out::Update {
                update: acp::SessionUpdate::ToolCall(tc),
                ..
            } => {
                let v = serde_json::to_value(tc).unwrap_or(Value::Null);
                assert_eq!(
                    v.get("toolCallId").and_then(Value::as_str),
                    Some("acpx-tool:r1:extract#1")
                );
                assert_eq!(v.get("kind").and_then(Value::as_str), Some("execute"));
                assert_eq!(
                    v.get("title").and_then(Value::as_str),
                    Some("git status -s")
                );
                assert_eq!(
                    v.pointer("/rawInput/command").and_then(Value::as_str),
                    Some("git status -s")
                );
            }
            _ => panic!("expected execute ToolCall"),
        }

        let fin_ok = proj.on_event(&DriverEvent::StepFinished {
            node: "extract".to_string(),
            attempt: "extract#1".to_string(),
            kind: StepKind::Shell,
            outcome: "ok".to_string(),
            duration_ms: 50,
            error: None,
            output: Value::Null,
            stdout: Some("M src/lib.rs\n".to_string()),
            stderr: None,
            exit_code: Some(0),
            at_ms: 60,
        });
        match &fin_ok[0] {
            Out::Update {
                update: acp::SessionUpdate::ToolCallUpdate(tcu),
                ..
            } => {
                let v = serde_json::to_value(tcu).unwrap_or(Value::Null);
                assert_eq!(
                    v.get("toolCallId").and_then(Value::as_str),
                    Some("acpx-tool:r1:extract#1")
                );
                assert_eq!(v.get("status").and_then(Value::as_str), Some("completed"));
                assert_eq!(
                    v.pointer("/rawOutput/type").and_then(Value::as_str),
                    Some("Bash")
                );
                assert_eq!(
                    v.pointer("/rawOutput/exit_code").and_then(Value::as_i64),
                    Some(0)
                );
                assert_eq!(
                    v.pointer("/rawOutput/total_bytes").and_then(Value::as_u64),
                    Some(13)
                );
            }
            _ => panic!("expected ToolCallUpdate"),
        }

        let fin_fail = proj.on_event(&DriverEvent::StepFinished {
            node: "extract".to_string(),
            attempt: "extract#2".to_string(),
            kind: StepKind::Shell,
            outcome: "timed_out".to_string(),
            duration_ms: 5000,
            error: Some("command timed out".to_string()),
            output: Value::Null,
            stdout: Some(String::new()),
            stderr: Some("fatal error".to_string()),
            exit_code: Some(124),
            at_ms: 5060,
        });
        // Unprepared attempt emits ToolCall then ToolCallUpdate
        match &fin_fail[1] {
            Out::Update {
                update: acp::SessionUpdate::ToolCallUpdate(tcu),
                ..
            } => {
                let v = serde_json::to_value(tcu).unwrap_or(Value::Null);
                assert_eq!(v.get("status").and_then(Value::as_str), Some("failed"));
                assert_eq!(
                    v.pointer("/rawOutput/timed_out").and_then(Value::as_bool),
                    Some(true)
                );
            }
            _ => panic!("expected failed ToolCallUpdate"),
        }
    }

    #[test]
    fn checkpoint_waiting_lists_numbered_choices_and_marks_plan_in_progress() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "T".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });

        let outs = proj.on_event(&DriverEvent::Waiting {
            node: "human_gate".to_string(),
            attempt: "human_gate#1".to_string(),
            question: "Approve changes?".to_string(),
            choices: vec![
                GateChoice {
                    label: "Approve".to_string(),
                    flow_file: "deploy.flow.json".to_string(),
                },
                GateChoice {
                    label: "Rework".to_string(),
                    flow_file: "rework.flow.json".to_string(),
                },
            ],
            at_ms: 100,
        });

        assert_eq!(outs.len(), 2);
        match &outs[0] {
            Out::Update { update, .. } => {
                let text = chunk_text(update).unwrap_or_default();
                assert!(text.contains("**◐ human_gate** · Approve changes?"));
                assert!(text.contains("`1` **Approve** → `deploy.flow.json`"));
                assert!(text.contains("`2` **Rework** → `rework.flow.json`"));
            }
            _ => panic!("expected AgentMessageChunk"),
        }
        match &outs[1] {
            Out::Update {
                update: acp::SessionUpdate::Plan(plan),
                ..
            } => {
                let v = serde_json::to_value(plan).unwrap_or(Value::Null);
                let entries = v
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    entries[3].get("status").and_then(Value::as_str),
                    Some("in_progress")
                );
            }
            _ => panic!("expected Plan update"),
        }
    }

    #[test]
    fn run_finished_formats_completed_failed_and_waiting() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "Build".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 1_000,
        });

        let waiting_outs = proj.on_event(&DriverEvent::RunFinished {
            status: "waiting".to_string(),
            error: None,
            at_ms: 1_400,
        });
        assert!(waiting_outs.is_empty());

        let ok_outs = proj.on_event(&DriverEvent::RunFinished {
            status: "completed".to_string(),
            error: None,
            at_ms: 1_400,
        });
        assert_eq!(ok_outs.len(), 1);
        match &ok_outs[0] {
            Out::Update { update, .. } => {
                let text = chunk_text(update).unwrap_or_default();
                assert!(text.contains("✓ **Build** finished in 400ms"));
            }
            _ => panic!("expected completed message"),
        }

        let fail_outs = proj.on_event(&DriverEvent::RunFinished {
            status: "failed".to_string(),
            error: Some("step crashed".to_string()),
            at_ms: 2_000,
        });
        match &fail_outs[0] {
            Out::Update { update, .. } => {
                let text = chunk_text(update).unwrap_or_default();
                assert!(text.contains("✗ **Build** failed: step crashed"));
            }
            _ => panic!("expected failed message"),
        }
    }

    #[test]
    fn plan_statuses_after_failed_step_mark_completed_with_cross() {
        let mut proj = Projector::new("parent-1");
        proj.on_event(&DriverEvent::RunStarted {
            run_id: "r1".to_string(),
            run_dir: PathBuf::from("/tmp/r1"),
            title: "Build".to_string(),
            flow_path: "f.json".to_string(),
            flow: sample_flow(),
            at_ms: 0,
        });
        proj.on_event(&DriverEvent::StepStarted {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            at_ms: 10,
        });
        let outs = proj.on_event(&DriverEvent::StepFinished {
            node: "review-code".to_string(),
            attempt: "a1".to_string(),
            kind: StepKind::Acp,
            outcome: "failed".to_string(),
            duration_ms: 100,
            error: Some("boom".to_string()),
            output: Value::Null,
            stdout: None,
            stderr: None,
            exit_code: None,
            at_ms: 110,
        });

        // Parent ToolCallUpdate is still Completed so the pager does not render a red generic card
        match &outs[1] {
            Out::Update {
                update: acp::SessionUpdate::ToolCallUpdate(tcu),
                ..
            } => {
                let v = serde_json::to_value(tcu).unwrap_or(Value::Null);
                assert_eq!(v.get("status").and_then(Value::as_str), Some("completed"));
            }
            _ => panic!("expected ToolCallUpdate"),
        }

        // Plan entry is Completed and has " ✗" appended
        match &outs[2] {
            Out::Update {
                update: acp::SessionUpdate::Plan(plan),
                ..
            } => {
                let v = serde_json::to_value(plan).unwrap_or(Value::Null);
                let entries = v
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    entries[1].get("status").and_then(Value::as_str),
                    Some("completed")
                );
                assert_eq!(
                    entries[1].get("content").and_then(Value::as_str),
                    Some("review-code · grok-build · grok-4 · high ✗")
                );
            }
            _ => panic!("expected Plan update"),
        }
    }

    #[test]
    fn mermaid_renders_glyphs_sanitized_ids_and_edge_labels() {
        let flow = sample_flow();
        let mut states = HashMap::new();
        states.insert("extract".to_string(), '✓');
        states.insert("review-code".to_string(), '●');
        states.insert("route".to_string(), '✗');
        // human_gate omitted -> defaults to ○

        let chart = mermaid(&flow, &states);
        assert!(chart.starts_with("flowchart LR\n"));
        assert!(chart.contains("extract[\"✓ extract\"]"));
        assert!(chart.contains("review_code[\"● review-code\"]"));
        assert!(chart.contains("route[\"✗ route\"]"));
        assert!(chart.contains("human_gate[\"○ human_gate\"]"));
        assert!(chart.contains("extract --> review_code"));
        assert!(chart.contains("route -->|needs_approval| human_gate"));
    }

    #[test]
    fn determinism_two_projectors_produce_identical_debug_output() {
        let events = vec![
            DriverEvent::RunStarted {
                run_id: "run-99".to_string(),
                run_dir: PathBuf::from("/tmp/run-99"),
                title: "Det".to_string(),
                flow_path: "flow.json".to_string(),
                flow: sample_flow(),
                at_ms: 1000,
            },
            DriverEvent::StepStarted {
                node: "review-code".to_string(),
                attempt: "review-code#1".to_string(),
                kind: StepKind::Acp,
                at_ms: 1100,
            },
            DriverEvent::AgentPrompt {
                node: "review-code".to_string(),
                attempt: "review-code#1".to_string(),
                text: "Go".to_string(),
                at_ms: 1110,
            },
            DriverEvent::StepFinished {
                node: "review-code".to_string(),
                attempt: "review-code#1".to_string(),
                kind: StepKind::Acp,
                outcome: "ok".to_string(),
                duration_ms: 200,
                error: None,
                output: Value::Null,
                stdout: None,
                stderr: None,
                exit_code: None,
                at_ms: 1300,
            },
            DriverEvent::StepFinished {
                node: "route".to_string(),
                attempt: "route#1".to_string(),
                kind: StepKind::Compute,
                outcome: "ok".to_string(),
                duration_ms: 5,
                error: None,
                output: json!({"action": "approve"}),
                stdout: None,
                stderr: None,
                exit_code: None,
                at_ms: 1305,
            },
            DriverEvent::RunFinished {
                status: "completed".to_string(),
                error: None,
                at_ms: 1310,
            },
        ];

        let mut p1 = Projector::new("session-a");
        let mut p2 = Projector::new("session-a");

        let out1: Vec<Vec<Out>> = events.iter().map(|e| p1.on_event(e)).collect();
        let out2: Vec<Vec<Out>> = events.iter().map(|e| p2.on_event(e)).collect();

        assert_eq!(format!("{out1:?}"), format!("{out2:?}"));
    }
}
```