# Notifications: standard session/update vs x.ai ext notifications and _meta keys

Notes: standard ACP updates go through acp_send(SessionNotification) and need _meta (streamStartMs etc.). xAI updates go as ext notifications named x.ai/session_notification (live) or x.ai/session/update (replay) whose JSON is {sessionId, update:{sessionUpdate:<snake_case tag>,...fields}, _meta}. Stdio caveat: typed ACP 0.10 has no subagent_spawned, so these MUST be ext notifications.

- send_update: live notification for the root session with _meta stamps
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/notifications.rs:4-33
    pub(super) async fn send_update(&self, update: acp::SessionUpdate) {
        let mut notification = acp::SessionNotification::new(self.session_id(), update);
        // Stamp the same live timing fields stock Grok shell puts on every
        // SessionNotification. Without streamStartMs the pager never pre-creates
        // Thinking… and the first-token empty window has no breathing accent.
        let mut meta = acp::Meta::new();
        {
            let state = self.state.borrow();
            if let Some(tokens) = state.last_context_tokens {
                meta.insert("totalTokens".into(), json!(tokens));
            }
            if let Some(ms) = state.turn_start_ms {
                meta.insert("turnStartMs".into(), json!(ms));
            }
            if let Some(ms) = state.stream_start_ms {
                meta.insert("streamStartMs".into(), json!(ms));
            }
            // Same pin stock Grok shell puts on every live notification.
            // Without promptId the pager can still render when current_prompt_id
            // is set locally, but turn-status / gate adoption stay incomplete.
            if let Some(pid) = state.live_prompt_id.as_ref() {
                meta.insert("promptId".into(), json!(pid));
            }
        }
        meta.insert("agentTimestampMs".into(), json!(utc_now_ms()));
        notification = notification.meta(Some(meta));
        if let Err(error) = acp_send(notification, &self.client_tx).await {
            tracing::debug!(%error, "Grok pager closed while sending Pi session update");
        }
    }
```

- send_update_for_session: notification for any (child) session with eventId and isReplay
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/notifications.rs:35-48
    pub(super) async fn send_update_for_session(
        &self,
        session_id: &str,
        update: acp::SessionUpdate,
        replay: bool,
        event_id: &str,
    ) {
        let mut notification =
            acp::SessionNotification::new(acp::SessionId::new(session_id.to_string()), update);
        let mut meta = acp::Meta::new();
        if replay {
            meta.insert("isReplay".into(), Value::Bool(true));
        }
        meta.insert("eventId".into(), Value::String(event_id.to_string()));
```

- send_replay_update: replay stamp isReplay and original agentTimestampMs
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/replay.rs:287-308
    pub(super) async fn send_replay_update(
        &self,
        update: acp::SessionUpdate,
        timestamp_ms: Option<i64>,
    ) {
        let mut notification = acp::SessionNotification::new(self.session_id(), update);
        let mut meta = acp::Meta::new();
        meta.insert("isReplay".into(), Value::Bool(true));
        if let Some(ms) = timestamp_ms {
            meta.insert("agentTimestampMs".into(), json!(ms));
        }
        if let Some(tokens) = self.state.borrow().last_context_tokens {
            meta.insert("totalTokens".into(), json!(tokens));
        }
        notification = notification.meta(Some(meta));
        if let Err(error) = acp_send(notification, &self.client_tx).await {
            tracing::debug!(%error, "Grok pager closed while sending Pi replay update");
        }
    }
}
```

- send_ext_notification
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/notifications.rs:241-249
    pub(super) async fn send_ext_notification(&self, method: &str, params: Value) {
        let Ok(raw) = serde_json::value::to_raw_value(&params) else {
            return;
        };
        let notification = acp::ExtNotification::new(method, raw.into());
        if let Err(error) = acp_send(notification, &self.client_tx).await {
            tracing::debug!(%error, method, "Grok pager closed while sending Pi UI notification");
        }
    }
```

- send_ui_notification and send_status (pi/ui toast and status)
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/notifications.rs:251-256
    pub(super) async fn send_ui_notification(&self, message: &str, kind: Option<&str>) {
        self.send_ext_notification(
            "pi/ui/notify",
            json!({ "message": message, "notifyType": kind }),
        )
        .await;
```

- send_status
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/notifications.rs:422-428
    pub(super) async fn send_status(&self, key: &str, text: Option<&str>) {
        self.send_ext_notification(
            "pi/ui/status",
            json!({ "statusKey": key, "statusText": text }),
        )
        .await;
    }
```

- pager side: NotificationMeta fields read from _meta
```rust
// from crates/codegen/xai-grok-pager/src/acp/meta.rs:11-36
/// Extracted once in [`acp_handler`](super::super::app::acp_handler) and passed downstream to the tracker and agent state.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NotificationMeta {
    /// Accumulated token count across the session (`totalTokens`).
    pub total_tokens: Option<u64>,
    /// UTC ms when this notification was sent (`agentTimestampMs`).
    pub agent_timestamp_ms: Option<i64>,
    /// UTC ms when the current LLM streaming response started (`streamStartMs`).
    /// Resets each tool-use loop iteration.
    pub stream_start_ms: Option<i64>,
    /// UTC ms when the current turn started (`turnStartMs`).
    /// Constant for the entire turn.
    pub turn_start_ms: Option<i64>,
    /// Stable id for the prompt this notification belongs to (`promptId`).
    /// The client passes a UUID in `PromptRequest._meta.promptId`; the agent echoes it on every notification it emits while processing that prompt.
    /// Used to drop chunks for cancelled / rewound turns.
    pub prompt_id: Option<String>,
    /// Whether this notification is historical replay from `session/load`.
    pub is_replay: bool,
    /// Raw `eventId` string (`"{sessionId}-{counter}"`). The agent resolves it by exact string match against persisted
    /// lines, so the full id is kept.
    pub event_id: Option<String>,
    /// Monotonic per-process sequence parsed from `eventId` (`"{sessionId}-{counter}"`, see `xai-grok-shell
    /// util::event_id`). Per-session events arrive in increasing order, so the pager keeps a highwater and drops
    /// anything at or below it. such updates always apply.
    pub event_seq: Option<u64>,
```

- pager side: NotificationMeta::from_json (key names)
```rust
// from crates/codegen/xai-grok-pager/src/acp/meta.rs:152-175
    pub fn from_json(meta: Option<&serde_json::Map<String, serde_json::Value>>) -> Self {
        let Some(m) = meta else {
            return Self::default();
        };
        let event_id = m
            .get("eventId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let event_seq = event_id.as_deref().and_then(event_id_counter);
        Self {
            total_tokens: json_u64(m.get("totalTokens")).or_else(|| json_u64(m.get("used"))),
            agent_timestamp_ms: m.get("agentTimestampMs").and_then(|v| v.as_i64()),
            stream_start_ms: m.get("streamStartMs").and_then(|v| v.as_i64()),
            turn_start_ms: m.get("turnStartMs").and_then(|v| v.as_i64()),
            prompt_id: m
                .get("promptId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            is_replay: m.get("isReplay").and_then(|v| v.as_bool()).unwrap_or(false),
            event_id,
            event_seq,
        }
    }
}
```

- pager side: eventId counter parse (last '-' segment)
```rust
// from crates/codegen/xai-grok-pager/src/acp/meta.rs:94-102
/// Extract the numeric counter from an `eventId` (`"{sessionId}-{counter}"`).
/// The counter is the part after the last `-` (session ids themselves contain `-`).
pub fn event_id_counter(event_id: &str) -> Option<u64> {
    event_id
        .rsplit('-')
        .next()
        .and_then(|c| c.parse::<u64>().ok())
}

```

- pager side: x.ai update ext method names accepted
```rust
// from crates/codegen/xai-grok-pager/src/acp/mod.rs:24-29
/// Ext methods that carry a session-scoped update and may stamp `isReplay`.
/// TUI dispatch, headless dispatch, and the session-load ACP barrier all share this list.
/// A new method thus cannot be handled in one path and classified `Unrelated` in another.
pub(crate) fn is_session_update_ext_method(method: &str) -> bool {
    matches!(method, "x.ai/session_notification" | "x.ai/session/update")
}
```

- xAI SessionNotification wrapper (camelCase)
```rust
// from crates/codegen/xai-grok-shell/src/extensions/notification.rs:66-78
/// xAI-specific session notification (parallel to acp::SessionNotification).
/// This wraps an XaiSessionUpdate with session context for persistence and replay.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotification {
    /// The ID of the session this update pertains to.
    pub session_id: acp::SessionId,
    /// The actual update content.
    pub update: SessionUpdate,
    /// Extension point for implementations
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}
```

- WorkflowPhaseInfo and WorkflowAgentInfo
```rust
// from crates/codegen/xai-grok-shell/src/extensions/notification.rs:17-36
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowPhaseInfo {
    pub title: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowAgentInfo {
    pub agent_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub state: String,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub duration_ms: u64,
}
```

- SessionUpdate enum header (tag = sessionUpdate, snake_case)
```rust
// from crates/codegen/xai-grok-shell/src/extensions/notification.rs:515-518
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "sessionUpdate")]
pub enum SessionUpdate {
    /// A diff review request containing one or more file diffs for user review.
```

- SessionUpdate::SubagentSpawned, SubagentProgress, SubagentFinished
```rust
// from crates/codegen/xai-grok-shell/src/extensions/notification.rs:737-847
    /// A subagent session has been spawned. Sent on the PARENT session's notification channel so the client knows this `child_session_id` is a subagent and can route its events.
    /// Emitted BEFORE dispatching `SessionCommand::Prompt` to the child. This prevents a race where child events arrive before the client has the session ID mapping.
    SubagentSpawned {
        /// Unique subagent identifier (same as child session ID).
        subagent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
        /// The parent session that spawned this subagent.
        parent_session_id: String,
        /// The parent prompt/turn that spawned this subagent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_prompt_id: Option<String>,
        /// The child session's ACP session ID.
        child_session_id: String,
        /// Agent type used for the subagent ("general-purpose", "explore", "plan", or custom).
        subagent_type: String,
        /// Short human-readable description of the task.
        description: String,
        /// Effective context source after bootstrap: "new" or "resumed".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effective_context_source: Option<String>,
        /// Whether the forked context was normalized into <background_context>.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        context_normalized: bool,
        /// Capability mode applied to this subagent (e.g. "read-only").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_mode: Option<String>,
        /// Named persona applied to this subagent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona: Option<String>,
        /// Role that supplied defaults for this subagent (e.g. "researcher").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        /// Effective model ID used by the subagent (may differ from the parent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// ID of the source subagent this session was resumed from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resumed_from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_run_id: Option<String>,
        /// Live-only opaque child address. Wire key is `agentAddress`; omitted from `updates.jsonl`.
        #[serde(
            default,
            rename = "agentAddress",
            alias = "agent_address",
            skip_serializing_if = "Option::is_none"
        )]
        agent_address: Option<String>,
    },
    /// Periodic progress update for a running subagent. Sent on the PARENT session's notification channel, rate-limited (every ~2s while the subagent is active). Stops automatically when the subagent completes or is cancelled.
    /// The TUI merges these into the same state path used by ACP poll responses.
    SubagentProgress {
        /// Unique subagent identifier.
        subagent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
        /// The parent session that owns this subagent.
        parent_session_id: String,
        /// The child session's ACP session ID.
        child_session_id: String,
        /// Elapsed wall-clock time in milliseconds.
        duration_ms: u64,
        /// Number of completed turns so far.
        turn_count: u32,
        /// Total tool calls executed so far.
        tool_call_count: u32,
        /// Current tokens used in the context window.
        tokens_used: u64,
        /// Total context window capacity (tokens).
        context_window_tokens: u64,
        /// Context window usage as a percentage (0-100).
        context_usage_pct: u8,
        /// Distinct tool names called so far.
        tools_used: Vec<String>,
        /// Number of errors encountered so far.
        error_count: u32,
    },
    /// A subagent session has finished (success, failure, or cancellation).
    ///
    /// Sent on the PARENT session's notification channel.
    SubagentFinished {
        /// Unique subagent identifier.
        subagent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
        /// The child session's ACP session ID.
        child_session_id: String,
        /// Outcome: "completed", "failed", or "cancelled".
        status: String,
        /// Error message if the subagent failed.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Number of tool calls made by the subagent.
        tool_calls: u32,
        /// Number of conversation turns taken by the subagent.
        turns: u32,
        /// Total wall-clock duration in milliseconds.
        duration_ms: u64,
        /// Total tokens consumed by the subagent's context window.
        #[serde(default)]
        tokens_used: u64,
        /// Final output text from the subagent (if completed).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        /// Advisory: an auto-wake prompt follows this completion.
        /// The first-party TUI no longer consumes it; its persistent "watching" status row already shows remaining background work.
        /// It stays for wire compatibility and other clients. Missing reads as `false`.
        #[serde(default)]
        will_wake: bool,
    },
```

- SessionUpdate::WorkflowUpdated
```rust
// from crates/codegen/xai-grok-shell/src/extensions/notification.rs:982-1022
    WorkflowUpdated {
        run_id: String,
        #[serde(default)]
        revision: u64,
        name: String,
        objective: String,
        status: String,
        #[serde(default)]
        foreground: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        phases: Vec<WorkflowPhaseInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_phase: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_budget: Option<u64>,
        #[serde(default)]
        agents_used: u64,
        #[serde(default)]
        agents_reserved: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agents_remaining: Option<u64>,
        #[serde(default)]
        agent_usage_incomplete: bool,
        elapsed_ms: u64,
        #[serde(default)]
        active_agents: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_agent_label: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agents: Vec<WorkflowAgentInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_event: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_event_detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_event_timestamp: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pause_message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_summary: Option<String>,
    },
```

- workflow notification JSON builder used by the Pi adapter
```rust
// from crates/codegen/xai-grok-shell/src/session/workflow/notify.rs:274-287
/// Serialize a pager-facing `x.ai/session_notification` payload for External hosts.
pub fn workflow_session_notification_json(
    session_id: &str,
    state: &WorkflowRunState,
    elapsed_ms: u64,
) -> serde_json::Value {
    let update = build_workflow_updated(state, elapsed_ms, 0);
    let notification = XaiSessionNotification {
        session_id: agent_client_protocol::SessionId::new(session_id.to_string()),
        update,
        meta: None,
    };
    serde_json::to_value(notification).unwrap_or_else(|_| serde_json::json!({}))
}
```
