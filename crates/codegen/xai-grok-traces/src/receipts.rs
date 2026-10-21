//! Hook receipts: one bounded row per harness event, so liveness does not
//! depend on file mtimes. The transcript itself is never copied.

use rusqlite::{OptionalExtension, params};

use crate::index::{Result, SAMPLE_EVERY, TraceIndex, now_ms};
use crate::live::{BOUNDARY_EVENTS, CHILD_EVENTS, LiveReceipt, boundary, child_kind};

/// Rows kept once pruning runs (every 1000th insert).
const KEEP_RECEIPTS: i64 = 100_000;

/// Id prefix of a Cursor chat known only from receipts (no transcript indexed yet).
pub const RECEIPT_ONLY_PREFIX: &str = "cursor:rcpt:";

/// Receipts `r` that belong to session row `s`: keyed by id, or unkeyed and matching its `native_id` in the same source.
pub(crate) const RECEIPT_OF: &str = "(r.session_id = s.id OR (r.session_id IS NULL AND r.native_session = s.native_id AND r.source = s.source_id))";

pub(crate) fn in_list(names: &[&str]) -> String {
    names.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(",")
}

const LIVE_COLS: &str = "r.id, r.session_id, r.native_session, r.agent_id, r.event, r.tool, r.detail, r.tool_use_id, r.at_ms";

fn live_receipt(r: &rusqlite::Row<'_>) -> rusqlite::Result<LiveReceipt> {
    Ok(LiveReceipt {
        id: r.get(0)?,
        session_id: r.get(1)?,
        native_session: r.get(2)?,
        agent_id: r.get(3)?,
        event: r.get(4)?,
        tool: r.get(5)?,
        detail: r.get(6)?,
        tool_use_id: r.get(7)?,
        at_ms: r.get(8)?,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    /// 0 = stamp with the current time on insert.
    pub at_ms: i64,
    pub source: String,
    pub event: String,
    pub native_session: Option<String>,
    pub path: Option<String>,
    /// Index session id, when `path` resolved to a trace source.
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub cwd: Option<String>,
    /// What the event acted on: a call's argument summary, a notification type, a SessionStart source.
    pub detail: Option<String>,
    pub tool_use_id: Option<String>,
    /// Set on events from inside a subagent.
    pub agent_id: Option<String>,
    /// The harness process (payload `harness_pid`, else the ancestry walk) and its start, Unix seconds.
    pub harness_pid: Option<i64>,
    pub harness_start: Option<i64>,
}

impl TraceIndex {
    pub fn record_receipt(&mut self, r: &Receipt) -> Result<i64> {
        self.check_writable()?;
        let at = if r.at_ms == 0 { now_ms() } else { r.at_ms };
        self.conn().execute(
            "INSERT INTO trace_receipts (at_ms, source, event, native_session, path, session_id, tool, cwd,
             detail, tool_use_id, agent_id, harness_pid, harness_start)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                at, r.source, r.event, r.native_session, r.path, r.session_id, r.tool, r.cwd, r.detail, r.tool_use_id, r.agent_id,
                r.harness_pid, r.harness_start
            ],
        )?;
        let id = self.conn().last_insert_rowid();
        // The row may not be re-indexed (unchanged file), so the receipt keeps the live process fresh itself.
        if let (Some(sid), Some(pid)) = (&r.session_id, r.harness_pid) {
            self.conn().execute(
                "UPDATE trace_sessions SET harness_pid = ?2, harness_start = ?3 WHERE id = ?1",
                params![sid, pid, r.harness_start],
            )?;
        }
        if id % SAMPLE_EVERY == 0 {
            // A trip is recorded in the db and refuses the next write; this receipt is already in.
            let _ = self.sample_growth();
        }
        if id % 1000 == 0 {
            self.conn().execute(
                "DELETE FROM trace_receipts WHERE id <= (SELECT MAX(id) FROM trace_receipts) - ?1",
                [KEEP_RECEIPTS],
            )?;
        }
        Ok(id)
    }

    /// The session of `source` whose `native_id` is `native`, for receipts whose path did not resolve.
    pub(crate) fn session_by_native(&self, source: &str, native: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT id FROM trace_sessions WHERE source_id = ?1 AND native_id = ?2 LIMIT 1", [source, native], |r| r.get(0))
            .optional()?)
    }

    /// Highest receipt id, or 0 when there are none.
    pub fn receipts_watermark(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COALESCE(MAX(id), 0) FROM trace_receipts", [], |r| r.get(0))?)
    }

    /// Time of the newest receipt of any session.
    pub fn last_receipt_at(&self) -> Result<Option<i64>> {
        Ok(self.conn().query_row("SELECT at_ms FROM trace_receipts ORDER BY at_ms DESC, id DESC LIMIT 1", [], |r| r.get(0)).optional()?)
    }

    /// (at_ms, event) of the newest receipt for a session.
    pub fn last_receipt(&self, session_id: &str) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT r.at_ms, r.event FROM trace_sessions s JOIN trace_receipts r ON {RECEIPT_OF}
                     WHERE s.id = ?1 ORDER BY r.at_ms DESC, r.id DESC LIMIT 1"
                ),
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Receipts since the session's newest turn boundary (the boundary itself included), or all of them
    /// when it has none, plus every subagent start/stop of the session at any age; by `(at_ms, id)`.
    /// `cursor:rcpt:<chat>` ids are receipt-only rows and match by `native_session`.
    pub fn turn_receipts(&self, session_id: &str) -> Result<Vec<LiveReceipt>> {
        let (source, key, from) = match session_id.strip_prefix(RECEIPT_ONLY_PREFIX) {
            Some(chat) => (
                "cursor".to_string(),
                chat,
                "FROM trace_receipts r WHERE r.source = 'cursor' AND r.session_id IS NULL AND r.native_session = ?1".to_string(),
            ),
            None => {
                let source: Option<String> =
                    self.conn().query_row("SELECT source_id FROM trace_sessions WHERE id = ?1", [session_id], |r| r.get(0)).optional()?;
                let Some(source) = source else { return Ok(Vec::new()) };
                (source, session_id, format!("FROM trace_sessions s JOIN trace_receipts r ON {RECEIPT_OF} WHERE s.id = ?1"))
            }
        };
        let conn = self.conn();
        // Newest boundary of the session's own thread (not a subagent's).
        let mut newest = conn.prepare_cached(&format!(
            "SELECT {LIVE_COLS} {from} AND r.agent_id IS NULL AND r.event IN ({}) ORDER BY r.at_ms DESC, r.id DESC",
            in_list(BOUNDARY_EVENTS)
        ))?;
        let mut rows = newest.query([key])?;
        let mut floor: Option<(i64, i64)> = None;
        while let Some(row) = rows.next()? {
            let r = live_receipt(row)?;
            if boundary(&source, &r.event, r.detail.as_deref()).is_some() {
                floor = Some((r.at_ms, r.id));
                break;
            }
        }
        drop(rows);
        let mut out: Vec<LiveReceipt> = match floor {
            Some((at, id)) => conn
                .prepare_cached(&format!("SELECT {LIVE_COLS} {from} AND (r.at_ms > ?2 OR (r.at_ms = ?2 AND r.id >= ?3)) ORDER BY r.at_ms, r.id"))?
                .query_map(params![key, at, id], live_receipt)?
                .collect::<rusqlite::Result<_>>()?,
            None => conn
                .prepare_cached(&format!("SELECT {LIVE_COLS} {from} ORDER BY r.at_ms, r.id"))?
                .query_map([key], live_receipt)?
                .collect::<rusqlite::Result<_>>()?,
        };
        if floor.is_some() {
            let kids: Vec<LiveReceipt> = conn
                .prepare_cached(&format!("SELECT {LIVE_COLS} {from} AND r.event IN ({}) ORDER BY r.at_ms, r.id", in_list(CHILD_EVENTS)))?
                .query_map([key], live_receipt)?
                .collect::<rusqlite::Result<_>>()?;
            let have: std::collections::HashSet<i64> = out.iter().map(|r| r.id).collect();
            out.extend(kids.into_iter().filter(|r| !have.contains(&r.id) && child_kind(&source, &r.event).is_some()));
            out.sort_by_key(|r| (r.at_ms, r.id));
        }
        Ok(out)
    }

    /// Receipts at or after `ms`, for the empty state ("hooks are firing, nothing to show").
    pub fn receipt_count_since(&self, ms: i64) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM trace_receipts WHERE at_ms >= ?1", [ms], |r| r.get(0))?)
    }
}
