//! Prompt-acknowledgment watch: arming at the local drain, disarming on acknowledgment, and the fail-safe abort.

use super::*;
use crate::app::dispatch::prompt_ack::{reconcile_overdue_prompt_acks_at, restore_target};
use crate::app::dispatch::turn::RewindTarget;
use crate::app::prompt_ack::{PromptAckDeadlines, PromptAckWatch};
use pretty_assertions::assert_eq;
use std::time::{Duration, Instant};
use xai_acp_lib::AcpClientMessage;

fn agent_ref(app: &AppView, id: AgentId) -> &AgentView {
    let Some(agent) = app.agents.get(&id) else {
        panic!("expected agent {id:?}");
    };
    agent
}

const DEADLINES: PromptAckDeadlines = PromptAckDeadlines {
    soft: Duration::from_secs(10),
    hard: Duration::from_secs(60),
};

fn past_hard_deadline() -> Instant {
    Instant::now() + DEADLINES.hard + Duration::from_secs(1)
}

/// Drive a live `session/prompt` through the local drain; the watch must name the live turn.
fn send_and_arm(app: &mut AppView, text: &str) -> String {
    let effects = dispatch(Action::SendPrompt(text.into()), app);
    assert!(
        matches!(effects.as_slice(), [Effect::SendPrompt { .. }]),
        "expected a local drain, got {effects:?}"
    );
    let agent = &agent_ref(app, AgentId(0));
    let watch = agent.prompt_ack.as_ref().expect("watch armed");
    assert_eq!(
        agent.session.current_prompt_id.as_deref(),
        Some(watch.prompt_id())
    );
    watch.prompt_id().to_owned()
}

fn expect_rewind_cancel(effects: &[Effect], prompt_id: &str) {
    match effects {
        [
            Effect::CancelTurn {
                cancel_subagents,
                trigger,
                rewind_prompt_id,
                ..
            },
        ] => assert_eq!(
            (false, None, Some(prompt_id)),
            (*cancel_subagents, *trigger, rewind_prompt_id.as_deref()),
            "a subagent-preserving rewind cancel with no gesture"
        ),
        other => panic!("expected exactly one rewind cancel, got {other:?}"),
    }
}

#[test]
fn local_drain_arms_the_watch_for_prompts_but_not_commands() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    send_and_arm(&mut app, "hello");
    dispatch(end_turn(), &mut app);
    assert!(
        agent_ref(&app, id).prompt_ack.is_none(),
        "the turn's response disarms the watch"
    );
    app.agents
        .get_mut(&id)
        .unwrap()
        .session
        .enqueue_command("/compact".into());
    let effects = dispatch(Action::DrainQueue, &mut app);
    assert!(matches!(effects.as_slice(), [Effect::Compact { .. }]));
    assert!(
        agent_ref(&app, id).prompt_ack.is_none(),
        "a slash command owns the pane through its own completion"
    );
}

#[test]
fn expired_watch_restores_the_prompt_and_sends_a_rewind_cancel() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let pid = send_and_arm(&mut app, "restore me");
    let effects = reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline())
        .expect("the expired watch fires");
    expect_rewind_cancel(&effects, &pid);
    let agent = agent_ref(&app, id);
    let bubbles = (0..agent.scrollback.len())
        .filter_map(|idx| agent.scrollback.get(idx))
        .filter(|entry| matches!(entry.block, RenderBlock::UserPrompt(_)))
        .count();
    assert_eq!(
        (true, None, "restore me", 0, true, true),
        (
            agent.session.state.is_idle(),
            agent.session.current_prompt_id.as_deref(),
            agent.prompt.text(),
            bubbles,
            agent.is_rewound_prompt(&pid),
            agent.prompt_ack.is_none(),
        ),
        "pane idle, text back, bubble removed, prompt recorded as rewound"
    );
}

#[test]
fn expired_watch_on_an_overlay_child_restores_the_child_prompt() {
    use crate::app::dispatch::queue::maybe_drain_queue;

    let mut app = test_app_with_agent();
    let parent_id = AgentId(0);
    let child_sid = "child-overlay-ack";
    let mut child = AgentView::new(
        make_test_agent_session(&app, AgentId(1), child_sid),
        ScrollbackState::new(),
    );
    child.session.enqueue_prompt("child text".into());
    assert!(matches!(
        maybe_drain_queue(&mut child, &mut Vec::new())
            .effects
            .as_slice(),
        [Effect::SendPrompt { .. }]
    ));
    let pid = child
        .prompt_ack
        .as_ref()
        .expect("watch armed on the child")
        .prompt_id()
        .to_owned();
    {
        let parent = app.agents.get_mut(&parent_id).unwrap();
        parent
            .subagent_views
            .insert(child_sid.to_string(), Box::new(child));
        parent.active_subagent = Some(child_sid.to_string());
    }

    let effects = reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline())
        .expect("the child's expired watch fires");
    expect_rewind_cancel(&effects, &pid);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::CancelTurn { session_id, .. }] if session_id.0.as_ref() == child_sid
        ),
        "the abort targets the child session, got {effects:?}"
    );
    let Some(child) = agent_ref(&app, parent_id).subagent_views.get(child_sid) else {
        panic!("expected overlay child {child_sid}");
    };
    assert_eq!(
        (true, "child text", true, true),
        (
            child.session.state.is_idle(),
            child.prompt.text(),
            child.is_rewound_prompt(&pid),
            child.prompt_ack.is_none(),
        ),
        "the overlay child ends idle with its text back"
    );
}

#[test]
fn restore_target_decides_where_the_text_goes() {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let empty_composer = restore_target(agent);
    agent.prompt.set_text("draft");
    let text_only_draft = restore_target(agent);
    agent.prompt_mode = PromptMode::EditingQueued {
        id: 7,
        original: "draft".into(),
        server_id: None,
        kind: crate::app::agent::QueueEntryKind::Prompt,
    };
    let editing_queued_row = restore_target(agent);
    agent.prompt_mode = PromptMode::Normal;
    agent
        .prompt
        .images
        .push(crate::prompt_images::from_clipboard_data(
            &crate::clipboard::ImageData {
                data: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0],
                mime_type: "image/png".into(),
            },
        ));
    let draft_with_images = restore_target(agent);
    assert_eq!(
        [
            Some(RewindTarget::ReplaceComposer),
            Some(RewindTarget::MergeIntoDraft),
            None,
            None,
        ],
        [
            empty_composer,
            text_only_draft,
            editing_queued_row,
            draft_with_images,
        ],
        "empty → replace; text-only → merge; queued-row edit or images → not restorable"
    );
}

#[test]
fn stale_watch_on_an_idle_pane_is_dropped_silently() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().prompt_ack =
        Some(PromptAckWatch::new("p-stale", Instant::now()));
    assert!(reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline()).is_none());
    let agent = agent_ref(&app, id);
    assert_eq!(
        (None, 0),
        (agent.prompt_ack.as_ref(), agent.scrollback.len())
    );
}

#[test]
fn expired_watch_while_cancelling_forces_idle() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    // A non-rewinding cancel leaves the pane cancelling with the stash intact and the watch armed
    app.cancel_rewind_enabled = false;
    let pid = send_and_arm(&mut app, "stuck");
    dispatch(Action::CancelTurn, &mut app);
    let agent = agent_ref(&app, id);
    assert!(agent.session.state.is_cancelling() && agent.prompt_ack.is_some());
    let effects = reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline())
        .expect("the expired watch fires while cancelling");
    expect_rewind_cancel(&effects, &pid);
    let agent = agent_ref(&app, id);
    assert_eq!(
        (true, "stuck", true),
        (
            agent.session.state.is_idle(),
            agent.prompt.text(),
            agent.pending_cancel_resend.is_none()
        ),
        "no response is coming; the abort ends the turn"
    );
}

fn unstamped_agent_chunk(session_id: &str, replay: bool) -> AcpClientMessage {
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let mut request = acp::SessionNotification::new(
        acp::SessionId::new(session_id),
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
            acp::TextContent::new("working"),
        ))),
    );
    if replay {
        request = request.meta(serde_json::json!({ "isReplay": true }).as_object().cloned());
    }
    AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
        request,
        response_tx: tx,
    })
}

fn external_roster_entry(session_id: &str) -> crate::app::roster::RosterEntry {
    crate::app::roster::RosterEntry {
        session_id: session_id.to_string(),
        title: Some("cursor-agent".into()),
        cwd: "/repo".into(),
        is_worktree: false,
        session_kind: Some("external".into()),
        model_id: None,
        yolo: false,
        activity: crate::app::roster::RosterActivity::Working,
        last_turn_summary: None,
        resident: true,
        last_change_unix_ms: 1,
        origin: crate::app::roster::RosterOrigin::default(),
    }
}

/// A native session ignores a live update that does not name the awaited prompt, so the fail-safe still aborts.
#[test]
fn native_unstamped_update_leaves_the_watch_armed_until_the_deadline() {
    let mut app = test_app_with_agent();
    let pid = send_and_arm(&mut app, "native");
    crate::app::acp_handler::handle(unstamped_agent_chunk("test-session", false), &mut app);
    let agent = agent_ref(&app, AgentId(0));
    assert!(
        agent.prompt_ack.is_some(),
        "an unstamped update is not an acknowledgment on a native session"
    );
    let effects = reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline())
        .expect("the watch still expires");
    expect_rewind_cancel(&effects, &pid);
}

/// An external session's first live update is the acknowledgment, even with no prompt id, so the deadline does not abort.
#[test]
fn external_unstamped_update_disarms_the_watch_before_the_deadline() {
    let mut app = test_app_with_agent();
    app.external_agent = true;
    send_and_arm(&mut app, "sleep 30");
    crate::app::acp_handler::handle(unstamped_agent_chunk("test-session", false), &mut app);
    let agent = agent_ref(&app, AgentId(0));
    assert_eq!(
        (None, true),
        (
            agent.prompt_ack.as_ref(),
            agent.session.state.is_turn_running()
        ),
        "the live update disarmed the watch and the turn is still running"
    );
    assert!(
        reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline()).is_none(),
        "past the hard deadline there is nothing left to abort"
    );
}

/// A replay of an external session is history, not proof the agent accepted this prompt.
#[test]
fn external_replay_update_does_not_disarm_the_watch() {
    let mut app = test_app_with_agent();
    app.external_agent = true;
    send_and_arm(&mut app, "sleep 30");
    crate::app::acp_handler::handle(unstamped_agent_chunk("test-session", true), &mut app);
    assert!(
        agent_ref(&app, AgentId(0)).prompt_ack.is_some(),
        "a replay must not acknowledge the live prompt"
    );
}

/// A leader client that did not pass `--agent-cmd` still treats a roster `sessionKind: external` session as external.
#[test]
fn leader_attached_external_session_unstamped_update_disarms_the_watch() {
    let mut app = test_app_with_agent();
    app.leader_mode = true;
    app.external_agent = false;
    app.leader_roster = vec![external_roster_entry("test-session")];
    send_and_arm(&mut app, "from the dashboard");
    crate::app::acp_handler::handle(unstamped_agent_chunk("test-session", false), &mut app);
    let agent = agent_ref(&app, AgentId(0));
    assert!(
        agent.prompt_ack.is_none() && agent.session.state.is_turn_running(),
        "the attached external session acknowledged on the first live update"
    );
    assert!(reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline()).is_none());
}

#[test]
fn late_prompt_response_after_the_fail_safe_leaves_the_pane_idle() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let pid = send_and_arm(&mut app, "restore me");
    reconcile_overdue_prompt_acks_at(&mut app, &DEADLINES, past_hard_deadline())
        .expect("the expired watch fires");
    let stale = dispatch(
        Action::TaskComplete(TaskResult::PromptResponse {
            agent_id: id,
            result: Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)
                .meta(serde_json::json!({ "promptId": pid }).as_object().cloned())),
            http_status: None,
            prompt_id: Some(pid),
        }),
        &mut app,
    );
    let agent = agent_ref(&app, id);
    assert!(stale.is_empty());
    assert_eq!(
        (true, "restore me", 1),
        (
            agent.session.state.is_idle(),
            agent.prompt.text(),
            agent.scrollback.len()
        ),
        "the discarded response paints no marker and touches no state"
    );
}
