//! `_claude/sdkMessage` session state from claude-agent-acp drives a wake turn while Claude works after a prompt's result.
use super::*;
use std::time::{Duration, Instant};

fn state(session: &str, state: &str) -> acp::ExtNotification {
    let params = serde_json::json!({
        "sessionId": session,
        "message": { "type": "system", "subtype": "session_state_changed", "state": state },
    });
    acp::ExtNotification::new(
        "claude/sdkMessage",
        serde_json::value::to_raw_value(&params).unwrap().into(),
    )
}

#[test]
fn running_while_idle_becomes_a_wake_turn_after_the_grace_and_idle_clears_it() {
    let mut app = make_app_with_agent("sess-claude");
    assert!(handle_ext_notification(&state("sess-claude", "running"), &mut app));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    assert!(agent.claude_sdk_running);
    // Inside the grace a normal turn end must not flash the spinner.
    assert!(agent.running_wake_turn.is_none());

    agent.claude_quiet_since = Some(Instant::now() - Duration::from_secs(1));
    assert!(agent.reconcile_claude_working());
    assert!(agent.wake_turn_active());

    assert!(handle_ext_notification(&state("sess-claude", "idle"), &mut app));
    let agent = test_agent(&app, AgentId(0));
    assert!(!agent.claude_sdk_running);
    assert!(agent.running_wake_turn.is_none());
}

#[test]
fn other_sdk_messages_and_unknown_sessions_are_ignored() {
    let mut app = make_app_with_agent("sess-claude");
    let other = acp::ExtNotification::new(
        "claude/sdkMessage",
        serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": "sess-claude",
            "message": { "type": "assistant" },
        }))
        .unwrap()
        .into(),
    );
    assert!(!handle_ext_notification(&other, &mut app));
    assert!(!handle_ext_notification(&state("nope", "running"), &mut app));
    assert!(!test_agent(&app, AgentId(0)).claude_sdk_running);
}
