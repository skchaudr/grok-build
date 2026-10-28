//! Read side: ledger rows, trajectory events, facets, raw lines.

use std::path::{Path, PathBuf};

use rusqlite::types::Value as Sql;
use rusqlite::{OptionalExtension, Row, params};

use crate::index::{Result, TraceIndex, now_ms};
use crate::lines;
use crate::live::Boundary;
use crate::receipts::{RECEIPT_ONLY_PREFIX, has_receipt_sql, in_list, newest_receipt_sql};
use crate::normalize::{Kind, Role};
use crate::roots::Format;

/// A non-terminal session written to within this window is live.
pub const ACTIVE_WINDOW_MS: i64 = 5 * 60 * 1000;
/// A hook receipt this fresh, with a non-terminal event, makes a session live whatever its mtime.
pub const RECEIPT_WINDOW_MS: i64 = 2 * 60 * 1000;
/// Hook events that end a run (any harness).
pub const TERMINAL_EVENTS: [&str; 5] = ["SessionEnd", "Stop", "stop", "session_shutdown", "agent_end"];

/// The dispatcher, resolved inside the source; a missing or evicted parent leaves a root.
const PARENT_ID: &str = "(SELECT p.id FROM trace_sessions p WHERE p.source_id = s.source_id AND p.native_id = s.parent_native LIMIT 1)";
const CHILDREN: &str = "(SELECT COUNT(*) FROM trace_sessions c WHERE c.source_id = s.source_id AND c.parent_native = s.native_id)";
static LAST_RECEIPT_AT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| newest_receipt_sql("at_ms"));
static LAST_RECEIPT_EVENT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| newest_receipt_sql("event"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Active,
    #[default]
    Idle,
    Completed,
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Idle => "idle",
            Status::Completed => "done",
            Status::Unknown => "-",
        }
    }

    pub fn parse(s: &str) -> Status {
        match s {
            "active" => Status::Active,
            "idle" => Status::Idle,
            "done" => Status::Completed,
            _ => Status::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Updated,
    Started,
    Events,
    Duration,
    Errors,
}

#[derive(Clone, Debug, Default)]
pub struct SessionFilter {
    pub source: Option<String>,
    pub text: Option<String>,
    pub errors_only: bool,
    pub active_only: bool,
    pub has_tools: bool,
    /// Only sessions nobody dispatched (no resolvable `parent_id`).
    pub roots_only: bool,
    /// Hide pings: fewer than 3 non-noise events and no tools.
    pub hide_trivial: bool,
    /// Only sessions updated at or after this time (ms).
    pub since_ms: Option<i64>,
    /// Also match event titles and previews (a scan of trace_events).
    pub deep: bool,
    pub sort: Sort,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionRow {
    pub id: String,
    pub source: String,
    pub title: String,
    pub file_path: PathBuf,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub event_count: i64,
    pub user_count: i64,
    pub assistant_count: i64,
    pub tool_count: i64,
    pub error_count: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: Status,
    /// The dispatching session, when known and indexed.
    pub parent_id: Option<String>,
    /// Short human label: the dispatch description, when the harness recorded one.
    pub label: Option<String>,
    pub agent_type: Option<String>,
    /// Last meaningful event: `Tool: arg` or the last assistant text, at most 100 chars.
    pub now: Option<String>,
    pub last_event_at: Option<i64>,
    /// Sessions dispatched by this one.
    pub children: u32,
    /// Ended in failure: a trailing error with nothing after it, or a finished-with-error sidecar.
    pub failed: bool,
    /// `None` = this machine. Remote rows (see `remote`) carry the host label and a `host:`-prefixed id.
    pub host: Option<String>,
    /// How the harness names this session; hook receipts without a path match it.
    pub native_id: Option<String>,
    /// Mtime of the transcript file, ms.
    pub mtime_ms: Option<i64>,
    /// The harness process of the newest receipt (start in Unix seconds).
    pub harness_pid: Option<i64>,
    pub harness_start: Option<i64>,
    /// Size of the last prompt, and the model's window when recorded.
    pub context_tokens: Option<i64>,
    pub context_window: Option<i64>,
    /// Where the harness was launched from, per `live::via_of`.
    pub via: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct EventFilter {
    pub text: Option<String>,
    pub errors_only: bool,
    pub tool: Option<String>,
    /// Include bookkeeping, telemetry and injected context (hidden by default).
    pub include_noise: bool,
    /// Only events after this line number (exclusive), for paging and following.
    pub since_line: Option<i64>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub line: i64,
    pub event_type: String,
    pub kind: String,
    pub role: Option<String>,
    pub title: String,
    pub preview: String,
    pub timestamp: Option<i64>,
    pub duration_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub turn: Option<i64>,
    pub step: Option<i64>,
    pub depth: i64,
    pub model: Option<String>,
    pub is_error: bool,
    pub noise: bool,
    /// Tool call: compact argument summary. Tool result: `exit N` or the first error line.
    pub detail: Option<String>,
    /// Tool call: why it was made, when the call says.
    pub intent: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facets {
    pub total: i64,
    pub errors: i64,
    /// Tool title → count, most used first.
    pub tools: Vec<(String, i64)>,
}

const SESSION_COLS: &str = "s.id, s.source_id, s.title, s.file_path, s.model, s.cwd, s.started_at, s.updated_at,
  s.event_count, s.user_count, s.assistant_count, s.tool_count, s.error_count, s.input_tokens, s.output_tokens,
  s.duration_ms, s.status, f.mtime_ms";

pub(crate) fn session_cols() -> String {
    format!("{SESSION_COLS}, {}, {}, {PARENT_ID}, s.label, s.agent_type, s.now_text, s.last_event_at, {CHILDREN}, s.failed,
  s.native_id, s.harness_pid, s.harness_start, s.context_tokens, s.context_window, s.via", *LAST_RECEIPT_AT, *LAST_RECEIPT_EVENT)
}

fn terminal_list() -> String {
    TERMINAL_EVENTS.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(",")
}

pub(crate) fn session_row(r: &Row<'_>, now: i64) -> rusqlite::Result<SessionRow> {
    let status: String = r.get(16)?;
    let mtime: Option<i64> = r.get(17)?;
    let receipt_at: Option<i64> = r.get(18)?;
    let receipt_event: Option<String> = r.get(19)?;
    let live_receipt = receipt_at.is_some_and(|at| now - at < RECEIPT_WINDOW_MS)
        && receipt_event.as_deref().is_some_and(|e| !TERMINAL_EVENTS.contains(&e));
    let status = match status.as_str() {
        _ if live_receipt => Status::Active,
        "completed" => Status::Completed,
        "open" if mtime.is_some_and(|m| now - m < ACTIVE_WINDOW_MS) => Status::Active,
        "open" => Status::Idle,
        _ => Status::Unknown,
    };
    Ok(SessionRow {
        id: r.get(0)?,
        source: r.get(1)?,
        title: r.get(2)?,
        file_path: PathBuf::from(r.get::<_, String>(3)?),
        model: r.get(4)?,
        cwd: r.get(5)?,
        started_at: r.get(6)?,
        updated_at: r.get(7)?,
        event_count: r.get(8)?,
        user_count: r.get(9)?,
        assistant_count: r.get(10)?,
        tool_count: r.get(11)?,
        error_count: r.get(12)?,
        input_tokens: r.get(13)?,
        output_tokens: r.get(14)?,
        duration_ms: r.get(15)?,
        status,
        parent_id: r.get(20)?,
        label: r.get(21)?,
        agent_type: r.get(22)?,
        now: r.get(23)?,
        last_event_at: r.get(24)?,
        children: r.get::<_, i64>(25)?.max(0) as u32,
        failed: r.get(26)?,
        host: None,
        native_id: r.get(27)?,
        mtime_ms: mtime,
        harness_pid: r.get(28)?,
        harness_start: r.get(29)?,
        context_tokens: r.get(30)?,
        context_window: r.get(31)?,
        via: r.get(32)?,
    })
}

fn like(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

impl TraceIndex {
    pub fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionRow>> {
        let now = now_ms();
        let mut sql = format!("SELECT {} FROM trace_sessions s LEFT JOIN trace_files f ON f.path = s.file_path WHERE 1=1", session_cols());
        let mut args: Vec<Sql> = Vec::new();
        if let Some(source) = &filter.source {
            sql.push_str(" AND s.source_id = ?");
            args.push(Sql::Text(source.clone()));
        }
        if let Some(text) = filter.text.as_deref().filter(|t| !t.trim().is_empty()) {
            sql.push_str(" AND (s.title LIKE ?1x ESCAPE '\\' OR s.cwd LIKE ?1x ESCAPE '\\' OR s.model LIKE ?1x ESCAPE '\\'");
            sql.push_str(" OR s.file_path LIKE ?1x ESCAPE '\\' OR s.id LIKE ?1x ESCAPE '\\'");
            if filter.deep {
                sql.push_str(
                    " OR s.sk IN (SELECT e.sk FROM trace_events e JOIN trace_names ti ON ti.id = e.title_id
                   WHERE e.preview LIKE ?1x ESCAPE '\\' OR e.detail LIKE ?1x ESCAPE '\\' OR ti.name LIKE ?1x ESCAPE '\\')",
                );
            }
            sql.push(')');
            // Positional placeholders: one per occurrence.
            let pat = like(text);
            let n = sql.matches("?1x").count();
            sql = sql.replace("?1x", "?");
            args.extend(std::iter::repeat_n(Sql::Text(pat), n));
        }
        if let Some(since) = filter.since_ms {
            sql.push_str(" AND COALESCE(s.updated_at,0) >= ?");
            args.push(Sql::Integer(since));
        }
        if filter.errors_only {
            sql.push_str(" AND s.error_count > 0");
        }
        if filter.has_tools {
            sql.push_str(" AND s.tool_count > 0");
        }
        if filter.roots_only {
            sql.push_str(&format!(" AND {PARENT_ID} IS NULL"));
        }
        if filter.hide_trivial {
            // A just-dispatched agent is trivial until it has done something;
            // keep live sessions visible so new work shows up immediately.
            sql.push_str(&format!(
                " AND (NOT (s.signal_count < 3 AND s.tool_count = 0) OR (s.status = 'open' AND f.mtime_ms > ?) OR ({} > ? AND {} NOT IN ({})))",
                *LAST_RECEIPT_AT,
                *LAST_RECEIPT_EVENT,
                terminal_list()
            ));
            args.push(Sql::Integer(now - ACTIVE_WINDOW_MS));
            args.push(Sql::Integer(now - RECEIPT_WINDOW_MS));
        }
        if filter.active_only {
            sql.push_str(&format!(
                " AND ((s.status = 'open' AND f.mtime_ms > ?) OR ({} > ? AND {} NOT IN ({})))",
                *LAST_RECEIPT_AT,
                *LAST_RECEIPT_EVENT,
                terminal_list()
            ));
            args.push(Sql::Integer(now - ACTIVE_WINDOW_MS));
            args.push(Sql::Integer(now - RECEIPT_WINDOW_MS));
        }
        sql.push_str(match filter.sort {
            Sort::Updated => " ORDER BY COALESCE(s.updated_at,0) DESC, s.file_path DESC",
            Sort::Started => " ORDER BY COALESCE(s.started_at,0) DESC, s.file_path DESC",
            Sort::Events => " ORDER BY s.event_count DESC, COALESCE(s.updated_at,0) DESC",
            Sort::Duration => " ORDER BY COALESCE(s.duration_ms,-1) DESC, COALESCE(s.updated_at,0) DESC",
            Sort::Errors => " ORDER BY s.error_count DESC, COALESCE(s.updated_at,0) DESC",
        });
        sql.push_str(" LIMIT ? OFFSET ?");
        args.push(Sql::Integer(if filter.limit == 0 { 200 } else { filter.limit as i64 }));
        args.push(Sql::Integer(filter.offset as i64));
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| session_row(r, now))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sessions of `claude`, `pi` and `cursor` with receipts whose newest turn boundary is not `End`,
    /// plus receipt-only Cursor rows (chats seen in hooks whose transcript is not indexed yet).
    /// Liveness of every row, receipt-only included, is the caller's `alive(harness_pid, harness_start)`;
    /// a row with a NULL pid stays until its newest boundary is `End`. Same-process supersede is the caller's too.
    pub fn live_sessions(&self) -> Result<Vec<SessionRow>> {
        const SOURCES: [&str; 3] = ["claude", "pi", "cursor"];
        let now = now_ms();
        let conn = self.conn();
        let sql = format!(
            "SELECT {} FROM trace_sessions s LEFT JOIN trace_files f ON f.path = s.file_path
             WHERE s.source_id IN ({}) AND {}",
            session_cols(),
            in_list(&SOURCES),
            has_receipt_sql()
        );
        let candidates: Vec<SessionRow> = conn.prepare_cached(&sql)?.query_map([], |r| session_row(r, now))?.collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for row in candidates {
            let ended = self.newest_boundary(&row.source, Some(&row.id), row.native_id.as_deref())?.is_some_and(|(_, _, b)| b == Boundary::End);
            if !ended {
                out.push(row);
            }
        }

        // Cursor chats with receipts but no session row: grouped by chat id, minus any chat a row now owns.
        // Only unkeyed receipts are read (partial index `trace_receipts_native`).
        let mut groups = conn.prepare_cached(
            "SELECT r.native_session, MIN(r.at_ms), MAX(r.at_ms) FROM trace_receipts r
             WHERE r.source = 'cursor' AND r.session_id IS NULL AND r.native_session IS NOT NULL
               AND r.native_session NOT IN (SELECT native_id FROM trace_sessions WHERE source_id = 'cursor' AND native_id IS NOT NULL)
             GROUP BY r.native_session",
        )?;
        let groups: Vec<(String, i64, i64)> = groups.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut cwd = conn.prepare_cached(
            "SELECT r.cwd FROM trace_receipts r WHERE r.source = 'cursor' AND r.session_id IS NULL AND r.native_session = ?1 AND r.cwd IS NOT NULL
             ORDER BY r.at_ms DESC, r.id DESC LIMIT 1",
        )?;
        let mut chain = conn.prepare_cached(
            "SELECT r.harness_chain FROM trace_receipts r
             WHERE r.source = 'cursor' AND r.session_id IS NULL AND r.native_session = ?1 AND r.harness_chain IS NOT NULL
             ORDER BY r.at_ms DESC, r.id DESC LIMIT 1",
        )?;
        let mut pid = conn.prepare_cached(
            "SELECT r.harness_pid, r.harness_start FROM trace_receipts r
             WHERE r.source = 'cursor' AND r.session_id IS NULL AND r.native_session = ?1 AND r.harness_pid IS NOT NULL
             ORDER BY r.at_ms DESC, r.id DESC LIMIT 1",
        )?;
        for (native, first, last) in groups {
            if self.newest_boundary("cursor", None, Some(&native))?.is_some_and(|(_, _, b)| b == Boundary::End) {
                continue;
            }
            let cwd: Option<String> = cwd.query_row([&native], |r| r.get(0)).optional()?;
            let (harness_pid, harness_start) = pid.query_row([&native], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.unwrap_or_default();
            let via = chain.query_row([&native], |r| r.get::<_, String>(0)).optional()?.and_then(|c| crate::live::via_of(&c));
            let title = cwd
                .as_deref()
                .and_then(|c| Path::new(c).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "cursor".into());
            out.push(SessionRow {
                id: format!("{RECEIPT_ONLY_PREFIX}{native}"),
                source: "cursor".into(),
                title,
                status: Status::Active,
                cwd,
                started_at: Some(first),
                updated_at: Some(last),
                native_id: Some(native),
                harness_pid,
                harness_start,
                via,
                ..Default::default()
            });
        }
        out.sort_by(|a, b| b.updated_at.unwrap_or(0).cmp(&a.updated_at.unwrap_or(0)).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRow>> {
        let now = now_ms();
        let sql = format!("SELECT {} FROM trace_sessions s LEFT JOIN trace_files f ON f.path = s.file_path WHERE s.id = ?1", session_cols());
        Ok(self.conn().query_row(&sql, [id], |r| session_row(r, now)).optional()?)
    }

    /// Sessions this one dispatched, oldest first.
    pub fn children(&self, session_id: &str) -> Result<Vec<SessionRow>> {
        let now = now_ms();
        let sql = format!(
            "SELECT {} FROM trace_sessions s LEFT JOIN trace_files f ON f.path = s.file_path
             WHERE s.parent_native IS NOT NULL AND s.parent_native = (SELECT native_id FROM trace_sessions WHERE id = ?1)
               AND s.source_id = (SELECT source_id FROM trace_sessions WHERE id = ?1)
             ORDER BY COALESCE(s.started_at, s.updated_at, 0), s.file_path",
            session_cols()
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map([session_id], |r| session_row(r, now))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn list_events(&self, session_id: &str, filter: &EventFilter) -> Result<Vec<EventRow>> {
        let mut sql = String::from(
            "SELECT e.line_number, ty.name, e.kind, e.role, ti.name, CASE WHEN e.preview = '' THEN ti.name ELSE e.preview END,
             e.timestamp, u.duration_ms, u.input_tokens, u.output_tokens, u.turn, u.step, e.depth, mo.name, e.is_error,
             e.noise, e.detail, e.intent
             FROM trace_events e JOIN trace_names ty ON ty.id = e.type_id JOIN trace_names ti ON ti.id = e.title_id
             LEFT JOIN trace_names mo ON mo.id = e.model_id
             LEFT JOIN trace_event_usage u ON u.sk = e.sk AND u.line_number = e.line_number
             WHERE e.sk = (SELECT sk FROM trace_sessions WHERE id = ?)",
        );
        let mut args = vec![Sql::Text(session_id.into())];
        if filter.errors_only {
            sql.push_str(" AND e.is_error = 1");
        }
        if !filter.include_noise {
            sql.push_str(" AND e.noise = 0");
        }
        if let Some(line) = filter.since_line {
            sql.push_str(" AND e.line_number > ?");
            args.push(Sql::Integer(line));
        }
        if let Some(tool) = &filter.tool {
            sql.push_str(&format!(" AND e.kind = {} AND ti.name = ?", Kind::Tool.code()));
            args.push(Sql::Text(tool.clone()));
        }
        if let Some(text) = filter.text.as_deref().filter(|t| !t.trim().is_empty()) {
            sql.push_str(" AND (ty.name LIKE ? ESCAPE '\\' OR ti.name LIKE ? ESCAPE '\\' OR e.preview LIKE ? ESCAPE '\\' OR e.detail LIKE ? ESCAPE '\\')");
            args.extend(std::iter::repeat_n(Sql::Text(like(text)), 4));
        }
        sql.push_str(" ORDER BY e.line_number LIMIT ? OFFSET ?");
        args.push(Sql::Integer(if filter.limit == 0 { 2000 } else { filter.limit as i64 }));
        args.push(Sql::Integer(filter.offset as i64));
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
            Ok(EventRow {
                line: r.get(0)?,
                event_type: r.get(1)?,
                kind: Kind::from_code(r.get(2)?).map_or("message", Kind::as_str).to_string(),
                role: r.get::<_, Option<i64>>(3)?.and_then(Role::from_code).map(|r| r.as_str().to_string()),
                title: r.get(4)?,
                preview: r.get(5)?,
                timestamp: r.get(6)?,
                duration_ms: r.get(7)?,
                input_tokens: r.get(8)?,
                output_tokens: r.get(9)?,
                turn: r.get(10)?,
                step: r.get(11)?,
                depth: r.get(12)?,
                model: r.get(13)?,
                is_error: r.get(14)?,
                noise: r.get(15)?,
                detail: r.get(16)?,
                intent: r.get(17)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn facets(&self, session_id: &str) -> Result<Facets> {
        let (total, errors) = self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(is_error),0) FROM trace_events WHERE sk = (SELECT sk FROM trace_sessions WHERE id = ?1)",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut stmt = self.conn().prepare(&format!(
            "SELECT ti.name, COUNT(*) c FROM trace_events e JOIN trace_names ti ON ti.id = e.title_id
             WHERE e.sk = (SELECT sk FROM trace_sessions WHERE id = ?1) AND e.kind = {}
             GROUP BY ti.name ORDER BY c DESC, ti.name LIMIT 100",
            Kind::Tool.code()
        ))?;
        let tools = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(Facets { total, errors, tools })
    }

    /// The source line behind an event, re-read from disk.
    pub fn raw_line(&self, session_id: &str, line: i64) -> Result<Option<String>> {
        let loc: Option<(String, String, i64)> = self
            .conn()
            .query_row(
                "SELECT f.path, f.format, s.sk FROM trace_sessions s JOIN trace_events e ON e.sk = s.sk
                 JOIN trace_files f ON f.path = s.file_path WHERE s.id = ?1 AND e.line_number = ?2",
                params![session_id, line],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((path, format, sk)) = loc else { return Ok(None) };
        if format == "sqlite" {
            // bb: the pointer is `<db>#<thread id>` plus the event sequence (the line number).
            let Some((db, thread)) = crate::bb::split_key(&path) else { return Ok(None) };
            return crate::bb::Bb::open(Path::new(db))?.raw_event(thread, line);
        }
        let format = if format == "zstd" { Format::Zstd } else { Format::Jsonl };
        // Only every Nth line keeps its offset; scan on from the nearest one before it.
        let (from_line, from_byte): (i64, i64) = self
            .conn()
            .query_row(
                "SELECT line_number, byte_offset FROM trace_events WHERE sk = ?1 AND line_number <= ?2 AND byte_offset IS NOT NULL
                 ORDER BY line_number DESC LIMIT 1",
                params![sk, line],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        Ok(lines::read_line(path.as_ref(), format, line, from_line, from_byte)?)
    }

    /// Sources that have at least one session, for the ledger tabs.
    pub fn sources(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn().prepare("SELECT source_id, COUNT(*) FROM trace_sessions GROUP BY source_id")?;
        Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub sessions: i64,
    pub events: i64,
    pub schema_version: Option<String>,
    pub parser_version: Option<String>,
    /// (source, receipt count, last receipt at_ms), by source.
    pub receipts: Vec<(String, i64, i64)>,
}

impl TraceIndex {
    pub fn stats(&self) -> Result<Stats> {
        let count = |t: &str| self.conn().query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0));
        let mut stmt = self.conn().prepare("SELECT source, COUNT(*), MAX(at_ms) FROM trace_receipts GROUP BY source ORDER BY source")?;
        let receipts = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(Stats {
            sessions: count("trace_sessions")?,
            events: count("trace_events")?,
            schema_version: self.meta("schema_version")?,
            parser_version: self.meta("parser_version")?,
            receipts,
        })
    }
}
