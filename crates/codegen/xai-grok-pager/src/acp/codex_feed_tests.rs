//! Converter tests on made-up rollouts.

use super::*;

fn done(ts: &str, item: Value, started: Option<i64>, completed: i64) -> Value {
    let mut p = json!({ "type": "item_completed", "thread_id": "t", "turn_id": "turn-1", "item": item, "completed_at_ms": completed });
    if let Some(s) = started {
        p["started_at_ms"] = json!(s);
    }
    json!({ "timestamp": ts, "type": "event_msg", "payload": p })
}

const T: &str = "2026-01-01T00:00:00.000Z";

fn write(dir: &Path, name: &str, lines: &[Value], zst: bool) -> std::path::PathBuf {
    let body = lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let path = dir.join(name);
    let bytes = if zst {
        zstd::encode_all(body.as_bytes(), 1).unwrap()
    } else {
        body.into_bytes()
    };
    fs::write(&path, bytes).unwrap();
    path
}

fn calls(feed: &[Fed]) -> Vec<&acp::ToolCall> {
    feed.iter()
        .filter_map(|f| match &f.event {
            Event::Acp(acp::SessionUpdate::ToolCall(tc)) => Some(tc),
            _ => None,
        })
        .collect()
}

fn text(c: &acp::ContentChunk) -> &str {
    let acp::ContentBlock::Text(t) = &c.content else {
        panic!("text")
    };
    &t.text
}

#[test]
fn a_zst_rollout_gives_messages_thought_and_command_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let cmd = |id: &str, script: &str, code: i64, out: &str| {
        json!({ "type": "CommandExecution", "id": id, "command": ["/bin/zsh", "-lc", script], "cwd": "file:///tmp/proj",
                "status": if code == 0 { "completed" } else { "failed" }, "aggregated_output": out, "exit_code": code,
                "duration": { "secs": 1, "nanos": 500_000_000 } })
    };
    let path = write(
        tmp.path(),
        "rollout-x.jsonl.zst",
        &[
            json!({ "timestamp": T, "type": "session_meta", "payload": {} }),
            done(
                T,
                json!({ "type": "UserMessage", "id": "i1", "content": [{ "type": "text", "text": "list files" }] }),
                None,
                1_000,
            ),
            done(
                T,
                json!({ "type": "UserMessage", "id": "i0", "content": [{ "type": "text", "text": "The following is the Codex agent history whose request ..." }] }),
                None,
                1_100,
            ),
            done(
                T,
                json!({ "type": "Reasoning", "id": "r1", "summary_text": [], "raw_content": [] }),
                Some(1_200),
                3_200,
            ),
            done(T, cmd("c1", "ls\nwc -l", 0, "a\nb\n"), Some(4_000), 5_500),
            done(T, cmd("c2", "false", 1, "boom"), Some(6_000), 6_100),
            done(
                T,
                json!({ "type": "AgentMessage", "id": "i2", "content": [{ "type": "Text", "text": "Done." }] }),
                None,
                7_000,
            ),
        ],
        true,
    );
    let feed = convert(&path, "codex:s1").unwrap();
    let shape: Vec<_> = feed
        .iter()
        .map(|f| match &f.event {
            Event::Acp(acp::SessionUpdate::UserMessageChunk(_)) => "user",
            Event::Acp(acp::SessionUpdate::AgentThoughtChunk(_)) => "thought",
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(_)) => "text",
            Event::Acp(acp::SessionUpdate::ToolCall(_)) => "call",
            _ => "other",
        })
        .collect();
    assert_eq!(
        shape,
        ["user", "thought", "call", "call", "text"],
        "the review prompt is not a user turn"
    );
    let Event::Acp(acp::SessionUpdate::UserMessageChunk(u)) = &feed[0].event else {
        panic!()
    };
    assert_eq!(text(u), "list files");
    assert_eq!(feed[0].meta["promptId"], "turn-1");
    assert_eq!(feed[0].meta["eventId"], "codex:s1-1");
    let Event::Acp(acp::SessionUpdate::AgentThoughtChunk(t)) = &feed[1].event else {
        panic!()
    };
    assert_eq!(
        text(t),
        "\u{200b}",
        "encrypted reasoning shows as a Thought with no body"
    );
    assert_eq!(feed[1].meta["streamStartMs"], 1_200);
    assert_eq!(feed[1].meta["agentTimestampMs"], 3_200);

    let cs = calls(&feed);
    assert_eq!(cs[0].kind, acp::ToolKind::Execute);
    assert_eq!(cs[0].status, acp::ToolCallStatus::Completed);
    assert_eq!(cs[0].title, "ls");
    assert_eq!(cs[0].raw_input.as_ref().unwrap()["command"], "ls\nwc -l");
    assert_eq!(cs[0].raw_input.as_ref().unwrap()["workdir"], "/tmp/proj");
    assert_eq!(cs[0].meta.as_ref().unwrap()["durationMs"], 1500);
    assert_eq!(
        crate::acp::tracker::content_blocks_text(&cs[0].content),
        "a\nb\n"
    );
    assert_eq!(cs[1].status, acp::ToolCallStatus::Failed);
    assert_eq!(cs[1].raw_output.as_ref().unwrap()["exit_code"], 1);
}

#[test]
fn file_changes_become_edits_with_a_diff_per_hunk() {
    let tmp = tempfile::tempdir().unwrap();
    let diff = "@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -9,2 +9,2 @@\n x\n-y\n+Y\n";
    let path = write(
        tmp.path(),
        "rollout-y.jsonl",
        &[
            done(
                T,
                json!({ "type": "FileChange", "id": "f1", "status": "completed", "changes": { "/p/a.rs": { "type": "update", "unified_diff": diff, "move_path": null } } }),
                None,
                2_000,
            ),
            done(
                T,
                json!({ "type": "FileChange", "id": "f2", "status": "completed", "changes": { "/p/new.rs": { "type": "add", "content": "fn main() {}\n", "unified_diff": "" } } }),
                None,
                3_000,
            ),
            done(
                T,
                json!({ "type": "FileChange", "id": "f3", "status": "completed", "changes": { "/p/old.rs": { "type": "delete", "content": "gone\n", "unified_diff": "" } } }),
                None,
                4_000,
            ),
            done(
                T,
                json!({ "type": "FileChange", "id": "f4", "status": "failed", "stderr": "Failed to find expected lines", "changes": { "/p/a.rs": { "type": "update", "unified_diff": diff, "move_path": null } } }),
                Some(4_500),
                5_000,
            ),
        ],
        false,
    );
    let feed = convert(&path, "codex:s2").unwrap();
    let cs = calls(&feed);
    let diff_of = |tc: &acp::ToolCall| match &tc.content[0] {
        acp::ToolCallContent::Diff(d) => (d.old_text.clone(), d.new_text.clone()),
        _ => panic!("diff"),
    };
    assert_eq!(
        cs.len(),
        5,
        "two hunks, add, delete, and the failed change once"
    );
    assert!(cs.iter().all(|c| c.kind == acp::ToolKind::Edit));
    assert_eq!(
        diff_of(cs[0]),
        (Some("a\nb\nc\n".into()), "a\nB\nc\n".into())
    );
    assert_eq!(diff_of(cs[1]), (Some("x\ny\n".into()), "x\nY\n".into()));
    assert_ne!(cs[0].tool_call_id, cs[1].tool_call_id);
    assert_eq!(diff_of(cs[2]), (None, "fn main() {}\n".into()));
    assert_eq!(cs[2].raw_input.as_ref().unwrap()["variant"], "Write");
    assert_eq!(diff_of(cs[3]), (Some("gone\n".into()), String::new()));
    assert_eq!(cs[4].status, acp::ToolCallStatus::Failed);
    assert_eq!(
        crate::acp::tracker::content_blocks_text(&cs[4].content),
        "Failed to find expected lines"
    );
}

#[test]
fn a_spawn_call_becomes_a_subagent_row_per_child_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(
        tmp.path(),
        "rollout-s.jsonl",
        &[
            done(T, json!({ "type": "CollabAgentToolCall", "id": "c1", "tool": "spawnAgent", "status": "completed",
                "sender_thread_id": "parent", "receiver_thread_ids": ["child-1"], "prompt": "review the diff\nand report" }), None, 5),
            done(T, json!({ "type": "CollabAgentToolCall", "id": "c2", "tool": "wait", "status": "completed",
                "sender_thread_id": "parent", "receiver_thread_ids": ["child-1"] }), None, 6),
        ],
        false,
    );
    let feed = convert(&path, "parent").unwrap();
    let spawns: Vec<_> = feed
        .iter()
        .filter_map(|f| match &f.event {
            Event::Xai(XaiSessionUpdate::SubagentSpawned { child_session_id, parent_session_id, description, .. }) => {
                Some((child_session_id.as_str(), parent_session_id.as_str(), description.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(spawns, vec![("child-1", "parent", "review the diff")], "wait is not a spawn");
}
