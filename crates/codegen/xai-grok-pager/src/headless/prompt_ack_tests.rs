use super::*;
use pretty_assertions::assert_eq;

#[test]
fn headless_ack_signal_classifies_messages() {
    let sid = acp::SessionId::new("sess-1");
    let ext = |method: &str, params: serde_json::Value| {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        AcpClientMessageBox::ExtNotification(xai_acp_lib::AcpArgsBox {
            request: Box::new(acp::ExtNotification::new(
                method,
                serde_json::value::to_raw_value(&params)
                    .expect("serialize")
                    .into(),
            )),
            response_tx: tx,
        })
    };
    let queue_changed = ext(
        xai_grok_shell::session::prompt_queue::QUEUE_CHANGED_METHOD,
        serde_json::json!({ "sessionId": "sess-1", "entries": [], "runningPromptId": "p1" }),
    );
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let update = AcpClientMessageBox::SessionNotification(xai_acp_lib::AcpArgsBox {
        request: Box::new(
            acp::SessionNotification::new(
                sid.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new("hi")),
                )),
            )
            .meta(serde_json::json!({ "promptId": "p1" }).as_object().cloned()),
        ),
        response_tx: tx,
    });
    let unrelated = ext("x.ai/models/update", serde_json::json!({}));
    assert_eq!(
        [
            Some(AckSignal::QueueChanged),
            Some(AckSignal::SessionUpdate),
            None
        ],
        [&queue_changed, &update, &unrelated]
            .map(|msg| headless_ack_signal(msg, &sid, "p1", false))
    );
}

#[test]
fn headless_external_unstamped_update_acks_and_native_does_not() {
    let sid = acp::SessionId::new("sess-1");
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let update = AcpClientMessageBox::SessionNotification(xai_acp_lib::AcpArgsBox {
        request: Box::new(acp::SessionNotification::new(
            sid.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("working"),
            ))),
        )),
        response_tx: tx,
    });
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let replay = AcpClientMessageBox::SessionNotification(xai_acp_lib::AcpArgsBox {
        request: Box::new(
            acp::SessionNotification::new(
                sid.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new("old")),
                )),
            )
            .meta(serde_json::json!({ "isReplay": true }).as_object().cloned()),
        ),
        response_tx: tx,
    });
    assert_eq!(
        None,
        headless_ack_signal(&update, &sid, "p1", false),
        "a native headless turn still requires the prompt id"
    );
    assert_eq!(
        Some(AckSignal::SessionUpdate),
        headless_ack_signal(&update, &sid, "p1", true),
        "an external headless turn accepts the first live update"
    );
    assert_eq!(
        None,
        headless_ack_signal(&replay, &sid, "p1", true),
        "a replay is not an acknowledgment"
    );
}
