//! Synthetic fixtures only; never real session content.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use xai_grok_traces::{EventFilter, Format, Root, SessionFilter, Status, TraceIndex, default_roots};

const CLAUDE: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","sessionId":"s","message":{"role":"user","content":"claude task"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":100,"output_tokens":20}}}
{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","content":"a b"}]}}
"#;
const CODEX: &str = r#"{"timestamp":"2026-10-01T00:00:00Z","type":"session_meta","payload":{"cwd":"/w/codex","model":"gpt-5"}}
{"timestamp":"2026-10-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex task"}]}}
{"timestamp":"2026-10-01T00:00:02Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}"}}
{"timestamp":"2026-10-01T00:00:03Z","type":"response_item","payload":{"type":"function_call_output","output":"error: no such file"}}
{"timestamp":"2026-10-01T00:00:04Z","type":"event_msg","payload":{"type":"task_complete"}}
"#;
const PI: &str = r#"{"type":"session","version":3,"id":"p","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/pi"}
{"type":"message","timestamp":"2026-10-01T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"pi task"}]}}
{"type":"message","timestamp":"2026-10-01T00:00:02Z","message":{"role":"assistant","model":"pi-model","content":[{"type":"text","text":"ok"}]}}
"#;
const OMP: &str = r#"{"type":"session","version":3,"id":"o","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/omp"}
{"type":"message","timestamp":"2026-10-01T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"omp task"}]}}
"#;
const DSH: &str = r#"{"type":"session","version":2,"id":"d","cwd":"/w/dsh","createdAt":1759276800000}
{"type":"user/message","seq":1,"time":1759276800000,"data":{"role":"user","content":"dsh task","id":"u1","source":"user"}}
{"type":"tool/call","seq":2,"time":1759276801000,"data":{"name":"read_file","arguments":{},"callId":"c1","step":1,"turn":1}}
{"type":"tool/result","seq":3,"time":1759276802000,"data":{"message":"done","step":1,"turn":1}}
{"type":"turn/end","seq":4,"time":1759276803000,"data":{"turn":1,"reason":"stop"}}
"#;
const GROK: &str = r#"{"type":"system","content":"sys"}
{"type":"user","content":[{"type":"text","text":"<user_info>injected</user_info>"}],"synthetic_reason":"x"}
{"type":"user","content":[{"type":"text","text":"grok task"}],"prompt_index":0}
{"type":"assistant","content":"x","model_id":"grok-5","tool_calls":[{"id":"t1","name":"read_file"}]}
{"type":"tool_result","tool_call_id":"t1","content":"ok"}
"#;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn zstd_write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, zstd::encode_all(text.as_bytes(), 3).unwrap()).unwrap();
}

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    roots: Vec<Root>,
    db: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let grok = home.join(".grok");
    write(&home.join(".claude/projects/-w-claude/c1.jsonl"), CLAUDE);
    write(&home.join(".codex/sessions/2026/10/01/rollout-x.jsonl"), CODEX);
    write(&home.join(".pi/agent/sessions/--w-pi--/p.jsonl"), PI);
    write(&home.join(".omp/agent/sessions/-w-omp/o.jsonl"), OMP);
    zstd_write(&home.join(".dsh/sessions/w-dsh/u/session.v2.jsonl.zstd"), DSH);
    write(&grok.join("sessions/%2Fw%2Fgrok/g1/chat_history.jsonl"), GROK);
    write(&grok.join("sessions/%2Fw%2Fgrok/g1/updates.jsonl"), "{\"type\":\"user\"}\n");
    write(&grok.join("sessions/%2Fw%2Fgrok/g1/summary.json"), r#"{"info":{"id":"g1","cwd":"/w/grok"}}"#);
    let roots = default_roots(&home, &grok);
    Env { db: tmp.path().join("idx/traces.sqlite"), _tmp: tmp, home, roots }
}

fn all(idx: &TraceIndex) -> Vec<xai_grok_traces::SessionRow> {
    idx.list_sessions(&SessionFilter::default()).unwrap()
}

fn by_source(idx: &TraceIndex, s: &str) -> xai_grok_traces::SessionRow {
    let rows = idx.list_sessions(&SessionFilter { source: Some(s.into()), ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1, "{s}: {rows:?}");
    rows.into_iter().next().unwrap()
}

#[test]
fn each_source_discovers_its_fixture() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    let report = idx.scan(&e.roots).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(all(&idx).len(), 6);

    let c = by_source(&idx, "claude");
    assert_eq!(c.title, "claude task");
    assert_eq!((c.event_count, c.tool_count, c.user_count), (3, 2, 1));
    assert_eq!(c.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(c.cwd.as_deref(), Some("/w/claude"));
    assert_eq!((c.input_tokens, c.output_tokens), (Some(100), Some(20)));
    assert_eq!(c.duration_ms, Some(6000));

    let x = by_source(&idx, "codex");
    assert_eq!(x.title, "codex task");
    assert_eq!((x.tool_count, x.error_count), (2, 1));
    assert_eq!(x.status, Status::Completed);

    assert_eq!(by_source(&idx, "pi").model.as_deref(), Some("pi-model"));
    assert_eq!(by_source(&idx, "omp").title, "omp task");

    let d = by_source(&idx, "dsh");
    assert_eq!((d.title.as_str(), d.tool_count, d.status), ("dsh task", 2, Status::Completed));
    assert_eq!(d.cwd.as_deref(), Some("/w/dsh"));

    let g = by_source(&idx, "grok");
    assert_eq!((g.title.as_str(), g.event_count), ("grok task", 5));
    assert_eq!(g.cwd.as_deref(), Some("/w/grok"), "cwd from summary.json");
    assert_eq!(g.model.as_deref(), Some("grok-5"));
    // Fresh, non-terminal file → active.
    assert_eq!(g.status, Status::Active);
}

#[test]
fn filters_and_search() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let f = |sf: SessionFilter| idx.list_sessions(&sf).unwrap().len();
    assert_eq!(f(SessionFilter { errors_only: true, ..Default::default() }), 1);
    assert_eq!(f(SessionFilter { text: Some("omp task".into()), ..Default::default() }), 1);
    // Event-level match: "no such file" only appears in a codex tool output.
    assert_eq!(f(SessionFilter { text: Some("no such file".into()), ..Default::default() }), 0, "shallow");
    assert_eq!(f(SessionFilter { text: Some("no such file".into()), deep: true, ..Default::default() }), 1);
    assert_eq!(f(SessionFilter { text: Some("100%".into()), ..Default::default() }), 0);
}

#[test]
fn events_facets_and_raw_line() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let x = by_source(&idx, "codex");
    let events = idx.list_events(&x.id, &EventFilter::default()).unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[3].depth, 2);
    let errs = idx.list_events(&x.id, &EventFilter { errors_only: true, ..Default::default() }).unwrap();
    assert_eq!(errs.iter().map(|e| e.line).collect::<Vec<_>>(), vec![3]);
    let shell = idx.list_events(&x.id, &EventFilter { tool: Some("shell".into()), ..Default::default() }).unwrap();
    assert_eq!(shell.len(), 1);
    let facets = idx.facets(&x.id).unwrap();
    assert_eq!((facets.total, facets.errors), (5, 1));

    let raw = idx.raw_line(&x.id, 2).unwrap().unwrap();
    assert_eq!(raw, CODEX.lines().nth(2).unwrap(), "byte for byte");

    let d = by_source(&idx, "dsh");
    assert_eq!(idx.raw_line(&d.id, 2).unwrap().unwrap(), DSH.lines().nth(2).unwrap());
}

#[test]
fn append_reindexes_only_new_lines_and_rewrite_reparses() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let pi_root = e.roots.iter().find(|r| r.source == "pi").unwrap();
    let path = e.home.join(".pi/agent/sessions/--w-pi--/p.jsonl");

    assert!(!idx.index_file(pi_root, &path).unwrap(), "unchanged file is skipped");

    // Unterminated tail is not indexed until its newline arrives.
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(br#"{"type":"message","message":{"role":"assistant","content":"late"}}"#).unwrap();
    idx.index_file(pi_root, &path).unwrap();
    assert_eq!(by_source(&idx, "pi").event_count, 3);
    f.write_all(b"\n").unwrap();
    drop(f);
    idx.index_file(pi_root, &path).unwrap();
    let p = by_source(&idx, "pi");
    assert_eq!(p.event_count, 4);
    let events = idx.list_events(&p.id, &EventFilter::default()).unwrap();
    let lines: Vec<i64> = events.iter().map(|e| e.line).collect();
    assert_eq!(lines, vec![0, 1, 2, 3], "no duplicates");
    assert_eq!(p.title, "pi task", "title kept across append");

    // Rewriting the prefix forces a full re-parse.
    write(&path, "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"rewritten\"}}\n");
    idx.index_file(pi_root, &path).unwrap();
    let p = by_source(&idx, "pi");
    assert_eq!((p.event_count, p.title.as_str()), (1, "rewritten"));
}

#[test]
fn zstd_matches_plain() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = Root { id: "p".into(), source: "dsh".into(), path: tmp.path().join("p"), format: Format::Jsonl, file_name: None };
    let packed = Root { id: "z".into(), source: "dsh".into(), path: tmp.path().join("z"), format: Format::Zstd, file_name: None };
    write(&plain.path.join("s.jsonl"), DSH);
    zstd_write(&packed.path.join("s.jsonl.zstd"), DSH);
    let mut idx = TraceIndex::open(&tmp.path().join("t.sqlite")).unwrap();
    idx.scan(&[plain, packed]).unwrap();
    let rows = all(&idx);
    assert_eq!(rows.len(), 2);
    let ev = |r: &xai_grok_traces::SessionRow| idx.list_events(&r.id, &EventFilter::default()).unwrap();
    assert_eq!(ev(&rows[0]), ev(&rows[1]));
    assert_eq!((rows[0].title.clone(), rows[0].event_count), (rows[1].title.clone(), rows[1].event_count));
}

#[test]
fn corrupt_db_is_quarantined_and_rebuilt() {
    let e = env();
    write(&e.db, "this is not a sqlite database, just garbage bytes ............................................");
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    assert_eq!(all(&idx).len(), 6);
    let quarantined = fs::read_dir(e.db.parent().unwrap())
        .unwrap()
        .filter_map(|d| d.ok())
        .any(|d| d.file_name().to_string_lossy().contains(".corrupt."));
    assert!(quarantined);
}

#[test]
fn file_cap_reports_limit_and_keeps_rows() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    write(&e.home.join(".pi/agent/sessions/--w-pi--/q.jsonl"), PI);
    let report = idx.scan_with_limit(&e.roots, 1).unwrap();
    let pi = report.roots.iter().find(|r| r.id == "pi-sessions").unwrap();
    assert!(pi.limited);
    assert_eq!(idx.list_sessions(&SessionFilter { source: Some("pi".into()), ..Default::default() }).unwrap().len(), 1);
}

#[test]
fn vanished_files_and_roots_are_pruned() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    fs::remove_file(e.home.join(".omp/agent/sessions/-w-omp/o.jsonl")).unwrap();
    idx.scan(&e.roots).unwrap();
    assert_eq!(all(&idx).len(), 5);
    let fewer: Vec<Root> = e.roots.iter().filter(|r| r.source != "pi").cloned().collect();
    idx.scan(&fewer).unwrap();
    assert_eq!(all(&idx).len(), 4);
}
