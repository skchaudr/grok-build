//! Converter tests on made-up Pi sessions.

use super::*;

fn at(sec: u64) -> String {
    format!("2026-01-01T00:00:{sec:02}.000Z")
}

fn msg(sec: u64, message: Value) -> Value {
    json!({ "type": "message", "id": format!("e{sec}"), "parentId": null, "timestamp": at(sec), "message": message })
}

fn user(sec: u64, text: &str) -> Value {
    msg(
        sec,
        json!({ "role": "user", "content": [{ "type": "text", "text": text }] }),
    )
}

fn assistant(sec: u64, dur: u64, blocks: Value) -> Value {
    msg(
        sec,
        json!({ "role": "assistant", "content": blocks, "durationMs": dur }),
    )
}

fn call(id: &str, name: &str, arguments: Value) -> Value {
    json!({ "type": "toolCall", "id": id, "name": name, "arguments": arguments })
}

fn result(sec: u64, id: &str, text: &str, is_error: bool, dur: u64, details: Value) -> Value {
    msg(
        sec,
        json!({ "role": "toolResult", "toolCallId": id, "toolName": "x", "content": [{ "type": "text", "text": text }], "isError": is_error, "durationMs": dur, "details": details }),
    )
}

fn run(lines: &[Value]) -> Vec<Fed> {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("s.jsonl");
    let body: Vec<String> = lines.iter().map(Value::to_string).collect();
    fs::write(&path, body.join("\n")).unwrap();
    convert(&path, "pi:s").unwrap()
}

fn calls(feed: &[Fed]) -> Vec<&acp::ToolCall> {
    feed.iter()
        .filter_map(|f| match &f.event {
            Event::Acp(acp::SessionUpdate::ToolCall(tc)) => Some(tc),
            _ => None,
        })
        .collect()
}

#[test]
fn each_pi_tool_gets_its_kind_and_the_message_parts_come_through() {
    let feed = run(&[
        json!({ "type": "session", "id": "s", "timestamp": at(0) }),
        user(1, "go"),
        assistant(
            5,
            3000,
            json!([
                { "type": "thinking", "thinking": "hmm" },
                { "type": "text", "text": "on it" },
                call("c1", "bash", json!({ "command": "ls -la\nsecond" })),
                call("c2", "read", json!({ "path": "/a/b.rs" })),
                call("c3", "edit", json!({ "path": "/a/b.rs", "edits": [
                    { "oldText": "one", "newText": "two" }, { "oldText": "x", "newText": "y" } ] })),
                call("c4", "write", json!({ "path": "/a/new.rs", "content": "fn main() {}" })),
                call("c5", "find", json!({ "pattern": "*.rs" })),
                call("c6", "ls", json!({ "path": "/a" })),
                call("c7", "workflow", json!({ "workflow": "wf", "action": "run" })),
                call("c8", "bash", json!({ "command": "false" })),
                call("c9", "bash", json!({ "command": "sleep 9" })),
            ]),
        ),
        result(6, "c1", "out", false, 15, Value::Null),
        result(6, "c8", "boom", true, 1, Value::Null),
    ]);
    assert!(matches!(
        feed[0].event,
        Event::Acp(acp::SessionUpdate::UserMessageChunk(_))
    ));
    assert!(matches!(
        feed[1].event,
        Event::Acp(acp::SessionUpdate::AgentThoughtChunk(_))
    ));
    assert_eq!(
        feed[1].meta.get("streamStartMs"),
        Some(&json!(ts_of(&json!({ "timestamp": at(2) })).unwrap()))
    );
    assert!(matches!(
        feed[2].event,
        Event::Acp(acp::SessionUpdate::AgentMessageChunk(_))
    ));
    let tcs = calls(&feed);
    let got: Vec<(&str, acp::ToolKind, &str)> = tcs
        .iter()
        .map(|t| (&*t.tool_call_id.0, t.kind, t.title.as_str()))
        .collect();
    use acp::ToolKind::*;
    assert_eq!(
        got,
        [
            ("c1", Execute, "ls -la"),
            ("c2", Read, "Read /a/b.rs"),
            ("c3#0", Edit, "/a/b.rs"),
            ("c3#1", Edit, "/a/b.rs"),
            ("c4", Edit, "/a/new.rs"),
            ("c5", Search, "find *.rs"),
            ("c6", Other, "ls /a"),
            ("c7", Other, "workflow: wf"),
            ("c8", Execute, "false"),
            ("c9", Execute, "sleep 9"),
        ]
    );
    let diff = |t: &acp::ToolCall| match &t.content[0] {
        acp::ToolCallContent::Diff(d) => (d.old_text.clone(), d.new_text.clone()),
        _ => panic!("diff"),
    };
    assert_eq!(diff(tcs[2]), (Some("one".into()), "two".into()));
    assert_eq!(diff(tcs[4]), (None, "fn main() {}".into()));
    assert_eq!(tcs[6].raw_input.as_ref().unwrap()["target_directory"], "/a");
    // Outcome: result present, errored, or none yet; the duration is the result's own.
    assert_eq!(tcs[0].status, acp::ToolCallStatus::Completed);
    assert_eq!(tcs[0].meta.as_ref().unwrap()["durationMs"], 15);
    assert_eq!(tcs[8].status, acp::ToolCallStatus::Failed);
    assert_eq!(tcs[9].status, acp::ToolCallStatus::InProgress);
}

#[test]
fn a_subagent_launch_is_a_spawn_and_a_finished_one_closes() {
    let feed = run(&[
        user(1, "go"),
        assistant(
            3,
            1000,
            json!([
                call(
                    "s1",
                    "subagent",
                    json!({ "agent": "scout", "task": "look around\nmore" })
                ),
                call(
                    "s2",
                    "subagent",
                    json!({ "agent": "scout", "task": "detached", "async": true })
                ),
                call("s3", "subagent", json!({ "action": "status", "id": "r2" })),
            ]),
        ),
        result(
            9,
            "s1",
            "found it",
            false,
            6000,
            json!({ "runId": "run-1" }),
        ),
        result(
            4,
            "s2",
            "Detached",
            false,
            5,
            json!({ "runId": "run-2", "asyncId": "run-2" }),
        ),
        result(4, "s3", "running", false, 1, Value::Null),
    ]);
    let kinds: Vec<&str> = feed
        .iter()
        .map(|f| match &f.event {
            Event::Xai(XaiSessionUpdate::SubagentSpawned { .. }) => "spawn",
            Event::Xai(XaiSessionUpdate::SubagentFinished { .. }) => "finish",
            Event::Acp(acp::SessionUpdate::ToolCall(_)) => "call",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["other", "spawn", "finish", "spawn", "call"]);
    let Event::Xai(XaiSessionUpdate::SubagentSpawned {
        child_session_id,
        subagent_type,
        description,
        parent_session_id,
        ..
    }) = &feed[1].event
    else {
        panic!("spawn")
    };
    assert_eq!(
        (
            child_session_id.as_str(),
            subagent_type.as_str(),
            description.as_str(),
            parent_session_id.as_str()
        ),
        ("run-1", "scout", "look around", "pi:s")
    );
    let Event::Xai(XaiSessionUpdate::SubagentFinished {
        status,
        duration_ms,
        output,
        ..
    }) = &feed[2].event
    else {
        panic!("finish")
    };
    assert_eq!(
        (status.as_str(), *duration_ms, output.as_deref()),
        ("completed", 6000, Some("found it"))
    );
}
