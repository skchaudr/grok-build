=== FILE: crates/codegen/xai-grok-pager/src/piw/mirror.rs ===
```rust
//! Mirrors pi-workflows runs into acpx-shaped run folders on disk so Grok Build's
//! existing flow screens (`bundle.rs`, `run.rs`, `detail.rs`, `ledger.rs`,
//! `side_panel/flows.rs`) can display them unchanged.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::acpx::bundle::parse_ts;
use crate::piw::client::{ActivityKind, ConnState, Content, PiwClient, RunView, StepActivity};
use crate::piw::gate::{PiwGate, answer_target, gate_from};

const FOLDER_PREFIX: &str = "piw-";
const META_FILE: &str = ".mirror_meta.json";
const SESSION_ID: &str = "main";
const POLL_INTERVAL_MS: u64 = 250;
const STALL_REFRESH_INTERVAL_MS: i64 = 30_000;

/// Everything known about one run at one moment; the pure input of the writer.
pub struct MirrorInput<'a> {
    pub summary: Option<&'a serde_json::Value>, // its WorkflowRunSummary, when the runs list has it
    pub view: Option<&'a RunView>,              // its run view, when the run is being watched
    pub activity: &'a [StepActivity],           // all activity received for it (may be empty)
    pub gate: Option<&'a PiwGate>,              // the gate it is parked on
    /// Text of a view value: a JSON string as itself, another inline value as compact JSON, a `$artifact`
    /// reference once fetched (None while pending).
    pub text_of: &'a dyn Fn(&serde_json::Value) -> Option<String>,
}

/// The run folder name used for a pi-workflows run id (safe as one path component, reversible).
pub fn folder_name(run_id: &str) -> String {
    let mut out = String::with_capacity(FOLDER_PREFIX.len() + run_id.len());
    out.push_str(FOLDER_PREFIX);
    for &b in run_id.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0f));
        }
    }
    out
}

/// The run id of a mirror folder, None for a folder this module did not write.
pub fn run_id_of_folder(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    let encoded = name.strip_prefix(FOLDER_PREFIX)?;
    percent_decode(encoded)
}

/// True when `dir` is a run folder inside `mirror_root` (so callers can route answers and cancels).
pub fn is_mirror_dir(mirror_root: &Path, dir: &Path) -> bool {
    if run_id_of_folder(dir).is_none() {
        return false;
    }
    if dir.parent() == Some(mirror_root) {
        return true;
    }
    if let (Ok(root_c), Ok(dir_c)) = (fs::canonicalize(mirror_root), fs::canonicalize(dir)) {
        return dir_c.parent() == Some(root_c.as_path());
    }
    false
}

/// Write or update the run's folder under `mirror_root` so that `bundle::list_runs(mirror_root)` and
/// `bundle::load_run` return the run as the screens should show it. Idempotent: writing the same input twice
/// changes no file. Each file is replaced atomically (temp file + rename in the same directory), so a reader on
/// another thread never sees a half-written file; append-only files (session events, the trace) are appended to.
pub fn write_run(mirror_root: &Path, input: &MirrorInput<'_>) -> Result<PathBuf> {
    write_run_at(mirror_root, input, None)
}

fn write_run_at(
    mirror_root: &Path,
    input: &MirrorInput<'_>,
    connected_now_ms: Option<i64>,
) -> Result<PathBuf> {
    let run_id = extract_run_id(input).ok_or_else(|| anyhow!("mirror input has no runId"))?;
    let run_dir = mirror_root.join(folder_name(&run_id));
    fs::create_dir_all(run_dir.join("projections"))
        .with_context(|| format!("creating projections dir in {}", run_dir.display()))?;

    let mut meta = load_meta(&run_dir);
    let orig_meta = meta.clone();

    let view_val = input.view.map(|rv| &rv.view);
    let state_val = view_val.and_then(|v| v.get("state")).filter(|v| v.is_object());
    let queue_val = view_val.and_then(|v| v.get("queue")).filter(|v| v.is_object());
    let display_val = view_val
        .and_then(|v| v.get("display"))
        .or_else(|| input.summary.and_then(|s| s.get("display")))
        .filter(|v| v.is_object());
    let manifest_src = view_val
        .and_then(|v| v.get("manifest"))
        .or_else(|| input.summary.and_then(|s| s.get("manifest")))
        .filter(|v| v.is_object());
    let workflow_val = view_val
        .and_then(|v| v.get("workflow"))
        .filter(|v| v.is_object());

    let flow_name = str_at(state_val, "workflowName")
        .or_else(|| str_at(workflow_val, "name"))
        .or_else(|| str_at(queue_val, "workflowName"))
        .or_else(|| str_at(manifest_src, "workflowName"))
        .or_else(|| str_at(input.summary, "workflowName"))
        .unwrap_or("")
        .to_string();

    let run_title = str_at(state_val, "runTitle")
        .or_else(|| str_at(manifest_src, "runTitle"))
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let flow_path = extract_flow_path(manifest_src, queue_val);

    let raw_status = str_at(display_val, "status")
        .or_else(|| str_at(state_val, "status"))
        .or_else(|| str_at(manifest_src, "status"))
        .unwrap_or("unknown");

    let (acpx_status, status_note) = map_status(raw_status);

    let started_at = str_at(state_val, "startedAt")
        .or_else(|| str_at(queue_val, "startedAt"))
        .or_else(|| str_at(manifest_src, "startedAt"))
        .or_else(|| str_at(queue_val, "createdAt"))
        .or_else(|| str_at(input.summary, "createdAt"))
        .map(ToOwned::to_owned)
        .unwrap_or_default();

    let finished_at = str_at(state_val, "finishedAt")
        .or_else(|| str_at(queue_val, "finishedAt"))
        .or_else(|| str_at(manifest_src, "finishedAt"))
        .map(ToOwned::to_owned);

    let mut updated_ms = [
        str_at(state_val, "updatedAt").and_then(parse_ts),
        str_at(queue_val, "updatedAt").and_then(parse_ts),
        str_at(input.summary, "updatedAt").and_then(parse_ts),
        str_at(state_val, "currentNodeStartedAt").and_then(parse_ts),
        parse_ts(&started_at),
        finished_at.as_deref().and_then(parse_ts),
        input
            .activity
            .iter()
            .filter_map(|a| parse_ts(&a.at))
            .max(),
    ]
    .into_iter()
    .flatten()
    .max();

    if acpx_status == "running" {
        if let Some(now_ms) = connected_now_ms {
            updated_ms = Some(updated_ms.map_or(now_ms, |u| u.max(now_ms)));
        }
    }

    let updated_at = updated_ms
        .map(fmt_iso_ms)
        .or_else(|| {
            str_at(state_val, "updatedAt")
                .or_else(|| str_at(input.summary, "updatedAt"))
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| started_at.clone());

    let current_node = str_at(state_val, "currentNode").map(ToOwned::to_owned);
    let current_attempt_id = str_at(state_val, "currentAttemptId").map(ToOwned::to_owned);
    let current_node_started_at = str_at(state_val, "currentNodeStartedAt").map(ToOwned::to_owned);

    let waiting_on = input
        .gate
        .map(|g| g.node_id.clone())
        .or_else(|| str_at(state_val, "waitingOn").map(ToOwned::to_owned));

    let error_text = str_at(state_val, "error")
        .or_else(|| {
            display_val
                .and_then(|d| d.get("reasonContent"))
                .and_then(|v| (input.text_of)(v))
                .as_deref()
                .map(|_| "")
                .and_then(|_| None)
        })
        .map(ToOwned::to_owned)
        .or_else(|| {
            if matches!(acpx_status, "failed" | "timed_out") {
                display_val
                    .and_then(|d| d.get("reasonContent"))
                    .and_then(|v| (input.text_of)(v))
                    .filter(|s| !s.is_empty())
                    .or_else(|| str_at(display_val, "reason").map(ToOwned::to_owned))
            } else {
                None
            }
        });

    let status_detail = if let Some(gate) = input.gate {
        if !gate.question.is_empty() {
            Some(gate.question.clone())
        } else {
            Some(gate.node_id.clone())
        }
    } else if let Some(note) = status_note {
        match str_at(state_val, "statusDetail").or_else(|| str_at(display_val, "reason")) {
            Some(d) if !d.is_empty() && !d.eq_ignore_ascii_case(note) => {
                Some(format!("{note}: {d}"))
            }
            _ => Some(note.to_string()),
        }
    } else {
        str_at(state_val, "statusDetail")
            .or_else(|| str_at(display_val, "reason"))
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    };

    // Build flow.json from workflow snapshot (when available).
    let mut node_meta_map: HashMap<String, NodeMeta> = HashMap::new();
    if let Some(wf) = workflow_val {
        let flow_json = build_flow_json(wf, &flow_name, input.gate, &mut node_meta_map);
        let flow_bytes = serde_json::to_vec_pretty(&flow_json)?;
        write_file_if_changed(&run_dir.join("flow.json"), &flow_bytes)?;
    }

    // Append new activity events to sessions/main/events.ndjson.
    let mut new_session_lines: Vec<String> = Vec::new();
    let mut sorted_activity: Vec<&StepActivity> = input
        .activity
        .iter()
        .filter(|a| a.seq > meta.max_activity_seq)
        .collect();
    sorted_activity.sort_by_key(|a| a.seq);

    for act in sorted_activity {
        if act.seq > meta.max_activity_seq {
            meta.max_activity_seq = act.seq;
        }
        match act.kind {
            ActivityKind::Thought => {
                if !act.text.is_empty() {
                    let seq = next_session_seq(&mut meta, &act.attempt_id);
                    let line = json!({
                        "seq": seq,
                        "at": act.at,
                        "direction": "inbound",
                        "message": {
                            "method": "session/update",
                            "params": {
                                "update": {
                                    "sessionUpdate": "agent_thought_chunk",
                                    "content": { "text": act.text }
                                }
                            }
                        }
                    });
                    new_session_lines.push(serde_json::to_string(&line)?);
                }
            }
            ActivityKind::Text => {
                if !act.text.is_empty() {
                    let seq = next_session_seq(&mut meta, &act.attempt_id);
                    let line = json!({
                        "seq": seq,
                        "at": act.at,
                        "direction": "inbound",
                        "message": {
                            "method": "session/update",
                            "params": {
                                "update": {
                                    "sessionUpdate": "agent_message_chunk",
                                    "content": { "text": act.text }
                                }
                            }
                        }
                    });
                    new_session_lines.push(serde_json::to_string(&line)?);
                }
            }
            ActivityKind::Tool => {
                if let Some(ref tool) = act.tool {
                    let tool_key = format!("{}:{}", act.attempt_id, tool.tool_call_id);
                    let is_first = meta.seen_tools.insert(tool_key);
                    let seq = next_session_seq(&mut meta, &act.attempt_id);
                    let status = tool.status.as_deref().unwrap_or("in_progress");
                    let update = if is_first {
                        let mut u = Map::new();
                        u.insert("sessionUpdate".to_string(), json!("tool_call"));
                        u.insert("toolCallId".to_string(), json!(tool.tool_call_id));
                        u.insert("title".to_string(), json!(tool.title));
                        u.insert("status".to_string(), json!(status));
                        if let Some(ref k) = tool.kind {
                            u.insert("kind".to_string(), json!(k));
                        }
                        Value::Object(u)
                    } else {
                        json!({
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": tool.tool_call_id,
                            "title": tool.title,
                            "status": status
                        })
                    };
                    let line = json!({
                        "seq": seq,
                        "at": act.at,
                        "direction": "inbound",
                        "message": {
                            "method": "session/update",
                            "params": { "update": update }
                        }
                    });
                    new_session_lines.push(serde_json::to_string(&line)?);
                }
            }
            ActivityKind::Status => {}
        }
    }

    // Build steps from view (graphSteps merged with state.steps).
    let mut steps_out: Vec<Value> = Vec::new();
    let mut trace_candidates: Vec<(String, String, Value)> = Vec::new();
    let mut primary_agent_name: Option<String> = None;

    if !started_at.is_empty() {
        trace_candidates.push((
            "run_started".to_string(),
            started_at.clone(),
            json!({
                "scope": "run",
                "type": "run_started",
                "payload": { "flowName": flow_name }
            }),
        ));
    }

    if let Some(v) = view_val {
        let merged_steps = collect_steps(v, state_val);
        let mut has_any_acp_session = !meta.conv_ranges.is_empty();

        for step in &merged_steps {
            let attempt_id = str_at(Some(step), "attemptId").unwrap_or("").to_string();
            let node_id = str_at(Some(step), "nodeId").unwrap_or("").to_string();
            let raw_node_type = str_at(Some(step), "nodeType").unwrap_or("");
            let acpx_node_type = map_node_type(raw_node_type);
            let outcome = str_at(Some(step), "outcome").unwrap_or("ok");
            let s_started = str_at(Some(step), "startedAt").unwrap_or("").to_string();
            let s_finished = str_at(Some(step), "finishedAt").unwrap_or("").to_string();

            let prompt_text = step
                .get("prompt")
                .and_then(|p| if p.is_null() { None } else { (input.text_of)(p) });

            let output_val = step
                .get("output")
                .map(|o| resolve_output_value(o, input.text_of))
                .unwrap_or(Value::Null);

            let step_error = str_at(Some(step), "error").map(ToOwned::to_owned);
            let nmeta = node_meta_map.get(&node_id);
            let agent_name = nmeta.and_then(|m| m.agent.clone());
            let model_name = nmeta.and_then(|m| m.model.clone());
            if primary_agent_name.is_none() && agent_name.is_some() {
                primary_agent_name = agent_name.clone();
            }

            // If this is a finished acp step with no activity recorded yet, synthesize
            // conversation events from its prompt and output so the detail panel shows it.
            if acpx_node_type == "acp" && !attempt_id.is_empty() {
                has_any_acp_session = true;
                if !meta.conv_ranges.contains_key(&attempt_id) {
                    if let Some(ref prompt) = prompt_text {
                        if !prompt.is_empty() {
                            let seq = next_session_seq(&mut meta, &attempt_id);
                            let line = json!({
                                "seq": seq,
                                "at": s_started,
                                "direction": "outbound",
                                "message": {
                                    "method": "session/prompt",
                                    "params": { "prompt": [{ "text": prompt }] }
                                }
                            });
                            new_session_lines.push(serde_json::to_string(&line)?);
                        }
                    }
                    let msg_text = step
                        .get("output")
                        .and_then(|o| if o.is_null() { None } else { (input.text_of)(o) })
                        .filter(|s| !s.is_empty());
                    if let Some(text) = msg_text {
                        let seq = next_session_seq(&mut meta, &attempt_id);
                        let line = json!({
                            "seq": seq,
                            "at": s_finished,
                            "direction": "inbound",
                            "message": {
                                "method": "session/update",
                                "params": {
                                    "update": {
                                        "sessionUpdate": "agent_message_chunk",
                                        "content": { "text": text }
                                    }
                                }
                            }
                        });
                        new_session_lines.push(serde_json::to_string(&line)?);
                    }
                }
            }

            let mut trace_obj = Map::new();
            if let Some(act_val) = step.get("action").filter(|a| a.is_object()) {
                trace_obj.insert("action".to_string(), act_val.clone());
            }
            if let Some(&(start_seq, end_seq)) = meta.conv_ranges.get(&attempt_id) {
                trace_obj.insert("sessionId".to_string(), json!(SESSION_ID));
                trace_obj.insert(
                    "conversation".to_string(),
                    json!({
                        "sessionId": SESSION_ID,
                        "eventStartSeq": start_seq,
                        "eventEndSeq": end_seq
                    }),
                );
            }

            let mut step_obj = Map::new();
            step_obj.insert("attemptId".to_string(), json!(attempt_id));
            step_obj.insert("nodeId".to_string(), json!(node_id));
            step_obj.insert("nodeType".to_string(), json!(acpx_node_type));
            step_obj.insert("outcome".to_string(), json!(outcome));
            step_obj.insert("startedAt".to_string(), json!(s_started));
            step_obj.insert("finishedAt".to_string(), json!(s_finished));
            step_obj.insert(
                "promptText".to_string(),
                prompt_text.map(Value::String).unwrap_or(Value::Null),
            );
            step_obj.insert("rawText".to_string(), Value::Null);
            step_obj.insert("output".to_string(), output_val);
            if let Some(ref err) = step_error {
                step_obj.insert("error".to_string(), json!(err));
            }
            if let Some(ref aname) = agent_name {
                step_obj.insert("agent".to_string(), json!({ "agentName": aname }));
            }
            if acpx_node_type == "acp" {
                let mut sess_obj = Map::new();
                sess_obj.insert("handle".to_string(), json!(SESSION_ID));
                if let Some(ref aname) = agent_name {
                    sess_obj.insert("agentName".to_string(), json!(aname));
                }
                if let Some(ref mname) = model_name {
                    sess_obj.insert(
                        "requestedSettings".to_string(),
                        json!({ "model": mname }),
                    );
                }
                step_obj.insert("session".to_string(), Value::Object(sess_obj));
            }
            step_obj.insert("trace".to_string(), Value::Object(trace_obj));
            steps_out.push(Value::Object(step_obj));

            // Trace events for this step.
            if !attempt_id.is_empty() {
                trace_candidates.push((
                    format!("node_started:{attempt_id}"),
                    s_started.clone(),
                    json!({
                        "scope": "node",
                        "type": "node_started",
                        "nodeId": node_id,
                        "attemptId": attempt_id,
                        "payload": { "nodeType": acpx_node_type }
                    }),
                ));
                if let Some(act_val) = step.get("action").filter(|a| a.is_object()) {
                    if str_at(Some(act_val), "actionType") == Some("shell") {
                        trace_candidates.push((
                            format!("action_prepared:{attempt_id}"),
                            s_started.clone(),
                            json!({
                                "scope": "action",
                                "type": "action_prepared",
                                "nodeId": node_id,
                                "attemptId": attempt_id,
                                "payload": { "action": act_val }
                            }),
                        ));
                        trace_candidates.push((
                            format!("action_completed:{attempt_id}"),
                            s_finished.clone(),
                            json!({
                                "scope": "action",
                                "type": "action_completed",
                                "nodeId": node_id,
                                "attemptId": attempt_id,
                                "payload": { "action": act_val }
                            }),
                        ));
                    }
                }
                let dur_ms = duration_between(&s_started, &s_finished);
                trace_candidates.push((
                    format!("node_outcome:{attempt_id}"),
                    s_finished.clone(),
                    json!({
                        "scope": "node",
                        "type": "node_outcome",
                        "nodeId": node_id,
                        "attemptId": attempt_id,
                        "payload": {
                            "nodeType": acpx_node_type,
                            "outcome": outcome,
                            "durationMs": dur_ms,
                            "error": step_error
                        }
                    }),
                ));
            }
        }

        // Running node trace event (if not yet in finished steps).
        if acpx_status == "running" {
            if let Some(ref cnode) = current_node {
                let cattempt = current_attempt_id
                    .clone()
                    .unwrap_or_else(|| format!("{cnode}#live"));
                let ctype = node_meta_map
                    .get(cnode)
                    .map(|m| m.acpx_kind.as_str())
                    .unwrap_or("acp");
                if ctype == "acp" {
                    has_any_acp_session = true;
                }
                let cat = current_node_started_at
                    .clone()
                    .unwrap_or_else(|| updated_at.clone());
                trace_candidates.push((
                    format!("node_started:{cattempt}"),
                    cat,
                    json!({
                        "scope": "node",
                        "type": "node_started",
                        "nodeId": cnode,
                        "attemptId": cattempt,
                        "payload": { "nodeType": ctype }
                    }),
                ));
            }
        }

        if has_any_acp_session {
            trace_candidates.push((
                "session_bound:main".to_string(),
                started_at.clone(),
                json!({
                    "scope": "session",
                    "type": "session_bound",
                    "sessionId": SESSION_ID,
                    "payload": { "sessionId": SESSION_ID, "handle": SESSION_ID }
                }),
            ));
        }
    }

    // If the run is waiting on a gate, ensure a checkpoint step exists with `output.next`
    // so `bundle::gate_choices` returns the choices.
    if let Some(gate) = input.gate {
        let mut out_obj = Map::new();
        out_obj.insert("checkpoint".to_string(), json!(gate.node_id));
        out_obj.insert("summary".to_string(), json!(gate.question));
        if !gate.choices.is_empty() {
            let mut next_map = Map::new();
            for ch in &gate.choices {
                next_map.insert(
                    ch.label.clone(),
                    Value::String(answer_target(&gate.run_id, &ch.key)),
                );
            }
            out_obj.insert("next".to_string(), Value::Object(next_map));
        }
        let gate_output = Value::Object(out_obj);

        let existing_pos = steps_out
            .iter()
            .rposition(|s| str_at(Some(s), "nodeId") == Some(gate.node_id.as_str()));
        if let Some(pos) = existing_pos {
            if let Some(obj) = steps_out[pos].as_object_mut() {
                obj.insert("output".to_string(), gate_output);
            }
        } else {
            let at = updated_at.clone();
            steps_out.push(json!({
                "attemptId": gate.attempt_id,
                "nodeId": gate.node_id,
                "nodeType": "checkpoint",
                "outcome": "ok",
                "startedAt": at,
                "finishedAt": at,
                "promptText": null,
                "rawText": null,
                "output": gate_output,
                "trace": {}
            }));
        }
    }

    // Terminal run trace event.
    match acpx_status {
        "completed" => {
            let at = finished_at.clone().unwrap_or_else(|| updated_at.clone());
            trace_candidates.push((
                "run_finished:completed".to_string(),
                at,
                json!({
                    "scope": "run",
                    "type": "run_completed",
                    "payload": { "flowName": flow_name }
                }),
            ));
        }
        "failed" | "timed_out" => {
            let at = finished_at.clone().unwrap_or_else(|| updated_at.clone());
            trace_candidates.push((
                format!("run_finished:{acpx_status}"),
                at,
                json!({
                    "scope": "run",
                    "type": "run_failed",
                    "payload": { "error": error_text }
                }),
            ));
        }
        _ => {}
    }

    // Append new session events to sessions/main/events.ndjson.
    if !new_session_lines.is_empty() {
        let sess_dir = run_dir.join("sessions").join(SESSION_ID);
        fs::create_dir_all(&sess_dir)?;
        append_lines(&sess_dir.join("events.ndjson"), &new_session_lines)?;
    }

    // Append new trace events to trace.ndjson (never duplicating).
    let mut new_trace_lines: Vec<String> = Vec::new();
    for (key, at, mut ev) in trace_candidates {
        if meta.written_trace_keys.insert(key) {
            meta.last_trace_seq += 1;
            if let Some(obj) = ev.as_object_mut() {
                obj.insert("seq".to_string(), json!(meta.last_trace_seq));
                obj.insert("at".to_string(), json!(at));
                obj.insert("runId".to_string(), json!(run_id));
            }
            new_trace_lines.push(serde_json::to_string(&ev)?);
        }
    }
    if !new_trace_lines.is_empty() {
        append_lines(&run_dir.join("trace.ndjson"), &new_trace_lines)?;
    }

    // Write projections/steps.json when we have a view or gate.
    if view_val.is_some() || input.gate.is_some() {
        let steps_bytes = serde_json::to_vec_pretty(&Value::Array(steps_out))?;
        write_file_if_changed(&run_dir.join("projections/steps.json"), &steps_bytes)?;
    }

    // Session bindings for manifest / live / run projections.
    let session_bindings = {
        let mut b = Map::new();
        let mut entry = Map::new();
        entry.insert("key".to_string(), json!(SESSION_ID));
        entry.insert("handle".to_string(), json!(SESSION_ID));
        entry.insert("bundleId".to_string(), json!(SESSION_ID));
        if let Some(ref aname) = primary_agent_name {
            entry.insert("agentName".to_string(), json!(aname));
        }
        b.insert(SESSION_ID.to_string(), Value::Object(entry));
        Value::Object(b)
    };

    // Write projections/run.json.
    let run_input = state_val
        .and_then(|s| s.get("input"))
        .map(|v| resolve_output_value(v, input.text_of))
        .unwrap_or(Value::Null);
    let run_proj = json!({
        "input": run_input,
        "sessionBindings": session_bindings
    });
    let run_proj_bytes = serde_json::to_vec_pretty(&run_proj)?;
    write_file_if_changed(&run_dir.join("projections/run.json"), &run_proj_bytes)?;

    // Write projections/live.json.
    let mut live_obj = Map::new();
    live_obj.insert("runId".to_string(), json!(run_id));
    live_obj.insert("flowName".to_string(), json!(flow_name));
    if let Some(ref t) = run_title {
        live_obj.insert("runTitle".to_string(), json!(t));
    }
    if let Some(ref fp) = flow_path {
        live_obj.insert("flowPath".to_string(), json!(fp));
    }
    live_obj.insert("startedAt".to_string(), json!(started_at));
    live_obj.insert("updatedAt".to_string(), json!(updated_at));
    if let Some(ref fin) = finished_at {
        live_obj.insert("finishedAt".to_string(), json!(fin));
    }
    live_obj.insert("status".to_string(), json!(acpx_status));
    if let Some(ref cn) = current_node {
        live_obj.insert("currentNode".to_string(), json!(cn));
    }
    if let Some(ref wo) = waiting_on {
        live_obj.insert("waitingOn".to_string(), json!(wo));
    }
    if let Some(ref sd) = status_detail {
        live_obj.insert("statusDetail".to_string(), json!(sd));
    }
    if let Some(ref err) = error_text {
        live_obj.insert("error".to_string(), json!(err));
    }
    live_obj.insert("sessionBindings".to_string(), session_bindings);
    let live_bytes = serde_json::to_vec_pretty(&Value::Object(live_obj))?;
    write_file_if_changed(&run_dir.join("projections/live.json"), &live_bytes)?;

    // Write manifest.json.
    let mut manifest_obj = Map::new();
    manifest_obj.insert("schema".to_string(), json!("acpx.flow-run-bundle.v1"));
    manifest_obj.insert("runId".to_string(), json!(run_id));
    manifest_obj.insert("flowName".to_string(), json!(flow_name));
    if let Some(ref t) = run_title {
        manifest_obj.insert("runTitle".to_string(), json!(t));
    }
    if let Some(ref fp) = flow_path {
        manifest_obj.insert("flowPath".to_string(), json!(fp));
    }
    manifest_obj.insert("startedAt".to_string(), json!(started_at));
    manifest_obj.insert("status".to_string(), json!(acpx_status));
    manifest_obj.insert("traceSchema".to_string(), json!("acpx.flow-trace-event.v1"));
    manifest_obj.insert(
        "paths".to_string(),
        json!({
            "flow": "flow.json",
            "trace": "trace.ndjson",
            "runProjection": "projections/run.json",
            "liveProjection": "projections/live.json",
            "stepsProjection": "projections/steps.json",
            "sessionsDir": "sessions",
            "artifactsDir": "artifacts"
        }),
    );
    manifest_obj.insert(
        "sessions".to_string(),
        json!([{
            "id": SESSION_ID,
            "handle": SESSION_ID,
            "eventsPath": format!("sessions/{SESSION_ID}/events.ndjson")
        }]),
    );
    let manifest_bytes = serde_json::to_vec_pretty(&Value::Object(manifest_obj))?;
    write_file_if_changed(&run_dir.join("manifest.json"), &manifest_bytes)?;

    if meta != orig_meta {
        let meta_bytes = serde_json::to_vec_pretty(&meta)?;
        write_file_if_changed(&run_dir.join(META_FILE), &meta_bytes)?;
    }

    Ok(run_dir)
}

/// Keeps the mirror current on its own thread: watches the runs list, mirrors every run's summary, and for runs
/// that are active or were asked for with `want_run` also watches the view, the activity and the session and
/// mirrors those. Never blocks the caller.
pub struct PiwMirror {
    client: Arc<PiwClient>,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    wanted: Arc<Mutex<HashSet<String>>>,
    gates: Arc<Mutex<HashMap<String, PiwGate>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl PiwMirror {
    pub fn start(client: Arc<PiwClient>, mirror_root: PathBuf) -> Result<PiwMirror> {
        fs::create_dir_all(&mirror_root)
            .with_context(|| format!("creating mirror root {}", mirror_root.display()))?;
        client.watch_runs();

        let stop = Arc::new(AtomicBool::new(false));
        let wanted = Arc::new(Mutex::new(HashSet::new()));
        let gates = Arc::new(Mutex::new(HashMap::new()));

        let w_client = Arc::clone(&client);
        let w_root = mirror_root.clone();
        let w_stop = Arc::clone(&stop);
        let w_wanted = Arc::clone(&wanted);
        let w_gates = Arc::clone(&gates);

        let handle = thread::Builder::new()
            .name("piw-mirror".to_string())
            .spawn(move || {
                mirror_worker(w_client, w_root, w_stop, w_wanted, w_gates);
            })
            .context("spawning piw-mirror thread")?;

        Ok(Self {
            client,
            root: mirror_root,
            stop,
            wanted,
            gates,
            worker: Mutex::new(Some(handle)),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A screen opened this run: mirror it in full from now on (finished runs included).
    pub fn want_run(&self, run_id: &str) {
        let mut guard = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(run_id.to_string());
    }

    /// The screen closed it: a finished run stops being watched.
    pub fn release_run(&self, run_id: &str) {
        let mut guard = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(run_id);
    }

    /// The gate a run is parked on, for the answer path.
    pub fn gate(&self, run_id: &str) -> Option<PiwGate> {
        let guard = self.gates.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(run_id).cloned()
    }

    /// "connected" / "connecting" / "reconnecting" / the last error, for a status line.
    pub fn status_line(&self) -> String {
        match self.client.state() {
            ConnState::Connected => "connected".to_string(),
            ConnState::Connecting => "connecting".to_string(),
            ConnState::Reconnecting => self
                .client
                .last_error()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "reconnecting".to_string()),
            ConnState::Closed => self
                .client
                .last_error()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "closed".to_string()),
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let mut guard = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PiwMirror {
    fn drop(&mut self) {
        self.stop();
    }
}

fn mirror_worker(
    client: Arc<PiwClient>,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    wanted: Arc<Mutex<HashSet<String>>>,
    gates: Arc<Mutex<HashMap<String, PiwGate>>>,
) {
    let mut last_gen: u64 = u64::MAX;
    let mut last_wanted: HashSet<String> = HashSet::new();
    let mut watched_runs: HashSet<String> = HashSet::new();
    let mut watched_sessions: HashSet<String> = HashSet::new();
    let mut last_stall_refresh: HashMap<String, i64> = HashMap::new();

    while !stop.load(Ordering::Relaxed) {
        let conn_state = client.state();
        if conn_state == ConnState::Closed {
            break;
        }
        if conn_state == ConnState::Reconnecting {
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            continue;
        }

        let cur_gen = client.generation();
        let cur_wanted = wanted.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let now_ms = Utc::now().timestamp_millis();

        let gen_changed = cur_gen != last_gen || cur_wanted != last_wanted;
        let connected = conn_state == ConnState::Connected;

        if gen_changed || connected {
            let summaries = client.runs();
            let mut summary_by_id: HashMap<String, Value> = HashMap::new();
            let mut active_ids: HashSet<String> = HashSet::new();
            let mut running_ids: HashSet<String> = HashSet::new();

            for s in summaries {
                if let Some(id) = str_at(Some(&s), "runId").map(ToOwned::to_owned) {
                    let st = s
                        .pointer("/display/status")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let is_live = s.get("live").and_then(Value::as_bool).unwrap_or(false);
                    if is_live || matches!(st, "running" | "waiting" | "queued" | "paused") {
                        active_ids.insert(id.clone());
                    }
                    if st == "running" || st == "queued" {
                        running_ids.insert(id.clone());
                    }
                    summary_by_id.insert(id, s);
                }
            }

            let mut desired_full: HashSet<String> = active_ids;
            desired_full.extend(cur_wanted.iter().cloned());

            for id in desired_full.difference(&watched_runs) {
                client.watch_run(id);
                client.watch_activity(id);
            }
            for id in watched_runs.difference(&desired_full) {
                client.unwatch_run(id);
            }
            watched_runs = desired_full.clone();

            let mut all_ids: HashSet<String> = summary_by_id.keys().cloned().collect();
            all_ids.extend(desired_full.iter().cloned());

            let mut next_gates: HashMap<String, PiwGate> = HashMap::new();

            for id in &all_ids {
                let is_running = running_ids.contains(id);
                let needs_stall = connected
                    && is_running
                    && last_stall_refresh
                        .get(id)
                        .is_none_or(|&t| now_ms.saturating_sub(t) >= STALL_REFRESH_INTERVAL_MS);

                if !gen_changed && !needs_stall {
                    continue;
                }

                let view = if desired_full.contains(id) {
                    client.run_view(id)
                } else {
                    None
                };

                let mut gate = None;
                if let Some(ref rv) = view {
                    if let Some(sess_id) = rv
                        .view
                        .pointer("/queue/originSessionId")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        if watched_sessions.insert(sess_id.to_string()) {
                            client.watch_session(sess_id, false);
                        }
                        if let Some(sess_val) = client.session(sess_id) {
                            gate = gate_from(rv, &sess_val);
                        }
                    }
                }
                if let Some(ref g) = gate {
                    next_gates.insert(id.clone(), g.clone());
                }

                let activity = if desired_full.contains(id) {
                    client.activity(id, 0)
                } else {
                    Vec::new()
                };

                let text_of = |val: &Value| resolve_client_text(&client, id, val);
                let input = MirrorInput {
                    summary: summary_by_id.get(id),
                    view: view.as_ref(),
                    activity: &activity,
                    gate: gate.as_ref(),
                    text_of: &text_of,
                };
                let connected_now = if connected { Some(now_ms) } else { None };
                let _ = write_run_at(&root, &input, connected_now);
                if needs_stall || (connected && is_running) {
                    last_stall_refresh.insert(id.clone(), now_ms);
                }
            }

            if gen_changed {
                let mut g_guard = gates.lock().unwrap_or_else(|e| e.into_inner());
                *g_guard = next_gates;
                last_gen = cur_gen;
                last_wanted = cur_wanted;
            }
        }

        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

fn resolve_client_text(client: &PiwClient, run_id: &str, val: &Value) -> Option<String> {
    if is_artifact_ref(val) {
        return match client.content(run_id, val) {
            Content::Ready(s) => Some(s),
            Content::Pending | Content::Invalid(_) => None,
        };
    }
    match val {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => serde_json::to_string(other).ok(),
    }
}

// ==== Private Helpers ====

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct MirrorMeta {
    max_activity_seq: u64,
    last_session_seq: u64,
    last_trace_seq: u64,
    written_trace_keys: HashSet<String>,
    seen_tools: HashSet<String>,
    conv_ranges: BTreeMap<String, (u64, u64)>,
}

struct NodeMeta {
    acpx_kind: String,
    agent: Option<String>,
    model: Option<String>,
}

fn load_meta(run_dir: &Path) -> MirrorMeta {
    fs::read_to_string(run_dir.join(META_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn next_session_seq(meta: &mut MirrorMeta, attempt_id: &str) -> u64 {
    meta.last_session_seq += 1;
    let seq = meta.last_session_seq;
    if !attempt_id.is_empty() {
        meta.conv_ranges
            .entry(attempt_id.to_string())
            .and_modify(|(_, end)| *end = seq)
            .or_insert((seq, seq));
    }
    seq
}

fn extract_run_id(input: &MirrorInput<'_>) -> Option<String> {
    input
        .view
        .and_then(|v| {
            str_at(Some(&v.view), "runId")
                .or_else(|| v.view.pointer("/state/runId").and_then(Value::as_str))
                .or_else(|| v.view.pointer("/manifest/runId").and_then(Value::as_str))
                .or_else(|| v.view.pointer("/queue/runId").and_then(Value::as_str))
        })
        .or_else(|| str_at(input.summary, "runId"))
        .or_else(|| input.gate.map(|g| g.run_id.as_str()))
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_flow_path(manifest: Option<&Value>, queue: Option<&Value>) -> Option<String> {
    if let Some(src) = manifest.and_then(|m| m.get("workflowSource")) {
        if let Some(path) = str_at(Some(src), "path").filter(|s| !s.is_empty()) {
            return Some(path.to_string());
        }
        if let Some(id) = str_at(Some(src), "id").filter(|s| !s.is_empty()) {
            return Some(format!("builtin:{id}"));
        }
    }
    str_at(queue, "workflowSourceRef")
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn map_status(raw: &str) -> (&'static str, Option<&'static str>) {
    match raw {
        "running" => ("running", None),
        "waiting" => ("waiting", None),
        "completed" => ("completed", None),
        "failed" => ("failed", None),
        "timed_out" => ("timed_out", None),
        "cancelled" => ("failed", Some("cancelled")),
        "paused" => ("waiting", Some("paused")),
        "queued" => ("running", Some("queued")),
        "ambiguous" => ("failed", Some("ambiguous")),
        _ => ("unknown", None),
    }
}

fn map_node_type(raw: &str) -> &'static str {
    match raw {
        "agent" | "acp" => "acp",
        "action" => "action",
        "compute" | "notify" => "compute",
        "checkpoint" => "checkpoint",
        _ => "unknown",
    }
}

fn build_flow_json(
    wf: &Value,
    fallback_name: &str,
    gate: Option<&PiwGate>,
    out_meta: &mut HashMap<String, NodeMeta>,
) -> Value {
    let name = str_at(Some(wf), "name")
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_name);
    let start_at = str_at(Some(wf), "startAt").unwrap_or("");

    let mut nodes_out = Map::new();
    if let Some(nodes_obj) = wf.get("nodes").and_then(Value::as_object) {
        for (id, nv) in nodes_obj {
            let raw_type = str_at(Some(nv), "nodeType").unwrap_or("");
            let acpx_kind = map_node_type(raw_type);
            let agent = str_at(Some(nv), "agent").map(ToOwned::to_owned);
            let model = str_at(Some(nv), "model").map(ToOwned::to_owned);

            out_meta.insert(
                id.clone(),
                NodeMeta {
                    acpx_kind: acpx_kind.to_string(),
                    agent: agent.clone(),
                    model: model.clone(),
                },
            );

            let mut nobj = Map::new();
            nobj.insert("nodeType".to_string(), json!(acpx_kind));
            if let Some(exec) = str_at(Some(nv), "actionExecution") {
                nobj.insert("actionExecution".to_string(), json!(exec));
            }
            if acpx_kind == "acp" {
                nobj.insert("session".to_string(), json!({ "handle": SESSION_ID }));
            }
            if let Some(ref a) = agent {
                nobj.insert("profile".to_string(), json!(a));
            }
            if let Some(ref m) = model {
                nobj.insert("modelId".to_string(), json!(m));
            }
            if acpx_kind == "checkpoint" {
                let summary = gate
                    .filter(|g| g.node_id == *id && !g.question.is_empty())
                    .map(|g| g.question.clone())
                    .or_else(|| str_at(Some(nv), "summary").map(ToOwned::to_owned));
                if let Some(s) = summary {
                    nobj.insert("summary".to_string(), json!(s));
                }
            }
            nodes_out.insert(id.clone(), Value::Object(nobj));
        }
    }

    let edges_out = wf
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    json!({
        "schema": "acpx.flow-definition-snapshot.v1",
        "name": name,
        "startAt": start_at,
        "nodes": Value::Object(nodes_out),
        "edges": Value::Array(edges_out)
    })
}

fn collect_steps(view: &Value, state: Option<&Value>) -> Vec<Value> {
    let mut full_by_attempt: HashMap<String, Value> = HashMap::new();
    let state_steps = state
        .and_then(|s| s.get("steps"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for s in &state_steps {
        if let Some(aid) = str_at(Some(s), "attemptId") {
            full_by_attempt.insert(aid.to_string(), s.clone());
        }
    }

    let graph_steps = view
        .get("graphSteps")
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty());

    match graph_steps {
        Some(gs) => gs
            .iter()
            .map(|compact| {
                if let Some(aid) = str_at(Some(compact), "attemptId") {
                    if let Some(full) = full_by_attempt.get(aid) {
                        return full.clone();
                    }
                }
                compact.clone()
            })
            .collect(),
        None => state_steps,
    }
}

fn resolve_output_value(val: &Value, text_of: &dyn Fn(&Value) -> Option<String>) -> Value {
    if is_artifact_ref(val) {
        return match text_of(val) {
            Some(s) => serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s)),
            None => Value::Null,
        };
    }
    val.clone()
}

fn is_artifact_ref(val: &Value) -> bool {
    val.as_object()
        .is_some_and(|obj| obj.len() == 1 && obj.contains_key("$artifact"))
}

fn str_at<'a>(val: Option<&'a Value>, key: &str) -> Option<&'a str> {
    val?.get(key)?.as_str()
}

fn duration_between(start_iso: &str, end_iso: &str) -> u64 {
    match (parse_ts(start_iso), parse_ts(end_iso)) {
        (Some(s), Some(e)) if e > s => (e - s) as u64,
        _ => 0,
    }
}

fn fmt_iso_ms(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt: DateTime<Utc>| dt.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn write_file_if_changed(path: &Path, content: &[u8]) -> io::Result<bool> {
    if let Ok(existing) = fs::read(path) {
        if existing == content {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    );
    let tmp_path = path.with_file_name(tmp_name);
    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(content)?;
        let _ = f.sync_all();
    }
    fs::rename(&tmp_path, path)?;
    Ok(true)
}

fn append_lines(path: &Path, lines: &[String]) -> io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    for line in lines {
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
    }
    let _ = f.sync_all();
    Ok(())
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + (nibble - 10)) as char,
        _ => '0',
    }
}

fn from_hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = from_hex_digit(bytes[i + 1])?;
            let lo = from_hex_digit(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        self, FlowEdge, NodeKind, NodeState, Outcome, RunStatus,
    };
    use crate::piw::client::ToolActivity;
    use crate::piw::gate::{GateKind, PiwGateChoice};
    use tempfile::tempdir;

    fn inline_text_of(val: &Value) -> Option<String> {
        match val {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => serde_json::to_string(other).ok(),
        }
    }

    fn base_workflow() -> Value {
        json!({
            "schema": "pi-workflows.definition-snapshot.v1",
            "name": "review-pr",
            "startAt": "analyze",
            "nodes": {
                "analyze": {
                    "nodeType": "agent",
                    "agent": "grok-build",
                    "model": "grok-4"
                },
                "implement": {
                    "nodeType": "agent",
                    "agent": "claude",
                    "model": "claude-opus"
                },
                "gate": {
                    "nodeType": "checkpoint",
                    "summary": "Ship it?"
                }
            },
            "edges": [
                { "from": "analyze", "to": "implement" },
                { "from": "implement", "to": "gate" }
            ]
        })
    }

    fn sample_running_view() -> RunView {
        RunView {
            revision: 3,
            run_revision: 3,
            view: json!({
                "schema": "pi-workflows.run-view.v1",
                "runId": "run-100",
                "revision": 3,
                "runRevision": 3,
                "display": {
                    "status": "running",
                    "activity": "supervised_runner",
                    "controls": ["pause", "cancel"],
                    "reason": null
                },
                "manifest": {
                    "runId": "run-100",
                    "workflowName": "review-pr",
                    "runTitle": "PR #42",
                    "workflowSource": { "kind": "file", "path": "/repo/review.workflow.ts", "hash": "abc" },
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "status": "running"
                },
                "queue": {
                    "runId": "run-100",
                    "workflowName": "review-pr",
                    "workflowSourceRef": "/repo/review.workflow.ts",
                    "status": "running",
                    "originSessionId": "sess-1",
                    "createdAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:01:00.000Z",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "finishedAt": null
                },
                "state": {
                    "runId": "run-100",
                    "workflowName": "review-pr",
                    "runTitle": "PR #42",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:01:00.000Z",
                    "status": "running",
                    "input": { "pr": 42 },
                    "currentNode": "implement",
                    "currentAttemptId": "implement#1",
                    "currentNodeStartedAt": "2026-11-18T19:00:30.000Z",
                    "steps": [
                        {
                            "attemptId": "analyze#1",
                            "nodeId": "analyze",
                            "nodeType": "agent",
                            "outcome": "ok",
                            "startedAt": "2026-11-18T19:00:00.000Z",
                            "finishedAt": "2026-11-18T19:00:25.000Z",
                            "prompt": "Analyze PR #42",
                            "output": "Looks clean"
                        }
                    ]
                },
                "graphSteps": [
                    {
                        "attemptId": "analyze#1",
                        "nodeId": "analyze",
                        "nodeType": "agent",
                        "outcome": "ok",
                        "startedAt": "2026-11-18T19:00:00.000Z",
                        "finishedAt": "2026-11-18T19:00:25.000Z",
                        "prompt": null,
                        "output": null
                    }
                ],
                "workflow": base_workflow()
            }),
        }
    }

    #[test]
    fn test_folder_name_and_run_id_roundtrip_and_is_mirror_dir() {
        let tmp = tempdir().expect("tempdir");
        let weird_id = "org/team run 42?x=1%20";
        let fname = folder_name(weird_id);
        assert!(!fname.contains('/'));
        assert!(!fname.contains(' '));
        let dir = tmp.path().join(&fname);
        assert_eq!(run_id_of_folder(&dir).as_deref(), Some(weird_id));
        assert!(is_mirror_dir(tmp.path(), &dir));
        assert!(!is_mirror_dir(tmp.path(), &tmp.path().join("other-dir")));
        assert!(!is_mirror_dir(tmp.path(), &tmp.path().join("nested").join(&fname)));
    }

    #[test]
    fn test_running_run_with_activity_grows_and_is_idempotent() {
        let tmp = tempdir().expect("tempdir");
        let rv = sample_running_view();
        let act1 = vec![
            StepActivity {
                run_id: "run-100".to_string(),
                node_id: "implement".to_string(),
                attempt_id: "implement#1".to_string(),
                seq: 1,
                at: "2026-11-18T19:00:35.000Z".to_string(),
                kind: ActivityKind::Text,
                text: "Part one ".to_string(),
                tool: None,
            },
            StepActivity {
                run_id: "run-100".to_string(),
                node_id: "implement".to_string(),
                attempt_id: "implement#1".to_string(),
                seq: 2,
                at: "2026-11-18T19:00:36.000Z".to_string(),
                kind: ActivityKind::Tool,
                text: String::new(),
                tool: Some(ToolActivity {
                    tool_call_id: "tc-1".to_string(),
                    title: "cargo test".to_string(),
                    status: Some("in_progress".to_string()),
                    kind: Some("execute".to_string()),
                    input: None,
                    output: None,
                    truncated: false,
                }),
            },
        ];

        let input1 = MirrorInput {
            summary: None,
            view: Some(&rv),
            activity: &act1,
            gate: None,
            text_of: &inline_text_of,
        };
        let dir = write_run(tmp.path(), &input1).expect("write 1");

        // Idempotent second write: mtime of manifest.json should not change (content identical).
        let manifest_before = fs::read(dir.join("manifest.json")).expect("read manifest");
        let trace_before = fs::read_to_string(dir.join("trace.ndjson")).expect("read trace");
        write_run(tmp.path(), &input1).expect("write 1 repeat");
        assert_eq!(fs::read(dir.join("manifest.json")).expect("read"), manifest_before);
        assert_eq!(fs::read_to_string(dir.join("trace.ndjson")).expect("read"), trace_before);

        let run = bundle::load_run(&dir).expect("load_run");
        assert_eq!(run.summary.status, RunStatus::Running);
        assert_eq!(run.summary.title, "PR #42");
        assert_eq!(run.node_state("analyze"), NodeState::Done(Outcome::Ok));
        assert_eq!(run.node_state("implement"), NodeState::Running);
        assert_eq!(run.node_state("gate"), NodeState::NotRun);

        let ev_path = run.session_events_path("main").expect("session events path");
        let ev_text = fs::read_to_string(ev_path).expect("read events");
        assert!(ev_text.contains("Part one "));
        assert!(ev_text.contains("cargo test"));

        // Write again with more activity: text grows, tool updates, no duplicate trace events.
        let mut act2 = act1.clone();
        act2.push(StepActivity {
            run_id: "run-100".to_string(),
            node_id: "implement".to_string(),
            attempt_id: "implement#1".to_string(),
            seq: 3,
            at: "2026-11-18T19:00:40.000Z".to_string(),
            kind: ActivityKind::Text,
            text: "part two.".to_string(),
            tool: None,
        });
        act2.push(StepActivity {
            run_id: "run-100".to_string(),
            node_id: "implement".to_string(),
            attempt_id: "implement#1".to_string(),
            seq: 4,
            at: "2026-11-18T19:00:41.000Z".to_string(),
            kind: ActivityKind::Tool,
            text: String::new(),
            tool: Some(ToolActivity {
                tool_call_id: "tc-1".to_string(),
                title: "cargo test".to_string(),
                status: Some("completed".to_string()),
                kind: Some("execute".to_string()),
                input: None,
                output: None,
                truncated: false,
            }),
        });
        let input2 = MirrorInput {
            summary: None,
            view: Some(&rv),
            activity: &act2,
            gate: None,
            text_of: &inline_text_of,
        };
        write_run(tmp.path(), &input2).expect("write 2");

        let ev_text2 = fs::read_to_string(ev_path).expect("read events 2");
        assert!(ev_text2.contains("part two."));
        assert!(ev_text2.contains("tool_call_update"));
        assert!(ev_text2.contains("completed"));
        assert_eq!(fs::read_to_string(dir.join("trace.ndjson")).expect("trace"), trace_before);

        // Less activity on a later write (e.g. server restart): nothing lost.
        let input3 = MirrorInput {
            summary: None,
            view: Some(&rv),
            activity: &[],
            gate: None,
            text_of: &inline_text_of,
        };
        write_run(tmp.path(), &input3).expect("write 3");
        let ev_text3 = fs::read_to_string(ev_path).expect("read events 3");
        assert_eq!(ev_text3, ev_text2);
    }

    #[test]
    fn test_finished_run_without_activity_uses_step_output_for_conversation() {
        let tmp = tempdir().expect("tempdir");
        let rv = RunView {
            revision: 5,
            run_revision: 5,
            view: json!({
                "runId": "run-fin",
                "display": { "status": "completed", "activity": null, "controls": [], "reason": null },
                "manifest": {
                    "runId": "run-fin",
                    "workflowName": "review-pr",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "finishedAt": "2026-11-18T19:01:00.000Z",
                    "status": "completed"
                },
                "state": {
                    "runId": "run-fin",
                    "workflowName": "review-pr",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:01:00.000Z",
                    "finishedAt": "2026-11-18T19:01:00.000Z",
                    "status": "completed",
                    "input": {},
                    "steps": [{
                        "attemptId": "analyze#1",
                        "nodeId": "analyze",
                        "nodeType": "agent",
                        "outcome": "ok",
                        "startedAt": "2026-11-18T19:00:00.000Z",
                        "finishedAt": "2026-11-18T19:00:50.000Z",
                        "prompt": "Check it",
                        "output": "All tests pass"
                    }]
                },
                "workflow": base_workflow()
            }),
        };
        let input = MirrorInput {
            summary: None,
            view: Some(&rv),
            activity: &[],
            gate: None,
            text_of: &inline_text_of,
        };
        let dir = write_run(tmp.path(), &input).expect("write");
        let run = bundle::load_run(&dir).expect("load");
        assert_eq!(run.summary.status, RunStatus::Completed);
        assert!(run.steps[0].conversation.is_some());
        let ev_text = fs::read_to_string(run.session_events_path("main").expect("path")).expect("read");
        assert!(ev_text.contains("All tests pass"));
    }

    #[test]
    fn test_decision_gate_and_checkpoint_gate() {
        let tmp = tempdir().expect("tempdir");
        let rv = RunView {
            revision: 4,
            run_revision: 4,
            view: json!({
                "runId": "run-gate",
                "display": { "status": "waiting", "activity": null, "controls": ["human-answer"], "reason": null },
                "state": {
                    "runId": "run-gate",
                    "workflowName": "review-pr",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:01:00.000Z",
                    "status": "waiting",
                    "waitingOn": "gate",
                    "input": {},
                    "steps": []
                },
                "workflow": base_workflow()
            }),
        };
        let decision_gate = PiwGate {
            run_id: "run-gate".to_string(),
            node_id: "gate".to_string(),
            attempt_id: "gate#1".to_string(),
            session_id: "sess-1".to_string(),
            kind: GateKind::Decision,
            request_id: "req-1".to_string(),
            revision: 1,
            question: "Deploy to prod?".to_string(),
            choices: vec![
                PiwGateChoice {
                    key: "approve".to_string(),
                    label: "Approve".to_string(),
                    text_input: None,
                },
                PiwGateChoice {
                    key: "reject".to_string(),
                    label: "Reject".to_string(),
                    text_input: None,
                },
            ],
        };
        let input = MirrorInput {
            summary: None,
            view: Some(&rv),
            activity: &[],
            gate: Some(&decision_gate),
            text_of: &inline_text_of,
        };
        let dir = write_run(tmp.path(), &input).expect("write decision");
        let run = bundle::load_run(&dir).expect("load decision");
        assert_eq!(run.summary.status, RunStatus::Waiting);
        assert_eq!(run.summary.waiting_on.as_deref(), Some("gate"));
        assert_eq!(run.summary.status_detail.as_deref(), Some("Deploy to prod?"));
        let choices = bundle::gate_choices(&run);
        assert_eq!(
            choices,
            vec![
                ("Approve".to_string(), answer_target("run-gate", "approve")),
                ("Reject".to_string(), answer_target("run-gate", "reject")),
            ]
        );

        // Checkpoint gate (no choices)
        let cp_gate = PiwGate {
            choices: vec![],
            kind: GateKind::Checkpoint,
            question: "Provide release notes".to_string(),
            ..decision_gate
        };
        let input_cp = MirrorInput {
            summary: None,
            view: Some(&rv),
            activity: &[],
            gate: Some(&cp_gate),
            text_of: &inline_text_of,
        };
        write_run(tmp.path(), &input_cp).expect("write cp");
        let run_cp = bundle::load_run(&dir).expect("load cp");
        assert_eq!(run_cp.summary.status, RunStatus::Waiting);
        assert_eq!(
            run_cp.summary.status_detail.as_deref(),
            Some("Provide release notes")
        );
        assert!(bundle::gate_choices(&run_cp).is_empty());
    }

    #[test]
    fn test_failed_and_cancelled_runs() {
        let tmp = tempdir().expect("tempdir");
        let fail_view = RunView {
            revision: 1,
            run_revision: 1,
            view: json!({
                "runId": "run-fail",
                "display": { "status": "failed", "reason": "boom" },
                "state": {
                    "runId": "run-fail",
                    "workflowName": "ci",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:00:10.000Z",
                    "finishedAt": "2026-11-18T19:00:10.000Z",
                    "status": "failed",
                    "error": "exit code 1",
                    "input": {},
                    "steps": []
                }
            }),
        };
        let dir_f = write_run(
            tmp.path(),
            &MirrorInput {
                summary: None,
                view: Some(&fail_view),
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
        )
        .expect("write fail");
        let run_f = bundle::load_run(&dir_f).expect("load fail");
        assert_eq!(run_f.summary.status, RunStatus::Failed);
        assert_eq!(run_f.summary.error.as_deref(), Some("exit code 1"));

        let cancel_view = RunView {
            revision: 1,
            run_revision: 1,
            view: json!({
                "runId": "run-cancel",
                "display": { "status": "cancelled", "reason": "stopped by user" },
                "state": {
                    "runId": "run-cancel",
                    "workflowName": "ci",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:00:10.000Z",
                    "finishedAt": "2026-11-18T19:00:10.000Z",
                    "status": "cancelled",
                    "input": {},
                    "steps": []
                }
            }),
        };
        let dir_c = write_run(
            tmp.path(),
            &MirrorInput {
                summary: None,
                view: Some(&cancel_view),
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
        )
        .expect("write cancel");
        let run_c = bundle::load_run(&dir_c).expect("load cancel");
        assert_eq!(run_c.summary.status, RunStatus::Failed);
        assert!(
            run_c
                .summary
                .status_detail
                .as_deref()
                .unwrap_or("")
                .contains("cancelled")
        );
    }

    #[test]
    fn test_branching_edges_and_agent_settings() {
        let tmp = tempdir().expect("tempdir");
        let rv = RunView {
            revision: 2,
            run_revision: 2,
            view: json!({
                "runId": "run-branch",
                "display": { "status": "completed" },
                "state": {
                    "runId": "run-branch",
                    "workflowName": "brancher",
                    "startedAt": "2026-11-18T19:00:00.000Z",
                    "updatedAt": "2026-11-18T19:00:10.000Z",
                    "status": "completed",
                    "input": {},
                    "steps": [{
                        "attemptId": "triage#1",
                        "nodeId": "triage",
                        "nodeType": "agent",
                        "outcome": "ok",
                        "startedAt": "2026-11-18T19:00:00.000Z",
                        "finishedAt": "2026-11-18T19:00:05.000Z",
                        "prompt": "triage",
                        "output": { "route": "fast" }
                    }]
                },
                "workflow": {
                    "name": "brancher",
                    "startAt": "triage",
                    "nodes": {
                        "triage": { "nodeType": "agent", "agent": "grok-build", "model": "grok-4" },
                        "fast": { "nodeType": "action", "actionExecution": "shell" },
                        "deep": { "nodeType": "compute" }
                    },
                    "edges": [
                        { "from": "triage", "switch": { "on": "$.route", "cases": { "fast": "fast", "deep": "deep" } } }
                    ]
                }
            }),
        };
        let dir = write_run(
            tmp.path(),
            &MirrorInput {
                summary: None,
                view: Some(&rv),
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
        )
        .expect("write");
        let run = bundle::load_run(&dir).expect("load");
        assert!(run.flow.edges.iter().any(|e| matches!(
            e,
            FlowEdge::Switch { from, on, cases }
            if from == "triage" && on == "$.route" && cases.len() == 2
        )));
        assert_eq!(run.flow.nodes[0].kind, NodeKind::Acp);

        let nsettings = bundle::node_settings(&dir);
        assert_eq!(nsettings["triage"].agent.as_deref(), Some("grok-build"));
        assert_eq!(nsettings["triage"].model.as_deref(), Some("grok-4"));

        let ssettings = bundle::step_settings(&dir);
        assert_eq!(ssettings["triage#1"].agent.as_deref(), Some("grok-build"));
        assert_eq!(ssettings["triage#1"].model.as_deref(), Some("grok-4"));
    }

    #[test]
    fn test_stall_clock_connected_vs_disconnected() {
        let tmp = tempdir().expect("tempdir");
        let rv = sample_running_view();
        let now_ms = parse_ts("2026-11-18T19:10:00.000Z").expect("ts"); // 9 min after updatedAt

        // Disconnected write: old timestamps -> stalled.
        let dir = write_run_at(
            tmp.path(),
            &MirrorInput {
                summary: None,
                view: Some(&rv),
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
            None,
        )
        .expect("write disconnected");
        let run = bundle::load_run(&dir).expect("load");
        assert!(bundle::is_stalled(&run.summary, now_ms));

        // Connected write: updatedAt refreshed to now_ms -> not stalled.
        write_run_at(
            tmp.path(),
            &MirrorInput {
                summary: None,
                view: Some(&rv),
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
            Some(now_ms),
        )
        .expect("write connected");
        let run2 = bundle::load_run(&dir).expect("load 2");
        assert!(!bundle::is_stalled(&run2.summary, now_ms));
    }

    #[test]
    fn test_summary_only_run_and_garbage_values() {
        let tmp = tempdir().expect("tempdir");
        let summary = json!({
            "runId": "run-sum",
            "workflowName": "nightly",
            "createdAt": "2026-11-18T18:00:00.000Z",
            "updatedAt": "2026-11-18T18:05:00.000Z",
            "display": { "status": "completed" }
        });
        write_run(
            tmp.path(),
            &MirrorInput {
                summary: Some(&summary),
                view: None,
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
        )
        .expect("write summary");

        let garbage = json!({
            "runId": "run-garbage",
            "workflowName": 12345,
            "createdAt": false,
            "display": "not-an-object"
        });
        write_run(
            tmp.path(),
            &MirrorInput {
                summary: Some(&garbage),
                view: None,
                activity: &[],
                gate: None,
                text_of: &inline_text_of,
            },
        )
        .expect("write garbage");

        let list = bundle::list_runs(tmp.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].run_id, "run-sum");
        assert_eq!(list[0].status, RunStatus::Completed);
        assert_eq!(list[1].run_id, "run-garbage");
        assert_eq!(list[1].status, RunStatus::Unknown);
    }
}
```