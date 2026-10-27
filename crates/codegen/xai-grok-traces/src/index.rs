//! The disposable SQLite index. Source files are the truth; a corrupt index
//! is quarantined and rebuilt.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use xai_sqlite_journal::JournalMode;

use crate::lines;
use crate::links::{
    HEADER_LINES, GrokSpawn, Links, apply_grok_spawn, file_sig, grok_dispatch_kind, grok_links, grok_session_spawns, grok_spawn_for,
    grok_spawn_index, grok_summary, header_links, path_links, subagents_sig, write_links,
};
use crate::normalize::{Kind, NormalizedEvent, Role, clip, normalize};
use crate::roots::{Format, MAX_FILES, Root, discover, resolve};

mod bb_index;

use crate::bb::{Bb, BbThread, thread_key};

/// 7 re-reads grok transcripts so a subagent's parent comes from `subagents/<id>/meta.json`.
pub const PARSER_VERSION: i64 = 7;
/// Bump when the table layout changes; a mismatch rebuilds the (disposable) db.
pub const SCHEMA_VERSION: i64 = 7;
/// Size the index may use (db + `-wal`). Passing it is normal: the oldest sessions are evicted
/// down to `EVICT_TO_PERCENT` of it. It is a cache of pointers, so this is not an alert.
pub const SIZE_BUDGET: u64 = 256 * 1024 * 1024;
/// Eviction stops at this share of the budget, so it runs rarely rather than on every write.
pub const EVICT_TO_PERCENT: u64 = 85;
/// Eviction failed to keep the db under this (2x budget): something is wrong, writes pause.
pub const HARD_CEILING: u64 = 2 * SIZE_BUDGET;
/// Gross growth (size plus bytes evicted) within `GROWTH_WINDOW_MS` that trips: a feedback loop.
pub const GROWTH_LIMIT: u64 = 128 * 1024 * 1024;
pub const GROWTH_WINDOW_MS: i64 = 3_600_000;
/// Events indexed per file; the rest of a runaway file is left to the source.
pub const MAX_EVENTS_PER_FILE: usize = 200_000;
/// More files than this hitting the event cap in one scan trips.
pub const MAX_CAPPED_FILES: usize = 5;
/// Growth is sampled on every Nth hook receipt (and at the end of each scan).
pub(crate) const SAMPLE_EVERY: i64 = 100;
const MAX_SAMPLES: usize = 100;
const MIN_SAMPLE_GAP_MS: i64 = 60_000;
/// Sessions deleted per eviction transaction.
const EVICT_BATCH: i64 = 25;

/// Thresholds; `Default` is production. Tests pass small ones instead of building huge files.
#[derive(Clone, Debug)]
pub struct Limits {
    pub budget: u64,
    pub ceiling: u64,
    pub growth_bytes: u64,
    pub growth_window_ms: i64,
    pub max_events: usize,
    pub max_capped_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            budget: SIZE_BUDGET,
            ceiling: HARD_CEILING,
            growth_bytes: GROWTH_LIMIT,
            growth_window_ms: GROWTH_WINDOW_MS,
            max_events: MAX_EVENTS_PER_FILE,
            max_capped_files: MAX_CAPPED_FILES,
        }
    }
}

fn mb(b: u64) -> u64 {
    b / (1024 * 1024)
}

const TRIP_HELP: &str = "writes paused; run grok-traces doctor, then doctor --reset";
const TITLE_CHARS: usize = 180;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS trace_roots (id TEXT PRIMARY KEY, source_id TEXT NOT NULL, path TEXT NOT NULL,
  exists_flag INTEGER NOT NULL DEFAULT 0, file_count INTEGER NOT NULL DEFAULT 0, last_scan_at INTEGER, error TEXT);
CREATE TABLE IF NOT EXISTS trace_files (path TEXT PRIMARY KEY, root_id TEXT NOT NULL, source_id TEXT NOT NULL,
  format TEXT NOT NULL, size_bytes INTEGER NOT NULL, mtime_ms INTEGER NOT NULL, indexed_bytes INTEGER NOT NULL DEFAULT 0,
  indexed_lines INTEGER NOT NULL DEFAULT 0, parser_version INTEGER NOT NULL DEFAULT 0, content_hash TEXT,
  session_id TEXT NOT NULL, indexed_at INTEGER NOT NULL, parse_error TEXT, aux_sig TEXT);
CREATE TABLE IF NOT EXISTS trace_sessions (sk INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, source_id TEXT NOT NULL,
  title TEXT NOT NULL, file_path TEXT NOT NULL UNIQUE, model TEXT, cwd TEXT, started_at INTEGER, updated_at INTEGER,
  event_count INTEGER NOT NULL DEFAULT 0, user_count INTEGER NOT NULL DEFAULT 0, assistant_count INTEGER NOT NULL DEFAULT 0,
  tool_count INTEGER NOT NULL DEFAULT 0, error_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER, output_tokens INTEGER,
  duration_ms INTEGER, status TEXT NOT NULL DEFAULT 'unknown', file_size_bytes INTEGER NOT NULL DEFAULT 0,
  -- Dispatch tree: `native_id` is how the harness names this session (a path under the root, a uuid, a thread id),
  -- `parent_native` the same for its dispatcher. Both resolve within one source at query time, so scan order is free.
  native_id TEXT, parent_native TEXT, label TEXT, agent_type TEXT, done INTEGER NOT NULL DEFAULT 0,
  -- What the session is doing: last meaningful event (`Tool: arg` or assistant text), its time, and whether it ended in failure.
  signal_count INTEGER NOT NULL DEFAULT 0, now_text TEXT, last_event_at INTEGER, failed INTEGER NOT NULL DEFAULT 0,
  -- Live: the harness process of the newest receipt (start in Unix seconds), and the last context size seen.
  harness_pid INTEGER, harness_start INTEGER, context_tokens INTEGER, context_window INTEGER,
  -- Where the harness was launched from (`bb`, `zed`, `herdr`, `cursor`), from the newest receipt's `harness_chain`.
  via TEXT);
-- Event types, titles and models are few and repeat on every row, so rows store ids into this table.
CREATE TABLE IF NOT EXISTS trace_names (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
-- kind/role are `Kind::code`/`Role::code`. No cwd here: it is session-level. `preview` is '' when it equals the
-- title. `byte_offset` is kept only on every `CHECKPOINT_EVERY`th line; a raw line is found by scanning on from there.
CREATE TABLE IF NOT EXISTS trace_events (sk INTEGER NOT NULL, line_number INTEGER NOT NULL, byte_offset INTEGER, type_id INTEGER NOT NULL, kind INTEGER NOT NULL, role INTEGER, title_id INTEGER NOT NULL, preview TEXT NOT NULL,
  timestamp INTEGER, depth INTEGER NOT NULL DEFAULT 0, model_id INTEGER, is_error INTEGER NOT NULL DEFAULT 0,
  noise INTEGER NOT NULL DEFAULT 0, detail TEXT,
  PRIMARY KEY (sk, line_number)) WITHOUT ROWID;
-- Most events carry none of these; only the ones that do get a row.
CREATE TABLE IF NOT EXISTS trace_event_usage (sk INTEGER NOT NULL, line_number INTEGER NOT NULL, duration_ms INTEGER,
  input_tokens INTEGER, output_tokens INTEGER, turn INTEGER, step INTEGER, context_tokens INTEGER, context_window INTEGER,
  PRIMARY KEY (sk, line_number)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS trace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS trace_receipts (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, source TEXT NOT NULL,
  event TEXT NOT NULL, native_session TEXT, path TEXT, session_id TEXT, tool TEXT, cwd TEXT,
  detail TEXT, tool_use_id TEXT, agent_id TEXT, harness_pid INTEGER, harness_start INTEGER, harness_chain TEXT);
CREATE INDEX IF NOT EXISTS trace_receipts_session ON trace_receipts(session_id, at_ms, id);
CREATE INDEX IF NOT EXISTS trace_receipts_at ON trace_receipts(at_ms, id);
-- Receipts not yet keyed to a session row: the native-id branch of every receipt match, and the receipt-only grouping.
CREATE INDEX IF NOT EXISTS trace_receipts_native ON trace_receipts(source, native_session, at_ms, id) WHERE session_id IS NULL;
CREATE INDEX IF NOT EXISTS trace_sessions_updated ON trace_sessions(updated_at DESC);
CREATE INDEX IF NOT EXISTS trace_sessions_source ON trace_sessions(source_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS trace_sessions_native ON trace_sessions(source_id, native_id);
CREATE INDEX IF NOT EXISTS trace_sessions_parent ON trace_sessions(source_id, parent_native);
";

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(io::Error),
    /// A runaway tripwire fired (see `Limits`); writes are refused until `reset_trip`.
    Tripped(String),
    /// A path that is not a trace source of the given root.
    Rejected(PathBuf),
}

fn ceiling_message(bytes: u64, l: &Limits) -> String {
    format!(
        "tripped: hard ceiling — db {}MB > {}MB (2x the {}MB budget; eviction could not keep up) — {TRIP_HELP}",
        mb(bytes),
        mb(l.ceiling),
        mb(l.budget)
    )
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "sqlite: {e}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Tripped(msg) => f.write_str(msg),
            Error::Rejected(p) => write!(f, "not a trace source: {}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sqlite(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn default_db_path() -> PathBuf {
    xai_dirs::grok_home().join("traces").join("traces.sqlite")
}

pub fn session_id(source: &str, file_path: &str) -> String {
    let mut h = Sha1::new();
    h.update(source.as_bytes());
    h.update([0u8]);
    h.update(file_path.as_bytes());
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{source}:{}", &hex[..24])
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// Bytes before `indexed_bytes` that vouch the old prefix is unchanged. Bounded so an
/// append costs the same on a 200MB transcript as on a 2KB one.
const TAIL_WINDOW: u64 = 64 * 1024;

/// sha256 of the last `TAIL_WINDOW` bytes before `end`.
fn tail_hash(path: &Path, end: u64) -> io::Result<String> {
    use std::io::{Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(end.saturating_sub(TAIL_WINDOW)))?;
    let mut h = Sha256::new();
    io::copy(&mut file.take(end.min(TAIL_WINDOW)), &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn is_unusable(e: &rusqlite::Error) -> bool {
    use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
    match e {
        rusqlite::Error::SqliteFailure(f, msg) => {
            matches!(f.code, DatabaseCorrupt | NotADatabase)
                || msg.as_deref().is_some_and(|m| m.contains("malformed") || m.contains("not a database"))
        }
        _ => false,
    }
}

/// Renames the db and its sidecars to `*.<tag>.<nanos>`; returns the new names.
fn move_aside(db: &Path, tag: &str) -> Vec<PathBuf> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut moved = Vec::new();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let from = PathBuf::from(format!("{}{suffix}", db.display()));
        if from.exists() {
            let to = PathBuf::from(format!("{}{suffix}.{tag}.{nanos}", db.display()));
            match fs::rename(&from, &to) {
                Ok(()) => moved.push(to),
                Err(e) => tracing::warn!(?from, %e, "traces: rename failed"),
            }
        }
    }
    moved
}

fn quarantine(db: &Path) {
    move_aside(db, "corrupt");
}

/// Drops a db from an older schema/parser: a smaller row does not shrink a SQLite file.
fn retire(db: &Path) {
    for old in move_aside(db, "old") {
        if let Err(e) = fs::remove_file(&old) {
            tracing::warn!(?old, %e, "traces: removing old index failed");
        }
    }
}

#[derive(Debug, Default)]
pub struct RootReport {
    pub id: String,
    pub files: usize,
    pub accessible: bool,
    pub limited: bool,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub roots: Vec<RootReport>,
    pub indexed: usize,
    pub errors: Vec<(PathBuf, String)>,
    /// Set when this scan tripped a runaway tripwire (also in `TraceIndex::health`).
    pub tripped: Option<String>,
}

/// One unit of scan work, ordered by recency.
enum Work {
    File(PathBuf),
    Bb(BbThread),
}

/// What discovery learned about a root, applied after its files are indexed.
struct RootPlan {
    files: usize,
    accessible: bool,
    limited: bool,
    error: Option<String>,
    /// Keys present now; `None` leaves cached rows alone (discovery was incomplete).
    seen: Option<HashSet<String>>,
    bb: Option<Bb>,
}

struct FileRow {
    size_bytes: i64,
    mtime_ms: i64,
    indexed_bytes: i64,
    indexed_lines: i64,
    parser_version: i64,
    content_hash: Option<String>,
    aux_sig: Option<String>,
}

#[derive(Default)]
struct Aggregate {
    title: String,
    user_title: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    events: i64,
    users: i64,
    assistants: i64,
    tools: i64,
    errors: i64,
    input: Option<i64>,
    output: Option<i64>,
    /// A title the harness recorded itself; wins over the first user message.
    session_title: Option<String>,
    /// Non-noise events.
    signals: i64,
    /// Last meaningful event: `Tool: arg` or assistant text.
    now: Option<String>,
    /// Newest timestamp among non-noise events.
    last_signal_ts: Option<i64>,
    /// The last non-noise event is an error: nothing happened after it.
    failed: bool,
    /// Latest per-message context size and window seen.
    context_tokens: Option<i64>,
    context_window: Option<i64>,
}

/// `Tool: arg`, clipped to the preview length.
fn now_line(tool: &str, detail: Option<&str>) -> String {
    let line = match detail {
        Some(d) => format!("{tool}: {d}"),
        None => tool.to_string(),
    };
    crate::normalize::preview(&line)
}

impl Aggregate {
    fn add(&mut self, e: &NormalizedEvent) {
        self.events += 1;
        match e.role {
            Some(Role::User) => self.users += 1,
            Some(Role::Assistant) => self.assistants += 1,
            _ => {}
        }
        if e.kind == Kind::Tool {
            self.tools += 1;
        }
        if e.is_error() {
            self.errors += 1;
        }
        if let Some(ts) = e.timestamp_ms {
            self.first_ts = Some(self.first_ts.map_or(ts, |f| f.min(ts)));
            self.last_ts = Some(self.last_ts.map_or(ts, |l| l.max(ts)));
        }
        if !e.noise {
            self.signals += 1;
            if let Some(ts) = e.timestamp_ms {
                self.last_signal_ts = Some(self.last_signal_ts.map_or(ts, |l| l.max(ts)));
            }
            if e.role == Some(Role::Tool) || e.tool.is_some() {
                self.now = Some(now_line(e.tool.as_deref().unwrap_or(&e.title), e.detail.as_deref()));
            } else if e.role == Some(Role::Assistant) && e.kind == Kind::Message && e.text != e.title {
                self.now = Some(e.preview.clone());
            }
        }
        // An error stands until anything else happens (a later turn, call or prompt).
        if e.is_error() {
            self.failed = true;
        } else if !e.noise {
            self.failed = false;
        }
        if let Some(t) = &e.session_title {
            self.session_title = Some(t.clone());
        }
        // Injected context (`<user_info>`, `<system-reminder>`, `<command-name>`) is not a title.
        if self.user_title.is_none()
            && e.role == Some(Role::User)
            && !e.noise
            && !e.text.is_empty()
            && !e.text.trim_start().starts_with('<')
        {
            let first = e.text.lines().next().unwrap_or(&e.text);
            self.user_title = Some(clip(first, TITLE_CHARS));
        }
        if self.model.is_none() {
            self.model = e.model.clone();
        }
        if self.cwd.is_none() {
            self.cwd = e.cwd.clone();
        }
        let fold = |total: &mut Option<i64>, v: Option<f64>| {
            if let Some(v) = v {
                let v = v.round() as i64;
                *total = Some(if e.usage_is_total { total.unwrap_or(0).max(v) } else { total.unwrap_or(0) + v });
            }
        };
        self.context_tokens = e.context_tokens.or(self.context_tokens);
        self.context_window = e.context_window.or(self.context_window);
        fold(&mut self.input, e.input_tokens);
        fold(&mut self.output, e.output_tokens);
    }
}

const INSERT_EVENT: &str = "INSERT OR REPLACE INTO trace_events (sk, line_number, byte_offset, type_id, kind, role, title_id,
  preview, timestamp, depth, model_id, is_error, noise, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)";
const INSERT_USAGE: &str = "INSERT OR REPLACE INTO trace_event_usage (sk, line_number, duration_ms, input_tokens, output_tokens,
  turn, step, context_tokens, context_window) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
/// Every Nth line keeps its byte offset.
pub(crate) const CHECKPOINT_EVERY: i64 = 16;

/// Interns strings into `trace_names` for the length of one write transaction.
#[derive(Default)]
struct Names(std::collections::HashMap<String, i64>);

impl Names {
    fn id(&mut self, conn: &Connection, name: &str) -> rusqlite::Result<i64> {
        if let Some(id) = self.0.get(name) {
            return Ok(*id);
        }
        let id: i64 = conn.prepare_cached("INSERT INTO trace_names (name) VALUES (?1) ON CONFLICT(name) DO UPDATE SET name = name RETURNING id")?
            .query_row([name], |r| r.get(0))?;
        self.0.insert(name.to_string(), id);
        Ok(id)
    }
}

/// Inserts or refreshes a session row and returns its integer key.
fn upsert_session(tx: &Transaction<'_>, sid: &str, source: &str, title: &str, file_path: &str, size: i64) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO trace_sessions (id, source_id, title, file_path, file_size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET file_path = ?4, file_size_bytes = ?5",
        params![sid, source, title, file_path, size],
    )?;
    tx.query_row("SELECT sk FROM trace_sessions WHERE id = ?1", [sid], |r| r.get(0))
}

/// One event row; `line` is the line number in a file, or the event sequence for bb.
fn insert_event(
    insert: &mut rusqlite::CachedStatement<'_>,
    usage: &mut rusqlite::CachedStatement<'_>,
    names: &mut Names,
    conn: &Connection,
    sk: i64,
    line: i64,
    offset: i64,
    e: &NormalizedEvent,
) -> rusqlite::Result<()> {
    let round = |v: Option<f64>| v.map(|v| v.round() as i64);
    let model = e.model.as_deref().map(|m| names.id(conn, m)).transpose()?;
    insert.execute(params![
        sk,
        line,
        (line % CHECKPOINT_EVERY == 0).then_some(offset),
        names.id(conn, &e.event_type)?,
        e.kind.code(),
        e.role.map(Role::code),
        names.id(conn, &e.title)?,
        // A call's detail already says what it did; its raw arguments are not kept.
        if e.preview == e.title || (e.role == Some(Role::Tool) && e.detail.is_some()) { "" } else { e.preview.as_str() },
        e.timestamp_ms,
        e.depth,
        model,
        e.is_error(),
        e.noise,
        e.detail,
    ])?;
    let extra = [e.duration_ms, e.input_tokens, e.output_tokens, e.turn, e.step].map(round);
    if extra.iter().any(Option::is_some) || e.context_tokens.is_some() || e.context_window.is_some() {
        usage.execute(params![sk, line, extra[0], extra[1], extra[2], extra[3], extra[4], e.context_tokens, e.context_window])?;
    }
    Ok(())
}

fn delete_events(tx: &Transaction<'_>, sk: i64) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM trace_events WHERE sk = ?1", [sk])?;
    tx.execute("DELETE FROM trace_event_usage WHERE sk = ?1", [sk])?;
    Ok(())
}

/// Last event type marks a finished run.
fn is_terminal(event_type: &str) -> bool {
    let squashed: String = event_type
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '/' | '_' | ' ' | '-'))
        .collect();
    ["sessionend", "sessionshutdown", "runend", "agentend"].iter().any(|w| squashed.ends_with(w))
}

/// What `TraceIndex::peek` found.
enum Peek {
    Ready,
    /// No tables yet: new, or just quarantined.
    Empty,
    /// Written by an older schema or parser.
    Stale,
    /// Written by a newer one; leave it alone.
    Newer(String),
}

/// The most a process waits for another's rebuild before giving up (a hook then skips).
const REBUILD_WAIT: Duration = Duration::from_secs(2);
/// A lock this old belongs to a crashed process.
const LOCK_STALE: Duration = Duration::from_secs(60);

/// `<db>.rebuild.lock`, created exclusively; removed on drop.
struct RebuildLock(PathBuf);

impl RebuildLock {
    fn path(db: &Path) -> PathBuf {
        PathBuf::from(format!("{}.rebuild.lock", db.display()))
    }

    /// Waits (bounded) while someone else holds the lock; a stale lock does not count.
    fn wait_free(db: &Path) {
        let path = Self::path(db);
        let deadline = std::time::Instant::now() + REBUILD_WAIT;
        while std::time::Instant::now() < deadline {
            let held = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age <= LOCK_STALE);
            if !held {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn acquire(db: &Path) -> io::Result<Self> {
        let path = Self::path(db);
        let deadline = std::time::Instant::now() + REBUILD_WAIT;
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = io::Write::write_all(&mut f, std::process::id().to_string().as_bytes());
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(io::Error::new(io::ErrorKind::WouldBlock, "index is being rebuilt by another process"));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for RebuildLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub struct TraceIndex {
    conn: Connection,
    /// The db file actually opened (per-host on network mounts).
    db: PathBuf,
    limits: Limits,
    /// Files that hit the event cap since the last scan began.
    capped_files: usize,
    /// During `scan`, child id → spawn for each grok sessions root. `None` outside a scan, so a hook re-reads disk.
    grok_spawns: Option<HashMap<PathBuf, HashMap<String, GrokSpawn>>>,
}

impl TraceIndex {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, None)
    }

    /// `busy`: lock wait for short-lived writers (hooks); `None` keeps the shared default.
    pub fn open_with(path: &Path, busy: Option<Duration>) -> Result<Self> {
        Self::open_with_limits(path, busy, Limits::default())
    }

    pub fn open_with_limits(path: &Path, busy: Option<Duration>, limits: Limits) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mode = JournalMode::for_db_path(path);
        let db = mode.effective_db_path(path);
        // Looking is read-only and never sets a journal mode, so it cannot disturb a rebuild in
        // progress. Building, quarantining and version rebuilds are serialized by a lock file (see
        // `rebuild`). A few rounds, then an error rather than a loop.
        let mut last: Option<rusqlite::Error> = None;
        for _ in 0..6 {
            match Self::peek(&db, busy) {
                Ok(Peek::Ready) => {
                    let mut idx = Self::attach(mode, &db, busy)?;
                    idx.limits = limits;
                    return Ok(idx);
                }
                Ok(Peek::Newer(msg)) => return Err(io::Error::other(msg).into()),
                Ok(Peek::Stale | Peek::Empty) => Self::rebuild(mode, &db, busy)?,
                Err(e) if is_unusable(&e) => Self::rebuild(mode, &db, busy)?,
                Err(e) => {
                    // Mid-rebuild states look like I/O errors or locks: wait for the lock, look again.
                    last = Some(e);
                    RebuildLock::wait_free(&db);
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
        Err(last.map_or_else(|| io::Error::other("index rebuild did not stick").into(), Error::from))
    }

    /// Reads the version keys with a plain connection: no pragma, no write.
    fn peek(db: &Path, busy: Option<Duration>) -> rusqlite::Result<Peek> {
        let conn = Connection::open(db)?;
        conn.busy_timeout(busy.unwrap_or(Duration::from_secs(5)))?;
        // No integrity scan here: it is O(db) and the pager opens on the UI thread.
        // A garbage file fails on the first query below.
        let has_meta = conn
            .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'trace_meta'", [], |_| Ok(()))
            .optional()?
            .is_some();
        if !has_meta {
            return Ok(Peek::Empty);
        }
        let stored = |key: &str| -> Option<i64> {
            conn.query_row("SELECT value FROM trace_meta WHERE key = ?1", [key], |r| r.get::<_, String>(0)).ok()?.parse().ok()
        };
        let (schema, parser) = (stored("schema_version"), stored("parser_version"));
        if schema == Some(SCHEMA_VERSION) && parser == Some(PARSER_VERSION) {
            return Ok(Peek::Ready);
        }
        // An older index is replaced. A newer one belongs to a newer binary: replacing it would make
        // two versions rebuild each other forever, so this one steps aside.
        if schema > Some(SCHEMA_VERSION) || parser > Some(PARSER_VERSION) {
            return Ok(Peek::Newer(format!(
                "index is from a newer grok-traces (schema {schema:?}, parser {parser:?}; this is {SCHEMA_VERSION}, {PARSER_VERSION}); not touching it"
            )));
        }
        Ok(Peek::Stale)
    }

    fn connect(mode: JournalMode, db: &Path, busy: Option<Duration>) -> rusqlite::Result<Connection> {
        Ok(match busy {
            None => mode.open(db)?,
            Some(wait) => {
                let conn = Connection::open(db)?;
                conn.busy_timeout(wait)?;
                mode.apply(&conn)?;
                conn
            }
        })
    }

    /// Opens a db that `peek` found current.
    fn attach(mode: JournalMode, db: &Path, busy: Option<Duration>) -> Result<Self> {
        let conn = Self::connect(mode, db, busy)?;
        Ok(Self { conn, db: db.to_path_buf(), limits: Limits::default(), capped_files: 0, grok_spawns: None })
    }

    /// Creates, replaces or quarantines the db while holding the rebuild lock. Whoever loses the race
    /// waits (at most `REBUILD_WAIT`) and finds the finished db; it never builds a second one over it.
    fn rebuild(mode: JournalMode, db: &Path, busy: Option<Duration>) -> Result<()> {
        let _lock = RebuildLock::acquire(db)?;
        match Self::peek(db, busy) {
            Ok(Peek::Ready | Peek::Newer(_)) => return Ok(()),
            Ok(Peek::Stale) => {
                tracing::info!(?db, "traces: index schema changed, rebuilding");
                retire(db);
            }
            Ok(Peek::Empty) => {}
            Err(e) if is_unusable(&e) => {
                tracing::warn!(?db, %e, "traces: index unusable, rebuilding");
                quarantine(db);
            }
            Err(e) => return Err(e.into()),
        }
        let conn = Self::connect(mode, db, busy)?;
        // Lets eviction give pages back to the filesystem. The journal mode has already written the
        // header, so the mode only takes effect through a VACUUM (free here: the db is empty).
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        conn.execute_batch("VACUUM;")?;
        conn.execute_batch(&format!(
            "BEGIN; {SCHEMA}
             INSERT OR REPLACE INTO trace_meta (key, value) VALUES ('schema_version', '{SCHEMA_VERSION}'), ('parser_version', '{PARSER_VERSION}');
             COMMIT;"
        ))?;
        Ok(())
    }

    pub fn db_path(&self) -> &Path {
        &self.db
    }

    /// Db plus `-wal` size on disk.
    pub fn db_bytes(&self) -> u64 {
        let size = |p: PathBuf| fs::metadata(p).map_or(0, |m| m.len());
        size(self.db.clone()) + size(PathBuf::from(format!("{}-wal", self.db.display())))
    }

    pub(crate) fn meta(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row("SELECT value FROM trace_meta WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO trace_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Gate for every write. A tripped index refuses. Past the budget the oldest sessions are
    /// evicted first (silently); only if the db is still past the hard ceiling does it trip.
    pub fn check_writable(&self) -> Result<()> {
        if let Some(msg) = self.meta("tripped")? {
            return Err(Error::Tripped(msg));
        }
        if self.db_bytes() > self.limits.budget {
            self.evict_if_over()?;
            let bytes = self.db_bytes();
            if bytes > self.limits.ceiling {
                return Err(self.trip(ceiling_message(bytes, &self.limits)));
            }
        }
        Ok(())
    }

    /// Records a tripwire; stays until `reset_trip`.
    fn trip(&self, msg: String) -> Error {
        let _ = self.set_meta("tripped", &msg);
        Error::Tripped(msg)
    }

    /// Why writes are paused, or `None`. Eviction never shows up here.
    pub fn health(&self) -> Option<String> {
        if let Some(msg) = self.meta("tripped").ok().flatten() {
            return Some(msg);
        }
        let bytes = self.db_bytes();
        (bytes > self.limits.ceiling).then(|| ceiling_message(bytes, &self.limits))
    }

    /// Clears a trip (and the growth baseline) after the user has looked; returns what was cleared.
    /// Then evicts if over budget, so a ceiling trip can clear.
    pub fn reset_trip(&self) -> Result<Option<String>> {
        let was = self.meta("tripped")?;
        self.conn.execute("DELETE FROM trace_meta WHERE key IN ('tripped', 'growth_samples')", [])?;
        self.evict_if_over()?;
        Ok(was)
    }

    fn pragma_i64(&self, name: &str) -> rusqlite::Result<i64> {
        self.conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
    }

    /// Bytes of live pages (excludes the freelist, which `incremental_vacuum` returns to the OS).
    fn used_bytes(&self) -> rusqlite::Result<u64> {
        let pages = self.pragma_i64("page_count")? - self.pragma_i64("freelist_count")?;
        Ok(pages.max(0) as u64 * self.pragma_i64("page_size")? as u64)
    }

    /// Over budget: deletes the oldest sessions (by `updated_at`: events, usage, receipts, file rows)
    /// until live pages are under `EVICT_TO_PERCENT` of it, then shrinks the file. Records the newest
    /// evicted file mtime as `evict_floor_ms`: files not yet in the index at or below it are not re-added
    /// (see `index_file`), so a scan does not undo this. A file that changes later has a newer mtime and
    /// comes back. Returns sessions evicted.
    pub fn evict_if_over(&self) -> Result<usize> {
        if self.db_bytes() <= self.limits.budget {
            return Ok(0);
        }
        let target = self.limits.budget / 100 * EVICT_TO_PERCENT;
        let before = self.used_bytes()?;
        let mut evicted = 0;
        while self.used_bytes()? > target {
            let victims: Vec<(i64, String)> = {
                let mut stmt =
                    self.conn.prepare("SELECT sk, id FROM trace_sessions ORDER BY COALESCE(updated_at, 0), sk LIMIT ?1")?;
                stmt.query_map([EVICT_BATCH], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
            };
            if victims.is_empty() {
                break;
            }
            let tx = self.conn.unchecked_transaction()?;
            for (sk, id) in &victims {
                // Live pages count uncommitted deletes, so the batch stops as soon as it is enough.
                if self.used_bytes()? <= target {
                    break;
                }
                evicted += 1;
                let floor: Option<i64> =
                    tx.query_row("SELECT MAX(mtime_ms) FROM trace_files WHERE session_id = ?1", [id], |r| r.get(0))?;
                if let Some(floor) = floor {
                    tx.execute(
                        "INSERT INTO trace_meta (key, value) VALUES ('evict_floor_ms', ?1)
                         ON CONFLICT(key) DO UPDATE SET value = MAX(CAST(value AS INTEGER), CAST(?1 AS INTEGER))",
                        [floor],
                    )?;
                }
                delete_events(&tx, *sk)?;
                tx.execute("DELETE FROM trace_sessions WHERE sk = ?1", [sk])?;
                tx.execute("DELETE FROM trace_files WHERE session_id = ?1", [id])?;
                tx.execute("DELETE FROM trace_receipts WHERE session_id = ?1", [id])?;
            }
            tx.commit()?;
        }
        self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        // Moves the vacuum's pages out of the wal and truncates it; a no-op in rollback-journal mode.
        let _ = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        let freed = before.saturating_sub(self.used_bytes()?);
        let total = self.meta("evicted_bytes")?.and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + freed;
        self.set_meta("evicted_bytes", &total.to_string())?;
        Ok(evicted)
    }

    /// Files first seen at or below this mtime were evicted; they stay out.
    fn evict_floor(&self) -> i64 {
        self.meta("evict_floor_ms").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Gross index growth: size on disk plus everything evicted so far, so a loop that is evicted as
    /// fast as it writes still shows.
    fn gross_bytes(&self) -> u64 {
        self.db_bytes() + self.meta("evicted_bytes").ok().flatten().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
    }

    fn samples(&self) -> Vec<(i64, u64)> {
        let raw = self.meta("growth_samples").ok().flatten().unwrap_or_default();
        raw.split(',')
            .filter_map(|p| {
                let (t, b) = p.split_once(':')?;
                Some((t.parse().ok()?, b.parse().ok()?))
            })
            .collect()
    }

    /// Hook-side sampling: trips quietly into `health`. Nothing is sampled before the first full scan
    /// has finished (`built`): until then the db is still being filled, which is growth, not a loop.
    pub(crate) fn sample_growth(&self) -> Result<()> {
        if self.meta("built")?.is_none() {
            return Ok(());
        }
        self.check_growth(false).map(|_| ())
    }

    /// Compares gross size against the smallest sample inside the window and trips past
    /// `growth_bytes`. Otherwise records a sample (always when `force`, else at most once a minute).
    /// Returns the trip message when it tripped.
    fn check_growth(&self, force: bool) -> Result<Option<String>> {
        let now = now_ms();
        let gross = self.gross_bytes();
        let mut samples = self.samples();
        samples.retain(|(t, _)| *t >= now - self.limits.growth_window_ms);
        if let Some(&(t, low)) = samples.iter().min_by_key(|(_, b)| *b) {
            let grew = gross.saturating_sub(low);
            if grew > self.limits.growth_bytes {
                let msg = format!(
                    "tripped: growth rate — db grew {}MB in {} min (limit {}MB per {} min) — {TRIP_HELP}",
                    mb(grew),
                    ((now - t) / 60_000).max(1),
                    mb(self.limits.growth_bytes),
                    self.limits.growth_window_ms / 60_000
                );
                self.trip(msg.clone());
                return Ok(Some(msg));
            }
        }
        if force || samples.last().is_none_or(|(t, _)| now - t >= MIN_SAMPLE_GAP_MS) {
            samples.push((now, gross));
            let skip = samples.len().saturating_sub(MAX_SAMPLES);
            let text: Vec<String> = samples[skip..].iter().map(|(t, b)| format!("{t}:{b}")).collect();
            self.set_meta("growth_samples", &text.join(","))?;
        }
        Ok(None)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Discovers and indexes every root, pruning vanished files and roots.
    pub fn scan(&mut self, roots: &[Root]) -> Result<ScanReport> {
        self.scan_with_limit(roots, MAX_FILES)
    }

    pub fn scan_with_limit(&mut self, roots: &[Root], max_files: usize) -> Result<ScanReport> {
        self.grok_spawns = Some(
            roots.iter().filter(|r| r.source == "grok").map(|r| (r.path.clone(), grok_spawn_index(&r.path))).collect(),
        );
        let result = self.scan_with_limit_inner(roots, max_files);
        self.grok_spawns = None;
        result
    }

    fn scan_with_limit_inner(&mut self, roots: &[Root], max_files: usize) -> Result<ScanReport> {
        self.check_writable()?;
        self.capped_files = 0;
        // Until a full scan has finished once (`built`), growth is the db being filled, not a runaway.
        let built = self.meta("built")?.is_some();
        let mut report = ScanReport::default();
        // Discover every root first, then index newest-first across all of them: if the
        // cap is ever hit, what is missing is the oldest.
        let mut plans: Vec<RootPlan> = Vec::with_capacity(roots.len());
        let mut work: Vec<(i64, usize, Work)> = Vec::new();
        for (i, root) in roots.iter().enumerate() {
            let plan = if root.format == Format::Sqlite {
                self.plan_bb_root(root, i, &mut work)
            } else {
                let d = discover(root, max_files);
                for file in &d.files {
                    let mtime = fs::metadata(file).map_or(0, |m| mtime_ms(&m));
                    work.push((mtime, i, Work::File(file.clone())));
                }
                let error = if !d.accessible {
                    Some("Root is missing or not readable")
                } else if d.limited {
                    Some("Discovery limit reached; cached rows were preserved")
                } else if !d.complete {
                    Some("Discovery incomplete; cached rows were preserved")
                } else {
                    None
                };
                let seen = d.complete.then(|| d.files.iter().map(|p| p.to_string_lossy().into_owned()).collect());
                RootPlan { files: d.files.len(), accessible: d.accessible, limited: d.limited, error: error.map(String::from), seen, bb: None }
            };
            plans.push(plan);
        }
        work.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, i, item) in &work {
            let root = &roots[*i];
            let (key, done) = match item {
                Work::File(file) => (file.clone(), self.index_file(root, file)),
                Work::Bb(t) => {
                    let done = match &plans[*i].bb {
                        Some(bb) => self.index_bb_thread(root, bb, t),
                        None => continue,
                    };
                    (PathBuf::from(thread_key(&root.path, &t.id)), done)
                }
            };
            match done {
                Ok(true) => report.indexed += 1,
                Ok(false) => {}
                Err(e @ Error::Tripped(_)) => return Err(e),
                Err(e) => report.errors.push((key, e.to_string())),
            }
        }
        for (root, plan) in roots.iter().zip(plans) {
            self.conn.execute(
                "INSERT INTO trace_roots (id, source_id, path, exists_flag, file_count, last_scan_at, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET source_id=?2, path=?3, exists_flag=?4, file_count=?5, last_scan_at=?6, error=?7",
                params![root.id, root.source, root.path.to_string_lossy(), plan.accessible, plan.files, now_ms(), plan.error],
            )?;
            if let Some(seen) = plan.seen {
                self.prune_files(&root.id, |p| !seen.contains(p))?;
            }
            report.roots.push(RootReport { id: root.id.clone(), files: plan.files, accessible: plan.accessible, limited: plan.limited });
        }
        let ids: HashSet<&str> = roots.iter().map(|r| r.id.as_str()).collect();
        let stale: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT id FROM trace_roots")?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter(|id| !ids.contains(id.as_str()))
                .collect()
        };
        for id in stale {
            self.prune_files(&id, |_| true)?;
            self.conn.execute("DELETE FROM trace_roots WHERE id = ?1", [&id])?;
        }
        // Over budget is routine: evict quietly. Only the tripwires are loud.
        self.evict_if_over()?;
        if self.capped_files > self.limits.max_capped_files {
            let msg = format!(
                "tripped: event flood — {} files hit the {}-event cap in one scan (limit {}) — {TRIP_HELP}",
                self.capped_files, self.limits.max_events, self.limits.max_capped_files
            );
            report.tripped = Some(self.trip(msg).to_string());
        } else if built {
            report.tripped = self.check_growth(true)?;
        } else if !report.roots.iter().any(|r| r.limited) {
            // First complete scan: its size is the baseline. Samples start here.
            self.set_meta("growth_samples", "")?;
            self.set_meta("built", "1")?;
            self.check_growth(true)?;
        }
        Ok(report)
    }

    fn prune_files(&mut self, root_id: &str, gone: impl Fn(&str) -> bool) -> Result<()> {
        let rows: Vec<(String, String)> = {
            let mut stmt = self.conn.prepare("SELECT path, session_id FROM trace_files WHERE root_id = ?1")?;
            stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect()
        };
        let tx = self.conn.transaction()?;
        for (path, sid) in rows.iter().filter(|(p, _)| gone(p)) {
            let sk: Option<i64> = tx.query_row("SELECT sk FROM trace_sessions WHERE id = ?1", [sid], |r| r.get(0)).optional()?;
            if let Some(sk) = sk {
                delete_events(&tx, sk)?;
            }
            tx.execute("DELETE FROM trace_sessions WHERE id = ?1", [sid])?;
            tx.execute("DELETE FROM trace_files WHERE path = ?1", [path])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Indexes one file. Returns whether anything was (re)parsed.
    pub fn index_file(&mut self, root: &Root, path: &Path) -> Result<bool> {
        self.check_writable()?;
        self.guard(root, path)?;
        let meta = fs::metadata(path)?;
        let (size, mtime) = (meta.len() as i64, mtime_ms(&meta));
        let path_str = path.to_string_lossy().into_owned();
        let sid = session_id(&root.source, &path_str);
        let existing: Option<FileRow> = self
            .conn
            .query_row(
                "SELECT size_bytes, mtime_ms, indexed_bytes, indexed_lines, parser_version, content_hash, aux_sig
                 FROM trace_files WHERE path = ?1",
                [&path_str],
                |r| {
                    Ok(FileRow {
                        size_bytes: r.get(0)?,
                        mtime_ms: r.get(1)?,
                        indexed_bytes: r.get(2)?,
                        indexed_lines: r.get(3)?,
                        parser_version: r.get(4)?,
                        content_hash: r.get(5)?,
                        aux_sig: r.get(6)?,
                    })
                },
            )
            .optional()?;
        if existing.is_none() && mtime <= self.evict_floor() {
            return Ok(false);
        }
        let (mut links, aux) = path_links(root, path);
        // A sidecar (Claude's `.meta.json`, grok's `subagents/*/meta.json`) can change while the transcript does not.
        let aux_sig = grok_aux_sig(root, path, aux.as_deref().and_then(file_sig));
        let aux_changed = existing.as_ref().is_some_and(|f| f.aux_sig != aux_sig);

        let tail_ok = |f: &FileRow| -> io::Result<bool> {
            Ok(f.indexed_bytes <= size && f.content_hash.as_deref() == Some(tail_hash(path, f.indexed_bytes as u64)?.as_str()))
        };
        let reset = match &existing {
            None => true,
            Some(f) if f.parser_version != PARSER_VERSION => true,
            Some(f) => {
                let complete = f.indexed_bytes >= f.size_bytes;
                if f.size_bytes == size && (complete || root.format == Format::Zstd) {
                    if f.mtime_ms == mtime {
                        return self.refresh_aux(root, aux_changed, &sid, &path_str, &links, &aux_sig);
                    }
                    if tail_ok(f)? {
                        self.conn.execute("UPDATE trace_files SET mtime_ms = ?1 WHERE path = ?2", params![mtime, path_str])?;
                        return self.refresh_aux(root, aux_changed, &sid, &path_str, &links, &aux_sig);
                    }
                    true
                } else {
                    root.format == Format::Zstd || f.size_bytes > size || !tail_ok(f)?
                }
            }
        };
        let (start_byte, start_line) = match (&existing, reset) {
            (Some(f), false) => (f.indexed_bytes, f.indexed_lines),
            _ => (0, 0),
        };

        let basename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut agg = if reset { Aggregate { title: basename.clone(), ..Default::default() } } else { self.load_aggregate(&sid, &basename)? };

        let tx = self.conn.transaction()?;
        let sk = upsert_session(&tx, &sid, &root.source, &basename, &path_str, size)?;
        if reset {
            delete_events(&tx, sk)?;
        }
        let mut names = Names::default();
        let mut parse_error: Option<String> = None;
        let mut capped = false;
        let max_events = self.limits.max_events;
        let (end_byte, next_line) = {
            let mut insert = tx.prepare_cached(INSERT_EVENT)?;
            let mut usage = tx.prepare_cached(INSERT_USAGE)?;
            let reader = lines::open(path, root.format, start_byte as u64, size as u64)?;
            lines::for_each(reader, start_byte, start_line, |line| {
                if line.number >= max_events as i64 {
                    capped = true;
                    return Ok(false);
                }
                if line.text.trim().is_empty() {
                    return Ok(true);
                }
                let value: serde_json::Value = match serde_json::from_str(line.text) {
                    Ok(v) => v,
                    Err(e) => {
                        parse_error.get_or_insert_with(|| format!("line {}: {e}", line.number));
                        return Ok(true);
                    }
                };
                if line.number < HEADER_LINES {
                    header_links(root, &value, &mut links);
                }
                let e = normalize(&value);
                agg.add(&e);
                insert_event(&mut insert, &mut usage, &mut names, &tx, sk, line.number, line.start_byte, &e).map_err(io::Error::other)?;
                Ok(true)
            })?
        };
        let (end_byte, next_line) = if capped {
            parse_error = Some(format!("event cap {max_events} reached; rest of file not indexed"));
            self.capped_files += 1;
            (size, max_events as i64)
        } else {
            (end_byte, next_line)
        };
        let indexed_bytes = if root.format == Format::Zstd { size } else { end_byte };
        let content_hash = tail_hash(path, indexed_bytes as u64)?;
        if root.source == "grok"
            && let Some(summary) = grok_summary(path)
        {
            if agg.cwd.is_none() {
                agg.cwd = summary.pointer("/info/cwd").and_then(|v| v.as_str()).map(str::to_string);
            }
            grok_links(path, &summary, &mut links);
            let kind = summary.get("session_kind").and_then(|v| v.as_str()).unwrap_or("");
            if links.parent_native.is_none() && grok_dispatch_kind(kind) {
                let child = links.native_id.clone().or_else(|| path.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()));
                if let Some(id) = child
                    && let Some(spawn) = cached_spawn(&self.grok_spawns, &root.path, &id)
                {
                    apply_grok_spawn(&spawn, &mut links);
                }
            }
        }
        write_links(&tx, &sid, &links)?;
        if root.source == "grok" {
            stamp_grok_children(&tx, &root.source, path)?;
        }
        Self::write_aggregate(&tx, &sid, &agg, mtime)?;
        tx.execute(
            "INSERT INTO trace_files (path, root_id, source_id, format, size_bytes, mtime_ms, indexed_bytes, indexed_lines,
             parser_version, content_hash, session_id, indexed_at, parse_error, aux_sig)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(path) DO UPDATE SET root_id=?2, source_id=?3, format=?4, size_bytes=?5, mtime_ms=?6, indexed_bytes=?7,
             indexed_lines=?8, parser_version=?9, content_hash=?10, session_id=?11, indexed_at=?12, parse_error=?13, aux_sig=?14",
            params![
                path_str, root.id, root.source, root.format.as_str(), size, mtime, indexed_bytes, next_line,
                PARSER_VERSION, content_hash, sid, now_ms(), parse_error, aux_sig
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Only files of this root's format, inside it and outside the index's own directory.
    fn guard(&self, root: &Root, path: &Path) -> Result<()> {
        let own = self.db.parent().and_then(|d| d.canonicalize().ok());
        let inside_own = own.zip(path.canonicalize().ok()).is_some_and(|(dir, p)| p.starts_with(dir));
        if inside_own || resolve(path, std::slice::from_ref(root)).is_none() {
            return Err(Error::Rejected(path.to_path_buf()));
        }
        Ok(())
    }

    fn load_aggregate(&self, sid: &str, basename: &str) -> Result<Aggregate> {
        let agg = self.conn.query_row(
            "SELECT title, model, cwd, started_at, updated_at, event_count, user_count, assistant_count, tool_count,
             error_count, input_tokens, output_tokens, signal_count, now_text, last_event_at, failed, context_tokens, context_window FROM trace_sessions WHERE id = ?1",
            [sid],
            |r| {
                let title: String = r.get(0)?;
                Ok(Aggregate {
                    user_title: (title != basename).then(|| title.clone()),
                    title,
                    model: r.get(1)?,
                    cwd: r.get(2)?,
                    first_ts: r.get(3)?,
                    last_ts: r.get(4)?,
                    events: r.get(5)?,
                    users: r.get(6)?,
                    assistants: r.get(7)?,
                    tools: r.get(8)?,
                    errors: r.get(9)?,
                    input: r.get(10)?,
                    output: r.get(11)?,
                    signals: r.get(12)?,
                    now: r.get(13)?,
                    last_signal_ts: r.get(14)?,
                    failed: r.get(15)?,
                    context_tokens: r.get(16)?,
                    context_window: r.get(17)?,
                    ..Default::default()
                })
            },
        );
        match agg {
            Ok(a) => Ok(a),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Aggregate { title: basename.into(), ..Default::default() }),
            Err(e) => Err(e.into()),
        }
    }

    fn write_aggregate(tx: &Transaction<'_>, sid: &str, agg: &Aggregate, mtime: i64) -> Result<()> {
        let latest: Option<String> = tx
            .query_row(
                "SELECT n.name FROM trace_events e JOIN trace_names n ON n.id = e.type_id
                 WHERE e.sk = (SELECT sk FROM trace_sessions WHERE id = ?1) ORDER BY e.line_number DESC LIMIT 1",
                [sid],
                |r| r.get(0),
            )
            .optional()?;
        // `done`: 1 = the harness says it finished, 2 = finished with an error.
        let done: i64 = tx.query_row("SELECT done FROM trace_sessions WHERE id = ?1", [sid], |r| r.get(0))?;
        let status = match latest {
            _ if done > 0 => "completed",
            Some(t) if is_terminal(&t) => "completed",
            Some(_) => "open",
            None => "unknown",
        };
        let updated = agg.last_ts.unwrap_or(mtime);
        let duration = agg.first_ts.map(|s| (updated - s).max(0));
        tx.execute(
            "UPDATE trace_sessions SET title=?2, model=?3, cwd=?4, started_at=?5, updated_at=?6, event_count=?7, user_count=?8,
             assistant_count=?9, tool_count=?10, error_count=?11, input_tokens=?12, output_tokens=?13, duration_ms=?14, status=?15,
             signal_count=?16, now_text=?17, last_event_at=?18, failed=?19, context_tokens=?20, context_window=?21
             WHERE id = ?1",
            params![
                sid,
                agg.session_title.as_deref().or(agg.user_title.as_deref()).unwrap_or(&agg.title),
                agg.model,
                agg.cwd,
                agg.first_ts,
                updated,
                agg.events,
                agg.users,
                agg.assistants,
                agg.tools,
                agg.errors,
                agg.input,
                agg.output,
                duration,
                status,
                agg.signals,
                agg.now,
                agg.last_signal_ts,
                agg.failed || done == 2,
                agg.context_tokens,
                agg.context_window
            ],
        )?;
        // Receipts that arrived before this row existed take its key (same rule as ingest).
        let (source, native): (String, Option<String>) =
            tx.query_row("SELECT source_id, native_id FROM trace_sessions WHERE id = ?1", [sid], |r| Ok((r.get(0)?, r.get(1)?)))?;
        for key in crate::receipts::native_keys(&source, native.as_deref()) {
            tx.execute(
                "UPDATE trace_receipts SET session_id = ?1 WHERE session_id IS NULL AND source = ?2 AND native_session = ?3",
                params![sid, source, key],
            )?;
        }
        // The newest receipt's process; a receipt without a resolved pid and start never wipes it.
        let newest = |col: &str| {
            format!(
                "(SELECT r.{col} FROM trace_receipts r WHERE r.harness_pid IS NOT NULL AND r.harness_start IS NOT NULL AND r.session_id = trace_sessions.id
                  ORDER BY r.at_ms DESC, r.id DESC LIMIT 1)"
            )
        };
        tx.execute(
            &format!(
                "UPDATE trace_sessions SET harness_pid = COALESCE({}, harness_pid), harness_start = COALESCE({}, harness_start) WHERE id = ?1",
                newest("harness_pid"),
                newest("harness_start")
            ),
            [sid],
        )?;
        // `via` has its own newest-non-NULL-chain subquery: a receipt with a chain but no readable start still counts.
        let chain: Option<String> = tx
            .query_row(
                "SELECT r.harness_chain FROM trace_receipts r WHERE r.harness_chain IS NOT NULL AND r.session_id = ?1
                 ORDER BY r.at_ms DESC, r.id DESC LIMIT 1",
                [sid],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(chain) = chain {
            tx.execute(
                "UPDATE trace_sessions SET via = COALESCE(?2, via) WHERE id = ?1 AND (?3 OR via IS NULL)",
                params![sid, crate::live::via_of(&chain), chain.ends_with("launchd")],
            )?;
        }
        Ok(())
    }

    /// The transcript is unchanged; only its sidecar moved. Updates the links and the finished state.
    fn refresh_aux(&self, root: &Root, changed: bool, sid: &str, path: &str, links: &Links, sig: &Option<String>) -> Result<bool> {
        if !changed {
            return Ok(false);
        }
        let tx = self.conn.unchecked_transaction()?;
        write_links(&tx, sid, links)?;
        tx.execute(
            "UPDATE trace_sessions SET status = CASE WHEN done > 0 THEN 'completed' ELSE status END,
             failed = CASE WHEN done = 2 THEN 1 ELSE failed END WHERE id = ?1",
            [sid],
        )?;
        if root.source == "grok" {
            stamp_grok_children(&tx, &root.source, Path::new(path))?;
        }
        tx.execute("UPDATE trace_files SET aux_sig = ?1 WHERE path = ?2", params![sig, path])?;
        tx.commit()?;
        // Nothing was parsed.
        Ok(false)
    }
}

/// Child id → spawn. A scan passes the map it built up front; a hook (`None`) reads that one meta off disk.
fn cached_spawn(cache: &Option<HashMap<PathBuf, HashMap<String, GrokSpawn>>>, root: &Path, child: &str) -> Option<GrokSpawn> {
    match cache {
        Some(by_root) => by_root.get(root).and_then(|m| m.get(child)).cloned(),
        None => grok_spawn_for(root, child),
    }
}

fn grok_aux_sig(root: &Root, path: &Path, aux: Option<String>) -> Option<String> {
    if root.source != "grok" {
        return aux;
    }
    let extra = path.parent().and_then(subagents_sig);
    match (aux, extra) {
        (Some(a), Some(s)) => Some(format!("{a}|{s}")),
        (other, None) => other,
        (None, Some(s)) => Some(s),
    }
}

/// Point children named in this session's `subagents/*/meta.json` at it. Scan order does not matter:
/// a child indexed earlier is updated here; one indexed later reads the same meta itself.
fn stamp_grok_children(tx: &Transaction<'_>, source: &str, chat_history: &Path) -> rusqlite::Result<()> {
    let Some(dir) = chat_history.parent() else { return Ok(()) };
    for (child, spawn) in grok_session_spawns(dir) {
        let path: Option<String> = tx
            .query_row("SELECT file_path FROM trace_sessions WHERE source_id = ?1 AND native_id = ?2", params![source, child], |r| r.get(0))
            .optional()?;
        let Some(path) = path else { continue };
        let kind = grok_summary(Path::new(&path)).and_then(|v| v.get("session_kind").and_then(|k| k.as_str()).map(str::to_string));
        if !kind.as_deref().is_some_and(grok_dispatch_kind) {
            continue;
        }
        tx.execute(
            "UPDATE trace_sessions SET parent_native = COALESCE(parent_native, ?2),
             label = COALESCE(label, ?3),
             agent_type = CASE
               WHEN ?4 IS NULL THEN agent_type
               WHEN agent_type IS NULL OR agent_type = 'subagent' OR agent_type LIKE 'subagent%' THEN ?4
               ELSE agent_type
             END
             WHERE source_id = ?1 AND native_id = ?5",
            params![source, spawn.parent_native, spawn.label, spawn.agent_type, child],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn scan_indexes_newest_files_first_across_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let line = "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n";
        let mk = |root: &str, name: &str, age_s: u64| {
            let p = tmp.path().join(root).join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, line).unwrap();
            let t = SystemTime::now() - Duration::from_secs(age_s);
            fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        };
        mk("a", "old.jsonl", 3000);
        mk("a", "mid.jsonl", 2000);
        mk("b", "new.jsonl", 1000);
        mk("b", "oldest.jsonl", 4000);
        let root = |id: &str, source: &str| Root {
            id: id.into(),
            source: source.into(),
            path: tmp.path().join(id),
            format: Format::Jsonl,
            file_name: None,
        };
        let mut idx = TraceIndex::open(&tmp.path().join("idx/t.sqlite")).unwrap();
        idx.scan(&[root("a", "pi"), root("b", "omp")]).unwrap();
        let mut stmt = idx.conn().prepare("SELECT file_path FROM trace_sessions ORDER BY sk").unwrap();
        let order: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|p| Path::new(&p.unwrap()).file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(order, ["new.jsonl", "mid.jsonl", "old.jsonl", "oldest.jsonl"]);
    }

    #[test]
    fn terminal_events() {
        for e in ["task_complete", "turn_end", "turn/complete"] {
            assert!(!is_terminal(e), "{e}");
        }
        for e in ["session_end", "session/end", "agent_end", "session_shutdown", "SessionEnd", "run_end"] {
            assert!(is_terminal(e), "{e}");
        }
    }
}
