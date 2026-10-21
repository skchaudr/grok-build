//! Replays real sessions from a copy of the index through the live fold. Ignored by default:
//! `GROK_TRACES_REPLAY_DB=/path/to/copy.sqlite cargo test -p xai-grok-traces --test live_replay -- --ignored --nocapture`

use xai_grok_traces::live::{Ev, fold_children, fold_session, norm, render, strict_ids};
use xai_grok_traces::{SessionFilter, TraceIndex};

#[test]
#[ignore]
fn replay_every_session_with_receipts() {
    let path = std::env::var("GROK_TRACES_REPLAY_DB").expect("GROK_TRACES_REPLAY_DB points at a copy of the index");
    let idx = TraceIndex::open(std::path::Path::new(&path)).unwrap();
    let sessions = idx.list_sessions(&SessionFilter { limit: 1_000_000, ..Default::default() }).unwrap();
    let (mut checked, mut agents_seen, mut agents_mapped) = (0, 0, 0);
    for s in sessions.iter().filter(|s| matches!(s.source.as_str(), "claude" | "pi" | "cursor")) {
        let receipts = idx.session_receipts(&s.id).unwrap();
        if receipts.is_empty() {
            continue;
        }
        let kids = idx.children(&s.id).unwrap();
        let strict = strict_ids(&s.source);
        println!("== {} {} {}", s.source, s.id, s.title);
        let mut last = String::new();
        for k in 0..receipts.len() {
            let now = receipts[k].at_ms;
            let prefix = &receipts[..=k];
            let f = fold_session(&s.source, s, prefix, &kids, now, strict);
            let line = match &f.row {
                None => "gone".to_string(),
                Some(row) => {
                    let v = render(row, 0);
                    format!("{} {}", v.glyph, v.text)
                }
            };
            if line != last {
                println!("{now} {line}");
                last = line;
            }
            if s.source == "claude" {
                for t in &f.open_tools {
                    let Some(id) = &t.id else { continue };
                    let closed = prefix.iter().any(|r| {
                        r.at_ms >= t.at_ms && r.tool_use_id.as_deref() == Some(id) && norm(&s.source, &r.event, r.detail.as_deref()) == Some(Ev::Post)
                    });
                    assert!(!closed, "{} {}: open tool {id} persists after its Post at {now}", s.source, s.id);
                }
            }
            if let Some(row) = &f.row {
                let _ = fold_children(&s.source, s, prefix, &kids, &f.open_children, row.age_ms + now, strict);
            }
        }
        checked += 1;
        if s.source == "claude" {
            let mut ids: Vec<&str> = receipts.iter().filter_map(|r| r.agent_id.as_deref()).collect();
            ids.sort_unstable();
            ids.dedup();
            for a in ids {
                agents_seen += 1;
                let want = format!("agent-{a}");
                if kids.iter().any(|k| k.native_id.as_deref().and_then(|n| n.rsplit('/').next()) == Some(want.as_str())) {
                    agents_mapped += 1;
                }
            }
        }
    }
    println!("sessions replayed: {checked}; hook agent_ids: {agents_seen}, matching a child file agent-<id>: {agents_mapped}");
}
