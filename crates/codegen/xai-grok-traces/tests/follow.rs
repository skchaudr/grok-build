//! Synthetic fixtures only; never real session content.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use xai_grok_traces::follow::Follower;
use xai_grok_traces::{Format, Root, SessionFilter, TraceIndex};

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
    let now = chrono::Utc::now().timestamp_millis();
    let two = f.tick(now).unwrap().join("\n");
    assert_eq!(
        two,
        "   ✓ 2.0s\n\
         → Agent  Check the lexer\n\
         \x20 ↳ haiku-4-5 started\n\
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
    let now = chrono::Utc::now().timestamp_millis();
    let two = f.tick(now).unwrap();
    assert_eq!(kinds(&two), ["result", "call", "agent_start", "user", "call"]);
    let start: serde_json::Value = serde_json::from_str(&two[2]).unwrap();
    assert_eq!((start["depth"].as_i64(), start["model"].as_str(), start["label"].as_str()), (Some(1), Some("claude-haiku-4-5"), Some("Check the lexer")));
    let first: serde_json::Value = serde_json::from_str(&two[0]).unwrap();
    assert_eq!(first["call_id"].as_str(), Some("t1"));

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
