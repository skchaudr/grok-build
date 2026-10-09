//! `grok -p --leader --leader-socket S --agent-cmd C` prints the worker's answer.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
async fn headless_prints_the_named_workers_answer() {
    let sock = std::env::temp_dir().join(format!(
        "headless-leader-agent-cmd-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    let server = xai_grok_shell::leader::spawn_leader_server_persistent(sock.clone())
        .await
        .expect("start leader");
    wait_for_socket(&sock).await;

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../xai-grok-shell/tests/fixtures/fake_acp_agent.py");
    let agent_cmd = format!("python3 {} headless-worker", fixture.display());
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_xai-grok-pager"))
            .args([
                "--no-auto-update",
                "--leader",
                "--leader-socket",
                sock.to_str().expect("socket path is utf-8"),
                "--agent-cmd",
                &agent_cmd,
                "-p",
                "run hostname and report it",
            ])
            .output(),
    )
    .await
    .expect("headless command timed out")
    .expect("spawn grok");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "grok -p --leader --agent-cmd failed: status={} stderr={stderr} stdout={stdout}",
        output.status
    );
    assert!(
        stdout.contains("command=headless-worker"),
        "answer should come from the named worker, got {stdout}"
    );
    server.cancel.cancel();
}

async fn wait_for_socket(path: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("leader socket did not come up: {}", path.display());
}
