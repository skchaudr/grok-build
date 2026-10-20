//! One JSON line from any harness → one [`NormalizedEvent`].
//!
//! Ported from bb-plugin-traces `normalizeRecord`. Heuristics key off type
//! and role names; the only envelope handling is `payload` (Codex), `data`
//! (DSH) and `message` (Claude). Lengths count chars, not UTF-16 units.

use serde_json::{Map, Value};

pub const SUMMARY_CHARS: usize = 1024;
const TYPE_CHARS: usize = 120;
const TITLE_CHARS: usize = 160;
const SEARCH_DEPTH: usize = 6;
const TEXT_DEPTH: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Message,
    Tool,
    Step,
    Turn,
    Reasoning,
    Telemetry,
    System,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Message => "message",
            Kind::Tool => "tool",
            Kind::Step => "step",
            Kind::Turn => "turn",
            Kind::Reasoning => "reasoning",
            Kind::Telemetry => "telemetry",
            Kind::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    ToolResult,
    Reasoning,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::ToolResult => "tool_result",
            Role::Reasoning => "reasoning",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedEvent {
    pub event_type: String,
    pub kind: Kind,
    pub role: Option<Role>,
    pub title: String,
    pub summary: String,
    pub timestamp_ms: Option<i64>,
    pub duration_ms: Option<f64>,
    pub input_tokens: Option<f64>,
    pub output_tokens: Option<f64>,
    pub usage_is_total: bool,
    pub turn: Option<f64>,
    pub step: Option<f64>,
    pub depth: u8,
    pub model: Option<String>,
    pub cwd: Option<String>,
    /// An explicit error marker: a non-empty `error` field or `is_error: true`.
    pub error_flag: bool,
}

impl NormalizedEvent {
    pub fn is_error(&self) -> bool {
        if self.error_flag {
            return true;
        }
        let t = self.event_type.to_lowercase();
        if t.contains("error") || t.contains("fail") {
            return true;
        }
        self.role == Some(Role::ToolResult) && {
            let s = self.summary.to_lowercase();
            ["error", "fail", "denied", "exception", "timeout"]
                .iter()
                .any(|w| s.contains(w))
        }
    }
}

static EMPTY: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);

fn record(v: Option<&Value>) -> &Map<String, Value> {
    v.and_then(Value::as_object).unwrap_or(&EMPTY)
}

fn string_value(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

fn number_value(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) if !s.trim().is_empty() => {
            s.trim().parse::<f64>().ok().filter(|f| f.is_finite())
        }
        _ => None,
    }
}

pub fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}\n…", &s[..i]),
    }
}

fn humanize(s: &str) -> String {
    let spaced: String = s
        .chars()
        .map(|c| if matches!(c, '/' | ':' | '_' | '-') { ' ' } else { c })
        .collect();
    let mut out = String::with_capacity(spaced.len());
    let mut prev_word = false;
    for c in spaced.chars() {
        let word = c.is_ascii_alphanumeric() || c == '_';
        out.push(if word && !prev_word { c.to_ascii_uppercase() } else { c });
        prev_word = word;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_text(s: &str) -> String {
    let long = s.chars().count() > SUMMARY_CHARS;
    let head: String = s.chars().take(SUMMARY_CHARS).collect();
    let mut t = head.replace("\r\n", "\n");
    while let Some((a, b)) = find_trailing_ws(&t) {
        t.replace_range(a..b, "");
    }
    let t = t.trim().to_string();
    if long { format!("{t}\n…") } else { t }
}

/// First `[ \t]+\n` run, as the byte range of the whitespace.
fn find_trailing_ws(s: &str) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b' ' || b[i] == b'\t' {
            let start = i;
            while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                i += 1;
            }
            if i < b.len() && b[i] == b'\n' {
                return Some((start, i));
            }
        } else {
            i += 1;
        }
    }
    None
}

fn find_string<'a>(v: &'a Value, keys: &[&str], depth: usize) -> Option<&'a str> {
    if depth > SEARCH_DEPTH {
        return None;
    }
    match v {
        Value::Array(items) => items.iter().find_map(|i| find_string(i, keys, depth + 1)),
        Value::Object(map) => keys
            .iter()
            .find_map(|k| string_value(map.get(*k)))
            .or_else(|| map.values().find_map(|i| find_string(i, keys, depth + 1))),
        _ => None,
    }
}

fn find_number(v: &Value, keys: &[&str], depth: usize) -> Option<f64> {
    if depth > SEARCH_DEPTH {
        return None;
    }
    match v {
        Value::Array(items) => items.iter().find_map(|i| find_number(i, keys, depth + 1)),
        Value::Object(map) => keys
            .iter()
            .find_map(|k| number_value(map.get(*k)))
            .or_else(|| map.values().find_map(|i| find_number(i, keys, depth + 1))),
        _ => None,
    }
}

const TEXT_KEYS: [&str; 12] = [
    "text", "message", "content", "summary", "arguments", "output", "result", "error", "input",
    "command", "description", "name",
];

fn text_from(v: &Value, depth: usize) -> String {
    if depth > TEXT_DEPTH {
        return String::new();
    }
    match v {
        Value::String(s) => normalize_text(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                let part = text_from(item, depth + 1);
                if part.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&part);
                if out.chars().count() >= SUMMARY_CHARS {
                    return clip(&out, SUMMARY_CHARS);
                }
            }
            out
        }
        Value::Object(map) => TEXT_KEYS
            .iter()
            .filter_map(|k| map.get(*k))
            .map(|v| text_from(v, depth + 1))
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
        Value::Null => String::new(),
    }
}

fn timestamp_ms(v: Option<&Value>) -> Option<i64> {
    if let Some(n) = number_value(v) {
        return Some(if n < 100_000_000_000.0 { (n * 1000.0).round() } else { n.round() } as i64);
    }
    let s = v?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp_millis())
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|d| d.and_utc().timestamp_millis())
        })
}

fn usage_object(v: &Map<String, Value>) -> Option<&Map<String, Value>> {
    v.get("usage")
        .and_then(Value::as_object)
        .or_else(|| record(v.get("info")).get("total_token_usage").and_then(Value::as_object))
        .or_else(|| v.get("total_token_usage").and_then(Value::as_object))
}

fn find_true(v: &Value, keys: &[&str], depth: usize) -> bool {
    if depth > SEARCH_DEPTH {
        return false;
    }
    match v {
        Value::Array(items) => items.iter().any(|i| find_true(i, keys, depth + 1)),
        Value::Object(map) => {
            keys.iter().any(|k| map.get(*k) == Some(&Value::Bool(true)))
                || map.values().any(|i| find_true(i, keys, depth + 1))
        }
        _ => false,
    }
}

fn has_error_field(map: &Map<String, Value>) -> bool {
    match map.get("error") {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(_) => true,
    }
}

fn first_number(obj: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| number_value(obj.get(*k)))
}

fn detect_role(event_type: &str, top: &Map<String, Value>, payload: &Map<String, Value>) -> Option<Role> {
    let explicit = string_value(top.get("role"))
        .or_else(|| string_value(payload.get("role")))
        .or_else(|| string_value(record(payload.get("message")).get("role")))
        .or_else(|| string_value(record(payload.get("data")).get("role")));
    match explicit {
        Some("user" | "human") => return Some(Role::User),
        Some("assistant" | "agent") => return Some(Role::Assistant),
        Some("tool") => {
            return Some(if event_type.contains("result") { Role::ToolResult } else { Role::Tool });
        }
        Some("reasoning" | "thinking") => return Some(Role::Reasoning),
        Some("toolResult" | "tool_result") => return Some(Role::ToolResult),
        _ => {}
    }
    let lower = event_type.to_lowercase();
    let has = |n: &str| lower.contains(n);
    if has("tool") || has("function_call") || has("function-call") {
        Some(if has("result") || has("output") { Role::ToolResult } else { Role::Tool })
    } else if has("user") || lower == "human" {
        Some(Role::User)
    } else if has("assistant") || has("agent_message") {
        Some(Role::Assistant)
    } else if has("reasoning") || has("thinking") {
        Some(Role::Reasoning)
    } else {
        None
    }
}

/// Claude and Pi put tool calls and results in `content[]` blocks of an
/// ordinary user/assistant message.
fn content_tool_role(payload: &Map<String, Value>) -> Option<Role> {
    let blocks = payload.get("content")?.as_array()?;
    blocks.iter().find_map(|b| match b.get("type").and_then(Value::as_str)? {
        "tool_use" | "toolCall" | "tool_call" | "function_call" => Some(Role::Tool),
        "tool_result" | "toolResult" | "function_call_output" => Some(Role::ToolResult),
        _ => None,
    })
}

fn detect_kind(event_type: &str, role: Option<Role>) -> Kind {
    let lower = event_type.to_lowercase();
    let has = |n: &str| lower.contains(n);
    if has("token") || has("usage") || has("rate_limit") {
        Kind::Telemetry
    } else if has("tool") || has("function_call") || matches!(role, Some(Role::Tool | Role::ToolResult)) {
        Kind::Tool
    } else if has("step") {
        Kind::Step
    } else if has("turn") {
        Kind::Turn
    } else if has("reasoning") || has("thinking") || role == Some(Role::Reasoning) {
        Kind::Reasoning
    } else if role.is_some() || has("message") || has("response_item") {
        Kind::Message
    } else {
        Kind::System
    }
}

pub fn normalize(value: &Value) -> NormalizedEvent {
    let top = record(Some(value));
    let payload_v = top.get("payload").or_else(|| top.get("data")).or_else(|| top.get("message"));
    let payload = record(payload_v.filter(|v| !v.is_null()));
    let payload_value = Value::Object(payload.clone());

    let nested = string_value(payload.get("type"));
    let top_type = string_value(top.get("type"));
    let event_type = match (top_type, nested) {
        (Some(t), Some(n)) => format!("{t}/{n}"),
        (Some(t), None) => t.to_string(),
        (None, Some(n)) => n.to_string(),
        (None, None) => "record".to_string(),
    };

    let role = content_tool_role(payload).or_else(|| detect_role(&event_type, top, payload));
    let kind = detect_kind(&event_type, role);
    let lower = event_type.to_lowercase();
    let depth = if role == Some(Role::ToolResult)
        || lower.contains("tool/result") || lower.contains("tool_result") || lower.contains("function_call_output") {
        2
    } else if kind == Kind::Tool || lower.contains("step") || lower.contains("reasoning") || lower.contains("text-chunk") {
        1
    } else {
        0
    };

    const IN: [&str; 4] = ["input_tokens", "inputTokens", "prompt_tokens", "promptTokens"];
    const OUT: [&str; 4] = ["output_tokens", "outputTokens", "completion_tokens", "completionTokens"];
    let usage = usage_object(top).or_else(|| usage_object(payload)).unwrap_or(&EMPTY);
    let input_tokens = first_number(usage, &IN).or_else(|| find_number(value, &IN, 0));
    let output_tokens = first_number(usage, &OUT).or_else(|| find_number(value, &OUT, 0));
    let usage_is_total = lower.contains("token_count")
        || record(payload.get("info")).get("total_token_usage").is_some_and(Value::is_object)
        || top.get("total_token_usage").is_some_and(Value::is_object);

    let timestamp_ms = timestamp_ms(top.get("timestamp"))
        .or_else(|| timestamp_ms(top.get("time")))
        .or_else(|| timestamp_ms(payload.get("timestamp")))
        .or_else(|| timestamp_ms(payload.get("time")));
    let duration_ms = first_number(top, &["duration_ms", "durationMs"])
        .or_else(|| first_number(payload, &["duration_ms", "durationMs"]));
    let pick = |k: &str| {
        first_number(top, &[k])
            .or_else(|| first_number(payload, &[k]))
            .or_else(|| first_number(record(payload.get("data")), &[k]))
    };
    let (turn, step) = (pick("turn"), pick("step"));

    const NAME: [&str; 3] = ["name", "toolName", "tool_name"];
    let tool_name = find_string(&payload_value, &NAME, 0).or_else(|| find_string(value, &NAME, 0));
    let text = {
        let t = text_from(&payload_value, 0);
        if t.is_empty() { text_from(value, 0) } else { t }
    };
    let display_type = humanize(&event_type);
    let title = match (kind, tool_name, role) {
        (Kind::Tool, Some(n), _) => n.to_string(),
        (_, _, Some(r)) => humanize(r.as_str()),
        _ => display_type.clone(),
    };
    const MODEL: [&str; 3] = ["model", "model_id", "modelId"];
    let model = find_string(value, &MODEL, 0).or_else(|| find_string(&payload_value, &MODEL, 0));
    let cwd = find_string(value, &["cwd", "working_directory", "workingDirectory"], 0)
        .or_else(|| find_string(&payload_value, &["cwd"], 0));

    let title = if title.is_empty() { "Event".to_string() } else { title };
    NormalizedEvent {
        event_type: clip(&event_type, TYPE_CHARS),
        kind,
        role,
        summary: if text.is_empty() { title.clone() } else { text },
        title: clip(&title, TITLE_CHARS),
        timestamp_ms,
        duration_ms,
        input_tokens,
        output_tokens,
        usage_is_total,
        turn,
        step,
        depth,
        model: model.map(str::to_string),
        cwd: cwd.map(str::to_string),
        error_flag: has_error_field(top)
            || has_error_field(payload)
            || find_true(value, &["is_error", "isError", "isApiErrorMessage"], 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_user_line() {
        let e = normalize(&json!({
            "type": "user", "timestamp": "2026-10-01T00:00:00Z", "cwd": "/w",
            "message": {"role": "user", "content": "hello\nworld"}
        }));
        assert_eq!((e.kind, e.role, e.depth), (Kind::Message, Some(Role::User), 0));
        assert_eq!(e.title, "User");
        assert_eq!(e.summary, "hello\nworld");
        assert_eq!(e.cwd.as_deref(), Some("/w"));
        assert_eq!(e.timestamp_ms, Some(1_790_812_800_000));
    }

    #[test]
    fn claude_assistant_usage_is_found_under_message() {
        let e = normalize(&json!({
            "type": "assistant",
            "message": {"role": "assistant", "model": "claude-opus-5-5",
                "content": [{"type": "text", "text": "hi"}],
                "usage": {"input_tokens": 10, "output_tokens": 3}}
        }));
        assert_eq!(e.role, Some(Role::Assistant));
        assert_eq!((e.input_tokens, e.output_tokens), (Some(10.0), Some(3.0)));
        assert_eq!(e.model.as_deref(), Some("claude-opus-5-5"));
        assert!(!e.usage_is_total);
    }

    #[test]
    fn codex_function_call_and_output() {
        let call = normalize(&json!({
            "type": "response_item", "timestamp": "2026-10-01T00:00:01Z",
            "payload": {"type": "function_call", "name": "shell", "arguments": "{\"cmd\":\"ls\"}"}
        }));
        assert_eq!(call.event_type, "response_item/function_call");
        assert_eq!((call.kind, call.role, call.depth), (Kind::Tool, Some(Role::Tool), 1));
        assert_eq!(call.title, "shell");

        let out = normalize(&json!({
            "type": "response_item",
            "payload": {"type": "function_call_output", "output": "permission denied"}
        }));
        assert_eq!((out.kind, out.role, out.depth), (Kind::Tool, Some(Role::ToolResult), 2));
        assert!(out.is_error());
    }

    #[test]
    fn codex_token_count_is_total() {
        let e = normalize(&json!({
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {"total_token_usage": {"input_tokens": 500, "output_tokens": 50}}}
        }));
        assert_eq!(e.kind, Kind::Telemetry);
        assert!(e.usage_is_total);
        assert_eq!(e.input_tokens, Some(500.0));
    }

    #[test]
    fn pi_tool_result() {
        let e = normalize(&json!({
            "type": "message", "timestamp": "2026-10-01T00:00:02Z",
            "message": {"role": "toolResult", "toolName": "bash", "content": [{"type": "text", "text": "ok"}]}
        }));
        assert_eq!((e.kind, e.role, e.depth), (Kind::Tool, Some(Role::ToolResult), 2));
        assert_eq!(e.title, "bash");
        assert_eq!(e.summary, "ok");
    }

    #[test]
    fn omp_message() {
        let e = normalize(&json!({
            "type": "message", "message": {"role": "assistant", "model": "m1", "content": [{"type": "text", "text": "done"}]}
        }));
        assert_eq!((e.kind, e.role), (Kind::Message, Some(Role::Assistant)));
        assert_eq!(e.model.as_deref(), Some("m1"));
    }

    #[test]
    fn dsh_data_envelope() {
        let e = normalize(&json!({
            "type": "tool/call", "seq": 4, "time": 1_759_276_800_000_i64,
            "data": {"name": "read_file", "arguments": {"path": "a"}, "callId": "c1", "step": 2, "turn": 1}
        }));
        assert_eq!((e.kind, e.role, e.depth), (Kind::Tool, Some(Role::Tool), 1));
        assert_eq!(e.title, "read_file");
        assert_eq!((e.turn, e.step), (Some(1.0), Some(2.0)));
        assert_eq!(e.timestamp_ms, Some(1_759_276_800_000));

        let r = normalize(&json!({"type": "tool/result", "data": {"error": "boom", "message": "x", "step": 2, "turn": 1}}));
        assert_eq!((r.role, r.depth), (Some(Role::ToolResult), 2));
        assert!(r.is_error());
    }

    #[test]
    fn grok_chat_history_entries() {
        let u = normalize(&json!({"type": "user", "content": [{"type": "text", "text": "fix it"}]}));
        assert_eq!((u.kind, u.role, u.summary.as_str()), (Kind::Message, Some(Role::User), "fix it"));
        let t = normalize(&json!({"type": "tool_result", "tool_call_id": "t1", "content": "ok"}));
        assert_eq!((t.kind, t.role, t.depth), (Kind::Tool, Some(Role::ToolResult), 2));
        let a = normalize(&json!({"type": "assistant", "model_id": "grok-5", "content": "x"}));
        assert_eq!(a.model.as_deref(), Some("grok-5"));
    }

    #[test]
    fn humanize_and_clip() {
        assert_eq!(humanize("response_item/message"), "Response Item Message");
        assert_eq!(clip("abcdef", 3), "abc\n…");
        assert_eq!(normalize_text("a  \nb"), "a\nb");
    }
}
