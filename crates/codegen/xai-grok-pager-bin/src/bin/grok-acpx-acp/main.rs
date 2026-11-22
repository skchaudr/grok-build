//! grok-acpx-acp: acpx flows as an ACP agent for the grok pager ("acpx mode").
//!
//! The pager runs it like any external agent (`--agent-cmd grok-acpx-acp`, or an `[[agent_choice]]`). A message
//! starts an acpx flow; the run streams into the pager's own transcript (agent steps as subagent blocks, shell steps
//! as tool calls, checkpoints as questions). acpx stays the engine.
//!
//!   grok-acpx-acp [--default-flow <file>] [--runs-dir <dir>] [--acpx <bin>]
//!
//! stdout carries ACP only.

mod agent;
mod driver;
mod piw_driver;
mod piw_server;
mod project;

use agent_client_protocol as acp;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::rc::Rc;
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use xai_acp_lib::{AcpAgentGatewayReceiver, LineBufferedRead};

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the Tokio runtime")?;
    runtime.block_on(LocalSet::new().run_until(run()))
}

/// `--name value` or `--name=value`.
fn flag(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    args.iter().enumerate().find_map(|(i, a)| {
        if a == name {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&eq).map(str::to_string)
        }
    })
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("grok-acpx-acp [--default-flow <file>] [--runs-dir <dir>] [--acpx <bin>]");
        return Ok(());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
    let cfg = agent::AgentConfig {
        driver: driver::DriverConfig {
            acpx_bin: flag(&args, "--acpx").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("acpx")),
            runs_dir: flag(&args, "--runs-dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".acpx/flows/runs")),
            cwd,
            extra_path: vec![home.join(".grok/bin")],
            poll_ms: 100,
        },
        default_flow: flag(&args, "--default-flow").map(PathBuf::from),
    };

    let (gw_tx, gw_rx) = tokio::sync::mpsc::unbounded_channel();
    let agent = Rc::new(agent::AcpxAgent::new(cfg, gw_tx));
    let incoming = LineBufferedRead::spawn_local(tokio::io::stdin().compat());
    let (conn, handle_io) =
        acp::AgentSideConnection::new(agent, tokio::io::stdout().compat_write(), incoming, |fut| {
            tokio::task::spawn_local(fut);
        });
    tokio::task::spawn_local(AcpAgentGatewayReceiver::new(gw_rx, conn).run());
    handle_io.await.context("ACP stdio connection failed")
}
