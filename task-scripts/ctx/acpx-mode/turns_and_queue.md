# Turns, prompt completion, adapter-initiated turns (queue/changed + prompt_complete), cancel

Notes: prompt() registers an ActivePrompt with a oneshot and awaits it; agent_start/agent_settled events resolve it via finish_prompts. A turn the pager did not start is announced by x.ai/queue/changed {runningPromptId, runningText, runningKind} (the pager adopts it as a viewer, _meta.promptId on every update must equal that id) and closed by x.ai/session/prompt_complete {sessionId, promptId, stopReason}. Client-origin prompts get their completion from the PromptResponse (with _meta.promptId) instead.

- ActivePrompt and PromptCompletion
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:126-139
struct ActivePrompt {
    id: u64,
    /// Pager-minted id (`_meta.promptId`); echoed on PromptResponse so non-running
    /// mid-turn completions are discarded instead of emitting phantom turns.
    client_prompt_id: Option<String>,
    completion: oneshot::Sender<PromptCompletion>,
    agent_started: bool,
    cancelled: bool,
}

struct PromptCompletion {
    reason: acp::StopReason,
    client_prompt_id: Option<String>,
}
```

- stop reason to wire string
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:173-178
fn stop_reason_wire(reason: &acp::StopReason) -> &'static str {
    match reason {
        acp::StopReason::Cancelled => "cancelled",
        _ => "end_turn",
    }
}
```

- prompt(): register active prompt, send to backend, hold response until completion
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:171-352
    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        if let Some(command) = direct_bash_command(&arguments.prompt) {
            return self.execute_bash(command, arguments.meta.as_ref()).await;
        }

        let (message, images) = prompt_to_pi(&arguments.prompt);
        let display_message = message.clone();
        if message.trim().is_empty() && images.is_empty() {
            return Err(acp::Error::invalid_params().data("Pi prompt is empty"));
        }
        let client_prompt_id = arguments
            .meta
            .as_ref()
            .and_then(|meta| meta.get("promptId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let plan_file_to_seed = {
            let state = self.state.borrow();
            (state.plan_mode.state() == crate::plan_mode::PiPlanState::Pending)
                .then(|| state.plan_mode.plan_file_path().to_path_buf())
        };
        if let Some(plan_file) = plan_file_to_seed {
            ensure_plan_file(&plan_file).map_err(acp_internal)?;
        }

        enum Disposition {
            Queued {
                id: String,
            },
            Direct {
                operation_id: u64,
                message: String,
                streaming_behavior: Option<&'static str>,
                pin_primary_running: bool,
            },
        }

        let (completion_tx, completion_rx) = oneshot::channel();
        let disposition = {
            let mut state = self.state.borrow_mut();
            let streaming_behavior =
                prompt_streaming_behavior(Self::adapter_busy(&state), arguments.meta.as_ref());
            // Steer requests are held locally too (lane is the only
            // difference): the row stays editable/cancellable until the
            // assistant message_end safe-point flush forwards it to Pi.
            if let Some(lane) = streaming_behavior.map(|behavior| {
                if behavior == "steer" {
                    QueueLane::Steering
                } else {
                    QueueLane::FollowUp
                }
            }) {
                let id = state.queue_mirror.enqueue_local(
                    client_prompt_id.clone(),
                    message,
                    display_message,
                    images.clone(),
                    lane,
                    QueueOrigin::Client,
                );
                state
                    .queued_prompt_completions
                    .insert(id.clone(), completion_tx);
                Disposition::Queued { id }
            } else {
                let execution_message = Self::prepare_execution_text(&mut state, message);
                let operation_id = state.next_prompt_id;
                state.next_prompt_id = state.next_prompt_id.wrapping_add(1).max(1);
                if let Some(lane) = streaming_behavior.and_then(queue_lane_for_behavior)
                    && let Some(client_id) = client_prompt_id.as_deref()
                {
                    state.queue_mirror.reserve(
                        client_id.to_string(),
                        execution_message.clone(),
                        display_message.clone(),
                        images.clone(),
                        lane,
                        QueueOrigin::Client,
                    );
                }
                let pin_primary_running = streaming_behavior.is_none()
                    && client_prompt_id.as_deref().is_some_and(|id| !id.is_empty());
                if pin_primary_running {
                    let client_id = client_prompt_id.clone().expect("checked non-empty");
                    state.live_prompt_id = Some(client_id.clone());
                    state.queue_mirror.set_running_primary(
                        client_id,
                        execution_message.clone(),
                        display_message,
                        images.clone(),
                        QueueOrigin::Client,
                    );
                }
                state.agent_running = true;
                state.active_prompts.push(ActivePrompt {
                    id: operation_id,
                    client_prompt_id: client_prompt_id.clone(),
                    completion: completion_tx,
                    agent_started: false,
                    cancelled: false,
                });
                Disposition::Direct {
                    operation_id,
                    message: execution_message,
                    streaming_behavior,
                    pin_primary_running,
                }
            }
        };

        self.persist_plan_mode_state().map_err(acp_internal)?;
        self.sync_plan_mode_control().map_err(acp_internal)?;

        match disposition {
            Disposition::Queued { id } => {
                self.publish_queue_snapshot().await;
                self.dispatch_next_queued().await;
                let completion = completion_rx.await.unwrap_or(PromptCompletion {
                    reason: acp::StopReason::Cancelled,
                    client_prompt_id: Some(id.clone()),
                });
                Ok(prompt_response(
                    completion.reason,
                    completion.client_prompt_id.as_deref().or(Some(id.as_str())),
                ))
            }
            Disposition::Direct {
                operation_id,
                message,
                streaming_behavior,
                pin_primary_running,
            } => {
                let mut request = json!({ "type": "prompt", "message": message });
                if !images.is_empty() {
                    request["images"] = Value::Array(images);
                }
                if let Some(streaming_behavior) = streaming_behavior {
                    request["streamingBehavior"] = Value::String(streaming_behavior.to_string());
                }
                if let Err(error) = self.rpc.request(request).await {
                    if let Some(client_id) = client_prompt_id.as_deref() {
                        self.state
                            .borrow_mut()
                            .queue_mirror
                            .release_reservation(client_id);
                    }
                    self.remove_prompt(operation_id);
                    if pin_primary_running {
                        let mut state = self.state.borrow_mut();
                        state.agent_running = false;
                        state.live_prompt_id = None;
                        state.queue_mirror.clear_running();
                    }
                    self.publish_queue_snapshot().await;
                    return Err(acp_internal(error));
                }
                if pin_primary_running {
                    self.rebroadcast_queue_mirror().await;
                }
                let probe = self.clone();
                tokio::task::spawn_local(async move {
                    probe.probe_prompt_without_agent().await;
                });
                let completion = completion_rx.await.unwrap_or(PromptCompletion {
                    reason: acp::StopReason::Cancelled,
                    client_prompt_id: client_prompt_id.clone(),
                });
                Ok(prompt_response(
                    completion.reason,
                    completion
                        .client_prompt_id
                        .as_deref()
                        .or(client_prompt_id.as_deref()),
                ))
            }
        }
    }
```

- cancel(): mark prompts cancelled, finish them synchronously, abort without awaiting
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:354-403
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

        let (command, queued, running) = {
            let mut state = self.state.borrow_mut();
            for active in &mut state.active_prompts {
                active.cancelled = true;
            }
            state.cancelling = true;
            let command = if state.bash_running {
                "abort_bash"
            } else {
                "abort"
            };
            let queued = state.queue_mirror.clear_local();
            let running = state.queue_mirror.clear_running();
            state.queue_mirror.clear();
            (command, queued, running)
        };

        self.finish_queued_entries(queued, acp::StopReason::Cancelled);
        self.finish_prompts(acp::StopReason::Cancelled);
        if let Some(entry) = running
            && entry.origin != QueueOrigin::Client
        {
            self.send_server_prompt_complete(&entry, acp::StopReason::Cancelled)
                .await;
        }
        self.publish_queue_snapshot().await;

        // Do not await Pi's abort RPC. AgentSession.abort() waits for idle, and
        // extension continuations can otherwise keep ACP cancel blocked forever.
        if let Err(error) = self.rpc.notify(json!({ "type": command })) {
            tracing::warn!(%error, "failed to notify Pi abort");
        }
        let probe = self.clone();
        tokio::task::spawn_local(async move {
            probe.settle_cancelled_prompts().await;
        });
        Ok(())
    }
```

- agent_start and agent_settled: claim running slot, prompt_complete for non-client origin
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/events.rs:52-136
            "agent_start" => {
                let now = utc_now_ms();
                let claimed_parked = {
                    let mut state = self.state.borrow_mut();
                    state.agent_running = true;
                    for active in &mut state.active_prompts {
                        active.agent_started = true;
                    }
                    if !state.cancelling {
                        state.turn_start_ms = Some(now);
                        state.stream_start_ms = Some(now);
                        if state.live_prompt_id.is_none() {
                            state.live_prompt_id = state
                                .active_prompts
                                .iter()
                                .rev()
                                .find_map(|p| p.client_prompt_id.clone())
                                .or_else(|| {
                                    state.queue_mirror.running().map(|entry| entry.id.clone())
                                });
                        }
                    }
                    // Server-chained turn (e.g. a Pi-queued goal reminder that
                    // starts right after the `/goal` command turn): claim the
                    // free running slot so the turn is broadcast to the pager
                    // and receives its paired prompt_complete at settle.
                    if !state.cancelling
                        && state.active_prompts.is_empty()
                        && state.queue_mirror.running().is_none()
                    {
                        state.queue_mirror.promote_parked_running()
                    } else {
                        false
                    }
                };
                if claimed_parked {
                    self.publish_queue_snapshot().await;
                }
            }
            "agent_settled" => {
                self.refresh_context_usage().await;
                let (mode_update, running) = {
                    let mut state = self.state.borrow_mut();
                    state.agent_running = false;
                    state.cancelling = false;
                    state.turn_start_ms = None;
                    state.stream_start_ms = None;
                    state.live_prompt_id = None;
                    state.bash_stream_output.clear();
                    let running = state.queue_mirror.clear_running();
                    let mode_update = if matches!(
                        state.plan_mode.state(),
                        crate::plan_mode::PiPlanState::ExitPending
                    ) {
                        state.plan_mode.complete_deferred_exit();
                        Some(acp::SessionModeId::new("default"))
                    } else {
                        None
                    };
                    (mode_update, running)
                };
                if let Some(mode_id) = mode_update {
                    self.persist_plan_mode_state()?;
                    self.sync_plan_mode_control()?;
                    self.send_update(acp::SessionUpdate::CurrentModeUpdate(
                        acp::CurrentModeUpdate::new(mode_id),
                    ))
                    .await;
                }
                self.rebroadcast_queue_mirror().await;
                self.finish_prompts(acp::StopReason::EndTurn);
                if let Some(entry) = running
                    && entry.origin != QueueOrigin::Client
                {
                    self.send_server_prompt_complete(&entry, acp::StopReason::EndTurn)
                        .await;
                }
                let dispatched = self.dispatch_next_queued().await;
                if !dispatched {
                    self.maybe_continue_goal().await;
                }
            }
            // `agent_end` is not the Pi idle barrier. Retry, compaction and
            // extension handlers can continue after it; `agent_settled` owns
            // ACP prompt completion.
```

- assistant message streaming: thought and text deltas to chunks
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/events.rs:312-328
    pub(super) async fn handle_message_update(&self, event: &Value) {
        let (text, thought) = extract_delta(event);
        {
            let mut state = self.state.borrow_mut();
            let seen = state.live_assistant.get_or_insert_with(StreamSeen::default);
            seen.text |= !text.is_empty();
            seen.thought |= !thought.is_empty();
        }
        if !thought.is_empty() {
            self.send_update(acp::SessionUpdate::AgentThoughtChunk(text_chunk(thought)))
                .await;
        }
        if !text.is_empty() {
            self.send_update(acp::SessionUpdate::AgentMessageChunk(text_chunk(text)))
                .await;
        }
    }
```

- finish_prompts and remove_prompt: resolve prompt oneshots
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/events.rs:379-412
    pub(super) fn finish_prompts(&self, requested_reason: acp::StopReason) {
        let active_prompts = {
            let mut state = self.state.borrow_mut();
            // Drop stream anchors so idle notifications do not re-trigger
            // Pager Thinking pre-create with a stale streamStartMs.
            state.turn_start_ms = None;
            state.stream_start_ms = None;
            state.live_prompt_id = None;
            state.bash_stream_output.clear();
            state.tool_usage.clear();
            std::mem::take(&mut state.active_prompts)
        };
        for active in active_prompts {
            let reason = if active.cancelled {
                acp::StopReason::Cancelled
            } else {
                requested_reason.clone()
            };
            let _ = active.completion.send(PromptCompletion {
                reason,
                client_prompt_id: active.client_prompt_id,
            });
        }
    }

    pub(super) fn remove_prompt(&self, id: u64) {
        let mut state = self.state.borrow_mut();
        if let Some(index) = state
            .active_prompts
            .iter()
            .position(|active| active.id == id)
        {
            state.active_prompts.remove(index);
        }
```

- publish queue snapshot (x.ai/queue/changed)
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/queue_runtime.rs:5-22
    pub(super) async fn publish_queue_snapshot(&self) {
        let (session_id, snapshot) = {
            let state = self.state.borrow();
            (state.acp_session_id.clone(), state.queue_mirror.snapshot())
        };
        let steering_text =
            (snapshot.steering_count > 0).then(|| format!("{} steering", snapshot.steering_count));
        let follow_up_text = (snapshot.follow_up_count > 0)
            .then(|| format!("{} follow-up", snapshot.follow_up_count));
        self.send_status("steering", steering_text.as_deref()).await;
        self.send_status("follow-up", follow_up_text.as_deref())
            .await;
        self.send_ext_notification(
            "x.ai/queue/changed",
            queue_changed_params(&session_id, &snapshot),
        )
        .await;
    }
```

- send_server_prompt_complete
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/queue_runtime.rs:78-93
    pub(super) async fn send_server_prompt_complete(
        &self,
        entry: &QueueEntry,
        reason: acp::StopReason,
    ) {
        self.send_ext_notification(
            "x.ai/session/prompt_complete",
            json!({
                "sessionId": self.session_id().0,
                "promptId": entry.id,
                "stopReason": stop_reason_wire(&reason),
                "agentResult": Value::Null,
            }),
        )
        .await;
    }
```

- queue running slot: set_running_primary, running, clear_running
```rust
// from crates/codegen/pi-grok-adapter/src/queue_bridge.rs:205-235
    pub(crate) fn set_running_primary(
        &mut self,
        id: String,
        execution_text: String,
        display_text: String,
        images: Vec<Value>,
        origin: QueueOrigin,
    ) {
        if id.trim().is_empty() {
            return;
        }
        self.running = Some(QueueEntry {
            id,
            execution_text,
            display_text,
            images,
            version: 0,
            lane: QueueLane::FollowUp,
            origin,
        });
    }

    pub(crate) fn running(&self) -> Option<&QueueEntry> {
        self.running.as_ref()
    }

    pub(crate) fn clear_running(&mut self) -> Option<QueueEntry> {
        self.reserved.clear();
        self.running.take()
    }

```

- queue snapshot
```rust
// from crates/codegen/pi-grok-adapter/src/queue_bridge.rs:349-385
    pub(crate) fn snapshot(&self) -> QueueSnapshot {
        let entries = self
            .pi_entries
            .iter()
            .chain(self.local_entries.iter())
            .enumerate()
            .map(|(position, entry)| {
                json!({
                    "id": entry.id,
                    "version": entry.version,
                    "kind": "prompt",
                    "text": entry.display_text,
                    "position": position,
                })
            })
            .collect();
        let steering_count = self
            .pi_entries
            .iter()
            .chain(self.local_entries.iter())
            .filter(|entry| entry.lane == QueueLane::Steering)
            .count();
        // Parked entries are not listed as queue rows, but Pi will still run
        // them — count them so continuation guards (e.g. maybe_continue_goal)
        // do not stack a duplicate directive on top of a pending reminder.
        let total = self.pi_entries.len() + self.local_entries.len() + self.parked.len();
        QueueSnapshot {
            entries,
            running_prompt_id: self.running.as_ref().map(|entry| entry.id.clone()),
            running_text: self
                .running
                .as_ref()
                .map(|entry| entry.display_text.clone()),
            steering_count,
            follow_up_count: total - steering_count,
        }
    }
```

- queue_changed_params
```rust
// from crates/codegen/pi-grok-adapter/src/queue_bridge.rs:410-423
pub(crate) fn queue_changed_params(session_id: &str, snapshot: &QueueSnapshot) -> Value {
    let mut params = json!({
        "sessionId": session_id,
        "entries": snapshot.entries,
    });
    if let Some(running) = &snapshot.running_prompt_id {
        params["runningPromptId"] = Value::String(running.clone());
        params["runningKind"] = Value::String("prompt".to_string());
    }
    if let Some(text) = &snapshot.running_text {
        params["runningText"] = Value::String(text.clone());
    }
    params
}
```

- wire types QueueEntryWire and QueueChanged (pager parses these)
```rust
// from crates/codegen/xai-prompt-queue/src/types.rs:28-76
/// One queue row on the wire.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntryWire {
    pub id: String,
    #[serde(default)]
    pub version: u64,
    /// Omitted from the wire when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Mirrors [`QueueEntryMeta::last_editor`]; omitted from the wire when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_editor: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    /// See [`QueueEntryMeta::combined_texts`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combined_texts: Option<Vec<String>>,
    /// 0-based position among queued, not-yet-running prompts.
    #[serde(default)]
    pub position: usize,
}

/// Broadcast payload for the `x.ai/queue/changed` notification.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueChanged {
    /// The session this queue belongs to; drives per-session fan-out routing.
    pub session_id: String,
    #[serde(default)]
    pub entries: Vec<QueueEntryWire>,
    /// The prompt the actor is currently draining, `None` when no turn runs. The correlation
    /// signal a subscriber uses to adopt `current_prompt_id` for notification routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_prompt_id: Option<String>,
    /// Display text for the running prompt. Carried explicitly because the
    /// running row is omitted from [`Self::entries`]; clients use this for the
    /// turn-start user block without relying on a stale local mirror.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_text: Option<String>,
    /// Kind for the running prompt (`"prompt"` / `"bash"` / …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_kind: Option<String>,
    /// Per-prompt display texts when the running turn was combined (len ≥ 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_combined_texts: Option<Vec<String>>,
}
```

- wire type PromptCompletePayload (pager parses this)
```rust
// from crates/codegen/xai-grok-pager/src/app/acp_handler/queue.rs:20-48
/// Wire payload of `x.ai/session/prompt_complete`, emitted by `MvpAgent::prompt()` on the shell after every turn.
/// `Serialize` is derived so tests construct payloads through the same type they are parsed into (shape drift fails at compile time, not at runtime).
/// Every field except `sessionId` is optional for wire compatibility with older shells; `promptId` only exists on shells with the lost-response fix.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PromptCompletePayload {
    pub(super) session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) agent_result: Option<String>,
    /// What triggered a cancelled turn's cancel (`"send_now"` suppresses the "Turn cancelled" marker); stamped top-level, absent on older shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) cancel_trigger: Option<String>,
    /// Why a cancelled turn was cancelled (`"HookDenied"` picks the blocked-by-hook marker); stamped top-level, absent on older shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) cancellation_category: Option<String>,
    /// Structured detail of a hook-denied cancel (hook name, reason) for the blocked-prompt card; stamped top-level, absent on older shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) cancellation_context: Option<serde_json::Value>,
    /// Typed kind of a failed stop; stamped top-level, absent on older shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error_kind: Option<String>,
    /// `_meta` extension point, parsed defensively as a trigger fallback.
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub(super) meta: Option<serde_json::Value>,
}
```
