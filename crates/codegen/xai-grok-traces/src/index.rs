//! The disposable SQLite index. Source files are the truth; a corrupt index
//! is quarantined and rebuilt.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use xai_sqlite_journal::JournalMode;

use crate::lines;
use crate::normalize::{Kind, NormalizedEvent, Role, clip, normalize};
use crate::roots::{Format, MAX_FILES, Root, discover, resolve};

pub const PARSER_VERSION: i64 = 3;
/// Bump when the table layout changes; a mismatch rebuilds the (disposable) db.
pub const SCHEMA_VERSION: i64 = 3;
/// Writes pause once the db plus its `-wal` pass this.
pub const MAX_DB_BYTES: u64 = 256 * 1024 * 1024;
/// Events indexed per file; the rest of a runaway file is left to the source.
pub const MAX_EVENTS_PER_FILE: usize = 200_000;
const TITLE_CHARS: usize = 180;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS trace_roots (id TEXT PRIMARY KEY, source_id TEXT NOT NULL, path TEXT NOT NULL,
  exists_flag INTEGER NOT NULL DEFAULT 0, file_count INTEGER NOT NULL DEFAULT 0, last_scan_at INTEGER, error TEXT);
CREATE TABLE IF NOT EXISTS trace_files (path TEXT PRIMARY KEY, root_id TEXT NOT NULL, source_id TEXT NOT NULL,
  format TEXT NOT NULL, size_bytes INTEGER NOT NULL, mtime_ms INTEGER NOT NULL, indexed_bytes INTEGER NOT NULL DEFAULT 0,
  indexed_lines INTEGER NOT NULL DEFAULT 0, parser_version INTEGER NOT NULL DEFAULT 0, content_hash TEXT,
  session_id TEXT NOT NULL, indexed_at INTEGER NOT NULL, parse_error TEXT);
CREATE TABLE IF NOT EXISTS trace_sessions (id TEXT PRIMARY KEY, source_id TEXT NOT NULL, title TEXT NOT NULL,
  file_path TEXT NOT NULL UNIQUE, model TEXT, cwd TEXT, started_at INTEGER, updated_at INTEGER,
  event_count INTEGER NOT NULL DEFAULT 0, user_count INTEGER NOT NULL DEFAULT 0, assistant_count INTEGER NOT NULL DEFAULT 0,
  tool_count INTEGER NOT NULL DEFAULT 0, error_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER, output_tokens INTEGER,
  duration_ms INTEGER, status TEXT NOT NULL DEFAULT 'unknown', file_size_bytes INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS trace_events (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, line_number INTEGER NOT NULL,
  byte_offset INTEGER NOT NULL, type TEXT NOT NULL, kind TEXT NOT NULL, role TEXT, title TEXT NOT NULL, preview TEXT NOT NULL,
  timestamp INTEGER, duration_ms INTEGER, input_tokens INTEGER, output_tokens INTEGER, usage_is_total INTEGER NOT NULL DEFAULT 0,
  turn INTEGER, step INTEGER, depth INTEGER NOT NULL DEFAULT 0, model TEXT, cwd TEXT, is_error INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS trace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS trace_receipts (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, source TEXT NOT NULL,
  event TEXT NOT NULL, native_session TEXT, path TEXT, session_id TEXT, tool TEXT, cwd TEXT);
CREATE INDEX IF NOT EXISTS trace_receipts_session ON trace_receipts(session_id, id);
CREATE INDEX IF NOT EXISTS trace_sessions_updated ON trace_sessions(updated_at DESC);
CREATE INDEX IF NOT EXISTS trace_sessions_source ON trace_sessions(source_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS trace_events_session ON trace_events(session_id, line_number);
";

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(io::Error),
    /// The db is past `MAX_DB_BYTES`; writes are refused until it is rebuilt.
    OverCap { bytes: u64 },
    /// A path that is not a trace source of the given root.
    Rejected(PathBuf),
}

pub fn over_cap_message(bytes: u64) -> String {
    let mb = |b: u64| b / (1024 * 1024);
    format!("over cap: {}MB / {}MB — writes paused; run grok-traces doctor", mb(bytes), mb(MAX_DB_BYTES))
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "sqlite: {e}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::OverCap { bytes } => f.write_str(&over_cap_message(*bytes)),
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
}

struct FileRow {
    size_bytes: i64,
    mtime_ms: i64,
    indexed_bytes: i64,
    indexed_lines: i64,
    parser_version: i64,
    content_hash: Option<String>,
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
        // Injected context (`<user_info>`, `<system-reminder>`, `<command-name>`) is not a title.
        if self.user_title.is_none()
            && e.role == Some(Role::User)
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
        fold(&mut self.input, e.input_tokens);
        fold(&mut self.output, e.output_tokens);
    }
}

/// Last event type marks a finished run.
fn is_terminal(event_type: &str) -> bool {
    let squashed: String = event_type
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '/' | '_' | ' ' | '-'))
        .collect();
    ["complete", "turnend", "sessionend", "runend"].iter().any(|w| squashed.contains(w))
}

/// Grok's `chat_history.jsonl` carries no cwd; its sibling `summary.json` does.
fn grok_cwd(file: &Path) -> Option<String> {
    let text = fs::read_to_string(file.with_file_name("summary.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.pointer("/info/cwd")?.as_str().map(str::to_string)
}

pub struct TraceIndex {
    conn: Connection,
    /// The db file actually opened (per-host on network mounts).
    db: PathBuf,
}

impl TraceIndex {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, None)
    }

    /// `busy`: lock wait for short-lived writers (hooks); `None` keeps the shared default.
    pub fn open_with(path: &Path, busy: Option<Duration>) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mode = JournalMode::for_db_path(path);
        let db = mode.effective_db_path(path);
        // At most one quarantine or version rebuild, then a fresh file.
        for _ in 0..2 {
            match Self::try_open(mode, &db, busy) {
                Ok(Some(idx)) => return Ok(idx),
                Ok(None) => {
                    tracing::info!(?db, "traces: index schema changed, rebuilding");
                    retire(&db);
                }
                Err(e) if is_unusable(&e) => {
                    tracing::warn!(?db, %e, "traces: index unusable, rebuilding");
                    quarantine(&db);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(Self::try_open(mode, &db, busy)?.ok_or_else(|| io::Error::other("index rebuild did not stick"))?)
    }

    /// `None` when the db was written by another schema or parser version.
    fn try_open(mode: JournalMode, db: &Path, busy: Option<Duration>) -> rusqlite::Result<Option<Self>> {
        let conn = match busy {
            None => mode.open(db)?,
            Some(wait) => {
                let conn = Connection::open(db)?;
                conn.busy_timeout(wait)?;
                mode.apply(&conn)?;
                conn
            }
        };
        // No integrity scan here: it is O(db) and the pager opens on the UI thread.
        // A garbage file fails on the first query below.
        let has_meta = conn
            .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'trace_meta'", [], |_| Ok(()))
            .optional()?
            .is_some();
        let idx = Self { conn, db: db.to_path_buf() };
        if has_meta {
            let same = |key: &str, want: i64| idx.meta(key).ok().flatten().and_then(|v| v.parse::<i64>().ok()) == Some(want);
            if !same("schema_version", SCHEMA_VERSION) || !same("parser_version", PARSER_VERSION) {
                return Ok(None);
            }
        } else {
            idx.conn.execute_batch(&format!(
                "BEGIN; {SCHEMA}
                 INSERT INTO trace_meta (key, value) VALUES ('schema_version', '{SCHEMA_VERSION}'), ('parser_version', '{PARSER_VERSION}');
                 COMMIT;"
            ))?;
        }
        Ok(Some(idx))
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

    /// Refuses a write batch while the db is over `MAX_DB_BYTES`, recording why in
    /// `trace_meta.health`; clears the record once under the cap again.
    pub fn check_cap(&self) -> Result<()> {
        let bytes = self.db_bytes();
        if bytes > MAX_DB_BYTES {
            let msg = over_cap_message(bytes);
            if self.meta("health").ok().flatten().as_deref() != Some(msg.as_str()) {
                let _ = self.set_meta("health", &msg);
            }
            return Err(Error::OverCap { bytes });
        }
        if self.meta("health")?.is_some_and(|h| h != "ok") {
            self.set_meta("health", "ok")?;
        }
        Ok(())
    }

    /// Why the index is unhealthy, or `None` when it is fine.
    pub fn health(&self) -> Option<String> {
        let bytes = self.db_bytes();
        if bytes > MAX_DB_BYTES {
            return Some(over_cap_message(bytes));
        }
        self.meta("health").ok().flatten().filter(|h| h != "ok")
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Discovers and indexes every root, pruning vanished files and roots.
    pub fn scan(&mut self, roots: &[Root]) -> Result<ScanReport> {
        self.scan_with_limit(roots, MAX_FILES)
    }

    pub fn scan_with_limit(&mut self, roots: &[Root], max_files: usize) -> Result<ScanReport> {
        self.check_cap()?;
        let mut report = ScanReport::default();
        for root in roots {
            let d = discover(root, max_files);
            for file in &d.files {
                match self.index_file(root, file) {
                    Ok(true) => report.indexed += 1,
                    Ok(false) => {}
                    Err(e @ Error::OverCap { .. }) => return Err(e),
                    Err(e) => report.errors.push((file.clone(), e.to_string())),
                }
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
            self.conn.execute(
                "INSERT INTO trace_roots (id, source_id, path, exists_flag, file_count, last_scan_at, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET source_id=?2, path=?3, exists_flag=?4, file_count=?5, last_scan_at=?6, error=?7",
                params![root.id, root.source, root.path.to_string_lossy(), d.accessible, d.files.len(), now_ms(), error],
            )?;
            if d.complete {
                let seen: HashSet<String> = d.files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                self.prune_files(&root.id, |p| !seen.contains(p))?;
            }
            report.roots.push(RootReport { id: root.id.clone(), files: d.files.len(), accessible: d.accessible, limited: d.limited });
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
        Ok(report)
    }

    fn prune_files(&mut self, root_id: &str, gone: impl Fn(&str) -> bool) -> Result<()> {
        let rows: Vec<(String, String)> = {
            let mut stmt = self.conn.prepare("SELECT path, session_id FROM trace_files WHERE root_id = ?1")?;
            stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect()
        };
        let tx = self.conn.transaction()?;
        for (path, sid) in rows.iter().filter(|(p, _)| gone(p)) {
            tx.execute("DELETE FROM trace_events WHERE session_id = ?1", [sid])?;
            tx.execute("DELETE FROM trace_sessions WHERE id = ?1", [sid])?;
            tx.execute("DELETE FROM trace_files WHERE path = ?1", [path])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Indexes one file. Returns whether anything was (re)parsed.
    pub fn index_file(&mut self, root: &Root, path: &Path) -> Result<bool> {
        self.check_cap()?;
        self.guard(root, path)?;
        let meta = fs::metadata(path)?;
        let (size, mtime) = (meta.len() as i64, mtime_ms(&meta));
        let path_str = path.to_string_lossy().into_owned();
        let sid = session_id(&root.source, &path_str);
        let existing: Option<FileRow> = self
            .conn
            .query_row(
                "SELECT size_bytes, mtime_ms, indexed_bytes, indexed_lines, parser_version, content_hash
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
                    })
                },
            )
            .optional()?;

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
                        return Ok(false);
                    }
                    if tail_ok(f)? {
                        self.conn.execute("UPDATE trace_files SET mtime_ms = ?1 WHERE path = ?2", params![mtime, path_str])?;
                        return Ok(false);
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
        tx.execute(
            "INSERT INTO trace_sessions (id, source_id, title, file_path, file_size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET file_path = ?4, file_size_bytes = ?5",
            params![sid, root.source, basename, path_str, size],
        )?;
        if reset {
            tx.execute("DELETE FROM trace_events WHERE session_id = ?1", [&sid])?;
        }
        let mut parse_error: Option<String> = None;
        let mut capped = false;
        let (end_byte, next_line) = {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO trace_events (id, session_id, line_number, byte_offset, type, kind, role, title, preview,
                 timestamp, duration_ms, input_tokens, output_tokens, usage_is_total, turn, step, depth, model, cwd, is_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            )?;
            let reader = lines::open(path, root.format, start_byte as u64, size as u64)?;
            lines::for_each(reader, start_byte, start_line, |line| {
                if line.number >= MAX_EVENTS_PER_FILE as i64 {
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
                let e = normalize(&value);
                agg.add(&e);
                let round = |v: Option<f64>| v.map(|v| v.round() as i64);
                insert
                    .execute(params![
                        format!("{sid}:{}", line.number),
                        sid,
                        line.number,
                        line.start_byte,
                        e.event_type,
                        e.kind.as_str(),
                        e.role.map(Role::as_str),
                        e.title,
                        e.preview,
                        e.timestamp_ms,
                        round(e.duration_ms),
                        round(e.input_tokens),
                        round(e.output_tokens),
                        e.usage_is_total,
                        round(e.turn),
                        round(e.step),
                        e.depth,
                        e.model,
                        e.cwd,
                        e.is_error(),
                    ])
                    .map_err(io::Error::other)?;
                Ok(true)
            })?
        };
        let (end_byte, next_line) = if capped {
            parse_error = Some(format!("event cap {MAX_EVENTS_PER_FILE} reached; rest of file not indexed"));
            (size, MAX_EVENTS_PER_FILE as i64)
        } else {
            (end_byte, next_line)
        };
        let indexed_bytes = if root.format == Format::Zstd { size } else { end_byte };
        let content_hash = tail_hash(path, indexed_bytes as u64)?;
        if root.source == "grok" && agg.cwd.is_none() {
            agg.cwd = grok_cwd(path);
        }
        Self::write_aggregate(&tx, &sid, &agg, mtime)?;
        tx.execute(
            "INSERT INTO trace_files (path, root_id, source_id, format, size_bytes, mtime_ms, indexed_bytes, indexed_lines,
             parser_version, content_hash, session_id, indexed_at, parse_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(path) DO UPDATE SET root_id=?2, source_id=?3, format=?4, size_bytes=?5, mtime_ms=?6, indexed_bytes=?7,
             indexed_lines=?8, parser_version=?9, content_hash=?10, session_id=?11, indexed_at=?12, parse_error=?13",
            params![
                path_str, root.id, root.source, root.format.as_str(), size, mtime, indexed_bytes, next_line,
                PARSER_VERSION, content_hash, sid, now_ms(), parse_error
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
             error_count, input_tokens, output_tokens FROM trace_sessions WHERE id = ?1",
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
                "SELECT type FROM trace_events WHERE session_id = ?1 ORDER BY line_number DESC LIMIT 1",
                [sid],
                |r| r.get(0),
            )
            .optional()?;
        let status = match latest {
            Some(t) if is_terminal(&t) => "completed",
            Some(_) => "open",
            None => "unknown",
        };
        let updated = agg.last_ts.unwrap_or(mtime);
        let duration = agg.first_ts.map(|s| (updated - s).max(0));
        tx.execute(
            "UPDATE trace_sessions SET title=?2, model=?3, cwd=?4, started_at=?5, updated_at=?6, event_count=?7, user_count=?8,
             assistant_count=?9, tool_count=?10, error_count=?11, input_tokens=?12, output_tokens=?13, duration_ms=?14, status=?15
             WHERE id = ?1",
            params![
                sid,
                agg.user_title.as_deref().unwrap_or(&agg.title),
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
                status
            ],
        )?;
        Ok(())
    }
}
