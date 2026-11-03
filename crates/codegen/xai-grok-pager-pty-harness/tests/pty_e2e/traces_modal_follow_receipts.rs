// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;
use super::traces_modal_follow::{END, TITLE, USER, WAIT, run_hook, traces_binary, append};

/// Follow shows structure from hook receipts before the transcript has it: `SubagentStart` makes the child row
/// appear with no child file on disk, `PreToolUse` makes the call row appear with its command while the
/// transcript has no such call, and the matching `PostToolUse` closes it with a duration. The transcript then
/// catches up with the call and result: nothing is shown twice, and the highlighted call shows its output inline after `l`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn traces_modal_follow_receipts() {
    let content = ContentController::start().await.expect("start content");
    let home = content.home().to_path_buf();
    let session = super::traces_modal_follow::SESSION;
    let transcript = home.join(format!(".claude/projects/-w-claude/{session}.jsonl"));
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, format!("{USER}\n")).unwrap();

    let binary = pager_binary().expect("resolve pager binary");
    let hook_bin = traces_binary(&binary);
    let hook = |event: &str, extra: serde_json::Value| run_hook(&hook_bin, &home, &transcript, event, extra);
    hook("UserPromptSubmit", serde_json::json!({}));

    let mut h = PtyHarness::spawn_with_content_env_ops_in_dir(&binary, DEFAULT_ROWS, DEFAULT_COLS, &content, &["--yolo", "--trust"], &[EnvOp::remove("GROK_HOME")], Some(content.home()))
        .expect("spawn pager with content");
    enter_session(&mut h, &content);
    inject_keys_paced(&mut h, b"/traces");
    h.inject_keys(b"\r").expect("submit /traces");
    h.wait_for_text(TITLE, Duration::from_secs(20)).expect("ledger lists the live fixture");
    inject_keys_paced(&mut h, b"/ptyfollowtask");
    h.inject_keys(b"\r").expect("keep the search");
    h.wait_for_text("f follow", WAIT).expect("the list hint offers f");
    h.inject_keys(b"f").expect("follow");
    h.wait_for_text("● live", WAIT).expect("the live header");

    // SubagentStart: the child row, with no child file anywhere.
    hook("SubagentStart", serde_json::json!({ "agent_id": "ptykid" }));
    h.wait_for_text("↳ started", WAIT).expect("the child row from its start receipt");
    assert!(!transcript.with_extension("").join("subagents").exists(), "no child file exists");

    // PreToolUse: the call row with its command, while the transcript has no call.
    hook("PreToolUse", serde_json::json!({ "tool_name": "Bash", "tool_use_id": "ptyrcpt1", "tool_input": { "command": "echo ptyreceiptcmd" } }));
    h.wait_for_text("ptyreceiptcmd", WAIT).expect("the call row with its command");
    assert!(h.contains_text("→ Bash") && !h.contains_text("✓"), "open call\n{}", h.screen_contents());
    assert!(!std::fs::read_to_string(&transcript).unwrap().contains("ptyrcpt1"), "the transcript has no such call yet");

    // PostToolUse a moment later: closed, with the duration between the two receipts.
    std::thread::sleep(Duration::from_millis(1200));
    hook("PostToolUse", serde_json::json!({ "tool_name": "Bash", "tool_use_id": "ptyrcpt1", "tool_input": { "command": "echo ptyreceiptcmd" } }));
    // The gap between the two hook processes is not exact: any duration will do.
    h.wait_until("a ✓ followed by a duration", WAIT, |h| h.screen_contents().split("✓ ").skip(1).any(|rest| rest.starts_with(|c: char| c.is_ascii_digit())))
        .expect("the call closes with a duration");

    // The transcript catches up: the call stays one row.
    append(&transcript, r#"{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"ptyrcpt1","name":"Bash","input":{"command":"echo ptyreceiptcmd"}}]}}"#);
    append(&transcript, r#"{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"ptyrcpt1","content":"ptyreceiptout"}]}}"#);
    append(&transcript, r#"{"type":"assistant","timestamp":"2026-10-01T00:00:07Z","message":{"role":"assistant","content":[{"type":"text","text":"ptyreceiptdone"}]}}"#);
    hook("Stop", serde_json::json!({}));
    h.wait_for_text("ptyreceiptdone", WAIT).expect("the reply arrives");
    let screen = h.screen_contents();
    assert_eq!(screen.matches("→ Bash").count(), 1, "one call row\n{screen}");

    // k pauses on the call: its output comes from the transcript line the call row was linked to.
    h.inject_keys(b"k").expect("pause on the call");
    h.wait_for_text("paused", WAIT).expect("paused footer");
    h.inject_keys(b"l").expect("open the call");
    h.wait_for_text("ptyreceiptout", WAIT).expect("l shows the call's output inline");
    assert!(!h.contains_text("could not read"), "{}", h.screen_contents());
    h.inject_keys(b"G").expect("resume the tail");
    h.wait_for_text("live tail", WAIT).expect("tail pinned again");

    append(&transcript, END);
    hook("SessionEnd", serde_json::json!({}));
    h.wait_for_text("■ ended", WAIT).expect("the end row");
    h.inject_keys(keys::ESC).expect("back to the ledger");
    h.wait_for_text_absent("j/k scroll", WAIT).expect("ledger again");
    h.inject_keys(keys::ESC).expect("close traces");
    h.wait_for_text_absent("SOURCE", WAIT).expect("modal closed");
    assert!(!h.contains_text("panicked"), "{}", h.screen_contents());
    h.quit().expect("clean quit");
}
