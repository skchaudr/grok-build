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
    let feed = localize(&row).and_then(|local| convert(&local));
    open_feed_in(app, row.id.clone(), row, feed, standalone);
}

/// A row from another machine, copied here over ssh with what its feed reads beside it (Claude's `<session>/` folder of
/// subagents and stored outputs, grok's whole session folder), under `<traces>/remote/<host>/<its path>`.
/// A local row comes back as it is.
fn localize(row: &xai_grok_traces::SessionRow) -> io::Result<xai_grok_traces::SessionRow> {
    use std::process::{Command, Stdio};
    let Some(host) = row.host.as_deref() else { return Ok(row.clone()) };
    let target = xai_grok_traces::remote::hosts()
        .into_iter()
        .find(|t| xai_grok_traces::remote::label(t) == host)
        .ok_or_else(|| io::Error::other(format!("{host} is not in the traces hosts file")))?;
    let path = &row.file_path;
    let (dir, name, stem) = match (path.parent(), path.file_name(), path.file_stem()) {
        (Some(d), Some(n), Some(s)) => (d, n.to_string_lossy().into_owned(), s.to_string_lossy().into_owned()),
        _ => return Err(io::Error::other("remote path has no folder")),
    };
    // grok needs the whole session folder (`updates.jsonl`, `subagents/`): copy it from its parent.
    let (from, items) = if row.source == "grok" {
        let sid = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (dir.parent().unwrap_or(dir), vec![sid])
    } else {
        (dir, vec![name, stem])
    };
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let list = items.iter().map(|i| quote(i)).collect::<Vec<_>>().join(" ");
    let remote_cmd = format!("cd {} && tar -cf - $(ls -d {list} 2>/dev/null)", quote(&from.to_string_lossy()));
    let local_dir = xai_grok_traces::default_db_path()
        .with_file_name("remote")
        .join(host)
        .join(from.strip_prefix("/").unwrap_or(from));
    std::fs::create_dir_all(&local_dir)?;
    let mut ssh = Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=3", &target, &remote_cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let out = ssh.stdout.take().ok_or_else(|| io::Error::other("ssh gave no output"))?;
    let status = Command::new("tar").arg("-xf").arg("-").arg("-C").arg(&local_dir).stdin(out).status()?;
    let _ = ssh.wait();
    if !status.success() {
        return Err(io::Error::other(format!("copying from {host} failed")));
    }
    let mut local = row.clone();
    local.file_path = local_dir.join(path.strip_prefix(from).unwrap_or(path));
    Ok(local)
}

/// The row's transcript, converted by its harness's feed.
fn convert(row: &xai_grok_traces::SessionRow) -> io::Result<Vec<Fed>> {
    if row.source == "pi" {
        pi_feed::convert(&row.file_path, &row.id)
    } else if row.source == "dpi" {
        pi_feed::convert_dpi(&row.file_path.to_string_lossy(), &row.id)
    } else if row.source == "codex" {
        codex_feed::convert(&row.file_path, &row.id)
    } else {
        claude_feed::convert(&row.file_path, &row.id)
    }
}

/// A running session being followed: how much of its transcript was read, how many events went in, and which of those
/// were calls still running (they are sent again, as upserts, once their result is in).
pub(crate) struct TraceFollow {
    row: xai_grok_traces::SessionRow,
    session_id: String,
    len: u64,
    sent: usize,
    /// Ids of calls sent while still running.
    pending: std::collections::HashSet<String>,
}

/// The id of a call that has not finished yet.
fn pending_id(fed: &Fed) -> Option<String> {
    match &fed.event {
        Event::Acp(acp::SessionUpdate::ToolCall(tc)) if matches!(tc.status, acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress) => {
            Some(tc.tool_call_id.0.to_string())
        }
        _ => None,
    }
}

fn call_id(fed: &Fed) -> Option<String> {
    match &fed.event {
        Event::Acp(acp::SessionUpdate::ToolCall(tc)) => Some(tc.tool_call_id.0.to_string()),
        _ => None,
    }
}

fn file_len(row: &xai_grok_traces::SessionRow) -> u64 {
    std::fs::metadata(&row.file_path).map(|m| m.len()).unwrap_or(0)
}

/// The 1s traces tick on a followed session: when its transcript grew, feed what is new. Returns whether to redraw.
pub(crate) fn poll_follow(app: &mut AppView) -> bool {
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return false };
    let Some(mut follow) = agent.trace_follow.take() else { return false };
    let len = file_len(&follow.row);
    if len == follow.len {
        agent.trace_follow = Some(follow);
        return false;
    }
    let Ok(events) = convert(&follow.row) else {
        agent.trace_follow = Some(follow);
        return false;
    };
    follow.len = len;
    let total = events.len();
    let mut out = Vec::new();
    for (i, fed) in events.into_iter().enumerate() {
        let id = call_id(&fed);
        let was_pending = id.as_ref().is_some_and(|id| follow.pending.contains(id));
        // A call sent while running goes again (an upsert) once it has finished; everything past what was sent is new.
        if i < follow.sent && !(was_pending && pending_id(&fed).is_none()) {
            continue;
        }
        match (pending_id(&fed), id) {
            (Some(p), _) => {
                follow.pending.insert(p);
            }
            (None, Some(id)) => {
                follow.pending.remove(&id);
            }
            _ => {}
        }
        out.push(fed);
    }
    follow.sent = follow.sent.max(total);
    let sid = follow.session_id.clone();
    agent.trace_follow = Some(follow);
    if out.is_empty() {
        return false;
    }
    // The converter marks its events as replay; the handler takes those only inside a load window.
    agent.session.loading_replay = true;
    agent.scrollback.begin_batch();
    for fed in out {
        super::super::acp_handler::handle(message(&sid, fed), app);
    }
    if let Some(agent) = super::ctx::get_active_agent_mut(app) {
        agent.scrollback.end_batch();
        agent.session.loading_replay = false;
    }
    true
}

/// Enter on a past grok session: same read-only view as Claude.
pub(crate) fn open_grok(app: &mut AppView, row: xai_grok_traces::SessionRow) {
    open_grok_in(app, row, is_standalone());
}

/// grok's `updates.jsonl` is already grok's events: no converter, replayed with the xAI updates kept so subagents survive.
/// The row's file is `<grok home>/sessions/<cwd>/<session id>/chat_history.jsonl`.
pub(crate) fn open_grok_in(app: &mut AppView, row: xai_grok_traces::SessionRow, standalone: bool) {
    let local = match localize(&row) {
        Ok(local) => local,
        Err(e) => return open_feed_in(app, row.id.clone(), row, Err(e), standalone),
    };
    let dir = local.file_path.parent();
    let sid = dir.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
    let home = dir
        .and_then(|d| d.ancestors().find(|a| a.file_name().is_some_and(|n| n == "sessions")))
        .and_then(Path::parent);
    // The session folder's own cwd finds `updates.jsonl` without scanning every session.
    let folder_cwd = dir.and_then(Path::parent).and_then(xai_grok_config::decode_cwd_from_dirname);
    let feed = match (&sid, home) {
        (Some(sid), Some(home)) => grok_feed(sid, home, folder_cwd.as_deref()),
        _ => Err(io::Error::other("not in a grok sessions directory")),
    };
    // Bound to the index's `grok:` id, not grok's own: a tab with this session open must not get the replay.
    open_feed_in(app, row.id.clone(), row, feed, standalone);
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
    // A running local session is followed: calls still running stay running, and the transcript is re-read as it grows.
    let live = row.status == xai_grok_traces::Status::Active && row.host.is_none();
    let follow = live.then(|| TraceFollow {
        len: file_len(&row),
        sent: events.len(),
        pending: events.iter().filter_map(pending_id).collect(),
        session_id: session_id.clone(),
        row: row.clone(),
    });
    agent.begin_replay_window();
    agent.scrollback.begin_batch();
    for fed in events {
        super::super::acp_handler::handle(message(&session_id, fed), app);
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.scrollback.end_batch();
    agent.session.loading_replay = false;
    if let Some(follow) = follow {
        agent.trace_follow = Some(Box::new(follow));
        agent.scrollback.clear_running_clocks();
        return;
    }
    agent.scrollback.clear_running_clocks();
    agent.session.finish_turn(&mut agent.scrollback);
    agent.scrollback.finish_all_running();
    agent.mark_turn_finished(crate::app::cancel_latency::TurnEnd::Aborted);
}

/// Esc in the read-only view: put the sessions list back, on the session it was opened from when there is one.
pub(crate) fn leave_view(app: &mut AppView) {
    let ActiveView::Agent(id) = app.active_view else { return };
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let Some(state) = agent.trace_return.take() else { return };
    agent.trace_follow = None;
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
fn message(session_id: &str, fed: Fed) -> AcpClientMessage {
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
