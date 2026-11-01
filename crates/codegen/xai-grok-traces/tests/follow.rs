//! Synthetic fixtures only; never real session content.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use xai_grok_traces::follow::Follower;
use xai_grok_traces::{Format, Receipt, Root, SessionFilter, TraceIndex};

fn append(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    OpenOptions::new().create(true).append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}

const T1: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/proj","message":{"role":"user","content":"fix the parser\nthen run the tests"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:01Z","message":{"role":"assistant","model":"claude-sonnet-5-5","content":[{"type":"text","text":"I will read the parser first, then patch the off-by-one in the tokenizer so tests pass."}]}}
{"type":"attachment","timestamp":"2026-10-01T00:00:02Z","attachment":{"type":"hook_success"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-sonnet-5-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test -p parser","description":"run the parser tests"}}]}}
"#;
const T2: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:07Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"3 passed"}]}}
{"type":"assistant","timestamp":"2026-10-01T00:00:08Z","message":{"role":"assistant","model":"claude-sonnet-5-5","content":[{"type":"tool_use","id":"t2","name":"Agent","input":{"description":"Check the lexer","prompt":"look"}}]}}
"#;
const CHILD: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:09Z","isSidechain":true,"message":{"role":"user","content":"look at the lexer"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:10Z","message":{"role":"assistant","model":"claude-haiku-4-5","content":[{"type":"tool_use","id":"c1","name":"Read","input":{"file_path":"/w/proj/src/lexer.rs"}}]}}
"#;
const T3: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:12Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","is_error":true,"content":"Error: no such file"}]}}
"#;

/// Parent and subagent paths plus a follower over the parent (already holding `T1`).
fn setup(tmp: &tempfile::TempDir, json: bool, backlog: usize) -> (Follower, std::path::PathBuf, std::path::PathBuf) {
    let projects = tmp.path().join("home/.claude/projects");
    let parent = projects.join("-w/p1.jsonl");
    let kid = projects.join("-w/p1/subagents/agent-a1.jsonl");
    let root = Root { id: "claude-projects".into(), source: "claude".into(), path: projects.clone(), format: Format::Jsonl, file_name: None };

    append(&parent, T1);
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    idx.scan(std::slice::from_ref(&root)).unwrap();
    let session = idx.list_sessions(&SessionFilter::default()).unwrap().remove(0);
    let home = Some(tmp.path().join("home").to_string_lossy().into_owned());
    (Follower::new(idx, vec![root], session, true, home).with_json(json).with_backlog(backlog), parent, kid)
}

/// A receipt for the only indexed session, as its hook would record it.
fn record(tmp: &tempfile::TempDir, event: &str, at_ms: i64, tool_use_id: Option<&str>, agent_id: Option<&str>, tool: Option<&str>, detail: Option<&str>) {
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    let session_id = idx.list_sessions(&SessionFilter::default()).unwrap().remove(0).id;
    let s = |v: Option<&str>| v.map(String::from);
    let r = Receipt { at_ms, source: "claude".into(), event: event.into(), session_id: Some(session_id), tool_use_id: s(tool_use_id), agent_id: s(agent_id), tool: s(tool), detail: s(detail), ..Default::default() };
    idx.record_receipt(&r).unwrap();
}

/// What the `SubagentStart` hook records: a child `a1`. Children are found from receipts.
fn subagent_start(tmp: &tempfile::TempDir) {
    record(tmp, "SubagentStart", 0, None, Some("a1"), None, None);
}

#[test]
fn follow_prints_calls_from_receipts_and_not_again_from_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, parent, _) = setup(&tmp, true, 10);
    assert_eq!(kinds(&f.tick(0).unwrap()), ["model", "user", "text", "call"]);

    // Pre: the call row, with its command, before the transcript has it. Post: closed, duration from the receipts.
    record(&tmp, "PreToolUse", 1_000, Some("p9"), None, Some("Bash"), Some("echo hi"));
    let pre = f.tick(1_000).unwrap();
    assert_eq!(kinds(&pre), ["call"]);
    let call: serde_json::Value = serde_json::from_str(&pre[0]).unwrap();
    assert_eq!((call["call_id"].as_str(), call["tool"].as_str(), call["what"].as_str()), (Some("p9"), Some("Bash"), Some("echo hi")));
    record(&tmp, "PostToolUse", 3_500, Some("p9"), None, Some("Bash"), None);
    let post = f.tick(3_500).unwrap();
    assert_eq!(kinds(&post), ["result"]);
    let result: serde_json::Value = serde_json::from_str(&post[0]).unwrap();
    assert_eq!((result["call_id"].as_str(), result["ms"].as_i64(), result["ok"].as_bool()), (Some("p9"), Some(2500), Some(true)));

    // The transcript catches up: neither its call nor its result is printed a second time; each is a `link`
    // saying which line holds it, so the pane can show the command and the output.
    append(&parent, r#"{"type":"assistant","timestamp":"2026-10-01T00:00:20Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"p9","name":"Bash","input":{"command":"echo hi"}}]}}
{"type":"user","timestamp":"2026-10-01T00:00:21Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"p9","content":"hi"}]}}
"#);
    let linked = f.tick(4_000).unwrap();
    assert_eq!(kinds(&linked), ["link", "link"]);
    let (c, r): (serde_json::Value, serde_json::Value) = (serde_json::from_str(&linked[0]).unwrap(), serde_json::from_str(&linked[1]).unwrap());
    assert_eq!((c["call_id"].as_str(), c["line"].as_i64().is_some(), c["session"].is_string()), (Some("p9"), true, true));
    assert_eq!((r["call_id"].as_str(), r["result_line"].as_i64().is_some(), r["line"].is_null()), (Some("p9"), true, true));
    assert!(f.tick(5_000).unwrap().is_empty());
}

/// Indexes the child file the way a scan would (the Follower no longer probes for them).
fn scan(tmp: &tempfile::TempDir) {
    let projects = tmp.path().join("home/.claude/projects");
    let root = Root { id: "claude-projects".into(), source: "claude".into(), path: projects, format: Format::Jsonl, file_name: None };
    TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap().scan(std::slice::from_ref(&root)).unwrap();
}

#[test]
fn a_child_found_by_its_file_is_not_adopted_again_when_its_receipts_arrive() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, parent, kid) = setup(&tmp, true, 10);
    f.tick(0).unwrap();
    append(&parent, T2);
    append(&kid, CHILD);
    append(&kid.with_extension("meta.json"), r#"{"agentType":"Explore","description":"Check the lexer","finished":false}"#);
    scan(&tmp);
    let now = chrono::Utc::now().timestamp_millis();
    let found = f.tick(now).unwrap();
    let start = found.iter().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|v| v["kind"] == "agent_start").expect("file-born start");
    let agent = start["agent"].as_str().unwrap().to_string();

    // Its first receipt (no SubagentStart in the window): the call lands on the same agent, with no second start.
    record(&tmp, "PreToolUse", now, Some("k1"), Some("a1"), Some("Bash"), Some("ls"));
    let items = f.tick(now + 500).unwrap();
    assert_eq!(kinds(&items), ["call"], "{items:?}");
    let call: serde_json::Value = serde_json::from_str(&items[0]).unwrap();
    assert_eq!((call["agent"].as_str(), call["depth"].as_i64()), (Some(agent.as_str()), Some(1)));
}

#[test]
fn a_pre_for_a_call_the_file_already_printed_is_not_a_second_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, _, _) = setup(&tmp, true, 10);
    assert_eq!(kinds(&f.tick(0).unwrap()), ["model", "user", "text", "call"]);
    // `t1` is in the backlog; its Pre and Post receipts arrive afterwards.
    record(&tmp, "PreToolUse", 1_000, Some("t1"), None, Some("Bash"), Some("cargo test -p parser"));
    assert!(f.tick(1_000).unwrap().is_empty());
    record(&tmp, "PostToolUse", 2_000, Some("t1"), None, Some("Bash"), None);
    assert!(f.tick(2_000).unwrap().is_empty());
}

#[test]
fn a_failed_call_closes_from_its_receipt_and_links_the_real_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, parent, _) = setup(&tmp, true, 10);
    f.tick(0).unwrap();
    record(&tmp, "PreToolUse", 1_000, Some("p8"), None, Some("Bash"), Some("false"));
    f.tick(1_000).unwrap();
    record(&tmp, "PostToolUseFailure", 1_400, Some("p8"), None, Some("Bash"), None);
    let items = f.tick(1_400).unwrap();
    assert_eq!(kinds(&items), ["result"]);
    let r: serde_json::Value = serde_json::from_str(&items[0]).unwrap();
    assert_eq!((r["ok"].as_bool(), r["error"].as_str(), r["ms"].as_i64()), (Some(false), Some("failed"), Some(400)));
    // A denied call closes the same way.
    record(&tmp, "PreToolUse", 2_000, Some("p7"), None, Some("Bash"), Some("rm x"));
    record(&tmp, "PermissionDenied", 2_100, Some("p7"), None, Some("Bash"), None);
    let denied = f.tick(2_100).unwrap();
    assert_eq!(kinds(&denied), ["call", "result"]);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&denied[1]).unwrap()["error"].as_str(), Some("denied"));

    append(&parent, r#"{"type":"assistant","timestamp":"2026-10-01T00:00:20Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"p8","name":"Bash","input":{"command":"false"}}]}}
{"type":"user","timestamp":"2026-10-01T00:00:21Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"p8","is_error":true,"content":"exit 1: boom"}]}}
"#);
    let linked = f.tick(3_000).unwrap();
    assert_eq!(kinds(&linked), ["link", "link"]);
    let r: serde_json::Value = serde_json::from_str(&linked[1]).unwrap();
    assert!(r["error"].as_str().unwrap().contains("boom"), "{r}");
}

#[test]
fn a_stop_receipt_ends_a_child_that_still_has_a_fresh_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, parent, kid) = setup(&tmp, true, 10);
    f.tick(0).unwrap();
    append(&parent, T2);
    append(&kid, CHILD);
    append(&kid.with_extension("meta.json"), r#"{"agentType":"Explore","description":"Check the lexer","finished":false}"#);
    subagent_start(&tmp);
    let now = chrono::Utc::now().timestamp_millis();
    f.tick(now).unwrap();
    record(&tmp, "SubagentStop", now + 100, None, Some("a1"), None, None);
    assert_eq!(kinds(&f.tick(now + 500).unwrap()), ["agent_end"]);
    assert!(f.tick(now + 1_000).unwrap().is_empty(), "the fresh file does not resurrect it");
    // Its next call (a resumed child) brings it back.
    record(&tmp, "PreToolUse", now + 2_000, Some("k2"), Some("a1"), Some("Read"), Some("x.rs"));
    assert_eq!(kinds(&f.tick(now + 2_000).unwrap()), ["agent_resume", "call"]);
}

#[test]
fn follow_replays_only_what_is_in_flight_at_attach_and_after_the_backlog() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, _, _) = setup(&tmp, true, 10);
    let now = chrono::Utc::now().timestamp_millis();
    // History: a closed call, then one still open; both before the follow starts.
    record(&tmp, "PreToolUse", now - 4_000, Some("h1"), None, Some("Bash"), Some("closed"));
    record(&tmp, "PostToolUse", now - 3_500, Some("h1"), None, Some("Bash"), None);
    record(&tmp, "PreToolUse", now - 1_000, Some("h2"), None, Some("Bash"), Some("still running"));
    let first = f.tick(now).unwrap();
    assert_eq!(kinds(&first), ["model", "user", "text", "call", "call"], "backlog first, then the open call");
    let open: serde_json::Value = serde_json::from_str(first.last().unwrap()).unwrap();
    assert_eq!((open["call_id"].as_str(), open["what"].as_str()), (Some("h2"), Some("still running")));
    // The cursor moved past the history: nothing is read twice.
    assert!(f.tick(now + 500).unwrap().is_empty());
}

#[test]
fn a_grok_child_is_followed_through_its_own_session_receipts_and_file() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("home/.grok/sessions");
    let root = Root { id: "grok-sessions".into(), source: "grok".into(), path: sessions.clone(), format: Format::Jsonl, file_name: Some("chat_history.jsonl") };
    let user = |t: &str| format!("{{\"type\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"{t}\"}}]}}\n");
    append(&sessions.join("%2Fw/p1/chat_history.jsonl"), &user("parent task"));
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    idx.scan(std::slice::from_ref(&root)).unwrap();
    let parent = idx.list_sessions(&SessionFilter::default()).unwrap().remove(0);
    let mut f = Follower::new(idx, vec![root], parent.clone(), true, None).with_json(true);
    f.tick(0).unwrap();

    let rec = |event: &str, at_ms: i64, session: Option<&str>, native: &str, id: Option<&str>, agent: Option<&str>, detail: Option<&str>| {
        let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
        let r = Receipt {
            at_ms,
            source: "grok".into(),
            event: event.into(),
            session_id: session.map(String::from),
            native_session: Some(native.into()),
            tool_use_id: id.map(String::from),
            agent_id: agent.map(String::from),
            tool: id.map(|_| "Bash".to_string()),
            detail: detail.map(String::from),
            ..Default::default()
        };
        idx.record_receipt(&r).unwrap();
    };
    // `SubagentStart` lands on the parent (`subagentId` is the agent id); the child's calls on the child's session.
    rec("SubagentStart", 1_000, Some(&parent.id), "p1", None, Some("k1"), None);
    rec("PreToolUse", 1_100, None, "k1", Some("g1"), None, Some("ls -la"));
    let items = f.tick_items(1_200).unwrap();
    let kinds: Vec<_> = items.iter().map(|v| v["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["agent_start", "call"], "{items:?}");
    assert_eq!((items[1]["agent"].as_str(), items[1]["depth"].as_i64(), items[1]["what"].as_str()), (Some("k1"), Some(1), Some("ls -la")));

    // Its transcript is its own session dir: once there, the child's rows come from it.
    append(&sessions.join("%2Fw/k1/chat_history.jsonl"), &user("child task"));
    let items = f.tick_items(1_300).unwrap();
    let kinds: Vec<_> = items.iter().map(|v| v["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["user"], "{items:?}");
    assert_eq!((items[0]["agent"].as_str(), items[0]["text"].as_str()), (Some("k1"), Some("child task")));

    // The child's own `SubagentStop` (its turn-end gate) ends it under the parent.
    rec("SubagentStop", 1_900, None, "k1", None, Some("k1"), None);
    let items = f.tick_items(2_000).unwrap();
    let kinds: Vec<_> = items.iter().map(|v| v["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["agent_end"], "{items:?}");
}

#[test]
fn follow_prints_new_rows_per_tick_and_children_under_their_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, parent, kid) = setup(&tmp, false, 10);

    // Tick 1: header, the backlog, nothing for bookkeeping lines.
    let one = f.tick(0).unwrap().join("\n");
    assert_eq!(
        one,
        "claude · sonnet-5-5 · fix the parser · /w/proj\n\
         \n▶ you  fix the parser (+1 lines)\n\
         ·  I will read the parser first, then patch the off-by-one in the tokenizer so tests pass.\n\
         → Bash  run the parser tests"
    );

    // Quiet tick: nothing until 30 s, then one idle line, then every 60 s.
    assert!(f.tick(29_000).unwrap().is_empty());
    assert_eq!(f.tick(30_000).unwrap(), ["… idle 30s"]);
    assert!(f.tick(31_000).unwrap().is_empty());
    assert_eq!(f.tick(90_000).unwrap(), ["… idle 1m30s"]);

    // Tick 2: the result (duration from the call), a new call, and a subagent that appeared meanwhile.
    append(&parent, T2);
    append(&kid, CHILD);
    append(&kid.with_extension("meta.json"), r#"{"agentType":"Explore","description":"Check the lexer","finished":false}"#);
    subagent_start(&tmp);
    let now = chrono::Utc::now().timestamp_millis();
    let two = f.tick(now).unwrap().join("\n");
    // The start comes from the receipt (before the parent's rows, no model yet); once its file is found the child's
    // model and label follow, then its rows.
    assert_eq!(
        two,
        "  ↳ started\n\
         \x20  ✓ 2.0s\n\
         → Agent  Check the lexer\n\
         \x20 ↳ haiku-4-5  Check the lexer\n\
         \n  ▶ you  look at the lexer\n\
         \x20 → Read  src/lexer.rs"
    );

    // Tick 3: the child's error result, then the child is marked finished.
    append(&kid, T3);
    fs::write(kid.with_extension("meta.json"), r#"{"agentType":"Explore","description":"Check the lexer","finished":true}"#).unwrap();
    let three = f.tick(now + 500).unwrap().join("\n");
    assert_eq!(three, "     ✗ Error: no such file\n  ↳ ended");

    // Ended children print once.
    assert!(f.tick(now + 1_000).unwrap().is_empty());
}

fn kinds(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["kind"].as_str().unwrap().to_string()).collect()
}

#[test]
fn follow_json_streams_items_and_subagent_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, parent, kid) = setup(&tmp, true, 10);

    // No header; the root's model, then the backlog.
    assert_eq!(kinds(&f.tick(0).unwrap()), ["model", "user", "text", "call"]);
    assert!(f.tick(30_000).unwrap().is_empty(), "no idle item");

    append(&parent, T2);
    append(&kid, CHILD);
    let meta = kid.with_extension("meta.json");
    append(&meta, r#"{"agentType":"Explore","description":"Check the lexer","finished":false}"#);
    subagent_start(&tmp);
    let now = chrono::Utc::now().timestamp_millis();
    let two = f.tick(now).unwrap();
    // The start comes from the receipt, so it leads and carries no model; the model and label follow as their own item.
    assert_eq!(kinds(&two), ["agent_start", "result", "call", "model", "user", "call"]);
    let start: serde_json::Value = serde_json::from_str(&two[0]).unwrap();
    assert_eq!((start["agent"].as_str(), start["depth"].as_i64(), start["model"].as_str()), (Some("a1"), Some(1), None));
    let model: serde_json::Value = serde_json::from_str(&two[3]).unwrap();
    assert_eq!((model["agent"].as_str(), model["model"].as_str(), model["label"].as_str()), (Some("a1"), Some("claude-haiku-4-5"), Some("Check the lexer")));
    let first: serde_json::Value = serde_json::from_str(&two[1]).unwrap();
    assert_eq!(first["call_id"].as_str(), Some("t1"));
    assert_eq!(first["ms"].as_i64(), Some(2000), "result items carry the call's duration");

    // A terminal last event ends it; the sidecar's `finished` flag cannot be undone, so resume rides on new rows.
    append(&kid, T3);
    append(&kid, "{\"type\":\"session_end\",\"timestamp\":\"2026-10-01T00:00:13Z\"}\n");
    assert_eq!(kinds(&f.tick(now + 500).unwrap()), ["result", "agent_end"]);
    assert!(f.tick(now + 1_000).unwrap().is_empty(), "end is emitted once");

    // The subagent is dispatched again: it resumes (never an item for the root).
    append(&kid, CHILD);
    assert_eq!(kinds(&f.tick(now + 1_500).unwrap()), ["agent_resume", "user", "call"]);
}

#[test]
fn tick_items_returns_the_json_items_as_values() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, _, _) = setup(&tmp, true, 10);
    let items = f.tick_items(0).unwrap();
    let kinds: Vec<_> = items.iter().map(|v| v["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["model", "user", "text", "call"]);
    let call = &items[3];
    assert_eq!((call["tool"].as_str(), call["what"].as_str(), call["call_id"].as_str()), (Some("Bash"), Some("run the parser tests"), Some("t1")));
}

#[test]
fn follow_backlog_limits_the_first_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, _, _) = setup(&tmp, true, 1);
    assert_eq!(kinds(&f.tick(0).unwrap()), ["model", "call"]);
}

#[test]
fn follow_backlog_zero_starts_after_the_last_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut f, _, _) = setup(&tmp, true, 0);
    assert_eq!(kinds(&f.tick(0).unwrap()), ["model"]);
}

#[test]
fn follow_waits_for_a_live_session_then_attaches_and_sees_the_root_end() {
    use xai_grok_traces::follow::find_live_root;
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("home/.claude/projects");
    let root = Root { id: "claude-projects".into(), source: "claude".into(), path: projects.clone(), format: Format::Jsonl, file_name: None };
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();

    // Nothing live yet: no root, by default or by id.
    idx.scan(std::slice::from_ref(&root)).unwrap();
    assert!(find_live_root(&idx, None).unwrap().is_none());
    assert!(find_live_root(&idx, Some("p1")).unwrap().is_none());

    // A session appears and becomes the one to attach to.
    let parent = projects.join("-w/p1.jsonl");
    append(&parent, T1);
    idx.scan(std::slice::from_ref(&root)).unwrap();
    let live = find_live_root(&idx, None).unwrap().expect("live root");
    assert_eq!(find_live_root(&idx, Some(&live.id)).unwrap().map(|s| s.id), Some(live.id.clone()));

    // Its end flips `root_ended`, which is what sends the CLI back to waiting.
    let mut f = Follower::new(idx, vec![root], live, true, None).with_json(true);
    assert!(f.tick(0).is_ok() && !f.root_ended());
    append(&parent, "{\"type\":\"session_end\",\"timestamp\":\"2026-10-01T00:00:13Z\"}\n");
    // The status is read before the file is re-indexed, so the end shows one tick later.
    f.tick(500).unwrap();
    assert_eq!(kinds(&f.tick(1_000).unwrap()), ["agent_end"]);
    assert!(f.root_ended());
}

#[test]
fn follow_an_ended_session_prints_its_tail_then_resumes_in_place() {
    use xai_grok_traces::follow::{recent_table, ended_line};
    use xai_grok_traces::live::recent_ended;
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("home/.claude/projects");
    let root = Root { id: "claude-projects".into(), source: "claude".into(), path: projects.clone(), format: Format::Jsonl, file_name: None };
    let parent = projects.join("-w/p1.jsonl");
    append(&parent, T1);
    append(&parent, "{\"type\":\"session_end\",\"timestamp\":\"2026-10-01T00:00:13Z\"}\n");
    // Old file: the session is ended, not active.
    fs::File::options().write(true).open(&parent).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_900_000)).unwrap();
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    idx.scan(std::slice::from_ref(&root)).unwrap();
    assert!(xai_grok_traces::follow::find_live_root(&idx, None).unwrap().is_none());

    // The idle listing names it.
    let now = chrono::Utc::now().timestamp_millis();
    let recent = recent_ended(&idx, &[], 10, now).unwrap();
    let table = recent_table(&recent, now);
    assert!(table.starts_with("claude:") && table.contains(" ago  fix the parser"), "{table}");

    // Following it by id: header, tail, then the end line, in text.
    let session = idx.session(&recent[0].session_id).unwrap().unwrap();
    let mut f = Follower::new(idx, vec![root], session, true, None).with_json(true).with_backlog(2);
    let first = f.tick(0).unwrap();
    assert_eq!(kinds(&first), ["model", "call", "agent_end"], "{first:?}");
    assert!(f.root_ended());
    assert_eq!(ended_line(0), "■ ended");

    // New activity makes it live again: agent_resume, then the new rows; no remount.
    append(&parent, T2);
    fs::File::options().write(true).open(&parent).unwrap().set_modified(std::time::SystemTime::now()).unwrap();
    f.tick(500).unwrap();
    let again = kinds(&f.tick(1_000).unwrap());
    assert_eq!(again.first().map(String::as_str), Some("agent_resume"), "{again:?}");
    assert!(!f.root_ended());
}
