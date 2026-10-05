use super::*;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use xai_grok_agent::config::ExternalAcpDefinition;
use xai_grok_tools::implementations::grok_build::task::coordinator::{
    ChildControl, LocalBoxFuture, SubagentProgress,
};

pub(crate) struct ExternalChildRuntime {
    pub cancellation: CancellationToken,
    pub progress: Arc<parking_lot::Mutex<SubagentProgress>>,
}

impl ChildControl for ExternalChildRuntime {
    type ProgressFuture = LocalBoxFuture<SubagentProgress>;
    fn progress(&self) -> Self::ProgressFuture {
        Box::pin(std::future::ready(self.progress.lock().clone()))
    }
    fn cancel(&self) {
        self.cancellation.cancel();
    }
}

struct ExternalTransport {
    child: Child,
    stdout: BufReader<ChildStdout>,
    gateway: GatewaySender,
    native_session_id: String,
    session_id: Option<String>,
    cancellation: CancellationToken,
    next_id: u64,
    output: String,
    progress: Arc<parking_lot::Mutex<SubagentProgress>>,
}

impl ExternalTransport {
    fn spawn(
        config: &ExternalAcpDefinition,
        native_session_id: &str,
        gateway: GatewaySender,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let (program, args) = config
            .argv
            .split_first()
            .ok_or("externalAcp.argv must not be empty")?;
        if program.is_empty()
            || config.identity.trim().is_empty()
            || config.machine.trim().is_empty()
            || config.harness.trim().is_empty()
        {
            return Err(
                "externalAcp requires nonempty executable, machine, harness and identity".into(),
            );
        }
        let mut child = Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("external ACP launch failed: {e}"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or("external ACP stdout missing")?);
        Ok(Self {
            child,
            stdout,
            gateway,
            native_session_id: native_session_id.into(),
            session_id: None,
            cancellation,
            next_id: 0,
            output: String::new(),
            progress: Default::default(),
        })
    }

    async fn write(&mut self, value: Value) -> Result<(), String> {
        let line = format!("{value}\n");
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or("external ACP stdin closed")?;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        })
        .await
        .map_err(|_| "external ACP write timed out".to_string())?
        .map_err(|e| format!("external ACP write failed: {e}"))
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .await?;
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(if method == "session/prompt" { 3600 } else { 30 });
        loop {
            let mut line = String::new();
            let mut limited = (&mut self.stdout).take(1024 * 1024 + 1);
            let read = tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Err("external ACP child cancelled".into()),
                result = tokio::time::timeout_at(deadline, limited.read_line(&mut line)) => result.map_err(|_| format!("external ACP {method} timed out"))?.map_err(|e| format!("external ACP read failed: {e}"))?,
            };
            if read == 0 {
                return Err("external ACP process closed its output".into());
            }
            if line.len() > 1024 * 1024 {
                return Err("external ACP message exceeds 1 MiB".into());
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|e| format!("invalid ACP message: {e}"))?;
            if value.get("method").is_some() {
                self.incoming(value).await?;
                continue;
            }
            if value.get("id") != Some(&json!(id)) {
                return Err("external ACP response id mismatch".into());
            }
            if let Some(error) = value.get("error") {
                return Err(format!("external ACP {method}: {error}"));
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| "external ACP response missing result".into());
        }
    }

    async fn incoming(&mut self, value: Value) -> Result<(), String> {
        let method = value["method"].as_str().unwrap_or_default();
        let mut params = value.get("params").cloned().unwrap_or(json!({}));
        if method == "session/update"
            || method == "session/request_permission"
            || params.get("sessionId").is_some()
        {
            if self.session_id.is_none()
                || params["sessionId"].as_str() != self.session_id.as_deref()
            {
                return Err("external ACP session identity mismatch".into());
            }
            params["sessionId"] = json!(self.native_session_id);
        }
        if method == "session/update" {
            if let Some(tool_id) = params
                .get_mut("update")
                .and_then(|update| update.get_mut("toolCallId"))
            {
                *tool_id = json!(format!(
                    "{}:{}",
                    self.native_session_id,
                    tool_id.as_str().unwrap_or_default()
                ));
            }
        }
        if method == "session/request_permission" {
            let tool_call = params
                .get_mut("toolCall")
                .and_then(Value::as_object_mut)
                .ok_or("external ACP permission toolCall missing")?;
            if let Some(tool_id) = tool_call.get_mut("toolCallId") {
                *tool_id = json!(format!(
                    "{}:{}",
                    self.native_session_id,
                    tool_id.as_str().unwrap_or_default()
                ));
            }
        }
        if let Some(id) = value.get("id") {
            let response = if method == "session/request_permission" {
                let request: acp::RequestPermissionRequest = serde_json::from_value(params)
                    .map_err(|e| format!("invalid ACP permission request: {e}"))?;
                let result = tokio::select! {
                    _ = self.cancellation.cancelled() => return Err("external ACP child cancelled during permission request".into()),
                    response = self.gateway.send(request) => response,
                };
                match result {
                    Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                    Err(error) => {
                        self.write(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":format!("permission gateway failed: {error}")}})).await?;
                        return Err(format!("external ACP permission gateway failed: {error}"));
                    }
                }
            } else {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("unsupported external ACP reverse request: {method}")}})
            };
            self.write(response).await?;
        } else if method == "session/update" {
            let update = &params["update"];
            match update["sessionUpdate"].as_str() {
                Some("agent_message_chunk") => {
                    if let Some(text) = update["content"]["text"].as_str() {
                        if self.output.len().saturating_add(text.len()) > 4 * 1024 * 1024 {
                            return Err("external ACP output exceeds 4 MiB".into());
                        }
                        self.output.push_str(text);
                    }
                }
                Some("tool_call") => self.progress.lock().tool_call_count += 1,
                _ => {}
            }
            let notification: acp::SessionNotification =
                serde_json::from_value(params).map_err(|e| format!("invalid ACP update: {e}"))?;
            if !self.gateway.forward_fire_and_forget(notification) {
                return Err("external ACP update gateway closed".into());
            }
        } else {
            return Err(format!("unsupported external ACP notification: {method}"));
        }
        Ok(())
    }

    async fn shutdown(&mut self) {
        if self.cancellation.is_cancelled() {
            if let Some(session_id) = self.session_id.clone() {
                let _ = self.write(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session_id}})).await;
                let _ =
                    tokio::time::timeout(std::time::Duration::from_millis(250), self.child.wait())
                        .await;
            }
        }
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait()).await;
    }
}

#[cfg(test)]
#[path = "external_acp_tests.rs"]
mod tests;

pub(super) async fn run_external_child(
    mut run: xai_grok_tools::implementations::grok_build::task::coordinator::ChildRunRequest<
        ChildRuntime,
    >,
    ctx: SubagentSpawnContext,
    completion_data: ShellCompletionData,
    gateway: GatewaySender,
    definition: xai_grok_agent::config::AgentDefinition,
) -> ChildRunOutput<ShellCompletionData> {
    let config = definition
        .external_acp
        .clone()
        .expect("external definition");
    let start = std::time::Instant::now();
    let mut transport = None;
    let mut meta = None;
    let parent_info = SessionInfo {
        id: acp::SessionId::new(ctx.parent_session_id.as_str()),
        cwd: ctx.parent_cwd.to_string_lossy().into_owned(),
    };
    let meta_dir = session::persistence::session_dir(&parent_info)
        .join("subagents")
        .join(&run.request.id);
    let attempt = async {
        if !matches!(gate_subagent_type(&definition.name, &ctx), SubagentValidateTypeOutcome::Ok) {
            return Err("external ACP subagent type is disabled or not allowed".into());
        }
        if xai_message_delivery_core::AgentId::from_uuid_v7(run.request.id.clone()).is_none() {
            return Err("Subagent id must be a UUIDv7".into());
        }
        if run.wake_origin.is_some() { return Err("external ACP automatic follow-up wake is not supported by this adapter yet".into()); }
        if definition.capability_mode.is_some() || run.request.runtime_overrides.capability_mode.is_some()
            || definition.isolation.is_some() || run.request.runtime_overrides.isolation.is_some()
            || !definition.tools.is_empty() || !definition.disallowed_tools.is_empty()
            || definition.session_tools_allowlist.is_some() || definition.session_tools_denylist.is_some()
            || !matches!(definition.permission_mode, PermissionMode::Default)
            || definition.hooks.is_some() || definition.max_turns.is_some()
            || ctx.parent_max_turns.is_some()
            || ctx.subagent_roles.contains_key(&run.request.subagent_type)
            || definition.initial_prompt.is_some()
            || definition.completion_requirement.is_some()
            || definition.tool_overrides.is_some()
            || ctx.subagent_model_overrides.contains_key(&run.request.subagent_type)
            || !definition.mcp_servers.is_empty()
            || run.request.runtime_overrides.output_schema.is_some()
            || run.request.runtime_overrides.output_token_budget.is_some()
            || run.request.runtime_overrides.harness_agent_type.is_some()
            || run.request.runtime_overrides.persona.is_some()
            || run.request.runtime_overrides.reasoning_effort.is_some()
            || run.request.fork_context || ctx.inherited_tool_overrides.is_some()
            || !ctx.client_hooks.is_empty()
        { return Err("external ACP cannot enforce requested Grok tool, capability, permission, hook, isolation or prompt policy".into()); }
        if ctx.parent_depth >= ctx.subagents_max_depth { return Err("external ACP subagent depth limit reached".into()); }
        let cwd = run.request.cwd.clone().or(config.cwd.clone()).unwrap_or_else(|| ctx.parent_cwd.to_string_lossy().into_owned());
        if !Path::new(&cwd).is_absolute() { return Err("external ACP cwd must be absolute on the worker machine".into()); }
        let resume = if let Some(id) = run.request.resume_from.as_deref().filter(|id| xai_tool_types::is_not_sentinel(id)) {
            if !matches!(run.reporter.resume_source(id, &ctx.parent_session_id).await, SubagentResumeLookup::Completed(_) | SubagentResumeLookup::Missing) {
                return Err("cannot resume an active external ACP child".into());
            }
            let path = session::persistence::session_dir(&parent_info).join("subagents").join(id).join("meta.json");
            let previous: SubagentMeta = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("external ACP resume metadata unavailable: {e}"))?).map_err(|e| format!("invalid external ACP resume metadata: {e}"))?;
            if previous.parent_session_id != ctx.parent_session_id || !matches!(previous.status.as_str(), "completed" | "failed" | "cancelled") {
                return Err("external ACP resume ownership or terminal-state mismatch".into());
            }
            let state = previous.external_acp.ok_or("resume source is not an external ACP child")?;
            if state.definition != config || previous.child_cwd.as_deref() != Some(&cwd) { return Err("external ACP resume backend identity or cwd mismatch".into()); }
            if !state.load_session { return Err("external ACP backend does not support session/load".into()); }
            run.request.subagent_type = previous.subagent_type;
            let _ = run.reporter.set_resolved_subagent_type(run.request.subagent_type.clone()).await;
            Some(state)
        } else { None };
        transport = Some(ExternalTransport::spawn(&config, &run.request.id, gateway.clone(), run.cancellation.clone())?);
        let t = transport.as_mut().unwrap();
        let init = t.rpc("initialize", json!({"protocolVersion":1,"clientCapabilities":{"fs":{},"terminal":false},"clientInfo":{"name":"grok-build-teammate","version":env!("CARGO_PKG_VERSION")}})).await?;
        let load_session = init["agentCapabilities"]["loadSession"].as_bool().unwrap_or(false);
        let external_id = if let Some(state) = resume {
            if !load_session { return Err("external ACP backend no longer supports session/load".into()); }
            t.session_id = Some(state.session_id.clone());
            t.rpc("session/load", json!({"sessionId":state.session_id,"cwd":cwd,"mcpServers":[]})).await?;
            state.session_id
        } else {
            let created = t.rpc("session/new", json!({"cwd":cwd,"mcpServers":[]})).await?;
            created["sessionId"].as_str().ok_or("external ACP session/new omitted sessionId")?.to_owned()
        };
        t.session_id = Some(external_id.clone());
        let definition_model = match &definition.model {
            ModelOverride::Override(model) => Some(model.clone()),
            _ => None,
        };
        let model = run.request.runtime_overrides.model.clone().or(config.model.clone()).or(definition_model);
        if let Some(model_id) = model.as_ref() { t.rpc("session/set_model", json!({"sessionId":external_id,"modelId":model_id})).await?; }
        let started_meta = SubagentMeta {
            external_acp: Some(ExternalAcpState { definition: config.clone(), session_id: external_id.clone(), load_session }),
            subagent_id: run.request.id.clone(), attempt_id: Some(run.attempt_id.to_string()), parent_session_id: ctx.parent_session_id.clone(),
            child_session_id: run.request.id.clone(), subagent_type: run.request.subagent_type.clone(), description: run.request.description.clone(), prompt: run.request.prompt.clone(),
            status: "running".into(), started_at: chrono::Utc::now(), completed_at: None, duration_ms: None, tool_calls: None, turns: None, error: None,
            effective_context_source: Some(if run.request.resume_from.is_some() { "resumed" } else { "new" }.into()), context_normalized: false, fork_copy_error: None, persona: None,
            resumed_from: run.request.resume_from.clone(), child_cwd: Some(cwd.clone()), worktree_path: None, snapshot_ref: None, effective_model_id: model.clone(),
        };
        std::fs::create_dir_all(&meta_dir).map_err(|e| format!("external ACP metadata directory failed: {e}"))?;
        if !write_subagent_meta(&meta_dir, &started_meta) { return Err("external ACP metadata persistence failed".into()); }
        meta = Some(started_meta);
        let promoted = run.reporter.started(StartedChild {
            child_session_id: run.request.id.clone(), persona: None, resumed_from: run.request.resume_from.clone(), child_cwd: cwd, worktree_path: None,
            effective_model_id: model.clone().unwrap_or_else(|| "external-default".into()), definition_background: definition.background.unwrap_or(false),
            control: ChildRuntime::External(ExternalChildRuntime { cancellation: run.cancellation.clone(), progress: t.progress.clone() }),
        }).await;
        if !promoted { return Err("external ACP child promotion rejected".into()); }
        if !emit_subagent_notification(&gateway, &ctx.parent_session_id, SessionUpdate::SubagentSpawned {
            subagent_id: run.request.id.clone(), attempt_id: Some(run.attempt_id.to_string()), parent_session_id: ctx.parent_session_id.clone(), parent_prompt_id: run.request.parent_prompt_id.clone(),
            child_session_id: run.request.id.clone(), subagent_type: run.request.subagent_type.clone(), description: run.request.description.clone(), effective_context_source: meta.as_ref().and_then(|m| m.effective_context_source.clone()),
            context_normalized: false, capability_mode: None, persona: None, role: None, model, resumed_from: run.request.resume_from.clone(), workflow_run_id: None,
            agent_address: run.agent_address.as_ref().map(ToString::to_string),
        }, ctx.parent_cmd_tx.as_ref()) { return Err("external ACP spawn notification gateway closed".into()); }
        completion_data.mark_spawned_notification_emitted();
        let mut prompt = definition.prompt_body.clone().unwrap_or_default();
        if !prompt.is_empty() { prompt.push_str("\n\n"); }
        prompt.push_str(&run.request.prompt);
        let progress = t.progress.clone();
        let progress_gateway = gateway.clone();
        let progress_parent = ctx.parent_session_id.clone();
        let progress_child = run.request.id.clone();
        let progress_attempt = run.attempt_id.to_string();
        let progress_parent_tx = ctx.parent_cmd_tx.clone();
        let _progress_task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            let mut ticks = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                ticks.tick().await;
                let snapshot = progress.lock().clone();
                if !emit_subagent_notification(&progress_gateway, &progress_parent, SessionUpdate::SubagentProgress {
                    subagent_id: progress_child.clone(), attempt_id: Some(progress_attempt.clone()), parent_session_id: progress_parent.clone(), child_session_id: progress_child.clone(),
                    duration_ms: start.elapsed().as_millis() as u64, turn_count: snapshot.turn_count, tool_call_count: snapshot.tool_call_count,
                    tokens_used: snapshot.tokens_used, context_window_tokens: snapshot.context_window_tokens, context_usage_pct: snapshot.context_usage_pct,
                    tools_used: snapshot.tools_used, error_count: snapshot.error_count,
                }, progress_parent_tx.as_ref()) { break; }
            }
        }));
        let response = t.rpc("session/prompt", json!({"sessionId":external_id,"prompt":[{"type":"text","text":prompt}]})).await?;
        if response["stopReason"] != "end_turn" { return Err(format!("external ACP prompt stopped: {}", response["stopReason"])); }
        t.progress.lock().turn_count += 1;
        Ok(t.output.clone())
    }.await;
    let mut result = match attempt {
        Ok(output) => SubagentResult {
            success: true,
            output: output.into(),
            subagent_id: run.request.id.clone(),
            child_session_id: run.request.id.clone(),
            ..Default::default()
        },
        Err(error) if run.cancellation.is_cancelled() => cancelled_result(&run.request, &error),
        Err(error) => failure_result(&run.request, &error),
    };
    if let Some(t) = transport.as_mut() {
        let progress = t.progress.lock().clone();
        result.turns = progress.turn_count;
        result.tool_calls = progress.tool_call_count;
        t.shutdown().await;
    }
    result.duration_ms = start.elapsed().as_millis() as u64;
    if let Some(mut meta) = meta {
        meta.status = result.status().into();
        meta.completed_at = Some(chrono::Utc::now());
        meta.duration_ms = Some(result.duration_ms);
        meta.tool_calls = Some(result.tool_calls);
        meta.turns = Some(result.turns);
        meta.error = result.error.clone();
        if !write_subagent_meta(&meta_dir, &meta) {
            result = failure_result(
                &run.request,
                "external ACP terminal metadata persistence failed",
            );
        }
        completion_data.set_persisted_output_dir(persist_subagent_output(&meta_dir, &result));
    }
    child_run_output(result, completion_data, None)
}
