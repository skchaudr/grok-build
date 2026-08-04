use std::time::Duration;

#[tokio::test]
async fn test_acp_subprocess() {
    // Create a mock script
    let script = r#"#!/usr/bin/env python3
import sys, json

def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()

prompt_id = None

for line in sys.stdin:
    if not line.strip(): continue
    req = json.loads(line)

    if "method" in req:
        if req["method"] == "initialize":
            send({"jsonrpc": "2.0", "id": req["id"], "result": {"capabilities": {}}})
        elif req["method"] == "session/new":
            send({"jsonrpc": "2.0", "id": req["id"], "result": {"sessionId": "test-session"}})
        elif req["method"] == "session/prompt":
            prompt_id = req["id"]
            # send update
            send({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "test-session",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "hello"}
                    }
                }
            })
            # send permission
            send({
                "jsonrpc": "2.0",
                "id": 999,
                "method": "session/request_permission",
                "params": {
                    "sessionId": "test-session",
                    "toolCall": {
                        "toolCallId": "tool-1",
                        "title": "Run test command",
                        "kind": "execute",
                        "status": "pending"
                    },
                    "options": [
                        {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
                        {"optionId": "reject-once", "name": "Reject once", "kind": "reject_once"}
                    ]
                }
            })
        else:
            send({"jsonrpc": "2.0", "id": req.get("id"), "result": {}})
    elif "result" in req:
        # Response to our permission request
        if prompt_id is not None:
            send({"jsonrpc": "2.0", "id": prompt_id, "result": {"message": {"type": "text", "text": "Done"}}})
"#;

    let temp_dir = tempfile::tempdir().unwrap();
    let script_path = temp_dir.path().join("mock_agent.py");
    std::fs::write(&script_path, script).unwrap();

    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut std::fs::File::open(&script_path).unwrap().metadata().unwrap().permissions(), 0o755);

    // Setup GROK_ACP_AGENT_CMD
    std::env::set_var("GROK_ACP_AGENT_CMD", format!("python3 {}", script_path.display()));

    let cancel = tokio_util::sync::CancellationToken::new();
    let config = xai_grok_shell::agent::config::Config::default();

    let spawned = xai_grok_pager::acp::spawn::spawn_grok_shell(config, &cancel, None).await.expect("Failed to spawn subprocess agent");

    // We can now interact with spawned.channel
    use agent_client_protocol as acp;
    use xai_acp_lib::{acp_send, AcpClientMessage};

    // Send initialize
    let init_res = acp_send(acp::InitializeRequest::new(acp::ClientInfo::new("test", "1.0")), &spawned.channel.tx).await.unwrap();
    assert!(init_res.capabilities.is_some());

    // Send session/new
    let new_res = acp_send(acp::NewSessionRequest::new(), &spawned.channel.tx).await.unwrap();
    assert_eq!(new_res.session_id.as_str(), "test-session");

    // Send session/prompt
    let mut rx = spawned.channel.rx;

    let prompt_fut = acp_send(acp::PromptRequest::new(acp::SessionId::new("test-session"), acp::PromptMessage::new(acp::ContentBlock::Text(acp::TextContent::new("hi")))), &spawned.channel.tx);

    // Listen for events
    let mut got_update = false;
    let mut got_perm = false;

    // We can poll in a separate task or just interleave
    let (tx_done, rx_done) = tokio::sync::oneshot::channel();

    let channel_tx = spawned.channel.tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                AcpClientMessage::SessionNotification(args) => {
                    if let acp::SessionUpdate::AgentMessageChunk(chunk) = &args.request.update {
                        if let acp::ContentBlock::Text(text) = &chunk.content {
                            assert_eq!(text.text, "hello");
                            got_update = true;
                        }
                    }
                }
                AcpClientMessage::RequestPermission(args) => {
                    assert_eq!(args.request.session_id.as_str(), "test-session");
                    got_perm = true;
                    // send response
                    let _ = args.response_tx.send(Ok(acp::RequestPermissionResponse::new(
                        acp::PermissionDecision::new(acp::PermissionOptionId::new("allow-once".into()))
                    )));
                }
                _ => {}
            }
            if got_update && got_perm {
                break;
            }
        }
        tx_done.send(()).unwrap();
    });

    // Wait for the prompt request to complete (it will complete when Python sends the final response to prompt)
    let prompt_res = prompt_fut.await.unwrap();

    // Ensure we got both
    rx_done.await.unwrap();

    assert!(prompt_res.message.is_some());

    cancel.cancel();
}
