//! The live hook view: what the worker sends after the load window reaches the screen.

use super::*;
use crate::acp::live_feed::{self, Tail, TraceLive, Update};
use crate::app::dispatch::standalone_traces::{drain_live, finish_live_load, message};
use crate::scrollback::blocks::ToolCallBlock;
use xai_grok_traces::live::LiveReceipt;
use xai_grok_traces::merge;

fn receipt(event: &str, id: &str, at_ms: i64, detail: &str) -> LiveReceipt {
    LiveReceipt { event: event.into(), tool: Some("Bash".into()), tool_use_id: Some(id.into()), at_ms, detail: Some(detail.into()), ..Default::default() }
}

/// What open_live_in does: replay the load inside a load window, close it, and hand the view a tail.
fn open(app: &mut AppView, pieces: &[merge::Piece], merger: &merge::Merger) -> std::sync::mpsc::Sender<Update> {
    let sid = "test-session";
    get_active_agent_mut(app).unwrap().begin_replay_window();
    for fed in live_feed::to_feds(sid, pieces) {
        crate::app::acp_handler::handle(message(sid, fed), app);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let agent = get_active_agent_mut(app).unwrap();
    finish_live_load(agent);
    let running = merger.running().map(|(id, name)| (id, name, Instant::now()));
    agent.trace_live = Some(TraceLive { tail: Tail::from_rx(rx), session_id: sid.into(), life: "alive".into(), running, phoenix_offline: false });
    tx
}

fn commands(app: &mut AppView) -> Vec<(String, Option<String>)> {
    let agent = get_active_agent_mut(app).unwrap();
    agent
        .scrollback
        .iter_entries()
        .filter_map(|(_, e)| match &e.block {
            RenderBlock::ToolCall(ToolCallBlock::Execute(x)) => Some((x.command.clone(), x.description.clone())),
            _ => None,
        })
        .collect()
}

/// Session 9643f8cd: the rtk hook rewrote the command between Pre and Post, and the result landed after the view
/// opened. The completion and every later call come through `drain_live`, after the load window has closed.
#[test]
fn live_traces_drain_applies_calls_that_finish_after_the_load_window() {
    let mut app = test_app_with_agent();
    let sid = "test-session";
    let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_C","name":"Bash","input":{"command":"cd /r; cat docs/leader.md","description":"Read leader external-agents proof and ACP matrix"}}]}}"#;
    let mut receipts = vec![
        receipt("PreToolUse", "toolu_C", 1_000, "cd /r; cat docs/leader.md"),
        receipt("PostToolUse", "toolu_C", 1_118, "cd /r; rtk read docs/leader.md"),
    ];
    let (pieces, mut merger, _) = merge::load("claude", call, &receipts, Ok(&[]));

    let tx = open(&mut app, &pieces, &merger);
    assert!(get_active_agent_mut(&mut app).unwrap().scrollback.iter_entries().any(|(_, e)| e.is_running), "the call opens running");

    let rest = r##"{"type":"user","message":{"content":[{"tool_use_id":"toolu_C","type":"tool_result","content":"# Remote external ACP proof"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_L","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_L","content":"ok"}]}}"##;
    receipts.extend([receipt("PreToolUse", "toolu_L", 2_000, "ls"), receipt("PostToolUse", "toolu_L", 2_050, "ls")]);
    let (next, _) = merger.ingest("claude", &format!("{call}\n{rest}"), &receipts, Ok(&[]));
    tx.send(Update { events: live_feed::to_feds(sid, &next), life: "alive".into(), running: merger.running(), phoenix_offline: false }).unwrap();
    assert!(drain_live(&mut app));

    let agent = get_active_agent_mut(&mut app).unwrap();
    assert_eq!(agent.scrollback.len(), 2, "the stuck call and the one after it");
    assert!(agent.scrollback.iter_entries().all(|(_, e)| !e.is_running), "nothing stays running");
    assert!(agent.trace_live.as_ref().unwrap().running.is_none());
}

/// Session 40e362e8: rows opened from receipts (a subagent's calls, a call the transcript has not written yet) showed
/// "Run Bash call" forever, a signature-only thought showed "Thought for 0.0s" and kept running, and the view opened
/// above the tail.
#[test]
fn live_traces_rows_carry_the_receipt_label_then_the_description_and_open_at_the_tail() {
    let mut app = test_app_with_agent();
    let sid = "test-session";
    let sub = |r: LiveReceipt| LiveReceipt { agent_id: Some("a7c7757506b19c0c4".into()), ..r };
    let thought = r#"{"type":"assistant","timestamp":"2026-11-08T23:01:54.867Z","thinkingDurationMs":1193,"message":{"content":[{"type":"thinking","thinking":"","signature":"s"}]}}"#;
    let mut receipts = vec![
        sub(receipt("PreToolUse", "toolu_S", 1_000, "git status --short")),
        receipt("PreToolUse", "toolu_C", 2_000, "herdr pane send-keys w3E:p8 Enter"),
    ];
    let (pieces, mut merger, _) = merge::load("claude", thought, &receipts, Ok(&[]));
    let tx = open(&mut app, &pieces, &merger);
    assert_eq!(
        commands(&mut app),
        [("git status --short".to_string(), None), ("herdr pane send-keys w3E:p8 Enter".to_string(), None)],
        "before the transcript has them, rows show the receipt's command"
    );
    {
        let agent = get_active_agent_mut(&mut app).unwrap();
        let thought = agent.scrollback.iter_entries().find_map(|(_, e)| match &e.block {
            RenderBlock::Thinking(t) => Some((e.is_running, t.elapsed_time_ms())),
            _ => None,
        });
        assert_eq!(thought, Some((false, Some(1193))), "the thought is finished and timed from the transcript");
        agent.scrollback.prepare_layout(80, 10);
        assert!(agent.scrollback.is_follow_mode());
        assert_eq!(agent.scrollback.selected(), Some(agent.scrollback.len() - 1), "the view opens on the last row");
    }

    let rest = r#"{"type":"assistant","timestamp":"2026-11-08T23:01:55.862Z","message":{"content":[{"type":"tool_use","id":"toolu_C","name":"Bash","input":{"command":"herdr pane send-keys w3E:p8 Enter","description":"Open the session in grok view"}}]}}
{"type":"user","timestamp":"2026-11-08T23:01:56.100Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_C","content":"ok"}]}}
{"type":"assistant","timestamp":"2026-11-08T23:01:57.000Z","message":{"content":[{"type":"text","text":"Opened."}]}}"#;
    receipts.extend([sub(receipt("PostToolUse", "toolu_S", 1_300, "git status --short")), receipt("PostToolUse", "toolu_C", 2_400, "herdr pane send-keys w3E:p8 Enter")]);
    let (next, _) = merger.ingest("claude", &format!("{thought}\n{rest}"), &receipts, Ok(&[]));
    tx.send(Update { events: live_feed::to_feds(sid, &next), life: "alive".into(), running: merger.running(), phoenix_offline: false }).unwrap();
    assert!(drain_live(&mut app));
    assert_eq!(
        commands(&mut app),
        [
            ("git status --short".to_string(), None),
            ("herdr pane send-keys w3E:p8 Enter".to_string(), Some("Open the session in grok view".to_string())),
        ],
        "the transcript's description back-fills the row"
    );
    let agent = get_active_agent_mut(&mut app).unwrap();
    assert!(agent.scrollback.iter_entries().all(|(_, e)| !e.is_running), "nothing stays running");
    agent.scrollback.prepare_layout(80, 10);
    assert!(agent.scrollback.is_follow_mode(), "still following the tail");
}
