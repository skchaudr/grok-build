//! Read-only Phoenix REST client: one session's spans, mapped to call structure.
//! Never blocks: connect and total timeouts are short and any failure is an `Err`.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::Duration;

/// Phoenix 21 on old-air.
pub const DEFAULT_BASE: &str = "http://100.109.233.105:6006";
const PAGE_LIMIT: u32 = 1000;
// shortcut: page cap bounds a runaway cursor loop, raise if a session exceeds 20k spans.
const MAX_PAGES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanStatus {
    Ok,
    Error,
    Unset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoenixSpan {
    pub span_id: String,
    pub trace_id: String,
    /// `tool_use_id` attribute when the emitter sets it.
    pub call_id: Option<String>,
    pub name: String,
    pub start: DateTime<Utc>,
    /// Always set: OTLP exports a span when it ends, so Phoenix never holds a running
    /// span. Running rows and liveness must come from receipts, not from Phoenix.
    pub end: DateTime<Utc>,
    pub status: SpanStatus,
    pub parent_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub transcript_path: Option<String>,
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn time(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(str_at(v, key)?).ok().map(|t| t.with_timezone(&Utc))
}

/// Map one Phoenix span object. `None` when it lacks a span id, start or end time.
pub fn parse_span(v: &Value) -> Option<PhoenixSpan> {
    let ctx = v.get("context")?;
    let attrs = v.get("attributes").unwrap_or(&Value::Null);
    let attr = |k: &str| str_at(attrs, k).map(str::to_owned);
    Some(PhoenixSpan {
        span_id: str_at(ctx, "span_id")?.to_owned(),
        trace_id: str_at(ctx, "trace_id").unwrap_or_default().to_owned(),
        call_id: attr("tool_use_id"),
        name: str_at(v, "name").unwrap_or_default().to_owned(),
        start: time(v, "start_time")?,
        end: time(v, "end_time")?,
        status: match str_at(v, "status_code") {
            Some("OK") => SpanStatus::Ok,
            Some("ERROR") => SpanStatus::Error,
            _ => SpanStatus::Unset,
        },
        parent_id: str_at(v, "parent_id").map(str::to_owned),
        session_id: attr("session.id"),
        agent_id: attr("agent_id"),
        transcript_path: attr("transcript_path"),
    })
}

/// Parse one `/spans` response page into spans and the next cursor.
pub fn parse_page(v: &Value) -> (Vec<PhoenixSpan>, Option<String>) {
    let spans = v.get("data").and_then(Value::as_array).map(|a| a.iter().filter_map(parse_span).collect()).unwrap_or_default();
    (spans, str_at(v, "next_cursor").map(str::to_owned))
}

/// Fetch every span of `session_id` in `project`, oldest first. Errors (including
/// Phoenix being unreachable) come back within `timeout`; nothing is retried.
pub fn fetch_session_spans(base: &str, project: &str, session_id: &str, timeout: Duration) -> Result<Vec<PhoenixSpan>, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("phoenix client: {e}"))?;
    let url = format!("{}/v1/projects/{project}/spans", base.trim_end_matches('/'));
    // JSON-quoted so a numeric-looking session id still matches as a string.
    let filter = format!("session.id:{}", Value::String(session_id.to_owned()));
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut req = client
            .get(&url)
            .query(&[("limit", PAGE_LIMIT.to_string()), ("sort", "start_time".into()), ("order", "asc".into()), ("attribute", filter.clone())]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c)]);
        }
        let resp = req.send().map_err(|e| format!("phoenix unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("phoenix http {}", resp.status()));
        }
        let body: Value = resp.json().map_err(|e| format!("phoenix bad json: {e}"))?;
        let (spans, next) = parse_page(&body);
        out.extend(spans);
        match next {
            Some(n) => cursor = Some(n),
            None => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn span(extra: Value) -> Value {
        let mut v = json!({
            "name": "claude tool:Bash",
            "context": {"trace_id": "t1", "span_id": "s1"},
            "parent_id": "p1",
            "start_time": "2026-01-02T03:04:05.250+00:00",
            "end_time": "2026-01-02T03:04:07.000+00:00",
            "status_code": "OK",
            "attributes": {"session.id": "sess", "tool_use_id": "toolu_x", "agent_id": "a1", "transcript_path": "/tmp/t.jsonl"}
        });
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    }

    #[test]
    fn maps_every_field() {
        let s = parse_span(&span(json!({}))).unwrap();
        assert_eq!(s.span_id, "s1");
        assert_eq!(s.trace_id, "t1");
        assert_eq!(s.call_id.as_deref(), Some("toolu_x"));
        assert_eq!(s.name, "claude tool:Bash");
        assert_eq!(s.end - s.start, chrono::Duration::milliseconds(1750));
        assert_eq!(s.status, SpanStatus::Ok);
        assert_eq!(s.parent_id.as_deref(), Some("p1"));
        assert_eq!(s.session_id.as_deref(), Some("sess"));
        assert_eq!(s.agent_id.as_deref(), Some("a1"));
        assert_eq!(s.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    }

    #[test]
    fn missing_attrs() {
        let s = parse_span(&span(json!({"status_code": "UNSET", "parent_id": null, "attributes": {}}))).unwrap();
        assert_eq!(s.status, SpanStatus::Unset);
        assert_eq!(s.parent_id, None);
        assert_eq!((s.call_id, s.session_id, s.agent_id, s.transcript_path), (None, None, None, None));
    }

    #[test]
    fn error_status_and_unparseable_spans() {
        assert_eq!(parse_span(&span(json!({"status_code": "ERROR"}))).unwrap().status, SpanStatus::Error);
        assert!(parse_span(&span(json!({"start_time": "nope"}))).is_none());
        assert!(parse_span(&span(json!({"end_time": null}))).is_none());
        assert!(parse_span(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn page_skips_bad_spans_and_returns_cursor() {
        let page = json!({"data": [span(json!({})), {"junk": 1}], "next_cursor": "c2"});
        let (spans, next) = parse_page(&page);
        assert_eq!(spans.len(), 1);
        assert_eq!(next.as_deref(), Some("c2"));
        let (spans, next) = parse_page(&json!({"data": [], "next_cursor": null}));
        assert!(spans.is_empty() && next.is_none());
    }

    #[test]
    fn unreachable_phoenix_errors_fast() {
        let t = std::time::Instant::now();
        // TEST-NET-1 address: never answers.
        let r = fetch_session_spans("http://192.0.2.1:6006", "claude", "s", Duration::from_millis(300));
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(3));
    }
}
