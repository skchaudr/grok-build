//! Tests for sending a prompt through a parked wait, including after `/btw`.

use super::*;

use crate::app::agent::AgentState;
use crate::app::agent_view::test_fixtures::simulate_task_output_wait;
use crate::scrollback::block::RenderBlock;
use crate::views::btw_overlay::BtwOverlayState;
use agent_client_protocol as acp;

fn agent_ref(app: &AppView, id: AgentId) -> &AgentView {
    let Some(agent) = app.agents.get(&id) else {
        panic!("expected agent {id:?}");
    };
    agent
}

fn running_turn_app() -> AppView {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session.state = AgentState::TurnRunning;
    agent.session.current_prompt_id = Some("p1".into());
    agent.front_message_committed = true;
    app
}

fn sent_texts(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SendInterject { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn sent_interjects(effects: &[Effect]) -> Vec<(String, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SendInterject {
                text,
                interjection_id,
                ..
            } => Some((text.clone(), interjection_id.clone())),
            _ => None,
        })
        .collect()
}

fn interjection_texts(app: &AppView) -> Vec<String> {
    let agent = agent_ref(app, AgentId(0));
    (0..agent.scrollback.len())
        .filter_map(|idx| {
            let entry = agent.scrollback.entry(idx)?;
            match &entry.block {
                RenderBlock::UserPrompt(p) if p.is_interjection => Some(p.text.clone()),
                _ => None,
            }
        })
        .collect()
}

/// Forces the local drip-feed send path so a mid-turn Enter enqueues locally instead of handing the send to the server.
struct LocalFollowUp {
    previous: crate::appearance::FollowUpBehavior,
}
impl LocalFollowUp {
    fn enter(app: &mut AppView, behavior: crate::appearance::FollowUpBehavior) -> Self {
        app.leader_mode = false;
        let previous = crate::appearance::cache::load_follow_up_behavior();
        crate::appearance::cache::set_follow_up_behavior(behavior);
        Self { previous }
    }
    fn queue(app: &mut AppView) -> Self {
        Self::enter(app, crate::appearance::FollowUpBehavior::Queue)
    }
    fn steer(app: &mut AppView) -> Self {
        Self::enter(app, crate::appearance::FollowUpBehavior::Steer)
    }
}
impl Drop for LocalFollowUp {
    fn drop(&mut self) {
        crate::appearance::cache::set_follow_up_behavior(self.previous);
    }
}

/// Sending while parked must go through even when the last thing the user did was `/btw` (the overlay is still open).
#[test]
fn send_while_waiting_goes_through_when_btw_overlay_is_open() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "held");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        assert!(agent.is_parked_on_sendable_wait());
        agent.btw_state = Some(BtwOverlayState::done(
            "what were we doing".into(),
            "waiting on the task".into(),
        ));
        agent.btw_focused = true;
    }

    let effects = dispatch_send_prompt(&mut app, "keep going".into());

    assert_eq!(
        sent_texts(&effects),
        vec!["held".to_string(), "keep going".to_string()]
    );
    assert!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .is_empty()
    );
    assert!(
        agent_ref(&app, AgentId(0)).btw_state.is_some(),
        "releasing the send must not dismiss the /btw overlay",
    );
}

/// A prompt queued *before* `/btw` (a follow-up typed while the model was thinking) must stay queued when the answer lands.
/// The send path is what releases a message typed while parked; `/btw` completion must not flush the queue.
#[test]
fn btw_response_does_not_flush_an_unrelated_queued_prompt() {
    let mut app = running_turn_app();
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        agent.btw_state = Some(BtwOverlayState::Loading {
            question: "status?".into(),
        });
    }
    enqueue_local(&mut app, AgentId(0), "queued before btw");

    let effects = dispatch_task_result(
        TaskResult::BtwResponse {
            image_notice: None,
            skipped_image_numbers: Vec::new(),
            agent_id: AgentId(0),
            result: Ok("still waiting".into()),
            minimal_request_id: None,
        },
        &mut app,
    );

    assert!(
        sent_texts(&effects).is_empty(),
        "btw completion must not interject a pre-queued follow-up, got {effects:?}"
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .front()
            .map(|p| p.text.as_str()),
        Some("queued before btw")
    );
    assert!(
        matches!(
            agent_ref(&app, AgentId(0)).btw_state,
            Some(BtwOverlayState::Done { .. })
        ),
        "the overlay must still show the answer",
    );
}

#[test]
fn queue_mode_send_while_waiting_stays_queued() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::queue(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        assert!(agent.is_parked_on_sendable_wait());
    }

    let effects = dispatch_send_prompt(&mut app, "just typed".into());

    assert!(
        sent_texts(&effects).is_empty(),
        "Queue must not interject during a wait, got {effects:?}"
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["queued while thinking", "just typed"]
    );
}

/// Enter during a wait flushes earlier held follow-ups, then the message just typed.
#[test]
fn send_while_waiting_flushes_held_follow_ups_then_the_new_prompt() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        assert!(agent.is_parked_on_sendable_wait());
    }

    let effects = dispatch_send_prompt(&mut app, "just typed".into());

    assert_eq!(
        sent_texts(&effects),
        vec![
            "queued while thinking".to_string(),
            "just typed".to_string()
        ]
    );
    assert!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .is_empty()
    );
}

#[test]
fn queue_mode_send_while_waiting_with_empty_queue_stays_queued() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::queue(&mut app);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        assert!(agent.is_parked_on_sendable_wait());
    }

    let effects = dispatch_send_prompt(&mut app, "just typed".into());

    assert!(
        sent_texts(&effects).is_empty(),
        "Queue must not wait-interject even with an empty pile, got {effects:?}"
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["just typed"]
    );
}

#[test]
fn parked_wait_does_not_flush_held_follow_ups_in_queue_mode() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::queue(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert!(
        sent_texts(&effects).is_empty(),
        "Queue mode must keep follow-ups held across a wait, got {effects:?}"
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["queued while thinking"]
    );
}

#[test]
fn parked_wait_flushes_held_follow_ups_in_steer_mode() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    enqueue_local(&mut app, AgentId(0), "also queued");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        assert!(agent.is_parked_on_sendable_wait());
    }

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert_eq!(
        sent_texts(&effects),
        vec![
            "queued while thinking".to_string(),
            "also queued".to_string()
        ]
    );
    assert!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .is_empty(),
        "wait-start flush must empty the local held queue"
    );
}

#[test]
fn send_after_wait_flush_does_not_resend_earlier_follow_up() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }
    let flushed = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));
    assert_eq!(
        sent_texts(&flushed),
        vec!["queued while thinking".to_string()]
    );

    let effects = dispatch_send_prompt(&mut app, "just typed".into());

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::SendPrompt { text, .. } if text == "just typed"
        )),
        "the new message must send, got {effects:?}"
    );
    assert!(
        !effects.iter().any(|e| match e {
            Effect::SendInterject { text, .. } | Effect::SendPrompt { text, .. } => {
                text == "queued while thinking"
            }
            _ => false,
        }),
        "wait-start flush already sent the earlier follow-up, got {effects:?}"
    );
    assert!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .is_empty(),
        "the earlier follow-up must not remain to drain a second time, leftover {:?}",
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn flush_skips_unflushable_front_and_releases_later_prompts() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.enqueue_bash_command("ls".into());
        simulate_task_output_wait(agent, "task-1");
    }
    enqueue_local(&mut app, AgentId(0), "queued while thinking");

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert_eq!(
        sent_texts(&effects),
        vec!["queued while thinking".to_string()]
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["ls"]
    );
}

#[test]
fn flush_does_not_run_while_send_now_cancel_is_armed() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        agent.expect_send_now_cancel = Some("send-now-echo".into());
    }

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert!(sent_texts(&effects).is_empty(), "got {effects:?}");
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .front()
            .map(|p| p.text.as_str()),
        Some("queued while thinking")
    );
}

#[test]
fn flush_paints_identical_follow_ups_separately() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "ok");
    enqueue_local(&mut app, AgentId(0), "ok");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert_eq!(
        sent_texts(&effects),
        vec!["ok".to_string(), "ok".to_string()]
    );
    assert_eq!(
        interjection_texts(&app),
        vec!["ok".to_string(), "ok".to_string()]
    );
}

#[test]
fn failed_flush_drops_trailing_echoes_before_requeue() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    enqueue_local(&mut app, AgentId(0), "also queued");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }
    let flushed = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));
    assert_eq!(interjection_texts(&app).len(), 2);
    let remaining = sent_interjects(&flushed)
        .into_iter()
        .map(|(text, id)| {
            (
                text,
                id,
                Some(vec![acp::ContentBlock::Text(acp::TextContent::new(
                    "<skill>body</skill>",
                ))]),
            )
        })
        .collect();

    let _ = dispatch_task_result(
        TaskResult::InterjectFailed {
            agent_id: AgentId(0),
            error: "channel closed".into(),
            remaining,
        },
        &mut app,
    );

    assert!(
        interjection_texts(&app).is_empty(),
        "failed send must not leave optimistic echoes, leftover {:?}",
        interjection_texts(&app)
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["queued while thinking", "also queued"]
    );
    assert!(agent_ref(&app, AgentId(0)).self_interjection_ids.is_empty());
    assert!(
        agent_ref(&app, AgentId(0))
            .interjection_painted_blocks
            .is_empty()
    );
    assert!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .all(|p| p.wire_matches_display() && p.wire_blocks.is_none()),
        "failed image/text leftover must stay flushable"
    );
}

#[test]
fn failed_flush_drops_only_the_failed_identical_follow_up() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "ok");
    enqueue_local(&mut app, AgentId(0), "ok");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }
    let flushed = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));
    let sent = sent_interjects(&flushed);
    assert_eq!(sent.len(), 2);
    let Some((first_text, first_id)) = sent.first().cloned() else {
        panic!("expected a flushed interject");
    };

    let _ = dispatch_task_result(
        TaskResult::InterjectFailed {
            agent_id: AgentId(0),
            error: "channel closed".into(),
            remaining: vec![(first_text, first_id, None)],
        },
        &mut app,
    );

    assert_eq!(interjection_texts(&app), vec!["ok".to_string()]);
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["ok"]
    );
}

#[test]
fn flush_does_not_leapfrog_held_server_queue_front() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        agent.shared_queue = vec![crate::app::prompt_queue::QueueEntryWire {
            id: "server-first".into(),
            version: 1,
            owner: None,
            last_editor: None,
            kind: "prompt".into(),
            text: "from another client".into(),
            position: 0,
            combined_texts: None,
        }];
    }

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert!(sent_texts(&effects).is_empty(), "got {effects:?}");
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .front()
            .map(|p| p.text.as_str()),
        Some("queued while thinking")
    );
}

#[test]
fn flush_does_not_run_during_loading_replay() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
        agent.session.loading_replay = true;
    }

    let effects = super::super::queue::flush_held_local_queue_into_wait(&mut app, Some(AgentId(0)));

    assert!(sent_texts(&effects).is_empty(), "got {effects:?}");
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .front()
            .map(|p| p.text.as_str()),
        Some("queued while thinking")
    );
}

#[test]
fn interject_flushes_held_follow_ups() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }

    let effects = dispatch(
        Action::Interject {
            text: "just typed".into(),
            images: vec![],
        },
        &mut app,
    );

    assert_eq!(
        sent_texts(&effects),
        vec![
            "queued while thinking".to_string(),
            "just typed".to_string()
        ]
    );
    assert!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .is_empty()
    );
}

#[test]
fn send_now_does_not_interject_held_follow_ups_into_cancelled_turn() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);
    enqueue_local(&mut app, AgentId(0), "queued while thinking");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        simulate_task_output_wait(agent, "task-1");
    }

    let effects = dispatch(
        Action::SendPromptNow {
            text: "just typed".into(),
            images: vec![],
            image_notice: None,
        },
        &mut app,
    );

    assert!(
        sent_texts(&effects).is_empty(),
        "send-now must not flush into the dying turn, got {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SendPromptNow { .. })),
        "send-now must still fire, got {effects:?}"
    );
    assert_eq!(
        agent_ref(&app, AgentId(0))
            .session
            .pending_prompts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["queued while thinking"]
    );
}

/// A thinking turn (running, but not parked / watching) still queues.
#[test]
fn send_while_thinking_stays_queued() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::queue(&mut app);

    let effects = dispatch_send_prompt(&mut app, "later".into());

    assert!(
        sent_texts(&effects).is_empty(),
        "a thinking turn must not interject, got {effects:?}"
    );
    assert_eq!(agent_ref(&app, AgentId(0)).session.pending_prompts.len(), 1);
}

/// What Claude shows while a background subagent runs after its prompt turn: a synthetic wake over an Idle pane.
fn claude_wake_app(external_agent: bool) -> AppView {
    let mut app = test_app_with_agent();
    app.external_agent = external_agent;
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.claude_sdk_running = true;
    agent.running_wake_turn = Some(crate::app::agent_view::RunningWakeTurn {
        prompt_id: "claude-autonomous".into(),
        cancel_sent: false,
    });
    agent
        .subagent_sessions
        .insert("child-1".into(), make_test_subagent("child-1", "sa-1"));
    app
}

fn wake_image() -> crate::prompt_images::PastedImage {
    crate::prompt_images::PastedImage {
        element_id: xai_ratatui_textarea::ElementId::from_raw(0),
        display_number: 0,
        mime_type: "image/png".to_owned(),
        dimensions: Some((8, 8)),
        byte_len: 1,
        encoded_bytes: Some(vec![0].into()),
        source_path: None,
        staged_temp_path: None,
        session_image_path: None,
        preview: crate::prompt_images::PromptImagePreview::default(),
    }
}

fn sent_prompt_texts(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SendPrompt { text, .. } => Some(text.clone()),
            Effect::SendPromptBlocks { blocks, .. } => Some(match blocks.first() {
                Some(acp::ContentBlock::Text(t)) => t.text.clone(),
                _ => String::new(),
            }),
            _ => None,
        })
        .collect()
}

/// The reported hang: an image prompt sent while only a background subagent keeps the session busy sat in the local
/// queue ("Queued 1") and held every later prompt until the subagent ended. It must go out at once, on every host kind.
#[test]
fn image_prompt_during_wake_with_running_subagent_is_sent_without_waiting() {
    for external_agent in [true, false] {
        let mut app = claude_wake_app(external_agent);
        let _mode = LocalFollowUp::queue(&mut app);
        app.agents
            .get_mut(&AgentId(0))
            .unwrap()
            .prompt
            .insert_image(wake_image())
            .unwrap();

        let effects = dispatch(Action::SendPrompt("look at this".into()), &mut app);

        assert!(
            matches!(effects.as_slice(), [Effect::SendPromptBlocks { blocks, .. }]
                if blocks.len() >= 2),
            "external={external_agent}: expected one blocks send carrying the image, got {effects:?}"
        );
        let agent = agent_ref(&app, AgentId(0));
        assert!(agent.session.pending_prompts.is_empty());
        assert!(agent.session.state.is_idle(), "no fake local turn over a wake");
        assert!(agent.running_wake_turn.is_some(), "the subagent is still running");
        assert!(agent.subagent_sessions.values().any(|s| s.is_running()));

        // A text prompt right behind it is not held by the earlier one, and nothing is sent twice.
        let next = dispatch(Action::SendPrompt("then this".into()), &mut app);
        assert_eq!(sent_prompt_texts(&next), vec!["then this".to_string()]);
        assert!(flush_held_local_queue_into_wait(&mut app, None).is_empty());
    }
}

/// Rows queued while the wake runs leave in first-in first-out order, images or not, and an unsendable row in front
/// holds everything behind it instead of being overtaken.
#[test]
fn wake_release_keeps_order_and_never_overtakes_a_held_row() {
    let mut app = claude_wake_app(true);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.enqueue_prompt("/compact".into());
        let id = agent.session.next_queue_id;
        agent.session.next_queue_id += 1;
        agent.session.pending_prompts.push_back(crate::app::agent::QueuedPrompt {
            images: vec![wake_image()],
            ..crate::app::agent::QueuedPrompt::plain(
                id,
                "two with image",
                crate::app::agent::QueueEntryKind::Prompt,
            )
        });
    }
    // Make the front row a command so it cannot be sent into a wake.
    agent_front_to_command(&mut app);

    assert!(
        flush_held_local_queue_into_wait(&mut app, None).is_empty(),
        "a command in front holds the prompts behind it"
    );
    assert_eq!(agent_ref(&app, AgentId(0)).session.pending_prompts.len(), 2);

    // Without the blocker the rows go out oldest first.
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.pending_prompts.pop_front();
        agent.session.pending_prompts.push_front(
            crate::app::agent::QueuedPrompt::plain(
                90,
                "one",
                crate::app::agent::QueueEntryKind::Prompt,
            ),
        );
        let id = agent.session.next_queue_id;
        agent.session.next_queue_id += 1;
        agent.session.pending_prompts.push_back(
            crate::app::agent::QueuedPrompt::plain(
                id,
                "three",
                crate::app::agent::QueueEntryKind::Prompt,
            ),
        );
    }
    let effects = flush_held_local_queue_into_wait(&mut app, None);
    assert_eq!(
        sent_prompt_texts(&effects),
        vec!["one".to_string(), "two with image".to_string(), "three".to_string()]
    );
    assert!(agent_ref(&app, AgentId(0)).session.pending_prompts.is_empty());
    assert!(flush_held_local_queue_into_wait(&mut app, None).is_empty());
}

/// Scrollback as `(who, text)` rows: `agent` for assistant text, `user` for a prompt row.
fn order_rows(app: &AppView) -> Vec<(&'static str, String)> {
    let agent = agent_ref(app, AgentId(0));
    (0..agent.scrollback.len())
        .filter_map(|i| match &agent.scrollback.entry(i).unwrap().block {
            RenderBlock::AgentMessage(b) => Some(("agent", b.text())),
            RenderBlock::UserPrompt(b) => Some(("user", b.text.clone())),
            _ => None,
        })
        .collect()
}

fn stream_agent_text(app: &mut AppView, text: &str) {
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

/// The reported misorder: the wake turn is streaming text, Sab sends a prompt, and the agent's reply to it
/// was appended to the earlier assistant entry ABOVE the prompt. A reply always lands below the prompt it answers,
/// as its own entry, for a straight send and for a row released from the queue.
#[test]
fn reply_to_a_prompt_sent_during_a_wake_lands_below_the_prompt() {
    let mut app = claude_wake_app(true);
    let _mode = LocalFollowUp::queue(&mut app);
    stream_agent_text(&mut app, "wake text so far");

    let effects = dispatch(Action::SendPrompt("my question".into()), &mut app);
    assert_eq!(sent_prompt_texts(&effects), vec!["my question".to_string()]);
    stream_agent_text(&mut app, "the reply");

    assert_eq!(
        order_rows(&app),
        vec![
            ("agent", "wake text so far".to_string()),
            ("user", "my question".to_string()),
            ("agent", "the reply".to_string()),
        ]
    );

    // A row released from the local queue takes the same route.
    stream_agent_text(&mut app, " more");
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.pending_prompts.push_back(crate::app::agent::QueuedPrompt::plain(
            91,
            "queued question",
            crate::app::agent::QueueEntryKind::Prompt,
        ));
    }
    let released = flush_held_local_queue_into_wait(&mut app, None);
    assert_eq!(sent_prompt_texts(&released), vec!["queued question".to_string()]);
    stream_agent_text(&mut app, "second reply");
    assert_eq!(
        order_rows(&app),
        vec![
            ("agent", "wake text so far".to_string()),
            ("user", "my question".to_string()),
            ("agent", "the reply more".to_string()),
            ("user", "queued question".to_string()),
            ("agent", "second reply".to_string()),
        ]
    );
}

fn agent_front_to_command(app: &mut AppView) {
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session.pending_prompts.front_mut().unwrap().kind =
        crate::app::agent::QueueEntryKind::Command;
}

/// A wake whose cancel is already in flight, or a row being edited, is left alone.
#[test]
fn wake_release_skips_a_cancelled_wake() {
    let mut app = claude_wake_app(true);
    enqueue_local(&mut app, AgentId(0), "held");
    app.agents.get_mut(&AgentId(0)).unwrap().mark_wake_cancel_sent();
    assert!(flush_held_local_queue_into_wait(&mut app, None).is_empty());
    assert_eq!(agent_ref(&app, AgentId(0)).session.pending_prompts.len(), 1);
}

/// A real prompt turn is unchanged: text goes straight to the server queue when steering, an image row is still held for the turn.
#[test]
fn real_prompt_turn_keeps_its_rules() {
    let mut app = running_turn_app();
    let _mode = LocalFollowUp::steer(&mut app);

    let effects = dispatch(Action::SendPrompt("mid turn".into()), &mut app);
    assert_eq!(sent_prompt_texts(&effects), vec!["mid turn".to_string()]);

    app.agents
        .get_mut(&AgentId(0))
        .unwrap()
        .prompt
        .insert_image(wake_image())
        .unwrap();
    let effects = dispatch(Action::SendPrompt("with image".into()), &mut app);
    assert!(
        effects.iter().all(|e| !matches!(
            e,
            Effect::SendPrompt { .. } | Effect::SendPromptBlocks { .. } | Effect::SendInterject { .. }
        )),
        "image row waits for the real turn, got {effects:?}"
    );
    assert_eq!(agent_ref(&app, AgentId(0)).session.pending_prompts.len(), 1);
}
