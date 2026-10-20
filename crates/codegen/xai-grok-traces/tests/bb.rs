//! bb threads indexed in place from a synthetic bb.db; never real bb content.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use xai_grok_traces::hook::{extract, ingest};
use xai_grok_traces::{EventFilter, MAX_EVENTS_PER_FILE, SessionFilter, SessionRow, Status, TraceIndex, default_roots, root_for};

const BASE: i64 = 1_790_000_000_000;

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    bb: PathBuf,
    idx_db: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let bb = home.join(".bb/bb.db");
    std::fs::create_dir_all(bb.parent().unwrap()).unwrap();
    let c = Connection::open(&bb).unwrap();
    c.execute_batch(
        "CREATE TABLE environments (id TEXT PRIMARY KEY, path TEXT);
         CREATE TABLE threads (id TEXT PRIMARY KEY, project_id TEXT, environment_id TEXT, provider_id TEXT NOT NULL,
           model_override TEXT, title TEXT, title_fallback TEXT, status TEXT NOT NULL, parent_thread_id TEXT,
           archived_at INTEGER, deleted_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE events (id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, sequence INTEGER NOT NULL, type TEXT NOT NULL,
           item_id TEXT, item_kind TEXT, turn_id TEXT, parent_tool_call_id TEXT, data TEXT NOT NULL DEFAULT '{}',
           created_at INTEGER NOT NULL);
         CREATE UNIQUE INDEX events_thread_sequence_idx ON events (thread_id, sequence);
         INSERT INTO environments VALUES ('env1', '/w/bb');
         INSERT INTO threads VALUES ('thr_a', 'p', 'env1', 'codex', NULL, 'Fix the bug', NULL, 'idle', NULL, NULL, NULL, 1000, 9000);
         INSERT INTO threads VALUES ('thr_b', 'p', NULL, 'pi', NULL, NULL, NULL, 'active', NULL, NULL, NULL, 1000, 8000);
         INSERT INTO threads VALUES ('thr_empty', 'p', NULL, 'pi', NULL, 'nothing yet', NULL, 'starting', NULL, NULL, NULL, 1000, 7000);",
    )
    .unwrap();
    let rows: [(i64, &str, Option<&str>, &str); 9] = [
        (1, "client/turn/requested", None, r#"{"input":[{"type":"text","text":"fix the bug"}],"execution":{"model":"gpt-5"}}"#),
        (2, "item/agentMessage/delta", None, r#"{"delta":"Loo"}"#),
        (3, "item/completed", Some("agentMessage"), r#"{"item":{"id":"i1","text":"Looking."}}"#),
        (4, "item/completed", Some("commandExecution"), r#"{"item":{"command":"ls","status":"completed","durationMs":12}}"#),
        (5, "item/completed", Some("toolCall"), r#"{"item":{"tool":"read_file","arguments":{"path":"a"},"status":"failed"}}"#),
        (6, "provider/unhandled", None, r#"{"raw":1}"#),
        (7, "thread/tokenUsage/updated", None, r#"{"tokenUsage":{"total":{"inputTokens":500,"outputTokens":50}}}"#),
        (8, "provider/error", None, r#"{"message":"rate limited"}"#),
        (9, "turn/completed", None, r#"{"status":"completed"}"#),
    ];
    for (seq, ty, kind, data) in rows {
        add_event(&c, "thr_a", seq, ty, kind, data);
    }
    add_event(&c, "thr_b", 1, "client/turn/requested", None, r#"{"input":[{"type":"text","text":"second thread"}]}"#);
    add_event(&c, "thr_b", 2, "item/completed", Some("agentMessage"), r#"{"item":{"id":"j1","text":"working"}}"#);
    Env { idx_db: tmp.path().join("idx/traces.sqlite"), _tmp: tmp, home, bb }
}

fn add_event(c: &Connection, thread: &str, seq: i64, ty: &str, kind: Option<&str>, data: &str) {
    c.execute(
        "INSERT INTO events (id, thread_id, sequence, type, item_kind, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![format!("evt_{thread}_{seq}"), thread, seq, ty, kind, data, BASE + seq * 1000],
    )
    .unwrap();
}

fn rw(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

fn sessions(idx: &TraceIndex) -> Vec<SessionRow> {
    idx.list_sessions(&SessionFilter { source: Some("bb".into()), ..Default::default() }).unwrap()
}

fn thread(idx: &TraceIndex, title: &str) -> SessionRow {
    sessions(idx).into_iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no bb session titled {title}"))
}

#[test]
fn threads_are_indexed_by_pointer_and_empty_threads_skipped() {
    let e = env();
    let roots = default_roots(&e.home, &e.home.join(".grok"));
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    let report = idx.scan(&roots).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let r = report.roots.iter().find(|r| r.id == "bb-threads").unwrap();
    assert_eq!((r.accessible, r.files), (true, 2));
    assert_eq!(sessions(&idx).len(), 2, "thread without events is skipped");

    let a = thread(&idx, "Fix the bug");
    // Indexed: 1, 3, 4, 5, 7, 8, 9; deltas and provider/unhandled are not.
    assert_eq!((a.event_count, a.user_count, a.assistant_count, a.tool_count), (7, 1, 1, 2));
    assert_eq!(a.error_count, 2, "failed tool call + provider/error");
    assert_eq!((a.input_tokens, a.output_tokens), (Some(500), Some(50)));
    assert_eq!((a.model.as_deref(), a.cwd.as_deref()), (Some("gpt-5"), Some("/w/bb")));
    assert_eq!(a.status, Status::Completed, "turn/completed is terminal; item/completed is not");
    assert!(a.file_path.to_string_lossy().ends_with("bb.db#thr_a"));

    assert_eq!(thread(&idx, "second thread").status, Status::Idle, "no terminal event, updated_at long ago");

    let lines: Vec<i64> = idx.list_events(&a.id, &EventFilter::default()).unwrap().iter().map(|e| e.line).collect();
    assert_eq!(lines, [1, 3, 4, 5, 7, 8, 9], "line number is the event sequence");
    let raw: serde_json::Value = serde_json::from_str(&idx.raw_line(&a.id, 3).unwrap().unwrap()).unwrap();
    assert_eq!((raw["id"].as_str(), raw["data"]["item"]["text"].as_str()), (Some("evt_thr_a_3"), Some("Looking.")));
    assert!(idx.raw_line(&a.id, 2).unwrap().is_none(), "skipped events are not indexed");

    assert!(root_for(&e.bb, &roots).is_none(), "bb.db is not a file source");
}

#[test]
fn rescan_is_incremental_and_follows_deletes() {
    let e = env();
    let roots = default_roots(&e.home, &e.home.join(".grok"));
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    idx.scan(&roots).unwrap();
    assert_eq!(idx.scan(&roots).unwrap().indexed, 0, "nothing changed");

    let c = rw(&e.bb);
    add_event(&c, "thr_b", 3, "item/completed", Some("toolCall"), r#"{"item":{"tool":"grep","status":"completed"}}"#);
    c.execute("UPDATE threads SET updated_at = 9500 WHERE id = 'thr_b'", []).unwrap();
    assert_eq!(idx.scan(&roots).unwrap().indexed, 1, "only the changed thread");
    let b = thread(&idx, "second thread");
    assert_eq!((b.event_count, b.tool_count), (3, 1));
    let lines: Vec<i64> = idx.list_events(&b.id, &EventFilter::default()).unwrap().iter().map(|e| e.line).collect();
    assert_eq!(lines, [1, 2, 3], "no duplicates after append");

    // A title change alone refreshes the session.
    c.execute("UPDATE threads SET title = 'renamed', updated_at = 9600 WHERE id = 'thr_b'", []).unwrap();
    idx.scan(&roots).unwrap();
    assert_eq!(thread(&idx, "renamed").event_count, 3);

    c.execute("UPDATE threads SET deleted_at = 1 WHERE id = 'thr_a'", []).unwrap();
    idx.scan(&roots).unwrap();
    assert_eq!(sessions(&idx).len(), 1, "deleted thread is pruned");
}

#[test]
fn missing_db_is_a_quiet_empty_root() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let roots = default_roots(&home, &home.join(".grok"));
    let mut idx = TraceIndex::open(&tmp.path().join("idx/t.sqlite")).unwrap();
    let report = idx.scan(&roots).unwrap();
    assert!(report.errors.is_empty());
    let r = report.roots.iter().find(|r| r.id == "bb-threads").unwrap();
    assert_eq!((r.accessible, r.files), (false, 0));
    assert!(sessions(&idx).is_empty());
}

#[test]
fn unreadable_db_keeps_cached_rows() {
    let e = env();
    let roots = default_roots(&e.home, &e.home.join(".grok"));
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    idx.scan(&roots).unwrap();
    std::fs::write(&e.bb, "not a database, just bytes ........................................").unwrap();
    let report = idx.scan(&roots).unwrap();
    assert!(report.errors.is_empty());
    assert_eq!(sessions(&idx).len(), 2, "cached rows survive a bad db");
}

#[test]
fn hook_indexes_the_given_thread_and_marks_it_live() {
    let e = env();
    let roots = default_roots(&e.home, &e.home.join(".grok"));
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    let payload = br#"{"hook_event_name":"item/completed","thread_id":"thr_b"}"#;
    ingest(&mut idx, &roots, extract("bb", payload)).unwrap();
    let rows = sessions(&idx);
    assert_eq!(rows.len(), 1, "only the named thread");
    assert_eq!(rows[0].event_count, 2);
    assert_eq!(idx.last_receipt(&rows[0].id).unwrap().unwrap().1, "item/completed");
    assert_eq!(rows[0].status, Status::Active);

    // No thread id, or an unknown one: the receipt is recorded and nothing else happens.
    let before = idx.receipts_watermark().unwrap();
    ingest(&mut idx, &roots, extract("bb", br#"{"event":"x"}"#)).unwrap();
    ingest(&mut idx, &roots, extract("bb", br#"{"event":"x","thread_id":"thr_nope"}"#)).unwrap();
    assert_eq!(idx.receipts_watermark().unwrap(), before + 2);
    assert_eq!(sessions(&idx).len(), 1);
}

#[test]
fn event_cap_stops_a_runaway_thread() {
    let e = env();
    let c = rw(&e.bb);
    c.execute(
        "WITH RECURSIVE n(i) AS (SELECT 10 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
         INSERT INTO events (id, thread_id, sequence, type, item_kind, data, created_at)
         SELECT 'big_' || i, 'thr_b', i, 'item/completed', 'agentMessage', '{\"item\":{\"text\":\"x\"}}', ?2 + i FROM n",
        params![MAX_EVENTS_PER_FILE as i64 + 20, BASE],
    )
    .unwrap();
    drop(c);
    let roots = default_roots(&e.home, &e.home.join(".grok"));
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    idx.scan(&roots).unwrap();
    assert_eq!(thread(&idx, "second thread").event_count, MAX_EVENTS_PER_FILE as i64);
    assert_eq!(idx.scan(&roots).unwrap().indexed, 0, "capped thread is not rescanned");
}
