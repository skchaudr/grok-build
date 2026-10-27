//! Cursor agent transcripts: `~/.cursor/projects/<project>/agent-transcripts/<id>/<id>.jsonl`.
//!
//! Lines are `{"role":"user|assistant","message":{"content":[{"type":"text"|"tool_use",..}]}}`
//! with no top-level `type` or timestamp, plus `{"type":"turn_ended","status":..}` markers.
//! `normalize` already reads that shape; the only Cursor-specific part is that user text is
//! wrapped as `<timestamp>..</timestamp><user_query>..</user_query>`.
//!
//! Token usage is not on every line. When a composer response does carry it, the payload is
//! `message.usage` or `tokenCount` with camelCase `inputTokens` / `outputTokens` (cache read
//! and write already included in `inputTokens`), a `turn_ended` line with the same counts, or
//! the composer context meter (`promptTokenBreakdown.totalUsedTokens`, else `contextTokensUsed`).
//! The window is `contextTokenLimit` / `context_window` / `contextWindow` on that line.
//! A transcript with none of these stays unset: message length is not a token count.

use std::borrow::Cow;

use serde_json::{Map, Value};

const OPEN: &str = "<user_query>";
const CLOSE: &str = "</user_query>";

/// The inner text of a Cursor `<user_query>` wrapper, or `s` unchanged.
pub(crate) fn unwrap_user_query(s: &str) -> Cow<'_, str> {
    let t = s.trim_start();
    if !t.starts_with('<') {
        return Cow::Borrowed(s);
    }
    match t.find(OPEN) {
        Some(a) => {
            let rest = &t[a + OPEN.len()..];
            Cow::Borrowed(rest.find(CLOSE).map_or(rest, |end| &rest[..end]).trim())
        }
        None => Cow::Borrowed(s),
    }
}

fn pos_i64(v: Option<&Value>) -> Option<i64> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && n > 0.0).then(|| n.round() as i64).filter(|n| *n > 0)
}

fn positive_tokens(m: &Map<String, Value>) -> bool {
    ["input_tokens", "inputTokens", "output_tokens", "outputTokens"]
        .iter()
        .any(|k| pos_i64(m.get(*k)).is_some())
}

fn token_map(v: Option<&Value>) -> Option<&Map<String, Value>> {
    let m = v?.as_object()?;
    positive_tokens(m).then_some(m)
}

/// A usage object the shared `usage` key missed: composer `tokenCount`, or hook counts
/// sitting on the `turn_ended` line itself. Zero counts are ignored.
pub(crate) fn extra_usage<'a>(
    top: &'a Map<String, Value>,
    payload: &'a Map<String, Value>,
) -> Option<&'a Map<String, Value>> {
    if let Some(m) = token_map(top.get("tokenCount")).or_else(|| token_map(payload.get("tokenCount"))) {
        return Some(m);
    }
    (top.get("type").and_then(Value::as_str) == Some("turn_ended") && positive_tokens(top)).then_some(top)
}

/// Composer context meter: the latest window snapshot, not a per-call bill.
/// `totalUsedTokens` is the figure behind the in-app bar; `contextTokensUsed` is the fallback.
pub(crate) fn context_meter(top: &Map<String, Value>, payload: &Map<String, Value>) -> Option<i64> {
    [top, payload].iter().find_map(|m| {
        m.get("promptTokenBreakdown")
            .and_then(Value::as_object)
            .and_then(|b| pos_i64(b.get("totalUsedTokens")))
            .or_else(|| pos_i64(m.get("contextTokensUsed")))
    })
}

/// Model window, when this line states one.
pub(crate) fn context_window(top: &Map<String, Value>, payload: &Map<String, Value>, usage: &Map<String, Value>) -> Option<i64> {
    const KEYS: [&str; 4] = ["contextTokenLimit", "context_window", "contextWindow", "context_token_limit"];
    [usage, payload, top].iter().find_map(|m| KEYS.iter().find_map(|k| pos_i64(m.get(*k))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_query_and_leaves_other_text() {
        let wrapped = "<timestamp>Tue Oct 20</timestamp>\n<user_query>\nfix it\nnow\n</user_query>";
        assert_eq!(unwrap_user_query(wrapped), "fix it\nnow");
        assert_eq!(unwrap_user_query("<user_query>open"), "open");
        assert_eq!(unwrap_user_query("plain <user_query>x</user_query>"), "plain <user_query>x</user_query>");
        assert_eq!(unwrap_user_query("<user_info>ctx</user_info>"), "<user_info>ctx</user_info>");
    }
}
