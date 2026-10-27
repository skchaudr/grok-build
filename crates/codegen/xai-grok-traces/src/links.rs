//! The dispatch tree: which session started which.
//!
//! Each harness names sessions its own way, so a session stores its own `native_id` and its
//! dispatcher's `parent_native`; the pair resolves inside one source at query time. Found so far:
//!
//! | Source | Parent comes from |
//! |---|---|
//! | claude | path `<session>/subagents/agent-*.jsonl` -> `<session>.jsonl`; label, type and finished from the sibling `agent-*.meta.json` |
//! | codex | `session_meta` `payload.parent_thread_id`, or `source.subagent.thread_spawn.parent_thread_id` |
//! | omp | path `<cwd>/<parent stem>/<Name>.jsonl` (task subagents), or the header's `parentSession` path |
//! | dsh | header `parentSession` (a session id; `seedLength` marks a fork) |
//! | grok | `summary.json` `parent_session_id` when `session_kind` starts with `subagent` |
//! | bb | `threads.parent_thread_id` |
//! | pi | no parent field in the files |
//! | cursor | none; `native_id` is the transcript file stem |

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use rusqlite::{Transaction, params};
use serde_json::Value;

use crate::normalize::{clip, prompt_context};
use crate::roots::Root;

/// Usage lines are small. A chunk line that happens to mention a marker is not parsed.
const GROK_CONTEXT_LINE_MAX: usize = 64 * 1024;

/// Header records (`session`, `session_meta`) sit in the first few lines.
pub(crate) const HEADER_LINES: i64 = 8;
const LABEL_CHARS: usize = 120;
const META_MAX_BYTES: u64 = 64 * 1024;

/// What a session says about its place in the tree. `None` leaves the stored value alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Links {
    pub native_id: Option<String>,
    pub parent_native: Option<String>,
    pub label: Option<String>,
    pub agent_type: Option<String>,
    /// 1 finished, 2 finished with an error.
    pub done: Option<i64>,
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

fn label(text: &str) -> String {
    clip(text.lines().next().unwrap_or(text).trim(), LABEL_CHARS)
}

/// `size:mtime` of a sidecar file, so a change shows without reparsing the transcript.
pub(crate) fn file_sig(path: &Path) -> Option<String> {
    let m = fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some(format!("{}:{mtime}", m.len()))
}

/// The path of `path` under `root`, without its extension; the `native_id` of file-based sessions.
fn rel_stem(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut name = rel.to_string_lossy().into_owned();
    for ext in [".jsonl.zstd", ".jsonl.zst", ".jsonl"] {
        if let Some(stripped) = name.strip_suffix(ext) {
            name = stripped.to_string();
            break;
        }
    }
    Some(name)
}

/// Links derivable from the path alone, plus the sidecar file to watch.
pub(crate) fn path_links(root: &Root, path: &Path) -> (Links, Option<PathBuf>) {
    let mut l = Links::default();
    match root.source.as_str() {
        "claude" => {
            l.native_id = rel_stem(&root.path, path);
            let rel = path.strip_prefix(&root.path).ok();
            let comps: Vec<String> = rel.map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()).unwrap_or_default();
            if let Some(at) = comps.iter().position(|c| c == "subagents").filter(|i| *i > 0) {
                l.parent_native = Some(comps[..at].join("/"));
                l.agent_type = Some("subagent".into());
                let meta = path.with_extension("meta.json");
                apply_claude_meta(&meta, &mut l);
                return (l, Some(meta));
            }
        }
        // `agent-transcripts/<id>/<id>.jsonl`: the stem is the chat's `conversation_id` that hooks report.
        "cursor" => l.native_id = path.file_stem().map(|n| n.to_string_lossy().into_owned()),
        "pi" | "omp" => {
            l.native_id = rel_stem(&root.path, path);
            // `<cwd dir>/<parent stem>/<Name>.jsonl`: a task subagent next to its parent's transcript.
            let rel = path.strip_prefix(&root.path).ok();
            if let (Some(rel), Some(dir)) = (rel, path.parent()) {
                if rel.components().count() >= 3 {
                    l.parent_native = rel_stem(&root.path, dir);
                    l.label = path.file_stem().map(|n| label(&n.to_string_lossy()));
                    l.agent_type = Some("task".into());
                }
            }
        }
        _ => {}
    }
    (l, None)
}

/// Claude's `agent-*.meta.json`: `{agentType, description, finished, model, ...}`. Tiny; read in full.
fn apply_claude_meta(meta: &Path, l: &mut Links) {
    let Ok(f) = fs::File::open(meta) else { return };
    let mut buf = String::new();
    if std::io::Read::read_to_string(&mut std::io::Read::take(f, META_MAX_BYTES), &mut buf).is_err() {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(&buf) else { return };
    if let Some(d) = s(v.get("description")) {
        l.label = Some(label(&d));
    }
    if let Some(t) = s(v.get("agentType")) {
        l.agent_type = Some(t);
    }
    if v.get("finished") == Some(&Value::Bool(true)) {
        let errored = v.get("error").is_some_and(|e| !e.is_null() && e != &Value::Bool(false))
            || matches!(s(v.get("status")).as_deref(), Some("failed" | "error"));
        l.done = Some(if errored { 2 } else { 1 });
    }
}

/// Links from a header line (`session_meta` for Codex, `session` for DSH, Pi and OMP).
pub(crate) fn header_links(root: &Root, v: &Value, l: &mut Links) {
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    match (root.source.as_str(), ty) {
        ("codex", "session_meta") => {
            let p = v.get("payload").unwrap_or(&Value::Null);
            l.native_id = s(p.get("id")).or(l.native_id.take());
            let spawn = p.pointer("/source/subagent/thread_spawn");
            l.parent_native = s(p.get("parent_thread_id")).or_else(|| s(spawn.and_then(|t| t.get("parent_thread_id"))));
            if let Some(sub) = p.pointer("/source/subagent") {
                l.agent_type = s(spawn.and_then(|t| t.get("agent_role")))
                    .or_else(|| s(sub.get("other")))
                    .or(Some("subagent".into()));
                l.label = s(spawn.and_then(|t| t.get("agent_path")))
                    .map(|path| label(path.rsplit('/').next().unwrap_or(&path)))
                    .or_else(|| s(spawn.and_then(|t| t.get("agent_nickname"))).map(|n| label(&n)))
                    .or_else(|| s(p.get("agent_nickname")).map(|n| label(&n)));
            }
        }
        ("dsh", "session") => {
            l.native_id = s(v.get("id")).or(l.native_id.take());
            l.parent_native = s(v.get("parentSession"));
            if l.parent_native.is_some() {
                l.agent_type = Some(if v.get("seedLength").is_some() { "fork" } else { "subagent" }.into());
            }
        }
        ("pi" | "omp", "session") if l.parent_native.is_none() => {
            if let Some(parent) = s(v.get("parentSession")) {
                l.parent_native = rel_stem(&root.path, Path::new(&parent))
                    .or_else(|| root.path.canonicalize().ok().and_then(|r| rel_stem(&r, Path::new(&parent))));
                if l.parent_native.is_some() {
                    l.agent_type = Some("branch".into());
                }
            }
        }
        _ => {}
    }
}

/// Grok keeps session facts in the sibling `summary.json`.
pub(crate) fn grok_summary(chat_history: &Path) -> Option<Value> {
    let text = fs::read_to_string(chat_history.with_file_name("summary.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// `size:mtime` of `updates.jsonl` and `summary.json`, so context can refresh when the transcript did not.
pub(crate) fn grok_aux_sig(chat_history: &Path) -> Option<String> {
    let part = |name: &str| file_sig(&chat_history.with_file_name(name)).unwrap_or_default();
    let (updates, summary) = (part("updates.jsonl"), part("summary.json"));
    if updates.is_empty() && summary.is_empty() { None } else { Some(format!("{updates}|{summary}")) }
}

/// Latest context pressure for one grok session.
///
/// `chat_history.jsonl` stores `ConversationItem`s. Assistant rows have no usage field, so the prompt
/// size comes from the sibling `updates.jsonl` model-response records (`response_completed.usage` is
/// uncached `input_tokens` plus cache read and cache creation; `response_started` carries the same
/// input fields; `auto_compact_completed.tokens_after` is the size after a compact). The window is
/// `summary.json` `context_window`, or the latest `model_changed.context_window_selection` when the
/// summary has none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GrokContext {
    pub tokens: Option<i64>,
    pub window: Option<i64>,
}

pub(crate) fn grok_context(chat_history: &Path) -> GrokContext {
    let mut ctx = GrokContext::default();
    let mut window_from_summary = false;
    if let Some(summary) = grok_summary(chat_history) {
        if let Some(n) = pos_i64(summary.get("context_window")) {
            ctx.window = Some(n);
            window_from_summary = true;
        }
    }
    let Ok(file) = fs::File::open(chat_history.with_file_name("updates.jsonl")) else { return ctx };
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        if line.len() > GROK_CONTEXT_LINE_MAX || !grok_context_marker(&line) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(update) = grok_update(&v) else { continue };
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("response_completed" | "response_started") => {
                if let Some(n) = prompt_context(update) {
                    ctx.tokens = Some(n);
                }
            }
            Some("auto_compact_completed") => {
                if let Some(n) = pos_i64(update.get("tokens_after")) {
                    ctx.tokens = Some(n);
                }
            }
            Some("auto_compact_started") => {
                if let Some(n) = pos_i64(update.get("tokens_used")) {
                    ctx.tokens = Some(n);
                }
                if !window_from_summary {
                    ctx.window = pos_i64(update.get("context_window")).or(ctx.window);
                }
            }
            Some("model_changed") if !window_from_summary => {
                if let Some(n) = pos_i64(update.get("context_window_selection")) {
                    ctx.window = Some(n);
                }
            }
            _ => {}
        }
    }
    ctx
}

fn grok_context_marker(line: &str) -> bool {
    line.contains("response_completed")
        || line.contains("response_started")
        || line.contains("auto_compact_")
        || line.contains("model_changed")
}

/// The xAI session update inside an `updates.jsonl` envelope, or the object itself.
fn grok_update(v: &Value) -> Option<&serde_json::Map<String, Value>> {
    v.pointer("/params/update")
        .and_then(Value::as_object)
        .or_else(|| v.get("update").and_then(Value::as_object))
        .or_else(|| v.as_object().filter(|m| m.contains_key("sessionUpdate")))
}

fn pos_i64(v: Option<&Value>) -> Option<i64> {
    let n = match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (n > 0).then_some(n)
}

pub(crate) fn grok_links(chat_history: &Path, summary: &Value, l: &mut Links) {
    l.native_id = s(summary.pointer("/info/id")).or_else(|| chat_history.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()));
    let kind = s(summary.get("session_kind")).unwrap_or_default();
    if kind.starts_with("subagent") {
        l.parent_native = s(summary.get("parent_session_id"));
        l.agent_type = s(summary.get("agent_name")).or(Some(kind));
    }
}

/// Stores what is known; fields that are `None` keep their stored value.
pub(crate) fn write_links(tx: &Transaction<'_>, sid: &str, l: &Links) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE trace_sessions SET native_id = COALESCE(?2, native_id), parent_native = COALESCE(?3, parent_native),
         label = COALESCE(?4, label), agent_type = COALESCE(?5, agent_type), done = COALESCE(?6, done) WHERE id = ?1",
        params![sid, l.native_id, l.parent_native, l.label, l.agent_type, l.done],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roots::Format;
    use serde_json::json;

    fn root(source: &str, path: &Path) -> Root {
        Root { id: source.into(), source: source.into(), path: path.to_path_buf(), format: Format::Jsonl, file_name: None }
    }

    #[test]
    fn claude_subagent_path_and_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("-w/sess1/subagents");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("agent-a1.jsonl"), "").unwrap();
        fs::write(sub.join("agent-a1.meta.json"), r#"{"agentType":"Explore","description":"Find the cache\nmore","finished":true}"#).unwrap();
        let r = root("claude", tmp.path());
        let (l, aux) = path_links(&r, &sub.join("agent-a1.jsonl"));
        assert_eq!(l.native_id.as_deref(), Some("-w/sess1/subagents/agent-a1"));
        assert_eq!(l.parent_native.as_deref(), Some("-w/sess1"));
        assert_eq!((l.label.as_deref(), l.agent_type.as_deref(), l.done), (Some("Find the cache"), Some("Explore"), Some(1)));
        assert!(aux.is_some_and(|p| p.ends_with("agent-a1.meta.json")));
        let (top, aux) = path_links(&r, &tmp.path().join("-w/sess1.jsonl"));
        assert_eq!((top.native_id.as_deref(), top.parent_native, aux), (Some("-w/sess1"), None, None));
    }

    #[test]
    fn cursor_native_id_is_the_transcript_stem() {
        let r = root("cursor", Path::new("/c"));
        let (l, aux) = path_links(&r, Path::new("/c/w-proj/agent-transcripts/abc/abc.jsonl"));
        assert_eq!((l.native_id.as_deref(), l.parent_native, aux), (Some("abc"), None, None));
    }

    #[test]
    fn codex_header_spawn_and_fork() {
        let r = root("codex", Path::new("/c"));
        let mut l = Links::default();
        header_links(&r, &json!({"type":"session_meta","payload":{"id":"kid","source":{"subagent":{"thread_spawn":{
            "parent_thread_id":"mom","agent_path":"/root/review_it","agent_nickname":"Linnaeus","agent_role":"explorer"}}}}}), &mut l);
        assert_eq!(l.native_id.as_deref(), Some("kid"));
        assert_eq!(l.parent_native.as_deref(), Some("mom"));
        assert_eq!((l.label.as_deref(), l.agent_type.as_deref()), (Some("review_it"), Some("explorer")));
        let mut top = Links::default();
        header_links(&r, &json!({"type":"session_meta","payload":{"id":"x","source":"cli"}}), &mut top);
        assert_eq!((top.native_id.as_deref(), top.parent_native, top.agent_type), (Some("x"), None, None));
    }

    #[test]
    fn omp_task_dir_and_parent_session_header() {
        let tmp = tempfile::tempdir().unwrap();
        let r = root("omp", tmp.path());
        let (l, _) = path_links(&r, &tmp.path().join("-w/2026_abc/RetryFix.jsonl"));
        assert_eq!((l.parent_native.as_deref(), l.label.as_deref()), (Some("-w/2026_abc"), Some("RetryFix")));
        let mut b = Links::default();
        let parent = tmp.path().join("-w/2026_abc.jsonl");
        header_links(&r, &json!({"type":"session","parentSession": parent.to_string_lossy()}), &mut b);
        assert_eq!(b.parent_native.as_deref(), Some("-w/2026_abc"));
    }

    #[test]
    fn dsh_and_grok() {
        let r = root("dsh", Path::new("/d"));
        let mut l = Links::default();
        header_links(&r, &json!({"type":"session","id":"c","parentSession":"p","seedLength":25}), &mut l);
        assert_eq!((l.native_id.as_deref(), l.parent_native.as_deref(), l.agent_type.as_deref()), (Some("c"), Some("p"), Some("fork")));
        let mut g = Links::default();
        grok_links(
            Path::new("/g/sessions/x/sid/chat_history.jsonl"),
            &json!({"info":{"id":"sid"},"session_kind":"subagent","parent_session_id":"pid","agent_name":"explore"}),
            &mut g,
        );
        assert_eq!((g.native_id.as_deref(), g.parent_native.as_deref(), g.agent_type.as_deref()), (Some("sid"), Some("pid"), Some("explore")));
        let mut f = Links::default();
        grok_links(Path::new("/g/sessions/x/sid/chat_history.jsonl"), &json!({"info":{"id":"sid"},"session_kind":"fork","parent_session_id":"pid"}), &mut f);
        assert_eq!(f.parent_native, None, "a fork is not a dispatch");
    }

    #[test]
    fn grok_context_reads_response_usage_and_keeps_the_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sid");
        fs::create_dir_all(&dir).unwrap();
        let chat = dir.join("chat_history.jsonl");
        fs::write(&chat, "{\"type\":\"assistant\",\"content\":\"x\"}\n").unwrap();
        fs::write(dir.join("summary.json"), r#"{"info":{"id":"sid"},"context_window":200000}"#).unwrap();
        fs::write(dir.join("updates.jsonl"), concat!(
            r#"{"timestamp":1,"method":"_x.ai/session/update","params":{"sessionId":"sid","update":{"sessionUpdate":"response_started","input_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#, "\n",
            r#"{"timestamp":2,"method":"_x.ai/session/update","params":{"sessionId":"sid","update":{"sessionUpdate":"response_completed","usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200,"reasoning_tokens":0}}}}"#, "\n",
            r#"{"timestamp":3,"method":"_x.ai/session/update","params":{"sessionId":"sid","update":{"sessionUpdate":"model_changed","model_id":"grok-5","context_window_selection":128000}}}"#, "\n",
            r#"{"timestamp":4,"method":"_x.ai/session/update","params":{"sessionId":"sid","update":{"sessionUpdate":"auto_compact_completed","tokens_after":400,"summary_preview":"shrunk"}}}"#, "\n",
        )).unwrap();
        let ctx = grok_context(&chat);
        assert_eq!((ctx.tokens, ctx.window), (Some(400), Some(200_000)), "summary window wins; compact replaces the prompt size");
        fs::write(dir.join("summary.json"), r#"{"info":{"id":"sid"}}"#).unwrap();
        let ctx = grok_context(&chat);
        assert_eq!(ctx.window, Some(128_000), "model_changed fills a missing summary window");
        assert!(grok_aux_sig(&chat).is_some());
    }
}
