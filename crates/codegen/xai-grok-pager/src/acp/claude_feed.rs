//! A Claude Code transcript turned into the events grok's session-event handler receives: session updates, plans and subagent spawns.
//! Content only. A call's start and end, and its parent, come from the transcript's own timestamps, the call ids and the sibling `subagents/` files.
//! Neither traces' normalizer nor the Claude import is used.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_client_protocol as acp;
use serde_json::{Map, Value, json};
use xai_grok_shell::extensions::notification::SessionUpdate as XaiSessionUpdate;
use xai_grok_traces::{ClaudeChild, claude_children};

/// One update for the handler. `Acp` goes in as `session/update`, `Xai` as `x.ai/session/update`.
pub(crate) enum Event {
    Acp(acp::SessionUpdate),
    Xai(XaiSessionUpdate),
}

/// An update and the `_meta` the handler reads: replay flag, event id, prompt id, timestamps.
pub(crate) struct Fed {
    pub event: Event,
    pub meta: Map<String, Value>,
}

/// What a tool call got back.
struct Res {
    text: String,
    is_error: bool,
    ts: Option<i64>,
    /// The line's `toolUseResult`: extra structure Claude keeps beside the text.
    tur: Value,
}

/// The session's events, in transcript order. `session_id` is the id the view is bound to.
pub(crate) fn convert(transcript: &Path, session_id: &str) -> io::Result<Vec<Fed>> {
    let raw = fs::read(transcript)?;
    let dir = transcript.with_extension("");
    let mut c = Converter {
        sid: session_id.to_string(),
        children: read_children(&dir),
        dir,
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
    /// `<project>/<session id>/`: `tool-results/` and `subagents/` live here.
    dir: PathBuf,
    children: HashMap<String, ClaudeChild>,
    out: Vec<Fed>,
    seq: u64,
    prompt_id: String,
    prompts: u64,
}

fn read_children(dir: &Path) -> HashMap<String, ClaudeChild> {
    claude_children(dir)
        .into_iter()
        .map(|c| (c.tool_use_id.clone(), c))
        .collect()
}

/// The last timestamp in a subagent's own transcript: when it stopped.
fn last_ts(path: &Path) -> Option<i64> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find_map(|l| ts_of(&serde_json::from_str::<Value>(l).ok()?))
}

/// What a user line really is. `None` is a harness message, not the user's words.
fn prompt_text(text: &str) -> Option<String> {
    let tag = |name: &str| {
        let body = text.split_once(&format!("<{name}>"))?.1;
        let body = body.split_once(&format!("</{name}>"))?.0.trim();
        (!body.is_empty()).then(|| body.to_string())
    };
    if let Some(cmd) = tag("command-name") {
        let cmd = if cmd.starts_with('/') {
            cmd
        } else {
            format!("/{cmd}")
        };
        return Some(match tag("command-args") {
            Some(args) => format!("{cmd} {args}"),
            None => cmd,
        });
    }
    const HARNESS: [&str; 6] = [
        "<task-notification>",
        "<command-message>",
        "<command-args>",
        "<local-command-",
        "<system-reminder>",
        "[Request interrupted by user",
    ];
    let t = text.trim_start();
    (!HARNESS.iter().any(|h| t.starts_with(h))).then(|| text.to_string())
}

pub(super) fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn num_of(v: &Value, key: &str) -> Option<i64> {
    let x = v.get(key)?;
    x.as_i64()
        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
}

fn ts_of(line: &Value) -> Option<i64> {
    let s = line.get("timestamp")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn flag(line: &Value, key: &str) -> bool {
    line.get(key) == Some(&Value::Bool(true))
}

/// Text of a message or tool-result `content`: a string, or text blocks joined by a blank line.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                Some("text") => b.get("text").and_then(Value::as_str).map(str::to_string),
                Some("image") => Some("[image]".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

pub(super) fn chunk(text: &str) -> acp::ContentChunk {
    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
        text.to_string(),
    )))
}

pub(super) fn text_content(text: &str) -> acp::ToolCallContent {
    acp::ToolCallContent::Content(acp::Content::new(acp::ContentBlock::Text(
        acp::TextContent::new(text.to_string()),
    )))
}

/// `Name: input` on one line, so a tool with no block of its own still shows what it was given.
fn one_line(name: &str, input: &Value) -> String {
    let arg = match input {
        Value::Object(o) => o
            .values()
            .find_map(|v| v.as_str().filter(|s| !s.trim().is_empty()))
            .map(str::to_string)
            .unwrap_or_else(|| input.to_string()),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let arg = arg.lines().next().unwrap_or("").trim();
    if arg.is_empty() || arg == "{}" {
        return name.to_string();
    }
    let clipped: String = arg.chars().take(100).collect();
    format!("{name}: {clipped}")
}

impl Converter {
    fn run(&mut self, text: &str) {
        let lines: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|l| {
                matches!(
                    l.get("type").and_then(Value::as_str),
                    Some("user" | "assistant")
                ) && !flag(l, "isMeta")
                    && !flag(l, "isCompactSummary")
            })
            .collect();
        let results = index_results(&lines, &self.dir);
        // An API message is written as one line per content block; the first line's time is when it began.
        let mut began: HashMap<String, i64> = HashMap::new();
        for l in &lines {
            if let (Some(id), Some(ts)) =
                (l.pointer("/message/id").and_then(Value::as_str), ts_of(l))
            {
                began.entry(id.to_string()).or_insert(ts);
            }
        }
        for l in &lines {
            let ts = ts_of(l);
            let content = l.pointer("/message/content").unwrap_or(&Value::Null);
            if l.get("type").and_then(Value::as_str) == Some("user") {
                let text = text_of(content);
                if text.trim().is_empty() {
                    continue; // a tool_result line, or an image only
                }
                // A slash command shows as "/name args"; other harness lines are not prompts and change nothing.
                let Some(text) = prompt_text(&text) else {
                    continue;
                };
                self.prompts += 1;
                self.prompt_id = str_of(l, "promptId")
                    .unwrap_or_else(|| format!("{}-p{}", self.sid, self.prompts));
                let mut m = self.meta(ts, None);
                if let Some(ts) = ts {
                    m.insert("turnStartMs".into(), json!(ts));
                }
                self.push(
                    Event::Acp(acp::SessionUpdate::UserMessageChunk(chunk(&text))),
                    m,
                );
                continue;
            }
            let start = l
                .pointer("/message/id")
                .and_then(Value::as_str)
                .and_then(|id| began.get(id))
                .copied()
                .or(ts);
            let Some(blocks) = content.as_array() else {
                continue;
            };
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("thinking") => {
                        let body = b.get("thinking").and_then(Value::as_str).unwrap_or("");
                        let dur = num_of(l, "thinkingDurationMs");
                        let m = self.meta(ts, ts.zip(dur).map(|(t, d)| t - d).or(start));
                        // shortcut: signature-only thinking is sent as one zero-width space (grok drops empty and whitespace-only thoughts) so the block stays and shows "Thought for Ns"; revisit if grok gains an empty-thought row
                        let body = if body.trim().is_empty() {
                            "\u{200b}"
                        } else {
                            body
                        };
                        self.push(
                            Event::Acp(acp::SessionUpdate::AgentThoughtChunk(chunk(body))),
                            m,
                        );
                    }
                    Some("text") => {
                        let body = b.get("text").and_then(Value::as_str).unwrap_or("");
                        if body.trim().is_empty() {
                            continue;
                        }
                        let m = self.meta(ts, start);
                        self.push(
                            Event::Acp(acp::SessionUpdate::AgentMessageChunk(chunk(body))),
                            m,
                        );
                    }
                    Some("tool_use") => {
                        let id = b.get("id").and_then(Value::as_str).unwrap_or("");
                        let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let input = b.get("input").cloned().unwrap_or(Value::Null);
                        self.tool(id, name, &input, results.get(id), ts, start);
                    }
                    _ => {}
                }
            }
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
        input: &Value,
        res: Option<&Res>,
        ts: Option<i64>,
        start: Option<i64>,
    ) {
        match name {
            "TodoWrite" => return self.plan(input, ts),
            "Task" | "Agent" => return self.subagent(id, input, res, ts),
            _ => {}
        }
        let dur = res
            .and_then(|r| r.ts)
            .zip(ts)
            .map(|(end, begin)| (end - begin).max(0));
        let status = match res {
            None => acp::ToolCallStatus::InProgress,
            Some(r) if r.is_error => acp::ToolCallStatus::Failed,
            Some(_) => acp::ToolCallStatus::Completed,
        };
        let failed = status == acp::ToolCallStatus::Failed;
        let text = res.map(|r| r.text.as_str()).unwrap_or("");
        let s = |k: &str| str_of(input, k).unwrap_or_default();
        // File text is exact: no trimming.
        let raw_s = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let text_blocks = |t: &str| {
            if t.is_empty() {
                Vec::new()
            } else {
                vec![text_content(t)]
            }
        };
        // (kind, title, raw_input, content, raw_output)
        let (kind, title, raw_input, content, raw_output): (
            acp::ToolKind,
            String,
            Value,
            Vec<acp::ToolCallContent>,
            Option<Value>,
        ) = match name {
            "Bash" => {
                let title = str_of(input, "description")
                    .unwrap_or_else(|| s("command").lines().next().unwrap_or("").to_string());
                // A failed run starts "Exit code N"; carry N so the block says so. The body is the rest.
                let code = failed
                    .then(|| {
                        text.lines()
                            .next()?
                            .strip_prefix("Exit code ")?
                            .trim()
                            .parse::<i32>()
                            .ok()
                    })
                    .flatten();
                let (body, raw) = match code {
                    Some(code) => {
                        let body = text.split_once('\n').map_or("", |(_, rest)| rest);
                        (body, Some(bash_raw(&s("command"), body, code)))
                    }
                    None => (text, None),
                };
                (
                    acp::ToolKind::Execute,
                    title,
                    input.clone(),
                    text_blocks(body),
                    raw,
                )
            }
            "Read" => {
                let file = res.map(|r| &r.tur).and_then(|t| t.get("file"));
                let body = file
                    .and_then(|f| f.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or(text);
                let total = file.and_then(|f| num_of(f, "totalLines"));
                let first = file
                    .and_then(|f| num_of(f, "startLine"))
                    .unwrap_or(1)
                    .max(1);
                let shown = file.and_then(|f| num_of(f, "numLines"));
                let range = shown
                    .filter(|n| *n > 0 && (first > 1 || total.is_some_and(|t| t > *n)))
                    .map(|n| json!({ "start": first, "end": first + n - 1 }));
                let raw = (total.is_some() || range.is_some()).then(|| {
                    let mut o = Map::new();
                    if let Some(t) = total {
                        o.insert("totalLines".into(), json!(t));
                    }
                    if let Some(r) = range {
                        o.insert("range".into(), r);
                    }
                    Value::Object(o)
                });
                // A failed read keeps its message; a good one keeps the file text without Claude's line-number gutter.
                let body = if failed { text } else { body };
                (
                    acp::ToolKind::Read,
                    format!("Read {}", s("file_path")),
                    input.clone(),
                    text_blocks(body),
                    raw,
                )
            }
            "Grep" | "Glob" => {
                let tur = res.map(|r| &r.tur).unwrap_or(&Value::Null);
                let mode = tur.get("mode").and_then(Value::as_str);
                let names: Vec<String> = res
                    .and_then(|r| r.tur.get("filenames"))
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                // Content and count modes put their result in `content`, not `filenames`.
                let raw = if failed {
                    None
                } else if let (Some(mode @ ("content" | "count")), Some(body)) =
                    (mode, tur.get("content").and_then(Value::as_str))
                {
                    grep_raw(mode == "count", body, &s("path"))
                } else {
                    (!names.is_empty())
                        .then(|| grep_json(&names.join("\n"), names.len(), json!([])))
                };
                (
                    acp::ToolKind::Search,
                    format!("{name} {}", s("pattern")),
                    input.clone(),
                    text_blocks(text),
                    raw,
                )
            }
            "Edit" | "Write" => {
                let path = s("file_path");
                // Writing over a file is a diff against what it held; only a new file is "Creating".
                let before = res
                    .map(|r| &r.tur)
                    .filter(|t| t.get("type").and_then(Value::as_str) == Some("update"))
                    .and_then(|t| t.get("originalFile")?.as_str());
                let overwrite = name == "Write" && before.is_some();
                let (old, new) = if name == "Write" {
                    (before.map(str::to_string), raw_s("content"))
                } else {
                    (Some(raw_s("old_string")), raw_s("new_string"))
                };
                let mut raw = input.clone();
                if name == "Write" && !overwrite {
                    raw["variant"] = json!("Write");
                }
                (
                    acp::ToolKind::Edit,
                    path.clone(),
                    raw,
                    edit_content(failed, text, &path, old, new),
                    None,
                )
            }
            "MultiEdit" => {
                // One Edit per replacement: grok merges adjacent edits to the same file into one row.
                let path = s("file_path");
                let edits = input
                    .get("edits")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for (i, e) in edits.iter().enumerate() {
                    let old = e
                        .get("old_string")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let new = e
                        .get("new_string")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let raw = json!({ "file_path": path, "old_string": old, "new_string": new });
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
            "WebFetch" => (
                acp::ToolKind::Fetch,
                format!("Fetch: {}", s("url")),
                input.clone(),
                text_blocks(text),
                None,
            ),
            "WebSearch" => (
                acp::ToolKind::Search,
                format!("Web search: {}", s("query")),
                input.clone(),
                text_blocks(text),
                None,
            ),
            "Skill" => (
                acp::ToolKind::Other,
                format!("Skill: {}", s("skill")),
                input.clone(),
                text_blocks(text),
                None,
            ),
            other => (
                acp::ToolKind::Other,
                one_line(other, input),
                input.clone(),
                text_blocks(text),
                None,
            ),
        };
        let tc = tool_call(id, kind, title, raw_input, status, content, raw_output, dur);
        let m = self.meta(ts, start);
        self.push(Event::Acp(acp::SessionUpdate::ToolCall(tc)), m);
    }

    fn plan(&mut self, input: &Value, ts: Option<i64>) {
        let entries: Vec<acp::PlanEntry> = input
            .get("todos")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        let status = match t.get("status").and_then(Value::as_str) {
                            Some("completed") => acp::PlanEntryStatus::Completed,
                            Some("in_progress") => acp::PlanEntryStatus::InProgress,
                            _ => acp::PlanEntryStatus::Pending,
                        };
                        Some(acp::PlanEntry::new(
                            str_of(t, "content")?,
                            acp::PlanEntryPriority::Medium,
                            status,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let m = self.meta(ts, None);
        self.push(
            Event::Acp(acp::SessionUpdate::Plan(acp::Plan::new(entries))),
            m,
        );
    }

    /// A Task/Agent call is a subagent row, not a tool row.
    fn subagent(&mut self, tool_use_id: &str, input: &Value, res: Option<&Res>, ts: Option<i64>) {
        let tur = res.map(|r| &r.tur).unwrap_or(&Value::Null);
        let linked = self.children.get(tool_use_id).cloned();
        let linked = linked.as_ref();
        let child_sid = linked
            .map(|c| c.stem.clone())
            .or_else(|| str_of(tur, "agentId").map(|a| format!("agent-{a}")))
            .unwrap_or_else(|| format!("agent-{tool_use_id}"));
        let model = linked
            .and_then(|c| c.model.clone())
            .or_else(|| str_of(tur, "resolvedModel"));
        let spawned = XaiSessionUpdate::SubagentSpawned {
            subagent_id: child_sid.clone(),
            attempt_id: None,
            parent_session_id: self.sid.clone(),
            parent_prompt_id: None,
            child_session_id: child_sid.clone(),
            subagent_type: str_of(input, "subagent_type")
                .or_else(|| linked.and_then(|c| c.agent_type.clone()))
                .unwrap_or_else(|| "general-purpose".into()),
            description: str_of(input, "description")
                .or_else(|| linked.and_then(|c| c.description.clone()))
                .unwrap_or_else(|| "subagent".into()),
            effective_context_source: None,
            context_normalized: false,
            capability_mode: None,
            persona: None,
            role: None,
            model,
            resumed_from: None,
            workflow_run_id: None,
            agent_address: None,
        };
        let m = self.meta(ts, None);
        self.push(Event::Xai(spawned), m);
        // An async launch's result is only the launch note; the sidecar's `finished` says it ended.
        let done = tur.get("status").and_then(Value::as_str) == Some("completed")
            || linked.is_some_and(|c| c.finished);
        let Some(r) = res.filter(|_| done) else {
            return;
        };
        let launched = tur.get("status").and_then(Value::as_str) == Some("async_launched");
        // An async launch's result time is only the launch; the subagent's own transcript says when it stopped.
        let end = linked
            .filter(|_| launched)
            .and_then(|c| last_ts(&self.dir.join("subagents").join(format!("{}.jsonl", c.stem))))
            .or(r.ts);
        let finished = XaiSessionUpdate::SubagentFinished {
            subagent_id: child_sid.clone(),
            attempt_id: None,
            child_session_id: child_sid,
            status: if r.is_error { "failed" } else { "completed" }.into(),
            error: r.is_error.then(|| r.text.clone()),
            tool_calls: num_of(tur, "totalToolUseCount").unwrap_or(0).max(0) as u32,
            turns: 0,
            duration_ms: num_of(tur, "totalDurationMs")
                .or_else(|| end.zip(ts).map(|(e, b)| e - b))
                .unwrap_or(0)
                .max(0) as u64,
            tokens_used: num_of(tur, "totalTokens").unwrap_or(0).max(0) as u64,
            output: (!launched && !r.is_error).then(|| r.text.clone()),
            will_wake: false,
        };
        let m = self.meta(end, None);
        self.push(Event::Xai(finished), m);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tool_call(
    id: &str,
    kind: acp::ToolKind,
    title: String,
    raw_input: Value,
    status: acp::ToolCallStatus,
    content: Vec<acp::ToolCallContent>,
    raw_output: Option<Value>,
    duration_ms: Option<i64>,
) -> acp::ToolCall {
    let meta = duration_ms.map(|d| {
        let mut m = Map::new();
        m.insert("durationMs".into(), json!(d));
        m
    });
    acp::ToolCall::new(acp::ToolCallId::new(Arc::from(id)), title)
        .kind(kind)
        .status(status)
        .content(content)
        .raw_input(Some(raw_input))
        .raw_output(raw_output)
        .locations(vec![])
        .meta(meta)
}

pub(super) fn bash_raw(command: &str, output: &str, exit_code: i32) -> Value {
    json!({ "type": "Bash", "output": output.as_bytes(), "exit_code": exit_code, "command": command, "truncated": false, "signal": null, "timed_out": false, "description": null, "current_dir": "", "output_file": "", "total_bytes": output.len() })
}

fn grep_json(stdout: &str, match_count: usize, file_matches: Value) -> Value {
    json!({ "type": "GrepSearch", "stdout": stdout.as_bytes(), "stderr": Vec::<u8>::new(), "exit_code": 0, "match_count": match_count, "file_matches": file_matches })
}

/// Claude's content mode lists `path:line:text` (`line:text` when one file is searched); count mode lists `path:n`.
/// Other lines (context, separators) are skipped.
fn grep_raw(count_mode: bool, body: &str, file: &str) -> Option<Value> {
    if count_mode {
        let rows: Vec<(&str, usize)> = body
            .lines()
            .filter_map(|l| {
                let (path, n) = l.rsplit_once(':')?;
                Some((path, n.trim().parse().ok()?))
            })
            .collect();
        let paths: Vec<&str> = rows.iter().map(|r| r.0).collect();
        let total = rows.iter().map(|r| r.1).sum();
        return (total > 0).then(|| grep_json(&paths.join("\n"), total, json!([])));
    }
    let mut files: Vec<(&str, Vec<Value>)> = Vec::new();
    let mut total = 0;
    for l in body.lines() {
        let Some((head, rest)) = l.split_once(':') else {
            continue;
        };
        let (path, n, text) = match head.parse::<usize>() {
            Ok(n) => (file, n, rest),
            Err(_) => {
                let Some((n, text)) = rest.split_once(':') else {
                    continue;
                };
                let Ok(n) = n.parse::<usize>() else { continue };
                (head, n, text)
            }
        };
        total += 1;
        let m = json!({ "line_number": n, "content": text });
        match files.last_mut() {
            Some((p, ms)) if *p == path => ms.push(m),
            _ => files.push((path, vec![m])),
        }
    }
    let fm: Vec<Value> = files
        .into_iter()
        .map(|(path, matches)| json!({ "path": path, "matches": matches }))
        .collect();
    (total > 0).then(|| grep_json("", total, Value::Array(fm)))
}

/// A good edit carries its diff; a failed one carries Claude's message.
fn edit_content(
    failed: bool,
    text: &str,
    path: &str,
    old: Option<String>,
    new: String,
) -> Vec<acp::ToolCallContent> {
    if failed {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text_content(text)]
        };
    }
    vec![acp::ToolCallContent::Diff(
        acp::Diff::new(path, new).old_text(old),
    )]
}

/// Every tool result by `tool_use_id`.
fn index_results(lines: &[Value], dir: &Path) -> HashMap<String, Res> {
    let mut out = HashMap::new();
    for l in lines {
        if l.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(blocks) = l.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for b in blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        {
            let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            out.insert(
                id.to_string(),
                Res {
                    text: unstub(&text_of(b.get("content").unwrap_or(&Value::Null)), dir),
                    is_error: flag(b, "is_error"),
                    ts: ts_of(l),
                    tur: l.get("toolUseResult").cloned().unwrap_or(Value::Null),
                },
            );
        }
    }
    out
}

/// A large output is stored as a stub naming `<session>/tool-results/<file>`. Only that directory is read, by file name.
fn unstub(text: &str, dir: &Path) -> String {
    const MARK: &str = "Full output saved to: ";
    if !text.starts_with("<persisted-output>") {
        return text.to_string();
    }
    let Some(at) = text.find(MARK) else {
        return text.to_string();
    };
    let path = text[at + MARK.len()..].lines().next().unwrap_or("").trim();
    let Some(name) = Path::new(path).file_name() else {
        return text.to_string();
    };
    fs::read_to_string(dir.join("tool-results").join(name)).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
#[path = "claude_feed_tests.rs"]
mod tests;
