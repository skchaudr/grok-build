//! Converter tests on made-up Pi sessions.

use super::*;

fn at(sec: u64) -> String {
    format!("2026-01-01T00:00:{sec:02}.000Z")
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A root-less entry with a fresh id; `run` links it to the line before it.
fn msg(sec: u64, message: Value) -> Value {
    let n = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    json!({ "type": "message", "id": format!("e{n}"), "parentId": null, "timestamp": at(sec), "message": message })
}

/// The entry, hung under `parent` instead of the line before it.
fn under(parent: &Value, mut e: Value) -> Value {
    e["parentId"] = parent["id"].clone();
    e
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
    // A null parent means "the line before", as in a plain Pi session.
    let mut lines = lines.to_vec();
    for i in 1..lines.len() {
        if lines[i]["parentId"].is_null() {
            lines[i]["parentId"] = lines[i - 1]["id"].clone();
        }
    }
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
                call("c10", "grep", json!({ "path": "/a", "pattern": "fn main" })),
            ]),
        ),
        result(6, "c1", "out", false, 15, Value::Null),
        result(6, "c6", "b.rs\nnew.rs", false, 1, Value::Null),
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
            ("c10", Search, "grep fn main"),
        ]
    );
    let diff = |t: &acp::ToolCall| match &t.content[0] {
        acp::ToolCallContent::Diff(d) => (d.old_text.clone(), d.new_text.clone()),
        _ => panic!("diff"),
    };
    assert_eq!(diff(tcs[2]), (Some("one".into()), "two".into()));
    assert_eq!(diff(tcs[4]), (None, "fn main() {}".into()));
    assert_eq!(tcs[6].raw_input.as_ref().unwrap()["target_directory"], "/a");
    // The ListDir block reads its listing from a ListDir raw_output, not from content.
    let listing = serde_json::from_value::<ToolOutput>(tcs[6].raw_output.clone().unwrap()).unwrap();
    assert!(matches!(listing, ToolOutput::ListDir(ListDirOutput::Content(c)) if c.content == "b.rs\nnew.rs"));
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

#[test]
fn only_the_active_branch_is_shown_and_a_compaction_is_grok_s_row() {
    let root = user(1, "root question");
    let old_a = under(
        &root,
        assistant(
            2,
            100,
            json!([{ "type": "text", "text": "abandoned answer" }]),
        ),
    );
    let new_a = under(
        &root,
        assistant(3, 100, json!([{ "type": "text", "text": "kept answer" }])),
    );
    let mut compaction = json!({ "type": "compaction", "id": "k1", "parentId": new_a["id"], "timestamp": at(4),
        "summary": "x".repeat(400), "firstKeptEntryId": new_a["id"], "tokensBefore": 12000 });
    let tail = under(
        &compaction,
        assistant(
            5,
            100,
            json!([{ "type": "text", "text": "after compaction" }]),
        ),
    );
    compaction["parentId"] = new_a["id"].clone();
    // File order interleaves the branches: the abandoned one is written before the kept one.
    let feed = run(&[root, old_a, new_a, compaction, tail]);
    let texts: Vec<String> = feed
        .iter()
        .filter_map(|f| match &f.event {
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(c)) => match &c.content {
                acp::ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["kept answer", "after compaction"]);
    let rows: Vec<_> = feed
        .iter()
        .filter_map(|f| match &f.event {
            Event::Xai(XaiSessionUpdate::AutoCompactCompleted {
                tokens_before,
                tokens_after,
                ..
            }) => Some((*tokens_before, *tokens_after)),
            _ => None,
        })
        .collect();
    assert_eq!(rows, [(Some(12000), 100)]);
    // The row sits between the answer it follows and the one after it.
    let pos = feed
        .iter()
        .position(|f| {
            matches!(
                f.event,
                Event::Xai(XaiSessionUpdate::AutoCompactCompleted { .. })
            )
        })
        .unwrap();
    assert_eq!(pos, 2);
}

#[test]
fn codemode_nested_calls_nest_under_it_and_a_lane_prompt_is_a_user_message() {
    let mut lane = msg(0, Value::Null);
    lane.as_object_mut().unwrap().remove("message");
    lane["type"] = json!("custom_message");
    lane["customType"] = json!("lane-step");
    lane["content"] = json!("Lane stage: made-up");
    let nested = json!({ "complete": true, "calls": [
        { "id": "cm1/1", "name": "bash", "status": "ok", "arguments": { "command": "ls" }, "durationMs": 7 },
        { "id": "cm1/2", "name": "read", "status": "error", "arguments": { "path": "/a" }, "durationMs": 3 },
    ] });
    let old = json!({ "calls": [
        { "id": "cm2/1", "name": "read", "status": "ok", "args": "{\"path\":\"/b\"}", "durationMs": 1.6 },
    ] });
    let feed = run(&[
        lane,
        user(1, "go"),
        assistant(
            5,
            1000,
            json!([
                call("cm1", "codemode", json!({ "code": "x" })),
                call("cm2", "codemode", json!({ "code": "y" })),
            ]),
        ),
        {
            let mut r = result(6, "cm1", "done", false, 9, Value::Null);
            r["message"]["nestedCalls"] = nested;
            r
        },
        result(7, "cm2", "done", false, 2, old),
    ]);
    let Event::Acp(acp::SessionUpdate::UserMessageChunk(_)) = &feed[0].event else {
        panic!("lane prompt is not a user message");
    };
    let tcs = calls(&feed);
    let parent = |t: &acp::ToolCall| {
        t.meta
            .as_ref()
            .and_then(|m| m.get("pi")?.get("parentToolCallId"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    use acp::ToolKind::*;
    let got: Vec<_> = tcs
        .iter()
        .map(|t| (&*t.tool_call_id.0, t.kind, parent(t), t.status))
        .collect();
    use acp::ToolCallStatus::{Completed, Failed};
    assert_eq!(
        got,
        [
            ("cm1", Other, None, Completed),
            ("cm1/1", Execute, Some("cm1".into()), Completed),
            ("cm1/2", Read, Some("cm1".into()), Failed),
            ("cm2", Other, None, Completed),
            ("cm2/1", Read, Some("cm2".into()), Completed),
        ]
    );
    assert_eq!(tcs[1].title, "ls");
    assert_eq!(tcs[4].title, "Read /b");
}

#[test]
fn dpi_records_go_through_the_pi_converter() {
    let t = 1_790_000_000_000i64;
    let records = [
        json!({ "kind": "pi.user", "model": [{ "role": "user", "content": "go", "timestamp": t }] }),
        json!({ "kind": "pi.assistant", "model": [{ "role": "assistant", "timestamp": t + 2000, "content": [call("c1", "bash", json!({ "command": "ls" }))] }] }),
        json!({ "kind": "pi.tool-result", "model": [{ "role": "toolResult", "toolCallId": "c1", "toolName": "bash", "isError": false, "timestamp": t + 3000, "content": [{ "type": "text", "text": "out" }] }] }),
        json!({ "kind": "pi.compaction", "model": [{ "role": "user", "timestamp": t + 4000, "content": [{ "type": "text",
            "text": "The conversation history before this point was compacted into the following summary:\n\n<summary>\nRan ls.\n</summary>" }] }] }),
        json!({ "kind": "pi.reset" }),
    ];
    let feed = convert_records(&records, "dpi:s:7");
    assert!(matches!(feed[0].event, Event::Acp(acp::SessionUpdate::UserMessageChunk(_))));
    let tcs = calls(&feed);
    assert_eq!((tcs.len(), tcs[0].title.as_str()), (1, "ls"));
    assert_eq!(feed[0].meta.get("turnStartMs"), Some(&json!(t)));
    let users = feed.iter().filter(|f| matches!(f.event, Event::Acp(acp::SessionUpdate::UserMessageChunk(_)))).count();
    assert_eq!(users, 1, "the compaction summary is not a prompt");
    assert!(matches!(
        &feed.last().unwrap().event,
        Event::Xai(XaiSessionUpdate::AutoCompactCompleted { summary_preview: Some(p), .. }) if p == "Ran ls."
    ));
    assert!(convert_dpi("no-hash", "x").is_err());
}
