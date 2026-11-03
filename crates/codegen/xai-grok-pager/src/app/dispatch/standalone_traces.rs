//! `grok traces`: the pager starts straight into the traces modal with no agent session behind it.
//! The mode is process-wide and set once by the CLI, so a static is enough.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_client_protocol as acp;
use xai_acp_lib::AcpClientMessage;
use xai_grok_shell::extensions::notification::SessionNotification;

use crate::acp::claude_feed::{self, Event, Fed};
use crate::app::actions::{Action, Effect};
use crate::app::app_view::{AppView, AuthState, TrustState};
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

/// Enter on a Claude session in the sessions list: the transcript, converted, goes through the session-event handler into this
/// view as a replay. The view is read-only (no composer); the sessions list waits in `trace_return` for Esc.
pub(crate) fn open_claude(app: &mut AppView, row: xai_grok_traces::SessionRow) {
    let feed = claude_feed::convert(&row.file_path, &row.id);
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
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
    agent.clear_trace_content();
    agent.trace_return = Some(state);
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
    agent.session.finish_turn(&mut agent.scrollback);
    agent.scrollback.finish_all_running();
    agent.mark_turn_finished(crate::app::cancel_latency::TurnEnd::Aborted);
}

/// Esc in the read-only view: empty it and put the sessions list back.
pub(crate) fn leave_view(app: &mut AppView) {
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let Some(state) = agent.trace_return.take() else { return };
    agent.clear_trace_content();
    agent.active_modal = Some(ActiveModal::Traces { state });
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

/// Whether the active view is a read-only past session.
pub(crate) fn in_trace_view(app: &mut AppView) -> bool {
    super::ctx::get_active_agent_mut(app).is_some_and(|a| a.trace_return.is_some())
}
