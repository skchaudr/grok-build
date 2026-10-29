// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

// Synthetic fixture only.
const SESSION: &str = "0b7e1c2a-0000-4000-8000-0000000000f1";
const TITLE: &str = "ptyfollowtask";
const WAIT: Duration = Duration::from_secs(10);

const USER: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","message":{"role":"user","content":"ptyfollowtask"}}"#;
const CALL: &str = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"f1","name":"Bash","input":{"command":"echo ptyfollowcmd","description":"say ptyfollowcmd"}}]}}"#;
const RESULT: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"f1","content":"ptyfollowout"}]}}"#;
// A terminal last line is what completes a transcript; the index keeps a fresh file active until then.
const END: &str = r#"{"type":"session_end","timestamp":"2026-10-01T00:00:08Z"}"#;
const REPLY: &str = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:07Z","message":{"role":"assistant","content":[{"type":"text","text":"ptyfollowdone"}]}}"#;

/// `grok-traces`: `GROK_TRACES_BIN`, else next to the pager binary.
fn traces_binary(pager: &Path) -> PathBuf {
    std::env::var_os("GROK_TRACES_BIN").map(PathBuf::from).unwrap_or_else(|| pager.with_file_name("grok-traces"))
}

/// What a harness hook does: `grok-traces hook claude` with the event JSON on stdin, same HOME as the pager.
/// The hook is a direct child of this test binary, so the row stays live until `SessionEnd`.
fn run_hook(bin: &Path, home: &Path, transcript: &Path, event: &str, extra: serde_json::Value) {
    use std::io::Write;
    let mut payload = serde_json::json!({
        "hook_event_name": event,
        "session_id": SESSION,
        "transcript_path": transcript,
        "cwd": "/w/claude",
    });
    payload.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
    let mut child = std::process::Command::new(bin)
        .args(["hook", "claude"])
        .env("HOME", home)
        .env_remove("GROK_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn grok-traces");
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(), "{event} hook: {out:?}");
}

fn append(path: &Path, line: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{line}").unwrap();
}

/// `f` on a live row opens the follow screen in `/traces`: the backlog shows the `→ Bash` call and the live
/// header, a result and a reply appended mid-follow arrive without a key press (`✓`, the text), `k` pauses the
/// tail and shows the call's command inline, and `SessionEnd` paints `■ ended`. Esc returns to the ledger.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn traces_modal_follow() {
    let content = ContentController::start().await.expect("start content");
    let home = content.home().to_path_buf();
    let transcript = home.join(format!(".claude/projects/-w-claude/{SESSION}.jsonl"));
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, format!("{USER}\n{CALL}\n")).unwrap();

    let binary = pager_binary().expect("resolve pager binary");
    let hook_bin = traces_binary(&binary);
    let hook = |event: &str| run_hook(&hook_bin, &home, &transcript, event, serde_json::json!({}));
    hook("UserPromptSubmit");
    hook("PreToolUse");

    let mut h = PtyHarness::spawn_with_content_env_ops_in_dir(&binary, DEFAULT_ROWS, DEFAULT_COLS, &content, &["--yolo", "--trust"], &[EnvOp::remove("GROK_HOME")], Some(content.home()))
        .expect("spawn pager with content");
    enter_session(&mut h, &content);

    inject_keys_paced(&mut h, b"/traces");
    h.inject_keys(b"\r").expect("submit /traces");
    h.wait_for_text(TITLE, Duration::from_secs(20)).expect("ledger lists the live fixture");
    // The pager's own session is listed too (and newer): search so the fixture is the selected row.
    inject_keys_paced(&mut h, b"/ptyfollowtask");
    h.inject_keys(b"\r").expect("keep the search");
    h.wait_for_text("f follow", WAIT).expect("the list hint offers f");

    h.inject_keys(b"f").expect("follow");
    h.wait_for_text("→ Bash", WAIT).expect("the backlog's call row");
    assert!(h.contains_text("▶ you") && h.contains_text("● live"), "prompt row and live header\n{}", h.screen_contents());
    assert!(h.contains_text("j/k scroll") && h.contains_text("e next error"), "follow hints\n{}", h.screen_contents());

    // Appended mid-follow: the result merges into the call (`✓`), the reply is a text row.
    append(&transcript, RESULT);
    append(&transcript, REPLY);
    hook("PostToolUse");
    h.wait_for_text("✓", WAIT).expect("the result arrives without a key press");
    h.wait_for_text("ptyfollowdone", WAIT).expect("the reply arrives");

    // k pauses the tail; the highlighted call shows its command under the row.
    h.inject_keys(b"k").expect("pause on the call");
    h.wait_for_text("paused", WAIT).expect("paused footer");
    h.wait_for_text("ptyfollowout", WAIT).expect("the highlighted call shows its result inline");
    h.inject_keys(b"G").expect("resume the tail");
    h.wait_for_text("live tail", WAIT).expect("tail pinned again");

    append(&transcript, END);
    hook("SessionEnd");
    h.wait_for_text("■ ended", WAIT).expect("the end row");

    h.inject_keys(keys::ESC).expect("back to the ledger");
    h.wait_for_text_absent("j/k scroll", WAIT).expect("ledger again");
    h.inject_keys(keys::ESC).expect("close traces");
    h.wait_for_text_absent("SOURCE", WAIT).expect("modal closed");
    assert!(!h.contains_text("panicked"), "{}", h.screen_contents());
    h.quit().expect("clean quit");
}
