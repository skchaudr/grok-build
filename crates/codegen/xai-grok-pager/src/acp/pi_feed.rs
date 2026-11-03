//! A Pi session file turned into the events grok's session-event handler receives, as `claude_feed` does for Claude.
//! Content only; a call's start and end come from the entries' own timestamps and `durationMs`.
//! Entries are read in file order: the active branch is a later piece.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use agent_client_protocol as acp;
use serde_json::{Map, Value, json};
use xai_grok_shell::extensions::notification::SessionUpdate as XaiSessionUpdate;

use super::claude_feed::{
    Event, Fed, chunk, edit_content, num_of, one_line, str_of, text_content, text_of, tool_call,
    ts_of,
};

/// What a tool call got back.
struct Res {
    text: String,
    is_error: bool,
    ts: Option<i64>,
    dur: Option<i64>,
    details: Value,
}

fn role(e: &Value) -> Option<&str> {
    e.pointer("/message/role").and_then(Value::as_str)
}

/// The session's events, in file order. `session_id` is the id the view is bound to.
pub(crate) fn convert(session: &Path, session_id: &str) -> io::Result<Vec<Fed>> {
    let raw = fs::read(session)?;
    let mut c = Converter {
        sid: session_id.to_string(),
        out: Vec::new(),
        seq: 0,
        prompt_id: String::new(),
        prompts: 0,
    };
    c.run(&String::from_utf8_lossy(&raw));
    Ok(c.out)
}

struct Converter {
    sid: String,
    out: Vec<Fed>,
    seq: u64,
    prompt_id: String,
    prompts: u64,
}

impl Converter {
    fn run(&mut self, text: &str) {
        let entries: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("message"))
            .collect();
        let mut results: HashMap<String, Res> = HashMap::new();
        for e in entries.iter().filter(|e| role(e) == Some("toolResult")) {
            let m = &e["message"];
            let Some(id) = m.get("toolCallId").and_then(Value::as_str) else {
                continue;
            };
            results.insert(
                id.to_string(),
                Res {
                    text: text_of(m.get("content").unwrap_or(&Value::Null)),
                    is_error: m.get("isError") == Some(&Value::Bool(true)),
                    ts: ts_of(e),
                    dur: num_of(m, "durationMs"),
                    details: m.get("details").cloned().unwrap_or(Value::Null),
                },
            );
        }
        for e in &entries {
            let ts = ts_of(e);
            let m = &e["message"];
            let content = m.get("content").unwrap_or(&Value::Null);
            match role(e) {
                Some("user") => {
                    let text = text_of(content);
                    if text.trim().is_empty() {
                        continue;
                    }
                    self.prompts += 1;
                    self.prompt_id = format!("{}-p{}", self.sid, self.prompts);
                    let mut meta = self.meta(ts, None);
                    if let Some(ts) = ts {
                        meta.insert("turnStartMs".into(), json!(ts));
                    }
                    self.push(
                        Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(&text))),
                        meta,
                    );
                }
                Some("assistant") => {
                    // The entry is stamped when the message ended.
                    let start = ts.zip(num_of(m, "durationMs")).map(|(t, d)| t - d).or(ts);
                    let Some(blocks) = content.as_array() else {
                        continue;
                    };
                    for b in blocks {
                        self.block(b, &results, ts, start);
                    }
                }
                _ => {}
            }
        }
    }

    fn block(
        &mut self,
        b: &Value,
        results: &HashMap<String, Res>,
        ts: Option<i64>,
        start: Option<i64>,
    ) {
        match b.get("type").and_then(Value::as_str) {
            Some("thinking") => {
                let body = b.get("thinking").and_then(Value::as_str).unwrap_or("");
                // Same as claude_feed: an empty thought is sent as a zero-width space so the block stays.
                let body = if body.trim().is_empty() {
                    "\u{200b}"
                } else {
                    body
                };
                let m = self.meta(ts, start);
                self.push(
                    Event::Acp(acp::SessionUpdate::AgentThoughtChunk(chunk(body))),
                    m,
                );
            }
            Some("text") => {
                let body = b.get("text").and_then(Value::as_str).unwrap_or("");
                if body.trim().is_empty() {
                    return;
                }
                let m = self.meta(ts, start);
                self.push(
                    Event::Acp(acp::SessionUpdate::AgentMessageChunk(chunk(body))),
                    m,
                );
            }
            Some("toolCall") => {
                let id = b.get("id").and_then(Value::as_str).unwrap_or("");
                let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = b.get("arguments").cloned().unwrap_or(Value::Null);
                self.tool(id, name, &args, results.get(id), ts, start);
            }
            _ => {}
        }
    }

    fn meta(&mut self, ts: Option<i64>, stream_start: Option<i64>) -> Map<String, Value> {
        self.seq += 1;
        let mut m = Map::new();
        m.insert("isReplay".into(), json!(true));
        m.insert(
            "eventId".into(),
            json!(format!("{}-{}", self.sid, self.seq)),
        );
        if !self.prompt_id.is_empty() {
            m.insert("promptId".into(), json!(self.prompt_id));
        }
        if let Some(ts) = ts {
            m.insert("agentTimestampMs".into(), json!(ts));
        }
        if let Some(s) = stream_start {
            m.insert("streamStartMs".into(), json!(s));
        }
        m
    }

    fn push(&mut self, event: Event, meta: Map<String, Value>) {
        self.out.push(Fed { event, meta });
    }

    fn tool(
        &mut self,
        id: &str,
        name: &str,
        args: &Value,
        res: Option<&Res>,
        ts: Option<i64>,
        start: Option<i64>,
    ) {
        // `subagent` also carries control calls (`action: status`, an `id`); only a launch has an agent or a task.
        if name == "subagent" && (args.get("agent").is_some() || args.get("task").is_some()) {
            return self.subagent(id, args, res, ts);
        }
        let dur = res.and_then(|r| {
            r.dur
                .or_else(|| r.ts.zip(ts).map(|(end, begin)| (end - begin).max(0)))
        });
        let status = match res {
            None => acp::ToolCallStatus::InProgress,
            Some(r) if r.is_error => acp::ToolCallStatus::Failed,
            Some(_) => acp::ToolCallStatus::Completed,
        };
        let failed = status == acp::ToolCallStatus::Failed;
        let text = res.map(|r| r.text.as_str()).unwrap_or("");
        let s = |k: &str| str_of(args, k).unwrap_or_default();
        // File text is exact: no trimming.
        let raw_s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let body = if text.is_empty() {
            Vec::new()
        } else {
            vec![text_content(text)]
        };
        let (kind, title, raw_input, content) = match name {
            "bash" => (
                acp::ToolKind::Execute,
                s("command").lines().next().unwrap_or("").to_string(),
                args.clone(),
                body,
            ),
            "read" => (
                acp::ToolKind::Read,
                format!("Read {}", s("path")),
                args.clone(),
                body,
            ),
            "find" => (
                acp::ToolKind::Search,
                format!("find {}", s("pattern")),
                args.clone(),
                body,
            ),
            // grok has no ls kind: the pager draws a list-directory row for any call with a `target_directory`.
            "ls" => (
                acp::ToolKind::Other,
                format!("ls {}", s("path")),
                json!({ "target_directory": str_of(args, "path").unwrap_or_else(|| ".".into()) }),
                body,
            ),
            "write" => {
                let path = s("path");
                let mut raw = args.clone();
                raw["variant"] = json!("Write");
                let content = edit_content(failed, text, &path, None, raw_s(args, "content"));
                (acp::ToolKind::Edit, path, raw, content)
            }
            "edit" => {
                // One Edit per replacement: grok merges adjacent edits to the same file into one row.
                let path = s("path");
                let edits = args.get("edits").and_then(Value::as_array);
                for (i, e) in edits.into_iter().flatten().enumerate() {
                    let (old, new) = (raw_s(e, "oldText"), raw_s(e, "newText"));
                    let raw = json!({ "path": path, "old_string": old, "new_string": new });
                    let tc = tool_call(
                        &format!("{id}#{i}"),
                        acp::ToolKind::Edit,
                        path.clone(),
                        raw,
                        status,
                        edit_content(failed, text, &path, Some(old), new),
                        None,
                        dur,
                    );
                    let m = self.meta(ts, start);
                    self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
                }
                return;
            }
            other => (
                acp::ToolKind::Other,
                one_line(other, args),
                args.clone(),
                body,
            ),
        };
        let tc = tool_call(id, kind, title, raw_input, status, content, None, dur);
        let m = self.meta(ts, start);
        self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
    }

    /// A subagent launch is a subagent row, not a tool row. The child's session is `runId` from the result.
    fn subagent(&mut self, call_id: &str, args: &Value, res: Option<&Res>, ts: Option<i64>) {
        let details = res.map(|r| &r.details).unwrap_or(&Value::Null);
        let child = str_of(details, "runId")
            .or_else(|| str_of(details, "asyncId"))
            .unwrap_or_else(|| call_id.to_string());
        let task = str_of(args, "task").unwrap_or_default();
        let task = task.lines().next().unwrap_or("");
        let spawned = XaiSessionUpdate::SubagentSpawned {
            subagent_id: child.clone(),
            attempt_id: None,
            parent_session_id: self.sid.clone(),
            parent_prompt_id: None,
            child_session_id: child.clone(),
            subagent_type: str_of(args, "agent").unwrap_or_else(|| "subagent".into()),
            description: if task.is_empty() {
                "subagent".into()
            } else {
                task.chars().take(100).collect()
            },
            effective_context_source: None,
            context_normalized: false,
            capability_mode: None,
            persona: None,
            role: None,
            model: None,
            resumed_from: None,
            workflow_run_id: None,
            agent_address: None,
        };
        let m = self.meta(ts, None);
        self.push(Event::Xai(spawned), m);
        // A detached launch's result is only the launch note (`asyncId`): the child is still running.
        let Some(r) = res.filter(|_| details.get("asyncId").is_none()) else {
            return;
        };
        let finished = XaiSessionUpdate::SubagentFinished {
            subagent_id: child.clone(),
            attempt_id: None,
            child_session_id: child,
            status: if r.is_error { "failed" } else { "completed" }.into(),
            error: r.is_error.then(|| r.text.clone()),
            tool_calls: 0,
            turns: 0,
            duration_ms: r.dur.unwrap_or(0).max(0) as u64,
            tokens_used: 0,
            output: (!r.is_error).then(|| r.text.clone()),
            will_wake: false,
        };
        let m = self.meta(r.ts, None);
        self.push(Event::Xai(finished), m);
    }
}

#[cfg(test)]
#[path = "pi_feed_tests.rs"]
mod tests;
