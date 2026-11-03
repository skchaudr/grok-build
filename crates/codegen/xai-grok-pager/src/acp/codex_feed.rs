//! A Codex rollout (`~/.codex/sessions/**/rollout-*.jsonl[.zst]`) turned into the events grok's session-event handler receives.
//! Only `event_msg` `item_completed` entries are read: messages, reasoning, `CommandExecution` and `FileChange`.
//! The `exec` JavaScript wrapper (`custom_tool_call`) and the `response_item` copies of the same calls are ignored.

use std::fs;
use std::io;
use std::path::Path;

use agent_client_protocol as acp;
use serde_json::{Map, Value, json};
use xai_grok_shell::extensions::notification::SessionUpdate as XaiSessionUpdate;

use super::claude_feed::{Event, Fed, bash_raw, chunk, num_of, str_of, text_content, tool_call};

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
    for line in String::from_utf8_lossy(&raw).lines() {
        let Ok(l) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if l.get("type").and_then(Value::as_str) == Some("event_msg")
            && l.pointer("/payload/type").and_then(Value::as_str) == Some("item_completed")
            && let Some(item) = l.pointer("/payload/item")
        {
            c.item(&l, item);
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
        let end = num_of(payload, "completed_at_ms").or_else(|| {
            let s = line.get("timestamp")?.as_str()?;
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.timestamp_millis())
        });
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
            Some("CollabAgentToolCall") => self.spawn(item, end),
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

    /// A spawn call is a subagent row; the child thread's own rollout is its transcript. Other collab tools (wait, send) are skipped.
    fn spawn(&mut self, item: &Value, ts: Option<i64>) {
        let tool = item.get("tool").and_then(Value::as_str).unwrap_or("");
        if !tool.eq_ignore_ascii_case("spawnAgent") && tool != "spawn_agent" {
            return;
        }
        let prompt = str_of(item, "prompt").unwrap_or_default();
        let description = prompt.lines().next().filter(|l| !l.is_empty()).unwrap_or("subagent").to_string();
        let children: Vec<String> = item
            .get("receiver_thread_ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        for child in children {
            let spawned = XaiSessionUpdate::SubagentSpawned {
                subagent_id: child.clone(),
                attempt_id: None,
                parent_session_id: self.sid.clone(),
                parent_prompt_id: None,
                child_session_id: child,
                subagent_type: "subagent".into(),
                description: description.clone(),
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
}

#[cfg(test)]
#[path = "codex_feed_tests.rs"]
mod tests;
