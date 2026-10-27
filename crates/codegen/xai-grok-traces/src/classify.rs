//! Signal versus noise, tool-call details and structured errors for one event.
//!
//! Everything keys off structure (record types, flags, exit-code headers the harness writes),
//! never off words found in output text.

use std::borrow::Cow;

use serde_json::{Map, Value};

/// Stored cap for `detail`.
pub const DETAIL_CHARS: usize = 100;

/// Tags a harness injects into user turns or tool output; none of it was typed by the user.
const INJECTED_TAGS: &[&str] = &[
    "system-reminder",
    "local-command-caveat",
    "local-command-stdout",
    "local-command-stderr",
    "command-name",
    "command-message",
    "command-args",
    "user_info",
    "environment_context",
    "task-notification",
    "ide_opened_file",
    "ide_selection",
    "permissions",
    "instructions",
    "collaboration_mode",
    "multi_agent_mode",
    "turn_aborted",
    "skill",
    "system_instruction",
    "rules",
    "recommended_plugins",
];

/// Text a harness writes as if the user had: continuation prompts and session notes. Not typed by anyone.
const HARNESS_PROMPTS: &[&str] = &[
    "### Session update",
    "The beginning of the above subagent result",
    "Perform any necessary follow-up actions",
    "Briefly inform the user about the task result",
    "The following is the Codex agent history",
    "Your current interruptible wait",
];

/// Whether a user message is one of the harness's own prompts.
pub fn harness_prompt(text: &str) -> bool {
    let t = text.trim_start();
    HARNESS_PROMPTS.iter().any(|p| t.starts_with(p))
}

fn is_injected_tag(tag: &str) -> bool {
    let t = tag.to_ascii_lowercase();
    INJECTED_TAGS.contains(&t.as_str()) || t.ends_with("-hook") || t.ends_with("_hook")
}

/// Removes injected wrapper blocks that start a line (so prose that merely mentions a tag is
/// kept). Returns the remaining text and the first tag removed.
pub fn strip_injected(s: &str) -> (Cow<'_, str>, Option<String>) {
    if s.trim_start().starts_with("# AGENTS.md instructions") {
        return (Cow::Borrowed(""), Some("agents-md".into()));
    }
    if !s.contains('<') {
        return (Cow::Borrowed(s), None);
    }
    let mut out = String::new();
    let mut first: Option<String> = None;
    let (mut i, mut copied) = (0, 0);
    while let Some(off) = s[i..].find('<') {
        let at = i + off;
        i = at + 1;
        let line_start = s[..at].rsplit('\n').next().is_some_and(|l| l.trim().is_empty());
        if !line_start {
            continue;
        }
        let rest = &s[at + 1..];
        let end = rest.find(|c: char| c == '>' || c.is_whitespace()).unwrap_or(rest.len());
        let tag = &rest[..end];
        if tag.is_empty() || tag.len() > 40 || tag.starts_with('/') || !is_injected_tag(tag) {
            continue;
        }
        out.push_str(&s[copied..at]);
        let close = format!("</{tag}>");
        copied = s[at..].find(&close).map_or(s.len(), |c| at + c + close.len());
        i = copied.max(i);
        first.get_or_insert_with(|| tag.to_string());
    }
    if first.is_none() {
        return (Cow::Borrowed(s), None);
    }
    out.push_str(&s[copied..]);
    (Cow::Owned(out.trim().to_string()), first)
}

/// Record types that are bookkeeping or telemetry in some harness (compared lowercase).
const NOISE_TYPES: &[&str] = &[
    // Claude
    "attachment", "file-history-snapshot", "file-history-delta", "last-prompt", "mode", "permission-mode", "summary",
    "ai-title", "queue-operation", "atis-latch", "cost-state", "system", "progress", "custom-title", "agent-name",
    // Codex
    "turn_context", "world_state", "token_usage_record", "session_meta", "compacted", "inter_agent_communication_metadata",
    "event_msg/token_count", "event_msg/thread_settings_applied", "event_msg/item_completed", "event_msg/task_started",
    "event_msg/user_message", "event_msg/agent_message",
    // Pi / OMP
    "session", "model_change", "thinking_level_change", "service_tier_change", "session_init", "title", "title_change",
    "custom", "custom_message", "credential_pin", "branch_summary", "compaction", "label", "session_exit",
    // DSH
    "turn/start", "session/title", "session/title-llm-request", "session/end-seed", "assistant/chunk",
    "permission/preset", "sandbox/mode", "approval/policy", "request/header", "request/context",
    // Grok / bb
    "thread/tokenusage/updated", "thread/contextwindowusage/updated", "turn/started", "provider/unhandled",
];

/// Prefixes of whole families of bookkeeping records.
const NOISE_PREFIXES: &[&str] = &["api-request", "telemetry", "step/", "agent/inbox", "sandbox/", "approval/"];

/// Whether a record type (`top/nested`, any case) is bookkeeping.
pub fn noise_type(event_type: &str) -> bool {
    let t = event_type.to_ascii_lowercase();
    NOISE_TYPES.contains(&t.as_str()) || NOISE_PREFIXES.iter().any(|p| t.starts_with(p))
}

const ERROR_KEYS: [&str; 3] = ["is_error", "isError", "isApiErrorMessage"];

fn flag(m: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| m.get(*k) == Some(&Value::Bool(true)))
}

fn nonempty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(_) => true,
    }
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

/// An error the harness recorded in a field: an error flag, a non-empty `error`, an error
/// result block, an `error` stop reason or turn status. A user abort is not a failure.
pub fn structured_error(top: &Map<String, Value>, payload: &Map<String, Value>, event_type: &str) -> bool {
    // A tool whose outcome was never recorded (imported transcripts) is unknown, not failed.
    if payload.get("error").and_then(|e| e.get("code")).and_then(Value::as_str) == Some("TOOL_OUTCOME_UNKNOWN") {
        return false;
    }
    let aborted = str_of(top, "status") == Some("aborted") || str_of(payload, "status") == Some("aborted");
    if (nonempty(top.get("error")) || nonempty(payload.get("error"))) && !aborted {
        return true;
    }
    if flag(top, &ERROR_KEYS) || flag(payload, &ERROR_KEYS) {
        return true;
    }
    if str_of(top, "status") == Some("error") || str_of(payload, "stopReason") == Some("error") {
        return true;
    }
    // DSH nests the result message one level down.
    let nested = payload.get("message").and_then(Value::as_object);
    if nested.is_some_and(|m| flag(m, &ERROR_KEYS)) {
        return true;
    }
    let blocks = |m: &Map<String, Value>| {
        m.get("content").and_then(Value::as_array).is_some_and(|bs| {
            bs.iter().filter_map(Value::as_object).any(|b| {
                matches!(str_of(b, "type"), Some("tool_result" | "toolResult" | "tool-result")) && flag(b, &ERROR_KEYS)
            })
        })
    };
    if blocks(payload) || nested.is_some_and(blocks) {
        return true;
    }
    // Record types that are themselves error records: `provider/error`, `api-error`, `turn_failed`.
    let last = event_type.rsplit('/').next().unwrap_or(event_type).to_ascii_lowercase();
    last.ends_with("error") || last.ends_with("failed") || last.ends_with("failure")
}

/// Codex writes an exit status header at the top of command output. `(failed, detail)`.
pub fn exit_status(text: &str) -> Option<(bool, String)> {
    let head: String = text.chars().take(600).collect();
    if let Some(i) = head.find("Process exited with code ") {
        let code: String = head[i + 25..].chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        if let Ok(n) = code.parse::<i64>() {
            return Some((n != 0, format!("exit {n}")));
        }
    }
    let first = head.lines().next().unwrap_or("");
    if let Some(n) = first.strip_prefix("Exit code: ").and_then(|r| r.trim().parse::<i64>().ok()) {
        return Some((n != 0, format!("exit {n}")));
    }
    if first.starts_with("Script failed") || first.starts_with("apply_patch verification failed") {
        return Some((true, clip_line(first)));
    }
    None
}

fn clip_line(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(DETAIL_CHARS) {
        None => flat,
        Some((i, _)) => format!("{}…", &flat[..i]),
    }
}

/// First non-empty line of `s`, clipped.
pub fn first_line(s: &str) -> Option<String> {
    s.lines().map(str::trim).find(|l| !l.is_empty()).map(clip_line)
}

pub fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.trim_end_matches('/').rsplit('/').filter(|s| !s.is_empty()).take(2).collect();
    match parts.as_slice() {
        [base] => (*base).to_string(),
        [base, parent] => format!("{parent}/{base}"),
        _ => p.to_string(),
    }
}

/// `mcp__server__tool` → `server.tool`.
fn mcp_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    Some(format!("{server}.{tool}"))
}

fn patch_path(s: &str) -> Option<String> {
    s.lines().find_map(|l| {
        ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|p| l.strip_prefix(p)).map(short_path)
    })
}

/// A compact summary of a tool call's arguments.
pub fn call_detail(name: &str, args: &Value) -> Option<String> {
    if let Some(mcp) = mcp_name(name) {
        return Some(clip_line(&mcp));
    }
    match args {
        Value::String(s) => patch_path(s).or_else(|| first_line(s)),
        Value::Object(m) => {
            let get = |keys: &[&str]| keys.iter().find_map(|k| str_of(m, k).filter(|s| !s.trim().is_empty()));
            if let Some(c) = get(&["command", "cmd", "script"]) {
                return first_line(c);
            }
            if let Some(Value::Array(a)) = m.get("command").or_else(|| m.get("cmd")) {
                let joined = a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
                return first_line(&joined);
            }
            if let Some(p) = get(&["pattern", "glob_pattern", "regex"]) {
                return first_line(p);
            }
            if let Some(q) = get(&["query", "description"]) {
                return first_line(q);
            }
            if let Some(p) = get(&["file_path", "path", "target_file", "filePath", "notebook_path", "target_directory", "directory"]) {
                return Some(short_path(p));
            }
            if let Some(p) = get(&["input", "patch"]).and_then(patch_path) {
                return Some(p);
            }
            get(&["url", "prompt", "text", "input"]).and_then(first_line)
        }
        _ => None,
    }
}

fn args_value(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok().filter(Value::is_object).unwrap_or_else(|| Value::String(s.clone())),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

fn name_of(b: &Map<String, Value>) -> Option<String> {
    str_of(b, "name")
        .or_else(|| str_of(b, "toolName"))
        .or_else(|| b.get("function").and_then(Value::as_object).and_then(|f| str_of(f, "name")))
        .map(str::to_string)
}

fn args_of(b: &Map<String, Value>) -> Value {
    let direct = ["input", "arguments", "args"].iter().find_map(|k| b.get(*k));
    args_value(direct.or_else(|| b.get("function").and_then(|f| f.get("arguments"))))
}

/// The (first) tool call carried by a record: a `tool_use`/`toolCall` content block, an
/// OpenAI-style `tool_calls` entry, or the record itself when it is a flat call (Codex, DSH).
pub fn tool_call(top: &Map<String, Value>, payload: &Map<String, Value>, flat: bool) -> Option<(String, Value)> {
    const CALL_BLOCKS: [&str; 4] = ["tool_use", "toolCall", "tool_call", "function_call"];
    if let Some(blocks) = payload.get("content").and_then(Value::as_array) {
        for b in blocks.iter().filter_map(Value::as_object) {
            if let (true, Some(name)) = (str_of(b, "type").is_some_and(|t| CALL_BLOCKS.contains(&t)), name_of(b)) {
                return Some((name, args_of(b)));
            }
        }
    }
    for holder in [payload, top] {
        let first = holder.get("tool_calls").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_object);
        if let Some((c, name)) = first.and_then(|c| name_of(c).map(|n| (c, n))) {
            return Some((name, args_of(c)));
        }
    }
    if flat {
        for holder in [payload, top] {
            if let Some(name) = name_of(holder) {
                return Some((name, args_of(holder)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_leading_and_trailing_wrappers_but_keeps_prose() {
        let (t, tag) = strip_injected("<system-reminder>\nctx\n</system-reminder>\nfix the bug");
        assert_eq!((t.as_ref(), tag.as_deref()), ("fix the bug", Some("system-reminder")));
        let (t, _) = strip_injected("real\n<system-reminder>x</system-reminder>");
        assert_eq!(t, "real");
        let (t, tag) = strip_injected("<local-command-caveat>Caveat</local-command-caveat>");
        assert_eq!((t.as_ref(), tag.as_deref()), ("", Some("local-command-caveat")));
        let (t, tag) = strip_injected("use the <system-reminder> tag sparingly");
        assert_eq!((t.as_ref(), tag), ("use the <system-reminder> tag sparingly", None));
        let (t, tag) = strip_injected("<div>html</div>");
        assert_eq!((t.as_ref(), tag), ("<div>html</div>", None));
        let (t, tag) = strip_injected("# AGENTS.md instructions for /w\n\n<INSTRUCTIONS>x</INSTRUCTIONS>");
        assert_eq!((t.as_ref(), tag.as_deref()), ("", Some("agents-md")));
        let (t, _) = strip_injected("<user-prompt-submit-hook>out</user-prompt-submit-hook>\nhi");
        assert_eq!(t, "hi");
    }

    #[test]
    fn exit_headers_are_structured_not_words() {
        assert_eq!(exit_status("Chunk ID: a\nWall time: 0.3 seconds\nProcess exited with code 0\nerror: none"), Some((false, "exit 0".into())));
        assert_eq!(exit_status("Chunk ID: a\nProcess exited with code 127\n"), Some((true, "exit 127".into())));
        assert_eq!(exit_status("Exit code: 2\nWall time: 1s"), Some((true, "exit 2".into())));
        assert_eq!(exit_status("Script failed\nWall time 1s"), Some((true, "Script failed".into())));
        assert_eq!(exit_status("Script completed\nOutput: error error"), None);
        assert_eq!(exit_status("plain output mentioning error"), None);
    }

    #[test]
    fn call_details() {
        let d = |n: &str, a: Value| call_detail(n, &a);
        assert_eq!(d("Bash", json!({"command": "git status\nand more"})).as_deref(), Some("git status"));
        assert_eq!(d("Edit", json!({"file_path": "/a/b/c/lib.rs", "old_string": "x"})).as_deref(), Some("c/lib.rs"));
        assert_eq!(d("Grep", json!({"pattern": "fn main", "path": "/x"})).as_deref(), Some("fn main"));
        assert_eq!(d("Agent", json!({"description": "Traces: evict", "prompt": "long"})).as_deref(), Some("Traces: evict"));
        assert_eq!(d("mcp__github__create_issue", json!({"title": "t"})).as_deref(), Some("github.create_issue"));
        assert_eq!(d("exec_command", json!({"cmd": "ls -la", "workdir": "/w"})).as_deref(), Some("ls -la"));
        assert_eq!(d("apply_patch", Value::String("*** Begin Patch\n*** Update File: src/a/x.rs\n@@".into())).as_deref(), Some("a/x.rs"));
        assert_eq!(d("TodoWrite", json!({"todos": []})), None);
        let long = "x".repeat(300);
        assert_eq!(d("Bash", json!({ "command": long })).unwrap().chars().count(), DETAIL_CHARS + 1);
    }

    #[test]
    fn errors_are_flags_not_words() {
        let top = json!({"type": "user"});
        let m = |v: &Value| v.as_object().unwrap().clone();
        let ok = m(&json!({"content": [{"type": "tool_result", "content": "error: failed with exception denied"}]}));
        assert!(!structured_error(top.as_object().unwrap(), &ok, "user"));
        let bad = m(&json!({"content": [{"type": "tool_result", "is_error": true, "content": "x"}]}));
        assert!(structured_error(top.as_object().unwrap(), &bad, "user"));
        let input_flag = m(&json!({"content": [{"type": "tool_use", "name": "Edit", "input": {"is_error": true}}]}));
        assert!(!structured_error(top.as_object().unwrap(), &input_flag, "assistant"));
        assert!(structured_error(&m(&json!({"error": "boom"})), &Map::new(), "assistant"));
        assert!(!structured_error(&m(&json!({"error": "User aborted", "status": "aborted"})), &Map::new(), "turn_ended"));
        assert!(structured_error(&Map::new(), &Map::new(), "provider/error"));
        assert!(!structured_error(&Map::new(), &Map::new(), "error_recovery"));
        assert!(structured_error(&Map::new(), &m(&json!({"stopReason": "error"})), "message"));
    }
}
