// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

// Synthetic fixtures only.
const CLAUDE_USER: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","message":{"role":"user","content":"ptyclaudetask"}}"#;
const CLAUDE: &[&str] = &[
    CLAUDE_USER,
    r#"{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
    r#"{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#,
    r#"{"type":"assistant","timestamp":"2026-10-01T00:00:07Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Agent","input":{"description":"ptychildjob","subagent_type":"Explore","prompt":"do the child job"}}]}}"#,
    r#"{"type":"user","timestamp":"2026-10-01T00:00:08Z","message":{"role":"user","content":[{"type":"tool_result","content":"child done"}]}}"#,
];
// The subagent transcript Claude writes beside the parent, plus its meta.
const CHILD: &[&str] = &[
    r#"{"type":"user","timestamp":"2026-10-01T00:00:07Z","message":{"role":"user","content":"do the child job and report back"}}"#,
    r#"{"type":"assistant","timestamp":"2026-10-01T00:00:07Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"echo ptychildcmd"}}]}}"#,
    r#"{"type":"user","timestamp":"2026-10-01T00:00:07Z","message":{"role":"user","content":[{"type":"tool_result","content":"ptychildcmd"}]}}"#,
    r#"{"type":"assistant","timestamp":"2026-10-01T00:00:08Z","message":{"role":"assistant","content":"the child job is finished"}}"#,
];
const CHILD_META: &str = r#"{"agentType":"Explore","description":"ptychildjob"}"#;
// Three non-noise events: a one-line session is trivial and hidden in browse mode.
const CODEX: &[&str] = &[
    r#"{"timestamp":"2026-10-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"ptycodextask"}]}}"#,
    r#"{"timestamp":"2026-10-02T00:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"working on it"}]}}"#,
    r#"{"timestamp":"2026-10-02T00:00:03Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"keep going"}]}}"#,
];
const PI: &[&str] = &[
    r#"{"type":"message","timestamp":"2026-10-03T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"ptypitask"}]}}"#,
    r#"{"type":"message","timestamp":"2026-10-03T00:00:02Z","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}"#,
    r#"{"type":"message","timestamp":"2026-10-03T00:00:03Z","message":{"role":"user","content":[{"type":"text","text":"keep going"}]}}"#,
];

fn seed(path: PathBuf, lines: &[&str]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

/// `grok-traces`: `GROK_TRACES_BIN`, else next to the pager binary.
fn traces_binary(pager: &Path) -> PathBuf {
    std::env::var_os("GROK_TRACES_BIN").map(PathBuf::from).unwrap_or_else(|| pager.with_file_name("grok-traces"))
}

/// What a harness hook does: `grok-traces hook claude` with the event JSON on stdin, same HOME as the pager.
fn run_hook(bin: &Path, home: &Path, transcript: &Path) {
    use std::io::Write;
    let payload = format!(
        r#"{{"hook_event_name":"PostToolUse","session_id":"live","transcript_path":"{}","tool_name":"Bash","cwd":"/w"}}"#,
        transcript.display()
    );
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
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(), "hook: {out:?}");
}

/// `/traces` lists sessions from HOME (a subagent nests under its parent), tab filters by source,
/// Enter opens the trajectory, Enter on an Agent row opens the child and Esc returns, `x` filters errors, Enter shows the payload (text, or a call's command then its result), `y` copies it, `m` shows the raw source line and `Y` copies it byte for byte,
/// and `r` on a source with no resume path shows the notice instead of launching anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn traces_modal_ledger_trajectory_inspector() {
    let content = ContentController::start().await.expect("start content");
    let home = content.home().to_path_buf();
    seed(home.join(".claude/projects/-w-claude/0b7e1c2a-0000-4000-8000-000000000001.jsonl"), CLAUDE);
    let child_dir = home.join(".claude/projects/-w-claude/0b7e1c2a-0000-4000-8000-000000000001/subagents");
    seed(child_dir.join("agent-x.jsonl"), CHILD);
    std::fs::write(child_dir.join("agent-x.meta.json"), CHILD_META).unwrap();
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
    h.wait_for_text("ptychildjob", Duration::from_secs(5)).expect("subagent listed under its parent by dispatch label");
    assert!(h.contains_text("└─"), "tree branch\n{}", h.screen_contents());

    // Live mode: hooks fire while the ledger is open and the rows change with no key press.
    let hook_bin = traces_binary(&binary);
    let claude_file = home.join(".claude/projects/-w-claude/0b7e1c2a-0000-4000-8000-000000000001.jsonl");
    let row_tokens = |h: &PtyHarness| -> Vec<String> {
        let screen = h.screen_contents();
        let row = screen.lines().find(|l| l.contains("ptyclaudetask")).unwrap_or_default();
        row.split_whitespace().map(str::to_string).collect()
    };
    assert!(!row_tokens(&h).contains(&"6".to_string()), "5 events before\n{}", h.screen_contents());
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&claude_file).unwrap();
        writeln!(f, r#"{{"type":"assistant","timestamp":"2026-10-01T00:00:09Z","message":{{"role":"assistant","content":"more"}}}}"#).unwrap();
    }
    run_hook(&hook_bin, &home, &claude_file);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !row_tokens(&h).contains(&"6".to_string()) {
        assert!(Instant::now() < deadline, "event count never reached 6\n{}", h.screen_contents());
        h.update(Duration::from_millis(200));
    }
    h.wait_for_text("live", Duration::from_secs(5)).expect("live marker");
    let fresh = home.join(".claude/projects/-w-claude/0b7e1c2a-0000-4000-8000-000000000009.jsonl");
    seed(
        fresh.clone(),
        &[
            r#"{"type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":"ptylivetask"}}"#,
            r#"{"type":"assistant","timestamp":"2026-09-01T00:00:01Z","message":{"role":"assistant","content":"working on it"}}"#,
            r#"{"type":"user","timestamp":"2026-09-01T00:00:02Z","message":{"role":"user","content":"keep going"}}"#,
        ],
    );
    run_hook(&hook_bin, &home, &fresh);
    h.wait_for_text("ptylivetask", Duration::from_secs(10)).expect("new session appears without a key press");

    // All → DeepSeek → Claude.
    h.inject_keys(b"\t\t").expect("tab to Claude");
    h.wait_for_text_absent("ptycodextask", Duration::from_secs(5)).expect("Claude tab hides codex");
    assert!(h.contains_text("ptyclaudetask"), "{}", h.screen_contents());

    h.inject_keys(keys::ENTER).expect("open trajectory");
    h.wait_for_text("events ·", Duration::from_secs(5)).expect("trajectory header renders");
    assert!(h.contains_text("Bash"), "tool row\n{}", h.screen_contents());
    assert!(h.contains_text("✗"), "the failed result is folded into its Bash call\n{}", h.screen_contents());
    assert!(h.contains_text("→ ptychildjob"), "dispatch row names its child\n{}", h.screen_contents());

    // Enter on the Agent row jumps into the child; Esc comes back to the same trajectory.
    // Rows: the turn (its prompt folded in), the Bash call, then the Agent call.
    h.inject_keys(b"jj").expect("move to the Agent row");
    h.inject_keys(keys::ENTER).expect("open the child");
    h.wait_for_text("ptychildcmd", Duration::from_secs(5)).expect("child trajectory shows its own tool row");
    assert!(h.contains_text("↑ "), "child header names its parent\n{}", h.screen_contents());
    h.inject_keys(keys::ESC).expect("back to the parent");
    h.wait_for_text_absent("ptychildcmd", Duration::from_secs(5)).expect("parent again");
    assert!(h.contains_text("ptyclaudetask") || h.contains_text("#0"), "{}", h.screen_contents());

    h.inject_keys(b"x").expect("errors only");
    h.wait_for_text("errors only", Duration::from_secs(5)).expect("error filter on");
    h.wait_for_text_absent("#0", Duration::from_secs(5)).expect("non-error rows hidden");
    assert!(h.contains_text("#2"), "the is_error tool result stays\n{}", h.screen_contents());

    h.inject_keys(b"x").expect("errors off");
    h.wait_for_text("#0", Duration::from_secs(5)).expect("rows back");
    // Every copy also lands in the backup file; wait for it to hold `want`, byte for byte.
    let wait_copy = |h: &mut PtyHarness, want: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            h.update(Duration::from_millis(100));
            let text = std::fs::read_to_string(&copy_file).unwrap_or_default();
            if text == want || Instant::now() >= deadline {
                break text;
            }
        }
    };
    h.inject_keys(b"k").expect("select the turn row");
    h.inject_keys(keys::ENTER).expect("inspect first row");
    h.wait_for_text("User ·", Duration::from_secs(5)).expect("payload header");
    assert!(h.contains_text("ptyclaudetask"), "the message text\n{}", h.screen_contents());
    assert!(!h.contains_text("\"content\""), "JSON is behind m\n{}", h.screen_contents());
    h.inject_keys(b"y").expect("copy payload");
    assert_eq!(wait_copy(&mut h, "ptyclaudetask"), "ptyclaudetask", "y copies the text, not the JSON");

    h.inject_keys(b"m").expect("metadata");
    h.wait_for_text("line 0", Duration::from_secs(5)).expect("metadata header");
    assert!(h.contains_text("\"content\": \"ptyclaudetask\""), "pretty JSON\n{}", h.screen_contents());
    h.inject_keys(b"Y").expect("copy raw line");
    assert_eq!(wait_copy(&mut h, CLAUDE_USER), CLAUDE_USER, "raw line copied byte for byte");

    h.inject_keys(keys::ESC).expect("leave metadata");
    h.wait_for_text_absent("line 0", Duration::from_secs(5)).expect("Esc leaves metadata first");
    h.inject_keys(keys::ESC).expect("back to trajectory");
    h.wait_for_text("events ·", Duration::from_secs(5)).expect("trajectory again");

    // Enter on the failed Bash call: its command with a line number, the error separator, then the result.
    h.inject_keys(b"j").expect("move to the Bash row");
    h.inject_keys(keys::ENTER).expect("open the Bash payload");
    h.wait_for_text("✗ error", Duration::from_secs(5)).expect("error header");
    assert!(h.contains_text("1 │ ls"), "numbered command\n{}", h.screen_contents());
    assert!(h.contains_text("── ✗ result ──") && h.contains_text("boom"), "error separator, then the result\n{}", h.screen_contents());
    h.inject_keys(b"y").expect("copy command");
    assert_eq!(wait_copy(&mut h, "ls"), "ls", "y copies the command only");
    h.inject_keys(keys::ESC).expect("back to trajectory");
    h.wait_for_text("events ·", Duration::from_secs(5)).expect("trajectory again");
    // Message and tool rows start folded; `E` expands everything, a second `E` folds everything, leaving the turn as one summary row.
    h.inject_keys(b"E").expect("expand all");
    h.inject_keys(b"E").expect("fold all");
    // The turn has two tool calls, one failed, and the reply the live append added: `2 tools · 1 msg · ✗1`.
    h.wait_for_text("2 tools", Duration::from_secs(5)).expect("collapsed turn counts its tools");
    h.wait_for_text("✗1", Duration::from_secs(5)).expect("collapsed turn counts its failure");
    h.inject_keys(keys::ESC).expect("back to ledger");
    h.wait_for_text_absent("events ·", Duration::from_secs(5)).expect("ledger again");

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
