//! `grok traces`: the pager starts straight into the traces modal with no agent session behind it.
//! The mode is process-wide and set once by the CLI, so a static is enough.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_client_protocol as acp;
use xai_acp_lib::AcpClientMessage;
use xai_grok_shell::extensions::notification::SessionNotification;

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
    let feed = if row.source == "pi" {
        pi_feed::convert(&row.file_path, &row.id)
    } else if row.source == "dpi" {
        pi_feed::convert_dpi(&row.file_path.to_string_lossy(), &row.id)
    } else if row.source == "codex" {
        codex_feed::convert(&row.file_path, &row.id)
    } else {
        claude_feed::convert(&row.file_path, &row.id)
    };
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
    agent.bind_session_id(acp::SessionId::new(row.id.clone()));
    if let Some(cwd) = row.cwd.as_deref() {
        agent.session.cwd = cwd.into();
        agent.session.tracker.set_session_cwd(cwd);
    }
    agent.begin_replay_window();
    agent.scrollback.begin_batch();
    for fed in events {
        super::super::acp_handler::handle(message(&row.id, fed), app);
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.scrollback.end_batch();
    agent.session.loading_replay = false;
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
