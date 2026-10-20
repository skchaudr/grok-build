//! Cursor agent transcripts: `~/.cursor/projects/<project>/agent-transcripts/<id>/<id>.jsonl`.
//!
//! Lines are `{"role":"user|assistant","message":{"content":[{"type":"text"|"tool_use",..}]}}`
//! with no top-level `type` or timestamp, plus `{"type":"turn_ended","status":..}` markers.
//! `normalize` already reads that shape; the only Cursor-specific part is that user text is
//! wrapped as `<timestamp>..</timestamp><user_query>..</user_query>`.

use std::borrow::Cow;

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
