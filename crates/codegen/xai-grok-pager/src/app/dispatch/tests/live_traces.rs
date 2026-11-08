//! The live hook view: what the worker sends after the load window reaches the screen.

use super::*;
use crate::acp::live_feed::{self, Tail, TraceLive, Update};
use crate::app::dispatch::standalone_traces::{drain_live, message};
use xai_grok_traces::live::LiveReceipt;
use xai_grok_traces::merge;

fn receipt(event: &str, id: &str, at_ms: i64, detail: &str) -> LiveReceipt {
    LiveReceipt { event: event.into(), tool: Some("Bash".into()), tool_use_id: Some(id.into()), at_ms, detail: Some(detail.into()), ..Default::default() }
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

    // What open_live_in does: replay the load inside a load window, then close it and start the tail.
    get_active_agent_mut(&mut app).unwrap().begin_replay_window();
    for fed in live_feed::to_feds(sid, &pieces) {
        crate::app::acp_handler::handle(message(sid, fed), &mut app);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let agent = get_active_agent_mut(&mut app).unwrap();
        agent.session.loading_replay = false;
        assert!(agent.scrollback.iter_entries().any(|(_, e)| e.is_running), "the call opens running");
        let running = merger.running().map(|(id, name)| (id, name, Instant::now()));
        agent.trace_live = Some(TraceLive { tail: Tail::from_rx(rx), session_id: sid.into(), life: "alive".into(), running, phoenix_offline: false });
    }

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
