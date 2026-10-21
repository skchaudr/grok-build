//! What a harness hook hands us on stdin, and what to do with it.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::bb::thread_key;
use crate::classify::call_detail;
use crate::live::{chain_from, find_harness_chain, proc_start};
use crate::index::{Result, TraceIndex, now_ms, session_id};
use crate::receipts::Receipt;
use crate::roots::{Format, Root, resolve};

/// Stdin beyond this is discarded.
pub const MAX_STDIN_BYTES: usize = 16 * 1024 * 1024;
const FIELD_CHARS: usize = 512;

const EVENT: [&str; 4] = ["hook_event_name", "hookEventName", "event", "type"];
const SESSION: [&str; 4] = ["session_id", "sessionId", "conversation_id", "thread_id"];
/// Cursor's `session_id` is not always its chat id; `conversation_id` is.
const CURSOR_SESSION: [&str; 4] = ["conversation_id", "session_id", "sessionId", "thread_id"];
const PATH: [&str; 4] = ["transcript_path", "transcriptPath", "session_file", "sessionFile"];
const TOOL: [&str; 3] = ["tool_name", "toolName", "tool"];
const TOOL_INPUT: [&str; 3] = ["tool_input", "toolInput", "input"];
const TOOL_USE_ID: [&str; 3] = ["tool_use_id", "toolUseId", "toolCallId"];
const AGENT_ID: [&str; 2] = ["agent_id", "agentId"];
const NOTIFICATION_TYPE: [&str; 2] = ["notification_type", "notificationType"];
const HARNESS_PID: [&str; 1] = ["harness_pid"];

fn first_str(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| v.get(*k)?.as_str())
        .find(|s| !s.trim().is_empty())
        .map(|s| s.chars().take(FIELD_CHARS).collect())
}

/// Tolerant field pull across harnesses; unparseable input yields event `unparsed`.
pub fn extract(source: &str, input: &[u8]) -> Receipt {
    let mut r = Receipt { at_ms: now_ms(), source: source.chars().take(32).collect(), ..Default::default() };
    let Ok(v) = serde_json::from_slice::<Value>(input) else {
        r.event = "unparsed".into();
        return r;
    };
    r.event = first_str(&v, &EVENT).unwrap_or_else(|| "unknown".into());
    r.native_session = first_str(&v, if source == "cursor" { &CURSOR_SESSION } else { &SESSION });
    r.path = first_str(&v, &PATH);
    r.tool = first_str(&v, &TOOL);
    r.cwd = first_str(&v, &["cwd"]).or_else(|| {
        let first = v.get("workspace_roots")?.as_array()?.first()?.as_str()?;
        Some(first.chars().take(FIELD_CHARS).collect())
    });
    r.tool_use_id = first_str(&v, &TOOL_USE_ID);
    r.agent_id = first_str(&v, &AGENT_ID);
    r.harness_pid = HARNESS_PID.iter().find_map(|k| v.get(*k)?.as_i64()).filter(|p| *p > 0);
    if source == "cursor" {
        cursor_tool(&v, &mut r);
    }
    if r.detail.is_none() {
        r.detail = detail(source, &v, &r);
    }
    r
}

/// Cursor's `before*`/`after*` events carry no `tool_name`: the event says which tool it is.
fn cursor_tool(v: &Value, r: &mut Receipt) {
    let field = |keys: &[&str]| first_str(v, keys);
    match r.event.as_str() {
        "beforeShellExecution" | "afterShellExecution" => {
            r.tool = Some("Shell".into());
            r.detail = field(&["command"]).and_then(|c| call_detail("Shell", &serde_json::json!({ "command": c })));
        }
        "beforeMCPExecution" | "afterMCPExecution" => {
            r.tool = Some("MCP".into());
            let tool = field(&["tool_name", "toolName", "tool"]);
            let server = field(&["server", "server_name", "serverName"]);
            r.detail = match (server, tool) {
                (Some(s), Some(t)) => Some(format!("{s}.{t}")),
                (None, Some(t)) => Some(t),
                _ => None,
            };
        }
        "beforeReadFile" => {
            r.tool = Some("Read".into());
            r.detail = field(&["file_path", "filePath", "path"]).and_then(|p| call_detail("Read", &serde_json::json!({ "file_path": p })));
        }
        _ => {}
    }
}

/// `detail` for events not handled above: a call's arguments, a notification type, a SessionStart source.
fn detail(source: &str, v: &Value, r: &Receipt) -> Option<String> {
    match (source, r.event.as_str()) {
        ("claude", "Notification") => first_str(v, &NOTIFICATION_TYPE),
        ("claude", "SessionStart") => first_str(v, &["source"]),
        _ => {
            let input = TOOL_INPUT.iter().find_map(|k| v.get(*k))?;
            call_detail(r.tool.as_deref()?, input)
        }
    }
}

/// Grok hands over its `updates.jsonl`; the indexed transcript is the sibling `chat_history.jsonl`.
pub fn transcript_for(source: &str, path: &Path) -> PathBuf {
    if source == "grok" && path.file_name().is_some_and(|n| n == "updates.jsonl") {
        return path.with_file_name("chat_history.jsonl");
    }
    path.to_path_buf()
}

/// Records the receipt and, when its path is a trace source, indexes just that file.
pub fn ingest(idx: &mut TraceIndex, roots: &[Root], mut r: Receipt) -> Result<i64> {
    if r.source == "bb" {
        return ingest_bb(idx, roots, r);
    }
    fill_harness(&mut r);
    let target = r.path.as_deref().and_then(|p| resolve(&transcript_for(&r.source, Path::new(p)), roots));
    if let Some((root, path)) = &target {
        r.session_id = Some(session_id(&root.source, &path.to_string_lossy()));
    } else if let Some(native) = &r.native_session {
        // No usable path (Cursor sends none early): the harness's own id, within the source. Else NULL, never dropped.
        r.session_id = idx.session_by_native(&r.source, native)?;
    }
    let id = idx.record_receipt(&r)?;
    if let Some((root, path)) = target {
        idx.index_file(root, &path)?;
    }
    Ok(id)
}

/// A payload `harness_pid` gets its start time; without one, the ancestry walk finds the harness.
fn fill_harness(r: &mut Receipt) {
    match r.harness_pid {
        Some(pid) => {
            r.harness_start = proc_start(pid);
            r.harness_chain = chain_from(pid);
        }
        None => {
            if let Some((pid, start, chain)) = find_harness_chain() {
                (r.harness_pid, r.harness_start, r.harness_chain) = (Some(pid), Some(start), chain);
            }
        }
    }
}

/// bb hooks carry a thread id, not a file: record the receipt and index just that thread.
fn ingest_bb(idx: &mut TraceIndex, roots: &[Root], mut r: Receipt) -> Result<i64> {
    let root = roots.iter().find(|x| x.source == "bb" && x.format == Format::Sqlite);
    let thread = r.native_session.clone();
    if let (Some(root), Some(thread)) = (root, &thread) {
        r.session_id = Some(session_id(&root.source, &thread_key(&root.path, thread)));
    }
    let id = idx.record_receipt(&r)?;
    if let (Some(root), Some(thread)) = (root, &thread) {
        idx.index_bb_thread_id(root, thread)?;
    }
    Ok(id)
}
