//! bb threads, read in place from `~/.bb/bb.db` (read-only). Indexed by
//! `index/bb_index.rs`; see `docs/traces/BB.md`.
//!
//! The pointer to an event is `(thread_id, sequence)`: `sequence` is unique and
//! increasing per thread (`events_thread_sequence_idx`), so it plays the role a
//! byte offset plays for files. Ported from agent-obs `bb_thread.py` / `bb_events.py`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

use crate::index::Result;
use crate::normalize::{Kind, NormalizedEvent, Role, normalize};

/// Event types that carry no session content: streaming deltas, `item/started`
/// (the `item/completed` row has the same item plus `durationMs`), provider
/// noise and bookkeeping. Deltas are matched by `LIKE '%delta'`.
const SKIPPED_TYPES: [&str; 8] = [
    "provider/unhandled",
    "item/started",
    "provider.env-resolved",
    "system/thread-provisioning",
    "thread/extensionState/updated",
    "thread/contextWindowUsage/updated",
    "client/thread/start",
    "turn/input/accepted",
];
const TOOL_KINDS: [&str; 7] =
    ["toolCall", "commandExecution", "fileChange", "fileRead", "webFetch", "webSearch", "delegation"];

/// Lock wait when reading bb.db; bb is the writer, so give up fast.
const BUSY: Duration = Duration::from_millis(500);

/// The key a thread is indexed under: `<db path>#<thread id>` (unique, so it can be a session's `file_path`).
pub fn thread_key(db: &Path, thread_id: &str) -> String {
    format!("{}#{thread_id}", db.display())
}

/// Inverse of [`thread_key`].
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    key.rsplit_once('#')
}

pub fn default_db_path(home: &Path) -> PathBuf {
    home.join(".bb/bb.db")
}

#[derive(Clone, Debug, PartialEq)]
pub struct BbThread {
    pub id: String,
    pub title: Option<String>,
    pub provider_id: String,
    pub model: Option<String>,
    /// bb's own status: starting, active, idle, completed, error.
    pub status: String,
    /// The environment's path.
    pub cwd: Option<String>,
    pub parent_thread_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
    /// Highest event sequence (any type); `None` for a thread with no events.
    pub last_sequence: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BbEvent {
    pub thread_id: String,
    pub row_id: String,
    pub sequence: i64,
    pub event: NormalizedEvent,
}

pub struct Bb {
    conn: Connection,
}

impl Bb {
    /// Opens `path` read-only; bb keeps writing to it (WAL), so readers never block it.
    /// Callers open per scan and drop the handle; nothing holds bb.db between scans.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, BUSY)
    }

    pub fn open_with(path: &Path, busy: Duration) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(busy)?;
        Ok(Self { conn })
    }

    /// Non-deleted threads, most recently updated first.
    pub fn threads(&self) -> Result<Vec<BbThread>> {
        self.query_threads(None)
    }

    pub fn thread(&self, id: &str) -> Result<Option<BbThread>> {
        Ok(self.query_threads(Some(id))?.into_iter().next())
    }

    fn query_threads(&self, only: Option<&str>) -> Result<Vec<BbThread>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, COALESCE(t.title, t.title_fallback), t.provider_id, t.model_override, t.status, env.path,
                    t.parent_thread_id, t.created_at, t.updated_at, t.archived_at IS NOT NULL,
                    (SELECT MAX(sequence) FROM events WHERE thread_id = t.id)
             FROM threads t LEFT JOIN environments env ON env.id = t.environment_id
             WHERE t.deleted_at IS NULL AND (?1 IS NULL OR t.id = ?1) ORDER BY t.updated_at DESC",
        )?;
        let rows = stmt.query_map([only], |r| {
            Ok(BbThread {
                id: r.get(0)?,
                title: r.get::<_, Option<String>>(1)?.filter(|t| !t.trim().is_empty()),
                provider_id: r.get(2)?,
                model: r.get(3)?,
                status: r.get(4)?,
                cwd: r.get(5)?,
                parent_thread_id: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
                archived: r.get(9)?,
                last_sequence: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Up to `limit` indexable events of a thread with `sequence > after`, in order.
    /// Fewer than `limit` means the thread is drained; resume from the last `sequence`.
    pub fn events(&self, thread_id: &str, after: i64, limit: usize) -> Result<Vec<BbEvent>> {
        let skipped = SKIPPED_TYPES.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(",");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, sequence, type, item_kind, created_at, data FROM events
             WHERE thread_id = ?1 AND sequence > ?2 AND type NOT IN ({skipped}) AND type NOT LIKE '%delta'
             ORDER BY sequence LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![thread_id, after, limit as i64], |r| {
            let data: String = r.get(5)?;
            Ok(BbEvent {
                thread_id: thread_id.to_string(),
                row_id: r.get(0)?,
                sequence: r.get(1)?,
                event: map_event(
                    &r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?.as_deref(),
                    r.get(4)?,
                    &serde_json::from_str(&data).unwrap_or(Value::Null),
                ),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The full event row as one JSON line (the analogue of a transcript's raw line).
    pub fn raw_event(&self, thread_id: &str, sequence: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, type, item_kind, item_id, turn_id, parent_tool_call_id, created_at, data
                 FROM events WHERE thread_id = ?1 AND sequence = ?2",
                params![thread_id, sequence],
                |r| {
                    let data: String = r.get(7)?;
                    Ok(json!({
                        "id": r.get::<_, String>(0)?, "thread_id": thread_id, "sequence": sequence,
                        "type": r.get::<_, String>(1)?, "item_kind": r.get::<_, Option<String>>(2)?,
                        "item_id": r.get::<_, Option<String>>(3)?, "turn_id": r.get::<_, Option<String>>(4)?,
                        "parent_tool_call_id": r.get::<_, Option<String>>(5)?, "created_at": r.get::<_, i64>(6)?,
                        "data": serde_json::from_str::<Value>(&data).unwrap_or(Value::Null),
                    })
                    .to_string())
                },
            )
            .optional()?)
    }
}

/// Plain text of a string, a `{text}` object, or an array of either.
fn flat_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(flat_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(m) => m.get("text").map(flat_text).unwrap_or_default(),
        _ => String::new(),
    }
}

fn non_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(_) => true,
    }
}

fn item_text(kind: &str, item: &Value) -> String {
    let get = |k: &str| item.get(k).map(flat_text).unwrap_or_default();
    match kind {
        "agentMessage" => get("text"),
        "userMessage" => get("content"),
        "reasoning" => Some(get("summary")).filter(|s| !s.is_empty()).unwrap_or_else(|| get("content")),
        "toolCall" => match item.get("arguments") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        },
        "commandExecution" => get("command"),
        "fileRead" => get("path"),
        "webFetch" => get("url"),
        "webSearch" => get("queries"),
        "delegation" => get("label"),
        "fileChange" => item
            .get("changes")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(|c| c.get("path").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn tool_name<'a>(kind: &'a str, item: &'a Value) -> &'a str {
    match kind {
        "toolCall" => item.get("tool").and_then(Value::as_str).unwrap_or("tool"),
        "commandExecution" => "bash",
        "fileChange" => "edit",
        "fileRead" => "read",
        other => other,
    }
}

/// One bb event row → the shared [`NormalizedEvent`]. `item/completed` rows are
/// typed `item/<kind>` so the "last event is terminal" status check does not
/// mistake a finished tool call for a finished turn.
pub fn map_event(ty: &str, item_kind: Option<&str>, created_at: i64, data: &Value) -> NormalizedEvent {
    let item = data.get("item").unwrap_or(&Value::Null);
    let kind = item_kind.unwrap_or("");
    let event_type = match (ty, item_kind) {
        ("item/completed", Some(k)) => format!("item/{k}"),
        _ => ty.to_string(),
    };
    let (text, name) = match ty {
        "client/turn/requested" => (flat_text(data.get("input").unwrap_or(&Value::Null)), String::new()),
        "item/completed" => (item_text(kind, item), tool_name(kind, item).to_string()),
        "provider/error" | "system/error" | "provider/warning" => {
            (data.get("message").map(flat_text).unwrap_or_default(), String::new())
        }
        "turn/completed" => (data.get("status").map(flat_text).unwrap_or_default(), String::new()),
        _ => (String::new(), String::new()),
    };
    let mut e = normalize(&json!({
        "type": event_type, "timestamp": created_at, "data": {"text": text, "name": name},
    }));

    let (k, role, depth, title) = match (ty, kind) {
        ("client/turn/requested", _) | ("item/completed", "userMessage") => (Kind::Message, Some(Role::User), 0, "User"),
        ("item/completed", "agentMessage") => (Kind::Message, Some(Role::Assistant), 0, "Assistant"),
        ("item/completed", "reasoning") => (Kind::Reasoning, Some(Role::Reasoning), 1, "Reasoning"),
        ("item/completed", k) if TOOL_KINDS.contains(&k) => (Kind::Tool, Some(Role::Tool), 1, name.as_str()),
        ("thread/tokenUsage/updated", _) => (Kind::Telemetry, None, 0, ""),
        (t, _) if t.starts_with("turn/") => (Kind::Turn, None, 0, ""),
        _ => (Kind::System, None, 0, ""),
    };
    e.kind = k;
    e.role = role;
    e.depth = depth;
    if !title.is_empty() {
        e.title = title.to_string();
    }

    if ty == "thread/tokenUsage/updated" {
        let total = data.pointer("/tokenUsage/total");
        let num = |key: &str| total.and_then(|t| t.get(key)).and_then(Value::as_f64);
        e.input_tokens = num("inputTokens");
        e.output_tokens = num("outputTokens");
        e.usage_is_total = true;
    }
    if ty == "client/turn/requested" {
        e.model = data.pointer("/execution/model").and_then(Value::as_str).map(str::to_string);
    }
    e.duration_ms = item.get("durationMs").and_then(Value::as_f64);
    let status = item.get("status").and_then(Value::as_str);
    if ty == "item/completed" && TOOL_KINDS.contains(&kind) {
        let name = tool_name(kind, item);
        e.detail = match kind {
            "toolCall" => {
                let args = match item.get("arguments") {
                    Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone())),
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
                crate::classify::call_detail(name, &args)
            }
            "fileRead" => Some(crate::classify::short_path(&item_text(kind, item))).filter(|p| !p.is_empty()),
            "fileChange" => item_text(kind, item).lines().next().map(crate::classify::short_path),
            _ => crate::classify::first_line(&item_text(kind, item)),
        };
        e.tool = Some(name.to_string());
    }
    e.error_flag = matches!(ty, "provider/error" | "system/error")
        || status == Some("failed")
        || (ty == "item/completed" && non_empty(item.get("error")))
        || (ty == "turn/completed" && (non_empty(data.get("error")) || matches!(data.get("status").and_then(Value::as_str), Some("failed" | "error"))));
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &Path) {
        let c = Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE environments (id TEXT PRIMARY KEY, path TEXT);
             CREATE TABLE threads (id TEXT PRIMARY KEY, project_id TEXT, environment_id TEXT, provider_id TEXT NOT NULL,
               model_override TEXT, title TEXT, title_fallback TEXT, status TEXT NOT NULL, parent_thread_id TEXT,
               archived_at INTEGER, deleted_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE events (id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, sequence INTEGER NOT NULL, type TEXT NOT NULL,
               item_id TEXT, item_kind TEXT, turn_id TEXT, parent_tool_call_id TEXT, data TEXT NOT NULL DEFAULT '{}',
               created_at INTEGER NOT NULL);
             CREATE UNIQUE INDEX events_thread_sequence_idx ON events (thread_id, sequence);
             INSERT INTO environments VALUES ('env1', '/w/bb');
             INSERT INTO threads VALUES ('thr_a', 'p', 'env1', 'codex', NULL, 'Fix the bug', NULL, 'idle', NULL, NULL, NULL, 1000, 9000);
             INSERT INTO threads VALUES ('thr_b', 'p', NULL, 'pi', 'm2', NULL, 'untitled', 'active', 'thr_a', 5, NULL, 1000, 8000);
             INSERT INTO threads VALUES ('thr_gone', 'p', NULL, 'pi', NULL, 'x', NULL, 'idle', NULL, NULL, 7, 1000, 7000);",
        )
        .unwrap();
        let rows: [(i64, &str, Option<&str>, &str); 12] = [
            (1, "client/turn/requested", None, r#"{"input":[{"type":"text","text":"fix the bug"}],"execution":{"model":"gpt-5"}}"#),
            (2, "turn/started", None, "{}"),
            (3, "item/started", Some("agentMessage"), r#"{"item":{"id":"i1"}}"#),
            (4, "item/agentMessage/delta", None, r#"{"delta":"Loo"}"#),
            (5, "item/completed", Some("agentMessage"), r#"{"item":{"id":"i1","text":"Looking."}}"#),
            (6, "item/completed", Some("commandExecution"), r#"{"item":{"command":"ls","status":"completed","durationMs":12}}"#),
            (7, "item/completed", Some("toolCall"), r#"{"item":{"tool":"read_file","arguments":{"path":"a"},"status":"failed","durationMs":3}}"#),
            (8, "provider/unhandled", None, r#"{"raw":1}"#),
            (9, "thread/tokenUsage/updated", None, r#"{"tokenUsage":{"total":{"inputTokens":500,"outputTokens":50,"totalTokens":550}}}"#),
            (10, "item/completed", Some("agentMessage"), r#"{"item":{"id":"i2","text":"Done."}}"#),
            (11, "provider/error", None, r#"{"message":"rate limited","willRetry":true}"#),
            (12, "turn/completed", None, r#"{"status":"completed","error":null}"#),
        ];
        for (seq, ty, kind, data) in rows {
            c.execute(
                "INSERT INTO events (id, thread_id, sequence, type, item_kind, data, created_at) VALUES (?1,'thr_a',?2,?3,?4,?5,?6)",
                params![format!("evt_{seq}"), seq, ty, kind, data, 1_790_000_000_000_i64 + seq * 1000],
            )
            .unwrap();
        }
    }

    #[test]
    fn threads_and_events_map_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("bb.db");
        fixture(&db);
        let bb = Bb::open(&db).unwrap();

        let threads = bb.threads().unwrap();
        assert_eq!(threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["thr_a", "thr_b"], "deleted thread hidden");
        assert_eq!(threads[0].title.as_deref(), Some("Fix the bug"));
        assert_eq!((threads[0].cwd.as_deref(), threads[0].last_sequence), (Some("/w/bb"), Some(12)));
        assert_eq!((threads[1].title.as_deref(), threads[1].model.as_deref(), threads[1].archived), (Some("untitled"), Some("m2"), true));
        assert_eq!(threads[1].last_sequence, None);

        let events = bb.events("thr_a", 0, 100).unwrap();
        let seqs: Vec<i64> = events.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, [1, 2, 5, 6, 7, 9, 10, 11, 12], "deltas, item/started and provider/unhandled are skipped");
        assert_eq!(events[0].row_id, "evt_1");

        let e = |i: usize| &events[i].event;
        assert_eq!((e(0).role, e(0).kind, e(0).model.as_deref()), (Some(Role::User), Kind::Message, Some("gpt-5")));
        assert_eq!(e(0).text, "fix the bug");
        assert_eq!((e(2).role, e(2).title.as_str()), (Some(Role::Assistant), "Assistant"));
        assert_eq!((e(3).kind, e(3).title.as_str(), e(3).duration_ms), (Kind::Tool, "bash", Some(12.0)));
        assert_eq!((e(4).title.as_str(), e(4).is_error()), ("read_file", true));
        assert!(!e(3).is_error());
        assert_eq!((e(5).kind, e(5).usage_is_total, e(5).input_tokens, e(5).output_tokens), (Kind::Telemetry, true, Some(500.0), Some(50.0)));
        assert!(e(7).is_error(), "provider/error");
        assert_eq!(e(8).event_type, "turn/completed");
        assert!(!e(8).is_error());
        assert_eq!(e(6).event_type, "item/agentMessage", "not mistaken for a terminal event");

        let users = events.iter().filter(|e| e.event.role == Some(Role::User)).count();
        let tools = events.iter().filter(|e| e.event.kind == Kind::Tool).count();
        assert_eq!((users, tools), (1, 2));

        // Resuming from a watermark and paging.
        assert_eq!(bb.events("thr_a", 9, 100).unwrap().iter().map(|e| e.sequence).collect::<Vec<_>>(), [10, 11, 12]);
        assert_eq!(bb.events("thr_a", 0, 2).unwrap().len(), 2);
        assert!(bb.events("thr_b", 0, 10).unwrap().is_empty());
    }

    #[test]
    fn raw_event_is_the_stored_row() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("bb.db");
        fixture(&db);
        let bb = Bb::open(&db).unwrap();
        let raw: Value = serde_json::from_str(&bb.raw_event("thr_a", 5).unwrap().unwrap()).unwrap();
        assert_eq!((raw["id"].as_str(), raw["item_kind"].as_str()), (Some("evt_5"), Some("agentMessage")));
        assert_eq!(raw["data"]["item"]["text"], "Looking.");
        assert!(bb.raw_event("thr_a", 99).unwrap().is_none());
    }

    #[test]
    fn open_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("bb.db");
        fixture(&db);
        let bb = Bb::open(&db).unwrap();
        assert!(bb.conn.execute("DELETE FROM events", []).is_err());
        assert!(Bb::open(&tmp.path().join("missing.db")).is_err());
    }
}
