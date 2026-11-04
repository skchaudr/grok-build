//! A Codex rollout (`~/.codex/sessions/**/rollout-*.jsonl[.zst]`) turned into the events grok's session-event handler receives.
//! `event_msg` `item_completed` entries give messages, reasoning, `CommandExecution`, `FileChange`, `McpToolCall` and `WebSearch`.
//! Spawns are only in `response_item`: a `spawn_agent` `function_call` and its `function_call_output` (`{"agent_id": ...}`).
//! The `exec` JavaScript wrapper (`custom_tool_call`) and the other `response_item` copies of the same calls are ignored.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use agent_client_protocol as acp;
use serde_json::{Map, Value, json};
use xai_grok_shell::extensions::notification::SessionUpdate as XaiSessionUpdate;

use super::claude_feed::{Event, Fed, bash_raw, chunk, num_of, one_line, str_of, text_content, text_of, tool_call};

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// The session's events, in rollout order. `session_id` is the id the view is bound to.
pub(crate) fn convert(rollout: &Path, session_id: &str) -> io::Result<Vec<Fed>> {
    let raw = fs::read(rollout)?;
    let raw = if raw.starts_with(&ZSTD_MAGIC) {
        zstd::decode_all(&raw[..])?
    } else {
        raw
    };
    let mut c = Converter {
        sid: session_id.to_string(),
        out: Vec::new(),
        seq: 0,
        prompt_id: String::new(),
        prompts: 0,
    };
    // `spawn_agent` calls waiting for their output, by call id.
    let mut spawns: HashMap<String, Value> = HashMap::new();
    for line in String::from_utf8_lossy(&raw).lines() {
        let Ok(l) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = |k: &str| l.pointer(k).and_then(Value::as_str);
        match (at("/type"), at("/payload/type")) {
            (Some("event_msg"), Some("item_completed")) => {
                if let Some(item) = l.pointer("/payload/item") {
                    c.item(&l, item);
                }
            }
            (Some("response_item"), Some("function_call")) if at("/payload/name") == Some("spawn_agent") => {
                if let Some(id) = at("/payload/call_id") {
                    spawns.insert(id.to_string(), l.clone());
                }
            }
            (Some("response_item"), Some("function_call_output")) => {
                if let Some(call) = at("/payload/call_id").and_then(|id| spawns.remove(id)) {
                    c.spawn(&call, &l);
                }
            }
            _ => {}
        }
    }
    Ok(c.out)
}

struct Converter {
    sid: String,
    out: Vec<Fed>,
    seq: u64,
    prompt_id: String,
    prompts: u64,
}

/// What a user message really is. `None` is a harness message (Codex's auto-review of a planned action, injected context), not the user's words.
fn user_text(item: &Value) -> Option<String> {
    let text = item
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|b| match b.get("type").and_then(Value::as_str) {
            Some("text") => b.get("text").and_then(Value::as_str).map(str::to_string),
            Some("image" | "local_image") => Some("[image]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    const HARNESS: [&str; 4] = [
        "<environment_context>",
        "<user_instructions>",
        "<turn_aborted>",
        "The following is the Codex agent history",
    ];
    let t = text.trim_start();
    (!t.is_empty() && !HARNESS.iter().any(|h| t.starts_with(h))).then_some(text)
}

fn line_ts(line: &Value) -> Option<i64> {
    let s = line.get("timestamp")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

fn duration_ms(item: &Value) -> Option<i64> {
    let d = item.get("duration")?;
    Some(num_of(d, "secs")? * 1000 + num_of(d, "nanos").unwrap_or(0) / 1_000_000)
}

/// The shell script of a `["/bin/zsh", "-lc", script]` argv; other argvs joined.
fn script_of(item: &Value) -> String {
    let argv: Vec<&str> = item
        .get("command")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match argv.as_slice() {
        [_, "-lc" | "-c", script] => (*script).to_string(),
        other => other.join(" "),
    }
}

/// A unified diff as (old, new) text per hunk. Context lines are in both. Line numbers are not kept.
fn hunks(diff: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in diff.lines() {
        if l.starts_with("@@") {
            out.push((String::new(), String::new()));
            continue;
        }
        let Some((old, new)) = out.last_mut() else {
            continue; // ---/+++ headers before the first hunk
        };
        match l.split_at_checked(1) {
            Some(("-", rest)) => {
                old.push_str(rest);
                old.push('\n');
            }
            Some(("+", rest)) => {
                new.push_str(rest);
                new.push('\n');
            }
            Some(("\\", _)) => {} // "\ No newline at end of file"
            rest => {
                let rest = rest.map_or("", |r| r.1);
                for s in [&mut *old, &mut *new] {
                    s.push_str(rest);
                    s.push('\n');
                }
            }
        }
    }
    out
}

impl Converter {
    fn item(&mut self, line: &Value, item: &Value) {
        // The times sit beside `item`, not in it.
        let payload = line.get("payload").unwrap_or(&Value::Null);
        let end = num_of(payload, "completed_at_ms").or_else(|| line_ts(line));
        let began = num_of(payload, "started_at_ms");
        match item.get("type").and_then(Value::as_str) {
            Some("UserMessage") => {
                let Some(text) = user_text(item) else { return };
                self.prompts += 1;
                self.prompt_id = line
                    .pointer("/payload/turn_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{}-p{}", self.sid, self.prompts));
                let mut m = self.meta(end, None);
                if let Some(ts) = end {
                    m.insert("turnStartMs".into(), json!(ts));
                }
                self.push(
                    Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(&text))),
                    m,
                );
            }
            Some("AgentMessage") => {
                let text = item
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|b| b.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                if text.trim().is_empty() {
                    return;
                }
                let m = self.meta(end, None);
                self.push(
                    Event::Acp(acp::SessionUpdate::AgentMessageChunk(chunk(&text))),
                    m,
                );
            }
            Some("Reasoning") => {
                // The text is encrypted; the summary is usually empty. An empty thought is sent as one zero-width space
                // (grok drops empty and whitespace-only thoughts) so the block stays and shows "Thought for Ns".
                let summary = item
                    .get("summary_text")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().or_else(|| s.get("text")?.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                let body = if summary.trim().is_empty() {
                    "\u{200b}"
                } else {
                    &summary
                };
                let m = self.meta(end, began);
                self.push(
                    Event::Acp(acp::SessionUpdate::AgentThoughtChunk(chunk(body))),
                    m,
                );
            }
            Some("CommandExecution") => self.command(item, end, began),
            Some("FileChange") => self.file_change(item, end, began),
            Some("McpToolCall") => self.mcp(item, end, began),
            Some("WebSearch") => {
                let id = str_of(item, "id").unwrap_or_else(|| format!("{}-ws{}", self.sid, self.seq + 1));
                let query = str_of(item, "query").unwrap_or_default();
                let tc = tool_call(
                    &id,
                    acp::ToolKind::Search,
                    format!("Web search: {query}"),
                    json!({ "query": query }),
                    acp::ToolCallStatus::Completed,
                    Vec::new(),
                    None,
                    None,
                );
                let m = self.meta(end, end);
                self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
            }
            _ => {}
        }
    }

    fn command(&mut self, item: &Value, end: Option<i64>, began: Option<i64>) {
        let id = str_of(item, "id").unwrap_or_else(|| format!("{}-cmd{}", self.sid, self.seq + 1));
        let script = script_of(item);
        let code = num_of(item, "exit_code").map(|c| c as i32);
        let failed = item.get("status").and_then(Value::as_str) == Some("failed")
            || code.is_some_and(|c| c != 0);
        let output = ["aggregated_output", "formatted_output"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .unwrap_or("");
        let dur = duration_ms(item).or_else(|| end.zip(began).map(|(e, b)| (e - b).max(0)));
        let start = began.or_else(|| end.zip(dur).map(|(e, d)| e - d));
        let cwd = str_of(item, "cwd").map(|c| c.strip_prefix("file://").unwrap_or(&c).to_string());
        let tc = tool_call(
            &id,
            acp::ToolKind::Execute,
            script.lines().next().unwrap_or("").to_string(),
            json!({ "command": script, "workdir": cwd }),
            if failed {
                acp::ToolCallStatus::Failed
            } else {
                acp::ToolCallStatus::Completed
            },
            if output.is_empty() {
                Vec::new()
            } else {
                vec![text_content(output)]
            },
            code.filter(|_| failed)
                .map(|c| bash_raw(&script, output, c)),
            dur,
        );
        let m = self.meta(start, start);
        self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
    }

    /// One Edit per file hunk (grok merges adjacent edits to the same file into one row); a new file is a Write, a deleted one a diff to empty.
    fn file_change(&mut self, item: &Value, end: Option<i64>, began: Option<i64>) {
        let id = str_of(item, "id").unwrap_or_else(|| format!("{}-fc{}", self.sid, self.seq + 1));
        let failed = item.get("status").and_then(Value::as_str) == Some("failed");
        let dur = end.zip(began).map(|(e, b)| (e - b).max(0));
        let start = began.or(end);
        let status = if failed {
            acp::ToolCallStatus::Failed
        } else {
            acp::ToolCallStatus::Completed
        };
        let err = str_of(item, "stderr").unwrap_or_default();
        let Some(changes) = item.get("changes").and_then(Value::as_object) else {
            return;
        };
        let mut n = 0;
        for (path, ch) in changes {
            let text = |k: &str| ch.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            // (old, new) pairs and whether the file is new
            let (edits, write): (Vec<(Option<String>, String)>, bool) =
                match ch.get("type").and_then(Value::as_str) {
                    Some("add") => (vec![(None, text("content"))], true),
                    Some("delete") => (vec![(Some(text("content")), String::new())], false),
                    _ => (
                        hunks(&text("unified_diff"))
                            .into_iter()
                            .map(|(o, nw)| (Some(o), nw))
                            .collect(),
                        false,
                    ),
                };
            // A failed change keeps its message once per file, not once per hunk.
            let edits = if failed || edits.is_empty() {
                vec![(None, String::new())]
            } else {
                edits
            };
            for (old, new) in edits {
                let raw = if write {
                    json!({ "file_path": path, "content": new, "variant": "Write" })
                } else {
                    json!({ "file_path": path, "old_string": old, "new_string": new })
                };
                let content = if failed {
                    if err.is_empty() {
                        Vec::new()
                    } else {
                        vec![text_content(&err)]
                    }
                } else {
                    vec![acp::ToolCallContent::Diff(
                        acp::Diff::new(path, new).old_text(old),
                    )]
                };
                let tc = tool_call(
                    &format!("{id}#{n}"),
                    acp::ToolKind::Edit,
                    path.clone(),
                    raw,
                    status,
                    content,
                    None,
                    dur,
                );
                n += 1;
                let m = self.meta(start, start);
                self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
            }
        }
    }

    /// An MCP call is an Other row with its `server.tool` name, arguments and result text.
    fn mcp(&mut self, item: &Value, end: Option<i64>, began: Option<i64>) {
        let id = str_of(item, "id").unwrap_or_else(|| format!("{}-mcp{}", self.sid, self.seq + 1));
        let name = format!(
            "{}.{}",
            str_of(item, "server").unwrap_or_default(),
            str_of(item, "tool").unwrap_or_default()
        );
        let args = item.get("arguments").cloned().unwrap_or(Value::Null);
        let failed = item.get("status").and_then(Value::as_str) == Some("failed")
            || item.pointer("/result/isError") == Some(&Value::Bool(true));
        let text = item.pointer("/result/content").map(text_of).unwrap_or_default();
        let dur = duration_ms(item).or_else(|| end.zip(began).map(|(e, b)| (e - b).max(0)));
        let start = began.or_else(|| end.zip(dur).map(|(e, d)| e - d));
        let tc = tool_call(
            &id,
            acp::ToolKind::Other,
            one_line(&name, &args),
            args,
            if failed {
                acp::ToolCallStatus::Failed
            } else {
                acp::ToolCallStatus::Completed
            },
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text_content(&text)]
            },
            None,
            dur,
        );
        let m = self.meta(start, start);
        self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
    }

    /// A `spawn_agent` call is a subagent row; the child thread is the `agent_id` in its output, and its own rollout is its transcript.
    fn spawn(&mut self, call: &Value, out: &Value) {
        let output = out.pointer("/payload/output");
        let parsed = output
            .and_then(Value::as_str)
            .and_then(|o| serde_json::from_str::<Value>(o).ok());
        let Some(child) = parsed.as_ref().or(output).and_then(|o| str_of(o, "agent_id")) else {
            return; // a failed spawn has no child
        };
        let args = call
            .pointer("/payload/arguments")
            .and_then(Value::as_str)
            .and_then(|a| serde_json::from_str::<Value>(a).ok())
            .unwrap_or(Value::Null);
        let message = str_of(&args, "message").unwrap_or_default();
        let description = message.lines().next().filter(|l| !l.is_empty()).unwrap_or("subagent").to_string();
        let spawned = XaiSessionUpdate::SubagentSpawned {
            subagent_id: child.clone(),
            attempt_id: None,
            parent_session_id: self.sid.clone(),
            parent_prompt_id: None,
            child_session_id: child,
            subagent_type: str_of(&args, "agent_type").unwrap_or_else(|| "subagent".into()),
            description,
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
        let m = self.meta(line_ts(call), None);
        self.push(Event::Xai(spawned), m);
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
}

#[cfg(test)]
#[path = "codex_feed_tests.rs"]
mod tests;
