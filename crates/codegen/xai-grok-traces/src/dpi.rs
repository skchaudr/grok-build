//! durable-pi (dpi) agent sessions, read in place from `~/.dpi/backend` (read-only). Indexed by
//! `index/dpi_index.rs`; see `docs/traces/DPI.md`.
//!
//! One `sessions/<session-uuid>/agent.sqlite` per dpi session; each of its `conversations` is one
//! agent and one grok-traces session. The pointer to an event is `(agent.sqlite, conversation id,
//! entries.id)`: `entries.id` is increasing in a db, so it plays the role bb's `sequence` does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

use crate::index::Result;
use crate::normalize::{Kind, NormalizedEvent, normalize};

/// Lock wait when reading dpi dbs; the dpi backend is the writer, so give up fast.
const BUSY: Duration = Duration::from_millis(500);
/// Entry kinds that carry no session content: the system prompt is injected on every conversation.
const SKIPPED_KINDS: [&str; 1] = ["pi.system"];

pub fn default_dir(home: &Path) -> PathBuf {
    home.join(".dpi/backend")
}

/// `$DPI_DATA_DIR`, else `$DPI_HOME/backend`, else `~/.dpi/backend` (the backend's own resolution).
pub fn data_dir(home: &Path) -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    env("DPI_DATA_DIR").or_else(|| env("DPI_HOME").map(|h| h.join("backend"))).unwrap_or_else(|| default_dir(home))
}

pub fn db_path(dir: &Path, session: &str) -> PathBuf {
    dir.join("sessions").join(session).join("agent.sqlite")
}

/// A session id from a hook payload becomes a path component: only a plain id qualifies.
pub fn valid_session_id(session: &str) -> bool {
    !session.is_empty() && session.len() <= 64 && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The key a conversation is indexed under: `<agent.sqlite path>#<conversation id>` (unique, so it can be a session's `file_path`).
pub fn conv_key(db: &Path, conversation: i64) -> String {
    format!("{}#{conversation}", db.display())
}

/// Inverse of [`conv_key`].
pub fn split_key(key: &str) -> Option<(&str, i64)> {
    let (db, conv) = key.rsplit_once('#')?;
    Some((db, conv.parse().ok()?))
}

/// The harness's own id for a conversation: `<session uuid>:<conversation id>`.
pub fn native_id(session: &str, conversation: i64) -> String {
    format!("{session}:{conversation}")
}

/// Inverse of [`native_id`].
pub fn split_native(native: &str) -> Option<(&str, i64)> {
    let (session, conv) = native.rsplit_once(':')?;
    Some((session, conv.parse().ok()?))
}

#[derive(Clone, Debug, PartialEq)]
pub struct DpiConversation {
    pub session: String,
    pub id: i64,
    pub db: PathBuf,
    /// A conversation name from the catalog; a top-level conversation falls back to the session's name.
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// The conversation that owns this one (a subagent's parent).
    pub parent_id: Option<i64>,
    pub created_at: i64,
    /// Newest message timestamp among the last entries, else `created_at`.
    pub updated_at: i64,
    /// Highest entry id (any kind); `None` for a conversation with no entries.
    pub last_sequence: Option<i64>,
}

impl DpiConversation {
    pub fn native_id(&self) -> String {
        native_id(&self.session, self.id)
    }

    pub fn key(&self) -> String {
        conv_key(&self.db, self.id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DpiEvent {
    pub entry_id: i64,
    pub event: NormalizedEvent,
}

#[derive(Default)]
struct SessionMeta {
    name: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    created_at: i64,
}

/// `catalog.sqlite`: names and defaults. Optional; sessions are found on disk.
#[derive(Default)]
struct Catalog {
    sessions: HashMap<String, SessionMeta>,
    names: HashMap<(String, i64), String>,
}

fn open_ro(path: &Path, busy: Duration) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(busy)?;
    Ok(conn)
}

fn nonblank(s: String) -> Option<String> {
    Some(s).filter(|s| !s.trim().is_empty())
}

impl Catalog {
    fn load(dir: &Path) -> Catalog {
        let path = dir.join("catalog.sqlite");
        if !path.is_file() {
            return Catalog::default();
        }
        open_ro(&path, BUSY).and_then(|c| Self::read(&c)).unwrap_or_default()
    }

    fn read(conn: &Connection) -> Result<Catalog> {
        let mut cat = Catalog::default();
        let mut stmt = conn.prepare("SELECT id, name, cwd, model, provider, createdAt FROM sessions")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                SessionMeta {
                    name: nonblank(r.get(1)?),
                    cwd: nonblank(r.get(2)?),
                    model: nonblank(r.get(3)?),
                    provider: nonblank(r.get(4)?),
                    created_at: r.get(5)?,
                },
            ))
        })?;
        for row in rows {
            let (id, meta) = row?;
            cat.sessions.insert(id, meta);
        }
        let mut stmt = conn.prepare("SELECT sessionId, conversationId, name FROM conversation_names")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (session, conv, name) = row?;
            if let Some(name) = nonblank(name) {
                cat.names.insert((session, conv), name);
            }
        }
        Ok(cat)
    }
}

/// Every conversation of every session under a dpi data dir, newest first, plus the sessions that could not be read.
pub struct Listing {
    pub conversations: Vec<DpiConversation>,
    pub unreadable: Vec<(String, String)>,
}

/// Session ids that have an `agent.sqlite`, in name order.
fn session_ids(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("sessions")) else { return Vec::new() };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (valid_session_id(&name) && db_path(dir, &name).is_file()).then_some(name)
        })
        .collect();
    ids.sort();
    ids
}

pub fn list(dir: &Path) -> Listing {
    let cat = Catalog::load(dir);
    let mut out = Listing { conversations: Vec::new(), unreadable: Vec::new() };
    for session in session_ids(dir) {
        match conversations_of(dir, &cat, &session, None) {
            Ok(mut c) => out.conversations.append(&mut c),
            Err(e) => out.unreadable.push((session, e.to_string())),
        }
    }
    out.conversations.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.key().cmp(&b.key())));
    out
}

/// One conversation (a hook's `session:conversation`), or `None` when it does not exist.
pub fn find(dir: &Path, session: &str, conversation: i64) -> Result<Option<DpiConversation>> {
    if !valid_session_id(session) || !db_path(dir, session).is_file() {
        return Ok(None);
    }
    let cat = Catalog::load(dir);
    Ok(conversations_of(dir, &cat, session, Some(conversation))?.into_iter().next())
}

fn conversations_of(dir: &Path, cat: &Catalog, session: &str, only: Option<i64>) -> Result<Vec<DpiConversation>> {
    let db = db_path(dir, session);
    let conn = open_ro(&db, BUSY)?;
    let meta = cat.sessions.get(session);
    let mut stmt = conn.prepare(
        "SELECT c.id, c.owner_conversation_id,
                (SELECT MAX(id) FROM entries WHERE conversation_id = c.id),
                (SELECT MAX(json_extract(record, '$.model[0].timestamp'))
                   FROM (SELECT record FROM entries WHERE conversation_id = c.id ORDER BY id DESC LIMIT 8))
         FROM conversations c WHERE (?1 IS NULL OR c.id = ?1) ORDER BY c.id",
    )?;
    let rows = stmt.query_map([only], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, Option<f64>>(3)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, parent_id, last_sequence, ts) = row?;
        let created_at = meta.map_or(0, |m| m.created_at);
        let name = cat
            .names
            .get(&(session.to_string(), id))
            .cloned()
            .or_else(|| if parent_id.is_none() { meta.and_then(|m| m.name.clone()) } else { None });
        out.push(DpiConversation {
            session: session.to_string(),
            id,
            db: db.clone(),
            name,
            cwd: meta.and_then(|m| m.cwd.clone()),
            model: meta.and_then(|m| m.model.clone()),
            provider: meta.and_then(|m| m.provider.clone()),
            parent_id,
            created_at,
            updated_at: ts.map_or(created_at, |t| t as i64),
            last_sequence,
        });
    }
    Ok(out)
}

/// An open `agent.sqlite`. Callers open per scan and drop the handle; nothing holds the db between scans.
pub struct DpiDb {
    conn: Connection,
}

impl DpiDb {
    /// Opens read-only; the backend keeps writing (WAL), so readers never block it.
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self { conn: open_ro(path, BUSY)? })
    }

    /// Up to `limit` indexable entries of a conversation with `id > after`, in order.
    /// Fewer than `limit` means it is drained; resume from the last `entry_id`.
    pub fn events(&self, conversation: i64, after: i64, limit: usize) -> Result<Vec<DpiEvent>> {
        let skipped = SKIPPED_KINDS.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(",");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, record FROM entries
             WHERE conversation_id = ?1 AND id > ?2 AND COALESCE(json_extract(record, '$.kind'), '') NOT IN ({skipped})
             ORDER BY id LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![conversation, after, limit as i64], |r| {
            let record: String = r.get(1)?;
            Ok(DpiEvent {
                entry_id: r.get(0)?,
                event: map_entry(&serde_json::from_str(&record).unwrap_or(Value::Null)),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every entry record of a conversation, in order: what a past-session view replays.
    pub fn records(&self, conversation: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare("SELECT record FROM entries WHERE conversation_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([conversation], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().filter_map(|r| serde_json::from_str(&r).ok()).collect())
    }

    /// The full entry row as one JSON line (the analogue of a transcript's raw line).
    pub fn raw_entry(&self, conversation: i64, entry_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT head, commit_seq, record FROM entries WHERE conversation_id = ?1 AND id = ?2",
                params![conversation, entry_id],
                |r| {
                    let record: String = r.get(2)?;
                    Ok(json!({
                        "id": entry_id, "conversation_id": conversation, "head": r.get::<_, Option<i64>>(0)?,
                        "commit_seq": r.get::<_, i64>(1)?,
                        "record": serde_json::from_str::<Value>(&record).unwrap_or(Value::Null),
                    })
                    .to_string())
                },
            )
            .optional()?)
    }
}

/// A conversation as Pi session lines, the format every Pi reader (the live merge, the replay converter) parses.
/// A compaction's message is the summary handed to the model as a user turn: its `<summary>` becomes a `compaction` line.
pub fn pi_lines(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| Some((r, r.pointer("/model/0").filter(|m| m.is_object())?)))
        .map(|(r, m)| {
            let ts = m
                .get("timestamp")
                .and_then(Value::as_i64)
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
            if r.get("kind").and_then(Value::as_str) == Some("pi.compaction") {
                let text = crate::merge::block_text(m.get("content").unwrap_or(&Value::Null));
                let summary = text
                    .split_once("<summary>")
                    .and_then(|(_, rest)| rest.rsplit_once("</summary>"))
                    .map_or(text.as_str(), |(inner, _)| inner);
                return json!({ "type": "compaction", "timestamp": ts, "summary": summary.trim() }).to_string();
            }
            json!({ "type": "message", "timestamp": ts, "message": m }).to_string()
        })
        .collect()
}

/// The conversation a [`conv_key`] names, as Pi jsonl; empty when it cannot be read.
pub fn pi_transcript(key: &str) -> String {
    let Some((db, conv)) = split_key(key) else { return String::new() };
    DpiDb::open(Path::new(db)).and_then(|d| d.records(conv)).map(|r| pi_lines(&r).join("\n")).unwrap_or_default()
}

/// One entry → the shared [`NormalizedEvent`]. `record.model[0]` is a Pi-shaped message, so it goes through the
/// same path a Pi session line does. An entry with no message (`pi.reset`) is a noise marker named by its kind.
pub fn map_entry(record: &Value) -> NormalizedEvent {
    match record.pointer("/model/0").filter(|m| m.is_object()) {
        Some(message) => normalize(&json!({
            "type": "message",
            "timestamp": message.get("timestamp").cloned().unwrap_or(Value::Null),
            "message": message,
        })),
        None => {
            let kind = record.get("kind").and_then(Value::as_str).unwrap_or("entry");
            let mut e = normalize(&json!({ "type": kind }));
            e.kind = Kind::System;
            e.noise = true;
            e
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::Role;

    pub(crate) const BASE: i64 = 1_790_000_000_000;

    /// A synthetic dpi data dir: `catalog.sqlite` plus `sessions/<uuid>/agent.sqlite` for two sessions.
    pub(crate) fn fixture(dir: &Path) {
        std::fs::create_dir_all(dir.join("sessions/sess-a")).unwrap();
        std::fs::create_dir_all(dir.join("sessions/sess-b")).unwrap();
        let cat = Connection::open(dir.join("catalog.sqlite")).unwrap();
        cat.execute_batch(&format!(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY,name TEXT NOT NULL,cwd TEXT NOT NULL,dbPath TEXT NOT NULL UNIQUE,createdAt INTEGER NOT NULL,model TEXT NOT NULL,provider TEXT NOT NULL);
             CREATE TABLE conversation_names(sessionId TEXT NOT NULL,conversationId INTEGER NOT NULL,name TEXT NOT NULL,PRIMARY KEY(sessionId,conversationId));
             INSERT INTO sessions VALUES ('sess-a','Fix the bug','/w/dpi','x',{BASE},'default-model','cliproxy');
             INSERT INTO sessions VALUES ('sess-b','Second','/w/other','y',{BASE},'m2','cliproxy');
             INSERT INTO conversation_names VALUES ('sess-a',3,'reviewer');"
        ))
        .unwrap();
        for s in ["sess-a", "sess-b"] {
            let c = Connection::open(db_path(dir, s)).unwrap();
            c.execute_batch(
                "CREATE TABLE conversations (id INTEGER PRIMARY KEY, owner_conversation_id INTEGER, owner_task_id INTEGER, record TEXT NOT NULL);
                 CREATE TABLE entries (id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL, head INTEGER, commit_seq INTEGER NOT NULL, record TEXT NOT NULL);
                 INSERT INTO conversations VALUES (1, NULL, NULL, '{\"id\":1}');",
            )
            .unwrap();
        }
        let a = Connection::open(db_path(dir, "sess-a")).unwrap();
        a.execute_batch(
            "INSERT INTO conversations VALUES (3, 1, 2, '{\"id\":3}');
             INSERT INTO conversations VALUES (5, NULL, NULL, '{\"id\":5}');",
        )
        .unwrap();
        let ts = |n: i64| BASE + n * 1000;
        let rows: Vec<(i64, i64, Value)> = vec![
            (9, 1, json!({"model":[{"role":"user","content":"fix the bug","timestamp":ts(1)}],"kind":"pi.user","id":9,"conversationId":1})),
            (12, 1, json!({"model":[{"role":"system","content":"","sections":{"p":"huge"}}],"kind":"pi.system","id":12,"conversationId":1})),
            (13, 1, json!({"model":[{"role":"assistant","content":[
                {"type":"thinking","thinking":"hmm"},
                {"type":"toolCall","id":"call_1","name":"bash","arguments":{"command":"ls -la"}}],
                "model":"claude-sonnet-5-5","usage":{"input":4,"output":65,"cacheRead":100,"cacheWrite":50,"totalTokens":219},
                "stopReason":"toolUse","timestamp":ts(2)}],"kind":"pi.assistant","id":13,"conversationId":1})),
            (15, 1, json!({"model":[{"role":"toolResult","toolCallId":"call_1","toolName":"bash","content":[{"type":"text","text":"a.txt\n"}],
                "isError":false,"timestamp":ts(5)}],"data":{"diagnostics":[]},"kind":"pi.tool-result","id":15,"conversationId":1})),
            (17, 1, json!({"model":[{"role":"assistant","content":[{"type":"toolCall","id":"call_2","name":"bash","arguments":{"command":"false"}}],
                "model":"claude-sonnet-5-5","timestamp":ts(6)}],"kind":"pi.assistant","id":17,"conversationId":1})),
            (19, 1, json!({"model":[{"role":"toolResult","toolCallId":"call_2","toolName":"bash","content":[{"type":"text","text":"boom"}],
                "isError":true,"timestamp":ts(7)}],"kind":"pi.tool-result","id":19,"conversationId":1})),
            (20, 1, json!({"model":[{"role":"assistant","content":[{"type":"text","text":"Done."}],"model":"claude-sonnet-5-5","timestamp":ts(8)}],"kind":"pi.assistant","id":20,"conversationId":1})),
            (21, 1, json!({"kind":"pi.reset","id":21,"conversationId":1,"head":21})),
            (30, 3, json!({"model":[{"role":"user","content":"review it","timestamp":ts(3)}],"kind":"pi.user","id":30,"conversationId":3})),
            (31, 3, json!({"model":[{"role":"assistant","content":[{"type":"text","text":"looks fine"}],"model":"m3","timestamp":ts(4)}],"kind":"pi.assistant","id":31,"conversationId":3})),
        ];
        for (id, conv, rec) in rows {
            a.execute(
                "INSERT INTO entries VALUES (?1, ?2, NULL, ?1, ?3)",
                params![id, conv, rec.to_string()],
            )
            .unwrap();
        }
        let b = Connection::open(db_path(dir, "sess-b")).unwrap();
        b.execute(
            "INSERT INTO entries VALUES (9, 1, NULL, 1, ?1)",
            [json!({"model":[{"role":"user","content":"second","timestamp":ts(1)}],"kind":"pi.user","id":9,"conversationId":1}).to_string()],
        )
        .unwrap();
    }

    #[test]
    fn keys_roundtrip() {
        let k = conv_key(Path::new("/d/sessions/s/agent.sqlite"), 7);
        assert_eq!(split_key(&k), Some(("/d/sessions/s/agent.sqlite", 7)));
        assert_eq!(split_native(&native_id("abc-1", 2)), Some(("abc-1", 2)));
        assert_eq!(split_native("no-colon"), None);
        assert!(valid_session_id("a2e0dfcd-bf7c") && !valid_session_id("../x") && !valid_session_id(""));
    }

    #[test]
    fn conversations_are_listed_with_names_and_parents() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let l = list(tmp.path());
        assert!(l.unreadable.is_empty());
        let get = |s: &str, id: i64| l.conversations.iter().find(|c| c.session == s && c.id == id).unwrap();
        assert_eq!(l.conversations.len(), 4);
        let root = get("sess-a", 1);
        assert_eq!((root.name.as_deref(), root.cwd.as_deref(), root.model.as_deref()), (Some("Fix the bug"), Some("/w/dpi"), Some("default-model")));
        assert_eq!((root.parent_id, root.last_sequence, root.updated_at), (None, Some(21), BASE + 8000));
        let child = get("sess-a", 3);
        assert_eq!((child.name.as_deref(), child.parent_id, child.last_sequence), (Some("reviewer"), Some(1), Some(31)));
        assert_eq!(get("sess-a", 5).last_sequence, None, "no entries");
        assert_eq!(get("sess-b", 1).name.as_deref(), Some("Second"));
        assert_eq!(find(tmp.path(), "sess-a", 3).unwrap().unwrap().id, 3);
        assert!(find(tmp.path(), "sess-a", 99).unwrap().is_none());
        assert!(find(tmp.path(), "../etc", 1).unwrap().is_none());
    }

    #[test]
    fn entries_map_like_pi_messages() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let db = DpiDb::open(&db_path(tmp.path(), "sess-a")).unwrap();
        let events = db.events(1, 0, 100).unwrap();
        let ids: Vec<i64> = events.iter().map(|e| e.entry_id).collect();
        assert_eq!(ids, [9, 13, 15, 17, 19, 20, 21], "pi.system is skipped");
        let e = |i: usize| &events[i].event;
        assert_eq!((e(0).role, e(0).kind, e(0).text.as_str()), (Some(Role::User), Kind::Message, "fix the bug"));
        assert_eq!(e(0).timestamp_ms, Some(BASE + 1000));
        assert_eq!((e(1).kind, e(1).tool.as_deref(), e(1).detail.as_deref()), (Kind::Tool, Some("bash"), Some("ls -la")));
        assert_eq!(e(1).model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(e(1).tool_call_id.as_deref(), Some("call_1"));
        assert_eq!((e(2).role, e(2).tool_result_id.as_deref(), e(2).is_error()), (Some(Role::ToolResult), Some("call_1"), false));
        assert!(e(4).is_error(), "isError tool result");
        assert_eq!((e(5).role, e(5).text.as_str()), (Some(Role::Assistant), "Done."));
        assert_eq!((e(6).kind, e(6).event_type.as_str(), e(6).noise), (Kind::System, "pi.reset", true));

        assert_eq!(db.events(1, 17, 100).unwrap().iter().map(|e| e.entry_id).collect::<Vec<_>>(), [19, 20, 21]);
        assert_eq!(db.events(1, 0, 2).unwrap().len(), 2);
        assert_eq!(db.events(3, 0, 100).unwrap().len(), 2, "conversations are separate");
    }

    #[test]
    fn raw_entry_is_the_stored_row_and_open_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let db = DpiDb::open(&db_path(tmp.path(), "sess-a")).unwrap();
        let raw: Value = serde_json::from_str(&db.raw_entry(1, 20).unwrap().unwrap()).unwrap();
        assert_eq!((raw["id"].as_i64(), raw["commit_seq"].as_i64()), (Some(20), Some(20)));
        assert_eq!(raw["record"]["model"][0]["content"][0]["text"], "Done.");
        assert!(db.raw_entry(1, 99).unwrap().is_none());
        assert!(db.raw_entry(3, 20).unwrap().is_none(), "entry belongs to another conversation");
        assert_eq!(db.records(3).unwrap().len(), 2);
        assert!(db.conn.execute("DELETE FROM entries", []).is_err());
        assert!(DpiDb::open(&tmp.path().join("missing.sqlite")).is_err());
    }

    /// A running dpi session goes through the live merge: its conversation reads as Pi lines, a change to its db or WAL
    /// wakes the poll, and a receipt opens a call the conversation has not written yet.
    #[test]
    fn a_dpi_conversation_merges_live_like_a_pi_session() {
        use crate::live::{LiveReceipt, transcript, transcript_files};
        use crate::merge::{Piece, load};
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let db = db_path(tmp.path(), "sess-a");
        let key = PathBuf::from(conv_key(&db, 1));
        assert_eq!(transcript_files("dpi", &key), [db.clone(), PathBuf::from(format!("{}-wal", db.display()))]);
        let pre = LiveReceipt {
            event: "tool_execution_start".into(),
            tool: Some("bash".into()),
            tool_use_id: Some("call_3".into()),
            detail: Some("cargo test".into()),
            at_ms: BASE + 9_000,
            ..Default::default()
        };
        let (pieces, m, _) = load("dpi", &transcript("dpi", &key), &[pre], Err(()));
        let shape: Vec<String> = pieces
            .iter()
            .map(|p| match p {
                Piece::User(t) => format!("user {t}"),
                Piece::Text(t) => format!("text {t}"),
                Piece::Thinking { text, dur_ms, .. } => format!("thought {text} {dur_ms:?}"),
                Piece::Open { id, label, .. } => format!("open {id} {label:?}"),
                Piece::Complete { id, output, error, .. } => format!("done {id} {output:?} {error}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "user fix the bug",
                "thought hmm None",
                "done call_1 \"a.txt\\n\" false",
                "done call_2 \"boom\" true",
                "text Done.",
                "open call_3 Some(\"cargo test\")",
            ]
        );
        assert_eq!(m.running().map(|(id, _)| id), Some("call_3".to_string()));
    }
}
