//! Live receipt queries over synthetic sessions; no real session text.

use std::fs;
use std::path::Path;

use xai_grok_traces::{Receipt, SessionFilter, SessionRow, TraceIndex, default_roots};

const CURSOR: &str = "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"<user_query>\\nhello\\n</user_query>\"}]}}\n";
const CLAUDE: &str = "{\"type\":\"user\",\"timestamp\":\"2026-10-01T00:00:00Z\",\"cwd\":\"/w/claude\",\"sessionId\":\"s\",\"message\":{\"role\":\"user\",\"content\":\"claude task\"}}\n";

struct Env {
    _tmp: tempfile::TempDir,
    home: std::path::PathBuf,
    idx: TraceIndex,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let idx = TraceIndex::open(&tmp.path().join("idx/t.sqlite")).unwrap();
    Env { _tmp: tmp, home, idx }
}

impl Env {
    fn scan(&mut self) {
        let roots = default_roots(&self.home, &self.home.join(".grok"));
        self.idx.scan(&roots).unwrap();
    }

    fn session(&self, source: &str) -> SessionRow {
        let rows = self.idx.list_sessions(&SessionFilter { source: Some(source.into()), ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 1, "{source}: {rows:?}");
        rows.into_iter().next().unwrap()
    }

    /// Records a receipt at a fixed time.
    fn rcpt(&mut self, source: &str, event: &str, at_ms: i64, f: impl FnOnce(&mut Receipt)) -> i64 {
        let mut r = Receipt { at_ms, source: source.into(), event: event.into(), ..Default::default() };
        f(&mut r);
        self.idx.record_receipt(&r).unwrap()
    }
}

fn ids(v: &[xai_grok_traces::live::LiveReceipt]) -> Vec<i64> {
    v.iter().map(|r| r.id).collect()
}

fn live_ids(e: &Env) -> Vec<String> {
    e.idx.live_sessions().unwrap().into_iter().map(|s| s.id).collect()
}

#[test]
fn turn_receipts_keep_the_boundary_children_and_unkeyed_cursor_receipts() {
    let mut e = env();
    write(&e.home.join(".cursor/projects/w-proj/agent-transcripts/abc/abc.jsonl"), CURSOR);
    e.scan();
    let row = e.session("cursor");
    assert_eq!(row.native_id.as_deref(), Some("abc"));
    let chat = |r: &mut Receipt| r.native_session = Some("abc".into());

    let old_tool = e.rcpt("cursor", "preToolUse", 1000, chat);
    let child_start = e.rcpt("cursor", "subagentStart", 1100, |r| {
        chat(r);
        r.agent_id = Some("k1".into());
    });
    let _old_prompt = e.rcpt("cursor", "beforeSubmitPrompt", 1200, chat);
    let _stop = e.rcpt("cursor", "stop", 1300, chat);
    // A subagent's own stop is not this session's boundary.
    let kid_stop_event = e.rcpt("cursor", "stop", 2500, |r| {
        chat(r);
        r.agent_id = Some("k1".into());
    });
    let prompt = e.rcpt("cursor", "beforeSubmitPrompt", 2000, chat);
    let tool = e.rcpt("cursor", "preToolUse", 2100, chat);
    let child_stop = e.rcpt("cursor", "subagentStop", 2200, |r| {
        chat(r);
        r.agent_id = Some("k1".into());
    });
    // Another chat and another source never leak in.
    e.rcpt("cursor", "preToolUse", 2300, |r| r.native_session = Some("other".into()));
    e.rcpt("claude", "PreToolUse", 2300, |r| r.native_session = Some("abc".into()));

    let got = e.idx.turn_receipts(&row.id).unwrap();
    assert_eq!(ids(&got), [child_start, prompt, tool, child_stop, kid_stop_event], "boundary itself, pre-boundary ChildStart, no duplicate, sorted");
    assert!(!ids(&got).contains(&old_tool));
    assert_eq!(got[0].agent_id.as_deref(), Some("k1"));
    assert_eq!(got[0].session_id, None, "unkeyed receipts are returned by native id");
    assert_eq!(got[1].native_session.as_deref(), Some("abc"));
}

#[test]
fn turn_receipts_without_a_boundary_return_everything() {
    let mut e = env();
    write(&e.home.join(".cursor/projects/w-proj/agent-transcripts/abc/abc.jsonl"), CURSOR);
    e.scan();
    let row = e.session("cursor");
    let a = e.rcpt("cursor", "preToolUse", 10, |r| r.session_id = Some(row.id.clone()));
    let b = e.rcpt("cursor", "afterFileEdit", 20, |r| r.native_session = Some("abc".into()));
    assert_eq!(ids(&e.idx.turn_receipts(&row.id).unwrap()), [a, b]);
    assert!(e.idx.turn_receipts("nope").unwrap().is_empty());
}

#[test]
fn live_sessions_follow_the_newest_boundary() {
    let mut e = env();
    write(&e.home.join(".claude/projects/-w-claude/c1.jsonl"), CLAUDE);
    write(&e.home.join(".claude/projects/-w-claude/c2.jsonl"), CLAUDE);
    write(&e.home.join(".claude/projects/-w-claude/c3.jsonl"), CLAUDE);
    e.scan();
    let rows = e.idx.list_sessions(&SessionFilter { source: Some("claude".into()), ..Default::default() }).unwrap();
    let id_of = |name: &str| rows.iter().find(|r| r.file_path.ends_with(name)).unwrap().id.clone();
    let (c1, c2, c3) = (id_of("c1.jsonl"), id_of("c2.jsonl"), id_of("c3.jsonl"));
    let keyed = |sid: &str| {
        let sid = sid.to_string();
        move |r: &mut Receipt| r.session_id = Some(sid.clone())
    };

    assert!(live_ids(&e).is_empty(), "no receipts, not live");

    // c1: End@100 then Prompt@101 (a resumed id) stays live.
    e.rcpt("claude", "SessionEnd", 100, keyed(&c1));
    e.rcpt("claude", "UserPromptSubmit", 101, keyed(&c1));
    // c2: ends, then a compaction's SessionStart (dropped) and tool traffic: still ended.
    e.rcpt("claude", "SessionEnd", 100, keyed(&c2));
    e.rcpt("claude", "SessionStart", 101, |r| {
        r.session_id = Some(c2.clone());
        r.detail = Some("compact".into());
    });
    e.rcpt("claude", "PreToolUse", 102, keyed(&c2));
    // c3: a subagent's SessionEnd (agent_id set) does not end it.
    e.rcpt("claude", "Stop", 100, keyed(&c3));
    e.rcpt("claude", "SessionEnd", 101, |r| {
        r.session_id = Some(c3.clone());
        r.agent_id = Some("a".into());
    });
    // Same at_ms: the higher id wins.
    let live = live_ids(&e);
    assert!(live.contains(&c1) && live.contains(&c3), "{live:?}");
    assert!(!live.contains(&c2), "{live:?}");
    e.rcpt("claude", "SessionEnd", 101, keyed(&c1));
    assert!(!live_ids(&e).contains(&c1), "tie on at_ms broken by id");
    e.rcpt("claude", "Notification", 150, |r| {
        r.session_id = Some(c1.clone());
        r.detail = Some("permission_prompt".into());
    });
    assert!(!live_ids(&e).contains(&c1), "a non-idle Notification is no boundary");
    e.rcpt("claude", "Notification", 160, |r| {
        r.session_id = Some(c1.clone());
        r.detail = Some("idle_prompt".into());
    });
    assert!(live_ids(&e).contains(&c1), "idle_prompt reopens");
}

#[test]
fn receipt_only_rows_group_by_chat_and_yield_to_the_session_row() {
    let mut e = env();
    let chat = |name: &'static str| move |r: &mut Receipt| r.native_session = Some(name.into());
    e.rcpt("cursor", "sessionStart", 10, |r| {
        chat("zzz")(r);
        r.harness_pid = Some(4242);
        r.harness_start = Some(1_700_000_000);
    });
    e.rcpt("cursor", "preToolUse", 20, |r| {
        chat("zzz")(r);
        r.cwd = Some("/w/proj".into());
    });
    e.rcpt("cursor", "sessionStart", 30, chat("done"));
    e.rcpt("cursor", "sessionEnd", 31, chat("done"));
    // Claude receipts with no key are never grouped, nor are Claude NULL-session ones with a chat id.
    e.rcpt("claude", "PreToolUse", 40, |r| r.native_session = Some("zzz".into()));
    e.rcpt("claude", "PreToolUse", 41, |_| {});

    let rows = e.idx.live_sessions().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!((row.id.as_str(), row.source.as_str(), row.title.as_str()), ("cursor:rcpt:zzz", "cursor", "proj"));
    assert_eq!((row.native_id.as_deref(), row.harness_pid, row.harness_start), (Some("zzz"), Some(4242), Some(1_700_000_000)));
    assert_eq!((row.started_at, row.updated_at, row.file_path.as_os_str().is_empty()), (Some(10), Some(20), true));
    assert_eq!((row.context_tokens, row.context_window), (None, None));
    assert_eq!(ids(&e.idx.turn_receipts("cursor:rcpt:zzz").unwrap()).len(), 2, "sessionStart is the boundary and the tool call follows; the claude receipt is another source");

    // The transcript appears: the session row takes over, no duplicate.
    let tr = "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"<user_query>\\nhi\\n</user_query>\"}]}}\n";
    write(&e.home.join(".cursor/projects/w-proj/agent-transcripts/zzz/zzz.jsonl"), tr);
    e.scan();
    let sess = e.session("cursor");
    let rows = e.idx.live_sessions().unwrap();
    assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [sess.id.as_str()]);
    assert_eq!(rows[0].harness_pid, Some(4242), "the newest receipt's process lands on the session row");
    assert_eq!(e.idx.turn_receipts(&sess.id).unwrap().len(), 2);
}

#[test]
fn last_receipt_orders_by_time_then_id_and_counts_since() {
    let mut e = env();
    write(&e.home.join(".cursor/projects/w-proj/agent-transcripts/abc/abc.jsonl"), CURSOR);
    e.scan();
    let row = e.session("cursor");
    e.rcpt("cursor", "stop", 200, |r| r.session_id = Some(row.id.clone()));
    e.rcpt("cursor", "preToolUse", 100, |r| r.session_id = Some(row.id.clone()));
    assert_eq!(e.idx.last_receipt(&row.id).unwrap(), Some((200, "stop".into())));
    assert_eq!(e.idx.last_receipt_at().unwrap(), Some(200));
    // Unkeyed receipts count once the row's native_id matches.
    e.rcpt("cursor", "beforeSubmitPrompt", 300, |r| r.native_session = Some("abc".into()));
    assert_eq!(e.idx.last_receipt(&row.id).unwrap(), Some((300, "beforeSubmitPrompt".into())));
    assert_eq!((e.idx.receipt_count_since(0).unwrap(), e.idx.receipt_count_since(200).unwrap(), e.idx.receipt_count_since(301).unwrap()), (3, 2, 0));
}

#[test]
fn session_json_carries_live_fields_and_tolerates_old_remotes() {
    use xai_grok_traces::remote::{session_from_json, session_to_json};
    let row = SessionRow {
        id: "x".into(),
        native_id: Some("n".into()),
        mtime_ms: Some(5),
        harness_pid: Some(7),
        harness_start: Some(8),
        context_tokens: Some(9),
        context_window: Some(10),
        ..Default::default()
    };
    assert_eq!(session_from_json(&session_to_json(&row)).unwrap(), row);
    let old = session_from_json(&serde_json::json!({"id": "y"})).unwrap();
    assert_eq!((old.native_id, old.mtime_ms, old.harness_pid, old.context_tokens), (None, None, None, None));
}
