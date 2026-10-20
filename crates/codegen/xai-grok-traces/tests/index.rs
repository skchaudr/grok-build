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
    let mut idx = TraceIndex::open(&tmp.path().join("idx/t.sqlite")).unwrap();
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

fn grow(path: &Path, to: u64) {
    fs::OpenOptions::new().write(true).open(path).unwrap().set_len(to).unwrap();
}

#[test]
fn preview_is_flat_and_clipped() {
    let e = env();
    let long = format!("line one\n\n   line   two {}", "x".repeat(400));
    write(&e.home.join(".pi/agent/sessions/--w-pi--/long.jsonl"), &format!(
        "{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":{}}}}}\n", serde_json::to_string(&long).unwrap()
    ));
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let rows = idx.list_sessions(&SessionFilter { text: Some("line one".into()), ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "line one", "title is the first line of the full text");
    let ev = idx.list_events(&rows[0].id, &EventFilter::default()).unwrap();
    assert!(ev[0].preview.starts_with("line one line two xxx"), "{}", ev[0].preview);
    assert_eq!(ev[0].preview.chars().count(), 161);
    assert!(!ev[0].preview.contains('\n'));
}

#[test]
fn over_cap_refuses_writes_and_reports_health() {
    let e = env();
    drop(TraceIndex::open(&e.db).unwrap());
    let small = fs::metadata(&e.db).unwrap().len();
    grow(&e.db, xai_grok_traces::MAX_DB_BYTES + 1024 * 1024);
    let mut idx = TraceIndex::open(&e.db).unwrap();
    let err = idx.scan(&e.roots).unwrap_err();
    assert!(matches!(err, xai_grok_traces::Error::OverCap { .. }), "{err}");
    let health = idx.health().unwrap();
    assert!(health.starts_with("over cap: 257MB / 256MB — writes paused; run grok-traces doctor"), "{health}");
    let pi = e.roots.iter().find(|r| r.source == "pi").unwrap();
    assert!(idx.index_file(pi, &e.home.join(".pi/agent/sessions/--w-pi--/p.jsonl")).is_err());
    let r = xai_grok_traces::Receipt { source: "claude".into(), event: "Stop".into(), ..Default::default() };
    assert!(matches!(idx.record_receipt(&r), Err(xai_grok_traces::Error::OverCap { .. })));
    assert_eq!(all(&idx).len(), 0, "nothing was written");

    drop(idx);
    grow(&e.db, small);
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    assert_eq!(idx.health(), None, "cleared once under the cap");
    assert_eq!(all(&idx).len(), 6);
}

#[test]
fn old_schema_is_replaced_not_migrated() {
    let e = env();
    fs::create_dir_all(e.db.parent().unwrap()).unwrap();
    {
        let c = rusqlite::Connection::open(&e.db).unwrap();
        c.execute_batch("CREATE TABLE trace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE trace_events (summary TEXT);
                         INSERT INTO trace_events VALUES ('stale');").unwrap();
    }
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    assert_eq!(all(&idx).len(), 6);
    let stats = idx.stats().unwrap();
    assert_eq!(stats.schema_version.as_deref(), Some("2"));
    assert_eq!(stats.parser_version.as_deref(), Some("2"));
    let names: Vec<String> = fs::read_dir(e.db.parent().unwrap()).unwrap().map(|d| d.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|n| !n.contains(".old.")), "old db removed: {names:?}");
    assert!(!names.iter().any(|n| n.contains(".corrupt.")));
}

#[test]
fn root_for_guards_paths() {
    let e = env();
    let ok = e.home.join(".claude/projects/-w-claude/c1.jsonl");
    assert_eq!(xai_grok_traces::root_for(&ok, &e.roots).map(|r| r.source.as_str()), Some("claude"));
    // Wrong name for a named root, outside every root, and index-ish files.
    let grok_other = e.home.join(".grok/sessions/%2Fw%2Fgrok/g1/updates.jsonl");
    assert!(xai_grok_traces::root_for(&grok_other, &e.roots).is_none());
    let outside = e.home.join("elsewhere.jsonl");
    write(&outside, "{}\n");
    assert!(xai_grok_traces::root_for(&outside, &e.roots).is_none());
    for name in ["x.db", "x.sqlite", "x.sqlite-wal", "x.sqlite-shm", "x.sqlite.corrupt.123", "x.jsonl.corrupt.9"] {
        let p = e.home.join(".claude/projects/-w-claude").join(name);
        write(&p, "{}\n");
        assert!(xai_grok_traces::root_for(&p, &e.roots).is_none(), "{name}");
    }
    assert!(xai_grok_traces::root_for(&e.home.join(".claude/projects/missing.jsonl"), &e.roots).is_none());
    // `..` cannot escape a root.
    let sneaky = e.home.join(".claude/projects/-w-claude/../../../elsewhere.jsonl");
    assert!(xai_grok_traces::root_for(&sneaky, &e.roots).is_none());
}

#[test]
fn index_file_rejects_non_sources_and_own_dir() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    let claude = e.roots.iter().find(|r| r.source == "claude").unwrap();
    let outside = e.home.join("elsewhere.jsonl");
    write(&outside, "{}\n");
    assert!(matches!(idx.index_file(claude, &outside), Err(xai_grok_traces::Error::Rejected(_))));
    // A root that happens to contain the index directory still cannot index it.
    let own = Root { id: "own".into(), source: "claude".into(), path: e.db.parent().unwrap().to_path_buf(), format: Format::Jsonl, file_name: None };
    let planted = e.db.parent().unwrap().join("x.jsonl");
    write(&planted, "{}\n");
    assert!(matches!(idx.index_file(&own, &planted), Err(xai_grok_traces::Error::Rejected(_))));
    assert_eq!(all(&idx).len(), 0);
}

#[test]
fn event_cap_stops_a_runaway_file() {
    let e = env();
    let line = "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n";
    write(&e.home.join(".pi/agent/sessions/--w-pi--/big.jsonl"), &line.repeat(xai_grok_traces::MAX_EVENTS_PER_FILE + 5));
    let mut idx = TraceIndex::open(&e.db).unwrap();
    let pi = e.roots.iter().find(|r| r.source == "pi").unwrap();
    let path = e.home.join(".pi/agent/sessions/--w-pi--/big.jsonl");
    assert!(idx.index_file(pi, &path).unwrap());
    let row = idx.list_sessions(&SessionFilter::default()).unwrap().remove(0);
    assert_eq!(row.event_count, xai_grok_traces::MAX_EVENTS_PER_FILE as i64);
    assert!(!idx.index_file(pi, &path).unwrap(), "capped file is not rescanned");
}

#[test]
fn receipts_watermark_and_active_status() {
    use xai_grok_traces::hook::{extract, ingest};
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    assert_eq!(idx.receipts_watermark().unwrap(), 0);
    // The codex fixture ends in task_complete → completed, until a live receipt says otherwise.
    let x = by_source(&idx, "codex");
    assert_eq!(x.status, Status::Completed);
    let path = e.home.join(".codex/sessions/2026/10/01/rollout-x.jsonl");
    let payload = |event: &str| format!(r#"{{"hook_event_name":"{event}","session_id":"n1","transcript_path":"{}","tool_name":"Bash","cwd":"/w"}}"#, path.display());

    let id = ingest(&mut idx, &e.roots, extract("codex", payload("PostToolUse").as_bytes())).unwrap();
    assert_eq!((id, idx.receipts_watermark().unwrap()), (1, 1));
    let x = by_source(&idx, "codex");
    assert_eq!(x.status, Status::Active, "fresh non-terminal receipt wins");
    let (at, event) = idx.last_receipt(&x.id).unwrap().unwrap();
    assert_eq!(event, "PostToolUse");
    assert!(at > 0);
    let active = |idx: &TraceIndex| idx.list_sessions(&SessionFilter { active_only: true, ..Default::default() }).unwrap();
    assert!(active(&idx).iter().any(|s| s.source == "codex"));

    ingest(&mut idx, &e.roots, extract("codex", payload("Stop").as_bytes())).unwrap();
    assert_eq!(by_source(&idx, "codex").status, Status::Completed, "terminal receipt falls back to file rules");
    assert!(active(&idx).iter().all(|s| s.source != "codex"));

    // A receipt for a path outside every root is still recorded, without a session.
    ingest(&mut idx, &e.roots, extract("codex", br#"{"hook_event_name":"Stop","transcript_path":"/nope.jsonl"}"#)).unwrap();
    assert_eq!(idx.receipts_watermark().unwrap(), 3);
    assert_eq!(idx.stats().unwrap().receipts.len(), 1);
}

#[test]
fn receipts_are_pruned_to_a_bound() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    let r = xai_grok_traces::Receipt { source: "pi".into(), event: "x".into(), ..Default::default() };
    for _ in 0..101_000 {
        idx.record_receipt(&r).unwrap();
    }
    let n: i64 = idx.stats().unwrap().receipts.iter().map(|r| r.1).sum();
    assert_eq!(n, 100_000);
    assert_eq!(idx.receipts_watermark().unwrap(), 101_000);
}

#[test]
fn hook_payload_fields_across_harnesses() {
    use xai_grok_traces::hook::extract;
    let c = extract("claude", br#"{"hook_event_name":"PreToolUse","session_id":"s1","transcript_path":"/t.jsonl","tool_name":"Bash","cwd":"/w"}"#);
    assert_eq!((c.event.as_str(), c.native_session.as_deref(), c.path.as_deref()), ("PreToolUse", Some("s1"), Some("/t.jsonl")));
    assert_eq!((c.tool.as_deref(), c.cwd.as_deref(), c.source.as_str()), (Some("Bash"), Some("/w"), "claude"));
    let k = extract("cursor", br#"{"hookEventName":"beforeShellExecution","conversation_id":"c9","workspace_roots":["/a","/b"]}"#);
    assert_eq!((k.event.as_str(), k.native_session.as_deref(), k.cwd.as_deref()), ("beforeShellExecution", Some("c9"), Some("/a")));
    let p = extract("pi", br#"{"type":"agent_end","sessionId":"p1","sessionFile":"/p.jsonl","toolName":"read"}"#);
    assert_eq!((p.event.as_str(), p.native_session.as_deref(), p.path.as_deref(), p.tool.as_deref()), ("agent_end", Some("p1"), Some("/p.jsonl"), Some("read")));
    let t = extract("codex", br#"{"event":"x","thread_id":"t1","transcriptPath":"/c.jsonl","tool":"shell"}"#);
    assert_eq!((t.event.as_str(), t.native_session.as_deref(), t.path.as_deref(), t.tool.as_deref()), ("x", Some("t1"), Some("/c.jsonl"), Some("shell")));
    assert_eq!(extract("x", b"not json").event, "unparsed");
    assert_eq!(extract("x", b"{}").event, "unknown");
    assert_eq!(extract("x", br#"{"event":"e","cwd":""}"#).cwd, None);
}

#[test]
fn grok_hook_updates_path_maps_to_chat_history() {
    use xai_grok_traces::hook::{extract, ingest, transcript_for};
    let e = env();
    let dir = e.home.join(".grok/sessions/%2Fw%2Fgrok/g1");
    assert_eq!(transcript_for("grok", &dir.join("updates.jsonl")), dir.join("chat_history.jsonl"));
    assert_eq!(transcript_for("claude", &dir.join("updates.jsonl")), dir.join("updates.jsonl"));
    let mut idx = TraceIndex::open(&e.db).unwrap();
    let payload = format!(r#"{{"hook_event_name":"PostToolUse","transcript_path":"{}"}}"#, dir.join("updates.jsonl").display());
    ingest(&mut idx, &e.roots, extract("grok", payload.as_bytes())).unwrap();
    let g = by_source(&idx, "grok");
    assert_eq!(g.event_count, 5);
    assert_eq!(idx.last_receipt(&g.id).unwrap().unwrap().1, "PostToolUse");
}
