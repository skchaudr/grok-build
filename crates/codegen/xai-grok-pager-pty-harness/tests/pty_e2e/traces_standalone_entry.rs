// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

// Synthetic fixture only.
const TITLE: &str = "ptystandalonetask";

/// `grok traces` opens the ledger straight away (no prompt, no `/traces`), lists sessions from HOME,
/// and `q` at the ledger's top level exits 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn traces_standalone_entry() {
    let content = ContentController::start().await.expect("start content");
    let transcript = content.home().join(".claude/projects/-w-claude/0b7e1c2a-0000-4000-8000-000000000001.jsonl");
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(
        &transcript,
        format!(
            "{}\n",
            serde_json::json!({"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","message":{"role":"user","content":TITLE}})
        ),
    )
    .unwrap();

    let binary = pager_binary().expect("resolve pager binary");
    let mut h = PtyHarness::spawn_with_content_env_ops_in_dir(
        &binary,
        DEFAULT_ROWS,
        DEFAULT_COLS,
        &content,
        &["traces"],
        &[EnvOp::remove("GROK_HOME")],
        Some(content.home()),
    )
    .expect("spawn grok traces");

    // Generous: a cold exec of the debug binary has taken over 30s here.
    h.wait_for_text(TITLE, Duration::from_secs(90)).expect("ledger lists the fixture session with no prompt");
    assert!(h.contains_text("Traces"), "modal title\n{}", h.screen_contents());

    h.inject_keys(b"q").expect("quit");
    let code = h.wait_exit_code(Duration::from_secs(10)).expect("wait exit");
    assert!(matches!(code, PtyExitPoll::Exited(0)), "{code:?}");
}
