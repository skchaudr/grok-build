//! Hook receipts: one bounded row per harness event, so liveness does not
//! depend on file mtimes. The transcript itself is never copied.

use rusqlite::{OptionalExtension, params};

use crate::index::{Result, SAMPLE_EVERY, TraceIndex, now_ms};
use crate::live::{BOUNDARY_EVENTS, Boundary, CHILD_EVENTS, END_EVENTS, LiveReceipt, boundary, child_kind};

/// Rows kept once pruning runs (every 1000th insert).
const KEEP_RECEIPTS: i64 = 100_000;

/// Id prefix of a Cursor chat known only from receipts (no transcript indexed yet).
pub const RECEIPT_ONLY_PREFIX: &str = "cursor:rcpt:";

/// Receipts of session row `s`, as two branches so each can walk its own index in `(at_ms, id)` order:
/// keyed by id (`trace_receipts_session`), or unkeyed and matching its `native_id` in the same source
/// (the partial `trace_receipts_native`). Never write this as one OR with an ORDER BY.
pub(crate) const BRANCH_KEYED: &str = "r.session_id = s.id";
pub(crate) const BRANCH_NATIVE: &str = "r.session_id IS NULL AND r.native_session = s.native_id AND r.source = s.source_id";
/// Whether `s` has any receipt.
pub(crate) fn has_receipt_sql() -> String {
    format!("(EXISTS (SELECT 1 FROM trace_receipts r WHERE {BRANCH_KEYED}) OR EXISTS (SELECT 1 FROM trace_receipts r WHERE {BRANCH_NATIVE}))")
}

/// Scalar subselect: `col` of `s`'s newest receipt by `(at_ms, id)`, each branch cut to its newest first.
pub(crate) fn newest_receipt_sql(col: &str) -> String {
    let branch = |cond: &str| format!("SELECT r.at_ms AS at_ms, r.id AS id, r.event AS event FROM trace_receipts r WHERE {cond} ORDER BY r.at_ms DESC, r.id DESC LIMIT 1");
    format!(
        "(SELECT {col} FROM (SELECT * FROM ({}) UNION ALL SELECT * FROM ({})) ORDER BY at_ms DESC, id DESC LIMIT 1)",
        branch(BRANCH_KEYED),
        branch(BRANCH_NATIVE)
    )
}

/// The `native_session` values a hook can send for a session of `source` whose `native_id` is `native`:
/// Cursor sends it whole; Claude sends the last path component (`<project>/<uuid>`), Pi the part of it after `_`.
pub(crate) fn native_keys(source: &str, native: Option<&str>) -> Vec<String> {
    let Some(native) = native else { return Vec::new() };
    match source {
        "cursor" => vec![native.to_string()],
        "claude" | "pi" => {
            let last = native.rsplit('/').next().unwrap_or(native);
            let mut keys = vec![last.to_string()];
            if let Some((_, tail)) = last.rsplit_once('_') {
                keys.push(tail.to_string());
            }
            keys
        }
        _ => Vec::new(),
    }
}

pub(crate) fn in_list(names: &[&str]) -> String {
    names.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(",")
}

/// `(source, keyed session id, unkeyed native key)`.
type ReceiptKeys = (String, Option<String>, Option<String>);

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
        // A receipt whose start could not be read never overwrites a stored pid/start pair.
        if let (Some(sid), Some(pid), Some(_)) = (&r.session_id, r.harness_pid, r.harness_start) {
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

    /// The session of `source` for a receipt whose path did not resolve. `native_id` equal to `native` wins
    /// (Cursor); else its last path component is `native` (Claude `<project>/<uuid>`) or ends in `_<native>`
    /// (Pi `<project>/<timestamp>_<uuid>`). Newest `updated_at` among several.
    pub(crate) fn session_by_native(&self, source: &str, native: &str) -> Result<Option<String>> {
        let exact: Option<String> = self
            .conn()
            .query_row("SELECT id FROM trace_sessions WHERE source_id = ?1 AND native_id = ?2 LIMIT 1", [source, native], |r| r.get(0))
            .optional()?;
        if exact.is_some() || !matches!(source, "claude" | "pi") {
            return Ok(exact);
        }
        let esc = native.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let mut stmt = self.conn().prepare_cached(
            "SELECT id, native_id FROM trace_sessions WHERE source_id = ?1 AND native_id LIKE ?2 ESCAPE '\\'
             ORDER BY COALESCE(updated_at, 0) DESC, id",
        )?;
        let mut rows = stmt.query(params![source, format!("%{esc}")])?;
        while let Some(r) = rows.next()? {
            let (id, nid): (String, String) = (r.get(0)?, r.get(1)?);
            let last = nid.rsplit('/').next().unwrap_or(&nid);
            if last == native || last.strip_suffix(native).is_some_and(|p| p.ends_with('_')) {
                return Ok(Some(id));
            }
        }
        Ok(None)
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
        let sql = format!("SELECT {}, {} FROM trace_sessions s WHERE s.id = ?1", newest_receipt_sql("at_ms"), newest_receipt_sql("event"));
        let row: Option<(Option<i64>, Option<String>)> = self.conn().query_row(&sql, [session_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        Ok(row.and_then(|(at, event)| Some((at?, event?))))
    }

    /// Newest own-thread turn boundary of a session, as `(at_ms, id, kind)`. `id` is the session row id (keyed
    /// branch), `native` its receipt key (unkeyed branch); a receipt-only chat has only `native`.
    pub(crate) fn newest_boundary(&self, source: &str, id: Option<&str>, native: Option<&str>) -> Result<Option<(i64, i64, Boundary)>> {
        let conn = self.conn();
        let tail = format!("AND r.agent_id IS NULL AND r.event IN ({}) ORDER BY r.at_ms DESC, r.id DESC", in_list(BOUNDARY_EVENTS));
        let first = |sql: String, args: &[&str]| -> Result<Option<(i64, i64, Boundary)>> {
            let mut stmt = conn.prepare_cached(&sql)?;
            let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
            while let Some(r) = rows.next()? {
                let detail: Option<String> = r.get(3)?;
                let event: String = r.get(2)?;
                if let Some(b) = boundary(source, &event, detail.as_deref()) {
                    return Ok(Some((r.get(0)?, r.get(1)?, b)));
                }
            }
            Ok(None)
        };
        let keyed = match id {
            Some(id) => first(format!("SELECT r.at_ms, r.id, r.event, r.detail FROM trace_receipts r WHERE r.session_id = ?1 {tail}"), &[id])?,
            None => None,
        };
        let unkeyed = match native {
            Some(n) => first(
                format!("SELECT r.at_ms, r.id, r.event, r.detail FROM trace_receipts r WHERE r.session_id IS NULL AND r.source = ?1 AND r.native_session = ?2 {tail}"),
                &[source, n],
            )?,
            None => None,
        };
        Ok([keyed, unkeyed].into_iter().flatten().max_by_key(|(at, id, _)| (*at, *id)))
    }

    /// Both receipt branches of a session with an extra filter, merged by `(at_ms, id)`.
    fn branch_receipts(&self, id: Option<&str>, source: &str, native: Option<&str>, filter: &str) -> Result<Vec<LiveReceipt>> {
        let conn = self.conn();
        let mut out: Vec<LiveReceipt> = Vec::new();
        if let Some(id) = id {
            let sql = format!("SELECT {LIVE_COLS} FROM trace_receipts r WHERE r.session_id = ?1 {filter} ORDER BY r.at_ms, r.id");
            out.extend(conn.prepare_cached(&sql)?.query_map([id], live_receipt)?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        if let Some(n) = native {
            let sql = format!("SELECT {LIVE_COLS} FROM trace_receipts r WHERE r.session_id IS NULL AND r.source = ?1 AND r.native_session = ?2 {filter} ORDER BY r.at_ms, r.id");
            out.extend(conn.prepare_cached(&sql)?.query_map([source, n], live_receipt)?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        out.sort_by_key(|r| (r.at_ms, r.id));
        Ok(out)
    }

    /// `(source, session id for the keyed branch, receipt key for the unkeyed branch)` of a session id.
    /// `cursor:rcpt:<chat>` ids are receipt-only rows and match by `native_session`.
    fn receipt_keys(&self, session_id: &str) -> Result<Option<ReceiptKeys>> {
        if let Some(chat) = session_id.strip_prefix(RECEIPT_ONLY_PREFIX) {
            return Ok(Some(("cursor".into(), None, Some(chat.to_string()))));
        }
        let row: Option<(String, Option<String>)> = self
            .conn()
            .query_row("SELECT source_id, native_id FROM trace_sessions WHERE id = ?1", [session_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(row.map(|(source, native)| (source, Some(session_id.to_string()), native)))
    }

    /// Newest own-thread `End` of a session at any age, as a receipt (`(at_ms, id)` newest across both branches).
    fn newest_end(&self, source: &str, id: Option<&str>, native: Option<&str>) -> Result<Option<LiveReceipt>> {
        let conn = self.conn();
        let tail = format!("AND r.agent_id IS NULL AND r.event IN ({}) ORDER BY r.at_ms DESC, r.id DESC", in_list(END_EVENTS));
        let first = |sql: String, args: &[&str]| -> Result<Option<LiveReceipt>> {
            let mut stmt = conn.prepare_cached(&sql)?;
            let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
            while let Some(r) = rows.next()? {
                let rec = live_receipt(r)?;
                if boundary(source, &rec.event, rec.detail.as_deref()) == Some(Boundary::End) {
                    return Ok(Some(rec));
                }
            }
            Ok(None)
        };
        let keyed = match id {
            Some(id) => first(format!("SELECT {LIVE_COLS} FROM trace_receipts r WHERE r.session_id = ?1 {tail}"), &[id])?,
            None => None,
        };
        let unkeyed = match native {
            Some(n) => first(
                format!("SELECT {LIVE_COLS} FROM trace_receipts r WHERE r.session_id IS NULL AND r.source = ?1 AND r.native_session = ?2 {tail}"),
                &[source, n],
            )?,
            None => None,
        };
        Ok([keyed, unkeyed].into_iter().flatten().max_by_key(|r| (r.at_ms, r.id)))
    }

    /// Receipts since the session's newest turn boundary (the boundary itself included), or all of them
    /// when it has none, plus every subagent start/stop of the session at any age and the newest own-thread
    /// `End` at any age (the fold drops children started before it); by `(at_ms, id)`.
    pub fn turn_receipts(&self, session_id: &str) -> Result<Vec<LiveReceipt>> {
        let Some((source, id, native)) = self.receipt_keys(session_id)? else { return Ok(Vec::new()) };
        let (id, native) = (id.as_deref(), native.as_deref());
        let floor = self.newest_boundary(&source, id, native)?;
        let Some((at, bid, _)) = floor else {
            return self.branch_receipts(id, &source, native, "");
        };
        let mut out = self.branch_receipts(id, &source, native, &format!("AND (r.at_ms > {at} OR (r.at_ms = {at} AND r.id >= {bid}))"))?;
        let kids = self.branch_receipts(id, &source, native, &format!("AND r.event IN ({})", in_list(CHILD_EVENTS)))?;
        let mut have: std::collections::HashSet<i64> = out.iter().map(|r| r.id).collect();
        out.extend(kids.into_iter().filter(|r| child_kind(&source, &r.event).is_some() && have.insert(r.id)));
        if let Some(end) = self.newest_end(&source, id, native)?.filter(|e| have.insert(e.id)) {
            out.push(end);
        }
        out.sort_by_key(|r| (r.at_ms, r.id));
        Ok(out)
    }

    /// Every receipt of a session by `(at_ms, id)`; the replay helper folds prefixes of this.
    pub fn session_receipts(&self, session_id: &str) -> Result<Vec<LiveReceipt>> {
        let Some((source, id, native)) = self.receipt_keys(session_id)? else { return Ok(Vec::new()) };
        self.branch_receipts(id.as_deref(), &source, native.as_deref(), "")
    }

    /// `(at_ms, id)` of the session's newest receipt of any kind.
    pub fn last_receipt_key(&self, session_id: &str) -> Result<Option<(i64, i64)>> {
        let sql = format!("SELECT {}, {} FROM trace_sessions s WHERE s.id = ?1", newest_receipt_sql("at_ms"), newest_receipt_sql("id"));
        let row: Option<(Option<i64>, Option<i64>)> = self.conn().query_row(&sql, [session_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        Ok(row.and_then(|(at, id)| Some((at?, id?))))
    }

    /// Receipts at or after `ms`, for the empty state ("hooks are firing, nothing to show").
    pub fn receipt_count_since(&self, ms: i64) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM trace_receipts WHERE at_ms >= ?1", [ms], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan lines of `sql` joined; every ORDER BY here must come from an index.
    fn plan(idx: &TraceIndex, sql: &str) -> Vec<String> {
        let mut stmt = idx.conn().prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        stmt.query_map([], |r| r.get::<_, String>(3)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn receipt_lookups_walk_indexes_without_sorting() {
        let tmp = tempfile::tempdir().unwrap();
        let idx = TraceIndex::open(&tmp.path().join("t.sqlite")).unwrap();
        let queries = [
            ("last receipt of a session", format!("SELECT {}, {} FROM trace_sessions s WHERE s.id = 'x'", newest_receipt_sql("at_ms"), newest_receipt_sql("event"))),
            ("keyed boundary scan", "SELECT r.at_ms, r.id, r.event, r.detail FROM trace_receipts r WHERE r.session_id = 'x' AND r.agent_id IS NULL AND r.event IN ('Stop') ORDER BY r.at_ms DESC, r.id DESC".into()),
            ("unkeyed boundary scan", "SELECT r.at_ms, r.id, r.event, r.detail FROM trace_receipts r WHERE r.session_id IS NULL AND r.source = 'cursor' AND r.native_session = 'x' AND r.agent_id IS NULL AND r.event IN ('stop') ORDER BY r.at_ms DESC, r.id DESC".into()),
            ("keyed end scan", "SELECT r.id FROM trace_receipts r WHERE r.session_id = 'x' AND r.agent_id IS NULL AND r.event IN ('SessionEnd') ORDER BY r.at_ms DESC, r.id DESC".into()),
            ("unkeyed end scan", "SELECT r.id FROM trace_receipts r WHERE r.session_id IS NULL AND r.source = 'cursor' AND r.native_session = 'x' AND r.agent_id IS NULL AND r.event IN ('sessionEnd') ORDER BY r.at_ms DESC, r.id DESC".into()),
            ("last receipt key", format!("SELECT {}, {} FROM trace_sessions s WHERE s.id = 'x'", newest_receipt_sql("at_ms"), newest_receipt_sql("id"))),
            ("keyed turn receipts", "SELECT r.id FROM trace_receipts r WHERE r.session_id = 'x' AND r.at_ms > 5 ORDER BY r.at_ms, r.id".into()),
            ("unkeyed turn receipts", "SELECT r.id FROM trace_receipts r WHERE r.session_id IS NULL AND r.source = 'cursor' AND r.native_session = 'x' AND r.at_ms > 5 ORDER BY r.at_ms, r.id".into()),
            ("receipt-only groups", "SELECT r.native_session, MIN(r.at_ms), MAX(r.at_ms) FROM trace_receipts r WHERE r.source = 'cursor' AND r.session_id IS NULL AND r.native_session IS NOT NULL GROUP BY r.native_session".into()),
            ("newest receipt overall", "SELECT at_ms FROM trace_receipts ORDER BY at_ms DESC, id DESC LIMIT 1".into()),
            ("count since", "SELECT COUNT(*) FROM trace_receipts WHERE at_ms >= 5".into()),
        ];
        for (name, sql) in queries {
            let lines = plan(&idx, &sql);
            eprintln!("PLAN {name}:\n  {}", lines.join("\n  "));
            // Only the merge of the two branch heads (at most 2 rows) may sort, once per scalar subquery.
            let sorts = lines.iter().filter(|l| l.contains("TEMP B-TREE")).count();
            let merges = lines.iter().filter(|l| l.starts_with("CORRELATED SCALAR SUBQUERY")).count();
            assert_eq!(sorts, merges, "{name}: {lines:?}");
            assert!(!lines.iter().any(|l| l.starts_with("SCAN") && !l.contains("subquery") && !l.contains("INDEX")), "{name}: full scan {lines:?}");
        }
    }
}
