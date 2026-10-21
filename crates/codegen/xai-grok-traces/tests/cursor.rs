//! Synthetic Cursor transcripts only; never real session content.

use std::fs;
use std::path::Path;

use xai_grok_traces::roots::root_for;
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

    let roots = default_roots(&home, &home.join(".grok"));
    assert_eq!(root_for(&file, &roots).map(|r| r.source.as_str()), Some("cursor"));
    assert!(root_for(&home.join(".cursor/projects/w-proj/repo.json"), &roots).is_none());

    let mut idx = TraceIndex::open(&tmp.path().join("idx/t.sqlite")).unwrap();
    let report = idx.scan(&roots).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let rows = idx.list_sessions(&SessionFilter { source: Some("cursor".into()), ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let s = &rows[0];
    assert_eq!(s.title, "fix the bug", "user_query wrapper is unwrapped for the title");
    assert_eq!((s.event_count, s.user_count, s.assistant_count, s.tool_count), (4, 1, 1, 1));
    assert_ne!(s.status, Status::Completed, "turn_ended ends a turn, not the session");

    let events = idx.list_events(&s.id, &EventFilter::default()).unwrap();
    assert_eq!(events[0].role.as_deref(), Some("user"));
    assert_eq!((events[1].kind.as_str(), events[1].title.as_str()), ("tool", "Shell"));
    let shell = idx.list_events(&s.id, &EventFilter { tool: Some("Shell".into()), ..Default::default() }).unwrap();
    assert_eq!(shell.len(), 1);
    assert_eq!(idx.raw_line(&s.id, 1).unwrap().unwrap(), CURSOR.lines().nth(1).unwrap(), "byte for byte");
}

#[test]
fn receipts_fall_back_to_native_id_or_stay_unkeyed() {
    use xai_grok_traces::hook::{extract, ingest};
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write(&home.join(".cursor/projects/w-proj/agent-transcripts/abc/abc.jsonl"), CURSOR);
    let roots = default_roots(&home, &home.join(".grok"));
    let mut idx = TraceIndex::open(&tmp.path().join("idx/t.sqlite")).unwrap();
    let key = |idx: &TraceIndex, id: i64| -> (Option<String>, Option<String>) {
        let c = rusqlite::Connection::open(idx.db_path()).unwrap();
        c.query_row("SELECT session_id, native_session FROM trace_receipts WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
    };

    // Before the transcript is indexed: stored, unkeyed, not dropped.
    let early = br#"{"hook_event_name":"sessionStart","conversation_id":"abc","transcript_path":null}"#;
    let id = ingest(&mut idx, &roots, extract("cursor", early)).unwrap();
    assert_eq!(key(&idx, id), (None, Some("abc".into())));

    idx.scan(&roots).unwrap();
    let row = idx.list_sessions(&SessionFilter { source: Some("cursor".into()), ..Default::default() }).unwrap().remove(0);
    let late = br#"{"hook_event_name":"preToolUse","conversation_id":"abc","transcript_path":null,"tool_name":"Shell"}"#;
    let id = ingest(&mut idx, &roots, extract("cursor", late)).unwrap();
    assert_eq!(key(&idx, id), (Some(row.id.clone()), Some("abc".into())), "native_id = stem matches");

    // Same id in another source does not match; an unknown chat stays unkeyed.
    let other = br#"{"hook_event_name":"PreToolUse","session_id":"abc"}"#;
    let id = ingest(&mut idx, &roots, extract("claude", other)).unwrap();
    assert_eq!(key(&idx, id).0, None);
    let none = br#"{"hook_event_name":"stop"}"#;
    let id = ingest(&mut idx, &roots, extract("cursor", none)).unwrap();
    assert_eq!(key(&idx, id), (None, None));
}
