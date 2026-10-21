//! Hook receipts: one bounded row per harness event, so liveness does not
//! depend on file mtimes. The transcript itself is never copied.

use rusqlite::{OptionalExtension, params};

use crate::index::{Result, SAMPLE_EVERY, TraceIndex, now_ms};

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

    /// Highest receipt id, or 0 when there are none.
    pub fn receipts_watermark(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COALESCE(MAX(id), 0) FROM trace_receipts", [], |r| r.get(0))?)
    }

    /// Time of the newest receipt of any session.
    pub fn last_receipt_at(&self) -> Result<Option<i64>> {
        Ok(self.conn().query_row("SELECT at_ms FROM trace_receipts ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).optional()?)
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
