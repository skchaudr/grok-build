//! Pi (non-Claude) external subagent child-view steer.

use super::*;
use crate::app::actions::{Action, Effect};
use crate::app::agent::AgentState;
use crate::app::agent_view::AgentView;
use crate::app::dispatch::prompt::{
    CLAUDE_CHILD_MID_RUN_STEER_NOTICE, PI_CHILD_FINISHED_STEER_NOTICE,
};
use crate::app::dispatch::router::dispatch;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::state::ScrollbackState;

fn pi_child_overlay(
    app: &mut AppView,
    child_sid: &str,
    running: bool,
) -> (AgentId, acp::SessionId) {
    let parent_id = AgentId(0);
    let parent_sid = app
        .agents
        .get(&parent_id)
        .unwrap()
        .session
        .session_id
        .clone()
        .expect("parent session");
    let child_session = make_test_agent_session(app, AgentId(1), child_sid);
    let mut child = AgentView::new(child_session, ScrollbackState::new());
    if running {
        child.session.state = AgentState::TurnRunning;
    }
    let mut info = make_test_subagent(child_sid, "sa-1");
    if !running {
        info.set_finished_for_test(true);
    }
    {
        let parent = app.agents.get_mut(&parent_id).unwrap();
        parent
            .subagent_sessions
            .insert(child_sid.into(), info);
        parent.insert_test_child(child_sid.to_string(), Box::new(child));
        parent.active_subagent = Some(child_sid.to_string());
        parent.session.state = AgentState::TurnRunning;
        parent.session.enqueue_prompt("must stay queued".into());
    }
    app.external_agent = true;
    app.external_agent_label = Some("pi".into());
    (parent_id, parent_sid)
}

#[test]
fn pi_child_view_enter_targets_child_session_and_leaves_parent_queue() {
    let mut app = test_app_with_agent();
    let child_sid = "pi-child-steer-running";
    let (parent_id, parent_sid) = pi_child_overlay(&mut app, child_sid, true);

    let parent_before = app.agents.get(&parent_id).unwrap();
    let parent_turn_running = parent_before.session.state.is_turn_running();
    let parent_queue_len = parent_before.session.queue_len();
    let parent_current_pid = parent_before.session.current_prompt_id.clone();

    let effects = dispatch(Action::SendPrompt("steer the child".into()), &mut app);

    assert!(
        matches!(
            effects.as_slice(),
            [Effect::SendChildPrompt {
                child_session_id,
                text,
                ..
            }] if child_session_id.0.as_ref() == child_sid && text == "steer the child"
        ),
        "steer must emit SendChildPrompt for the child session, got {effects:?}"
    );
    let parent = app.agents.get(&parent_id).unwrap();
    assert_eq!(
        parent.session.state.is_turn_running(),
        parent_turn_running
    );
    assert_eq!(parent.session.queue_len(), parent_queue_len);
    assert_eq!(parent.session.current_prompt_id, parent_current_pid);
    assert!(
        parent.session.pending_prompts.iter().any(|p| p.text == "must stay queued"),
        "parent queue must be untouched"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::SendPrompt { session_id, .. } if session_id == &parent_sid)),
        "must not send on the parent session"
    );
    let child = parent.subagent_views.get(child_sid).unwrap();
    assert!(
        child.scrollback.len() >= 1
            && matches!(
                child.scrollback.entry(0).map(|e| &e.block),
                Some(RenderBlock::UserPrompt(_))
            ),
        "user text must land on the child scrollback"
    );
}

#[test]
fn pi_child_view_finished_subagent_shows_notice_and_sends_nothing() {
    let mut app = test_app_with_agent();
    let child_sid = "pi-child-steer-finished";
    let (_parent_id, _parent_sid) = pi_child_overlay(&mut app, child_sid, false);

    let effects = dispatch(Action::SendPrompt("too late".into()), &mut app);

    assert!(effects.is_empty(), "finished child must not send, got {effects:?}");
    let parent = app.agents.get(&AgentId(0)).unwrap();
    let child = parent.subagent_views.get(child_sid).unwrap();
    let saw_notice = child.toast.as_ref().is_some_and(|(m, _)| m.contains("finished"))
        || (0..child.scrollback.len()).any(|idx| {
            matches!(
                child.scrollback.entry(idx).map(|e| &e.block),
                Some(RenderBlock::System(b)) if b.text.contains("finished")
            )
        });
    assert!(saw_notice, "expected {PI_CHILD_FINISHED_STEER_NOTICE}");
}

#[test]
fn pi_child_steer_ack_does_not_finish_parent_turn() {
    let mut app = test_app_with_agent();
    let child_sid = "pi-child-ack-parent";
    let (parent_id, _) = pi_child_overlay(&mut app, child_sid, true);
    let parent_pid = "parent-running-pid".to_string();
    {
        let parent = app.agents.get_mut(&parent_id).unwrap();
        parent.session.current_prompt_id = Some(parent_pid.clone());
        assert!(parent.session.state.is_turn_running());
    }

    let steer = dispatch(Action::SendPrompt("steer the child".into()), &mut app);
    assert!(
        matches!(steer.as_slice(), [Effect::SendChildPrompt { .. }]),
        "expected SendChildPrompt, got {steer:?}"
    );

    let ack = dispatch(
        Action::TaskComplete(crate::app::actions::TaskResult::ChildPromptAck {
            parent_id,
            child_session_id: child_sid.into(),
            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        }),
        &mut app,
    );
    assert!(ack.is_empty(), "child ack must not dispatch follow-on effects");

    let parent = app.agents.get(&parent_id).unwrap();
    assert!(
        parent.session.state.is_turn_running(),
        "parent turn must stay running after child steer ack"
    );
    assert_eq!(
        parent.session.current_prompt_id.as_deref(),
        Some(parent_pid.as_str()),
        "parent current_prompt_id must be unchanged"
    );
}

#[test]
fn claude_child_mid_run_steer_refusal_unchanged() {
    let mut app = test_app_with_agent();
    let child_sid = "claude-child-steer-refused";
    let (_parent_id, _) = pi_child_overlay(&mut app, child_sid, true);
    app.external_agent_label = Some("claude".into());

    let effects = dispatch(Action::SendPrompt("nope".into()), &mut app);

    assert!(effects.is_empty(), "Claude child steer must refuse, got {effects:?}");
    let parent = app.agents.get(&AgentId(0)).unwrap();
    let notice = parent
        .toast
        .as_ref()
        .map(|(m, _)| m.as_str())
        .unwrap_or("");
    assert!(
        notice.contains("mid-run") || notice.contains("can't take"),
        "expected {CLAUDE_CHILD_MID_RUN_STEER_NOTICE}, got {notice:?}"
    );
}

#[test]
fn native_agent_child_view_still_routes_through_parent_session() {
    let mut app = test_app_with_agent();
    let parent_id = AgentId(0);
    let parent_sid = app
        .agents
        .get(&parent_id)
        .unwrap()
        .session
        .session_id
        .clone()
        .expect("parent session");
    let child_sid = "native-child-not-external";
    let child = AgentView::new(
        make_test_agent_session(&app, AgentId(1), child_sid),
        ScrollbackState::new(),
    );
    {
        let parent = app.agents.get_mut(&parent_id).unwrap();
        parent.insert_test_child(child_sid.to_string(), Box::new(child));
        parent.active_subagent = Some(child_sid.to_string());
    }
    app.external_agent = false;

    let effects = dispatch(Action::SendPrompt("parent turn".into()), &mut app);

    assert!(
        matches!(
            effects.as_slice(),
            [Effect::SendPrompt { session_id, text, .. }]
                if session_id == &parent_sid && text == "parent turn"
        ),
        "native child overlay must keep parent routing, got {effects:?}"
    );
}
