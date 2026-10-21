//! Step-1 end proof (spec line 240): real sessions through the live fold. Ignored by default.
//!
//! ```text
//! GROK_TRACES_REPLAY_SRC=~/.grok/traces/traces.sqlite \
//! GROK_TRACES_REPLAY_CAPTURE=~/.grok/traces/capture \
//!   cargo test -p xai-grok-traces --test live_replay -- --ignored --nocapture
//! ```
//!
//! The source db is copied (with its wal) into a scratch dir and opened read-only through plain rusqlite;
//! a fresh temp `TraceIndex` is the only db this code opens. Capture files are only read. Output carries
//! glyphs, tool names, event names and counts: never titles, details, cwds, paths or prompt text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use xai_grok_traces::hook::{extract, transcript_for};
use xai_grok_traces::live::{
    Ev, LiveReceipt, LiveState, fold_children, fold_session, norm, render, strict_ids,
};
use xai_grok_traces::roots::resolve;
use xai_grok_traces::{
    Receipt, Root, SessionFilter, SessionRow, TraceIndex, default_roots, session_id,
};

const GATE0_CLAUDE: &str = "2740b20d-a74f-4fef-ac50-9a159ff1df2e";
const PI: &str = "01a15e46-c4c8-7056-82cb-69d09fabd968";
const CURSOR: &str = "d4bc8505-5df3-4908-9226-dc1a468cdee6";

/// The variant name only: `Rejected(..)` and `Io(..)` carry paths.
fn kind<E: std::fmt::Debug>(e: &E) -> String {
    format!("{e:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

fn hms(ms: i64) -> String {
    let d = ms.rem_euclid(86_400_000);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        d / 3_600_000,
        d / 60_000 % 60,
        d / 1000 % 60,
        d % 1000
    )
}

fn under(path: &Path, dir: &Path) -> bool {
    match (dunce::canonicalize(path), dunce::canonicalize(dir)) {
        (Ok(p), Ok(d)) => p.starts_with(d),
        _ => false,
    }
}

/// Old schema-6 receipts of `native` sessions of `source`: only the columns that exist.
fn old_receipts(conn: &Connection, source: &str, natives: &[String]) -> Vec<Receipt> {
    let mut cols: Vec<String> = Vec::new();
    let mut stmt = conn.prepare("PRAGMA table_info(trace_receipts)").unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
    for c in rows {
        cols.push(c.unwrap());
    }
    let want = [
        "at_ms",
        "source",
        "event",
        "native_session",
        "path",
        "tool",
        "cwd",
    ];
    let have: Vec<&str> = want
        .iter()
        .copied()
        .filter(|c| cols.iter().any(|x| x == c))
        .collect();
    assert!(
        have.contains(&"at_ms") && have.contains(&"event"),
        "old receipts lack at_ms/event"
    );
    let sql = format!(
        "SELECT {} FROM trace_receipts WHERE source = ?1 ORDER BY at_ms, id",
        have.join(", ")
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let mut out = Vec::new();
    let mut rows = stmt.query([source]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let mut r = Receipt {
            source: source.into(),
            ..Default::default()
        };
        for (i, c) in have.iter().enumerate() {
            match *c {
                "at_ms" => r.at_ms = row.get(i).unwrap(),
                "event" => r.event = row.get(i).unwrap(),
                "native_session" => r.native_session = row.get(i).unwrap(),
                "path" => r.path = row.get(i).unwrap(),
                "tool" => r.tool = row.get(i).unwrap(),
                "cwd" => r.cwd = row.get(i).unwrap(),
                _ => {}
            }
        }
        if r.native_session
            .as_ref()
            .is_some_and(|n| natives.contains(n))
        {
            out.push(r);
        }
    }
    out
}

/// The three Claude sessions with the most `PreToolUse` receipts.
fn top_claude(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT native_session FROM trace_receipts WHERE source = 'claude' AND event = 'PreToolUse' AND native_session IS NOT NULL
             GROUP BY native_session ORDER BY COUNT(*) DESC, native_session LIMIT 3",
        )
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Capture receipts of one session, in `(mtime, filename)` order with `at_ms` = file mtime. Read only.
fn capture_receipts(dir: &Path, source: &str, native: &str) -> Vec<Receipt> {
    let mut files: Vec<(i64, String, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir.join(source)) else {
        return Vec::new();
    };
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(mtime) = meta.modified() else { continue };
        let ms = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        files.push((ms, e.file_name().to_string_lossy().into_owned(), e.path()));
    }
    files.sort();
    let mut out = Vec::new();
    for (ms, _, path) in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut payload = String::new();
        for line in text.lines() {
            if line.starts_with("--- process") {
                break;
            }
            payload.push_str(line);
            payload.push('\n');
        }
        let mut r = extract(source, payload.trim().as_bytes());
        r.at_ms = ms;
        if r.native_session.as_deref() == Some(native) {
            out.push(r);
        }
    }
    out
}

/// The Cursor chat transcript: `~/.cursor/projects/*/agent-transcripts/<uuid>/<uuid>.jsonl`.
fn cursor_transcript(home: &Path, uuid: &str) -> Option<PathBuf> {
    for proj in std::fs::read_dir(home.join(".cursor/projects"))
        .ok()?
        .flatten()
    {
        let f = proj
            .path()
            .join("agent-transcripts")
            .join(uuid)
            .join(format!("{uuid}.jsonl"));
        if f.is_file() {
            return Some(f);
        }
    }
    None
}

/// What `ingest` does, minus the PID walk: path -> index session id, else the native id within the source.
/// Indexes the file first so the session exists.
fn resolve_receipt(
    idx: &mut TraceIndex,
    roots: &[Root],
    r: &mut Receipt,
    indexed: &mut BTreeMap<PathBuf, bool>,
    errs: &mut Vec<String>,
) {
    let target = r
        .path
        .as_deref()
        .and_then(|p| resolve(&transcript_for(&r.source, Path::new(p)), roots));
    if let Some((root, path)) = target {
        r.session_id = Some(session_id(&root.source, &path.to_string_lossy()));
        if let std::collections::btree_map::Entry::Vacant(slot) = indexed.entry(path.clone()) {
            let ok = match idx.index_file(root, &path) {
                Ok(_) => true,
                Err(e) => {
                    errs.push(format!(
                        "{} index_file: {}",
                        r.native_session.as_deref().unwrap_or("?"),
                        kind(&e)
                    ));
                    false
                }
            };
            slot.insert(ok);
        }
    }
}

/// Claude subagent transcript `<session>/subagents/agent-<id>.jsonl`, when it exists (so B16 can match children).
fn index_agent_file(idx: &mut TraceIndex, roots: &[Root], r: &Receipt, errs: &mut Vec<String>) {
    let (Some(a), Some(p)) = (r.agent_id.as_deref(), r.path.as_deref()) else {
        return;
    };
    let p = Path::new(p);
    let file = p
        .with_extension("")
        .join("subagents")
        .join(format!("agent-{a}.jsonl"));
    if let Some((root, path)) = resolve(&file, roots)
        && let Err(e) = idx.index_file(root, &path)
    {
        errs.push(format!("agent index_file: {}", kind(&e)));
    }
}

fn session_row(idx: &TraceIndex, id: &str) -> Option<SessionRow> {
    idx.list_sessions(&SessionFilter {
        limit: 1_000_000,
        ..Default::default()
    })
    .ok()?
    .into_iter()
    .find(|s| s.id == id)
}

#[derive(Default)]
struct Summary {
    receipts: usize,
    changes: usize,
    glyph_counts: BTreeMap<String, usize>,
    seq: Vec<String>,
    close_older: usize,
    violations: usize,
}

/// Folds every prefix of the session's receipts at each receipt's own `at_ms`; prints glyph changes only.
fn replay(
    idx: &TraceIndex,
    sid: &str,
    label: &str,
    assert_post_closes: bool,
) -> (Summary, usize, usize) {
    let mut sum = Summary::default();
    let Some(row) = session_row(idx, sid) else {
        println!("== {label}: session row missing");
        return (sum, 0, 0);
    };
    let receipts: Vec<LiveReceipt> = idx.session_receipts(sid).unwrap_or_default();
    let kids = idx.children(sid).unwrap_or_default();
    let source = row.source.clone();
    let strict = strict_ids(&source);
    sum.receipts = receipts.len();
    println!("== {label} ({source}) receipts={}", receipts.len());
    let mut last = String::new();
    let mut prev_open = 0usize;
    // Each receipt's own time, plus one settled look 2s later when the next receipt is further off (Idle -> Prompt).
    let mut points: Vec<(usize, i64, bool)> = Vec::new();
    for k in 0..receipts.len() {
        points.push((k, receipts[k].at_ms, false));
        if receipts
            .get(k + 1)
            .is_none_or(|n| n.at_ms - receipts[k].at_ms > 2_000)
        {
            points.push((k, receipts[k].at_ms + 2_000, true));
        }
    }
    for (k, now, settled) in points {
        let prefix = &receipts[..=k];
        let f = fold_session(&source, &row, prefix, &kids, now, strict);
        let line = match &f.row {
            None => ("gone".to_string(), "end".to_string()),
            Some(r) => {
                let g = render(r, 0).glyph.to_string();
                let what = match r.state {
                    LiveState::Tool => {
                        let t = f
                            .open_tools
                            .last()
                            .and_then(|t| t.tool.clone())
                            .unwrap_or_else(|| "tool".into());
                        if r.extra_tools > 0 {
                            format!("{t} +{}", r.extra_tools)
                        } else {
                            t
                        }
                    }
                    LiveState::Waiting => "waiting".into(),
                    LiveState::Compacting => "compacting".into(),
                    LiveState::Prompt => "your-turn".into(),
                    LiveState::Working => "working".into(),
                };
                (g, what)
            }
        };
        let text = format!("{} {}", line.0, line.1);
        if text != last {
            println!("{} {text}", hms(now));
            *sum.glyph_counts.entry(line.0.clone()).or_default() += 1;
            sum.seq.push(line.0.clone());
            sum.changes += 1;
            last = text;
        }
        // close-older: a Post that closed more than the one tool it names.
        let ev = norm(&source, &receipts[k].event, receipts[k].detail.as_deref());
        if settled {
            continue;
        }
        if ev == Some(Ev::Post) && receipts[k].agent_id.is_none() {
            let now_open = f.open_tools.len();
            if prev_open > now_open + 1 {
                sum.close_older += 1;
            }
        }
        prev_open = f.open_tools.len();
        if assert_post_closes {
            for t in &f.open_tools {
                let pre = prefix.iter().rposition(|r| {
                    r.at_ms == t.at_ms
                        && r.tool_use_id == t.id
                        && r.tool == t.tool
                        && norm(&source, &r.event, r.detail.as_deref()) == Some(Ev::Pre)
                });
                let Some(pre) = pre else { continue };
                let closed = prefix.iter().skip(pre + 1).any(|r| {
                    r.agent_id.is_none()
                        && norm(&source, &r.event, r.detail.as_deref()) == Some(Ev::Post)
                        && match &t.id {
                            Some(id) => r.tool_use_id.as_deref() == Some(id.as_str()),
                            None => r.tool == t.tool,
                        }
                });
                if closed {
                    sum.violations += 1;
                    println!(
                        "VIOLATION {label}: open {:?} has a later Post (event {})",
                        t.tool, receipts[k].event
                    );
                }
            }
        }
        if let Some(r) = &f.row {
            let _ = fold_children(
                &source,
                &row,
                prefix,
                &kids,
                &f.open_children,
                r.age_ms + now,
                strict,
            );
        }
    }
    // agent_id -> child file match (B16).
    let mut ids: Vec<&str> = receipts
        .iter()
        .filter_map(|r| r.agent_id.as_deref())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mapped = ids
        .iter()
        .filter(|a| {
            let want = format!("agent-{a}");
            kids.iter().any(|k| {
                k.native_id.as_deref().and_then(|n| n.rsplit('/').next()) == Some(want.as_str())
            })
        })
        .count();
    (sum, ids.len(), mapped)
}

fn collapse(seq: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for g in seq.iter().take(60) {
        out.push(g.clone());
    }
    let s = out.join("");
    if seq.len() > 60 {
        format!("{s}... (+{})", seq.len() - 60)
    } else {
        s
    }
}

#[test]
#[ignore]
fn replay_real_sessions() {
    let (Some(src), Some(cap)) = (
        std::env::var_os("GROK_TRACES_REPLAY_SRC"),
        std::env::var_os("GROK_TRACES_REPLAY_CAPTURE"),
    ) else {
        println!("SKIP: set GROK_TRACES_REPLAY_SRC and GROK_TRACES_REPLAY_CAPTURE");
        return;
    };
    let (src, cap) = (PathBuf::from(src), PathBuf::from(cap));
    let real_home = xai_dirs::home_dir().expect("home");
    let grok_home = xai_dirs::grok_home();
    let traces_dir = grok_home.join("traces");

    // Scratch: copy of the source db (+ wal), and the only index this test opens.
    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join("src/copy.sqlite");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(&src, &copy).unwrap();
    let mut wal = src.clone().into_os_string();
    wal.push("-wal");
    if Path::new(&wal).exists() {
        let mut dst = copy.clone().into_os_string();
        dst.push("-wal");
        std::fs::copy(&wal, dst).unwrap();
    }
    let new_db = tmp.path().join("idx/x.sqlite");
    std::fs::create_dir_all(new_db.parent().unwrap()).unwrap();
    // Pre-flight: nothing we open is under ~/.grok/traces, and the new db is not the source copy.
    for p in [&copy, new_db.parent().unwrap()] {
        assert!(
            !under(p, &traces_dir),
            "scratch path inside the real traces dir"
        );
    }
    assert_ne!(
        dunce::canonicalize(new_db.parent().unwrap()).unwrap(),
        dunce::canonicalize(copy.parent().unwrap()).unwrap()
    );
    let old = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut idx = TraceIndex::open(&new_db).unwrap();
    assert!(
        !under(idx.db_path(), &traces_dir),
        "index db inside the real traces dir"
    );

    let roots = default_roots(&real_home, &grok_home);
    let mut indexed: BTreeMap<PathBuf, bool> = BTreeMap::new();
    let mut errs: Vec<String> = Vec::new();

    // (a) three old-copy Claude sessions.
    let top = top_claude(&old);
    let mut plan: Vec<(String, String, Vec<Receipt>)> = Vec::new();
    let all_old = old_receipts(&old, "claude", &top);
    for n in &top {
        let rs: Vec<Receipt> = all_old
            .iter()
            .filter(|r| r.native_session.as_deref() == Some(n))
            .cloned()
            .collect();
        plan.push((format!("claude-old-{}", plan.len() + 1), n.clone(), rs));
    }
    // (b) capture sessions.
    plan.push((
        "claude-gate0".into(),
        GATE0_CLAUDE.into(),
        capture_receipts(&cap, "claude", GATE0_CLAUDE),
    ));
    plan.push(("pi".into(), PI.into(), capture_receipts(&cap, "pi", PI)));
    plan.push((
        "cursor".into(),
        CURSOR.into(),
        capture_receipts(&cap, "cursor", CURSOR),
    ));

    // Index the Cursor transcript first, as a real session would already have one.
    let mut cursor_sid: Option<String> = None;
    if let Some(p) = cursor_transcript(&real_home, CURSOR) {
        if let Some((root, path)) = resolve(&p, &roots) {
            match idx.index_file(root, &path) {
                Ok(_) => cursor_sid = Some(session_id(&root.source, &path.to_string_lossy())),
                Err(e) => errs.push(format!("cursor index_file: {}", kind(&e))),
            }
        }
    } else {
        println!("cursor transcript not found");
    }

    let mut sids: Vec<(String, String)> = Vec::new();
    for (label, _native, mut receipts) in plan {
        receipts.sort_by_key(|r| r.at_ms);
        let mut sid: Option<String> = None;
        for r in receipts.iter_mut() {
            index_agent_file(&mut idx, &roots, r, &mut errs);
            resolve_receipt(&mut idx, &roots, r, &mut indexed, &mut errs);
            if r.session_id.is_none() && r.source == "cursor" {
                r.session_id = cursor_sid.clone();
            }
            if sid.is_none() {
                sid = r.session_id.clone();
            }
            idx.record_receipt(r).unwrap();
        }
        println!(
            "-- {label}: {} receipts imported, resolved={}",
            receipts.len(),
            sid.is_some()
        );
        if let Some(sid) = sid {
            sids.push((label, sid));
        }
    }
    for e in &errs {
        println!("ERR {e}");
    }

    let (mut tot_ids, mut tot_map, mut co, mut viol) = (0, 0, 0, 0);
    for (label, sid) in &sids {
        let (s, ids, mapped) = replay(&idx, sid, label, true);
        println!(
            "SUMMARY {label}: receipts={} changes={} glyphs={:?} seq={} close_older={} violations={} agent_ids={ids} mapped={mapped}",
            s.receipts,
            s.changes,
            s.glyph_counts,
            collapse(&s.seq),
            s.close_older,
            s.violations
        );
        tot_ids += ids;
        tot_map += mapped;
        co += s.close_older;
        viol += s.violations;
    }
    println!(
        "TOTAL sessions={} close_older={co} violations={viol} agent_ids={tot_ids} mapped={tot_map}",
        sids.len()
    );
    assert_eq!(viol, 0, "an open tool persists after a later Post with its id");
}
