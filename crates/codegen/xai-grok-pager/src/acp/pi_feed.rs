//! A Pi session file turned into the events grok's session-event handler receives, as `claude_feed` does for Claude.
//! Content only; a call's start and end come from the entries' own timestamps and `durationMs`.
//! Only the active branch is read: the chain of `parentId`s from the last entry back to the root.

use std::collections::{HashMap, HashSet};
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
    /// A codemode result's `nestedCalls`, a sibling of `details`.
    nested: Value,
}

fn role(e: &Value) -> Option<&str> {
    e.pointer("/message/role").and_then(Value::as_str)
}

fn type_of(e: &Value) -> Option<&str> {
    e.get("type").and_then(Value::as_str)
}

/// The entries on the chain from the last entry back to the root, oldest first.
/// A file whose entries carry no ids (older Pi) is one straight line: all of it.
fn active_branch(all: Vec<Value>) -> Vec<Value> {
    let id = |e: &Value| e.get("id").and_then(Value::as_str).map(str::to_string);
    let Some(last) = all.iter().rposition(|e| id(e).is_some()) else {
        return all;
    };
    let index: HashMap<String, usize> = all
        .iter()
        .enumerate()
        .filter_map(|(i, e)| id(e).map(|id| (id, i)))
        .collect();
    let (mut chain, mut seen, mut cur) = (vec![last], HashSet::from([last]), last);
    // `seen` stops a parent loop in a damaged file.
    while let Some(p) = all[cur]
        .get("parentId")
        .and_then(Value::as_str)
        .and_then(|p| index.get(p).copied())
        .filter(|p| seen.insert(*p))
    {
        chain.push(p);
        cur = p;
    }
    chain.into_iter().rev().map(|i| all[i].clone()).collect()
}

/// The session's events, in file order. `session_id` is the id the view is bound to.
pub(crate) fn convert(session: &Path, session_id: &str) -> io::Result<Vec<Fed>> {
    let raw = fs::read(session)?;
    Ok(convert_text(&String::from_utf8_lossy(&raw), session_id))
}

/// A dpi conversation (`<agent.sqlite>#<conversation id>`): each entry's `model[0]` is a Pi message, so the rows
/// become Pi `message` lines and go through the same converter.
pub(crate) fn convert_dpi(key: &str, session_id: &str) -> io::Result<Vec<Fed>> {
    let bad = |e: String| io::Error::new(io::ErrorKind::InvalidData, e);
    let (db, conv) = xai_grok_traces::dpi::split_key(key).ok_or_else(|| bad(format!("not a dpi key: {key}")))?;
    let records = xai_grok_traces::dpi::DpiDb::open(Path::new(db))
        .and_then(|d| d.records(conv))
        .map_err(|e| bad(e.to_string()))?;
    Ok(convert_records(&records, session_id))
}

fn convert_records(records: &[Value], session_id: &str) -> Vec<Fed> {
    let lines: Vec<String> = records
        .iter()
        .filter_map(|r| r.pointer("/model/0").filter(|m| m.is_object()))
        .map(|m| {
            let ts = m
                .get("timestamp")
                .and_then(Value::as_i64)
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
            json!({ "type": "message", "timestamp": ts, "message": m }).to_string()
        })
        .collect();
    convert_text(&lines.join("\n"), session_id)
}

fn convert_text(text: &str, session_id: &str) -> Vec<Fed> {
    let mut c = Converter {
        sid: session_id.to_string(),
        out: Vec::new(),
        seq: 0,
        prompt_id: String::new(),
        prompts: 0,
        parent: None,
    };
    c.run(text);
    c.out
}

struct Converter {
    sid: String,
    out: Vec<Fed>,
    seq: u64,
    prompt_id: String,
    prompts: u64,
    /// The codemode call whose nested calls are being emitted.
    parent: Option<String>,
}

impl Converter {
    fn run(&mut self, text: &str) {
        let all: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .collect();
        let entries: Vec<Value> = active_branch(all)
            .into_iter()
            .filter(|e| {
                matches!(
                    type_of(e),
                    Some("message" | "compaction" | "custom_message")
                )
            })
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
                    nested: m.get("nestedCalls").cloned().unwrap_or(Value::Null),
                },
            );
        }
        for e in &entries {
            let ts = ts_of(e);
            if type_of(e) == Some("compaction") {
                self.compaction(e, ts);
                continue;
            }
            // A lane prompt is a `custom_message` entry: its content goes to the model as a user turn.
            if type_of(e) == Some("custom_message") {
                self.user(&text_of(e.get("content").unwrap_or(&Value::Null)), ts);
                continue;
            }
            let m = &e["message"];
            let content = m.get("content").unwrap_or(&Value::Null);
            match role(e) {
                Some("user") => self.user(&text_of(content), ts),
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

    fn user(&mut self, text: &str, ts: Option<i64>) {
        if text.trim().is_empty() {
            return;
        }
        self.prompts += 1;
        self.prompt_id = format!("{}-p{}", self.sid, self.prompts);
        let mut meta = self.meta(ts, None);
        if let Some(ts) = ts {
            meta.insert("turnStartMs".into(), json!(ts));
        }
        self.push(
            Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(text))),
            meta,
        );
    }

    /// grok nests a call under its parent by `_meta.pi.parentToolCallId`.
    fn nest(&self, mut tc: acp::ToolCall) -> acp::ToolCall {
        if let Some(p) = &self.parent {
            let mut m = tc.meta.take().unwrap_or_default();
            m.insert("pi".into(), json!({ "parentToolCallId": p }));
            tc.meta = Some(m);
        }
        tc
    }

    /// A compaction entry is grok's compaction row. Pi records only `tokensBefore`, so the
    /// "after" count is the summary's size at Pi's own four-characters-a-token estimate.
    fn compaction(&mut self, e: &Value, ts: Option<i64>) {
        let summary = str_of(e, "summary").unwrap_or_default();
        let done = XaiSessionUpdate::AutoCompactCompleted {
            tokens_before: e.get("tokensBefore").and_then(Value::as_u64),
            tokens_after: summary.chars().count() as u64 / 4,
            elapsed_ms: None,
            summary_preview: Some(summary.chars().take(100).collect()),
        };
        let m = self.meta(ts, None);
        self.push(Event::Xai(done), m);
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
                    let tc = self.nest(tc);
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
        let tc = self.nest(tc);
        let m = self.meta(ts, start);
        self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
        if name == "codemode" && self.parent.is_none() {
            self.nested(id, res, ts, start);
        }
    }

    /// A codemode result lists its inner calls (`nestedCalls` with `arguments`, else `details.calls` with `args` as JSON text).
    fn nested(&mut self, parent: &str, res: Option<&Res>, ts: Option<i64>, start: Option<i64>) {
        let Some(r) = res else { return };
        fn list(v: &Value) -> Option<&Vec<Value>> {
            v.get("calls").and_then(Value::as_array)
        }
        let calls = list(&r.nested).or_else(|| list(&r.details));
        self.parent = Some(parent.to_string());
        for c in calls.into_iter().flatten() {
            let (Some(id), Some(name)) = (str_of(c, "id"), str_of(c, "name")) else {
                continue;
            };
            let args = c.get("arguments").cloned().unwrap_or_else(|| {
                c.get("args")
                    .and_then(Value::as_str)
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or(Value::Null)
            });
            let inner = Res {
                text: String::new(),
                is_error: str_of(c, "status").is_some_and(|s| s != "ok"),
                ts: None,
                dur: c
                    .get("durationMs")
                    .and_then(Value::as_f64)
                    .map(|d| d.round() as i64),
                details: Value::Null,
                nested: Value::Null,
            };
            self.tool(&id, &name, &args, Some(&inner), ts, start);
        }
        self.parent = None;
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
