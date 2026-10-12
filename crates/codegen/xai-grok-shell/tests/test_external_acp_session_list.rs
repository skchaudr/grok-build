//! External ACP `session/list` and `session/load` replay (pi-acp parity fixture).

#![cfg(unix)]

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

fn fake_pi_cmd(catalog: &std::path::Path) -> String {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_acp_agent.py");
    format!(
        "python3 {} fake-pi-sessions {}",
        script.display(),
        catalog.display()
    )
}

fn rpc(stdin: &mut std::process::ChildStdin, id: u64, method: &str, params: serde_json::Value) {
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    stdin
        .write_all(format!("{line}\n").as_bytes())
        .expect("write rpc");
    stdin.flush().expect("flush");
}

fn read_result(
    stdout: &mut std::io::BufReader<std::process::ChildStdout>,
    id: u64,
    updates: &mut Vec<String>,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let mut line = String::new();
        stdout.read_line(&mut line).expect("read line");
        let json: serde_json::Value = serde_json::from_str(line.trim()).expect("json line");
        if json.get("method") == Some(&serde_json::json!("session/update")) {
            let kind = json
                .pointer("/params/update/sessionUpdate")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            updates.push(kind.to_string());
        }
        if json.get("id") == Some(&serde_json::json!(id)) {
            return json;
        }
    }
    panic!("timed out waiting for rpc {id}");
}

#[test]
fn external_agent_list_sessions_and_load_replay_without_duplicates() {
    let dir = TempDir::new().unwrap();
    let catalog = dir.path().join("catalog.json");
    let catalog_body = serde_json::json!({
        "sessions": [{
            "sessionId": "sess-pi-1",
            "cwd": "/tmp/pi-work",
            "title": "Earlier turn",
            "updatedAt": "2026-10-12T09:00:00Z"
        }],
        "replays": {
            "sess-pi-1": [
                {
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                },
                {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hi there" }
                }
            ]
        },
        "usage": { "sess-pi-1": 1234 }
    });
    std::fs::write(&catalog, catalog_body.to_string()).unwrap();

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(fake_pi_cmd(&catalog))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn fake pi agent");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = std::io::BufReader::new(stdout);
    let mut updates = Vec::new();

    rpc(&mut stdin, 1, "initialize", serde_json::json!({ "protocolVersion": 1 }));
    let init = read_result(&mut reader, 1, &mut updates);
    assert_eq!(
        init.pointer("/result/agentCapabilities/loadSession")
            .and_then(|v| v.as_bool()),
        Some(true)
    );

    rpc(
        &mut stdin,
        2,
        "session/list",
        serde_json::json!({ "cwd": "/tmp/pi-work" }),
    );
    let listed = read_result(&mut reader, 2, &mut updates);
    let sessions = listed
        .pointer("/result/sessions")
        .and_then(|v| v.as_array())
        .expect("sessions array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].get("sessionId").and_then(|v| v.as_str()),
        Some("sess-pi-1")
    );
    assert_eq!(
        sessions[0].get("title").and_then(|v| v.as_str()),
        Some("Earlier turn")
    );
    assert_eq!(
        sessions[0].get("cwd").and_then(|v| v.as_str()),
        Some("/tmp/pi-work")
    );
    assert_eq!(
        sessions[0].get("updatedAt").and_then(|v| v.as_str()),
        Some("2026-10-12T09:00:00Z")
    );

    rpc(
        &mut stdin,
        3,
        "session/load",
        serde_json::json!({
            "sessionId": "sess-pi-1",
            "cwd": "/tmp/pi-work",
            "mcpServers": []
        }),
    );
    let _load = read_result(&mut reader, 3, &mut updates);
    let first_load_updates = updates.clone();
    assert_eq!(
        first_load_updates
            .iter()
            .filter(|u| *u == "user_message_chunk")
            .count(),
        1
    );
    assert_eq!(
        first_load_updates
            .iter()
            .filter(|u| *u == "agent_message_chunk")
            .count(),
        1
    );
    assert!(
        first_load_updates.iter().any(|u| *u == "usage_update"),
        "replay must publish token usage: {first_load_updates:?}"
    );

    rpc(
        &mut stdin,
        4,
        "session/load",
        serde_json::json!({
            "sessionId": "sess-pi-1",
            "cwd": "/tmp/pi-work",
            "mcpServers": []
        }),
    );
    let _load2 = read_result(&mut reader, 4, &mut updates);
    let second_only = updates[first_load_updates.len()..].to_vec();
    assert!(
        second_only.is_empty(),
        "second load must not duplicate replay updates, got {second_only:?}"
    );

    let _ = child.kill();
}
