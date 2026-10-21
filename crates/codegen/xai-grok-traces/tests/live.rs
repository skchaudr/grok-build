//! Live receipt queries over synthetic sessions; no real session text.

use std::fs;
use std::path::Path;

use xai_grok_traces::live::{LiveRow, LiveState, fold_all, render};
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

// ---- sequence rows that go through a real index ----

/// Receipt times are `B + seconds`, after every timestamp in the fixtures.
const B: i64 = 1_800_000_000_000;

fn at(s: f64) -> i64 {
    B + (s * 1000.0).round() as i64
}

/// Writes a transcript and sets its mtime before `B` so only receipts and the fixture timestamp count.
fn write_old(path: &Path, text: &str) {
    write(path, text);
    fs::File::options().write(true).open(path).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_900_000)).unwrap();
}

fn show(r: &LiveRow) -> String {
    let v = render(r, 0);
    format!("{} {}", v.glyph, v.text)
}

fn all(e: &Env, now: f64) -> Vec<LiveRow> {
    fold_all(&e.idx, at(now), |_, _| true).unwrap()
}

fn claude_env(names: &[&str]) -> (Env, Vec<String>) {
    let mut e = env();
    for n in names {
        write_old(&e.home.join(format!(".claude/projects/-w-claude/{n}.jsonl")), CLAUDE);
    }
    e.scan();
    let rows = e.idx.list_sessions(&SessionFilter { source: Some("claude".into()), ..Default::default() }).unwrap();
    let ids = names.iter().map(|n| rows.iter().find(|r| r.file_path.ends_with(format!("{n}.jsonl"))).unwrap().id.clone()).collect();
    (e, ids)
}

fn ck(e: &mut Env, sid: &str, event: &str, t: f64, f: impl FnOnce(&mut Receipt)) {
    e.rcpt("claude", event, at(t), |r| {
        r.session_id = Some(sid.into());
        r.harness_pid = Some(4242);
        r.harness_start = Some(1_700_000_000);
        f(r);
    });
}

#[test]
fn seq_07_child_stop_does_not_set_the_clock() {
    let (mut e, ids) = claude_env(&["c1"]);
    let s = &ids[0];
    ck(&mut e, s, "UserPromptSubmit", 0.0, |_| {});
    ck(&mut e, s, "SubagentStart", 1.0, |r| r.agent_id = Some("x".into()));
    ck(&mut e, s, "Stop", 5.0, |_| {});
    ck(&mut e, s, "SubagentStop", 6.0, |r| r.agent_id = Some("x".into()));
    let rows = all(&e, 9.0);
    assert_eq!(rows.len(), 1);
    assert_eq!((show(&rows[0]), rows[0].bg, rows[0].age_ms), ("○ your turn".into(), 0, 4000));
}

#[test]
fn seq_12_orphan_pre_before_the_boundary_is_closed() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "PreToolUse", 0.0, |r| r.tool = Some("A".into()));
    ck(&mut e, &ids[0], "Stop", 5.0, |_| {});
    let rows = all(&e, 8.0);
    assert_eq!(show(&rows[0]), "○ your turn");
}

#[test]
fn seq_20_child_start_with_agent_id_counts_after_the_boundary() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "Stop", 0.0, |_| {});
    ck(&mut e, &ids[0], "SubagentStart", 1.0, |r| r.agent_id = Some("x".into()));
    assert_eq!(show(&all(&e, 5.0)[0]), "○ your turn · 1 bg");
}

#[test]
fn seq_46_child_started_before_the_boundary_runs_across_stop() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "UserPromptSubmit", 0.0, |_| {});
    ck(&mut e, &ids[0], "SubagentStart", 1.0, |r| r.agent_id = Some("x".into()));
    ck(&mut e, &ids[0], "Stop", 5.0, |_| {});
    assert_eq!(show(&all(&e, 9.0)[0]), "○ your turn · 1 bg");
}

#[test]
fn child_started_before_an_older_end_is_dropped() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "SubagentStart", -5.0, |r| r.agent_id = Some("x".into()));
    ck(&mut e, &ids[0], "SessionEnd", 0.0, |_| {});
    ck(&mut e, &ids[0], "UserPromptSubmit", 1.0, |_| {});
    let got = e.idx.turn_receipts(&ids[0]).unwrap();
    assert_eq!(got.len(), 3, "start, the older End, and the Prompt");
    let rows = all(&e, 2.0);
    assert_eq!((rows[0].bg, rows[0].state), (0, LiveState::Working));
}

#[test]
fn seq_47_prompt_after_end_is_on_screen() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "SessionEnd", 0.0, |_| {});
    ck(&mut e, &ids[0], "UserPromptSubmit", 1.0, |_| {});
    let rows = all(&e, 2.0);
    assert_eq!((rows.len(), rows[0].state), (1, LiveState::Working));
}

#[test]
fn seq_32_old_pre_in_the_current_turn_is_returned_and_flagged() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "UserPromptSubmit", 0.0, |_| {});
    ck(&mut e, &ids[0], "PreToolUse", 300.0, |r| r.tool = Some("A".into()));
    let rows = all(&e, 1200.0);
    let v = render(&rows[0], 0);
    assert_eq!((show(&rows[0]), v.marks), ("⏳ A · 15m".into(), vec!["?"]));
}

#[test]
fn seq_24_dead_pid_is_gone_and_seq_25_reused_pid_is_gone() {
    let (mut e, ids) = claude_env(&["c1"]);
    ck(&mut e, &ids[0], "UserPromptSubmit", 0.0, |_| {});
    assert_eq!(fold_all(&e.idx, at(1.0), |_, _| true).unwrap().len(), 1);
    assert!(fold_all(&e.idx, at(1.0), |_, _| false).unwrap().is_empty(), "dead");
    let seen = std::cell::Cell::new(None);
    let rows = fold_all(&e.idx, at(1.0), |pid, start| {
        seen.set(Some((pid, start)));
        start == 1_700_000_001
    })
    .unwrap();
    assert!(rows.is_empty(), "same pid, different start time");
    assert_eq!(seen.get(), Some((4242, 1_700_000_000)));
}

#[test]
fn seq_26_alive_is_never_called_without_a_pid() {
    let (mut e, ids) = claude_env(&["c1"]);
    e.rcpt("claude", "UserPromptSubmit", at(0.0), |r| r.session_id = Some(ids[0].clone()));
    let rows = fold_all(&e.idx, at(3600.0), |_, _| panic!("no pid, no liveness check")).unwrap();
    let v = render(&rows[0], 0);
    assert_eq!((v.glyph, v.text.as_str(), v.marks), ("◌", "quiet", vec!["pid?"]));
}

#[test]
fn seq_48_same_process_supersede_for_claude_not_cursor_not_unknown_pid() {
    let (mut e, ids) = claude_env(&["c1", "c2"]);
    ck(&mut e, &ids[0], "UserPromptSubmit", 0.0, |_| {});
    ck(&mut e, &ids[0], "Stop", 10.0, |_| {});
    ck(&mut e, &ids[1], "UserPromptSubmit", 11.0, |_| {});
    let rows = all(&e, 12.0);
    assert_eq!(rows.iter().map(|r| r.session_id.as_str()).collect::<Vec<_>>(), [ids[1].as_str()]);

    // Unknown pids are never superseded.
    let (mut e, ids) = claude_env(&["c1", "c2"]);
    for (i, t) in [(0, 0.0), (1, 11.0)] {
        e.rcpt("claude", "UserPromptSubmit", at(t), |r| r.session_id = Some(ids[i].clone()));
    }
    assert_eq!(all(&e, 12.0).len(), 2);

    // Cursor: same process, both stay.
    let mut e = env();
    write_old(&e.home.join(".cursor/projects/w-a/agent-transcripts/a1/a1.jsonl"), CURSOR);
    write_old(&e.home.join(".cursor/projects/w-b/agent-transcripts/b1/b1.jsonl"), CURSOR);
    e.scan();
    let rows = e.idx.list_sessions(&SessionFilter { source: Some("cursor".into()), ..Default::default() }).unwrap();
    for (i, r) in rows.iter().enumerate() {
        e.rcpt("cursor", "beforeSubmitPrompt", at(i as f64), |x| {
            x.session_id = Some(r.id.clone());
            x.harness_pid = Some(77);
            x.harness_start = Some(1_700_000_000);
        });
    }
    assert_eq!(all(&e, 5.0).len(), 2);
}

#[test]
fn seq_37_receipt_only_cursor_row() {
    let mut e = env();
    let chat = |r: &mut Receipt| {
        r.native_session = Some("c1".into());
        r.cwd = Some("/w/proj".into());
    };
    e.rcpt("cursor", "beforeSubmitPrompt", at(0.0), chat);
    e.rcpt("cursor", "beforeShellExecution", at(1.0), |r| {
        chat(r);
        r.tool = Some("Shell".into());
    });
    let rows = all(&e, 2.0);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].title.as_str(), show(&rows[0]).as_str(), rows[0].context_tokens), ("proj", "▶ Shell · 1s", None));
}

#[test]
fn seq_38_a_session_row_takes_over_the_receipt_only_row() {
    let mut e = env();
    let chat = |r: &mut Receipt| r.native_session = Some("c1".into());
    e.rcpt("cursor", "beforeSubmitPrompt", at(0.0), chat);
    e.rcpt("cursor", "beforeShellExecution", at(1.0), |r| {
        chat(r);
        r.tool = Some("Shell".into());
    });
    assert!(all(&e, 2.0)[0].session_id.starts_with("cursor:rcpt:"));
    write_old(&e.home.join(".cursor/projects/w-proj/agent-transcripts/c1/c1.jsonl"), CURSOR);
    e.scan();
    let sess = e.session("cursor");
    let rows = all(&e, 2.0);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0].session_id.as_str(), show(&rows[0]).as_str()), (sess.id.as_str(), "▶ Shell · 1s"));
    assert_eq!((rows[0].title.as_str(), rows[0].context_tokens), (sess.title.as_str(), sess.context_tokens));
}

#[test]
fn claude_subagent_rows_map_by_exact_agent_file_id() {
    let (mut e, ids) = claude_env(&["c1"]);
    write_old(&e.home.join(".claude/projects/-w-claude/c1/subagents/agent-ab12.jsonl"), CLAUDE);
    e.scan();
    ck(&mut e, &ids[0], "UserPromptSubmit", 0.0, |_| {});
    ck(&mut e, &ids[0], "SubagentStart", 1.0, |r| r.agent_id = Some("ab12".into()));
    ck(&mut e, &ids[0], "PreToolUse", 2.0, |r| {
        r.agent_id = Some("ab12".into());
        r.tool = Some("Read".into());
        r.tool_use_id = Some("r".into());
    });
    let rows = all(&e, 3.0);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!((rows[1].parent_id.as_deref(), show(&rows[1]).as_str()), (Some(ids[0].as_str()), "▶ Read · 1s"));
    assert_eq!(show(&rows[0]), "● 1 bg");
}
