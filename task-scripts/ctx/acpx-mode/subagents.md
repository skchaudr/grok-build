# Subagent (child session) emission: spawned, child updates, parent Task tool metadata, finished, replay

Notes: order for a live child is: parent ToolCall (title Task) -> ParentTaskMetadata ToolCallUpdate (raw_input variant Task) + x.ai/session_notification subagent_spawned (parent session id) -> child updates as SessionNotification with sessionId = child_session_id -> subagent_progress -> subagent_finished. Replay uses the x.ai/session/update method and _meta.isReplay=true. The pager keys children by child_session_id and expects spawned before any child update.

- operation types produced from a bridge message
```rust
// from crates/codegen/pi-grok-adapter/src/subagent_projection.rs:8-30
const BRIDGE_TYPE: &str = "pi-grok-subagent/v1";

pub(crate) enum BridgeOperation {
    ParentTaskMetadata {
        tool_call_id: String,
        raw_input: Value,
    },
    ParentLifecycle(Value),
    ChildUpdate {
        child_session_id: String,
        update: acp::SessionUpdate,
    },
    ReplayComplete {
        request_id: String,
    },
}

pub(crate) struct BridgeProjection {
    pub sequence: u64,
    pub replay: bool,
    pub kind: String,
    pub parent_session_id: String,
    pub subagent_id: String,
```

- projection of spawned, progress, finished, child_update, replay_complete
```rust
// from crates/codegen/pi-grok-adapter/src/subagent_projection.rs:67-211
pub(crate) fn parse_bridge_message(
    event: &Value,
    _root_session_id: &str,
) -> Result<Option<BridgeProjection>> {
    let Some(details) = bridge_details(event)? else {
        return Ok(None);
    };
    if details.get("version").and_then(Value::as_u64) != Some(1) {
        bail!("unsupported subagent bridge version");
    }
    let sequence = required_u64(details, "sequence")?;
    let replay = details
        .get("replay")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let parent_session_id = bridge_parent_session_id(event)?
        .expect("validated subagent bridge")
        .to_string();
    let subagent_id = required_str(details, "subagentId")?.to_string();
    let child_session_id = required_str(details, "childSessionId")?.to_string();
    let payload = details
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("subagent bridge payload must be an object"))?;
    let kind = required_str(details, "kind")?;

    let operations = match kind {
        "spawned" => {
            let parent_tool_call_id = required_str_value(payload, "parentToolCallId")?.to_string();
            let background = payload
                .get("background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let description = required_str_value(payload, "description")?;
            let subagent_type = required_str_value(payload, "subagentType")?;
            let mut update = json!({
                "sessionUpdate": "subagent_spawned",
                "subagent_id": subagent_id,
                "parent_session_id": parent_session_id,
                "child_session_id": child_session_id,
                "subagent_type": subagent_type,
                "description": description,
            });
            for (source, target) in [
                ("capabilityMode", "capability_mode"),
                ("persona", "persona"),
                ("role", "role"),
                ("model", "model"),
            ] {
                if let Some(value) = payload.get(source).filter(|value| !value.is_null()) {
                    update[target] = value.clone();
                }
            }
            let mut lifecycle =
                session_update_envelope(&parent_session_id, update, replay, &subagent_id, sequence);
            lifecycle["_meta"]["subagentBackground"] = Value::Bool(background);
            vec![
                BridgeOperation::ParentTaskMetadata {
                    tool_call_id: parent_tool_call_id,
                    raw_input: json!({
                        "variant": "Task",
                        "task_id": subagent_id,
                        "run_in_background": background,
                        "reconcile": payload.get("reconcile").and_then(Value::as_bool).unwrap_or(false),
                    }),
                },
                BridgeOperation::ParentLifecycle(lifecycle),
            ]
        }
        "progress" => {
            let update = json!({
                "sessionUpdate": "subagent_progress",
                "subagent_id": subagent_id,
                "parent_session_id": parent_session_id,
                "child_session_id": child_session_id,
                "duration_ms": required_u64_value(payload, "durationMs")?,
                "turn_count": required_u64_value(payload, "turnCount")?,
                "tool_call_count": required_u64_value(payload, "toolCallCount")?,
                "tokens_used": payload.get("tokensUsed").and_then(Value::as_u64).unwrap_or(0),
                "context_window_tokens": payload.get("contextWindowTokens").and_then(Value::as_u64).unwrap_or(0),
                "context_usage_pct": payload.get("contextUsagePct").and_then(Value::as_u64).unwrap_or(0),
                "tools_used": payload.get("toolsUsed").cloned().unwrap_or_else(|| json!([])),
                "error_count": required_u64_value(payload, "errorCount")?,
            });
            vec![BridgeOperation::ParentLifecycle(session_update_envelope(
                &parent_session_id,
                update,
                replay,
                &subagent_id,
                sequence,
            ))]
        }
        "finished" => {
            let status = required_str_value(payload, "status")?;
            if !matches!(status, "completed" | "failed" | "cancelled") {
                bail!("invalid subagent terminal status: {status}");
            }
            let mut update = json!({
                "sessionUpdate": "subagent_finished",
                "subagent_id": subagent_id,
                "child_session_id": child_session_id,
                "status": status,
                "tool_calls": required_u64_value(payload, "toolCalls")?,
                "turns": required_u64_value(payload, "turns")?,
                "duration_ms": required_u64_value(payload, "durationMs")?,
                "tokens_used": payload.get("tokensUsed").and_then(Value::as_u64).unwrap_or(0),
                "will_wake": false,
            });
            if let Some(error) = payload.get("error").filter(|value| !value.is_null()) {
                update["error"] = error.clone();
            }
            if let Some(output) = payload.get("output").filter(|value| !value.is_null()) {
                update["output"] = output.clone();
            }
            vec![BridgeOperation::ParentLifecycle(session_update_envelope(
                &parent_session_id,
                update,
                replay,
                &subagent_id,
                sequence,
            ))]
        }
        "child_update" => {
            let update = parse_child_update(payload.get("update"))?;
            vec![BridgeOperation::ChildUpdate {
                child_session_id: child_session_id.clone(),
                update,
            }]
        }
        "replay_complete" => vec![BridgeOperation::ReplayComplete {
            request_id: required_str_value(payload, "requestId")?.to_string(),
        }],
        other => bail!("unknown subagent bridge event: {other}"),
    };

    Ok(Some(BridgeProjection {
        sequence,
        replay,
        kind: kind.to_string(),
        parent_session_id,
        subagent_id,
        child_session_id,
        operations,
    }))
}
```

- child update types to ACP updates (user, assistant_delta, thinking_delta, tool_call, tool_update, tool_result)
```rust
// from crates/codegen/pi-grok-adapter/src/subagent_projection.rs:213-295
fn parse_child_update(value: Option<&Value>) -> Result<acp::SessionUpdate> {
    let update = value
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("subagent child update must be an object"))?;
    match required_str_value(update, "type")? {
        "user" => Ok(acp::SessionUpdate::UserMessageChunk(text_chunk(
            required_str_value(update, "text")?,
        ))),
        "assistant_delta" => Ok(acp::SessionUpdate::AgentMessageChunk(text_chunk(
            required_str_value(update, "text")?,
        ))),
        "thinking_delta" => Ok(acp::SessionUpdate::AgentThoughtChunk(text_chunk(
            required_str_value(update, "text")?,
        ))),
        "tool_call" => {
            let id = required_str_value(update, "toolCallId")?;
            let name = required_str_value(update, "toolName")?;
            let args = normalize_tool_raw_input(name, update.get("args").cloned());
            let content = edit_diff_content(name, args.as_ref(), None).unwrap_or_default();
            Ok(acp::SessionUpdate::ToolCall(
                acp::ToolCall::new(acp::ToolCallId::new(id.to_string()), name.to_string())
                    .kind(tool_kind(name))
                    .status(acp::ToolCallStatus::InProgress)
                    .content(content)
                    .locations(Vec::new())
                    .raw_input(args),
            ))
        }
        "tool_update" => {
            let name = required_str_value(update, "toolName")?;
            let args = normalize_tool_raw_input(name, update.get("args").cloned());
            let result = update.get("partialResult").cloned().unwrap_or(Value::Null);
            let mut fields = acp::ToolCallUpdateFields::new()
                .status(Some(acp::ToolCallStatus::InProgress))
                .raw_output(Some(normalize_tool_raw_output(
                    name,
                    args.as_ref(),
                    &result,
                    false,
                )));
            if tool_kind(name) != acp::ToolKind::Edit {
                fields = fields.content(Some(tool_content(&result)));
            }
            Ok(acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new(
                    acp::ToolCallId::new(required_str_value(update, "toolCallId")?.to_string()),
                    fields,
                ),
            ))
        }
        "tool_result" => {
            let name = required_str_value(update, "toolName")?;
            let args = normalize_tool_raw_input(name, update.get("args").cloned());
            let result = update.get("result").cloned().unwrap_or(Value::Null);
            let is_error = update.get("isError").and_then(Value::as_bool) == Some(true);
            let status = if is_error {
                acp::ToolCallStatus::Failed
            } else {
                acp::ToolCallStatus::Completed
            };
            let mut fields = acp::ToolCallUpdateFields::new()
                .status(Some(status))
                .raw_output(Some(normalize_tool_raw_output(
                    name,
                    args.as_ref(),
                    &result,
                    is_error,
                )));
            if tool_kind(name) == acp::ToolKind::Edit {
                fields = fields.content(edit_diff_content(name, args.as_ref(), Some(&result)));
            } else {
                fields = fields.content(Some(tool_content(&result)));
            }
            Ok(acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new(
                    acp::ToolCallId::new(required_str_value(update, "toolCallId")?.to_string()),
                    fields,
                ),
            ))
        }
        other => bail!("unknown child update type: {other}"),
    }
}
```

- x.ai session update envelope with isReplay and eventId, and text_chunk
```rust
// from crates/codegen/pi-grok-adapter/src/subagent_projection.rs:297-318
fn session_update_envelope(
    session_id: &str,
    update: Value,
    replay: bool,
    subagent_id: &str,
    sequence: u64,
) -> Value {
    json!({
        "sessionId": session_id,
        "update": update,
        "_meta": {
            "isReplay": replay,
            "eventId": format!("pi-grok-subagent:{subagent_id}:{sequence}"),
        },
    })
}

fn text_chunk(text: &str) -> acp::ContentChunk {
    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
        text.to_string(),
    )))
}
```

- emitting the operations: parent Task ToolCallUpdate, lifecycle ext notification, child updates
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/notifications.rs:109-195
    pub(super) async fn handle_subagent_bridge_message(&self, event: &Value) -> Result<bool> {
        let root_session_id = self.session_id().0.to_string();
        let Some(projection) = parse_bridge_message(event, &root_session_id)? else {
            return Ok(false);
        };
        let parent_session_id = projection.parent_session_id.clone();
        let parent_known = parent_session_id == root_session_id
            || self
                .state
                .borrow()
                .subagent_session_to_id
                .contains_key(&parent_session_id);
        if !parent_known {
            bail!("subagent bridge parentSessionId is not a known Pager session");
        }
        if !self.accept_subagent_bridge_sequence(
            &projection.subagent_id,
            projection.sequence,
            projection.replay,
        ) {
            return Ok(true);
        }
        if projection.kind == "spawned" {
            self.state.borrow_mut().subagent_session_to_id.insert(
                projection.child_session_id.clone(),
                projection.subagent_id.clone(),
            );
        }
        let replay = projection.replay;
        let event_id = format!(
            "pi-grok-subagent:{}:{}",
            projection.subagent_id, projection.sequence
        );
        for operation in projection.operations {
            match operation {
                BridgeOperation::ParentTaskMetadata {
                    tool_call_id,
                    raw_input,
                } => {
                    let reconcile = raw_input
                        .get("reconcile")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let mut fields = acp::ToolCallUpdateFields::new().raw_input(Some(raw_input));
                    if !replay && !reconcile {
                        fields = fields.status(Some(acp::ToolCallStatus::InProgress));
                    }
                    let update = acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        acp::ToolCallId::new(tool_call_id),
                        fields,
                    ));
                    if parent_session_id == root_session_id {
                        self.send_update(update).await;
                    } else {
                        self.send_update_for_session(&parent_session_id, update, replay, &event_id)
                            .await;
                    }
                }
                BridgeOperation::ParentLifecycle(notification) => {
                    let method = if replay {
                        "x.ai/session/update"
                    } else {
                        "x.ai/session_notification"
                    };
                    self.send_ext_notification(method, notification).await;
                }
                BridgeOperation::ChildUpdate {
                    child_session_id,
                    update,
                } => {
                    self.send_update_for_session(&child_session_id, update, replay, &event_id)
                        .await;
                }
                BridgeOperation::ReplayComplete { request_id } => {
                    if let Some(tx) = self
                        .state
                        .borrow_mut()
                        .pending_subagent_replays
                        .remove(&request_id)
                    {
                        let _ = tx.send(());
                    }
                }
            }
        }
        Ok(true)
    }
```

- sequence gating helper and child to subagent route map
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:149-169
fn accept_subagent_sequence(
    sequences: &mut HashMap<String, u64>,
    subagent_id: &str,
    sequence: u64,
    replay: bool,
) -> bool {
    let previous = sequences.get(subagent_id).copied();
    if !replay && previous.is_some_and(|last| sequence <= last) {
        return false;
    }
    sequences
        .entry(subagent_id.to_string())
        .and_modify(|last| *last = (*last).max(sequence))
        .or_insert(sequence);
    true
}

fn subagent_cancel_target(
    routes: &HashMap<String, String>,
    child_session_id: &str,
) -> Option<String> {
```

- replay_subagents: request replay, wait for replay_complete marker
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/session.rs:240-267
    pub(super) async fn replay_subagents(&self, mode: &str) -> Result<(), acp::Error> {
        self.require_bridge_command(SUBAGENT_REPLAY_COMMAND)?;
        let request_id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = oneshot::channel();
        self.state
            .borrow_mut()
            .pending_subagent_replays
            .insert(request_id.clone(), tx);
        let args = json!({ "mode": mode, "requestId": request_id.clone() }).to_string();
        if let Err(error) = self
            .run_bridge_command(SUBAGENT_REPLAY_COMMAND, &args)
            .await
        {
            self.state
                .borrow_mut()
                .pending_subagent_replays
                .remove(&request_id);
            return Err(error);
        }
        match tokio::time::timeout(SUBAGENT_REPLAY_TIMEOUT, rx).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(acp_internal(anyhow!(
                "subagent replay marker channel closed"
            ))),
            Err(_) => {
                self.state
                    .borrow_mut()
                    .pending_subagent_replays
```

- cancel aimed at a child session id
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:354-364
    async fn cancel(&self, arguments: acp::CancelNotification) -> Result<(), acp::Error> {
        let child_session_id = arguments.session_id.0.as_ref();
        let subagent_id = subagent_cancel_target(
            &self.state.borrow().subagent_session_to_id,
            child_session_id,
        );
        if let Some(subagent_id) = subagent_id {
            self.run_bridge_command(SUBAGENT_CANCEL_COMMAND, &subagent_id)
                .await?;
            return Ok(());
        }
```
