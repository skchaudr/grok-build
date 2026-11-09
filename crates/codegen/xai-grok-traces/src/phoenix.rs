//! Read-only Phoenix REST client: one session's spans, mapped to call structure.
//! Never blocks: connect and total timeouts are short and any failure is an `Err`.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::{Duration, Instant};

/// Phoenix 21 on old-air.
pub const DEFAULT_BASE: &str = "http://100.109.233.105:6006";
const PAGE_LIMIT: u32 = 1000;
// shortcut: page cap bounds a runaway cursor loop and errors past 20k spans, raise if a real session hits it.
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
    /// Span name as emitted, harness prefix included (`grok tool:read_file`).
    pub name: String,
    /// `hook.event` (PreToolUse, PostToolUse, Stop, ...): tells a call's start span from
    /// its end span when the emitter sends both (grok), and calls from non-call spans.
    pub hook_event: Option<String>,
    /// `tool.name`; `None` on non-call spans.
    pub tool_name: Option<String>,
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
        hook_event: attr("hook.event"),
        tool_name: attr("tool.name"),
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

/// Fetch every span of `session_id` in `project`, oldest first. The whole fetch,
/// every page included, ends within `timeout`; a timeout, an unreachable Phoenix or
/// a session past the page cap is an `Err`, never a partial `Ok`. Nothing is retried.
///
/// `session_id` must be the `session.id` string the emitter put on its spans, matched
/// exactly. That is not always the harness's own id: live Pi spans carry `pi-<epoch ms>`
/// (not the Pi jsonl header id) and dpi spans carry `<uuid>:1` (not the bare uuid).
/// A harness id that differs matches nothing, and `Ok(vec![])` then looks the same as a
/// session with no spans, so callers must resolve the emitted id first.
pub fn fetch_session_spans(base: &str, project: &str, session_id: &str, timeout: Duration) -> Result<Vec<PhoenixSpan>, String> {
    let deadline = Instant::now() + timeout;
    // One client per process: building one loads the system trust store, which cost the live view's poll most of its CPU.
    static CLIENT: std::sync::OnceLock<Result<reqwest::blocking::Client, String>> = std::sync::OnceLock::new();
    let client = CLIENT
        .get_or_init(|| reqwest::blocking::Client::builder().build().map_err(|e| format!("phoenix client: {e}")))
        .as_ref()
        .map_err(Clone::clone)?;
    let url = format!("{}/v1/projects/{project}/spans", base.trim_end_matches('/'));
    // JSON-quoted so a numeric-looking session id still matches as a string.
    let filter = format!("session.id:{}", Value::String(session_id.to_owned()));
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("phoenix timed out".into());
        }
        let mut req = client
            .get(&url)
            .timeout(left)
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
            None => return Ok(out),
        }
    }
    Err(format!("phoenix: session has more than {} spans", MAX_PAGES as u32 * PAGE_LIMIT))
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
            "attributes": {"session.id": "sess", "tool_use_id": "toolu_x", "hook.event": "PostToolUse", "tool.name": "Bash", "agent_id": "a1", "transcript_path": "/tmp/t.jsonl"}
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
        assert_eq!(s.hook_event.as_deref(), Some("PostToolUse"));
        assert_eq!(s.tool_name.as_deref(), Some("Bash"));
        assert_eq!(s.end - s.start, chrono::Duration::milliseconds(1750));
        assert_eq!(s.status, SpanStatus::Ok);
        assert_eq!(s.parent_id.as_deref(), Some("p1"));
        assert_eq!(s.session_id.as_deref(), Some("sess"));
        assert_eq!(s.agent_id.as_deref(), Some("a1"));
        assert_eq!(s.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    }

    #[test]
    fn missing_attrs() {
        let s = parse_span(&span(json!({"status_code": "UNSET", "parent_id": null, "attributes": {"tool.name": ""}}))).unwrap();
        assert_eq!((s.hook_event, s.tool_name), (None, None));
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

    /// One-thread HTTP stub on loopback: `handler(request_index, request_line)` returns
    /// (status, body, delay before replying). Returns the base URL and the request lines seen.
    fn stub(handler: impl Fn(usize, &str) -> (u16, String, Duration) + Send + 'static) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (i, conn) in listener.incoming().enumerate() {
                let Ok(mut conn) = conn else { return };
                let mut rd = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                rd.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    if rd.read_line(&mut h).unwrap_or(0) <= 2 {
                        break;
                    }
                }
                log.lock().unwrap().push(line.clone());
                let (code, body, delay) = handler(i, &line);
                std::thread::sleep(delay);
                let _ = write!(conn, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, seen)
    }

    const FAST: Duration = Duration::ZERO;

    #[test]
    fn follows_cursor_across_pages() {
        let (base, seen) = stub(|i, _| {
            let body = if i == 0 { json!({"data": [span(json!({}))], "next_cursor": "c2"}) } else { json!({"data": [span(json!({"context": {"trace_id": "t1", "span_id": "s2"}}))], "next_cursor": null}) };
            (200, body.to_string(), FAST)
        });
        let spans = fetch_session_spans(&base, "claude", "sess", Duration::from_secs(5)).unwrap();
        assert_eq!(spans.iter().map(|s| s.span_id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
        let seen = seen.lock().unwrap();
        assert!(!seen[0].contains("cursor=") && seen[1].contains("cursor=c2"), "{seen:?}");
        assert!(seen[0].contains("/v1/projects/claude/spans") && seen[0].contains("session.id"));
    }

    #[test]
    fn page_cap_is_an_error_not_a_partial_ok() {
        let (base, seen) = stub(|_, _| (200, json!({"data": [span(json!({}))], "next_cursor": "more"}).to_string(), FAST));
        let r = fetch_session_spans(&base, "claude", "s", Duration::from_secs(10));
        assert!(r.unwrap_err().contains("more than"));
        assert_eq!(seen.lock().unwrap().len(), MAX_PAGES);
    }

    #[test]
    fn non_2xx_is_an_error() {
        let (base, _) = stub(|_, _| (500, "{}".into(), FAST));
        assert!(fetch_session_spans(&base, "claude", "s", Duration::from_secs(5)).unwrap_err().contains("http 500"));
    }

    #[test]
    fn deadline_covers_the_whole_multi_page_fetch() {
        // Each page is well inside the timeout; 20 of them are not. Without a shared deadline this takes ~3s.
        let (base, _) = stub(|_, _| (200, json!({"data": [], "next_cursor": "more"}).to_string(), Duration::from_millis(150)));
        let t = Instant::now();
        assert!(fetch_session_spans(&base, "claude", "s", Duration::from_millis(500)).is_err());
        assert!(t.elapsed() < Duration::from_millis(1500), "{:?}", t.elapsed());
    }

    #[test]
    fn silent_phoenix_is_bounded() {
        let (base, _) = stub(|_, _| (200, "{}".into(), Duration::from_secs(5)));
        let t = Instant::now();
        assert!(fetch_session_spans(&base, "claude", "s", Duration::from_millis(300)).is_err());
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn refused_connection_is_an_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(fetch_session_spans(&format!("http://127.0.0.1:{port}"), "claude", "s", Duration::from_millis(300)).is_err());
    }
}
