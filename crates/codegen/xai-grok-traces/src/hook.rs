//! What a harness hook hands us on stdin, and what to do with it.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::bb::thread_key;
use crate::index::{Result, TraceIndex, now_ms, session_id};
use crate::receipts::Receipt;
use crate::roots::{Format, Root, resolve};

/// Stdin beyond this is discarded.
pub const MAX_STDIN_BYTES: usize = 16 * 1024 * 1024;
const FIELD_CHARS: usize = 512;

const EVENT: [&str; 4] = ["hook_event_name", "hookEventName", "event", "type"];
const SESSION: [&str; 4] = ["session_id", "sessionId", "conversation_id", "thread_id"];
const PATH: [&str; 4] = ["transcript_path", "transcriptPath", "session_file", "sessionFile"];
const TOOL: [&str; 3] = ["tool_name", "toolName", "tool"];

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
    r.native_session = first_str(&v, &SESSION);
    r.path = first_str(&v, &PATH);
    r.tool = first_str(&v, &TOOL);
    r.cwd = first_str(&v, &["cwd"]).or_else(|| {
        let first = v.get("workspace_roots")?.as_array()?.first()?.as_str()?;
        Some(first.chars().take(FIELD_CHARS).collect())
    });
    r
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
    let target = r.path.as_deref().and_then(|p| resolve(&transcript_for(&r.source, Path::new(p)), roots));
    if let Some((root, path)) = &target {
        r.session_id = Some(session_id(&root.source, &path.to_string_lossy()));
    }
    let id = idx.record_receipt(&r)?;
    if let Some((root, path)) = target {
        idx.index_file(root, &path)?;
    }
    Ok(id)
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
