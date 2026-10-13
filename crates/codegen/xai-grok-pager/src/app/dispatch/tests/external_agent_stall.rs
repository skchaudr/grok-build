use super::{end_turn, test_app_with_agent};
use crate::app::actions::{Action, Effect};
use crate::app::agent::AgentId;
use crate::app::dispatch::router::dispatch;

#[test]
fn test_external_agent_queue_drain_stall() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.external_agent = true;

    // Simulate running a turn
    dispatch(Action::SendPrompt("first prompt".into()), &mut app);

    // Queue two prompts
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.enqueue_prompt("queued one".into());
    agent.session.enqueue_prompt("queued two".into());

    // Send now
    dispatch(Action::SendPromptNow {
        text: "send-now-echo".into(),
        images: vec![],
        image_notice: None,
    }, &mut app);

    let agent = app.agents.get_mut(&id).unwrap();

    // The SendPromptNow dispatched above uses dispatch_external_steer and enqueues a steering prompt.
    // Therefore expectation isn't armed since we're external
    assert!(agent.expect_send_now_cancel.is_none());
    assert!(agent.shared_queue.is_empty());

    // Simulate something erroneously arming the send-now expectation or pushing a shared queue
    // to verify the defensive clear works.
    agent.expect_send_now_cancel = Some("stale-expectation".into());
    agent.shared_queue.push(crate::app::prompt_queue::QueueEntryWire {
        id: "stale-echo".into(),
        version: 0,
        owner: None,
        last_editor: None,
        kind: "prompt".into(),
        text: "stale text".into(),
        position: 0,
        combined_texts: None,
    });

    // Complete the running turn
    let effects = dispatch(end_turn(), &mut app);

    // After the turn ends, the queue should drain the next prompt
    let agent = app.agents.get(&id).unwrap();
    assert_eq!(
        agent.session.pending_prompts.len(),
        2, // The two previously queued ones + the external steer prompt = 3 total. 1 drains, 2 left.
        "queue should drain one prompt and leave two"
    );
}
