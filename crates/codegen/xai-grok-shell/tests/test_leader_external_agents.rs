//! Leader hosts external ACP agents, keyed by the client's `agent_cmd`.
//! The fixture is `tests/fixtures/fake_acp_agent.py`. It is not DSH.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use xai_grok_shell::leader::{
    ClientCapabilities, ClientMode, LeaderClient, LeaderServerControlState, LeaderServerMetadata,
    protocol::{ClientMessage, ServerMessage, read_message, write_message},
    run_leader_server, spawn_leader_server,
};

fn fake_cmd(name: &str) -> String {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_acp_agent.py");
    format!("python3 {} {name}", script.display())
}

fn caps_for(cmd: Option<&str>) -> ClientCapabilities {
    ClientCapabilities {
        agent_cmd: cmd.map(str::to_string),
        ..Default::default()
    }
}

async fn wait_for_socket(sock_path: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if sock_path.exists() && UnixStream::connect(sock_path).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("socket did not come up: {}", sock_path.display());
}

async fn recv_where(
    client: &mut LeaderClient,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            panic!("timed out waiting for a matching ACP payload");
        }
        let msg = tokio::time::timeout(left, client.recv())
            .await
            .expect("timed out waiting for ACP payload")
            .expect("leader closed the client channel");
        let json: serde_json::Value =
            serde_json::from_str(&msg).unwrap_or_else(|e| panic!("bad json {msg}: {e}"));
        if pred(&json) {
            return json;
        }
    }
}

fn pid_of(json: &serde_json::Value) -> u32 {
    json.pointer("/result/pid")
        .or_else(|| json.pointer("/result/meta/pid"))
        .and_then(|v| v.as_u64())
        .expect("response has a pid") as u32
}

fn command_of(json: &serde_json::Value) -> String {
    json.pointer("/result/command")
        .or_else(|| json.pointer("/result/meta/command"))
        .and_then(|v| v.as_str())
        .expect("response names the command")
        .to_string()
}

async fn initialize(client: &mut LeaderClient) -> serde_json::Value {
    client
        .send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#.into())
        .unwrap();
    recv_where(client, |j| j.get("id").is_some() && j.get("result").is_some()).await
}

async fn session_new(client: &mut LeaderClient, id: u64) -> serde_json::Value {
    client
        .send(format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"session/new","params":{{"cwd":"/tmp","mcpServers":[]}}}}"#
        ))
        .unwrap();
    recv_where(client, |j| j.get("result").and_then(|r| r.get("sessionId")).is_some()).await
}

fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

struct ReadyServer {
    sock: PathBuf,
    cancel: CancellationToken,
    acp_rx: mpsc::UnboundedReceiver<String>,
    response_tx: mpsc::UnboundedSender<String>,
    _temp: TempDir,
}

async fn start_ready() -> ReadyServer {
    let temp = TempDir::new().unwrap();
    let sock = temp.path().join("leader.sock");
    let handle = spawn_leader_server(sock.clone()).await.unwrap();
    wait_for_socket(&sock).await;
    ReadyServer {
        sock,
        cancel: handle.cancel,
        acp_rx: handle.acp_rx,
        response_tx: handle.response_tx,
        _temp: temp,
    }
}

async fn connect(sock: &PathBuf, name: &str, cmd: Option<&str>) -> LeaderClient {
    LeaderClient::connect(sock.clone(), name, ClientMode::Stdio, caps_for(cmd))
        .await
        .unwrap_or_else(|e| panic!("connect {name}: {e}"))
}

#[tokio::test]
async fn external_session_is_answered_by_the_fake_agent_not_the_native_backend() {
    let mut server = start_ready().await;
    let cmd = fake_cmd("fake-a");
    let mut client = connect(&server.sock, "ext-a", Some(&cmd)).await;
    let init = initialize(&mut client).await;
    assert_eq!(command_of(&init), "fake-a");
    assert!(pid_of(&init) > 0);
    let host = init
        .pointer("/result/meta/hostname")
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(!host.is_empty());

    let created = session_new(&mut client, 2).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_string();
    assert_eq!(pid_of(&created), pid_of(&init));
    assert_eq!(command_of(&created), "fake-a");

    client
        .send(format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"{sid}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
        ))
        .unwrap();
    let update = recv_where(&mut client, |j| {
        j.get("method").and_then(|m| m.as_str()) == Some("session/update")
    })
    .await;
    let text = update
        .pointer("/params/update/content/text")
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(text.contains("command=fake-a"), "{text}");
    assert!(text.contains(&format!("pid={}", pid_of(&init))), "{text}");

    let native = tokio::time::timeout(Duration::from_millis(200), server.acp_rx.recv()).await;
    assert!(
        native.is_err(),
        "external traffic must not reach the native backend, got {native:?}"
    );
    client.cancel();
    server.cancel.cancel();
}

#[tokio::test]
async fn two_clients_with_the_same_command_share_one_process_and_live_updates() {
    let server = start_ready().await;
    let cmd = fake_cmd("fake-a");
    let mut a = connect(&server.sock, "a", Some(&cmd)).await;
    let mut b = connect(&server.sock, "b", Some(&cmd)).await;
    let init_a = initialize(&mut a).await;
    let init_b = initialize(&mut b).await;
    assert_eq!(pid_of(&init_a), pid_of(&init_b));
    assert_eq!(init_a.pointer("/result/meta/initCount"), Some(&serde_json::json!(1)));
    assert_eq!(
        init_b.pointer("/result/meta/initCount"),
        Some(&serde_json::json!(1)),
        "the second client must receive the cached initialize, not a second handshake"
    );

    let created = session_new(&mut a, 2).await;
    let sid = created["result"]["sessionId"].as_str().unwrap();
    b.send(format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"session/load","params":{{"sessionId":"{sid}","cwd":"/tmp","mcpServers":[]}}}}"#
    ))
    .unwrap();
    let loaded = recv_where(&mut b, |j| j.pointer("/result/sessionId").is_some()).await;
    assert_eq!(pid_of(&loaded), pid_of(&init_a));
    assert_eq!(loaded["result"]["known"], true);

    a.send(format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"session/prompt","params":{{"sessionId":"{sid}","prompt":[{{"type":"text","text":"ping"}}]}}}}"#
    ))
    .unwrap();
    let seen_by_b = recv_where(&mut b, |j| {
        j.pointer("/params/update/content/text")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.contains("command=fake-a"))
    })
    .await;
    assert!(
        seen_by_b
            .pointer("/params/update/content/text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains(&format!("pid={}", pid_of(&init_a)))
    );
    a.cancel();
    b.cancel();
    server.cancel.cancel();
}

#[tokio::test]
async fn two_commands_stay_on_two_processes() {
    let server = start_ready().await;
    let mut a = connect(&server.sock, "a", Some(&fake_cmd("fake-a"))).await;
    let mut b = connect(&server.sock, "b", Some(&fake_cmd("fake-b"))).await;
    let init_a = initialize(&mut a).await;
    let init_b = initialize(&mut b).await;
    assert_ne!(pid_of(&init_a), pid_of(&init_b));
    let created_a = session_new(&mut a, 2).await;
    let created_b = session_new(&mut b, 2).await;
    assert_eq!(command_of(&created_a), "fake-a");
    assert_eq!(command_of(&created_b), "fake-b");
    assert_eq!(pid_of(&created_a), pid_of(&init_a));
    assert_eq!(pid_of(&created_b), pid_of(&init_b));
    let sid_a = created_a["result"]["sessionId"].as_str().unwrap();
    a.send(format!(
        r#"{{"jsonrpc":"2.0","id":6,"method":"session/prompt","params":{{"sessionId":"{sid_a}","prompt":[]}}}}"#
    ))
    .unwrap();
    let update = recv_where(&mut a, |j| j.get("method").and_then(|m| m.as_str()) == Some("session/update")).await;
    let text = update.pointer("/params/update/content/text").and_then(|v| v.as_str()).unwrap();
    assert!(text.contains("command=fake-a"), "{text}");
    assert!(!text.contains("command=fake-b"), "{text}");
    a.cancel();
    b.cancel();
    server.cancel.cancel();
}

#[tokio::test]
async fn native_and_external_clients_both_work_on_one_leader() {
    let mut server = start_ready().await;
    let mut ext = connect(&server.sock, "ext", Some(&fake_cmd("fake-a"))).await;
    let mut native = connect(&server.sock, "native", None).await;
    let init = initialize(&mut ext).await;
    let created = session_new(&mut ext, 2).await;
    assert_eq!(pid_of(&created), pid_of(&init));

    native
        .send(r#"{"jsonrpc":"2.0","id":7,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#.into())
        .unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(2), server.acp_rx.recv())
        .await
        .expect("native session/new must reach the native backend")
        .expect("native channel closed");
    let fwd: serde_json::Value = serde_json::from_str(&forwarded).unwrap();
    assert_eq!(fwd["method"], "session/new");
    let namespaced = fwd["id"].as_str().unwrap();
    server
        .response_tx
        .send(format!(
            r#"{{"jsonrpc":"2.0","id":"{namespaced}","result":{{"sessionId":"native-sess"}}}}"#
        ))
        .unwrap();
    let native_resp = recv_where(&mut native, |j| j.pointer("/result/sessionId").is_some()).await;
    assert_eq!(native_resp["result"]["sessionId"], "native-sess");
    ext.cancel();
    native.cancel();
    server.cancel.cancel();
}

#[tokio::test]
async fn no_grok_login_serves_external_and_rejects_native_with_the_auth_error() {
    let temp = TempDir::new().unwrap();
    let sock = temp.path().join("leader.sock");
    let (acp_tx, _acp_rx) = mpsc::unbounded_channel::<String>();
    let (_response_tx, response_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    let (_ready_tx, ready_rx) = watch::channel(false);
    let sock_clone = sock.clone();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        let control = LeaderServerControlState::new(LeaderServerMetadata {
            pid: std::process::id(),
            socket_path: sock_clone.clone(),
            lock_path: sock_clone.with_extension("lock"),
            ws_url_suffix: String::new(),
            leader_binary_version: env!("CARGO_PKG_VERSION").to_string(),
        });
        let _ = run_leader_server(
            sock_clone,
            acp_tx,
            response_rx,
            cancel_clone,
            true,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
            xai_grok_shell::agent::activity::AgentActivity::default(),
            ready_rx,
            watch::channel(false).0,
            watch::channel(xai_grok_shell::leader::ShutdownReason::Manual).0,
            None,
            control,
        )
        .await;
    });
    wait_for_socket(&sock).await;

    let cmd = fake_cmd("fake-a");
    let mut ext = tokio::time::timeout(
        Duration::from_secs(3),
        LeaderClient::connect(sock.clone(), "ext", ClientMode::Stdio, caps_for(Some(&cmd))),
    )
    .await
    .expect("external client must not wait on native auth")
    .expect("external connect");
    let created = session_new(&mut ext, 2).await;
    assert_eq!(command_of(&created), "fake-a");

    let stream = UnixStream::connect(&sock).await.unwrap();
    let (mut reader, mut writer) = tokio::io::split(stream);
    write_message(
        &mut writer,
        &ClientMessage::Register {
            client_type: "native".into(),
            mode: ClientMode::Stdio,
            capabilities: ClientCapabilities::default(),
        },
    )
    .await
    .unwrap();
    let reg: ServerMessage = tokio::time::timeout(Duration::from_secs(2), read_message(&mut reader))
        .await
        .expect("register timeout")
        .unwrap();
    match reg {
        ServerMessage::Registered { ready, .. } => assert!(!ready, "native client stays gated on auth"),
        other => panic!("expected Registered, got {other:?}"),
    }
    write_message(
        &mut writer,
        &ClientMessage::Acp {
            payload: r#"{"jsonrpc":"2.0","id":9,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#.into(),
        },
    )
    .await
    .unwrap();
    let denied: ServerMessage = tokio::time::timeout(Duration::from_secs(2), read_message(&mut reader))
        .await
        .expect("native client should get the auth error, not a hang")
        .unwrap();
    match denied {
        ServerMessage::Acp { payload } => {
            let json: serde_json::Value = serde_json::from_str(&payload).unwrap();
            let message = json["error"]["message"].as_str().unwrap_or("");
            let data = json["error"]["data"].as_str().unwrap_or("");
            assert!(
                message.contains("leader_starting") || data.contains("auth"),
                "native auth error was {payload}"
            );
        }
        other => panic!("expected ACP auth error, got {other:?}"),
    }
    ext.cancel();
    cancel.cancel();
}

#[tokio::test]
async fn killing_the_agent_reports_an_error_and_the_next_session_respawns() {
    let server = start_ready().await;
    let mut client = connect(&server.sock, "ext", Some(&fake_cmd("fake-a"))).await;
    let init = initialize(&mut client).await;
    let first_pid = pid_of(&init);
    let created = session_new(&mut client, 2).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_string();
    assert!(std::process::Command::new("kill")
        .args(["-KILL", &first_pid.to_string()])
        .status()
        .unwrap()
        .success());
    client
        .send(format!(
            r#"{{"jsonrpc":"2.0","id":8,"method":"session/prompt","params":{{"sessionId":"{sid}","prompt":[]}}}}"#
        ))
        .unwrap();
    let err = recv_where(&mut client, |j| {
        let blob = j.to_string();
        blob.contains("exited")
    })
    .await;
    assert!(err.to_string().contains("exited"), "{err}");

    let again = session_new(&mut client, 9).await;
    let new_pid = pid_of(&again);
    assert_ne!(new_pid, first_pid);
    assert!(pid_alive(new_pid));
    client.cancel();
    server.cancel.cancel();
}

#[tokio::test]
async fn old_client_without_agent_cmd_stays_on_the_native_backend() {
    let mut server = start_ready().await;
    let stream = UnixStream::connect(&server.sock).await.unwrap();
    let (mut reader, mut writer) = tokio::io::split(stream);
    write_raw(
        &mut writer,
        r#"{"type":"register","client_type":"legacy","mode":"stdio","capabilities":{"yolo_mode":false}}"#,
    )
    .await;
    let reg: ServerMessage = tokio::time::timeout(Duration::from_secs(2), read_message(&mut reader))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reg, ServerMessage::Registered { .. }));
    write_message(
        &mut writer,
        &ClientMessage::Acp {
            payload: r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#.into(),
        },
    )
    .await
    .unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(2), server.acp_rx.recv())
        .await
        .expect("legacy client must hit the native backend")
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&forwarded).unwrap();
    assert_eq!(json["method"], "session/new");
    server.cancel.cancel();
}

#[tokio::test]
async fn leader_shutdown_kills_the_external_process() {
    let server = start_ready().await;
    let mut client = connect(&server.sock, "ext", Some(&fake_cmd("fake-a"))).await;
    let init = initialize(&mut client).await;
    let pid = pid_of(&init);
    assert!(pid_alive(pid));
    client.cancel();
    server.cancel.cancel();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while pid_alive(pid) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(!pid_alive(pid), "external agent pid {pid} survived leader shutdown");
}

async fn write_raw(writer: &mut (impl AsyncWrite + Unpin), json: &str) {
    let len = json.len() as u32;
    writer.write_all(&len.to_be_bytes()).await.unwrap();
    writer.write_all(json.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}
