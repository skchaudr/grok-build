//! Synthetic fixtures only; never real session content.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use xai_grok_traces::{EventFilter, Format, Limits, Root, SessionFilter, Status, TraceIndex, default_roots};

const CLAUDE: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","sessionId":"s","message":{"role":"user","content":"claude task"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":100,"output_tokens":20}}}
{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","content":"a b"}]}}
"#;
const CODEX: &str = r#"{"timestamp":"2026-10-01T00:00:00Z","type":"session_meta","payload":{"cwd":"/w/codex","model":"gpt-5"}}
{"timestamp":"2026-10-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex task"}]}}
{"timestamp":"2026-10-01T00:00:02Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}"}}
{"timestamp":"2026-10-01T00:00:03Z","type":"response_item","payload":{"type":"function_call_output","output":"Chunk ID: a\nProcess exited with code 1\nerror: no such file"}}
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
    assert_ne!(x.status, Status::Completed, "task_complete ends a turn, not the session");

    assert_eq!(by_source(&idx, "pi").model.as_deref(), Some("pi-model"));
    assert_eq!(by_source(&idx, "omp").title, "omp task");

    let d = by_source(&idx, "dsh");
    assert_eq!((d.title.as_str(), d.tool_count, d.status), ("dsh task", 2, d.status));
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
    let all_events = EventFilter { include_noise: true, ..Default::default() };
    let events = idx.list_events(&x.id, &all_events).unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[3].depth, 2);
    assert_eq!(idx.list_events(&x.id, &EventFilter::default()).unwrap().len(), 4, "session_meta is noise");
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
    let events = idx.list_events(&p.id, &EventFilter { include_noise: true, ..Default::default() }).unwrap();
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
    let ev = |r: &xai_grok_traces::SessionRow| idx.list_events(&r.id, &EventFilter { include_noise: true, ..Default::default() }).unwrap();
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
    assert_eq!(pi.files, 1, "the cap discovers one file");
    // A limited scan does not prune. The previously indexed file stays when the one slot is the new file,
    // so the row count is 1 or 2 depending on directory order.
    let n = idx.list_sessions(&SessionFilter { source: Some("pi".into()), ..Default::default() }).unwrap().len();
    assert!((1..=2).contains(&n), "cached rows kept under the cap, got {n}");
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
    assert_eq!(ev[0].preview.chars().count(), xai_grok_traces::normalize::PREVIEW_CHARS + 1);
    assert!(!ev[0].preview.contains('\n'));
}

/// `n` pi sessions of `lines` events each, file i `age(i)` seconds old.
fn pi_sessions(e: &Env, from: usize, n: usize, lines: usize, age: impl Fn(usize) -> u64) {
    for i in from..from + n {
        let p = e.home.join(format!(".pi/agent/sessions/--w-pi--/bulk{i:03}.jsonl"));
        let line = format!("{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"session {i} some text to index\"}}}}\n");
        write(&p, &line.repeat(lines));
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age(i));
        fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
    }
}

fn pi_only(e: &Env) -> Vec<Root> {
    let _ = fs::remove_file(e.home.join(".pi/agent/sessions/--w-pi--/p.jsonl"));
    e.roots.iter().filter(|r| r.source == "pi").cloned().collect()
}

fn small(budget: u64) -> Limits {
    Limits { budget, ceiling: budget * 2, ..Default::default() }
}

fn titles(idx: &TraceIndex) -> Vec<String> {
    all(idx).into_iter().map(|s| s.title).collect()
}

#[test]
fn past_budget_evicts_oldest_sessions_and_shrinks_the_file() {
    let e = env();
    let roots = pi_only(&e);
    // File i is (i+1)*100s old: bulk000 is the newest.
    pi_sessions(&e, 0, 40, 250, |i| (i as u64 + 1) * 100);
    let mut big = TraceIndex::open(&e.db).unwrap();
    big.scan(&roots).unwrap();
    assert_eq!(all(&big).len(), 40);
    drop(big);
    let full = fs::metadata(&e.db).unwrap().len();
    fs::remove_file(&e.db).unwrap();
    let _ = fs::remove_file(format!("{}-wal", e.db.display()));
    let _ = fs::remove_file(format!("{}-shm", e.db.display()));

    let budget = full / 2;
    let mut idx = TraceIndex::open_with_limits(&e.db, None, small(budget)).unwrap();
    let report = idx.scan(&roots).unwrap();
    assert_eq!(report.tripped, None);
    assert_eq!(idx.health(), None, "eviction is not an alert");
    assert!(idx.db_bytes() <= budget * 85 / 100 + 64 * 1024, "{} vs {budget}", idx.db_bytes());
    assert!(fs::metadata(&e.db).unwrap().len() < full, "file shrank");
    let kept = titles(&idx);
    assert!(kept.len() < 40 && kept.len() > 5, "{}", kept.len());
    assert!(kept.contains(&"session 0 some text to index".to_string()), "newest kept");
    assert!(!kept.contains(&"session 39 some text to index".to_string()), "oldest evicted");
    // The kept ones are exactly the newest k.
    for i in 0..kept.len() {
        assert!(kept.contains(&format!("session {i} some text to index")), "{i} in {kept:?}");
    }
    let events: i64 = idx.stats().unwrap().events;
    assert_eq!(events, kept.len() as i64 * 250, "evicted sessions took their events");

    // A rescan does not bring the evicted files back.
    let again = idx.scan(&roots).unwrap();
    assert_eq!(again.indexed, 0);
    assert_eq!(titles(&idx).len(), kept.len());
    assert_eq!(idx.health(), None);

    // An evicted file that is written again returns.
    let p = e.home.join(".pi/agent/sessions/--w-pi--/bulk039.jsonl");
    let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(b"{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"back\"}}\n").unwrap();
    drop(f);
    assert_eq!(idx.scan(&roots).unwrap().indexed, 1);
    assert_eq!(idx.health(), None);
}

#[test]
fn normal_growth_past_budget_never_sets_health() {
    let e = env();
    let roots = pi_only(&e);
    let mut idx = TraceIndex::open_with_limits(&e.db, None, small(300 * 1024)).unwrap();
    for round in 0..4 {
        pi_sessions(&e, round * 10, 10, 100, |i| 10_000 - i as u64 * 10);
        let r = idx.scan(&roots).unwrap();
        assert_eq!(r.tripped, None);
        assert_eq!(idx.health(), None, "round {round}");
        let rc = xai_grok_traces::Receipt { source: "pi".into(), event: "Stop".into(), ..Default::default() };
        for _ in 0..250 {
            idx.record_receipt(&rc).unwrap();
        }
        assert_eq!(idx.health(), None);
    }
}

fn expect_tripped(idx: &mut TraceIndex, roots: &[Root], e: &Env, needle: &str) {
    let health = idx.health().unwrap_or_default();
    assert!(health.starts_with("tripped: ") && health.contains(needle), "{health}");
    assert!(health.contains("doctor --reset"), "{health}");
    assert!(matches!(idx.scan(roots), Err(xai_grok_traces::Error::Tripped(_))));
    let pi = roots.iter().find(|r| r.source == "pi").unwrap();
    assert!(idx.index_file(pi, &e.home.join(".pi/agent/sessions/--w-pi--/bulk000.jsonl")).is_err());
    let r = xai_grok_traces::Receipt { source: "claude".into(), event: "Stop".into(), ..Default::default() };
    assert!(matches!(idx.record_receipt(&r), Err(xai_grok_traces::Error::Tripped(_))));
}

fn reset_and_resume(idx: &mut TraceIndex, roots: &[Root]) {
    assert!(idx.reset_trip().unwrap().is_some());
    assert_eq!(idx.health(), None);
    idx.scan(roots).unwrap();
}

#[test]
fn hard_ceiling_trips_and_reset_clears() {
    let e = env();
    let roots = pi_only(&e);
    pi_sessions(&e, 0, 2, 10, |i| i as u64);
    drop(TraceIndex::open(&e.db).unwrap());
    grow(&e.db, 3 * 1024 * 1024);
    let limits = Limits { budget: 1024 * 1024, ceiling: 2 * 1024 * 1024, ..Default::default() };
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits).unwrap();
    assert!(idx.health().unwrap().contains("hard ceiling — db 3MB > 2MB (2x the 1MB budget"));
    assert!(matches!(idx.scan(&roots), Err(xai_grok_traces::Error::Tripped(_))));
    expect_tripped(&mut idx, &roots, &e, "hard ceiling");
    assert_eq!(all(&idx).len(), 0, "nothing was written");
    drop(idx);
    // Looked at it; the file is fine under roomier limits.
    let mut idx = TraceIndex::open_with_limits(&e.db, None, small(64 * 1024 * 1024)).unwrap();
    assert!(idx.health().is_some(), "trip persists across opens");
    reset_and_resume(&mut idx, &roots);
    assert_eq!(all(&idx).len(), 2);
}

#[test]
fn growth_rate_trips_and_reset_clears() {
    let e = env();
    let roots = pi_only(&e);
    let limits = Limits { growth_bytes: 200 * 1024, ..Default::default() };
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits).unwrap();
    pi_sessions(&e, 0, 1, 10, |_| 5);
    assert_eq!(idx.scan(&roots).unwrap().tripped, None, "first scan is the baseline");
    pi_sessions(&e, 1, 40, 250, |i| 100 + i as u64);
    let report = idx.scan(&roots).unwrap();
    let msg = report.tripped.unwrap();
    assert!(msg.starts_with("tripped: growth rate — db grew ") && msg.contains("limit 0MB per 60 min"), "{msg}");
    expect_tripped(&mut idx, &roots, &e, "growth rate");
    reset_and_resume(&mut idx, &roots);
}

#[test]
fn evicted_growth_still_counts() {
    // A loop that is evicted as fast as it writes never makes the file bigger, but is still caught.
    let e = env();
    let roots = pi_only(&e);
    let limits = Limits { budget: 300 * 1024, ceiling: 600 * 1024, growth_bytes: 400 * 1024, ..Default::default() };
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits).unwrap();
    pi_sessions(&e, 0, 1, 10, |_| 5);
    idx.scan(&roots).unwrap();
    let mut tripped = None;
    for round in 0..6 {
        pi_sessions(&e, 1 + round * 20, 20, 150, |i| 100_000 - i as u64);
        tripped = tripped.or(match idx.scan(&roots) {
            Ok(r) => r.tripped,
            Err(e) => Some(e.to_string()),
        });
    }
    assert!(tripped.clone().unwrap_or_default().contains("growth rate"), "{tripped:?}");
}

#[test]
fn event_flood_trips_and_reset_clears() {
    let e = env();
    let roots = pi_only(&e);
    let limits = Limits { max_events: 20, max_capped_files: 5, ..Default::default() };
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits).unwrap();
    pi_sessions(&e, 0, 5, 50, |i| i as u64);
    let report = idx.scan(&roots).unwrap();
    assert_eq!(report.tripped, None, "five capped files is still normal");
    assert!(report.errors.is_empty());
    let capped: Vec<_> = all(&idx).iter().map(|s| s.event_count).collect();
    assert_eq!(capped, [20; 5], "capped files are still indexed to the cap");
    pi_sessions(&e, 5, 6, 50, |i| 100 + i as u64);
    let report = idx.scan(&roots).unwrap();
    let msg = report.tripped.unwrap();
    assert!(msg.contains("event flood — 6 files hit the 20-event cap in one scan (limit 5)"), "{msg}");
    expect_tripped(&mut idx, &roots, &e, "event flood");
    reset_and_resume(&mut idx, &roots);
}

#[test]
fn db_is_created_with_incremental_auto_vacuum() {
    let e = env();
    drop(TraceIndex::open(&e.db).unwrap());
    let c = rusqlite::Connection::open(&e.db).unwrap();
    let mode: i64 = c.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, 2);
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
    assert_eq!(stats.schema_version.as_deref(), Some("7"));
    assert_eq!(stats.parser_version.as_deref(), Some("7"));
    let names: Vec<String> = fs::read_dir(e.db.parent().unwrap()).unwrap().map(|d| d.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|n| !n.contains(".old.")), "old db removed: {names:?}");
    assert!(!names.iter().any(|n| n.contains(".corrupt.")));
}

#[test]
fn schema_6_db_is_rebuilt_on_open() {
    let e = env();
    fs::create_dir_all(e.db.parent().unwrap()).unwrap();
    {
        let c = rusqlite::Connection::open(&e.db).unwrap();
        c.execute_batch("CREATE TABLE trace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                         INSERT INTO trace_meta VALUES ('schema_version', '6'), ('parser_version', '5');
                         CREATE TABLE trace_receipts (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, source TEXT NOT NULL,
                           event TEXT NOT NULL, native_session TEXT, path TEXT, session_id TEXT, tool TEXT, cwd TEXT);
                         CREATE INDEX trace_receipts_session ON trace_receipts(session_id, id);").unwrap();
    }
    drop(TraceIndex::open(&e.db).unwrap());
    let c = rusqlite::Connection::open(&e.db).unwrap();
    let cols: Vec<String> = c.prepare("SELECT name FROM pragma_table_info('trace_receipts')").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    for want in ["detail", "tool_use_id", "agent_id", "harness_pid", "harness_start"] {
        assert!(cols.iter().any(|c| c == want), "{want} in {cols:?}");
    }
    let idx_cols: i64 = c.query_row("SELECT count(*) FROM pragma_index_info('trace_receipts_session')", [], |r| r.get(0)).unwrap();
    assert_eq!(idx_cols, 3, "old index replaced");
    assert!(!e.db.with_extension("sqlite.rebuild.lock").exists());
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
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    fs::File::options().write(true).open(e.home.join(".codex/sessions/2026/10/01/rollout-x.jsonl")).unwrap().set_modified(old).unwrap();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    assert_eq!(idx.receipts_watermark().unwrap(), 0);
    // The codex fixture ends in task_complete (a turn end, not terminal) and is old → idle, until a live receipt says otherwise.
    let x = by_source(&idx, "codex");
    assert_eq!(x.status, Status::Idle);
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
    assert_eq!(by_source(&idx, "codex").status, Status::Idle, "terminal receipt falls back to file rules");
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
fn hook_extract_live_fields() {
    use xai_grok_traces::hook::extract;
    let c = extract("claude", br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls -la\nsecond"},"tool_use_id":"tu1","agent_id":"a1"}"#);
    assert_eq!((c.detail.as_deref(), c.tool_use_id.as_deref(), c.agent_id.as_deref()), (Some("ls -la"), Some("tu1"), Some("a1")));
    let n = extract("claude", br#"{"hook_event_name":"Notification","notification_type":"idle_prompt"}"#);
    assert_eq!(n.detail.as_deref(), Some("idle_prompt"));
    let s = extract("claude", br#"{"hook_event_name":"SessionStart","source":"compact"}"#);
    assert_eq!(s.detail.as_deref(), Some("compact"));
    let pi = extract("pi", br#"{"event":"session_start","source":"x","harness_pid":4242}"#);
    assert_eq!((pi.harness_pid, pi.detail), (Some(4242), None));
    assert_eq!(extract("pi", br#"{"event":"e","harness_pid":0}"#).harness_pid, None);
    assert_eq!(extract("pi", br#"{"event":"e","harness_pid":-3}"#).harness_pid, None);

    // Cursor: conversation_id wins, before* events name their own tool.
    let sh = extract("cursor", br#"{"hook_event_name":"beforeShellExecution","conversation_id":"c1","session_id":"other","command":"git status\nmore","tool_use_id":"u1"}"#);
    assert_eq!((sh.native_session.as_deref(), sh.tool.as_deref(), sh.detail.as_deref()), (Some("c1"), Some("Shell"), Some("git status")));
    let after = extract("cursor", br#"{"hook_event_name":"afterShellExecution","conversation_id":"c1","command":"git status","output":"x"}"#);
    assert_eq!((after.tool.as_deref(), after.detail.as_deref()), (Some("Shell"), Some("git status")));
    let mcp = extract("cursor", br#"{"hook_event_name":"beforeMCPExecution","tool_name":"search","server":"docs"}"#);
    assert_eq!((mcp.tool.as_deref(), mcp.detail.as_deref()), (Some("MCP"), Some("docs.search")));
    let read = extract("cursor", br#"{"hook_event_name":"beforeReadFile","file_path":"/a/b/c.rs"}"#);
    assert_eq!((read.tool.as_deref(), read.detail.as_deref()), (Some("Read"), Some("b/c.rs")));
    let pre = extract("cursor", br#"{"hook_event_name":"preToolUse","tool_name":"Read","tool_input":{"file_path":"/a/b/c.rs"}}"#);
    assert_eq!((pre.tool.as_deref(), pre.detail.as_deref()), (Some("Read"), Some("b/c.rs")));
    let post = extract("cursor", br#"{"hook_event_name":"postToolUseFailure","tool_name":"Shell","tool_input":{"command":"x"}}"#);
    assert_eq!(post.tool.as_deref(), Some("Shell"));
}

/// `proc_pidinfo` is macOS-only; on other hosts a payload pid has no start time and is not stored.
#[cfg(target_os = "macos")]
#[test]
fn receipt_pid_refreshes_the_session_row() {
    use xai_grok_traces::hook::{extract, ingest};
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let path = e.home.join(".codex/sessions/2026/10/01/rollout-x.jsonl");
    let me = std::process::id();
    let payload = |pid: &str| format!(r#"{{"hook_event_name":"PostToolUse","session_id":"n1","transcript_path":"{}"{pid}}}"#, path.display());
    ingest(&mut idx, &e.roots, extract("codex", payload(&format!(r#","harness_pid":{me}"#)).as_bytes())).unwrap();
    let c = rusqlite::Connection::open(&e.db).unwrap();
    let pid = || c.query_row("SELECT harness_pid FROM trace_sessions WHERE source_id = 'codex'", [], |r| r.get::<_, Option<i64>>(0)).unwrap();
    assert_eq!(pid(), Some(i64::from(me)));
    // A receipt without a pid whose walk finds nothing never wipes it.
    let mut r = extract("codex", payload("").as_bytes());
    r.harness_pid = None;
    idx.record_receipt(&xai_grok_traces::Receipt { session_id: Some(xai_grok_traces::session_id("codex", &path.to_string_lossy())), ..r }).unwrap();
    assert_eq!(pid(), Some(i64::from(me)));
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

#[test]
fn same_size_edit_near_the_end_reparses() {
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let pi_root = e.roots.iter().find(|r| r.source == "pi").unwrap();
    let path = e.home.join(".pi/agent/sessions/--w-pi--/p.jsonl");
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&path, &PI.replace("\"ok\"", "\"no\""));
    assert!(idx.index_file(pi_root, &path).unwrap());
    // Touch only: same bytes, new mtime → no reparse.
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&path, &PI.replace("\"ok\"", "\"no\""));
    assert!(!idx.index_file(pi_root, &path).unwrap());
}

#[test]
fn compact_rows_round_trip() {
    let e = env();
    // 40 lines: blank/garbage mixed in so line numbers have gaps; usage on every 7th assistant line.
    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&match i % 7 {
            0 => format!("{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"model\":\"m\",\"content\":\"a{i}\",\"usage\":{{\"input_tokens\":{i},\"output_tokens\":2}}}}}}\n"),
            3 => "\n".to_string(),
            5 => "not json\n".to_string(),
            _ => format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"u{i}\"}}}}\n"),
        });
    }
    write(&e.home.join(".pi/agent/sessions/--w-pi--/many.jsonl"), &text);
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let row = idx.list_sessions(&SessionFilter { text: Some("many".into()), ..Default::default() }).unwrap().remove(0);
    for n in [0, 1, 7, 16, 17, 21, 35, 39] {
        let want = text.lines().nth(n).unwrap();
        if want.is_empty() || want == "not json" {
            assert_eq!(idx.raw_line(&row.id, n as i64).unwrap(), None, "line {n} has no event");
        } else {
            assert_eq!(idx.raw_line(&row.id, n as i64).unwrap().as_deref(), Some(want), "line {n}");
        }
    }
    let events = idx.list_events(&row.id, &EventFilter::default()).unwrap();
    assert!(events.iter().all(|e| !e.preview.is_empty()), "title stands in for an empty preview");
    let ev = |l: i64| events.iter().find(|e| e.line == l).unwrap();
    assert_eq!((ev(7).input_tokens, ev(7).output_tokens, ev(7).model.as_deref()), (Some(7), Some(2), Some("m")));
    assert_eq!(ev(1).input_tokens, None);
    assert_eq!((ev(1).kind.as_str(), ev(1).role.as_deref(), ev(1).preview.as_str()), ("message", Some("user"), "u1"));
}

#[test]
fn pathless_claude_and_pi_receipts_match_the_session_uuid() {
    use xai_grok_traces::hook::{extract, ingest};
    let e = env();
    write(&e.home.join(".claude/projects/-w-claude/3a680ac6-2551-4c67-88d3-a18d8b3bbe73.jsonl"), CLAUDE);
    write(&e.home.join(".pi/agent/sessions/--w-pi--/2026-10-01T00-00-00-000Z_0199aaaa-bbbb.jsonl"), PI);
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let key = |idx: &TraceIndex, id: i64| -> Option<String> {
        rusqlite::Connection::open(idx.db_path()).unwrap().query_row("SELECT session_id FROM trace_receipts WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    };
    let claude = ingest(&mut idx, &e.roots, extract("claude", br#"{"hook_event_name":"PermissionRequest","session_id":"3a680ac6-2551-4c67-88d3-a18d8b3bbe73","tool_name":"Bash"}"#)).unwrap();
    let want = idx.list_sessions(&SessionFilter { source: Some("claude".into()), ..Default::default() }).unwrap().into_iter().find(|s| s.file_path.to_string_lossy().contains("3a680ac6")).unwrap().id;
    assert_eq!(key(&idx, claude), Some(want));
    let pi = ingest(&mut idx, &e.roots, extract("pi", br#"{"event":"tool_execution_start","session_id":"0199aaaa-bbbb"}"#)).unwrap();
    let pi_id = idx.list_sessions(&SessionFilter { source: Some("pi".into()), ..Default::default() }).unwrap().into_iter().find(|s| s.file_path.to_string_lossy().contains("0199aaaa")).unwrap().id;
    assert_eq!(key(&idx, pi), Some(pi_id));
    let miss = ingest(&mut idx, &e.roots, extract("claude", br#"{"hook_event_name":"PermissionRequest","session_id":"3a680ac6"}"#)).unwrap();
    assert_eq!(key(&idx, miss), None, "a partial uuid is not a match");
}

#[test]
fn orphan_receipts_are_rekeyed_when_the_session_row_is_written() {
    use xai_grok_traces::hook::{extract, ingest};
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    // Before any session exists: stored unkeyed.
    let c = ingest(&mut idx, &e.roots, extract("claude", br#"{"hook_event_name":"PermissionRequest","session_id":"9f0e1d2c-0000-4000-8000-aaaaaaaaaaaa"}"#)).unwrap();
    let p = ingest(&mut idx, &e.roots, extract("pi", br#"{"event":"agent_start","session_id":"0199cccc-dddd"}"#)).unwrap();
    let other = ingest(&mut idx, &e.roots, extract("claude", br#"{"hook_event_name":"PermissionRequest","session_id":"someone-else"}"#)).unwrap();
    write(&e.home.join(".claude/projects/-w-claude/9f0e1d2c-0000-4000-8000-aaaaaaaaaaaa.jsonl"), CLAUDE);
    write(&e.home.join(".pi/agent/sessions/--w-pi--/2026-10-01T00-00-00-000Z_0199cccc-dddd.jsonl"), PI);
    idx.scan(&e.roots).unwrap();
    let key = |id: i64| -> Option<String> {
        rusqlite::Connection::open(&e.db).unwrap().query_row("SELECT session_id FROM trace_receipts WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    };
    let sid = |src: &str, needle: &str| idx.list_sessions(&SessionFilter { source: Some(src.into()), ..Default::default() }).unwrap().into_iter().find(|s| s.file_path.to_string_lossy().contains(needle)).unwrap().id;
    assert_eq!(key(c), Some(sid("claude", "9f0e1d2c")));
    assert_eq!(key(p), Some(sid("pi", "0199cccc")));
    assert_eq!(key(other), None);
}

#[test]
fn a_receipt_without_a_start_never_overwrites_the_stored_pair() {
    use xai_grok_traces::hook::{extract, ingest};
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let path = e.home.join(".codex/sessions/2026/10/01/rollout-x.jsonl");
    let me = std::process::id();
    ingest(&mut idx, &e.roots, extract("codex", format!(r#"{{"hook_event_name":"PostToolUse","transcript_path":"{}","harness_pid":{me}}}"#, path.display()).as_bytes())).unwrap();
    let sid = xai_grok_traces::session_id("codex", &path.to_string_lossy());
    let pair = || rusqlite::Connection::open(&e.db).unwrap().query_row("SELECT harness_pid, harness_start FROM trace_sessions WHERE id = ?1", [&sid], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?))).unwrap();
    let before = pair();
    if cfg!(target_os = "macos") {
        assert!(before.0.is_some() && before.1.is_some());
    }
    idx.record_receipt(&xai_grok_traces::Receipt { source: "codex".into(), event: "x".into(), session_id: Some(sid.clone()), harness_pid: Some(1234), harness_start: None, ..Default::default() }).unwrap();
    assert_eq!(pair(), before);
}

#[test]
fn a_payload_harness_pid_still_builds_the_chain() {
    use xai_grok_traces::hook::{extract, ingest};
    let e = env();
    let mut idx = TraceIndex::open(&e.db).unwrap();
    idx.scan(&e.roots).unwrap();
    let me = std::process::id();
    ingest(&mut idx, &e.roots, extract("pi", format!(r#"{{"event":"session_start","harness_pid":{me}}}"#).as_bytes())).unwrap();
    let chain: Option<String> = rusqlite::Connection::open(&e.db).unwrap().query_row("SELECT harness_chain FROM trace_receipts ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    if cfg!(target_os = "macos") {
        let chain = chain.expect("chain");
        assert!(chain.split('>').count() >= 2 && chain.split('>').count() <= 12, "{chain}");
    } else {
        assert_eq!(chain, None);
    }
}
