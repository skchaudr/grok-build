//! Agent spawning — creates the agent process and ACP channels.
//!
//! Defaults to GrokShell (in-process) mode. Setting `GROK_ACP_AGENT_CMD`
//! switches the pager into external-agent client mode over ACP stdio.

use std::{
    fs::OpenOptions,
    io::{IsTerminal, Write},
    path::PathBuf,
    process::Stdio,
    rc::Rc,
    thread,
    time::Duration,
};

use agent_client_protocol as acp;
use anyhow::{Context, Result, anyhow, bail};
use tokio::io::AsyncReadExt;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tokio_util::sync::CancellationToken;

use xai_acp_lib::{
    AcpAgentChannel, AcpClientChannel, AcpClientTx, AcpGatewayReceiver, AcpGatewaySender,
    LineBufferedRead, acp_channels,
};
use xai_grok_shell::{
    agent::{
        MvpAgent, activity::SESSION_FLUSH_GRACE, config::Config as AgentConfig,
        models::RefreshStrategy,
    },
    auth::AuthManager,
    util::grok_home::grok_home,
};

/// Extra slack when joining the agent OS thread after cancel so the flush
/// can finish and the thread can unwind.
const AGENT_JOIN_SLACK: Duration = Duration::from_secs(2);

/// How long the join stays silent before telling an interactive user why exit
/// is taking a moment. Short joins (the common case) print nothing.
const JOIN_NOTICE_AFTER: Duration = Duration::from_millis(1500);

/// Full command string for an external ACP agent subprocess. The value is
/// shlex-split into argv and connected as NDJSON ACP over stdin/stdout.
const EXTERNAL_AGENT_CMD_ENV: &str = "GROK_ACP_AGENT_CMD";

/// Result of spawning a child agent.
pub struct SpawnedAgent {
    /// Agent worker OS thread. In the default in-process path this owns the
    /// GrokShell worker; in external-agent mode it owns the ACP stdio bridge and
    /// child process. Hand to [`AgentShutdownGuard`] for cancellation + join on
    /// every exit path.
    pub thread_handle: thread::JoinHandle<Result<()>>,
    pub channel: AcpClientChannel,
    pub cancel: CancellationToken,
    /// The agent's `AuthManager`, shared so pager-side consumers (e.g. the voice
    /// channel) resolve the same refreshing bearer as chat traffic.
    pub auth_manager: std::sync::Arc<AuthManager>,
}

/// The single teardown mechanism for an in-process agent: cancels the worker
/// and joins it on drop, so session actors always get
/// `SessionCommand::Shutdown` (SessionEnd hooks, memory save) before the
/// process exits — on normal return, `?` bail, or panic unwind alike.
///
/// Hold one from every site that calls [`spawn_grok_shell`] (headless, the TUI,
/// `models`, `worktree`, `share`). Scope-end drop is the default; the TUI is the
/// one caller that drops it explicitly, because the join has to happen before
/// background processes are reaped (see `app::run`).
pub struct AgentShutdownGuard {
    cancel: CancellationToken,
    thread: Option<thread::JoinHandle<Result<()>>>,
}

impl AgentShutdownGuard {
    /// Guard an in-process agent worker. A `None` thread makes the guard a
    /// no-op cancel (leader mode has no in-process worker to join).
    pub fn new(cancel: CancellationToken, thread: Option<thread::JoinHandle<Result<()>>>) -> Self {
        Self { cancel, thread }
    }
}

impl Drop for AgentShutdownGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        let Some(handle) = self.thread.take() else {
            return;
        };
        let timeout = SESSION_FLUSH_GRACE + AGENT_JOIN_SLACK;
        match join_agent_thread(handle, timeout) {
            JoinOutcome::Joined => {}
            JoinOutcome::Failed(error) => {
                tracing::warn!(%error, "agent worker exited with error after cancel");
            }
            JoinOutcome::Panicked(panic) => {
                tracing::warn!(%panic, "agent worker panicked after cancel");
            }
            JoinOutcome::TimedOut => {
                tracing::warn!(
                    timeout_ms = timeout.as_millis() as u64,
                    "agent worker did not exit within grace after cancel; \
                     session hooks may be incomplete"
                );
            }
            JoinOutcome::HelperLost => {
                tracing::warn!("agent worker join helper disappeared; proceeding");
            }
        }
    }
}

/// Why the join ended, so each case is explicit at the call site (and callers
/// can tell a completed flush from an abandoned one).
#[derive(Debug, PartialEq, Eq)]
enum JoinOutcome {
    /// Worker returned cleanly: session actors flushed within the grace.
    Joined,
    /// Worker returned an error; the flush may be incomplete.
    Failed(String),
    /// Worker panicked, with the payload rendered as text.
    Panicked(String),
    /// Worker was still running when the budget elapsed.
    TimedOut,
    /// The join helper vanished without reporting (helper thread itself died).
    HelperLost,
}

/// Wait up to `timeout` for a cancelled agent worker to exit.
///
/// The blocking `join` runs on a helper thread so this stays callable from
/// `Drop` — which cannot await — while every caller sits on the async runtime.
/// On timeout that helper is abandoned rather than joined; this is safe **only
/// because every caller is on its way out of the process**, so the OS reaps the
/// thread at exit. Do not reuse this outside teardown.
fn join_agent_thread(handle: thread::JoinHandle<Result<()>>, timeout: Duration) -> JoinOutcome {
    use std::sync::mpsc::RecvTimeoutError;

    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(handle.join());
    });

    // Two-phase wait: silent for a short join (overwhelmingly the common case),
    // then a one-line notice so a slow SessionEnd hook does not look like a
    // frozen exit. Only for a terminal — piped/JSON consumers stay clean.
    let quiet = timeout.min(JOIN_NOTICE_AFTER);
    match rx.recv_timeout(quiet) {
        Ok(result) => return classify_join(result),
        Err(RecvTimeoutError::Timeout) => {
            if std::io::stderr().is_terminal() {
                eprintln!("Finishing session hooks…");
            }
        }
        Err(RecvTimeoutError::Disconnected) => return JoinOutcome::HelperLost,
    }
    match rx.recv_timeout(timeout.saturating_sub(quiet)) {
        Ok(result) => classify_join(result),
        Err(RecvTimeoutError::Timeout) => JoinOutcome::TimedOut,
        Err(RecvTimeoutError::Disconnected) => JoinOutcome::HelperLost,
    }
}

fn classify_join(result: thread::Result<Result<()>>) -> JoinOutcome {
    match result {
        Ok(Ok(())) => JoinOutcome::Joined,
        Ok(Err(e)) => JoinOutcome::Failed(e.to_string()),
        Err(payload) => JoinOutcome::Panicked(panic_message(payload)),
    }
}

/// Render a panic payload as text — `panic!` payloads are `&str` or `String`,
/// so the log shows the message instead of an opaque `Any`.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Spawn a GrokShell agent in a background thread.
///
/// Returns the ACP client channel for communication and a cancellation token.
pub async fn spawn_grok_shell(
    agent_config: AgentConfig,
    cancel: &CancellationToken,
    memory_config: Option<xai_grok_shell::config::MemoryConfig>,
) -> Result<SpawnedAgent> {
    let auth_manager = std::sync::Arc::new(AuthManager::new(
        &grok_home(),
        agent_config.grok_com_config.clone(),
    ));
    auth_manager.configure_refresher(
        agent_config.grok_com_config.auth_provider_command.clone(),
        None,
    );
    // Pause token refreshes across system sleep so an OIDC refresh can't
    // straddle a suspend (which can revoke the refresh token and force
    // re-login). No-op where the OS listener is unavailable.
    auth_manager.start_system_power_listener();

    if let Some(command) = external_agent_command()? {
        let agent_cancel = cancel.child_token();
        let (channel, handle) = spawn_external_agent_subprocess(command, agent_cancel.clone())?;
        return Ok(SpawnedAgent {
            thread_handle: handle,
            channel,
            cancel: agent_cancel,
            auth_manager,
        });
    }

    // Both embedded-agent paths (`--no-leader` and leader fallback) converge
    // here, so the agent's external-OTEL gate is applied exactly once, before boot.
    xai_grok_shell::agent::app::apply_otel_config(&auth_manager, &agent_config.grok_com_config);

    // Best-effort refresh of managed policy before bootstrap reads it (repairs a wrong-identity/missing
    // cache). Never errors — the OS-protected system/MDM layers still apply, and every network step
    // inside is bounded (SESSION_START_AUTH_DEADLINE / SyncBudget::SessionStart).
    xai_grok_shell::managed_config::ensure_managed_policy_present(&auth_manager).await;

    // Run the full bootstrap sequence: config resolution, process-level
    // singletons, and model catalog construction.
    let (agent_config, models_manager) =
        xai_grok_shell::agent::init::bootstrap(&agent_config, &auth_manager, None)
            .map_err(|e| anyhow::anyhow!(e))?;
    models_manager
        .list_models(RefreshStrategy::OnlineIfUncached)
        .await;
    // Self-heal a cold-cache/failed boot fetch once the backend recovers,
    // matching the leader and stdio paths.
    models_manager.spawn_background_refresh();

    let agent_cancel = cancel.child_token();
    let (acp_client, acp_agent) = acp_channels();

    // Clone before `auth_manager` is moved into the agent closure below, so the
    // pager (voice channel) can share the same refreshing bearer.
    let auth_manager_for_pager = auth_manager.clone();

    let skills_paths = agent_config.skills.paths.clone();

    let spawn_fn: Box<dyn FnOnce(AcpClientTx) -> Result<Rc<MvpAgent>> + Send + 'static> = {
        Box::new(move |client_tx| {
            let gateway = AcpGatewaySender::new(client_tx);

            let mut agent =
                MvpAgent::with_models(gateway, &agent_config, auth_manager, models_manager);
            if let Some(mc) = memory_config {
                agent.set_memory_config(mc);
            }
            Ok(Rc::new(agent))
        })
    };

    // Spawn the agent thread with direct dispatch
    let handle =
        spawn_agent_thread_direct(spawn_fn, acp_agent, agent_cancel.clone(), skills_paths)?;

    Ok(SpawnedAgent {
        thread_handle: handle,
        channel: acp_client,
        cancel: agent_cancel,
        auth_manager: auth_manager_for_pager,
    })
}

fn external_agent_command() -> Result<Option<String>> {
    match std::env::var(EXTERNAL_AGENT_CMD_ENV) {
        Ok(command) if command.trim().is_empty() => {
            bail!("{EXTERNAL_AGENT_CMD_ENV} is set but empty")
        }
        Ok(command) => Ok(Some(command)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(anyhow!(
            "{EXTERNAL_AGENT_CMD_ENV} is not valid UTF-8: {err}"
        )),
    }
}

fn split_external_agent_command(command: &str) -> Result<Vec<String>> {
    let argv = shlex::split(command)
        .ok_or_else(|| anyhow!("{EXTERNAL_AGENT_CMD_ENV} has invalid shell quoting"))?;
    if argv.is_empty() {
        bail!("{EXTERNAL_AGENT_CMD_ENV} produced an empty argv");
    }
    Ok(argv)
}

fn external_agent_stderr_log_path() -> PathBuf {
    let log_dir = grok_home().join("logs");
    if std::fs::create_dir_all(&log_dir).is_ok() {
        log_dir.join("acp-external-agent-stderr.log")
    } else {
        std::env::temp_dir().join("grok-acp-external-agent-stderr.log")
    }
}

fn spawn_external_agent_subprocess(
    command: String,
    cancel: CancellationToken,
) -> Result<(AcpClientChannel, thread::JoinHandle<Result<()>>)> {
    let argv = split_external_agent_command(&command)?;
    let (program, args) = argv
        .split_first()
        .context("external ACP agent argv unexpectedly empty")?;
    let program = program.clone();
    let args = args.to_vec();
    let stderr_log_path = external_agent_stderr_log_path();
    let (acp_client, acp_agent) = acp_channels();

    tracing::info!(
        command = %command,
        stderr_log = %stderr_log_path.display(),
        "spawning external ACP agent subprocess"
    );

    let thread_handle = thread::Builder::new()
        .name("acp-external-agent".into())
        .spawn(move || -> Result<()> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                run_external_agent_bridge(program, args, stderr_log_path, acp_agent, cancel).await
            })
        })?;

    Ok((acp_client, thread_handle))
}

async fn run_external_agent_bridge(
    program: String,
    args: Vec<String>,
    stderr_log_path: PathBuf,
    channel: AcpAgentChannel,
    cancel: CancellationToken,
) -> Result<()> {
    let mut child = tokio::process::Command::new(&program);
    child
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = child
        .spawn()
        .with_context(|| format!("failed to spawn external ACP agent '{program}'"))?;
    let stdin = child
        .stdin
        .take()
        .context("external ACP agent stdin was not piped")?;
    let stdout = child
        .stdout
        .take()
        .context("external ACP agent stdout was not piped")?;
    if let Some(stderr) = child.stderr.take() {
        let log_path = stderr_log_path.clone();
        let pid = child.id();
        tokio::task::spawn_local(async move {
            tee_external_agent_stderr(stderr, log_path, pid).await;
        });
    }

    let gw_tx = AcpGatewaySender::new(channel.tx).with_tracing(true);
    let incoming = LineBufferedRead::spawn_local(stdout.compat());
    let (conn, handle_io) =
        acp::ClientSideConnection::new(gw_tx, stdin.compat_write(), incoming, |fut| {
            tokio::task::spawn_local(fut);
        });
    let gw_rx = AcpGatewayReceiver::new(channel.rx, conn).with_tracing(true);
    let handle_io_task = tokio::task::spawn_local(handle_io);
    let gateway_task = tokio::task::spawn_local(gw_rx.run());
    tokio::task::yield_now().await;

    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            handle_io_task.abort();
            gateway_task.abort();
            kill_external_agent(&mut child, &stderr_log_path).await;
            Ok(())
        }
        status = child.wait() => {
            cancel.cancel();
            handle_io_task.abort();
            gateway_task.abort();
            let status = status.context("failed waiting for external ACP agent")?;
            if status.success() {
                Ok(())
            } else {
                Err(anyhow!(
                    "external ACP agent exited with {status}; stderr log: {}",
                    stderr_log_path.display()
                ))
            }
        }
    }
}

async fn kill_external_agent(child: &mut tokio::process::Child, stderr_log_path: &PathBuf) {
    if let Err(error) = child.start_kill() {
        tracing::debug!(%error, "external ACP agent was already stopped before kill");
    }
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => {
            tracing::debug!(%status, "external ACP agent exited after cancel");
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed waiting for external ACP agent after kill");
        }
        Err(_) => {
            tracing::warn!(
                stderr_log = %stderr_log_path.display(),
                "external ACP agent did not exit within kill wait"
            );
        }
    }
}

async fn tee_external_agent_stderr(
    mut stderr: tokio::process::ChildStderr,
    log_path: PathBuf,
    pid: Option<u32>,
) {
    let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(
                %error,
                stderr_log = %log_path.display(),
                "failed to open external ACP agent stderr log"
            );
            return;
        }
    };
    let _ = writeln!(file, "\n--- external ACP agent stderr pid={pid:?} ---");

    let mut buf = [0_u8; 8192];
    loop {
        match stderr.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if let Err(error) = file.write_all(&buf[..n]) {
                    tracing::warn!(
                        %error,
                        stderr_log = %log_path.display(),
                        "failed writing external ACP agent stderr log"
                    );
                    break;
                }
                let _ = file.flush();
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    stderr_log = %log_path.display(),
                    "failed reading external ACP agent stderr"
                );
                break;
            }
        }
    }
}

/// Spawn an agent in a dedicated thread with direct RPC dispatch.
///
/// The agent runs on a single-threaded tokio LocalSet runtime.
/// RPC requests go directly to the agent via Rc, bypassing simplex pipes.
fn spawn_agent_thread_direct(
    spawn_agent: Box<dyn FnOnce(AcpClientTx) -> Result<Rc<MvpAgent>> + Send + 'static>,
    channel: AcpAgentChannel,
    cancel: CancellationToken,
    skills_paths: Vec<String>,
) -> Result<thread::JoinHandle<Result<()>>> {
    Ok(thread::Builder::new()
        .name("acp-agent-worker".into())
        .spawn(move || -> Result<()> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                let client_tx = channel.tx.clone();
                let agent_rc = spawn_agent(client_tx)?;

                // Direct dispatch: RPC requests go straight to the agent
                let gw_rx =
                    AcpGatewayReceiver::new(channel.rx, agent_rc.clone()).with_tracing(true);
                tokio::task::spawn_local(gw_rx.run());

                let _skills_watcher = {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    let workspace_user_dir =
                        xai_grok_agent::prompt::workspace_user::optional_workspace_user_dir();
                    xai_grok_shell::config::watcher::SkillsFileWatcher::start(
                        Some(cwd.as_path()),
                        workspace_user_dir.as_deref(),
                        &skills_paths,
                    )
                    .map(|(mut watcher, mut skills_rx)| {
                        let agent = agent_rc.clone();
                        tokio::task::spawn_local(async move {
                            while let Some(change) = skills_rx.recv().await {
                                let created_discovery_dir = watcher.refresh_new_discovery_dirs();
                                match change {
                                    xai_grok_shell::config::watcher::DiscoveryChange::Skills => {
                                        tracing::info!(
                                            "skill directory changed on disk; reloading skills for all sessions"
                                        );
                                        agent.reload_skills_all_sessions();
                                        if created_discovery_dir {
                                            agent.advertise_commands_all_sessions();
                                        }
                                    }
                                    xai_grok_shell::config::watcher::DiscoveryChange::Workflows => {
                                        tracing::info!(
                                            "workflow directory changed on disk; re-advertising commands for all sessions"
                                        );
                                        agent.advertise_commands_all_sessions();
                                    }
                                }
                            }
                        })
                    })
                };
                tokio::task::yield_now().await;

                // Keep running until cancelled, then flush every live session
                // actor (SessionEnd hooks + memory save) before the LocalSet /
                // agent drop. Session actors live on dedicated OS threads and
                // only exit cleanly on SessionCommand::Shutdown; without this
                // flush, /exit and headless quit race process death and skip
                // SessionEnd. Mirrors leader auto-update / relaunch.
                cancel.cancelled().await;
                agent_rc.flush_all_sessions(SESSION_FLUSH_GRACE).await;
                anyhow::Result::Ok(())
            })
        })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_reports_clean_worker_exit() {
        let handle = thread::spawn(|| Ok(()));
        assert_eq!(
            join_agent_thread(handle, Duration::from_secs(5)),
            JoinOutcome::Joined
        );
    }

    #[test]
    fn join_reports_worker_error() {
        let handle = thread::spawn(|| Err(anyhow::anyhow!("flush failed")));
        assert_eq!(
            join_agent_thread(handle, Duration::from_secs(5)),
            JoinOutcome::Failed("flush failed".to_string())
        );
    }

    /// The timeout branch the built-binary e2e cannot reach: a wedged worker
    /// (e.g. a hung SessionEnd hook) is abandoned once the budget elapses
    /// instead of holding the process open indefinitely.
    #[test]
    fn join_abandons_wedged_worker_at_budget() {
        let handle = thread::spawn(|| {
            thread::sleep(Duration::from_secs(30));
            Ok(())
        });
        let started = std::time::Instant::now();
        assert_eq!(
            join_agent_thread(handle, Duration::from_millis(50)),
            JoinOutcome::TimedOut
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "join must return at its budget, not wait out the worker"
        );
    }

    #[test]
    fn panic_payloads_render_as_text() {
        assert_eq!(
            classify_join(Err(Box::new("boom"))),
            JoinOutcome::Panicked("boom".to_string())
        );
        assert_eq!(
            classify_join(Err(Box::new("boom".to_string()))),
            JoinOutcome::Panicked("boom".to_string())
        );
        assert_eq!(
            classify_join(Err(Box::new(7u32))),
            JoinOutcome::Panicked("non-string panic payload".to_string())
        );
    }

    #[test]
    fn external_command_is_shlex_split() {
        assert_eq!(
            split_external_agent_command("pi-acp --name 'Grok Build'").unwrap(),
            vec!["pi-acp", "--name", "Grok Build"]
        );
    }

    #[test]
    fn external_command_rejects_bad_quoting() {
        assert!(split_external_agent_command("pi-acp 'unterminated").is_err());
    }
}
