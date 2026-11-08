//! `grok traces`: the pager starts straight into the traces modal with no agent session behind it.
//! The mode is process-wide and set once by the CLI, so a static is enough.

use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_client_protocol as acp;
use xai_acp_lib::AcpClientMessage;
use xai_grok_shell::extensions::notification::SessionNotification;
use xai_grok_shell::session::storage::{
    ReplayEmission, ReplayLookupFallback, ReplayPathHint, ReplayedUpdate, stream_replay_updates_at_hinted,
};

use crate::acp::claude_feed::{self, Event, Fed};
use crate::acp::live_feed::{self, Job, TraceLive};
use crate::acp::pi_feed;
use crate::acp::codex_feed;
use crate::acp::tracker::AcpUpdateTracker;
use crate::app::actions::{Action, Effect};
use crate::app::agent::{AgentId, AgentSession, AgentState};
use crate::app::agent_view::AgentView;
use crate::app::app_view::{ActiveView, AppView, AuthState, TrustState};
use crate::scrollback::state::ScrollbackState;
use crate::app::consent::ConsentState;
use crate::views::modal::ActiveModal;

static STANDALONE: AtomicBool = AtomicBool::new(false);
/// Session id (prefix) still to open once the index can answer for it.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

pub const TITLE: &str = "traces";

pub(crate) use super::traces_live::local_copy;

pub fn enable(session: Option<String>) {
    STANDALONE.store(true, Ordering::Release);
    *PENDING.lock().unwrap() = session.filter(|s| !s.trim().is_empty());
}

pub fn is_standalone() -> bool {
    STANDALONE.load(Ordering::Acquire)
}

/// Build the agent view the modal lives on without sending `session/new`, then open the modal on it.
/// Trust, consent and login gate sessions; the traces modal reads local files only, so none applies.
pub(crate) fn start(app: &mut AppView) -> Vec<Effect> {
    app.trust_state = TrustState::Done;
    app.consent_state = ConsentState::Done;
    app.auth_state = AuthState::Done;
    let (agent_id, _create_session) =
        super::session::lifecycle::dispatch_new_session_inner_with_id(app, None, false);
    if let Some(agent) = app.agents.get_mut(&agent_id) {
        agent.session_starting_since = None;
        agent.session.prompt_history_loading = false;
    }
    let effects = super::dispatch(Action::OpenTraces, app);
    open_pending(app, false);
    effects
}

/// Open the requested session once it is indexed. After the scan finished, a miss is reported instead.
pub(crate) fn open_pending(app: &mut AppView, scan_done: bool) {
    let mut pending = PENDING.lock().unwrap();
    let Some(prefix) = pending.clone() else { return };
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let Some(ActiveModal::Traces { state }) = agent.active_modal.as_mut() else { return };
    match state.open_by_prefix(&prefix) {
        Ok(()) => *pending = None,
        Err(msg) if scan_done => {
            state.status = Some(msg);
            *pending = None;
        }
        Err(_) => {}
    }
}

/// Enter on a Claude, Pi or Codex session in the sessions list: the transcript, converted, goes through the session-event handler into a
/// read-only view (no composer) as a replay; the sessions list waits in `trace_return` for Esc.
pub(crate) fn open_claude(app: &mut AppView, row: xai_grok_traces::SessionRow) {
    open_claude_in(app, row, is_standalone());
}

/// `grok traces` replays into its own view. Inside a grok session the replay gets a new agent view, so the user's session
/// is left as it was.
pub(crate) fn open_claude_in(app: &mut AppView, row: xai_grok_traces::SessionRow, standalone: bool) {
    if row.status == xai_grok_traces::Status::Active && matches!(row.source.as_str(), "claude" | "pi") {
        open_live_in(app, row, standalone);
        return;
    }
    let feed = super::traces_live::local_copy(&row).and_then(|path| feed_for(&row, &path));
    open_feed_in(app, row.id.clone(), row, feed, standalone);
}

/// The session's events from its transcript at `path` (the row's own file, or a local copy of another machine's).
pub(crate) fn feed_for(row: &xai_grok_traces::SessionRow, path: &Path) -> io::Result<Vec<Fed>> {
    match row.source.as_str() {
        "pi" => pi_feed::convert(path, &row.id),
        "dpi" => pi_feed::convert_dpi(&path.to_string_lossy(), &row.id),
        "codex" => codex_feed::convert(path, &row.id),
        "grok" => grok_feed_at(path),
        _ => claude_feed::convert(path, &row.id),
    }
}

/// Enter on a past grok session: same read-only view as Claude.
pub(crate) fn open_grok(app: &mut AppView, row: xai_grok_traces::SessionRow) {
    open_grok_in(app, row, is_standalone());
}

/// grok's `updates.jsonl` is already grok's events: no converter, replayed with the xAI updates kept so subagents survive.
/// The row's file is `<grok home>/sessions/<cwd>/<session id>/chat_history.jsonl`.
pub(crate) fn open_grok_in(app: &mut AppView, row: xai_grok_traces::SessionRow, standalone: bool) {
    let feed = super::traces_live::local_copy(&row).and_then(|path| grok_feed_at(&path));
    // Bound to the index's `grok:` id, not grok's own: a tab with this session open must not get the replay.
    open_feed_in(app, row.id.clone(), row, feed, standalone);
}

fn grok_feed_at(path: &Path) -> io::Result<Vec<Fed>> {
    let dir = path.parent();
    let sid = dir.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
    let home = dir
        .and_then(|d| d.ancestors().find(|a| a.file_name().is_some_and(|n| n == "sessions")))
        .and_then(Path::parent);
    // The session folder's own cwd finds `updates.jsonl` without scanning every session.
    let folder_cwd = dir.and_then(Path::parent).and_then(xai_grok_config::decode_cwd_from_dirname);
    match (&sid, home) {
        (Some(sid), Some(home)) => grok_feed(sid, home, folder_cwd.as_deref()),
        _ => Err(io::Error::other("not in a grok sessions directory")),
    }
}

fn grok_feed(sid: &str, home: &Path, cwd: Option<&str>) -> io::Result<Vec<Fed>> {
    let hint = ReplayPathHint {
        parent_cwd: cwd.map(Path::new),
        child_cwd: None,
        // The folder is known, so a miss means no `updates.jsonl`; scan only when its cwd can't be read.
        fallback: if cwd.is_some() { ReplayLookupFallback::HintedOnly } else { ReplayLookupFallback::Relocation },
    };
    let mut out = Vec::new();
    let emission = stream_replay_updates_at_hinted(sid, home, hint, |update| {
        let (event, meta) = match update {
            ReplayedUpdate::Acp(update, meta) => (Event::Acp(update), meta.unwrap_or_default()),
            ReplayedUpdate::Xai(update) => (Event::Xai(update), Default::default()),
        };
        let mut meta = meta;
        meta.insert("isReplay".into(), true.into());
        out.push(Fed { event, meta });
    })?;
    if emission == ReplayEmission::Empty {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no updates.jsonl to replay"));
    }
    Ok(out)
}

fn open_feed_in(
    app: &mut AppView,
    session_id: String,
    row: xai_grok_traces::SessionRow,
    feed: io::Result<Vec<Fed>>,
    standalone: bool,
) {
    let ActiveView::Agent(origin) = app.active_view else { return };
    let Some(agent) = app.agents.get_mut(&origin) else { return };
    let events = match feed {
        Ok(events) => events,
        Err(e) => {
            if let Some(ActiveModal::Traces { state }) = agent.active_modal.as_mut() {
                state.status = Some(format!("can't read {}: {e}", row.file_path.display()));
            }
            return;
        }
    };
    let Some(ActiveModal::Traces { state }) = agent.active_modal.take() else { return };
    let agent = if standalone {
        agent.clear_trace_content();
        agent
    } else {
        let id = AgentId(app.next_agent_id);
        app.next_agent_id += 1;
        let view = read_only_view(app, id);
        app.agents.insert(id, view);
        super::ctx::switch_to_agent(app, id, super::ctx::SwitchCause::Load);
        let agent = app.agents.get_mut(&id).expect("just inserted");
        agent.trace_origin = Some(origin);
        agent
    };
    agent.trace_return = Some(state);
    // Esc back to the list needs the scrollback focused; in simple mode nothing else moves it there.
    agent.set_active_pane(crate::app::agent_view::AgentPane::Scrollback, true);
    agent.bind_session_id(acp::SessionId::new(session_id.clone()));
    if let Some(cwd) = row.cwd.as_deref() {
        agent.session.cwd = cwd.into();
        agent.session.tracker.set_session_cwd(cwd);
    }
    agent.begin_replay_window();
    agent.scrollback.begin_batch();
    let live = row.status == xai_grok_traces::Status::Active;
    let seen = live.then(|| super::traces_live::signatures(&events));
    for fed in events {
        super::super::acp_handler::handle(message(&session_id, fed), app);
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.scrollback.end_batch();
    agent.session.loading_replay = false;
    if let Some(seen) = seen {
        // Running: its pending calls stay open, and later lines of the transcript keep arriving.
        let id = agent.session.id;
        super::traces_live::start(id, session_id, row, seen);
        return;
    }
    agent.scrollback.clear_running_clocks();
    agent.session.finish_turn(&mut agent.scrollback);
    agent.scrollback.finish_all_running();
    agent.mark_turn_finished(crate::app::cancel_latency::TurnEnd::Aborted);
}

/// A live Claude or Pi session. The row opens as running the moment a receipt or span is known.
/// Phoenix is read on the worker. A missing transcript still opens: receipts are enough.
fn open_live_in(app: &mut AppView, row: xai_grok_traces::SessionRow, standalone: bool) {
    let ActiveView::Agent(origin) = app.active_view else { return };
    let Some(agent) = app.agents.get_mut(&origin) else { return };
    let Some(state) = agent.active_modal.take().and_then(|m| match m {
        ActiveModal::Traces { state } => Some(state),
        other => {
            agent.active_modal = Some(other);
            None
        }
    }) else { return };
    let agent = if standalone {
        agent.clear_trace_content();
        agent
    } else {
        let id = AgentId(app.next_agent_id);
        app.next_agent_id += 1;
        let view = read_only_view(app, id);
        app.agents.insert(id, view);
        super::ctx::switch_to_agent(app, id, super::ctx::SwitchCause::Load);
        let agent = app.agents.get_mut(&id).expect("just inserted");
        agent.trace_origin = Some(origin);
        agent
    };
    agent.trace_return = Some(state);
    agent.set_active_pane(crate::app::agent_view::AgentPane::Scrollback, true);
    agent.bind_session_id(acp::SessionId::new(row.id.clone()));
    if let Some(cwd) = row.cwd.as_deref() {
        agent.session.cwd = cwd.into();
        agent.session.tracker.set_session_cwd(cwd);
    }
    let session_id = row.id.clone();
    let remote = row.host.is_some();
    let (pieces, merger, life) = if remote {
        (Vec::new(), xai_grok_traces::merge::Merger::default(), "alive".to_string())
    } else {
        let transcript = std::fs::read_to_string(&row.file_path).unwrap_or_default();
        let (receipts, pid_alive) = match xai_grok_traces::TraceIndex::open(&xai_grok_traces::default_db_path()) {
            Ok(idx) => {
                let receipts = idx.session_receipts(&row.id).unwrap_or_default();
                let pid_alive = row.harness_pid.map(|pid| match row.harness_start {
                    Some(start) => xai_grok_traces::live::alive(pid, start),
                    None => xai_grok_traces::live::alive_pid(pid),
                });
                (receipts, pid_alive)
            }
            Err(_) => (Vec::new(), None),
        };
        let life = xai_grok_traces::merge::session_life(&row.source, &receipts, pid_alive).to_string();
        let (pieces, merger, _) = xai_grok_traces::merge::load(&row.source, &transcript, &receipts, Ok(&[]));
        (pieces, merger, life)
    };
    let running = merger.running().map(|n| (n, std::time::Instant::now()));
    let events = live_feed::to_feds(&session_id, &pieces);
    agent.begin_replay_window();
    agent.scrollback.begin_batch();
    for fed in events {
        super::super::acp_handler::handle(message(&session_id, fed), app);
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.scrollback.end_batch();
    agent.session.loading_replay = false;
    agent.trace_live = Some(TraceLive {
        tail: live_feed::start(Job {
            source: row.source,
            session_id: session_id.clone(),
            file_path: row.file_path,
            native_id: row.native_id,
            host: row.host,
            harness_pid: row.harness_pid,
            harness_start: row.harness_start,
            merger,
            remote,
        }),
        session_id,
        life,
        running,
        phoenix_offline: false,
    });
}

/// Apply whatever the live worker has queued. True when the status line or the scrollback changed,
/// or a call is running and its clock should tick.
pub(crate) fn drain_live(app: &mut AppView) -> bool {
    let ActiveView::Agent(_) = app.active_view else { return false };
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return false };
    let Some(live) = agent.trace_live.as_mut() else { return false };
    let clock = live.running.is_some();
    let Some(update) = live.tail.try_recv() else { return clock };
    let mut changed = false;
    if live.life != update.life {
        live.life = update.life;
        changed = true;
    }
    if live.phoenix_offline != update.phoenix_offline {
        live.phoenix_offline = update.phoenix_offline;
        changed = true;
    }
    match (&live.running, update.running.as_deref()) {
        (Some((have, _)), Some(next)) if have == next => {}
        (_, Some(next)) => {
            live.running = Some((next.to_string(), std::time::Instant::now()));
            changed = true;
        }
        (Some(_), None) => {
            live.running = None;
            changed = true;
        }
        (None, None) => {}
    }
    let session_id = live.session_id.clone();
    let events = update.events;
    if events.is_empty() {
        return changed || clock;
    }
    agent.scrollback.begin_batch();
    for fed in events {
        super::super::acp_handler::handle(message(&session_id, fed), app);
    }
    if let Some(agent) = super::ctx::get_active_agent_mut(app) {
        agent.scrollback.end_batch();
    }
    true
}

/// Esc in the read-only view: put the sessions list back, on the session it was opened from when there is one.
pub(crate) fn leave_view(app: &mut AppView) {
    let ActiveView::Agent(id) = app.active_view else { return };
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let Some(state) = agent.trace_return.take() else { return };
    super::traces_live::stop(id);
    if let Some(origin) = agent.trace_origin {
        // The origin may have been closed meanwhile: another session takes the list, else the welcome screen.
        let back = Some(origin)
            .filter(|o| app.agents.contains_key(o))
            .or_else(|| app.agents.keys().copied().find(|k| *k != id));
        match back {
            Some(back) => {
                app.agents[&back].active_modal = Some(ActiveModal::Traces { state });
                super::ctx::switch_to_agent(app, back, super::ctx::SwitchCause::Picker);
            }
            None => super::ctx::show_welcome(app),
        }
        super::session::modal::remove_agent_and_cleanup(app, id);
        return;
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.clear_trace_content();
    agent.active_modal = Some(ActiveModal::Traces { state });
}

/// A blank top-level view for a past session; nothing is sent for it, and it has no session until the replay binds one.
fn read_only_view(app: &AppView, id: AgentId) -> AgentView {
    let mut scrollback = ScrollbackState::new();
    scrollback.set_appearance(app.appearance.clone());
    AgentView::from_app(
        app,
        AgentSession {
            id,
            acp_tx: app.acp_tx.clone(),
            session_id: None,
            models: app.models.clone(),
            state: AgentState::Idle,
            tracker: AcpUpdateTracker::new(),
            cwd: app.cwd.clone(),
            is_worktree: false,
            forked_from: None,
            pending_prompts: std::collections::VecDeque::new(),
            next_queue_id: 0,
            yolo_mode: false,
            auto_mode: false,
            prompt_history: Vec::new(),
            prompt_history_loading: false,
            loading_replay: false,
            restore_degree: None,
            rate_limited: false,
            model_incompatible: false,
            credit_limit_blocked: false,
            free_usage_blocked: false,
            available_commands: Vec::new(),
            available_commands_generation: 0,
            available_tools: None,
            model_switch_pending: false,
            hook_block_hold: false,
            blocked_prompt: None,
            user_model_preference: None,
            deferred_model_switch: None,
            bg_tasks: std::collections::BTreeMap::new(),
            bg_tool_call_to_task: std::collections::HashMap::new(),
            scheduled_tasks: std::collections::HashMap::new(),
            in_flight_prompt: None,
            forwarded_slash: None,
            turn_saw_agent_text: false,
            compact_held_prompt: None,
            current_prompt_id: None,
            created_via_new: false,
        },
        scrollback,
    )
}

/// One converted event as the message the handler takes off the wire.
pub(super) fn message(session_id: &str, fed: Fed) -> AcpClientMessage {
    let (response_tx, _rx) = tokio::sync::oneshot::channel();
    match fed.event {
        Event::Acp(update) => AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
            request: acp::SessionNotification::new(acp::SessionId::new(session_id), update).meta(Some(fed.meta)),
            response_tx,
        }),
        Event::Xai(update) => {
            let payload = SessionNotification {
                session_id: acp::SessionId::new(session_id),
                update,
                meta: Some(serde_json::Value::Object(fed.meta)),
            };
            let params = serde_json::value::to_raw_value(&payload).expect("session notification serializes");
            AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                request: acp::ExtNotification::new("x.ai/session/update", params.into()),
                response_tx,
            })
        }
    }
}

/// Whether the active view is a read-only past session, or a subagent opened from one (the parent holds `trace_return`).
pub(crate) fn in_trace_view(app: &mut AppView) -> bool {
    let crate::app::app_view::ActiveView::Agent(id) = app.active_view else { return false };
    app.agents.get(&id).is_some_and(|a| a.trace_return.is_some())
}
