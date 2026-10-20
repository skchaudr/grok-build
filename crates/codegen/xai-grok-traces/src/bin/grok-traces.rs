//! `grok-traces hook <source> | scan | list [--limit N] [--json] | doctor`.
//!
//! `hook` runs once per harness event: it must be fast, silent and always
//! exit 0 so it can never block an agent.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use xai_grok_traces::hook::{MAX_STDIN_BYTES, extract, ingest};
use xai_grok_traces::{Error, MAX_DB_BYTES, SessionFilter, TraceIndex, default_db_path, user_roots};

const HOOK_BUSY: Duration = Duration::from_millis(250);
const NOTIFY_EVERY: Duration = Duration::from_secs(3600);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => hook(args.get(1).map_or("unknown", String::as_str)),
        Some("scan") => finish(scan()),
        Some("list") => finish(list(&args[1..])),
        Some("doctor") => finish(doctor()),
        _ => {
            eprintln!("usage: grok-traces hook <source> | scan | list [--limit N] [--json] | doctor");
            ExitCode::from(2)
        }
    }
}

fn finish(r: Result<bool, Box<dyn std::error::Error>>) -> ExitCode {
    match r {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("grok-traces: {e}");
            ExitCode::from(1)
        }
    }
}

fn hook(source: &str) -> ExitCode {
    // A panic must not turn into a failing hook.
    let run = std::panic::catch_unwind(|| {
        let mut buf = Vec::new();
        let mut stdin = io::stdin().lock();
        let _ = (&mut stdin).take(MAX_STDIN_BYTES as u64).read_to_end(&mut buf);
        let _ = io::copy(&mut stdin, &mut io::sink());
        let receipt = extract(source, &buf);
        drop(buf);
        let db = default_db_path();
        let result = TraceIndex::open_with(&db, Some(HOOK_BUSY)).and_then(|mut idx| ingest(&mut idx, &user_roots(), receipt));
        if let Err(e) = result {
            eprintln!("grok-traces hook {source}: {e}");
            if matches!(e, Error::OverCap { .. }) {
                notify_once(db.parent().unwrap_or(Path::new(".")), &e.to_string());
            }
        }
    });
    if run.is_err() {
        eprintln!("grok-traces hook {source}: internal error");
    }
    if source == "cursor" {
        let _ = io::stdout().write_all(b"{}\n");
    }
    ExitCode::SUCCESS
}

/// macOS notification, at most once an hour (stamp file next to the db).
fn notify_once(dir: &Path, msg: &str) {
    let stamp = dir.join("overcap.notified");
    let fresh = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < NOTIFY_EVERY);
    if fresh || std::fs::write(&stamp, msg).is_err() {
        return;
    }
    if cfg!(target_os = "macos") {
        let text = msg.replace(['\\', '"'], "");
        let script = format!("display notification \"{text}\" with title \"grok-traces\"");
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

fn scan() -> Result<bool, Box<dyn std::error::Error>> {
    let mut idx = TraceIndex::open(&default_db_path())?;
    let t = std::time::Instant::now();
    let report = idx.scan(&user_roots())?;
    println!("indexed {} files in {:?}, {} errors", report.indexed, t.elapsed(), report.errors.len());
    for r in &report.roots {
        println!("  {:16} files={:6} accessible={} limited={}", r.id, r.files, r.accessible, r.limited);
    }
    for (path, e) in report.errors.iter().take(5) {
        println!("  error {}: {e}", path.display());
    }
    Ok(true)
}

fn list(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let mut limit = 50;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => {}
            "--limit" => limit = it.next().and_then(|v| v.parse().ok()).ok_or("--limit needs a number")?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    let idx = TraceIndex::open(&default_db_path())?;
    let mut out = io::stdout().lock();
    for s in idx.list_sessions(&SessionFilter { limit, ..Default::default() })? {
        let line = serde_json::json!({
            "id": s.id, "source": s.source, "title": s.title, "status": s.status.as_str(),
            "updated_at": s.updated_at, "path": s.file_path,
        });
        writeln!(out, "{line}")?;
    }
    Ok(true)
}

fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(|| ms.to_string(), |t| t.to_rfc3339())
}

/// Exit 1 unless the index is healthy.
fn doctor() -> Result<bool, Box<dyn std::error::Error>> {
    let idx = TraceIndex::open(&default_db_path())?;
    let mb = |b: u64| b / (1024 * 1024);
    let stats = idx.stats()?;
    let health = idx.health();
    println!("db       {}", idx.db_path().display());
    println!("size     {}MB / {}MB (db + wal)", mb(idx.db_bytes()), mb(MAX_DB_BYTES));
    println!("health   {}", health.as_deref().unwrap_or("ok"));
    println!(
        "version  schema={} parser={}",
        stats.schema_version.as_deref().unwrap_or("?"),
        stats.parser_version.as_deref().unwrap_or("?")
    );
    println!("indexed  sessions={} events={}", stats.sessions, stats.events);
    if stats.receipts.is_empty() {
        println!("receipts none");
    }
    for (source, n, last) in &stats.receipts {
        println!("receipts {source:8} count={n} last={}", when(*last));
    }
    Ok(health.is_none())
}
