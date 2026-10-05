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
    transport.publish_updates().unwrap();
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
    transport.publish_updates().unwrap();
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

#[tokio::test]
async fn config_model_setter_verifies_selection_without_legacy_call() {
    let script = r#"
import json,sys
r=json.loads(sys.stdin.readline()); assert r['method']=='session/set_config_option'
assert r['params']['configId']=='model' and r['params']['value']=='claude-test'
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'configOptions':[{'id':'model','currentValue':'claude-test'}]}}),flush=True)
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
    transport
        .select_model("external-123", "/remote", "claude-test")
        .await
        .unwrap();
    transport.shutdown().await;
}

#[tokio::test]
async fn model_setter_error_other_than_method_not_found_never_falls_back() {
    let script = r#"
import json,sys
r=json.loads(sys.stdin.readline()); assert r['method']=='session/set_config_option'
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':{'code':-32602,'message':'invalid model'}}),flush=True)
"#;
    let (gateway, _rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
        "child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    assert!(
        transport
            .select_model("external-123", "/remote", "bad-model")
            .await
            .unwrap_err()
            .contains("invalid model")
    );
    assert_eq!(transport.next_id, 1);
    transport.shutdown().await;
}

#[tokio::test]
async fn legacy_model_fallback_requires_method_not_found_and_actual_picker_confirmation() {
    let script = r#"
import json,sys
for line in sys.stdin:
 r=json.loads(line); m=r['method']
 if m=='session/set_config_option': response={'error':{'code':-32601,'message':'method not found'}}
 elif m=='session/set_model': response={'result':{}}
 elif m=='session/load': response={'result':{'models':{'currentModelId':'legacy-test'}}}
 else: raise Exception(m)
 response.update({'jsonrpc':'2.0','id':r['id']});print(json.dumps(response),flush=True)
"#;
    let (gateway, _rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
        "child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    transport.picker = json!({"models":{"availableModels":[{"modelId":"legacy-test"}]}});
    transport
        .select_model("external-123", "/remote", "legacy-test")
        .await
        .unwrap();
    assert_eq!(transport.next_id, 3);
    transport.shutdown().await;
}

#[tokio::test]
async fn load_replay_is_buffered_until_spawn_publication_and_not_result_output() {
    let script = r#"
import json,sys
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'external-123','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'old history'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{}}),flush=True)
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
    transport.rpc("session/load", json!({})).await.unwrap();
    assert!(rx.try_recv().is_err());
    transport.publish_updates().unwrap();
    assert!(rx.try_recv().is_ok());
    assert!(transport.output.is_empty());
    transport.shutdown().await;
}

#[test]
fn external_queue_persists_rows_and_refuses_closed_admission() {
    let dir = tempfile::tempdir().unwrap();
    let mut queue = ExternalQueue {
        path: dir.path().join("queue.json"),
        rows: vec![],
        accepting: true,
    };
    queue
        .insert("message-1".into(), "follow up".into())
        .unwrap();
    let rows: Vec<QueuedMessage> =
        serde_json::from_slice(&std::fs::read(&queue.path).unwrap()).unwrap();
    assert_eq!(rows[0].text, "follow up");
    assert!(!rows[0].consumed);
    queue.accepting = false;
    assert!(
        queue
            .insert("message-2".into(), "not accepted".into())
            .is_err()
    );
}

#[tokio::test]
async fn unverified_config_model_success_is_rejected() {
    let script = r#"
import json,sys
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'configOptions':[{'id':'model','currentValue':'other-model'}]}}),flush=True)
"#;
    let (gateway, _rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
        "child",
        gateway,
        CancellationToken::new(),
    )
    .unwrap();
    assert!(
        transport
            .select_model("external-123", "/remote", "requested")
            .await
            .unwrap_err()
            .contains("did not verify")
    );
    assert_eq!(transport.next_id, 1);
    transport.shutdown().await;
}

#[test]
fn external_definition_rejects_unknown_backend_fields() {
    assert!(xai_grok_agent::config::AgentDefinition::parse("---\nname: fake\ndescription: worker\nexternalAcp:\n  argv: [fake]\n  machine: local\n  harness: fake\n  identity: worker\n  silentlyIgnoredPolicy: true\n---\n").is_err());
}

#[tokio::test]
async fn bootstrap_permission_returns_protocol_error_without_hidden_ui() {
    let script = r#"
import json,sys
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':99,'method':'session/request_permission','params':{'sessionId':'external-123','toolCall':{'toolCallId':'t','title':'write','status':'pending'},'options':[]}}),flush=True)
answer=json.loads(sys.stdin.readline());assert answer['error']['code']==-32000
"#;
    let (gateway, mut rx) = test_gateway_with_receiver();
    let mut transport = ExternalTransport::spawn(
        &fake_config(script),
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
            .contains("bootstrap permission")
    );
    assert!(rx.try_recv().is_err());
    transport.shutdown().await;
}

#[test]
fn inherited_web_and_todo_policies_fail_closed_unless_explicitly_disabled() {
    let mut ctx = crate::test_support::lsp_runtime::ctx_with_toggle(Default::default());
    ctx.todo_gate = true;
    assert!(inherited_policy_error(&ctx).unwrap().contains("todo_gate"));
    ctx.todo_gate = false;
    ctx.disable_web_search = true;
    assert!(
        inherited_policy_error(&ctx)
            .unwrap()
            .contains("disable_web_search")
    );
    ctx.disable_web_search = false;
    assert!(inherited_policy_error(&ctx).is_none());
}

#[tokio::test]
async fn external_session_lease_is_exclusive_until_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let config = fake_config("import time; time.sleep(60)");
    let (gateway, _rx) = test_gateway_with_receiver();
    let mut first =
        ExternalTransport::spawn(&config, "first", gateway.clone(), CancellationToken::new())
            .unwrap();
    let mut second =
        ExternalTransport::spawn(&config, "second", gateway, CancellationToken::new()).unwrap();
    first
        .acquire_session_lease(&config, "same-session", dir.path())
        .unwrap();
    assert!(
        second
            .acquire_session_lease(&config, "same-session", dir.path())
            .unwrap_err()
            .contains("already leased")
    );
    first.shutdown().await;
    second
        .acquire_session_lease(&config, "same-session", dir.path())
        .unwrap();
    second.shutdown().await;
}

#[test]
fn deferred_queue_rejects_active_admission_but_accepts_transaction_row() {
    let dir = tempfile::tempdir().unwrap();
    let mut queue = ExternalQueue {
        path: dir.path().join("queue.json"),
        rows: vec![],
        accepting: false,
    };
    assert!(
        queue
            .insert("active".into(), "not admitted".into())
            .is_err()
    );
    queue
        .insert_protected("initial-wake".into(), "transaction message".into())
        .unwrap();
    assert!(!queue.accepting);
    queue.accepting = true;
    queue
        .insert("active".into(), "now admitted".into())
        .unwrap();
    assert_eq!(queue.rows.len(), 2);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn external_transport_cleanup_kills_local_descendant_process_group() {
    let script = r#"
import json,sys,subprocess,time
child=subprocess.Popen(['sleep','60'])
r=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'descendantPid':child.pid}}),flush=True)
time.sleep(60)
"#;
    let (gateway, _rx) = test_gateway_with_receiver();
    let scope = xai_tty_utils::ProcessScope::new();
    let mut transport = ExternalTransport::spawn_scoped(
        &fake_config(script),
        "child",
        gateway,
        CancellationToken::new(),
        &scope,
    )
    .unwrap();
    let response = transport.rpc("initialize", json!({})).await.unwrap();
    let pid = response["descendantPid"].as_u64().unwrap();
    assert_eq!(scope.live_count(), 1);
    transport.shutdown().await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
        if state.as_ref().is_none_or(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        }) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant remains live after group cleanup: {pid}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(scope.live_count(), 0);
}
