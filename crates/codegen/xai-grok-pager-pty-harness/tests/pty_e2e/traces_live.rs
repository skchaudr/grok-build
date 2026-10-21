// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

// Synthetic fixture only.
const SESSION: &str = "0b7e1c2a-0000-4000-8000-000000000001";
const TITLE: &str = "ptyclaudetask";
const WAIT: Duration = Duration::from_secs(6);

/// `grok-traces`: `GROK_TRACES_BIN`, else next to the pager binary.
fn traces_binary(pager: &Path) -> PathBuf {
    std::env::var_os("GROK_TRACES_BIN").map(PathBuf::from).unwrap_or_else(|| pager.with_file_name("grok-traces"))
}

/// What a harness hook does: `grok-traces hook claude` with the event JSON on stdin, same HOME as the pager.
/// The hook is a direct child of this test binary, so its parent-chain walk records the test binary's pid,
/// which is alive for the whole test: the row stays live until `SessionEnd`.
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

/// The first glyph on the screen line that holds the session's title.
fn row_glyph(h: &PtyHarness) -> Option<String> {
    let screen = h.screen_contents();
    let line = screen.lines().find(|l| l.contains(TITLE))?;
    line.split_whitespace().next().map(str::to_string)
}

fn wait_glyph(h: &mut PtyHarness, glyph: &str) {
    let deadline = Instant::now() + WAIT;
    while row_glyph(h).as_deref() != Some(glyph) {
        assert!(Instant::now() < deadline, "row glyph never became {glyph}\n{}", h.screen_contents());
        h.update(Duration::from_millis(100));
    }
}

/// `grok live` is a standalone full-screen loop: a hook-driven session shows as a row, a permission request
/// raises `⚠` with its text on that row, the tool finishing returns it to `●`, `SessionEnd` empties the screen,
/// and `q` exits 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn traces_live() {
    let content = ContentController::start().await.expect("start content");
    let home = content.home().to_path_buf();
    let transcript = home.join(format!(".claude/projects/-w-claude/{SESSION}.jsonl"));
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
    let hook_bin = traces_binary(&binary);
    // HOME decides the index (`~/.grok/traces`), as for the hook: no GROK_HOME override on either side.
    let mut h = PtyHarness::spawn_with_content_env_ops_in_dir(
        &binary,
        DEFAULT_ROWS,
        DEFAULT_COLS,
        &content,
        &["live"],
        &[EnvOp::remove("GROK_HOME")],
        Some(content.home()),
    )
    .expect("spawn grok live");

    h.wait_for_text("no live agents", Duration::from_secs(10)).expect("live screen up");
    assert!(!h.contains_text("no index"), "GROK_HOME/HOME mismatch: the pager has no index\n{}", h.screen_contents());

    let hook = |event: &str, extra: serde_json::Value| run_hook(&hook_bin, &home, &transcript, event, extra);
    let bash = || serde_json::json!({"tool_name":"Bash","tool_input":{"command":"cargo build"},"tool_use_id":"t1"});
    hook("UserPromptSubmit", serde_json::json!({"prompt": TITLE}));
    hook("PreToolUse", bash());
    hook("PermissionRequest", bash());

    h.wait_for_text("⚠", WAIT).expect("permission raises the warning glyph");
    wait_glyph(&mut h, "⚠");
    let screen = h.screen_contents();
    let line = screen.lines().find(|l| l.contains(TITLE)).unwrap_or_default();
    assert!(line.contains("permission: Bash cargo build"), "{screen}");

    hook("PostToolUse", bash());
    h.wait_for_text_absent("permission:", WAIT).expect("permission text goes");
    wait_glyph(&mut h, "●");
    assert!(!h.contains_text("⚠"), "{}", h.screen_contents());

    hook("SessionEnd", serde_json::json!({}));
    h.wait_for_text_absent(TITLE, WAIT).expect("row leaves");
    h.wait_for_text("no live agents", WAIT).expect("empty state");

    h.inject_keys(b"q").expect("quit");
    let code = h.wait_exit_code(WAIT).expect("wait exit");
    assert!(matches!(code, PtyExitPoll::Exited(0)), "{code:?}");
}
