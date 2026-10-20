//! Synthetic Cursor transcripts only; never real session content.

use std::fs;
use std::path::Path;

use xai_grok_traces::{EventFilter, SessionFilter, Status, TraceIndex, default_roots};

const CURSOR: &str = r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Tuesday, Oct 20, 2026</timestamp>\n<user_query>\nfix the bug\n</user_query>"}]}}
{"role":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","name":"Shell","input":{"command":"ls"}}]}}
{"role":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}
{"type":"turn_ended","status":"success"}
"#;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn cursor_transcripts_are_indexed_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let file = home.join(".cursor/projects/w-proj/agent-transcripts/abc/abc.jsonl");
    write(&file, CURSOR);
    // Not a transcript: Cursor's per-project metadata sits beside it.
    write(&home.join(".cursor/projects/w-proj/repo.json"), r#"{"id":"x"}"#);

    let mut idx = TraceIndex::open(&tmp.path().join("t.sqlite")).unwrap();
    let report = idx.scan(&default_roots(&home, &home.join(".grok"))).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let rows = idx.list_sessions(&SessionFilter { source: Some("cursor".into()), ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let s = &rows[0];
    assert_eq!(s.title, "fix the bug", "user_query wrapper is unwrapped for the title");
    assert_eq!((s.event_count, s.user_count, s.assistant_count, s.tool_count), (4, 1, 1, 1));
    assert_eq!(s.status, Status::Completed, "turn_ended is terminal");

    let events = idx.list_events(&s.id, &EventFilter::default()).unwrap();
    assert_eq!(events[0].role.as_deref(), Some("user"));
    assert_eq!((events[1].kind.as_str(), events[1].title.as_str()), ("tool", "Shell"));
    let shell = idx.list_events(&s.id, &EventFilter { tool: Some("Shell".into()), ..Default::default() }).unwrap();
    assert_eq!(shell.len(), 1);
    assert_eq!(idx.raw_line(&s.id, 1).unwrap().unwrap(), CURSOR.lines().nth(1).unwrap(), "byte for byte");
}
