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

#[test]
fn follow_prints_new_rows_per_tick_and_children_under_their_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("home/.claude/projects");
    let parent = projects.join("-w/p1.jsonl");
    let kid = projects.join("-w/p1/subagents/agent-a1.jsonl");
    let root = Root { id: "claude-projects".into(), source: "claude".into(), path: projects.clone(), format: Format::Jsonl, file_name: None };

    append(&parent, T1);
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    idx.scan(std::slice::from_ref(&root)).unwrap();
    let session = idx.list_sessions(&SessionFilter::default()).unwrap().remove(0);
    let mut f = Follower::new(idx, vec![root], session, true, Some(tmp.path().join("home").to_string_lossy().into_owned()));

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
