// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

// Synthetic fixtures only.
const CLAUDE_USER: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","message":{"role":"user","content":"ptyclaudetask"}}"#;
const CLAUDE: &[&str] = &[
    CLAUDE_USER,
    r#"{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
    r#"{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#,
];
const CODEX: &[&str] = &[
    r#"{"timestamp":"2026-10-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"ptycodextask"}]}}"#,
];
const PI: &[&str] = &[
    r#"{"type":"message","timestamp":"2026-10-03T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"ptypitask"}]}}"#,
];

fn seed(path: PathBuf, lines: &[&str]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

/// `/traces` lists sessions from HOME, tab filters by source, Enter opens the trajectory,
/// `e` filters errors, Enter shows the raw source line, `y` copies it byte for byte,
/// and `r` on a source with no resume path shows the notice instead of launching anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn traces_modal_ledger_trajectory_inspector() {
    let content = ContentController::start().await.expect("start content");
    let home = content.home().to_path_buf();
    seed(home.join(".claude/projects/-w-claude/0b7e1c2a-0000-4000-8000-000000000001.jsonl"), CLAUDE);
    seed(home.join(".codex/sessions/2026/10/02/rollout-2026-10-02T00-00-00-0b7e1c2a-0000-4000-8000-000000000002.jsonl"), CODEX);
    seed(home.join(".pi/agent/sessions/--w-pi--/p.jsonl"), PI);

    let copy_file = home.join("traces-copy.txt");
    let copy_env = copy_file.to_string_lossy().into_owned();
    let binary = pager_binary().expect("resolve pager binary");
    let mut h = PtyHarness::spawn_with_content_env_in_dir(
        &binary,
        DEFAULT_ROWS,
        DEFAULT_COLS,
        &content,
        &["--yolo", "--trust"],
        &[("GROK_COPY_FILE", copy_env.as_str())],
        Some(content.home()),
    )
    .expect("spawn pager with content");
    enter_session(&mut h, &content);

    inject_keys_paced(&mut h, b"/traces");
    h.inject_keys(b"\r").expect("submit /traces");
    h.wait_for_text("ptyclaudetask", Duration::from_secs(20)).expect("ledger lists the claude fixture");
    h.wait_for_text("ptycodextask", Duration::from_secs(5)).expect("ledger lists the codex fixture");
    h.wait_for_text("ptypitask", Duration::from_secs(5)).expect("ledger lists the pi fixture");

    // All → DeepSeek → Claude.
    h.inject_keys(b"\t\t").expect("tab to Claude");
    h.wait_for_text_absent("ptycodextask", Duration::from_secs(5)).expect("Claude tab hides codex");
    assert!(h.contains_text("ptyclaudetask"), "{}", h.screen_contents());

    h.inject_keys(keys::ENTER).expect("open trajectory");
    h.wait_for_text("INPUT", Duration::from_secs(5)).expect("timeline strip renders");
    assert!(h.contains_text("Bash"), "tool row\n{}", h.screen_contents());

    h.inject_keys(b"e").expect("errors only");
    h.wait_for_text("errors only", Duration::from_secs(5)).expect("error filter on");
    h.wait_for_text_absent("#0", Duration::from_secs(5)).expect("non-error rows hidden");
    assert!(h.contains_text("#2"), "the is_error tool result stays\n{}", h.screen_contents());

    h.inject_keys(b"e").expect("errors off");
    h.wait_for_text("#0", Duration::from_secs(5)).expect("rows back");
    h.inject_keys(keys::ENTER).expect("inspect first row");
    h.wait_for_text("line 0", Duration::from_secs(5)).expect("inspector header");
    assert!(h.contains_text("\"content\": \"ptyclaudetask\""), "pretty JSON\n{}", h.screen_contents());

    h.inject_keys(b"y").expect("copy raw line");
    // Every copy also lands in the backup file; compare it byte for byte with the source line.
    let deadline = Instant::now() + Duration::from_secs(5);
    let copied = loop {
        h.update(Duration::from_millis(100));
        let text = std::fs::read_to_string(&copy_file).unwrap_or_default();
        if !text.is_empty() || Instant::now() >= deadline {
            break text;
        }
    };
    assert_eq!(copied, CLAUDE_USER, "raw line copied byte for byte");

    h.inject_keys(keys::ESC).expect("back to trajectory");
    h.wait_for_text("INPUT", Duration::from_secs(5)).expect("trajectory again");
    h.inject_keys(keys::ESC).expect("back to ledger");
    h.wait_for_text_absent("INPUT", Duration::from_secs(5)).expect("ledger again");

    // Claude → Pi, then r: Pi has no resume path.
    h.inject_keys(b"\t").expect("tab to Pi");
    h.wait_for_text("ptypitask", Duration::from_secs(5)).expect("Pi tab");
    h.inject_keys(b"r").expect("resume pi");
    h.wait_for_text("no resume for Pi", Duration::from_secs(5)).expect("no-resume notice");
    assert!(h.contains_text("ptypitask"), "modal stays open\n{}", h.screen_contents());

    h.inject_keys(keys::ESC).expect("close traces");
    h.wait_for_text_absent("SOURCE", Duration::from_secs(5)).expect("modal closed");
    assert!(!h.contains_text("panicked"), "{}", h.screen_contents());
    h.quit().expect("clean quit");
}
