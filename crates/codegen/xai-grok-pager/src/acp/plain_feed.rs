//! Sessions with no harness converter (omp, bb, cursor, dsh, and the rest) as the events grok's
//! session view already draws. A tool result folds into the call it answers, so one call stays one row.

use agent_client_protocol as acp;
use serde_json::{Map, Value, json};
use xai_grok_traces::EventRow;

use super::claude_feed::{Event, Fed, chunk, text_content, tool_call};

/// `texts[i]` is the body for `events[i]` (the raw line when we have it, else the index preview).
pub(crate) fn from_events(events: &[EventRow], texts: &[String], live: bool) -> Vec<Fed> {
    let mut items = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if e.noise {
            continue;
        }
        let text = texts.get(i).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| {
            if e.preview.is_empty() { e.title.clone() } else { e.preview.clone() }
        });
        push_item(&mut items, &e.kind, e.role.as_deref(), e.line, &e.title, text, e.is_error, e.duration_ms, e.timestamp);
    }
    finish(items, live, "plain")
}

/// One JSON object per line. A line that is not JSON is skipped.
pub(crate) fn from_jsonl(raw: &str, live: bool) -> Vec<Fed> {
    let mut items = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let role = v.get("role").and_then(Value::as_str).or_else(|| v.pointer("/message/role").and_then(Value::as_str));
        let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
        let title = v.get("name").and_then(Value::as_str).unwrap_or("tool");
        let text = crate::views::traces_modal::body_text(line);
        if text.trim().is_empty() {
            continue;
        }
        let (kind, role) = sniff(typ, role);
        push_item(&mut items, kind, role, i as i64 + 1, title, text, false, None, None);
    }
    finish(items, live, "jsonl")
}

fn sniff<'a>(typ: &str, role: Option<&'a str>) -> (&'static str, Option<&'a str>) {
    let typ_l = typ.to_ascii_lowercase();
    if role == Some("user") || typ_l == "user" || typ_l == "human" {
        return ("message", Some("user"));
    }
    if role == Some("tool_result") || typ_l.contains("tool_result") || typ_l == "function_call_output" {
        return ("tool", Some("tool_result"));
    }
    if role == Some("reasoning") || typ_l == "reasoning" || typ_l == "thinking" {
        return ("reasoning", role);
    }
    if role == Some("tool") || typ_l.contains("tool") || typ_l.contains("function_call") {
        return ("tool", role);
    }
    ("message", role)
}

struct Item {
    kind: Kind,
    line: i64,
    title: String,
    text: String,
    error: bool,
    dur: Option<i64>,
    ts: Option<i64>,
}

enum Kind {
    User,
    Thought,
    Assistant,
    Tool { open: bool },
}

fn push_item(items: &mut Vec<Item>, kind: &str, role: Option<&str>, line: i64, title: &str, text: String, error: bool, dur: Option<i64>, ts: Option<i64>) {
    if text.trim().is_empty() && kind != "tool" {
        return;
    }
    let text = if text.trim().is_empty() { title.to_string() } else { text };
    if kind == "tool" && role == Some("tool_result") {
        if let Some(Item { kind: Kind::Tool { open }, text: body, error: err, .. }) = items.iter_mut().rev().find(|i| matches!(i.kind, Kind::Tool { open: true })) {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(&text);
            *open = false;
            *err = error;
        } else {
            items.push(Item { kind: Kind::Assistant, line, title: title.into(), text, error, dur, ts });
        }
        return;
    }
    let kind = if kind == "tool" {
        Kind::Tool { open: true }
    } else if kind == "reasoning" || role == Some("reasoning") {
        Kind::Thought
    } else if role == Some("user") {
        Kind::User
    } else {
        Kind::Assistant
    };
    items.push(Item { kind, line, title: title.into(), text, error, dur, ts });
}

fn finish(items: Vec<Item>, live: bool, sid: &str) -> Vec<Fed> {
    let last_open = items.iter().rposition(|i| matches!(i.kind, Kind::Tool { open: true }));
    let mut seq = 0u64;
    let mut prompt = String::new();
    let mut prompts = 0u64;
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            if matches!(item.kind, Kind::User) {
                prompts += 1;
                prompt = format!("{sid}-p{prompts}");
            }
            let mut meta = meta(&mut seq, sid, &prompt, item.ts);
            if matches!(item.kind, Kind::User) {
                if let Some(ts) = item.ts {
                    meta.insert("turnStartMs".into(), json!(ts));
                }
            }
            let event = match item.kind {
                Kind::User => Event::Acp(acp_user(&item.text)),
                Kind::Thought => Event::Acp(acp_thought(&item.text)),
                Kind::Assistant => Event::Acp(acp_assistant(&item.text)),
                Kind::Tool { open } => {
                    let running = live && open && last_open == Some(i);
                    let status = if running {
                        acp::ToolCallStatus::InProgress
                    } else if item.error {
                        acp::ToolCallStatus::Failed
                    } else {
                        acp::ToolCallStatus::Completed
                    };
                    let body = item.text;
                    Event::Acp(acp::SessionUpdate::ToolCall(tool_call(
                        &format!("{sid}-{}", item.line),
                        acp::ToolKind::Other,
                        item.title,
                        json!(&body),
                        status,
                        vec![text_content(&body)],
                        None,
                        item.dur,
                    )))
                }
            };
            Fed { event, meta }
        })
        .collect()
}

fn meta(seq: &mut u64, sid: &str, prompt: &str, ts: Option<i64>) -> Map<String, Value> {
    *seq += 1;
    let mut m = Map::new();
    m.insert("isReplay".into(), true.into());
    m.insert("eventId".into(), json!(format!("{sid}-{}", *seq)));
    if !prompt.is_empty() {
        m.insert("promptId".into(), json!(prompt));
    }
    if let Some(ts) = ts {
        m.insert("agentTimestampMs".into(), json!(ts));
    }
    m
}

fn acp_user(text: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::UserMessageChunk(chunk(text))
}
fn acp_assistant(text: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::AgentMessageChunk(chunk(text))
}
fn acp_thought(text: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::AgentThoughtChunk(chunk(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp::ToolCallStatus;

    fn ev(line: i64, kind: &str, role: Option<&str>, title: &str) -> EventRow {
        EventRow {
            line,
            event_type: kind.into(),
            kind: kind.into(),
            role: role.map(str::to_string),
            title: title.into(),
            preview: String::new(),
            timestamp: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: None,
            step: None,
            depth: 0,
            model: None,
            is_error: false,
            noise: false,
            detail: None,
            intent: None,
        }
    }

    fn tool_status(fed: &Fed) -> Option<ToolCallStatus> {
        match &fed.event {
            Event::Acp(acp::SessionUpdate::ToolCall(tc)) => Some(tc.status),
            _ => None,
        }
    }

    #[test]
    fn a_result_folds_into_its_call_and_a_live_call_stays_open() {
        let events = vec![ev(1, "message", Some("user"), "go"), ev(2, "tool", None, "Bash"), ev(3, "tool", Some("tool_result"), "Bash")];
        let texts = vec!["go".into(), "ls".into(), "a.rs".into()];
        let done = from_events(&events, &texts, false);
        assert_eq!(done.len(), 2, "the result is part of the call");
        assert_eq!(tool_status(&done[1]), Some(ToolCallStatus::Completed));
        let live_events = vec![ev(1, "message", Some("user"), "go"), ev(2, "tool", None, "Bash")];
        let live = from_events(&live_events, &["go".into(), "ls".into()], true);
        assert_eq!(tool_status(&live[1]), Some(ToolCallStatus::InProgress));
    }

    #[test]
    fn jsonl_user_and_tool_lines_become_those_rows() {
        let raw = "{\"role\":\"user\",\"content\":\"hi\"}\n{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}\n";
        let fed = from_jsonl(raw, false);
        assert_eq!(fed.len(), 2);
        assert!(matches!(fed[0].event, Event::Acp(acp::SessionUpdate::UserMessageChunk(_))));
        assert_eq!(tool_status(&fed[1]), Some(ToolCallStatus::Completed));
    }
}
