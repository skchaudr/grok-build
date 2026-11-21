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

fn say(app: &mut AppView, text: &str) {
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let chunk = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
        acp::ContentBlock::Text(acp::TextContent::new(text.to_string())),
    ));
    agent.session.handle_update(
        chunk,
        &crate::acp::meta::NotificationMeta::default(),
        &mut agent.scrollback,
    );
}

fn agent_texts(app: &AppView) -> Vec<String> {
    let agent = test_agent(app, AgentId(0));
    (0..agent.scrollback.len())
        .filter_map(|i| match &agent.scrollback.entry(i).unwrap().block {
            crate::scrollback::RenderBlock::AgentMessage(b) => Some(b.text()),
            _ => None,
        })
        .collect()
}

/// The reported glue: text after a background task's follow-up cycle was appended to the reply before it ("begun1. Cats ...").
/// A state change is a new assistant message; a repeat of the same state is not.
#[test]
fn a_new_sdk_cycle_starts_a_new_assistant_entry() {
    let mut app = make_app_with_agent("sess-claude");
    handle_ext_notification(&state("sess-claude", "running"), &mut app);
    say(&mut app, "begun");
    say(&mut app, " still");
    handle_ext_notification(&state("sess-claude", "running"), &mut app);
    say(&mut app, " going");
    assert_eq!(agent_texts(&app), vec!["begun still going".to_string()]);

    handle_ext_notification(&state("sess-claude", "idle"), &mut app);
    handle_ext_notification(&state("sess-claude", "running"), &mut app);
    say(&mut app, "1. Cats");
    assert_eq!(agent_texts(&app), vec!["begun still going".to_string(), "1. Cats".to_string()]);
}
