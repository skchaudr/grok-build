//! Translate claude-agent-acp 0.76.0 native subagent notifications into the
//! pager's `SubagentSpawned` / `SubagentProgress` / `SubagentFinished` events.
//!
//! The Rust ACP schema (0.10) has no `subagent_spawned` variant, so those
//! notifications would be dropped at decode. This bridge rewrites them to
//! `_x.ai/session/update` before the client parses the stream. Child tool
//! calls already arrive with the child `sessionId` and stay on `session/update`
//! so the existing child-view path streams them.
//!
//! `session/cancel` aimed at a child is a no-op in 0.76.0 (`cancel` only
//! knows root sessions). The adapter's working stop is `_session/async_task/stop`
//! with the parent session and `asyncTaskId` equal to the child session id
//! (that id is the SDK task id). `session/prompt` to the child is the draft
//! steer request; 0.76.0's `prompt()` still rejects unknown session ids.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc;

const CONTROL_TOOLS: &[&str] = &["Agent", "Task"];

#[derive(Debug)]
struct Child {
    parent_session_id: String,
    tool_calls: u32,
    turns: u32,
    tools_used: Vec<String>,
    saw_message: bool,
    started: Instant,
}

/// Connection-local map of Claude child sessions learned from `subagent_spawned`.
#[derive(Debug, Default)]
pub(crate) struct ClaudeSubagentBridge {
    children: HashMap<String, Child>,
    stop_seq: u64,
}

impl ClaudeSubagentBridge {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Inbound agent→client NDJSON line, without the trailing newline.
    /// Returns the lines to feed the ACP client, also without newlines.
    pub(crate) fn translate_inbound(&mut self, line: &str) -> Vec<String> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(mut msg) = serde_json::from_str::<Value>(line) else {
            return vec![line.to_owned()];
        };
        if msg.get("method").and_then(Value::as_str) != Some("session/update") {
            return vec![line.to_owned()];
        }
        let Some(params) = msg.get_mut("params").and_then(Value::as_object_mut) else {
            return vec![line.to_owned()];
        };
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(update) = params.get("update").cloned() else {
            return vec![line.to_owned()];
        };
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "subagent_spawned" => self.spawned(&session_id, &update),
            "subagent_state_update" => self.state_update(&session_id, &update),
            "tool_call" | "tool_call_update"
                if is_control_tool(&update) && !self.is_child(&session_id) =>
            {
                Vec::new()
            }
            "tool_call" | "tool_call_update" | "agent_message_chunk"
                if self.is_child(&session_id) =>
            {
                let mut lines = vec![line.to_owned()];
                if let Some(progress) = self.note_child_activity(&session_id, kind, &update) {
                    lines.push(progress);
                }
                lines
            }
            _ => vec![line.to_owned()],
        }
    }

    /// Outbound client→agent NDJSON line, without the trailing newline.
    pub(crate) fn translate_outbound(&mut self, line: &str) -> String {
        let line = line.trim();
        if line.is_empty() {
            return String::new();
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return line.to_owned();
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let session_id = msg
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .unwrap_or("");
        if method == "session/cancel"
            && let Some(child) = self.children.get(session_id)
        {
            self.stop_seq += 1;
            return json!({
                "jsonrpc": "2.0",
                "id": format!("grok-child-stop-{}", self.stop_seq),
                "method": "_session/async_task/stop",
                "params": {
                    "sessionId": child.parent_session_id,
                    "asyncTaskId": session_id,
                }
            })
            .to_string();
        }
        line.to_owned()
    }

    fn is_child(&self, session_id: &str) -> bool {
        self.children.contains_key(session_id)
    }

    fn spawned(&mut self, parent_session_id: &str, update: &Value) -> Vec<String> {
        let Some(child_id) = update.get("subagentSessionId").and_then(Value::as_str) else {
            return Vec::new();
        };
        let name = non_empty(update.get("name"));
        let task = non_empty(update.get("task"));
        let description = task.or(name).unwrap_or("Delegated task").to_owned();
        self.children.insert(
            child_id.to_owned(),
            Child {
                parent_session_id: parent_session_id.to_owned(),
                tool_calls: 0,
                turns: 0,
                tools_used: Vec::new(),
                saw_message: false,
                started: Instant::now(),
            },
        );
        let mut body = json!({
            "sessionUpdate": "subagent_spawned",
            "subagent_id": child_id,
            "parent_session_id": parent_session_id,
            "child_session_id": child_id,
            "subagent_type": "general-purpose",
            "description": description,
        });
        if let Some(name) = name {
            body.as_object_mut()
                .expect("spawn object")
                .insert("persona".into(), Value::String(name.to_owned()));
        }
        vec![ext_session_update(parent_session_id, body)]
    }

    fn state_update(&mut self, parent_session_id: &str, update: &Value) -> Vec<String> {
        let state = update.get("state").and_then(Value::as_str).unwrap_or("");
        match state {
            "completed" | "failed" | "cancelled" => self.finished(parent_session_id, update, state),
            "running" | "" => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn finished(&mut self, parent_session_id: &str, update: &Value, state: &str) -> Vec<String> {
        let Some(child_id) = update.get("subagentSessionId").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(child) = self.children.get(child_id) else {
            return Vec::new();
        };
        let status = finish_status(state);
        let tool_calls = child.tool_calls;
        let turns = child.turns;
        let duration_ms = child.started.elapsed().as_millis() as u64;
        let parent = child.parent_session_id.as_str();
        let line = ext_session_update(
            parent,
            json!({
                "sessionUpdate": "subagent_finished",
                "subagent_id": child_id,
                "child_session_id": child_id,
                "status": status,
                "tool_calls": tool_calls,
                "turns": turns,
                "duration_ms": duration_ms,
                "tokens_used": 0,
            }),
        );
        self.children.remove(child_id);
        vec![line]
    }

    fn note_child_activity(
        &mut self,
        child_id: &str,
        kind: &str,
        update: &Value,
    ) -> Option<String> {
        let child = self.children.get_mut(child_id)?;
        let mut changed = false;
        if kind == "tool_call" {
            child.tool_calls = child.tool_calls.saturating_add(1);
            if let Some(title) = update.get("title").and_then(Value::as_str) {
                let title = title.trim();
                if !title.is_empty() && !child.tools_used.iter().any(|t| t == title) {
                    child.tools_used.push(title.to_owned());
                }
            }
            changed = true;
        } else if kind == "agent_message_chunk" && !child.saw_message {
            child.saw_message = true;
            child.turns = child.turns.saturating_add(1);
            changed = true;
        }
        if !changed {
            return None;
        }
        let parent = child.parent_session_id.clone();
        let tool_calls = child.tool_calls;
        let turns = child.turns;
        let tools_used = child.tools_used.clone();
        let duration_ms = child.started.elapsed().as_millis() as u64;
        Some(ext_session_update(
            &parent,
            json!({
                "sessionUpdate": "subagent_progress",
                "subagent_id": child_id,
                "parent_session_id": parent,
                "child_session_id": child_id,
                "duration_ms": duration_ms,
                "turn_count": turns,
                "tool_call_count": tool_calls,
                "tokens_used": 0,
                "context_window_tokens": 0,
                "context_usage_pct": 0,
                "tools_used": tools_used,
                "error_count": 0,
            }),
        ))
    }
}

fn ext_session_update(session_id: &str, update: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": "_x.ai/session/update",
        "params": {
            "sessionId": session_id,
            "update": update,
        }
    })
    .to_string()
}

fn non_empty(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn is_control_tool(update: &Value) -> bool {
    update
        .pointer("/_meta/claudeCode/toolName")
        .and_then(Value::as_str)
        .is_some_and(|name| CONTROL_TOOLS.iter().any(|tool| *tool == name))
}

fn finish_status(state: &str) -> &'static str {
    match state {
        "completed" => "completed",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "cancelled",
    }
}

/// True when `--agent-cmd` is a Claude ACP adapter (0.76.0 or the 0.16.2 fallback).
pub(crate) fn command_is_claude_acp(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.contains("claude-agent-acp")
        || lower.contains("claude-code-acp")
        || lower.contains("claude-acp")
}

/// AIR capability object the 0.76.0 adapter reads from `clientCapabilities._meta`.
pub(crate) fn native_subagent_capability_meta() -> Value {
    json!({
        "air": {
            "version": 1,
            "capabilities": ["nativeSubagentSessions"]
        }
    })
}

pub(crate) struct ByteChanRead {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    current: Vec<u8>,
    pos: usize,
}

impl AsyncRead for ByteChanRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.pos >= this.current.len() {
            match this.rx.poll_recv(cx) {
                Poll::Ready(Some(bytes)) => {
                    this.current = bytes;
                    this.pos = 0;
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = (this.current.len() - this.pos).min(buf.remaining());
        if n == 0 {
            return Poll::Ready(Ok(()));
        }
        buf.put_slice(&this.current[this.pos..this.pos + n]);
        this.pos += n;
        Poll::Ready(Ok(()))
    }
}

pub(crate) struct LineChanWrite {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    buf: Vec<u8>,
}

impl AsyncWrite for LineChanWrite {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.buf.extend_from_slice(data);
        while let Some(i) = this.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = this.buf.drain(..=i).collect();
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if this.tx.send(line).is_err() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "claude subagent bridge closed",
                )));
            }
        }
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.buf.is_empty() {
            let line = std::mem::take(&mut this.buf);
            let _ = this.tx.send(line);
        }
        Poll::Ready(Ok(()))
    }
}

/// Split the Claude adapter's stdio so subagent lines are rewritten before the ACP client sees them.
pub(crate) fn spawn_stdio_bridge(
    stdin: ChildStdin,
    stdout: ChildStdout,
    translate: bool,
) -> (ByteChanRead, LineChanWrite) {
    let state = std::rc::Rc::new(std::cell::RefCell::new(ClaudeSubagentBridge::new()));
    let (in_tx, in_rx) = mpsc::unbounded_channel();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let inbound_state = state.clone();
    let translate_in = translate;
    tokio::task::spawn_local(async move {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let raw = line.trim_end_matches(['\n', '\r']);
                    let translated = if translate_in {
                        inbound_state.borrow_mut().translate_inbound(raw)
                    } else if raw.is_empty() {
                        Vec::new()
                    } else {
                        vec![raw.to_owned()]
                    };
                    for out in translated {
                        let mut bytes = out.into_bytes();
                        bytes.push(b'\n');
                        if in_tx.send(bytes).is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "claude subagent bridge stdout closed");
                    break;
                }
            }
        }
    });
    let outbound_state = state;
    let translate_out = translate;
    tokio::task::spawn_local(async move {
        use tokio::io::AsyncWriteExt;
        let mut writer = stdin;
        while let Some(line) = out_rx.recv().await {
            let text = String::from_utf8_lossy(&line);
            let raw = text.trim_end_matches(['\n', '\r']);
            let out = if translate_out {
                outbound_state.borrow_mut().translate_outbound(raw)
            } else {
                raw.to_owned()
            };
            if out.is_empty() {
                continue;
            }
            if writer.write_all(out.as_bytes()).await.is_err()
                || writer.write_all(b"\n").await.is_err()
            {
                break;
            }
            let _ = writer.flush().await;
        }
    });
    (
        ByteChanRead {
            rx: in_rx,
            current: Vec::new(),
            pos: 0,
        },
        LineChanWrite {
            tx: out_tx,
            buf: Vec::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARENT: &str = "12311b0f-d327-4e2b-b13f-c0fc60457f4a";
    const CHILD: &str = "a974a7cffae9d40a3";

    fn spawned_line() -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{PARENT}","update":{{"sessionUpdate":"subagent_spawned","subagentSessionId":"{CHILD}","name":"Reply with pong","task":"Reply with the single word pong and nothing else.","capabilities":{{}}}}}}}}"#
        )
    }

    fn child_message_line() -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{CHILD}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"pong"}},"_meta":{{"claudeCode":{{"parentToolUseId":"toolu_01G38uAbC4mrCM8Mg6MEse7Q"}}}},"messageId":"msg_011Cfz347PdbYFHbry4AsSVr"}}}}}}"#
        )
    }

    fn child_tool_line() -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{CHILD}","update":{{"sessionUpdate":"tool_call","toolCallId":"toolu_child_read","title":"Read pong","kind":"read","status":"pending","_meta":{{"claudeCode":{{"parentToolUseId":"toolu_01G38uAbC4mrCM8Mg6MEse7Q","toolName":"Read"}}}}}}}}}}"#
        )
    }

    fn finished_line() -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{PARENT}","update":{{"sessionUpdate":"subagent_state_update","subagentSessionId":"{CHILD}","state":"completed"}}}}}}"#
        )
    }

    fn running_state_line() -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{PARENT}","update":{{"sessionUpdate":"subagent_state_update","subagentSessionId":"{CHILD}","state":"running"}}}}}}"#
        )
    }

    fn control_tool_line() -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{PARENT}","update":{{"sessionUpdate":"tool_call_update","toolCallId":"toolu_01G38uAbC4mrCM8Mg6MEse7Q","_meta":{{"claudeCode":{{"toolName":"Agent"}}}}}}}}}}"#
        )
    }

    fn update_of(line: &str) -> Value {
        serde_json::from_str::<Value>(line).unwrap()["params"]["update"].clone()
    }

    #[test]
    fn command_match_is_claude_adapters_only() {
        assert!(command_is_claude_acp(
            "env CLAUDE_CODE_EXECUTABLE=/usr/bin/claude node /opt/claude-agent-acp/dist/index.js"
        ));
        assert!(command_is_claude_acp("claude-acp"));
        assert!(command_is_claude_acp("/home/u/.local/bin/claude-code-acp"));
        assert!(!command_is_claude_acp("pi-acp"));
        assert!(!command_is_claude_acp("cursor-agent acp"));
        assert!(!command_is_claude_acp(""));
    }

    #[test]
    fn trace_events_become_spawned_progress_finished() {
        let mut bridge = ClaudeSubagentBridge::new();
        let spawned = bridge.translate_inbound(&spawned_line());
        assert_eq!(spawned.len(), 1);
        let spawned_update = update_of(&spawned[0]);
        assert_eq!(spawned_update["sessionUpdate"], "subagent_spawned");
        assert_eq!(spawned_update["subagent_id"], CHILD);
        assert_eq!(spawned_update["child_session_id"], CHILD);
        assert_eq!(spawned_update["parent_session_id"], PARENT);
        assert_eq!(spawned_update["subagent_type"], "general-purpose");
        assert_eq!(spawned_update["persona"], "Reply with pong");
        assert_eq!(
            spawned_update["description"],
            "Reply with the single word pong and nothing else."
        );
        assert!(spawned[0].contains("\"method\":\"_x.ai/session/update\""));

        assert!(bridge.translate_inbound(&control_tool_line()).is_empty());

        let message = bridge.translate_inbound(&child_message_line());
        assert_eq!(message.len(), 2, "child text stays, plus a progress tick");
        assert!(message[0].contains("\"method\":\"session/update\""));
        assert!(message[0].contains("\"sessionId\":\"a974a7cffae9d40a3\""));
        let progress = update_of(&message[1]);
        assert_eq!(progress["sessionUpdate"], "subagent_progress");
        assert_eq!(progress["child_session_id"], CHILD);
        assert_eq!(progress["turn_count"], 1);
        assert_eq!(progress["tool_call_count"], 0);

        let tool = bridge.translate_inbound(&child_tool_line());
        assert_eq!(tool.len(), 2);
        assert!(tool[0].contains("\"toolCallId\":\"toolu_child_read\""));
        let progress = update_of(&tool[1]);
        assert_eq!(progress["sessionUpdate"], "subagent_progress");
        assert_eq!(progress["tool_call_count"], 1);
        assert_eq!(progress["tools_used"][0], "Read pong");

        let finished = bridge.translate_inbound(&finished_line());
        assert_eq!(finished.len(), 1);
        let finished_update = update_of(&finished[0]);
        assert_eq!(finished_update["sessionUpdate"], "subagent_finished");
        assert_eq!(finished_update["status"], "completed");
        assert_eq!(finished_update["tool_calls"], 1);
        assert_eq!(finished_update["turns"], 1);
        assert_eq!(finished_update["child_session_id"], CHILD);
    }

    #[test]
    fn running_state_update_does_not_finish_child() {
        let mut bridge = ClaudeSubagentBridge::new();
        bridge.translate_inbound(&spawned_line());
        assert!(bridge.translate_inbound(&running_state_line()).is_empty());

        let message = bridge.translate_inbound(&child_message_line());
        assert_eq!(message.len(), 2, "child still streams after running tick");

        let finished = bridge.translate_inbound(&finished_line());
        assert_eq!(finished.len(), 1);
        let finished_update = update_of(&finished[0]);
        assert_eq!(finished_update["sessionUpdate"], "subagent_finished");
        assert_eq!(finished_update["status"], "completed");

        assert!(
            bridge.translate_inbound(&finished_line()).is_empty(),
            "second terminal update is ignored"
        );
    }

    #[test]
    fn child_cancel_becomes_async_task_stop_and_prompt_stays() {
        let mut bridge = ClaudeSubagentBridge::new();
        bridge.translate_inbound(&spawned_line());
        let prompt = format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"session/prompt","params":{{"sessionId":"{CHILD}","prompt":[{{"type":"text","text":"also say ping"}}]}}}}"#
        );
        let steered = bridge.translate_outbound(&prompt);
        let steered: Value = serde_json::from_str(&steered).unwrap();
        assert_eq!(steered["method"], "session/prompt");
        assert_eq!(steered["params"]["sessionId"], CHILD);
        assert_eq!(steered["id"], 7);

        let cancel = format!(
            r#"{{"jsonrpc":"2.0","method":"session/cancel","params":{{"sessionId":"{CHILD}"}}}}"#
        );
        let stopped: Value = serde_json::from_str(&bridge.translate_outbound(&cancel)).unwrap();
        assert_eq!(stopped["method"], "_session/async_task/stop");
        assert_eq!(stopped["params"]["sessionId"], PARENT);
        assert_eq!(stopped["params"]["asyncTaskId"], CHILD);
        assert!(stopped.get("id").is_some());

        let parent_cancel = format!(
            r#"{{"jsonrpc":"2.0","method":"session/cancel","params":{{"sessionId":"{PARENT}"}}}}"#
        );
        let kept = bridge.translate_outbound(&parent_cancel);
        assert!(kept.contains("\"method\":\"session/cancel\""));
        assert!(kept.contains(PARENT));
    }
}
