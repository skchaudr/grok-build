//! Read-only view of another machine's index over `ssh` (`grok-traces list|events|raw --json` there).
//!
//! * Hosts: `~/.grok/traces/hosts`, one ssh target per line (`sab-mini@sab-air`), `#` comments. Absent = none.
//! * Identity: the host *label* is the part after `@`. Remote rows get `host = Some(label)` and the id
//!   `label:<remote id>` (so ids never collide with local ones; `parent_id` is prefixed the same way).
//!   `fetch_events`/`fetch_raw` accept the prefixed id and strip it.
//! * Never blocks: ssh runs with BatchMode, a 2s connect timeout, and a hard 4s wall clock (child killed).
//!   Every failure is an [`Error`] value whose `Display` is UI-ready ("sab-air unreachable").
//! * Nothing is written to disk: the last good result per host lives in memory ([`cached_sessions`], [`cached_live`]).
//! * Version skew: if the remote binary rejects a flag, [`fetch_sessions`] falls back to plain
//!   `list --json`, filters locally and reports it in [`Fetched::note`].

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::live::{LIVE_SCHEMA, LiveRow};
use crate::query::{EventRow, SessionRow, Status};

pub const WALL_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_OUTPUT: u64 = 32 * 1024 * 1024;
const REMOTE_BIN: &str = "~/.grok/bin/grok-traces";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// ssh failed to connect/run, or timed out.
    Unreachable { host: String, detail: String },
    /// ssh worked, the remote command failed (binary missing, too old, ...).
    Remote { host: String, detail: String },
    /// Output was not what we expect.
    Parse { host: String, detail: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable { host, detail } => write!(f, "{} unreachable ({detail})", label(host)),
            Error::Remote { host, detail } => write!(f, "{}: {detail}", label(host)),
            Error::Parse { host, detail } => write!(f, "{}: bad response ({detail})", label(host)),
        }
    }
}
impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Rows plus an optional status line (e.g. version-skew degrade).
#[derive(Clone, Debug)]
pub struct Fetched<T = SessionRow> {
    pub rows: Vec<T>,
    pub note: Option<String>,
}

// Not derived: a derive would require `T: Default`.
impl<T> Default for Fetched<T> {
    fn default() -> Self {
        Fetched { rows: Vec::new(), note: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RemoteFilter {
    pub since_ms: Option<i64>,
    pub active_only: bool,
    pub roots_only: bool,
    /// 0 = remote default (50).
    pub limit: usize,
}

#[derive(Clone, Debug, Default)]
pub struct EventQuery {
    pub include_noise: bool,
    /// 0 = remote default.
    pub limit: usize,
}

/// How to run ssh. Tests point `bin` at a fake script.
#[derive(Clone, Debug)]
pub struct Ssh {
    pub bin: String,
    pub timeout: Duration,
}

impl Default for Ssh {
    fn default() -> Self {
        Ssh { bin: "ssh".into(), timeout: WALL_TIMEOUT }
    }
}

/// `sab-mini@sab-air` -> `sab-air`.
pub fn label(target: &str) -> &str {
    target.rsplit('@').next().unwrap_or(target)
}

pub fn hosts_path() -> PathBuf {
    crate::default_db_path().parent().map_or_else(|| PathBuf::from("."), PathBuf::from).join("hosts")
}

pub fn parse_hosts(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = l.split('#').next().unwrap_or("").trim();
        // A target is passed to ssh as an argument: refuse anything that could be an option or contain spaces.
        if !l.is_empty() && !l.starts_with('-') && !l.contains(char::is_whitespace) && !out.iter().any(|h| h == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// Configured remotes; a missing or unreadable file means none.
pub fn hosts() -> Vec<String> {
    std::fs::read_to_string(hosts_path()).map(|t| parse_hosts(&t)).unwrap_or_default()
}

// ---- in-memory cache of the last successful result per host ----

type Cache = Mutex<HashMap<String, (Instant, Vec<SessionRow>)>>;
static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();

fn cache() -> &'static Cache {
    CACHE.get_or_init(Default::default)
}

/// Last successful `fetch_sessions` rows for `target` and their age.
pub fn cached_sessions(target: &str) -> Option<(Duration, Vec<SessionRow>)> {
    let c = cache().lock().ok()?;
    c.get(target).map(|(at, rows)| (at.elapsed(), rows.clone()))
}

type LiveCache = Mutex<HashMap<String, (Instant, Vec<LiveRow>)>>;
static LIVE_CACHE: std::sync::OnceLock<LiveCache> = std::sync::OnceLock::new();

fn live_cache() -> &'static LiveCache {
    LIVE_CACHE.get_or_init(Default::default)
}

/// Last good `fetch_live` rows for `target` (tagged) and their age. Gated results and errors never replace it.
pub fn cached_live(target: &str) -> Option<(Duration, Vec<LiveRow>)> {
    let c = live_cache().lock().ok()?;
    c.get(target).map(|(at, rows)| (at.elapsed(), rows.clone()))
}

/// Newest first across local + remote rows, then truncated to `limit` (0 = no cap).
pub fn merge(local: Vec<SessionRow>, remote: Vec<Vec<SessionRow>>, limit: usize) -> Vec<SessionRow> {
    let mut all = local;
    all.extend(remote.into_iter().flatten());
    all.sort_by(|a, b| b.updated_at.unwrap_or(0).cmp(&a.updated_at.unwrap_or(0)).then_with(|| a.id.cmp(&b.id)));
    if limit > 0 {
        all.truncate(limit);
    }
    all
}

// ---- JSON wire format (shared with the `grok-traces` bin) ----

pub fn session_to_json(s: &SessionRow) -> Value {
    json!({
        "id": s.id, "source": s.source, "title": s.title, "path": s.file_path, "model": s.model, "cwd": s.cwd,
        "started_at": s.started_at, "updated_at": s.updated_at, "event_count": s.event_count, "user_count": s.user_count,
        "assistant_count": s.assistant_count, "tool_count": s.tool_count, "error_count": s.error_count,
        "input_tokens": s.input_tokens, "output_tokens": s.output_tokens, "duration_ms": s.duration_ms,
        "status": s.status.as_str(), "parent_id": s.parent_id, "label": s.label, "agent_type": s.agent_type,
        "now": s.now, "last_event_at": s.last_event_at, "children": s.children, "failed": s.failed,
        "native_id": s.native_id, "mtime_ms": s.mtime_ms, "harness_pid": s.harness_pid, "harness_start": s.harness_start,
        "context_tokens": s.context_tokens, "context_window": s.context_window, "via": s.via,
    })
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}
fn i64_of(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}
fn n_of(v: &Value, k: &str) -> i64 {
    i64_of(v, k).unwrap_or(0)
}

/// Tolerant: older remotes omit newer fields. `id` is required.
pub fn session_from_json(v: &Value) -> Option<SessionRow> {
    Some(SessionRow {
        id: str_of(v, "id")?,
        source: str_of(v, "source").unwrap_or_default(),
        title: str_of(v, "title").unwrap_or_default(),
        file_path: PathBuf::from(str_of(v, "path").unwrap_or_default()),
        model: str_of(v, "model"),
        cwd: str_of(v, "cwd"),
        started_at: i64_of(v, "started_at"),
        updated_at: i64_of(v, "updated_at"),
        event_count: n_of(v, "event_count"),
        user_count: n_of(v, "user_count"),
        assistant_count: n_of(v, "assistant_count"),
        tool_count: n_of(v, "tool_count"),
        error_count: n_of(v, "error_count"),
        input_tokens: i64_of(v, "input_tokens"),
        output_tokens: i64_of(v, "output_tokens"),
        duration_ms: i64_of(v, "duration_ms"),
        status: Status::parse(v.get("status").and_then(Value::as_str).unwrap_or("-")),
        parent_id: str_of(v, "parent_id"),
        label: str_of(v, "label"),
        agent_type: str_of(v, "agent_type"),
        now: str_of(v, "now"),
        last_event_at: i64_of(v, "last_event_at"),
        children: n_of(v, "children").max(0) as u32,
        failed: v.get("failed").and_then(Value::as_bool).unwrap_or(false),
        host: None,
        native_id: str_of(v, "native_id"),
        mtime_ms: i64_of(v, "mtime_ms"),
        harness_pid: i64_of(v, "harness_pid"),
        harness_start: i64_of(v, "harness_start"),
        context_tokens: i64_of(v, "context_tokens"),
        context_window: i64_of(v, "context_window"),
        via: str_of(v, "via"),
    })
}

pub fn event_to_json(e: &EventRow) -> Value {
    json!({
        "line": e.line, "type": e.event_type, "kind": e.kind, "role": e.role, "title": e.title, "preview": e.preview,
        "timestamp": e.timestamp, "duration_ms": e.duration_ms, "input_tokens": e.input_tokens,
        "output_tokens": e.output_tokens, "turn": e.turn, "step": e.step, "depth": e.depth, "model": e.model,
        "is_error": e.is_error, "noise": e.noise, "detail": e.detail,
    })
}

pub fn event_from_json(v: &Value) -> Option<EventRow> {
    Some(EventRow {
        line: i64_of(v, "line")?,
        event_type: str_of(v, "type").unwrap_or_default(),
        kind: str_of(v, "kind").unwrap_or_else(|| "message".into()),
        role: str_of(v, "role"),
        title: str_of(v, "title").unwrap_or_default(),
        preview: str_of(v, "preview").unwrap_or_default(),
        timestamp: i64_of(v, "timestamp"),
        duration_ms: i64_of(v, "duration_ms"),
        input_tokens: i64_of(v, "input_tokens"),
        output_tokens: i64_of(v, "output_tokens"),
        turn: i64_of(v, "turn"),
        step: i64_of(v, "step"),
        depth: n_of(v, "depth"),
        model: str_of(v, "model"),
        is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        noise: v.get("noise").and_then(Value::as_bool).unwrap_or(false),
        detail: str_of(v, "detail"),
    })
}

fn parse_lines<T>(target: &str, out: &str, f: impl Fn(&Value) -> Option<T>) -> Result<Vec<T>> {
    let mut rows = Vec::new();
    for (i, l) in out.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(l).map_err(|e| Error::Parse { host: target.into(), detail: format!("line {}: {e}", i + 1) })?;
        rows.push(f(&v).ok_or_else(|| Error::Parse { host: target.into(), detail: format!("line {}: missing field", i + 1) })?);
    }
    Ok(rows)
}

/// Marks rows as remote: host label, `label:`-prefixed id and parent id.
pub fn tag(target: &str, mut rows: Vec<SessionRow>) -> Vec<SessionRow> {
    let l = label(target);
    for r in &mut rows {
        r.id = format!("{l}:{}", r.id);
        r.parent_id = r.parent_id.take().map(|p| format!("{l}:{p}"));
        r.host = Some(l.to_string());
    }
    rows
}

/// The id as the remote knows it.
pub fn remote_id<'a>(target: &str, id: &'a str) -> &'a str {
    id.strip_prefix(label(target)).and_then(|r| r.strip_prefix(':')).unwrap_or(id)
}

// ---- ssh runner ----

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(ssh: &Ssh, target: &str, remote_cmd: &str) -> Result<Out> {
    let unreachable = |detail: String| Error::Unreachable { host: target.into(), detail };
    let mut child = Command::new(&ssh.bin)
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=2", "-o", "ServerAliveInterval=2", "-o", "ServerAliveCountMax=1"])
        .arg(target)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| unreachable(format!("ssh: {e}")))?;
    fn drain(r: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = r.take(MAX_OUTPUT).read_to_end(&mut b);
            let _ = tx.send(b);
        });
        rx
    }
    let out_rx = drain(child.stdout.take().expect("piped"));
    let err_rx = drain(child.stderr.take().expect("piped"));
    let deadline = Instant::now() + ssh.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unreachable(format!("timed out after {}s", ssh.timeout.as_secs())));
            }
            Err(e) => return Err(unreachable(e.to_string())),
        }
    };
    // A pipe that does not reach EOF in time is unknown output, not empty output: fail so callers keep their cache.
    let grab = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        rx.recv_timeout(Duration::from_millis(500)).map_err(|e| unreachable(format!("output not read: {e}")))
    };
    let (stdout, stderr) = (String::from_utf8_lossy(&grab(out_rx)?).into_owned(), String::from_utf8_lossy(&grab(err_rx)?).into_owned());
    let code = status.code().unwrap_or(-1);
    if code == 255 {
        let first = stderr.lines().next().unwrap_or("ssh failed").trim();
        return Err(unreachable(first.chars().take(120).collect()));
    }
    Ok(Out { code, stdout, stderr })
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(120).collect()
}

fn remote_failed(target: &str, o: &Out) -> Error {
    let detail = if o.code == 127 { "grok-traces not installed there".to_string() } else { first_line(&o.stderr) };
    Error::Remote { host: target.into(), detail: format!("exit {}: {detail}", o.code) }
}

/// Ids go into a remote shell command: only plain id characters.
fn safe_id(target: &str, id: &str) -> Result<()> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.')) {
        Ok(())
    } else {
        Err(Error::Remote { host: target.into(), detail: "invalid session id".into() })
    }
}

// ---- fetch ----

pub fn fetch_sessions(target: &str, f: &RemoteFilter) -> Result<Fetched> {
    fetch_sessions_with(&Ssh::default(), target, f)
}

pub fn fetch_sessions_with(ssh: &Ssh, target: &str, f: &RemoteFilter) -> Result<Fetched> {
    let mut cmd = format!("{REMOTE_BIN} list --json");
    if let Some(s) = f.since_ms {
        cmd += &format!(" --since {s}");
    }
    if f.active_only {
        cmd += " --active";
    }
    if f.roots_only {
        cmd += " --roots";
    }
    if f.limit > 0 {
        cmd += &format!(" --limit {}", f.limit);
    }
    let first = run(ssh, target, &cmd)?;
    let (rows, note) = if first.code == 0 {
        (parse_lines(target, &first.stdout, session_from_json)?, None)
    } else if first.code == 127 {
        return Err(remote_failed(target, &first));
    } else {
        // Older grok-traces: no such flag. Plain list, filter here.
        let plain = format!("{REMOTE_BIN} list --json --limit {}", if f.limit == 0 { 200 } else { (f.limit * 4).max(200) });
        let second = run(ssh, target, &plain)?;
        if second.code != 0 {
            return Err(remote_failed(target, &second));
        }
        let version = run(ssh, target, &format!("{REMOTE_BIN} version"))
            .ok()
            .filter(|o| o.code == 0)
            .map_or_else(|| "version unknown".to_string(), |o| first_line(&o.stdout));
        let mut rows = parse_lines(target, &second.stdout, session_from_json)?;
        rows.retain(|r| {
            f.since_ms.is_none_or(|s| r.updated_at.unwrap_or(0) >= s)
                && (!f.active_only || r.status == Status::Active)
                && (!f.roots_only || r.parent_id.is_none())
        });
        if f.limit > 0 {
            rows.truncate(f.limit);
        }
        (rows, Some(format!("{} remote grok-traces is older ({version}); filtered locally", label(target))))
    };
    let rows = tag(target, rows);
    if let Ok(mut c) = cache().lock() {
        c.insert(target.to_string(), (Instant::now(), rows.clone()));
    }
    Ok(Fetched { rows, note })
}

/// `grok-traces live --json` on `target`. Rows are tagged like sessions (`host`, `label:`-prefixed
/// `session_id` and `parent_id`; `native_id` stays as the remote knows it). A remote that exits 2, prints
/// nothing/garbage or speaks another `live_schema` gives `Ok` with no rows and a note: never guessed at.
pub fn fetch_live(target: &str) -> Result<Fetched<LiveRow>> {
    fetch_live_with(&Ssh::default(), target)
}

pub fn fetch_live_with(ssh: &Ssh, target: &str) -> Result<Fetched<LiveRow>> {
    let o = run(ssh, target, &format!("{REMOTE_BIN} live --json"))?;
    let gate = |schema: String| Ok(Fetched { rows: Vec::new(), note: Some(format!("{}: update grok-traces (live_schema {schema})", label(target))) });
    match o.code {
        0 => {}
        2 => return gate("none".into()),
        _ => return Err(remote_failed(target, &o)),
    }
    let Ok(v) = serde_json::from_str::<Value>(o.stdout.trim()) else { return gate("none".into()) };
    match v.get("live_schema").and_then(Value::as_u64) {
        None => return gate("none".into()),
        Some(n) if n != u64::from(LIVE_SCHEMA) => return gate(n.to_string()),
        Some(_) => {}
    }
    let rows: Vec<LiveRow> = serde_json::from_value(v.get("rows").cloned().unwrap_or(Value::Array(Vec::new())))
        .map_err(|e| Error::Parse { host: target.into(), detail: e.to_string() })?;
    let l = label(target);
    let rows: Vec<LiveRow> = rows
        .into_iter()
        .map(|mut r| {
            r.session_id = format!("{l}:{}", r.session_id);
            r.parent_id = r.parent_id.take().map(|p| format!("{l}:{p}"));
            r.host = Some(l.to_string());
            r
        })
        .collect();
    if let Ok(mut c) = live_cache().lock() {
        c.insert(target.to_string(), (Instant::now(), rows.clone()));
    }
    Ok(Fetched { rows, note: None })
}

pub fn fetch_events(target: &str, session_id: &str, q: &EventQuery) -> Result<Vec<EventRow>> {
    fetch_events_with(&Ssh::default(), target, session_id, q)
}

pub fn fetch_events_with(ssh: &Ssh, target: &str, session_id: &str, q: &EventQuery) -> Result<Vec<EventRow>> {
    let id = remote_id(target, session_id);
    safe_id(target, id)?;
    let mut cmd = format!("{REMOTE_BIN} events {id} --json");
    if q.include_noise {
        cmd += " --noise";
    }
    if q.limit > 0 {
        cmd += &format!(" --limit {}", q.limit);
    }
    let o = run(ssh, target, &cmd)?;
    match o.code {
        0 => parse_lines(target, &o.stdout, event_from_json),
        // Old binary: usage error for an unknown subcommand.
        2 => Err(Error::Remote { host: target.into(), detail: "remote grok-traces is too old for events; update it".into() }),
        _ => Err(remote_failed(target, &o)),
    }
}

/// `Ok(None)` = the remote has no such line.
pub fn fetch_raw(target: &str, session_id: &str, line: i64) -> Result<Option<String>> {
    fetch_raw_with(&Ssh::default(), target, session_id, line)
}

pub fn fetch_raw_with(ssh: &Ssh, target: &str, session_id: &str, line: i64) -> Result<Option<String>> {
    let id = remote_id(target, session_id);
    safe_id(target, id)?;
    let o = run(ssh, target, &format!("{REMOTE_BIN} raw {id} {line}"))?;
    match o.code {
        0 => Ok(Some(o.stdout.strip_suffix('\n').unwrap_or(&o.stdout).to_string())),
        3 => Ok(None),
        2 => Err(Error::Remote { host: target.into(), detail: "remote grok-traces is too old for raw; update it".into() }),
        _ => Err(remote_failed(target, &o)),
    }
}
