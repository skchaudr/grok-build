//! An acpx session event log (`events.ndjson` or `<id>.stream.ndjson`) turned into the events the pager's session handler consumes.

use std::fs;
use std::io;
use std::path::Path;

use agent_client_protocol as acp;
use chrono::DateTime;
use serde_json::{Map, Value, json};

use super::claude_feed::{Event, Fed, chunk};

/// `range` = inclusive `(start_seq, end_seq)` for a flow step; `None` = the whole file.
pub(crate) fn convert(
    events: &Path,
    range: Option<(u64, u64)>,
    session_id: &str,
) -> io::Result<Vec<Fed>> {
    let text = fs::read_to_string(events)?;
    Ok(convert_text(&text, range, session_id))
}

/// Convert raw NDJSON text from an acpx flow bundle `events.ndjson` or bare `<id>.stream.ndjson` into `Fed` events.
pub(crate) fn convert_text(
    text: &str,
    range: Option<(u64, u64)>,
    session_id: &str,
) -> Vec<Fed> {
    let mut state = Builder {
        sid: session_id,
        seq: 0,
        prompt_id: String::new(),
        out: Vec::new(),
    };

    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        // Detect wrapped format (`{"seq":N,"at":"...","direction":"...","message":{...}}`) vs bare format.
        let (line_seq, ts_ms, msg) = if let Some(message) = val.get("message").filter(|m| m.is_object()) {
            let s = val
                .get("seq")
                .and_then(Value::as_u64)
                .unwrap_or((idx + 1) as u64);
            let ts = val
                .get("at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);
            (s, ts, message)
        } else {
            // Bare format: skip journal header lines.
            if val.get("schema").and_then(Value::as_str) == Some("acpx.session.journal.v1")
                || val.get("type").and_then(Value::as_str) == Some("segment")
            {
                continue;
            }
            ((idx + 1) as u64, None, &val)
        };

        if let Some((start, end)) = range
            && (line_seq < start || line_seq > end)
        {
            continue;
        }

        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue;
        };
        if method.starts_with('_') {
            continue;
        }

        match method {
            "session/prompt" => {
                if let Some(id_val) = msg.get("id") {
                    state.prompt_id = render_id(id_val);
                }
                if let Some(prompts) = msg
                    .get("params")
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_array)
                {
                    for block in prompts {
                        if block.get("type").and_then(Value::as_str) == Some("text")
                            && let Some(t) = block.get("text").and_then(Value::as_str)
                        {
                            let meta = state.meta(ts_ms);
                            state.push(
                                Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(t))),
                                meta,
                            );
                        }
                    }
                }
            }
            "session/update" => {
                let Some(update_val) = msg.get("params").and_then(|p| pContent_update(p)) else {
                    continue;
                };
                let Ok(update) = serde_json::from_value::<acp::SessionUpdate>(update_val.clone())
                else {
                    continue;
                };
                if is_allowed_update(&update) {
                    let meta = state.meta(ts_ms);
                    state.push(Event::Acp(update), meta);
                }
            }
            _ => {}
        }
    }

    state.out
}

fn pContent_update(params: &Value) -> Option<&Value> {
    params.get("update")
}

fn is_allowed_update(update: &acp::SessionUpdate) -> bool {
    matches!(
        update,
        acp::SessionUpdate::AgentMessageChunk(_)
            | acp::SessionUpdate::AgentThoughtChunk(_)
            | acp::SessionUpdate::ToolCall(_)
            | acp::SessionUpdate::ToolCallUpdate(_)
            | acp::SessionUpdate::Plan(_)
    )
}

fn render_id(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

struct Builder<'a> {
    sid: &'a str,
    seq: usize,
    prompt_id: String,
    out: Vec<Fed>,
}

impl Builder<'_> {
    fn meta(&mut self, ts: Option<i64>) -> Map<String, Value> {
        self.seq += 1;
        let mut m = Map::new();
        m.insert("isReplay".into(), json!(true));
        m.insert("eventId".into(), json!(format!("{}-{}", self.sid, self.seq)));
        if !self.prompt_id.is_empty() {
            m.insert("promptId".into(), json!(self.prompt_id));
        }
        if let Some(ts) = ts {
            m.insert("agentTimestampMs".into(), json!(ts));
        }
        m
    }

    fn push(&mut self, event: Event, meta: Map<String, Value>) {
        self.out.push(Fed { event, meta });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_text(c: &acp::ContentChunk) -> Option<&str> {
        match &c.content {
            acp::ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        }
    }

    #[test]
    fn prompt_emits_user_message_chunk_and_sets_prompt_id() {
        let input = r#"{"seq": 1, "at": "2026-11-18T19:12:15.658Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/prompt", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "prompt": [{"type": "text", "text": "A directory has 3 entries."}]}}}"#;
        let feds = convert_text(input, None, "sess-1");
        assert_eq!(feds.len(), 1);
        assert!(matches!(
            &feds[0].event,
            Event::Acp(acp::SessionUpdate::UserMessageChunk(c))
                if chunk_text(c) == Some("A directory has 3 entries.")
        ));
        assert_eq!(
            feds[0].meta.get("promptId").and_then(Value::as_str),
            Some("2")
        );
    }

    #[test]
    fn agent_message_and_thought_chunks_forwarded_in_order() {
        let input = concat!(
            r#"{"seq": 1, "at": "2026-11-18T19:12:16.100Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking..."}}}}}"#,
            "\n",
            r#"{"seq": 2, "at": "2026-11-18T19:12:16.438Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "{"}, "messageId": "msg_01"}}}}"#,
            "\n",
            r#"{"seq": 3, "at": "2026-11-18T19:12:16.500Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "\"route\":\"small\"}"}}}}}"#,
        );
        let feds = convert_text(input, None, "sess-2");
        assert_eq!(feds.len(), 3);
        assert!(matches!(
            &feds[0].event,
            Event::Acp(acp::SessionUpdate::AgentThoughtChunk(c)) if chunk_text(c) == Some("thinking...")
        ));
        assert!(matches!(
            &feds[1].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("{")
        ));
        assert!(matches!(
            &feds[2].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("\"route\":\"small\"}")
        ));
    }

    #[test]
    fn tool_call_and_tool_call_update_forwarded() {
        let input = concat!(
            r#"{"seq": 28, "at": "2026-11-18T19:12:20.421Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "update": {"_meta": {"claudeCode": {"toolName": "Bash"}}, "toolCallId": "toolu_018xNPGJzRNHztQxHUoXbWPq", "sessionUpdate": "tool_call", "rawInput": {}, "status": "pending", "title": "Terminal", "kind": "execute", "content": []}}}}"#,
            "\n",
            r#"{"seq": 34, "at": "2026-11-18T19:12:22.094Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "update": {"_meta": {"claudeCode": {"toolName": "Bash"}}, "toolCallId": "toolu_018xNPGJzRNHztQxHUoXbWPq", "sessionUpdate": "tool_call_update", "status": "completed", "rawOutput": "total 16", "content": [{"type": "content", "content": {"type": "text", "text": "```console\ntotal 16"}}]}}}}"#,
        );
        let feds = convert_text(input, None, "sess-3");
        assert_eq!(feds.len(), 2);
        assert!(matches!(
            &feds[0].event,
            Event::Acp(acp::SessionUpdate::ToolCall(tc)) if tc.title == "Terminal"
        ));
        assert!(matches!(
            &feds[1].event,
            Event::Acp(acp::SessionUpdate::ToolCallUpdate(tcu))
                if tcu.fields.status == Some(acp::ToolCallStatus::Completed)
        ));
    }

    #[test]
    fn non_transcript_session_updates_are_dropped() {
        let input = concat!(
            r#"{"seq": 2, "at": "2026-11-18T19:12:15.697Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "update": {"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "deep-research", "description": "Deep research harness", "input": null}]}}}}"#,
            "\n",
            r#"{"seq": 4, "at": "2026-11-18T19:12:16.434Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "update": {"sessionUpdate": "usage_update", "used": 19398, "size": 200000}}}}"#,
            "\n",
            r#"{"seq": 16, "at": "2026-11-18T19:12:17.008Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "update": {"sessionUpdate": "session_info_update", "title": "Directory size classification", "updatedAt": "2026-11-18T19:12:16.999Z"}}}}"#,
            "\n",
            r#"{"seq": 17, "at": "2026-11-18T19:12:18.801Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": 1}}}"#,
            "\n",
            r#"{"seq": 19, "at": "2026-11-18T19:12:19.035Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 1, "method": "session/resume", "params": {"sessionId": "00000000-0000-4000-8000-000000000000", "cwd": "/tmp"}}}"#,
            "\n",
            r#"{"seq": 15, "at": "2026-11-18T19:12:17.002Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "id": 2, "result": {"stopReason": "end_turn"}}}"#,
        );
        let feds = convert_text(input, None, "sess-4");
        assert!(feds.is_empty());
    }

    #[test]
    fn underscore_methods_like_auth_status_update_are_dropped() {
        let input = concat!(
            r#"{"seq": 1, "at": "2026-11-18T19:12:15.000Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "_auth/status_update", "params": {"account": "secret@example.com"}}}"#,
            "\n",
            r#"{"seq": 2, "at": "2026-11-18T19:12:15.100Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "_custom/internal", "params": {}}}"#,
            "\n",
            r#"{"seq": 3, "at": "2026-11-18T19:12:15.200Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "visible"}}}}}"#,
        );
        let feds = convert_text(input, None, "sess-5");
        assert_eq!(feds.len(), 1);
        assert!(matches!(
            &feds[0].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("visible")
        ));
    }

    #[test]
    fn range_filters_inclusively_on_seq() {
        let input = concat!(
            r#"{"seq": 1, "at": "2026-11-18T19:12:15.000Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 1, "method": "session/prompt", "params": {"sessionId": "s", "prompt": [{"type": "text", "text": "first"}]}}}"#,
            "\n",
            r#"{"seq": 5, "at": "2026-11-18T19:12:16.000Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "second"}}}}}"#,
            "\n",
            r#"{"seq": 16, "at": "2026-11-18T19:12:17.000Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "third"}}}}}"#,
            "\n",
            r#"{"seq": 17, "at": "2026-11-18T19:12:18.000Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/prompt", "params": {"sessionId": "s", "prompt": [{"type": "text", "text": "fourth"}]}}}"#,
        );
        let feds = convert_text(input, Some((5, 16)), "sess-6");
        assert_eq!(feds.len(), 2);
        assert!(matches!(
            &feds[0].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("second")
        ));
        assert!(matches!(
            &feds[1].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("third")
        ));
    }

    #[test]
    fn bare_stream_format_skips_journal_header_and_uses_line_numbers_for_range() {
        let input = concat!(
            r#"{"schema":"acpx.session.journal.v1","type":"segment","segment":1}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":"req-9","method":"session/prompt","params":{"sessionId":"s","prompt":[{"type":"text","text":"bare prompt"}]}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"bare reply"}}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"out of range"}}}}"#,
        );
        let all = convert_text(input, None, "bare-sid");
        assert_eq!(all.len(), 3);
        assert!(matches!(
            &all[0].event,
            Event::Acp(acp::SessionUpdate::UserMessageChunk(c)) if chunk_text(c) == Some("bare prompt")
        ));
        assert!(matches!(
            &all[1].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("bare reply")
        ));
        assert_eq!(
            all[1].meta.get("promptId").and_then(Value::as_str),
            Some("req-9")
        );
        assert!(all[0].meta.get("agentTimestampMs").is_none());

        // 1-based line numbers: line 2 is prompt, line 3 is first reply, line 4 is second reply.
        let ranged = convert_text(input, Some((2, 3)), "bare-sid");
        assert_eq!(ranged.len(), 2);
    }

    #[test]
    fn meta_contains_is_replay_unique_event_ids_prompt_id_and_timestamp() {
        let input = concat!(
            r#"{"seq": 1, "at": "2026-11-18T19:12:15.658Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 7, "method": "session/prompt", "params": {"sessionId": "s", "prompt": [{"type": "text", "text": "hello"}]}}}"#,
            "\n",
            r#"{"seq": 2, "at": "2026-11-18T19:12:16.438Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "world"}}}}}"#,
        );
        let feds = convert_text(input, None, "my-session");
        assert_eq!(feds.len(), 2);

        assert_eq!(feds[0].meta.get("isReplay"), Some(&json!(true)));
        assert_eq!(feds[1].meta.get("isReplay"), Some(&json!(true)));

        assert_eq!(feds[0].meta.get("eventId"), Some(&json!("my-session-1")));
        assert_eq!(feds[1].meta.get("eventId"), Some(&json!("my-session-2")));

        assert_eq!(feds[0].meta.get("promptId"), Some(&json!("7")));
        assert_eq!(feds[1].meta.get("promptId"), Some(&json!("7")));

        let ts0 = feds[0]
            .meta
            .get("agentTimestampMs")
            .and_then(Value::as_i64);
        let ts1 = feds[1]
            .meta
            .get("agentTimestampMs")
            .and_then(Value::as_i64);
        assert!(ts0.is_some());
        assert!(ts1.is_some());
        assert_eq!(ts1.unwrap_or(0) - ts0.unwrap_or(0), 780);
    }

    #[test]
    fn unparseable_and_blank_lines_in_the_middle_are_skipped() {
        let input = concat!(
            "\n",
            r#"{"seq": 1, "at": "2026-11-18T19:12:15.000Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 1, "method": "session/prompt", "params": {"sessionId": "s", "prompt": [{"type": "text", "text": "before"}]}}}"#,
            "\n",
            "   not valid json {{{",
            "\n",
            r#"{"seq": 2, "at": "2026-11-18T19:12:15.500Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "not_a_known_variant"}}}}"#,
            "\n",
            r#"{"seq": 3, "at": "2026-11-18T19:12:16.000Z", "direction": "inbound", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "after"}}}}}"#,
            "\n",
        );
        let feds = convert_text(input, None, "sess-9");
        assert_eq!(feds.len(), 2);
        assert!(matches!(
            &feds[0].event,
            Event::Acp(acp::SessionUpdate::UserMessageChunk(c)) if chunk_text(c) == Some("before")
        ));
        assert!(matches!(
            &feds[1].event,
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) if chunk_text(c) == Some("after")
        ));
    }

    #[test]
    fn convert_reads_from_file() {
        let dir = tempfile::tempdir().ok();
        assert!(dir.is_some());
        if let Some(dir) = dir {
            let path = dir.path().join("events.ndjson");
            let content = r#"{"seq": 1, "at": "2026-11-18T19:12:15.658Z", "direction": "outbound", "message": {"jsonrpc": "2.0", "id": 1, "method": "session/prompt", "params": {"sessionId": "s", "prompt": [{"type": "text", "text": "from disk"}]}}}"#;
            assert!(fs::write(&path, content).is_ok());
            let res = convert(&path, None, "disk-sid");
            assert!(res.is_ok());
            if let Ok(feds) = res {
                assert_eq!(feds.len(), 1);
                assert!(matches!(
                    &feds[0].event,
                    Event::Acp(acp::SessionUpdate::UserMessageChunk(c)) if chunk_text(c) == Some("from disk")
                ));
            }
        }
    }
}
