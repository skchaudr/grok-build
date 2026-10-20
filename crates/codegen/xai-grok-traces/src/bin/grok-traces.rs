//! `grok-traces hook <source> | scan | list [--json] [--limit N] [--since MS] [--active] [--roots] | events <id> --json [--noise] [--limit N] | raw <id> <line> | version | doctor [--reset]`.
//!
//! `hook` runs once per harness event: it must be fast, silent and always
//! exit 0 so it can never block an agent.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use xai_grok_traces::hook::{MAX_STDIN_BYTES, extract, ingest};
use xai_grok_traces::remote::{event_to_json, session_to_json};
use xai_grok_traces::{EventFilter, SessionFilter, TraceIndex, default_db_path, user_roots};

const HOOK_BUSY: Duration = Duration::from_millis(250);
const NOTIFY_EVERY: Duration = Duration::from_secs(3600);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => hook(args.get(1).map_or("unknown", String::as_str)),
        Some("scan") => finish(scan()),
        Some("list") => finish(list(&args[1..])),
        Some("doctor") => finish(doctor(&args[1..])),
        Some("events") => finish(events(&args[1..])),
        Some("raw") => raw(&args[1..]),
        Some("version") => {
            println!("grok-traces {} remote=1", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: grok-traces hook <source> | scan | list [--json] [--limit N] [--since MS] [--active] [--roots] | events <id> --json [--noise] [--limit N] | raw <id> <line> | version | doctor [--reset]");
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
        match TraceIndex::open_with(&db, Some(HOOK_BUSY)) {
            Ok(mut idx) => {
                if let Err(e) = ingest(&mut idx, &user_roots(), receipt) {
                    eprintln!("grok-traces hook {source}: {e}");
                }
                // Only a runaway tripwire is health; eviction never alerts.
                if let Some(msg) = idx.health() {
                    notify_once(db.parent().unwrap_or(Path::new(".")), &msg);
                }
            }
            Err(e) => eprintln!("grok-traces hook {source}: {e}"),
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
    let stamp = dir.join("tripped.notified");
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
    if let Some(msg) = &report.tripped {
        println!("  {msg}");
    }
    Ok(report.tripped.is_none())
}

fn list(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let mut filter = SessionFilter { limit: 50, ..Default::default() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut num = |name: &str| -> Result<i64, String> { it.next().and_then(|v| v.parse().ok()).ok_or(format!("{name} needs a number")) };
        match a.as_str() {
            "--json" => {}
            "--limit" => filter.limit = num("--limit")?.max(1) as usize,
            "--since" => filter.since_ms = Some(num("--since")?),
            "--active" => filter.active_only = true,
            "--roots" => filter.roots_only = true,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    let idx = TraceIndex::open(&default_db_path())?;
    let mut out = io::stdout().lock();
    for s in idx.list_sessions(&filter)? {
        writeln!(out, "{}", session_to_json(&s))?;
    }
    Ok(true)
}

fn events(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let id = args.first().ok_or("events needs a session id")?;
    let mut filter = EventFilter::default();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => {}
            "--noise" => filter.include_noise = true,
            "--limit" => filter.limit = it.next().and_then(|v| v.parse().ok()).ok_or("--limit needs a number")?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    let idx = TraceIndex::open(&default_db_path())?;
    let mut out = io::stdout().lock();
    for e in idx.list_events(id, &filter)? {
        writeln!(out, "{}", event_to_json(&e))?;
    }
    Ok(true)
}

/// Prints the raw source line. Exit 3 = no such line.
fn raw(args: &[String]) -> ExitCode {
    let run = || -> Result<Option<String>, Box<dyn std::error::Error>> {
        let id = args.first().ok_or("raw needs a session id and a line number")?;
        let line: i64 = args.get(1).and_then(|v| v.parse().ok()).ok_or("raw needs a line number")?;
        Ok(TraceIndex::open(&default_db_path())?.raw_line(id, line)?)
    };
    match run() {
        Ok(Some(l)) => {
            println!("{l}");
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::from(3),
        Err(e) => {
            eprintln!("grok-traces: {e}");
            ExitCode::from(1)
        }
    }
}

fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(|| ms.to_string(), |t| t.to_rfc3339())
}

/// Exit 1 unless the index is healthy. `--reset` first clears a tripwire (after you have looked).
fn doctor(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let idx = TraceIndex::open(&default_db_path())?;
    match args.first().map(String::as_str) {
        None => {}
        Some("--reset") => match idx.reset_trip()? {
            Some(was) => println!("reset    cleared: {was}"),
            None => println!("reset    nothing was tripped"),
        },
        Some(other) => return Err(format!("unknown flag {other}").into()),
    }
    let mb = |b: u64| b / (1024 * 1024);
    let stats = idx.stats()?;
    let health = idx.health();
    println!("db       {}", idx.db_path().display());
    let l = idx.limits();
    println!("size     {}MB (db + wal); budget {}MB (oldest sessions evicted), hard ceiling {}MB", mb(idx.db_bytes()), mb(l.budget), mb(l.ceiling));
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
