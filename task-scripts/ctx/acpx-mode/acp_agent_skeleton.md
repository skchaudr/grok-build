# ACP agent skeleton (stdio) for grok-acpx-acp, copied from grok-pi-acp and PiAgent

Notes: agent_client_protocol = 0.10.4 with feature "unstable" (workspace dependency); there is no list-sessions trait method, Pi uses ext methods (`pi/session/list`). All ACP types are used as `acp::...` after `use agent_client_protocol as acp;`.

- bin target, feature and dependency lines
```toml
// from crates/codegen/xai-grok-pager-bin/Cargo.toml:18-22
# grok-pi's Pi agent as a stdio ACP agent for the leader's agent picker.
[[bin]]
name = "grok-pi-acp"
path = "src/bin/grok-pi-acp.rs"
required-features = ["pi-grok"]
```

- bin crate dependencies used by grok-pi-acp (ACP, tokio, anyhow, serde_json, tokio-util compat)
```toml
// from crates/codegen/xai-grok-pager-bin/Cargo.toml:36-63

# Agent runtime + leader/stdio/headless entry points.
xai-grok-cloud-config = { workspace = true }
xai-grok-login = { workspace = true }
xai-grok-shell = { path = "../xai-grok-shell", default-features = false }
xai-grok-update = { path = "../xai-grok-update", default-features = false }
xai-grok-version = { workspace = true }
xai-grok-telemetry = { workspace = true }
xai-grok-workspace = { workspace = true }
xai-crash-handler = { path = "../xai-crash-handler" }
xai-acp-lib = { workspace = true }
pi-grok-adapter = { path = "../pi-grok-adapter", optional = true }
agent-client-protocol = { workspace = true }

# Async runtime + misc.
tokio = { workspace = true, features = [
    "sync",
    "rt",
    "macros",
    "rt-multi-thread",
    "signal",
    "io-util",
    "io-std",
] }
tokio-util = { workspace = true, features = ["compat", "rt"] }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- features: pi-grok gates the ACP bins
```toml
// from crates/codegen/xai-grok-pager-bin/Cargo.toml:140-144
    "xai-grok-update/default",
]
# grok-pi, grok-pi-acp and the single-binary helpers they share.
pi-grok = ["dep:pi-grok-adapter", "clap", "tempfile", "toml"]
jemalloc = ["dep:tikv-jemallocator", "dep:tikv-jemalloc-sys", "dep:tikv-jemalloc-ctl"]
```

- workspace ACP version
```toml
// from Cargo.toml:125-125
agent-client-protocol = { version = "0.10.4", features = ["unstable"] }
```

- imports of grok-pi-acp
```rust
// from crates/codegen/xai-grok-pager-bin/src/bin/grok-pi-acp.rs:69-80
use agent_client_protocol as acp;
use anyhow::{Context, Result};
use clap::Parser;
use pi_grok_adapter::{PiAgent, PiBootstrap, PiRpc, SpawnConfig, SubagentEventTransport};
use std::rc::Rc;
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use xai_acp_lib::{AcpAgentGatewayReceiver, LineBufferedRead};
use xai_grok_pager::{pi_resource_config::PiResourceCatalog, pi_resource_policy::ResourcePolicy};
use xai_grok_shell::host_features::HostFeatureManifest;

use cli::{Args, normalize_compound_short_flags, pi_args_with_startup_flags};
```

- main: multi-thread runtime, LocalSet::run_until
```rust
// from crates/codegen/xai-grok-pager-bin/src/bin/grok-pi-acp.rs:105-118
fn main() -> Result<()> {
    let mut args = Args::parse_from(normalize_compound_short_flags(std::env::args_os()));
    if args.pi_bin == "pi"
        && let Ok(pi_bin) = std::env::var("PI_BIN")
        && !pi_bin.trim().is_empty()
    {
        args.pi_bin = pi_bin;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the Tokio runtime")?;
    runtime.block_on(LocalSet::new().run_until(run(args)))
}
```

- wire the agent over stdio: gateway channel, AgentSideConnection, gateway receiver
```rust
// from crates/codegen/xai-grok-pager-bin/src/bin/grok-pi-acp.rs:603-628
    let (gw_tx, gw_rx) = tokio::sync::mpsc::unbounded_channel();
    let agent = Rc::new(
        PiAgent::new(
            process.rpc,
            gw_tx,
            bootstrap,
            pi_session_dir,
            bash_control_meta,
            context_breakdown,
            plan_mode_control,
            goal_control,
            subagent_transport,
            workflows_enabled,
            eval_v2_only_tool_policy_applied,
        )
        .context("failed to restore Pi plan-mode state")?,
    );
    let events_agent = agent.clone();
    tokio::task::spawn_local(async move { events_agent.run_events(process.events).await });

    let incoming = LineBufferedRead::spawn_local(tokio::io::stdin().compat());
    let (conn, handle_io) =
        acp::AgentSideConnection::new(agent, tokio::io::stdout().compat_write(), incoming, |fut| {
            tokio::task::spawn_local(fut);
        });
    tokio::task::spawn_local(AcpAgentGatewayReceiver::new(gw_rx, conn).run());
```

- end of run: await the IO task
```rust
// from crates/codegen/xai-grok-pager-bin/src/bin/grok-pi-acp.rs:650-650
    handle_io.await.context("ACP stdio connection failed")
```

- xai-acp-lib: spawn_local line reader, gateway aliases, acp_send
```rust
// from crates/codegen/xai-acp-lib/src/gateway.rs:104-107
pub type AcpAgentGatewayReceiver = AcpGatewayReceiver<acp::AgentSide, acp::AgentSideConnection>;
pub type AcpAgentGatewaySender = AcpGatewaySender<acp::AgentSide>;
pub type AcpClientGatewayReceiver = AcpGatewayReceiver<acp::ClientSide, acp::ClientSideConnection>;
pub type AcpClientGatewaySender = AcpGatewaySender<acp::ClientSide>;
```

- acp_send signature (agent to client request or notification)
```rust
// from crates/codegen/xai-acp-lib/src/channel.rs:34-38
pub async fn acp_send<R, T>(request: T, tx: &mpsc::UnboundedSender<R>) -> AcpResult<T::Response>
where
    T: AcpRequest,
    R: From<AcpArgs<T>> + fmt::Debug,
{
```

- LineBufferedRead::spawn_local
```rust
// from crates/codegen/xai-acp-lib/src/line_reader.rs:43-47
    pub fn spawn_local(source: impl AsyncRead + Unpin + 'static) -> Self {
        Self::new(source, |fut| {
            tokio::task::spawn_local(fut);
        })
    }
```

- agent struct holds client_tx: sender of AcpClientMessage
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:273-277
pub struct PiAgent {
    rpc: PiRpc,
    client_tx: mpsc::UnboundedSender<AcpClientMessage>,
    state: Rc<RefCell<AdapterState>>,
    bash_control_meta: Option<PathBuf>,
```

- adapter module imports
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:38-56
        normalize_tool_raw_output, pi_result_text, tool_content, tool_kind,
    },
    workflow_host::{
        WorkflowHost, WorkflowRequest, format_outcome_for_tool, outcome_to_json,
        parse_workflow_request,
    },
};
use agent_client_protocol as acp;
use anyhow::{Result, anyhow, bail};
use indexmap::IndexMap;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
```

- helpers: content_chunk, text_chunk
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:1167-1173
fn content_chunk(content: acp::ContentBlock) -> acp::ContentChunk {
    acp::ContentChunk::new(content)
}

fn text_chunk(text: impl Into<String>) -> acp::ContentChunk {
    content_chunk(acp::ContentBlock::Text(acp::TextContent::new(text)))
}
```

- helpers: ext_response, acp_internal, utc_now_ms
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:1735-1750
fn ext_response(value: Value) -> Result<acp::ExtResponse> {
    let raw = serde_json::value::to_raw_value(&json!({ "result": value }))?;
    Ok(acp::ExtResponse::new(raw.into()))
}

fn acp_internal(error: impl std::fmt::Display) -> acp::Error {
    acp::Error::internal_error().data(error.to_string())
}

/// Wall-clock UTC ms for ACP `_meta.agentTimestampMs` / stream anchors.
fn utc_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
```

- prompt_response stamps promptId in PromptResponse _meta
```rust
// from crates/codegen/pi-grok-adapter/src/prompt_bridge.rs:20-33
/// Stamp client `promptId` on PromptResponse so the pager can discard queued
/// mid-turn RPC completions that never became `current_prompt_id`.
pub(crate) fn prompt_response(
    reason: acp::StopReason,
    client_prompt_id: Option<&str>,
) -> acp::PromptResponse {
    let mut response = acp::PromptResponse::new(reason);
    if let Some(prompt_id) = client_prompt_id.filter(|id| !id.is_empty()) {
        let mut meta = acp::Meta::new();
        meta.insert("promptId".into(), Value::String(prompt_id.to_string()));
        response = response.meta(Some(meta));
    }
    response
}
```

- current session id accessor
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/session.rs:949-951
    pub(super) fn session_id(&self) -> acp::SessionId {
        acp::SessionId::new(self.state.borrow().acp_session_id.clone())
    }
```

- trait impl header and initialize (capabilities, agent_info)
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:3-25
#[async_trait::async_trait(?Send)]
impl acp::Agent for PiAgent {
    async fn initialize(
        &self,
        _arguments: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        // Only advertise recap when its injected bridge command is available.
        let meta = json!({ "sessionRecap": recap_extension_enabled() })
            .as_object()
            .cloned();
        Ok(acp::InitializeResponse::new(acp::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new()
                    .load_session(true)
                    .prompt_capabilities(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .embedded_context(true),
                    ),
            )
            .agent_info(acp::Implementation::new("pi", env!("CARGO_PKG_VERSION")).title("Pi"))
            .meta(meta))
    }
```

- authenticate
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:27-32
    async fn authenticate(
        &self,
        _arguments: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        Ok(acp::AuthenticateResponse::new())
    }
```

- new_session signature and response construction
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:34-37
    async fn new_session(
        &self,
        _arguments: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
```

- new_session response (models, modes)
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:68-77
        let mut response = acp::NewSessionResponse::new(bootstrap.state.session_id.clone());
        if let Some(models) = bootstrap.acp_models() {
            response = response.models(Some(models));
        }
        response = response.modes(Some(self.acp_session_modes()));
        self.persist_plan_mode_state().map_err(acp_internal)?;
        self.sync_plan_mode_control().map_err(acp_internal)?;
        self.publish_plan_file_path().await;
        Ok(response)
    }
```

- load_session signature
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:79-83
    async fn load_session(
        &self,
        arguments: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        let requested = arguments.session_id.0.to_string();
```

- load_session tail: replay, then response with models and modes
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:118-132
        self.replay_history().await.map_err(acp_internal)?;
        if bridge_command_is_registered(&bootstrap.commands, SUBAGENT_REPLAY_COMMAND) {
            self.replay_subagents("load").await?;
        }
        self.publish_bootstrap(&bootstrap).await;
        self.refresh_context_usage().await;
        let mut response = acp::LoadSessionResponse::new();
        if let Some(models) = bootstrap.acp_models() {
            response = response.models(Some(models));
        }
        response = response.modes(Some(self.acp_session_modes()));
        self.sync_plan_mode_control().map_err(acp_internal)?;
        self.publish_plan_file_path().await;
        Ok(response)
    }
```

- set_session_mode
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:134-169
    async fn set_session_mode(
        &self,
        arguments: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        let mode_id = arguments.mode_id.0.to_string();
        let mode = mode_id.as_str();
        let (changed, current_mode_id) = {
            let mut state = self.state.borrow_mut();
            let turn_in_flight = !state.active_prompts.is_empty();
            let changed = if mode == "plan" {
                state.plan_mode.enter_pending()
            } else {
                // Any non-plan mode exits plan mode (default/ask/agent).
                let was_plan = state.plan_mode.state() != crate::plan_mode::PiPlanState::Inactive;
                if was_plan {
                    state.plan_mode.user_exit(turn_in_flight);
                    true
                } else {
                    false
                }
            };
            // ACP display state follows the request. ExitPending is an internal
            // turn-drain state only; Pager must immediately confirm default.
            let current = if mode == "plan" { "plan" } else { mode };
            (changed, current.to_string())
        };
        if changed {
            self.persist_plan_mode_state().map_err(acp_internal)?;
            self.sync_plan_mode_control().map_err(acp_internal)?;
            self.send_update(acp::SessionUpdate::CurrentModeUpdate(
                acp::CurrentModeUpdate::new(acp::SessionModeId::new(current_mode_id)),
            ))
            .await;
        }
        Ok(acp::SetSessionModeResponse::new())
    }
```

- prompt signature
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:171-174
    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
```

- cancel signature
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:354-354
    async fn cancel(&self, arguments: acp::CancelNotification) -> Result<(), acp::Error> {
```

- set_session_model signature and unknown-model error
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:405-418
    async fn set_session_model(
        &self,
        arguments: acp::SetSessionModelRequest,
    ) -> Result<acp::SetSessionModelResponse, acp::Error> {
        let model_id = arguments.model_id.0.to_string();
        let model = self
            .state
            .borrow()
            .model_map
            .get(&model_id)
            .cloned()
            .ok_or_else(|| {
                acp::Error::invalid_params().data(format!("unknown Pi model: {model_id}"))
            })?;
```

- ext_method signature, first arm, fallthrough error
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:466-468
    async fn ext_method(&self, arguments: acp::ExtRequest) -> Result<acp::ExtResponse, acp::Error> {
        match arguments.method.as_ref() {
            "x.ai/interject" => self.handle_steer_message(arguments.params.get()).await,
```

- ext_method fallthrough
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:812-817
            method => Err(acp::Error::new(
                acp::ErrorCode::MethodNotFound.into(),
                format!("Method not found: {method}"),
            )),
        }
    }
```

- ext_notification signature
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/agent.rs:819-821
    async fn ext_notification(&self, arguments: acp::ExtNotification) -> Result<(), acp::Error> {
        match arguments.method.as_ref() {
            "pi/extension_command" => {
```

- session modes and model state constructors
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/session.rs:958-976
    pub(super) fn acp_session_modes(&self) -> acp::SessionModeState {
        let current = {
            let state = self.state.borrow();
            match state.plan_mode.state() {
                crate::plan_mode::PiPlanState::Pending | crate::plan_mode::PiPlanState::Active => {
                    "plan"
                }
                crate::plan_mode::PiPlanState::ExitPending
                | crate::plan_mode::PiPlanState::Inactive => "default",
            }
        };
        acp::SessionModeState::new(
            acp::SessionModeId::new(current),
            vec![
                acp::SessionMode::new(acp::SessionModeId::new("default"), "Agent"),
                acp::SessionMode::new(acp::SessionModeId::new("plan"), "Plan Mode"),
            ],
        )
    }
```

- model state constructor
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:98-109
    pub fn acp_models(&self) -> Option<acp::SessionModelState> {
        let (available, current) = build_model_catalog(
            &self.models,
            self.state.model.as_ref(),
            &self.state.thinking_level,
        );
        let current = current.or_else(|| available.first().map(|(id, _)| id.clone()))?;
        Some(acp::SessionModelState::new(
            current,
            available.into_values().collect(),
        ))
    }
```
