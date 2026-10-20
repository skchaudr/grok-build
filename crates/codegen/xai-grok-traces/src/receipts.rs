//! Hook receipts: one bounded row per harness event, so liveness does not
//! depend on file mtimes. The transcript itself is never copied.

use rusqlite::{OptionalExtension, params};

use crate::index::{Result, TraceIndex, now_ms};

/// Rows kept once pruning runs (every 1000th insert).
const KEEP_RECEIPTS: i64 = 100_000;

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
}

impl TraceIndex {
    pub fn record_receipt(&mut self, r: &Receipt) -> Result<i64> {
        self.check_cap()?;
        let at = if r.at_ms == 0 { now_ms() } else { r.at_ms };
        self.conn().execute(
            "INSERT INTO trace_receipts (at_ms, source, event, native_session, path, session_id, tool, cwd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![at, r.source, r.event, r.native_session, r.path, r.session_id, r.tool, r.cwd],
        )?;
        let id = self.conn().last_insert_rowid();
        if id % 1000 == 0 {
            self.conn().execute(
                "DELETE FROM trace_receipts WHERE id <= (SELECT MAX(id) FROM trace_receipts) - ?1",
                [KEEP_RECEIPTS],
            )?;
        }
        Ok(id)
    }

    /// Highest receipt id, or 0 when there are none.
    pub fn receipts_watermark(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COALESCE(MAX(id), 0) FROM trace_receipts", [], |r| r.get(0))?)
    }

    /// (at_ms, event) of the newest receipt for a session.
    pub fn last_receipt(&self, session_id: &str) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT at_ms, event FROM trace_receipts WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}
