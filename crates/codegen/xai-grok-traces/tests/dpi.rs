//! dpi agents indexed in place from a synthetic data dir (catalog.sqlite + sessions/*/agent.sqlite); never real dpi content.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use xai_grok_traces::hook::{extract, ingest};
use xai_grok_traces::{EventFilter, Format, Root, SessionFilter, SessionRow, Status, TraceIndex};

const BASE: i64 = 1_790_000_000_000;

struct Env {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    idx_db: PathBuf,
}

fn ts(n: i64) -> i64 {
    BASE + n * 1000
}

fn db_of(dir: &Path, session: &str) -> PathBuf {
    dir.join("sessions").join(session).join("agent.sqlite")
}

fn add(c: &Connection, id: i64, conv: i64, rec: Value) {
    c.execute("INSERT INTO entries VALUES (?1, ?2, NULL, ?1, ?3)", params![id, conv, rec.to_string()]).unwrap();
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("backend");
    for s in ["sess-a", "sess-b"] {
        std::fs::create_dir_all(dir.join("sessions").join(s)).unwrap();
        let c = Connection::open(db_of(&dir, s)).unwrap();
        c.execute_batch(
            "CREATE TABLE conversations (id INTEGER PRIMARY KEY, owner_conversation_id INTEGER, owner_task_id INTEGER, record TEXT NOT NULL);
             CREATE TABLE entries (id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL, head INTEGER, commit_seq INTEGER NOT NULL, record TEXT NOT NULL);
             INSERT INTO conversations VALUES (1, NULL, NULL, '{\"id\":1}');",
        )
        .unwrap();
    }
    Connection::open(dir.join("catalog.sqlite"))
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY,name TEXT NOT NULL,cwd TEXT NOT NULL,dbPath TEXT NOT NULL UNIQUE,createdAt INTEGER NOT NULL,model TEXT NOT NULL,provider TEXT NOT NULL);
             CREATE TABLE conversation_names(sessionId TEXT NOT NULL,conversationId INTEGER NOT NULL,name TEXT NOT NULL,PRIMARY KEY(sessionId,conversationId));
             INSERT INTO sessions VALUES ('sess-a','Fix the bug','/w/dpi','x',{BASE},'default-model','cliproxy');
             INSERT INTO sessions VALUES ('sess-b','Second','/w/other','y',{BASE},'m2','cliproxy');
             INSERT INTO conversation_names VALUES ('sess-a',3,'reviewer');"
        ))
        .unwrap();

    let a = Connection::open(db_of(&dir, "sess-a")).unwrap();
    a.execute_batch("INSERT INTO conversations VALUES (3, 1, 2, '{\"id\":3}'); INSERT INTO conversations VALUES (5, NULL, NULL, '{\"id\":5}');").unwrap();
    add(&a, 9, 1, json!({"model":[{"role":"user","content":"fix the bug","timestamp":ts(1)}],"kind":"pi.user","id":9,"conversationId":1}));
    add(&a, 12, 1, json!({"model":[{"role":"system","content":"","sections":{"p":"huge"}}],"kind":"pi.system","id":12,"conversationId":1}));
    add(&a, 13, 1, json!({"model":[{"role":"assistant","content":[
        {"type":"toolCall","id":"call_1","name":"bash","arguments":{"command":"ls -la"}}],
        "model":"claude-sonnet-5-5","usage":{"input":4,"output":65,"cacheRead":100,"cacheWrite":50,"totalTokens":219},
        "stopReason":"toolUse","timestamp":ts(2)}],"kind":"pi.assistant","id":13,"conversationId":1}));
    add(&a, 15, 1, json!({"model":[{"role":"toolResult","toolCallId":"call_1","toolName":"bash","content":[{"type":"text","text":"a.txt\n"}],
        "isError":false,"timestamp":ts(5)}],"kind":"pi.tool-result","id":15,"conversationId":1}));
    add(&a, 17, 1, json!({"model":[{"role":"assistant","content":[{"type":"toolCall","id":"call_2","name":"bash","arguments":{"command":"false"}}],
        "model":"claude-sonnet-5-5","timestamp":ts(6)}],"kind":"pi.assistant","id":17,"conversationId":1}));
    add(&a, 19, 1, json!({"model":[{"role":"toolResult","toolCallId":"call_2","toolName":"bash","content":[{"type":"text","text":"boom"}],
        "isError":true,"timestamp":ts(7)}],"kind":"pi.tool-result","id":19,"conversationId":1}));
    add(&a, 20, 1, json!({"model":[{"role":"assistant","content":[{"type":"text","text":"Done."}],"model":"claude-sonnet-5-5","timestamp":ts(8)}],"kind":"pi.assistant","id":20,"conversationId":1}));
    add(&a, 21, 1, json!({"kind":"pi.reset","id":21,"conversationId":1,"head":21}));
    add(&a, 30, 3, json!({"model":[{"role":"user","content":"review it","timestamp":ts(3)}],"kind":"pi.user","id":30,"conversationId":3}));
    add(&a, 31, 3, json!({"model":[{"role":"assistant","content":[{"type":"text","text":"looks fine"}],"model":"m3","timestamp":ts(4)}],"kind":"pi.assistant","id":31,"conversationId":3}));
    let b = Connection::open(db_of(&dir, "sess-b")).unwrap();
    add(&b, 9, 1, json!({"model":[{"role":"user","content":"second","timestamp":ts(1)}],"kind":"pi.user","id":9,"conversationId":1}));

    Env { idx_db: tmp.path().join("idx/traces.sqlite"), _tmp: tmp, dir }
}

fn roots(e: &Env) -> Vec<Root> {
    vec![Root { id: "dpi-sessions".into(), source: "dpi".into(), path: e.dir.clone(), format: Format::Sqlite, file_name: None }]
}

fn sessions(idx: &TraceIndex) -> Vec<SessionRow> {
    idx.list_sessions(&SessionFilter { source: Some("dpi".into()), ..Default::default() }).unwrap()
}

fn by_title(idx: &TraceIndex, title: &str) -> SessionRow {
    sessions(idx).into_iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no dpi session titled {title}"))
}

#[test]
fn conversations_are_indexed_as_sessions_by_pointer() {
    let e = env();
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    let report = idx.scan(&roots(&e)).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let r = report.roots.iter().find(|r| r.id == "dpi-sessions").unwrap();
    assert_eq!((r.accessible, r.files), (true, 3), "the conversation without entries is skipped");
    assert_eq!(sessions(&idx).len(), 3);

    let a = by_title(&idx, "Fix the bug");
    // Indexed: 9, 13, 15, 17, 19, 20, 21; pi.system is not.
    assert_eq!((a.event_count, a.user_count, a.assistant_count, a.tool_count), (7, 1, 1, 4), "calls and results count, as for Pi: {a:?}");
    assert_eq!(a.error_count, 1, "the isError tool result");
    assert_eq!((a.model.as_deref(), a.cwd.as_deref()), (Some("claude-sonnet-5-5"), Some("/w/dpi")));
    assert_eq!(a.native_id.as_deref(), Some("sess-a:1"));
    assert_eq!(a.agent_type.as_deref(), Some("cliproxy"));
    assert!(a.file_path.to_string_lossy().ends_with("sess-a/agent.sqlite#1"));
    assert_eq!(a.status, Status::Idle);

    let all = EventFilter { include_noise: true, ..Default::default() };
    let lines: Vec<i64> = idx.list_events(&a.id, &all).unwrap().iter().map(|e| e.line).collect();
    assert_eq!(lines, [9, 13, 15, 17, 19, 20, 21], "line number is the entry id");
    let tool = idx.list_events(&a.id, &all).unwrap().into_iter().find(|e| e.line == 13).unwrap();
    assert_eq!(tool.duration_ms, Some(3000), "call to result");

    let raw: Value = serde_json::from_str(&idx.raw_line(&a.id, 20).unwrap().unwrap()).unwrap();
    assert_eq!((raw["id"].as_i64(), raw["record"]["kind"].as_str()), (Some(20), Some("pi.assistant")));
    assert!(idx.raw_line(&a.id, 12).unwrap().is_none(), "skipped entries are not indexed");

    assert_eq!(by_title(&idx, "Second").native_id.as_deref(), Some("sess-b:1"));
}

#[test]
fn subagent_conversation_links_to_its_owner() {
    let e = env();
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    idx.scan(&roots(&e)).unwrap();
    let parent = by_title(&idx, "Fix the bug");
    let child = by_title(&idx, "reviewer");
    assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!((child.native_id.as_deref(), child.label.as_deref()), (Some("sess-a:3"), Some("reviewer")));
    assert_eq!(parent.children, 1);
    assert_eq!(by_title(&idx, "Second").parent_id, None);
}

#[test]
fn rescan_is_incremental_and_follows_growth() {
    let e = env();
    let roots = roots(&e);
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    idx.scan(&roots).unwrap();
    assert_eq!(idx.scan(&roots).unwrap().indexed, 0, "nothing changed");

    let b = Connection::open(db_of(&e.dir, "sess-b")).unwrap();
    add(&b, 12, 1, json!({"model":[{"role":"assistant","content":[{"type":"text","text":"ok"}],"model":"m2","timestamp":ts(20)}],"kind":"pi.assistant","id":12,"conversationId":1}));
    assert_eq!(idx.scan(&roots).unwrap().indexed, 1, "only the conversation that grew");
    let s = by_title(&idx, "Second");
    assert_eq!((s.event_count, s.assistant_count), (2, 1));
    let lines: Vec<i64> = idx.list_events(&s.id, &EventFilter::default()).unwrap().iter().map(|e| e.line).collect();
    assert_eq!(lines, [9, 12], "no duplicates after append");

    std::fs::remove_dir_all(e.dir.join("sessions/sess-b")).unwrap();
    idx.scan(&roots).unwrap();
    assert_eq!(sessions(&idx).len(), 2, "a vanished session is pruned");
}

#[test]
fn missing_root_is_a_quiet_empty_root_and_unreadable_db_keeps_cache() {
    let e = env();
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    let gone = vec![Root { path: e.dir.join("nope"), ..roots(&e).remove(0) }];
    let report = idx.scan(&gone).unwrap();
    assert!(report.errors.is_empty());
    assert_eq!((report.roots[0].accessible, report.roots[0].files), (false, 0));

    let roots = roots(&e);
    idx.scan(&roots).unwrap();
    std::fs::write(db_of(&e.dir, "sess-b"), "not a database, just bytes ........................................").unwrap();
    let report = idx.scan(&roots).unwrap();
    assert!(report.errors.is_empty());
    assert_eq!(sessions(&idx).len(), 3, "cached rows survive a bad db");
}

#[test]
fn hook_indexes_the_named_conversation_and_marks_it_live() {
    let e = env();
    let roots = roots(&e);
    let mut idx = TraceIndex::open(&e.idx_db).unwrap();
    let file = db_of(&e.dir, "sess-a");
    let payload = json!({"event":"tool_execution_end","tool":"bash","cwd":"/w/dpi","session_id":"sess-a:3","session_file":file,"tool_use_id":"call_1","tool_input":{"command":"ls"}});
    ingest(&mut idx, &roots, extract("dpi", payload.to_string().as_bytes())).unwrap();
    let rows = sessions(&idx);
    assert_eq!(rows.len(), 1, "only the named conversation");
    assert_eq!(rows[0].model.as_deref(), Some("m3"));
    assert_eq!((rows[0].title.as_str(), rows[0].event_count), ("reviewer", 2));
    assert_eq!(idx.last_receipt(&rows[0].id).unwrap().unwrap().1, "tool_execution_end");
    assert_eq!(rows[0].status, Status::Active);

    // No conversation id, an unknown one, or a path-shaped session id: the receipt is recorded and nothing else happens.
    let before = idx.receipts_watermark().unwrap();
    for sid in ["sess-a", "sess-a:99", "../../etc:1", "nope:1", ""] {
        let p = json!({"event":"x","session_id":sid});
        ingest(&mut idx, &roots, extract("dpi", p.to_string().as_bytes())).unwrap();
    }
    assert_eq!(idx.receipts_watermark().unwrap(), before + 5);
    assert_eq!(sessions(&idx).len(), 1);
}
