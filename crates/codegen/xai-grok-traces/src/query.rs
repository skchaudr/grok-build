//! Read side: ledger rows, trajectory events, facets, raw lines.

use std::path::{Path, PathBuf};

use rusqlite::types::Value as Sql;
use rusqlite::{OptionalExtension, Row, params};

use crate::index::{Result, TraceIndex, now_ms};
use crate::lines;
use crate::roots::Format;

/// A non-terminal session written to within this window is live.
pub const ACTIVE_WINDOW_MS: i64 = 5 * 60 * 1000;
/// A hook receipt this fresh, with a non-terminal event, makes a session live whatever its mtime.
pub const RECEIPT_WINDOW_MS: i64 = 2 * 60 * 1000;
/// Hook events that end a run (any harness).
pub const TERMINAL_EVENTS: [&str; 5] = ["SessionEnd", "Stop", "stop", "session_shutdown", "agent_end"];

const LAST_RECEIPT_AT: &str = "(SELECT r.at_ms FROM trace_receipts r WHERE r.session_id = s.id ORDER BY r.id DESC LIMIT 1)";
const LAST_RECEIPT_EVENT: &str = "(SELECT r.event FROM trace_receipts r WHERE r.session_id = s.id ORDER BY r.id DESC LIMIT 1)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
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
    /// Also match event titles and previews (a scan of trace_events).
    pub deep: bool,
    pub sort: Sort,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq)]
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
}

#[derive(Clone, Debug, Default)]
pub struct EventFilter {
    pub text: Option<String>,
    pub errors_only: bool,
    pub tool: Option<String>,
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

fn session_cols() -> String {
    format!("{SESSION_COLS}, {LAST_RECEIPT_AT}, {LAST_RECEIPT_EVENT}")
}

fn terminal_list() -> String {
    TERMINAL_EVENTS.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(",")
}

fn session_row(r: &Row<'_>, now: i64) -> rusqlite::Result<SessionRow> {
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
                    " OR s.id IN (SELECT session_id FROM trace_events WHERE preview LIKE ?1x ESCAPE '\\' OR title LIKE ?1x ESCAPE '\\')",
                );
            }
            sql.push(')');
            // Positional placeholders: one per occurrence.
            let pat = like(text);
            let n = sql.matches("?1x").count();
            sql = sql.replace("?1x", "?");
            args.extend(std::iter::repeat_n(Sql::Text(pat), n));
        }
        if filter.errors_only {
            sql.push_str(" AND s.error_count > 0");
        }
        if filter.has_tools {
            sql.push_str(" AND s.tool_count > 0");
        }
        if filter.active_only {
            sql.push_str(&format!(
                " AND ((s.status = 'open' AND f.mtime_ms > ?) OR ({LAST_RECEIPT_AT} > ? AND {LAST_RECEIPT_EVENT} NOT IN ({})))",
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

    pub fn session(&self, id: &str) -> Result<Option<SessionRow>> {
        let now = now_ms();
        let sql = format!("SELECT {} FROM trace_sessions s LEFT JOIN trace_files f ON f.path = s.file_path WHERE s.id = ?1", session_cols());
        Ok(self.conn().query_row(&sql, [id], |r| session_row(r, now)).optional()?)
    }

    pub fn list_events(&self, session_id: &str, filter: &EventFilter) -> Result<Vec<EventRow>> {
        let mut sql = String::from(
            "SELECT line_number, type, kind, role, title, preview, timestamp, duration_ms, input_tokens, output_tokens,
             turn, step, depth, model, is_error FROM trace_events WHERE session_id = ?",
        );
        let mut args = vec![Sql::Text(session_id.into())];
        if filter.errors_only {
            sql.push_str(" AND is_error = 1");
        }
        if let Some(tool) = &filter.tool {
            sql.push_str(" AND kind = 'tool' AND title = ?");
            args.push(Sql::Text(tool.clone()));
        }
        if let Some(text) = filter.text.as_deref().filter(|t| !t.trim().is_empty()) {
            sql.push_str(" AND (type LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\' OR preview LIKE ? ESCAPE '\\')");
            args.extend(std::iter::repeat_n(Sql::Text(like(text)), 3));
        }
        sql.push_str(" ORDER BY line_number LIMIT ? OFFSET ?");
        args.push(Sql::Integer(if filter.limit == 0 { 2000 } else { filter.limit as i64 }));
        args.push(Sql::Integer(filter.offset as i64));
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
            Ok(EventRow {
                line: r.get(0)?,
                event_type: r.get(1)?,
                kind: r.get(2)?,
                role: r.get(3)?,
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
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn facets(&self, session_id: &str) -> Result<Facets> {
        let (total, errors) = self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(is_error),0) FROM trace_events WHERE session_id = ?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut stmt = self.conn().prepare(
            "SELECT title, COUNT(*) c FROM trace_events WHERE session_id = ?1 AND kind = 'tool'
             GROUP BY title ORDER BY c DESC, title LIMIT 100",
        )?;
        let tools = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(Facets { total, errors, tools })
    }

    /// The source line behind an event, re-read from disk.
    pub fn raw_line(&self, session_id: &str, line: i64) -> Result<Option<String>> {
        let loc: Option<(String, String, i64)> = self
            .conn()
            .query_row(
                "SELECT f.path, f.format, e.byte_offset FROM trace_events e JOIN trace_sessions s ON s.id = e.session_id
                 JOIN trace_files f ON f.path = s.file_path WHERE e.session_id = ?1 AND e.line_number = ?2",
                params![session_id, line],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((path, format, offset)) = loc else { return Ok(None) };
        if format == "sqlite" {
            // bb: the pointer is `<db>#<thread id>` plus the event sequence (the line number).
            let Some((db, thread)) = crate::bb::split_key(&path) else { return Ok(None) };
            return crate::bb::Bb::open(Path::new(db))?.raw_event(thread, line);
        }
        let format = if format == "zstd" { Format::Zstd } else { Format::Jsonl };
        Ok(lines::read_line(path.as_ref(), format, line, offset)?)
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
