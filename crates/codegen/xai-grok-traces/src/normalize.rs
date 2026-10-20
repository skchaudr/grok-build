//! One JSON line from any harness → one [`NormalizedEvent`].
//!
//! Ported from bb-plugin-traces `normalizeRecord`. Heuristics key off type
//! and role names; the only envelope handling is `payload` (Codex), `data`
//! (DSH) and `message` (Claude). Lengths count chars, not UTF-16 units.

use serde_json::{Map, Value};

/// Transient per-event text (error heuristic, titles); only the preview is stored.
const TEXT_CHARS: usize = 1024;
/// Stored per-event preview; the full line is re-read from the source on demand.
pub const PREVIEW_CHARS: usize = 100;
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
    pub const ALL: [Kind; 7] =
        [Kind::Message, Kind::Tool, Kind::Step, Kind::Turn, Kind::Reasoning, Kind::Telemetry, Kind::System];

    /// Small integer stored in the index; `ALL[code]` is the inverse.
    pub fn code(self) -> i64 {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as i64
    }

    pub fn from_code(code: i64) -> Option<Kind> {
        usize::try_from(code).ok().and_then(|i| Self::ALL.get(i).copied())
    }

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
    pub const ALL: [Role; 5] = [Role::User, Role::Assistant, Role::Tool, Role::ToolResult, Role::Reasoning];

    /// Small integer stored in the index; `ALL[code]` is the inverse.
    pub fn code(self) -> i64 {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0) as i64
    }

    pub fn from_code(code: i64) -> Option<Role> {
        usize::try_from(code).ok().and_then(|i| Self::ALL.get(i).copied())
    }

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
    /// Up to `TEXT_CHARS` of content; not stored.
    pub text: String,
    /// Whitespace-collapsed `text`, clipped to `PREVIEW_CHARS`; this is what the index keeps.
    pub preview: String,
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
    /// A structured error marker (see [`structured_error`]); never inferred from output text.
    pub error_flag: bool,
    /// Bookkeeping, telemetry or injected context; the default trajectory hides it.
    pub noise: bool,
    /// Tool call: compact argument summary. Tool result: `exit N` or the first line of an error.
    pub detail: Option<String>,
    /// Name of the tool being called (calls only; not stored).
    pub tool: Option<String>,
    /// A title the harness itself recorded for the session (Claude `ai-title`, OMP `title`, DSH `session/title`).
    pub session_title: Option<String>,
}

impl NormalizedEvent {
    pub fn is_error(&self) -> bool {
        self.error_flag
    }
}

static EMPTY: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);

pub(crate) fn record(v: Option<&Value>) -> &Map<String, Value> {
    v.and_then(Value::as_object).unwrap_or(&EMPTY)
}

pub(crate) fn string_value(v: Option<&Value>) -> Option<&str> {
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

/// One line, single spaces, at most `PREVIEW_CHARS` chars.
pub fn preview(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(PREVIEW_CHARS) {
        None => flat,
        Some((i, _)) => format!("{}…", &flat[..i]),
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

/// What `text_from` removed: the first injected wrapper tag seen.
#[derive(Default)]
struct Injected(Option<String>);

fn normalize_text(s: &str, inj: &mut Injected) -> String {
    let unwrapped = crate::cursor::unwrap_user_query(s);
    let (stripped, tag) = crate::classify::strip_injected(unwrapped.as_ref());
    if inj.0.is_none() {
        inj.0 = tag;
    }
    let s = stripped.as_ref();
    let long = s.chars().count() > TEXT_CHARS;
    let head: String = s.chars().take(TEXT_CHARS).collect();
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

fn text_from(v: &Value, depth: usize, inj: &mut Injected) -> String {
    if depth > TEXT_DEPTH {
        return String::new();
    }
    match v {
        Value::String(s) => normalize_text(s, inj),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                let part = text_from(item, depth + 1, inj);
                if part.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&part);
                if out.chars().count() >= TEXT_CHARS {
                    return clip(&out, TEXT_CHARS);
                }
            }
            out
        }
        Value::Object(map) => TEXT_KEYS
            .iter()
            .filter_map(|k| map.get(*k))
            .map(|v| text_from(v, depth + 1, inj))
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

fn first_number(obj: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| number_value(obj.get(*k)))
}

fn detect_role(event_type: &str, top: &Map<String, Value>, payload: &Map<String, Value>) -> Option<Role> {
    let explicit = string_value(top.get("role"))
        .or_else(|| string_value(payload.get("role")))
        .or_else(|| string_value(record(payload.get("message")).get("role")))
        .or_else(|| string_value(record(payload.get("data")).get("role")));
    // DSH wraps a tool result in a user-role message.
    let lower_type = event_type.to_lowercase();
    if matches!(explicit, Some("user" | "human")) && (lower_type.contains("tool/result") || lower_type.contains("tool-result")) {
        return Some(Role::ToolResult);
    }
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

/// A message whose content is only thinking blocks (Claude writes one line per block).
fn thinking_only(payload: &Map<String, Value>) -> Option<String> {
    let blocks = payload.get("content")?.as_array()?;
    if blocks.is_empty() {
        return None;
    }
    let mut text = String::new();
    for b in blocks {
        match b.get("type").and_then(Value::as_str)? {
            "thinking" | "redacted_thinking" => {
                if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            _ => return None,
        }
    }
    Some(text)
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
        (None, None) => string_value(top.get("role")).unwrap_or("record").to_string(),
    };

    let thinking = thinking_only(payload);
    let role = if thinking.is_some() {
        Some(Role::Reasoning)
    } else {
        content_tool_role(payload).or_else(|| detect_role(&event_type, top, payload))
    };
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
    let mut inj = Injected::default();
    let mut text = thinking.clone().map(|t| normalize_text(&t, &mut inj)).unwrap_or_else(|| text_from(&payload_value, 0, &mut inj));
    if text.is_empty() && inj.0.is_none() {
        text = text_from(value, 0, &mut inj);
    }
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

    // Noise: bookkeeping record types, telemetry, injected user context, empty reasoning.
    let explicit_role = string_value(top.get("role")).or_else(|| string_value(payload.get("role")));
    let had_text = !text.is_empty();
    let mut noise = crate::classify::noise_type(&event_type)
        || kind == Kind::Telemetry
        || matches!(explicit_role, Some("developer" | "system"))
        || (kind == Kind::Reasoning && !had_text);
    if role == Some(Role::User) {
        let flagged = top.get("isMeta") == Some(&Value::Bool(true))
            || top.get("isCompactSummary") == Some(&Value::Bool(true))
            || string_value(top.get("synthetic_reason")).is_some();
        if flagged || (!had_text && inj.0.is_some()) || crate::classify::harness_prompt(&text) {
            noise = true;
        }
    }
    if noise && !had_text {
        if let Some(tag) = &inj.0 {
            text = format!("<{tag}>");
        }
    }

    // Structured errors; Codex writes an exit-status header instead of a flag.
    let is_result = role == Some(Role::ToolResult);
    let exit = (is_result && matches!(nested, Some("function_call_output" | "custom_tool_call_output")))
        .then(|| crate::classify::exit_status(&text))
        .flatten();
    let error_flag = crate::classify::structured_error(top, payload, &event_type) || exit.as_ref().is_some_and(|(failed, _)| *failed);

    // Tool-call summary, or a tool result's exit status / first error line.
    let flat_call = role == Some(Role::Tool) && kind == Kind::Tool;
    let call = (flat_call || payload.contains_key("tool_calls") || top.contains_key("tool_calls") || payload.contains_key("content"))
        .then(|| crate::classify::tool_call(top, payload, flat_call))
        .flatten();
    let tool = call.as_ref().map(|(n, _)| n.clone());
    let detail = if let Some((name, args)) = &call {
        crate::classify::call_detail(name, args)
    } else if is_result {
        exit.map(|(_, d)| d).or_else(|| if error_flag { crate::classify::first_line(&text) } else { None })
    } else {
        None
    };

    let session_title = session_title(top, payload);
    let title = if title.is_empty() { "Event".to_string() } else { title };
    let text = if text.is_empty() { title.clone() } else { text };
    NormalizedEvent {
        event_type: clip(&event_type, TYPE_CHARS),
        kind,
        role,
        preview: preview(&text),
        text,
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
        error_flag,
        noise,
        detail,
        tool,
        session_title,
    }
}

/// A title the harness wrote itself: Claude `ai-title`, OMP `title`/`title_change`, DSH `session/title`.
fn session_title(top: &Map<String, Value>, payload: &Map<String, Value>) -> Option<String> {
    let t = string_value(top.get("type"))?;
    let raw = match t {
        "ai-title" => string_value(top.get("aiTitle")),
        "title" | "title_change" => string_value(top.get("title")),
        "session/title" => string_value(payload.get("title")),
        _ => None,
    }?;
    let line = raw.lines().next().unwrap_or(raw).trim();
    (!line.is_empty()).then(|| clip(line, TITLE_CHARS))
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
        assert_eq!(e.text, "hello\nworld");
        assert_eq!(e.preview, "hello world");
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
            "payload": {"type": "function_call_output", "output": "Chunk ID: a\nProcess exited with code 126\npermission denied"}
        }));
        assert_eq!((out.kind, out.role, out.depth), (Kind::Tool, Some(Role::ToolResult), 2));
        assert!(out.is_error());
        assert_eq!(out.detail.as_deref(), Some("exit 126"));
        let words = normalize(&json!({
            "type": "response_item",
            "payload": {"type": "function_call_output", "output": "Process exited with code 0\nerror: failed, denied, timeout"}
        }));
        assert!(!words.is_error(), "words in output are not errors");
        assert_eq!(words.detail.as_deref(), Some("exit 0"));
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
        assert_eq!(e.text, "ok");
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
        assert_eq!((u.kind, u.role, u.text.as_str()), (Kind::Message, Some(Role::User), "fix it"));
        let t = normalize(&json!({"type": "tool_result", "tool_call_id": "t1", "content": "ok"}));
        assert_eq!((t.kind, t.role, t.depth), (Kind::Tool, Some(Role::ToolResult), 2));
        let a = normalize(&json!({"type": "assistant", "model_id": "grok-5", "content": "x"}));
        assert_eq!(a.model.as_deref(), Some("grok-5"));
    }

    #[test]
    fn cursor_lines() {
        let u = normalize(&json!({"role": "user", "message": {"content": [
            {"type": "text", "text": "<timestamp>Tue</timestamp>\n<user_query>\nfix it\n</user_query>"}]}}));
        assert_eq!((u.event_type.as_str(), u.kind, u.role), ("user", Kind::Message, Some(Role::User)));
        assert_eq!((u.text.as_str(), u.preview.as_str()), ("fix it", "fix it"));
        let t = normalize(&json!({"role": "assistant", "message": {"content": [
            {"type": "text", "text": "looking"}, {"type": "tool_use", "name": "Shell", "input": {"command": "ls"}}]}}));
        assert_eq!((t.kind, t.role, t.title.as_str()), (Kind::Tool, Some(Role::Tool), "Shell"));
        let end = normalize(&json!({"type": "turn_ended", "status": "success"}));
        assert_eq!(end.kind, Kind::Turn);
    }

    #[test]
    fn noise_detail_and_errors_across_shapes() {
        // Grok: tool calls ride on the assistant line.
        let a = normalize(&json!({"type": "assistant", "content": "I'll look.", "tool_calls": [
            {"id": "t1", "name": "run_terminal_command", "arguments": "{\"command\":\"node q.mjs \\\"x\\\"\"}"}]}));
        assert_eq!((a.role, a.tool.as_deref(), a.detail.as_deref()), (Some(Role::Assistant), Some("run_terminal_command"), Some("node q.mjs \"x\"")));
        assert!(!a.noise);
        // Grok injected user turns and system prompt.
        let s = normalize(&json!({"type": "user", "synthetic_reason": "system_reminder", "content": [{"type": "text", "text": "<system-reminder>\nctx\n</system-reminder>"}]}));
        assert!(s.noise && s.role == Some(Role::User));
        assert!(normalize(&json!({"type": "system", "content": "prompt"})).noise);
        // Claude: bookkeeping lines, thinking-only lines.
        for t in ["attachment", "file-history-snapshot", "last-prompt", "mode", "permission-mode", "api-request", "queue-operation"] {
            assert!(normalize(&json!({"type": t})).noise, "{t}");
        }
        let th = normalize(&json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": "", "signature": "s"}]}}));
        assert_eq!((th.role, th.kind, th.noise), (Some(Role::Reasoning), Kind::Reasoning, true));
        let th2 = normalize(&json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": "hmm"}]}}));
        assert_eq!((th2.noise, th2.text.as_str()), (false, "hmm"));
        // Codex bookkeeping versus content.
        assert!(normalize(&json!({"type": "event_msg", "payload": {"type": "task_started"}})).noise);
        assert!(!normalize(&json!({"type": "event_msg", "payload": {"type": "task_complete"}})).noise);
        assert!(normalize(&json!({"type": "response_item", "payload": {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "perm"}]}})).noise);
        // Pi/OMP bookkeeping.
        for t in ["session", "model_change", "thinking_level_change", "custom", "session_init"] {
            assert!(normalize(&json!({"type": t})).noise, "{t}");
        }
        // DSH: a tool result wrapped in a user message is a tool result; bookkeeping is noise; session titles surface.
        let r = normalize(&json!({"type": "tool/result", "data": {"message": {"role": "user", "content": [{"type": "tool-result", "isError": false, "content": [{"type": "text", "text": "fine"}]}]}}}));
        assert_eq!((r.role, r.is_error()), (Some(Role::ToolResult), false));
        assert!(normalize(&json!({"type": "step/start", "data": {"step": 1}})).noise);
        let t = normalize(&json!({"type": "session/title", "data": {"title": "Read the spec\nmore"}}));
        assert_eq!((t.noise, t.session_title.as_deref()), (true, Some("Read the spec")));
        assert_eq!(normalize(&json!({"type": "ai-title", "aiTitle": "Fix parser"})).session_title.as_deref(), Some("Fix parser"));
        // Cursor: an abort carries an error field but is not a failure.
        assert!(!normalize(&json!({"type": "turn_ended", "status": "aborted", "error": "User aborted"})).is_error());
        assert!(normalize(&json!({"type": "turn_ended", "status": "error"})).is_error());
    }

    #[test]
    fn humanize_and_clip() {
        assert_eq!(humanize("response_item/message"), "Response Item Message");
        assert_eq!(clip("abcdef", 3), "abc\n…");
        assert_eq!(normalize_text("a  \nb", &mut Injected::default()), "a\nb");
        assert_eq!(preview("a \n\t b"), "a b");
        let long = "x".repeat(500);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }
}
