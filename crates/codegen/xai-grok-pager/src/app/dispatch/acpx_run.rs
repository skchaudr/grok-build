//! `/acpx run`: one flow per agent view, started as a child process and followed in a transcript card.
//! acpx prints nothing until the run ends, so the card is fed from the run bundle on the one-second tick.

use std::path::PathBuf;
use std::time::Instant;

use crate::acpx::bundle::{self, NodeState, Outcome, Run, RunStatus};
use crate::acpx::launch::{Launch, LaunchSpec, LaunchState};
use crate::app::app_view::AppView;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::{AcpxFlowBlock, AcpxFlowStatus};
use crate::scrollback::entry::EntryId;

pub(crate) struct AcpxRunning {
    launch: Launch,
    entry: EntryId,
    started: Instant,
    run: Option<Run>,
}

pub(crate) fn start(app: &mut AppView, flow_file: PathBuf, input_json: Option<String>) {
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let mut block = AcpxFlowBlock::started(flow_file.display().to_string());
    let launch = if agent.acpx_run.is_some() {
        Err("a flow is already running in this view; /acpx cancel stops it".to_string())
    } else {
        match bundle::runs_base() {
            None => Err("HOME is not set, so acpx has no runs directory".to_string()),
            Some(base) => {
                let spec = LaunchSpec {
                    flow_file,
                    input_json,
                    default_agent: None,
                    // Nothing can answer a permission prompt from a child with no terminal.
                    approve_all: true,
                    cwd: Some(PathBuf::from(&agent.session.cwd)),
                    acpx_bin: None,
                };
                Launch::start(&spec, &base).map_err(|e| format!("could not start acpx: {e}"))
            }
        }
    };
    match launch {
        Ok(launch) => {
            let entry = agent.scrollback.push_block(RenderBlock::AcpxFlow(block));
            agent.scrollback.set_entry_running(entry, true);
            agent.acpx_run = Some(AcpxRunning { launch, entry, started: Instant::now(), run: None });
        }
        Err(message) => {
            block.set_status(AcpxFlowStatus::Failed(message));
            agent.scrollback.push_block(RenderBlock::AcpxFlow(block));
        }
    }
}

pub(crate) fn cancel(app: &mut AppView) {
    let mut cancelled_flow = false;
    if let Some(running) = super::ctx::get_active_agent_mut(app).and_then(|agent| agent.acpx_run.as_mut()) {
        running.launch.cancel();
        cancelled_flow = true;
    }
    if cancelled_flow {
        return;
    }
    // No flow of ours is running: a mirrored pi-workflows run open in the
    // /acpx modal can still be cancelled, through the server socket.
    if let Some(agent) = super::ctx::get_active_agent_mut(app) {
        if let Some(crate::views::modal::ActiveModal::Acpx { state }) = agent.active_modal.as_mut() {
            let _ = state.cancel_open_mirror_run();
        }
    }
}

pub(crate) fn any_running(app: &AppView) -> bool {
    app.agents.values().any(|agent| agent.acpx_run.is_some())
}

/// The one-second tick: every view's flow, not only the one on screen. True when a card changed.
pub(crate) fn tick(app: &mut AppView) -> bool {
    // The screens are about to read runs: pick up a workflow server that has
    // appeared since the pager started. Cheap when the mirror already runs.
    let _ = crate::piw::runtime::ensure_mirror_started();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut dirty = false;
    for agent in app.agents.values_mut() {
        let Some(running) = agent.acpx_run.as_mut() else { continue };
        let state = running.launch.poll();
        if let Some(dir) = running.launch.run_dir().map(|d| d.to_path_buf()) {
            match running.run.as_mut() {
                Some(run) => {
                    bundle::refresh(run);
                }
                None => running.run = bundle::load_run(&dir).ok(),
            }
        }
        let entry = running.entry;
        let done = !matches!(state, LaunchState::Starting | LaunchState::Running { .. });
        if let Some(item) = agent.scrollback.get_by_id_mut(entry)
            && let RenderBlock::AcpxFlow(block) = &mut item.block
        {
            match &running.run {
                Some(run) => block.update_from_run(run, now_ms),
                None => block.elapsed = running.started.elapsed(),
            }
            match state {
                LaunchState::Starting | LaunchState::Running { .. } => {}
                LaunchState::Cancelled => {
                    // The bundle still says the killed node is running.
                    for row in &mut block.rows {
                        if row.state == NodeState::Running {
                            row.state = NodeState::Done(Outcome::Cancelled);
                        }
                    }
                    block.set_status(AcpxFlowStatus::Cancelled);
                }
                LaunchState::Failed(message) => {
                    block.note = running.launch.stderr_tail().lines().last().map(str::to_string).filter(|l| !message.contains(l.as_str()));
                    block.set_status(AcpxFlowStatus::Failed(message));
                }
                // The bundle already set the status; the result line only matters when the bundle could not be read.
                LaunchState::Finished(result) if running.run.is_none() => block.set_status(match result.status {
                    RunStatus::Waiting => AcpxFlowStatus::Waiting {
                        node: result.waiting_on.unwrap_or_default(),
                        detail: result.status_detail.unwrap_or_default(),
                    },
                    RunStatus::Failed | RunStatus::TimedOut => AcpxFlowStatus::Failed(result.error.unwrap_or_else(|| "failed".into())),
                    _ => AcpxFlowStatus::Completed,
                }),
                LaunchState::Finished(_) => {}
            }
            item.invalidate_cache();
        }
        if done {
            agent.scrollback.finish_running(entry);
            agent.acpx_run = None;
        }
        dirty = true;
    }
    dirty
}
