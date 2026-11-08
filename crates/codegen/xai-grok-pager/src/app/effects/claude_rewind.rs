//! `/rewind` for external Claude sessions: the adapter has no rewind method, so fork the session
//! just before the chosen prompt (`session/fork` with a `messageId` fork point). Files are not restored.
use std::collections::HashMap;

use serde_json::Value;

/// A user entry that starts a turn: not a tool result, sidechain, meta/summary entry, local-command output or interrupt marker.
fn is_real_prompt(e: &Value) -> bool {
    if e["type"] != "user"
        || e["isSidechain"] == true
        || e["isMeta"] == true
        || e["isCompactSummary"] == true
    {
        return false;
    }
    let text = match &e["message"]["content"] {
        Value::String(s) => s.as_str(),
        Value::Array(parts) => match parts.iter().find(|p| p["type"] == "text") {
            Some(p) => p["text"].as_str().unwrap_or(""),
            None => return false,
        },
        _ => return false,
    };
    !text.starts_with("<local-command-") && !text.starts_with("[Request interrupted by user")
}

/// The `messageId` to fork at so the conversation continues from just before the `target`th (0-based) real user prompt.
/// Walks up from that prompt's parent to the nearest assistant (its API `message.id`) or user (its `uuid`) entry,
/// because the adapter only resolves those. `None` for the first prompt or an unresolvable chain.
pub(super) fn fork_point_message_id(transcript: &str, target: usize) -> Option<String> {
    let entries: Vec<Value> = transcript
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let by_uuid: HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|e| Some((e["uuid"].as_str()?, e)))
        .collect();
    let prompt = entries.iter().filter(|e| is_real_prompt(e)).nth(target)?;
    let mut cursor = prompt["parentUuid"].as_str()?;
    for _ in 0..entries.len() {
        let e = by_uuid.get(cursor)?;
        match e["type"].as_str() {
            Some("assistant") => {
                return Some(e["message"]["id"].as_str().unwrap_or(cursor).to_owned());
            }
            // Meta entries (local-command caveats) are not conversation turns; keep walking up.
            Some("user") if e["isMeta"] != true => return Some(cursor.to_owned()),
            _ => cursor = e["parentUuid"].as_str()?,
        }
    }
    None
}

/// Blocking; run off the UI thread.
pub(super) fn read_transcript(session_id: &str) -> Result<String, String> {
    let path = super::claude_title::find_transcript(session_id)?;
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // p1 -> a1 (two entries, same API message) -> tool_result -> a2 -> p2 (interrupt marker after) -> a3 -> p3
    // A sidechain prompt and a meta entry are interleaved and must not be counted.
    const FIXTURE: &str = r#"
{"type":"user","uuid":"p1","parentUuid":null,"isSidechain":false,"message":{"role":"user","content":"first"}}
{"type":"assistant","uuid":"a1x","parentUuid":"p1","message":{"id":"msg_1","content":[{"type":"text","text":"hi"}]}}
{"type":"assistant","uuid":"a1y","parentUuid":"a1x","message":{"id":"msg_1","content":[{"type":"tool_use","id":"t1"}]}}
{"type":"user","uuid":"tr1","parentUuid":"a1y","isSidechain":false,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}
{"type":"user","uuid":"side","parentUuid":"tr1","isSidechain":true,"message":{"role":"user","content":"sub prompt"}}
{"type":"assistant","uuid":"a2","parentUuid":"tr1","message":{"id":"msg_2","content":[{"type":"text","text":"done"}]}}
{"type":"system","uuid":"s1","parentUuid":"a2","subtype":"turn_duration"}
{"type":"user","uuid":"meta","parentUuid":"s1","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>x"}}
{"type":"user","uuid":"p2","parentUuid":"meta","isSidechain":false,"message":{"role":"user","content":[{"type":"text","text":"second"}]}}
{"type":"assistant","uuid":"a3","parentUuid":"p2","message":{"id":"msg_3","content":[]}}
{"type":"user","uuid":"intr","parentUuid":"a3","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}
{"type":"user","uuid":"p3","parentUuid":"intr","isSidechain":false,"message":{"role":"user","content":"third"}}
not json
"#;

    #[test]
    fn picks_assistant_api_id_before_second_prompt() {
        // p2's parent chain: meta -> s1 (system, skipped) -> a2 (assistant): fork at its API id
        assert_eq!(fork_point_message_id(FIXTURE, 1).as_deref(), Some("msg_2"));
    }

    #[test]
    fn user_parent_uses_uuid_and_interrupt_marker_is_not_a_prompt() {
        // third prompt is index 2: intr is skipped as a prompt but is the nearest user entry above p3
        assert_eq!(fork_point_message_id(FIXTURE, 2).as_deref(), Some("intr"));
    }

    #[test]
    fn first_prompt_and_out_of_range_have_no_fork_point() {
        assert_eq!(fork_point_message_id(FIXTURE, 0), None);
        assert_eq!(fork_point_message_id(FIXTURE, 3), None);
    }

    #[test]
    fn rejects_path_like_session_id() {
        assert!(read_transcript("../x").is_err());
        assert!(read_transcript("").is_err());
    }
}
