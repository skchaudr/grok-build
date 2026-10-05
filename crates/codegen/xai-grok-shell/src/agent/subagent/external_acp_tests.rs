use super::*;
use crate::test_support::lsp_runtime::test_gateway_with_receiver;

fn fake_config(script: &str) -> ExternalAcpDefinition {
    ExternalAcpDefinition {
        argv: vec!["python3".into(), "-u".into(), "-c".into(), script.into()],
        machine: "local".into(),
        harness: "fake".into(),
        identity: "fake-worker".into(),
        model: None,
        cwd: None,
    }
}

const FAKE: &str = r#"
import json, sys
for line in sys.stdin:
    r=json.loads(line); m=r.get('method'); p=r.get('params', {})
    if m=='initialize': result={'agentCapabilities': {'loadSession': True}}
    elif m=='session/new': result={'sessionId':'external-123'}
    elif m=='session/load': result={}
    elif m=='session/prompt':
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'external-123','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'done'}}}}), flush=True)
        result={'stopReason':'end_turn'}
    else: continue
    print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}), flush=True)
"#;

#[tokio::test]
async fn fake_acp_initialize_new_prompt_and_load_preserve_identity() {
    let (gateway, mut rx) = test_gateway_with_receiver();
    let token = CancellationToken::new();
    let mut transport =
        ExternalTransport::spawn(&fake_config(FAKE), "native-child", gateway, token).unwrap();
    let initialized = transport
        .rpc(
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        )
        .await
        .unwrap();
    assert_eq!(initialized["agentCapabilities"]["loadSession"], true);
    let created = transport
        .rpc("session/new", json!({"cwd":"/tmp","mcpServers":[]}))
        .await
        .unwrap();
    transport.session_id = Some(created["sessionId"].as_str().unwrap().into());
    transport
        .rpc(
            "session/load",
            json!({"sessionId":"external-123","cwd":"/tmp","mcpServers":[]}),
        )
        .await
        .unwrap();
    transport
        .rpc(
            "session/prompt",
            json!({"sessionId":"external-123","prompt":[{"type":"text","text":"work"}]}),
        )
        .await
        .unwrap();
    assert_eq!(transport.output, "done");
    match rx.try_recv().unwrap() {
        xai_acp_lib::AcpClientMessage::SessionNotification(args) => {
            assert_eq!(args.request.session_id.to_string(), "native-child")
        }
        other => panic!("unexpected gateway message: {other:?}"),
    }
    transport.shutdown().await;
    assert!(transport.child.try_wait().unwrap().is_some());
}

#[tokio::test]
async fn fake_acp_cancel_interrupts_unanswered_request_and_reaps_process() {
    let (gateway, _rx) = test_gateway_with_receiver();
    let token = CancellationToken::new();
    let mut transport = ExternalTransport::spawn(
        &fake_config("import time; time.sleep(60)"),
        "child",
        gateway,
        token.clone(),
    )
    .unwrap();
    token.cancel();
    let error = transport.rpc("initialize", json!({})).await.unwrap_err();
    assert!(error.contains("cancelled"));
    transport.shutdown().await;
    assert!(transport.child.try_wait().unwrap().is_some());
}

#[tokio::test]
async fn fake_acp_permission_is_visible_and_uses_native_session_id() {
    let script = r#"
import json, sys
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':'permission-1','method':'session/request_permission','params':{'sessionId':'external-123','toolCall':{'toolCallId':'tool-1','title':'write file','status':'pending'},'options':[]}}), flush=True)
answer=json.loads(sys.stdin.readline())
assert answer['id']=='permission-1' and answer['result']['outcome']['outcome']=='cancelled'
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'stopReason':'end_turn'}}), flush=True)
"#;
    let (gateway, mut rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
        "native-child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    transport.session_id = Some("external-123".into());
    let answer = async {
        match rx.recv().await.unwrap() {
            xai_acp_lib::AcpClientMessage::RequestPermission(args) => {
                assert_eq!(args.request.session_id.to_string(), "native-child");
                args.response_tx
                    .send(Ok(serde_json::from_value(
                        json!({"outcome":{"outcome":"cancelled"}}),
                    )
                    .unwrap()))
                    .unwrap();
            }
            other => panic!("unexpected gateway message: {other:?}"),
        }
    };
    let (result, ()) = tokio::join!(transport.rpc("session/prompt", json!({})), answer);
    assert!(result.is_ok(), "{result:?}");
    transport.shutdown().await;
}

#[tokio::test]
async fn fake_acp_unknown_reverse_request_receives_visible_protocol_error() {
    let script = r#"
import json, sys
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':99,'method':'fs/read_text_file','params':{'sessionId':'external-123','path':'/tmp/nope'}}), flush=True)
answer=json.loads(sys.stdin.readline()); assert answer['error']['code']==-32601
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{}}), flush=True)
"#;
    let (gateway, _rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
        "child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    transport.session_id = Some("external-123".into());
    transport.rpc("session/prompt", json!({})).await.unwrap();
    transport.shutdown().await;
}

#[test]
fn external_definition_roundtrips_and_native_default_is_unchanged() {
    let definition = xai_grok_agent::config::AgentDefinition::parse("---\nname: fake\ndescription: worker\nexternalAcp:\n  argv: [python3, '-u']\n  machine: local\n  harness: fake\n  identity: worker-1\n---\nDo work.").unwrap();
    assert_eq!(definition.external_acp.unwrap().identity, "worker-1");
    assert!(
        xai_grok_agent::config::AgentDefinition::general_purpose()
            .external_acp
            .is_none()
    );
}

#[tokio::test]
async fn fake_acp_wrong_session_update_fails_instead_of_forwarding() {
    let script = r#"
import json,sys
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'other-worker','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'wrong'}}}}),flush=True)
"#;
    let (gateway, mut rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
        "child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    transport.session_id = Some("external-123".into());
    assert!(
        transport
            .rpc("session/prompt", json!({}))
            .await
            .unwrap_err()
            .contains("identity mismatch")
    );
    assert!(rx.try_recv().is_err());
    transport.shutdown().await;
}

#[tokio::test]
async fn fake_acp_process_exit_is_a_visible_failure() {
    let (gateway, _rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config("pass"),
        "child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    assert!(
        transport
            .rpc("initialize", json!({}))
            .await
            .unwrap_err()
            .contains("closed")
    );
    transport.shutdown().await;
}
