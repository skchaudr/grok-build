//! `grok-pi-acp`: grok-pi's Pi agent as a stdio ACP agent, so the grok-fork leader can offer it as
//! one `[[agent_choice]]` beside Claude. Bootstrap is Dwsy's `grok-pi.rs` (719ba57f) minus the
//! pager, theme, tutorial, updater and F2 resource policy; Pi's own `~/.pi` stays authoritative.
//!
//! stdout carries ACP; errors go to stderr.

#[path = "grok_pi/ask_user_extension.rs"]
mod ask_user_extension;
#[path = "grok_pi/auth_extension.rs"]
mod auth_extension;
#[path = "grok_pi/bash_extension.rs"]
mod bash_extension;
#[path = "grok_pi/btw_extension.rs"]
mod btw_extension;
#[path = "grok_pi/cli.rs"]
mod cli;
#[path = "grok_pi/context_extension.rs"]
mod context_extension;
#[path = "grok_pi/export_extension.rs"]
mod export_extension;
#[path = "grok_pi/goal_extension.rs"]
mod goal_extension;
#[path = "grok_pi/herdr_extension.rs"]
mod herdr_extension;
#[path = "grok_pi/host_feature_extension.rs"]
mod host_feature_extension;
#[path = "grok_pi/loop_extension.rs"]
mod loop_extension;
#[path = "grok_pi/native_commands_extension.rs"]
mod native_commands_extension;
#[path = "grok_pi/pi_version.rs"]
mod pi_version;
#[path = "grok_pi/plan_mode_extension.rs"]
mod plan_mode_extension;
#[path = "grok_pi/recap_extension.rs"]
mod recap_extension;
#[path = "grok_pi/remote_tui_extension.rs"]
mod remote_tui_extension;
#[path = "grok_pi/rollback_extension.rs"]
mod rollback_extension;
#[path = "grok_pi/rpc_compat_extension.rs"]
mod rpc_compat_extension;
#[path = "grok_pi/runtime_config.rs"]
mod runtime_config;
#[path = "grok_pi/rust_tui_bridge_extension.rs"]
mod rust_tui_bridge_extension;
#[path = "grok_pi/session_paths.rs"]
mod session_paths;
#[path = "grok_pi/shortcut_manager_extension.rs"]
mod shortcut_manager_extension;
#[path = "grok_pi/subagent_extension.rs"]
mod subagent_extension;
#[path = "grok_pi/todo_extension.rs"]
mod todo_extension;
#[path = "grok_pi/tools_extension.rs"]
mod tools_extension;
#[path = "grok_pi/tree_bridge.rs"]
mod tree_bridge;

use agent_client_protocol as acp;
use anyhow::{Context, Result};
use clap::Parser;
use pi_grok_adapter::{PiAgent, PiBootstrap, PiRpc, SpawnConfig, SubagentEventTransport};
use std::rc::Rc;
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use xai_acp_lib::{AcpAgentGatewayReceiver, LineBufferedRead};

use cli::{Args, normalize_compound_short_flags, pi_args_with_startup_flags};
use runtime_config::{
    bash_bridge_enabled, bash_control_meta_for_adapter, env_flag_default_off,
    eval_mcp_enabled, eval_v2_language, eval_v2_only_enabled, eval_v2_only_tool_policy_applies,
    eval_version, normal_f2_tool_policy_applies, resolve_bash_max_wait_mins,
};
use tools_extension::{
    cli_tool_exclusions, codemode_requested, configured_builtin_tools,
    disabled_builtin_tools_from_selected, has_no_tools_arg, merge_tool_exclusions,
    tool_name_allowed_by_cli,
};

const GROK_PI_VERSION: &str = env!("CARGO_PKG_VERSION");
const PI_BOOTSTRAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// `[ui].<key>` from the grok config, the same switch grok-pi's F2 host features read.
/// Defaults are the `grok-pi.json` manifest defaults at 719ba57f.
fn ui_feature(config: Option<&toml::Value>, key: &str, default: bool) -> bool {
    config
        .and_then(|root| root.get("ui"))
        .and_then(|ui| ui.get(key))
        .and_then(toml::Value::as_bool)
        .unwrap_or(default)
}

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

async fn run(mut args: Args) -> Result<()> {
    let cwd = match args.pi_cwd.as_ref() {
        Some(path) => std::path::absolute(path).context("failed to resolve --pi-cwd")?,
        None => std::env::current_dir().context("failed to read current directory")?,
    };
    let (_pi_version, resolved_pi_bin) =
        pi_version::ensure_compatible_pi_host(&args.pi_bin).context("Pi host version check failed")?;
    args.pi_bin = resolved_pi_bin;

    let config = xai_grok_shell::config::load_effective_config().ok();
    let bridge = !args.no_bridge_extensions;
    let feature = |key: &str, default: bool| bridge && ui_feature(config.as_ref(), key, default);

    let navigate_tree_extension = bridge
        .then(tree_bridge::write_navigate_tree_extension)
        .transpose()
        .context("failed to create Pi navigateTree bridge extension")?;
    let herdr_extension = feature("pi_herdr", false)
        .then(herdr_extension::write_herdr_extension)
        .transpose()
        .context("failed to create Pi Herdr extension")?;
    let bash_bridge_enabled = bash_bridge_enabled();
    let bash_extension = bridge
        .then(bash_extension::write_bash_extension)
        .transpose()
        .context("failed to create Pi Bash/Eval extension")?;
    let eval_v2_only = eval_v2_only_enabled();
    let eval_version = if eval_v2_only { "v2" } else { eval_version() };
    let eval_v2_language = eval_v2_language();
    let subagents_on = feature("pi_subagents", true);
    let subagent_extension = subagents_on
        .then(subagent_extension::write_subagent_extension)
        .transpose()
        .context("failed to create Pi subagent extension")?;
    let subagent_transport = subagent_extension
        .as_ref()
        .map(|_| SubagentEventTransport::bind())
        .transpose()
        .context("failed to bind Pi subagent local socket")?;
    let todo_on = feature("pi_todo", true);
    let todo_extension = todo_on
        .then(todo_extension::write_todo_extension)
        .transpose()
        .context("failed to create Pi todo extension")?;
    let workflows_enabled = feature("pi_workflows", false);
    let workflow_extension = workflows_enabled
        .then(host_feature_extension::write_host_feature_extension)
        .transpose()
        .context("failed to create Pi workflows extension")?;
    let goal_extension = feature("pi_goal", false)
        .then(goal_extension::write_goal_extension)
        .transpose()
        .context("failed to create Pi goal extension")?;
    let loop_extension = feature("pi_loop", false)
        .then(loop_extension::write_loop_extension)
        .transpose()
        .context("failed to create Pi loop extension")?;
    let ask_user_extension = feature("pi_ask_user_question", false)
        .then(ask_user_extension::write_ask_user_extension)
        .transpose()
        .context("failed to create Pi ask_user_question extension")?;
    let btw_extension = feature("pi_btw", false)
        .then(btw_extension::write_btw_extension)
        .transpose()
        .context("failed to create Pi btw extension")?;
    let recap_extension = bridge
        .then(recap_extension::write_recap_extension)
        .transpose()
        .context("failed to create Pi recap extension")?;
    let context_extension = bridge
        .then(context_extension::write_context_extension)
        .transpose()
        .context("failed to create Pi context breakdown extension")?;
    let auth_extension = bridge
        .then(auth_extension::write_auth_extension)
        .transpose()
        .context("failed to create Pi auth extension")?;
    let export_extension = bridge
        .then(export_extension::write_export_extension)
        .transpose()
        .context("failed to create Pi export extension")?;
    let native_commands_extension = (bridge && env_flag_default_off("PI_GROK_NATIVE_COMMANDS"))
        .then(native_commands_extension::write_native_commands_extension)
        .transpose()
        .context("failed to create Pi native commands extension")?;
    // Off until the pager has a Remote TUI surface: its frames would be invisible and its keys unrouted.
    let remote_tui_enabled = bridge && env_flag_default_off("PI_GROK_REMOTE_TUI");
    let remote_tui_extension = remote_tui_enabled
        .then(remote_tui_extension::write_remote_tui_extension)
        .transpose()
        .context("failed to create Pi remote-tui extension")?;
    let rpc_compat_extension = bridge
        .then(rpc_compat_extension::write_rpc_compat_extension)
        .transpose()
        .context("failed to create Pi RPC compatibility extension")?;
    let shortcut_manager_extension = remote_tui_enabled
        .then(shortcut_manager_extension::write_shortcut_manager_extension)
        .transpose()
        .context("failed to create Pi shortcut manager extension")?;
    let rust_tui_bridge_extension =
        (remote_tui_enabled && env_flag_default_off("PI_GROK_RUST_TUI_BRIDGE"))
            .then(rust_tui_bridge_extension::write_rust_tui_bridge_extension)
            .transpose()
            .context("failed to create Pi Rust TUI bridge extension")?;
    let plan_mode_extension = bridge
        .then(plan_mode_extension::write_plan_mode_extension)
        .transpose()
        .context("failed to create Pi plan-mode extension")?;

    let mut pi_args = pi_args_with_startup_flags(
        std::mem::take(&mut args.pi_args),
        &args,
        navigate_tree_extension.as_ref().map(|e| e.path()),
    );
    let pi_session_dir = session_paths::pi_session_dir(&pi_args, &cwd);

    // Host bridges load before any user resource, in Dwsy's order. Pi keeps its own
    // auto-discovery of user extensions, prompts and themes (no F2 admission policy here).
    let mut startup_extensions = Vec::new();
    for path in [
        rpc_compat_extension.as_ref().map(|e| e.path()),
        herdr_extension.as_ref().map(|e| e.path()),
        remote_tui_extension.as_ref().map(|e| e.source_path()),
        shortcut_manager_extension.as_ref().map(|e| e.source_path()),
        rust_tui_bridge_extension.as_ref().map(|e| e.path()),
    ]
    .into_iter()
    .flatten()
    {
        startup_extensions.extend(["--extension".to_string(), path.to_string_lossy().into_owned()]);
    }
    pi_args.splice(0..0, startup_extensions);

    let eval_v2_only_tool_policy_applied =
        eval_v2_only_tool_policy_applies(&pi_args, bridge, eval_v2_only);
    let f2_tools_enabled =
        normal_f2_tool_policy_applies(&pi_args, bridge, eval_v2_only_tool_policy_applied);
    let selected_builtin_tools = f2_tools_enabled.then(configured_builtin_tools);
    let cli_exclusions = if let Some(selected) = selected_builtin_tools.as_deref() {
        let disabled = disabled_builtin_tools_from_selected(selected);
        merge_tool_exclusions(&mut pi_args, &disabled)
    } else {
        cli_tool_exclusions(&pi_args)
    };
    let tools_extension = f2_tools_enabled
        .then(tools_extension::write_tools_extension)
        .transpose()
        .context("failed to create Pi tools extension")?;
    let codemode_extension_requested =
        codemode_requested(&pi_args, selected_builtin_tools.as_deref());
    let bash_bridge_runtime_enabled =
        bash_bridge_enabled && tool_name_allowed_by_cli(&pi_args, "bash");
    let rollback_on =
        bridge && rollback_extension::rollback_enabled() && !has_no_tools_arg(&pi_args);
    let rollback_control_dir = rollback_on
        .then(rollback_extension::create_control_dir)
        .transpose()?;
    let rollback_ext = rollback_on
        .then(rollback_extension::write_rollback_extension)
        .transpose()
        .context("failed to create Pi rollback extension")?;

    let mut subagent_extension_catalog: Vec<serde_json::Value> = Vec::new();
    for (label, path) in [
        ("grok-pi: navigate tree", navigate_tree_extension.as_ref().map(|e| e.path())),
        ("grok-pi: RPC compatibility", rpc_compat_extension.as_ref().map(|e| e.path())),
        ("grok-pi: Herdr", herdr_extension.as_ref().map(|e| e.path())),
        ("grok-pi: remote TUI", remote_tui_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: shortcut manager", shortcut_manager_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: Rust TUI bridge", rust_tui_bridge_extension.as_ref().map(|e| e.path())),
        ("grok-pi: subagents", subagent_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: todo", todo_extension.as_ref().map(|e| e.path())),
        ("grok-pi: workflows", workflow_extension.as_ref().map(|e| e.path())),
        ("grok-pi: goal", goal_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: loop", loop_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: ask user", ask_user_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: btw", btw_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: recap", recap_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: context breakdown", context_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: auth", auth_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: export", export_extension.as_ref().map(|e| e.path())),
        ("grok-pi: native commands", native_commands_extension.as_ref().map(|e| e.path())),
        ("grok-pi: Bash/Eval", bash_extension.as_ref().map(|e| e.source_path())),
        ("grok-pi: tools", tools_extension.as_ref().map(|e| e.path())),
        ("grok-pi: rollback", rollback_ext.as_ref().map(|e| e.source_path())),
        ("grok-pi: plan mode", plan_mode_extension.as_ref().map(|e| e.source_path())),
    ] {
        if let Some(path) = path {
            subagent_extension_catalog.push(serde_json::json!({
                "path": path,
                "label": label,
                "description": "Pi-Grok extension admitted for this session",
            }));
        }
    }
    for path in [
        subagent_extension.as_ref().map(|e| e.source_path()),
        todo_extension.as_ref().map(|e| e.path()),
        workflow_extension.as_ref().map(|e| e.path()),
        goal_extension.as_ref().map(|e| e.source_path()),
        loop_extension.as_ref().map(|e| e.source_path()),
        ask_user_extension.as_ref().map(|e| e.source_path()),
        btw_extension.as_ref().map(|e| e.source_path()),
        recap_extension.as_ref().map(|e| e.source_path()),
        context_extension.as_ref().map(|e| e.source_path()),
        auth_extension.as_ref().map(|e| e.source_path()),
        export_extension.as_ref().map(|e| e.path()),
        native_commands_extension.as_ref().map(|e| e.path()),
        bash_extension.as_ref().map(|e| e.source_path()),
        codemode_extension_requested.then(|| std::path::Path::new("builtin:codemode")),
        tools_extension.as_ref().map(|e| e.path()),
        rollback_ext.as_ref().map(|e| e.source_path()),
        plan_mode_extension.as_ref().map(|e| e.source_path()),
    ]
    .into_iter()
    .flatten()
    {
        pi_args.extend(["--extension".to_string(), path.to_string_lossy().into_owned()]);
    }

    // Child env, and the same flags in this process where the adapter reads them.
    let mut env: Vec<(String, String)> = vec![("PI_GROK".into(), "1".into())];
    let flag = |on: bool| if on { "1" } else { "0" }.to_string();
    env.push(("PI_GROK_EVAL_V2_ONLY".into(), flag(eval_v2_only_tool_policy_applied)));
    env.push((
        "PI_GROK_EVAL_MCP".into(),
        flag(eval_v2_only_tool_policy_applied && eval_mcp_enabled()),
    ));
    let set_process_flag = |name: &str, on: bool| unsafe {
        // SAFETY: single-threaded startup, before the adapter or Pi child exist.
        if on {
            std::env::set_var(name, "1");
        } else {
            std::env::remove_var(name);
        }
    };
    if recap_extension.is_some() {
        env.push(("PI_GROK_RECAP".into(), "1".into()));
    }
    set_process_flag("PI_GROK_RECAP", recap_extension.is_some());
    if let Some(selected) = selected_builtin_tools {
        env.push(("PI_GROK_BUILTIN_TOOLS".into(), selected));
    }
    if !cli_exclusions.is_empty() && tools_extension.is_some() {
        env.push(("PI_GROK_EXCLUDE_TOOLS".into(), cli_exclusions));
    }
    if let Some(transport) = subagent_transport.as_ref() {
        env.push(("PI_GROK_SUBAGENTS".into(), "1".into()));
        env.push(("PI_GROK_SUBAGENT_SOCKET".into(), transport.endpoint().to_string()));
        env.push((
            "PI_GROK_SUBAGENT_EXTENSION_CATALOG".into(),
            serde_json::to_string(&subagent_extension_catalog)?,
        ));
    } else {
        env.push(("PI_GROK_SUBAGENTS".into(), "0".into()));
        env.push(("PI_GROK_SUBAGENT_EXTENSION_CATALOG".into(), "[]".into()));
    }
    // grok-pi.json startupEnv bindings, pushed in both states so inherited values cannot leak.
    env.push((
        "PI_GROK_SUBAGENTS_V2".into(),
        flag(subagents_on && ui_feature(config.as_ref(), "pi_subagents_v2", false)),
    ));
    env.push((
        "PI_GROK_TODO_VERSION".into(),
        if todo_on && ui_feature(config.as_ref(), "pi_todo_v2", false) { "v2" } else { "v1" }.into(),
    ));
    if workflows_enabled {
        env.push(("PI_GROK_WORKFLOWS".into(), "1".into()));
    }
    if let Some(extension) = goal_extension.as_ref() {
        env.push(("PI_GROK_GOAL".into(), "1".into()));
        env.push((
            "PI_GROK_GOAL_CONTROL".into(),
            extension.control_path().to_string_lossy().into_owned(),
        ));
    }
    set_process_flag("PI_GROK_GOAL", goal_extension.is_some());
    if let Some(extension) = loop_extension.as_ref() {
        env.push(("PI_GROK_LOOP".into(), "1".into()));
        env.push((
            "PI_GROK_LOOP_CONTROL".into(),
            extension.control_path().to_string_lossy().into_owned(),
        ));
    }
    set_process_flag("PI_GROK_LOOP", loop_extension.is_some());
    if let Some(extension) = ask_user_extension.as_ref() {
        let dir = extension.dir_path().to_string_lossy().into_owned();
        env.push(("PI_GROK_ASK_USER".into(), "1".into()));
        env.push(("PI_GROK_ASK_USER_DIR".into(), dir.clone()));
        unsafe { std::env::set_var("PI_GROK_ASK_USER_DIR", &dir) };
    } else {
        unsafe { std::env::remove_var("PI_GROK_ASK_USER_DIR") };
    }
    set_process_flag("PI_GROK_ASK_USER", ask_user_extension.is_some());
    if let Some(extension) = context_extension.as_ref() {
        env.push((
            "PI_GROK_CONTEXT_BREAKDOWN".into(),
            extension.breakdown_path().to_string_lossy().into_owned(),
        ));
    }
    if let Some(extension) = bash_extension.as_ref() {
        let inherited = std::env::var("PI_GROK_BASH_MAX_WAIT_MINS").ok();
        env.push(("PI_GROK_BASH".into(), flag(bash_bridge_runtime_enabled)));
        env.push(("PI_GROK_EVAL_VERSION".into(), eval_version.to_string()));
        env.push(("PI_GROK_EVAL_V2_LANGUAGE".into(), eval_v2_language.to_string()));
        env.push((
            "PI_GROK_BASH_MAX_WAIT_MINS".into(),
            resolve_bash_max_wait_mins(args.bash_max_wait_mins, inherited.as_deref()),
        ));
        env.push((
            "PI_GROK_BASH_CONTROL_META".into(),
            if bash_bridge_runtime_enabled {
                extension.control_meta_path().to_string_lossy().into_owned()
            } else {
                String::new()
            },
        ));
    }
    if let Some(extension) = remote_tui_extension.as_ref() {
        env.push(("PI_GROK_REMOTE_TUI".into(), "1".into()));
        env.push(("PI_GROK_EXTENSION_TUI_COMPAT".into(), "1".into()));
        let meta = extension
            .source_path()
            .with_file_name("active.json")
            .to_string_lossy()
            .into_owned();
        env.push(("PI_GROK_REMOTE_TUI_META".into(), meta.clone()));
        unsafe { std::env::set_var("PI_GROK_REMOTE_TUI_META", meta) };
        // stdout is the ACP pipe, so there is no host terminal size to forward here.
        let keys = std::env::temp_dir().join(format!(
            "pi-grok-shortcut-keys-host-{}.jsonl",
            std::process::id()
        ));
        if let Err(err) = std::fs::write(&keys, b"") {
            tracing::warn!(%err, "failed to create shortcut dispatch keyfile");
        } else {
            let keys = keys.to_string_lossy().into_owned();
            env.push(("PI_GROK_SHORTCUT_KEYS".into(), keys.clone()));
            unsafe { std::env::set_var("PI_GROK_SHORTCUT_KEYS", &keys) };
        }
    }
    if let Some(extension) = plan_mode_extension.as_ref() {
        env.push((
            "PI_GROK_PLAN_CONTROL".into(),
            extension.control_path().to_string_lossy().into_owned(),
        ));
    }
    if rollback_ext.is_some() {
        env.push(("PI_GROK_ROLLBACK".into(), "1".into()));
        env.push(("GROK_PI_ROLLBACK_STATE".into(), rollback_extension::rollback_state_root()));
        if let Some(control) = rollback_control_dir.as_ref() {
            env.push(("GROK_PI_ROLLBACK_CONTROL".into(), control.clone()));
        }
    }
    if btw_extension.is_some() {
        env.push(("PI_GROK_BTW".into(), "1".into()));
    }
    set_process_flag("PI_GROK_BTW", btw_extension.is_some());

    let bash_control_meta =
        bash_control_meta_for_adapter(bash_bridge_runtime_enabled, bash_extension.as_ref());
    let context_breakdown = context_extension.as_ref().map(|e| e.breakdown_path().to_path_buf());
    let plan_mode_control = plan_mode_extension.as_ref().map(|e| e.control_path().to_path_buf());
    let goal_control = goal_extension.as_ref().map(|e| e.control_path().to_path_buf());

    let process = PiRpc::spawn(SpawnConfig {
        program: args.pi_bin.clone(),
        prefix_args: args.pi_prefix_args.clone(),
        cwd: cwd.clone(),
        pi_args,
        env,
    })
    .await?;
    let bootstrap = tokio::time::timeout(PI_BOOTSTRAP_TIMEOUT, PiBootstrap::load(&process.rpc))
        .await
        .map_err(|_| anyhow::anyhow!("Pi RPC bootstrap timed out after {PI_BOOTSTRAP_TIMEOUT:?}"))??;

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

    // Extension temp files must outlive the Pi child.
    let _keep = (
        navigate_tree_extension,
        herdr_extension,
        bash_extension,
        subagent_extension,
        todo_extension,
        workflow_extension,
        goal_extension,
        loop_extension,
        ask_user_extension,
        btw_extension,
        recap_extension,
        context_extension,
        auth_extension,
        export_extension,
        (native_commands_extension, remote_tui_extension, rpc_compat_extension),
        (shortcut_manager_extension, rust_tui_bridge_extension, plan_mode_extension),
        (tools_extension, rollback_ext),
    );
    handle_io.await.context("ACP stdio connection failed")
}
