//! One ACP stdio process per distinct `agent_cmd`, shared by every client that asks for it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::protocol::ClientCapabilities;

/// Which agent process a session or a client is talking to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum BackendId {
    Native,
    External(String),
}

impl BackendId {
    pub(crate) fn is_native(&self) -> bool {
        matches!(self, Self::Native)
    }

    pub(crate) fn from_capabilities(caps: &ClientCapabilities) -> Self {
        match caps
            .agent_cmd
            .as_deref()
            .map(str::trim)
            .filter(|cmd| !cmd.is_empty())
        {
            Some(cmd) => Self::External(cmd.to_string()),
            None => Self::Native,
        }
    }
}

/// A live external process. Dropping it kills the child.
pub(crate) struct LiveExternal {
    pub tx: mpsc::UnboundedSender<String>,
    pub pid: u32,
    pub child: Child,
    pub cached_initialize: Option<serde_json::Value>,
    pub alive: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionRoute {
    pub backend: BackendId,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct Inflight {
    pub pid: Option<u32>,
}

/// Lines and deaths from every backend, merged into the server loop.
pub(crate) enum AgentTraffic {
    FromAgent {
        backend: BackendId,
        payload: String,
    },
    Exited {
        cmd: String,
        pid: u32,
        message: String,
    },
}

pub(crate) fn spawn_external_backend(
    cmd: &str,
    events: mpsc::UnboundedSender<AgentTraffic>,
) -> Result<LiveExternal, String> {
    let mut parts = shlex::split(cmd).ok_or_else(|| format!("invalid agent command: {cmd}"))?;
    if parts.is_empty() {
        return Err("empty agent command".into());
    }
    let bin = parts.remove(0);
    let mut child = Command::new(&bin)
        .args(&parts)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn external agent `{cmd}`: {e}"))?;
    let pid = child.id().unwrap_or(0);
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("external agent `{cmd}` has no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("external agent `{cmd}` has no stdout"))?;
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err()
                || stdin.write_all(b"\n").await.is_err()
                || stdin.flush().await.is_err()
            {
                break;
            }
        }
    });
    let cmd_owned = cmd.to_string();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let msg = line.trim_end_matches(['\r', '\n']).to_string();
                    if msg.is_empty() {
                        continue;
                    }
                    if events
                        .send(AgentTraffic::FromAgent {
                            backend: BackendId::External(cmd_owned.clone()),
                            payload: msg,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = events.send(AgentTraffic::Exited {
            cmd: cmd_owned,
            pid,
            message: "process closed its output".into(),
        });
    });
    tracing::info!(%cmd, pid, "spawned external ACP backend");
    Ok(LiveExternal {
        tx,
        pid,
        child,
        cached_initialize: None,
        alive: true,
    })
}

pub(crate) fn ensure_external<'a>(
    cmd: &str,
    externals: &'a mut HashMap<String, LiveExternal>,
    events: &mpsc::UnboundedSender<AgentTraffic>,
) -> Result<&'a mut LiveExternal, String> {
    let needs_spawn = match externals.get(cmd) {
        Some(slot) if slot.alive => false,
        _ => true,
    };
    if needs_spawn {
        externals.remove(cmd);
        let slot = spawn_external_backend(cmd, events.clone())?;
        externals.insert(cmd.to_string(), slot);
    }
    externals
        .get_mut(cmd)
        .ok_or_else(|| format!("external agent `{cmd}` was not spawned"))
}

pub(crate) fn backend_pid(
    backend: &BackendId,
    externals: &HashMap<String, LiveExternal>,
) -> Option<u32> {
    match backend {
        BackendId::Native => None,
        BackendId::External(cmd) => externals.get(cmd).map(|slot| slot.pid),
    }
}

pub(crate) fn send_to_backend(
    backend: &BackendId,
    payload: String,
    native_tx: &mpsc::UnboundedSender<String>,
    externals: &mut HashMap<String, LiveExternal>,
    events: &mpsc::UnboundedSender<AgentTraffic>,
) -> Result<(), String> {
    match backend {
        BackendId::Native => native_tx
            .send(payload)
            .map_err(|_| "native agent channel closed".to_string()),
        BackendId::External(cmd) => {
            let slot = ensure_external(cmd, externals, events)?;
            if slot.tx.send(payload).is_ok() {
                return Ok(());
            }
            // The process is gone. Fail this request. The next one respawns.
            // Respawning here would swallow the crash and let a stale `Exited`
            // for the old pid land on the replacement.
            if let Some(slot) = externals.get_mut(cmd) {
                slot.alive = false;
                slot.cached_initialize = None;
                let _ = slot.child.start_kill();
            }
            Err(format!(
                "External agent `{cmd}` exited: process closed its output"
            ))
        }
    }
}

/// cwd + agent command for an external session, so a restarted leader can `session/load` it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalSessionBinding {
    pub cwd: String,
    pub cmd: String,
}

/// Sibling of the leader socket. The socket file itself is removed on shutdown.
pub(crate) fn external_session_store_path(socket_path: &Path) -> PathBuf {
    let mut name = socket_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "leader.sock".to_string());
    name.push_str(".external-sessions.json");
    socket_path.with_file_name(name)
}

pub(crate) fn load_external_sessions(path: &Path) -> HashMap<String, ExternalSessionBinding> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        tracing::warn!(
            path = %path.display(),
            "external session store is not JSON; ignoring it"
        );
        return HashMap::new();
    };
    let Some(rows) = value.get("sessions").and_then(|v| v.as_array()) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for row in rows {
        let Some(id) = row.get("sessionId").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(cwd) = row.get("cwd").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(cmd) = row.get("cmd").and_then(|v| v.as_str()) else {
            continue;
        };
        if id.is_empty() || cwd.is_empty() || cmd.is_empty() {
            continue;
        }
        out.insert(
            id.to_string(),
            ExternalSessionBinding {
                cwd: cwd.to_string(),
                cmd: cmd.to_string(),
            },
        );
    }
    out
}

fn save_external_sessions(path: &Path, sessions: &HashMap<String, ExternalSessionBinding>) {
    let rows: Vec<_> = sessions
        .iter()
        .map(|(id, binding)| {
            serde_json::json!({
                "sessionId": id,
                "cwd": binding.cwd,
                "cmd": binding.cmd,
            })
        })
        .collect();
    let Ok(body) = serde_json::to_vec_pretty(&serde_json::json!({ "sessions": rows })) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if let Err(error) = std::fs::write(&tmp, body) {
        tracing::warn!(path = %tmp.display(), %error, "failed to write external session store");
        return;
    }
    if let Err(error) = std::fs::rename(&tmp, path) {
        tracing::warn!(path = %path.display(), %error, "failed to replace external session store");
    }
}

pub(crate) fn remember_external_session(
    path: &Path,
    sessions: &mut HashMap<String, ExternalSessionBinding>,
    session_id: &str,
    cwd: &str,
    cmd: &str,
) {
    if session_id.is_empty() || cwd.is_empty() || cmd.is_empty() {
        return;
    }
    let next = ExternalSessionBinding {
        cwd: cwd.to_string(),
        cmd: cmd.to_string(),
    };
    if sessions.get(session_id) == Some(&next) {
        return;
    }
    sessions.insert(session_id.to_string(), next);
    save_external_sessions(path, sessions);
}

pub(crate) fn forget_external_session(
    path: &Path,
    sessions: &mut HashMap<String, ExternalSessionBinding>,
    session_id: &str,
) {
    if sessions.remove(session_id).is_some() {
        save_external_sessions(path, sessions);
    }
}
