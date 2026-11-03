//! Converter tests on made-up transcripts.

use super::*;

fn at(sec: u64) -> String {
    format!("2026-01-01T00:00:{sec:02}.000Z")
}

fn user(sec: u64, text: &str) -> Value {
    json!({ "type": "user", "timestamp": at(sec), "promptId": "p-1", "message": { "role": "user", "content": text } })
}

fn assistant(sec: u64, msg_id: &str, block: Value) -> Value {
    json!({ "type": "assistant", "timestamp": at(sec), "message": { "id": msg_id, "role": "assistant", "content": [block] } })
}

fn tool_use(sec: u64, id: &str, name: &str, input: Value) -> Value {
    assistant(sec, &format!("m-{id}"), json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
}

fn result(sec: u64, id: &str, text: &str, is_error: bool, tur: Value) -> Value {
    json!({
        "type": "user", "timestamp": at(sec), "toolUseResult": tur,
        "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id, "content": text, "is_error": is_error }] }
    })
}

/// Writes `<tmp>/proj/<name>.jsonl`; the session's own directory is `<tmp>/proj/<name>/`.
fn write(tmp: &Path, name: &str, lines: &[Value]) -> PathBuf {
    let dir = tmp.join("proj");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.jsonl"));
    let body: Vec<String> = lines.iter().map(Value::to_string).collect();
    fs::write(&path, body.join("\n")).unwrap();
    path
}

fn run(lines: &[Value]) -> Vec<Fed> {
    let tmp = tempfile::tempdir().unwrap();
    convert(&write(tmp.path(), "s1", lines), "claude:s1").unwrap()
}

fn calls(feed: &[Fed]) -> Vec<&acp::ToolCall> {
    feed.iter()
        .filter_map(|f| match &f.event {
            Event::Acp(acp::SessionUpdate::ToolCall(tc)) => Some(tc),
            _ => None,
        })
        .collect()
}

fn shape(feed: &[Fed]) -> Vec<&'static str> {
    feed.iter()
        .map(|f| match &f.event {
            Event::Acp(acp::SessionUpdate::UserMessageChunk(_)) => "user",
            Event::Acp(acp::SessionUpdate::AgentThoughtChunk(_)) => "thought",
            Event::Acp(acp::SessionUpdate::AgentMessageChunk(_)) => "text",
            Event::Acp(acp::SessionUpdate::ToolCall(_)) => "call",
            Event::Acp(acp::SessionUpdate::Plan(_)) => "plan",
            Event::Acp(_) => "other",
            Event::Xai(XaiSessionUpdate::SubagentSpawned { .. }) => "spawn",
            Event::Xai(XaiSessionUpdate::SubagentFinished { .. }) => "finish",
            Event::Xai(_) => "xai",
        })
        .collect()
}

#[test]
fn a_call_is_filed_under_the_assistant_turn_with_its_real_duration() {
    let feed = run(&[
        user(0, "list the files"),
        assistant(1, "m1", json!({ "type": "thinking", "thinking": "", "signature": "sig" })),
        assistant(2, "m1", json!({ "type": "text", "text": "Listing." })),
        tool_use(3, "t1", "Bash", json!({ "command": "ls -la", "description": "List files" })),
        result(5, "t1", "a.txt\nb.txt", false, Value::Null),
        user(6, "thanks"),
        assistant(7, "m3", json!({ "type": "text", "text": "Done." })),
    ]);
    assert_eq!(shape(&feed), ["user", "thought", "text", "call", "user", "text"]);
    let tc = calls(&feed)[0];
    assert_eq!(tc.kind, acp::ToolKind::Execute);
    assert_eq!(tc.status, acp::ToolCallStatus::Completed);
    assert_eq!(tc.title, "List files");
    assert_eq!(tc.raw_input.as_ref().unwrap()["command"], "ls -la");
    assert_eq!(tc.meta.as_ref().unwrap()["durationMs"], 2000);
    assert_eq!(crate::acp::tracker::content_blocks_text(&tc.content), "a.txt\nb.txt");
}

#[test]
fn every_event_is_a_numbered_replay_with_the_prompt_id() {
    let feed = run(&[user(0, "hi"), assistant(1, "m1", json!({ "type": "text", "text": "hello" }))]);
    assert_eq!(feed[0].meta["isReplay"], true);
    assert_eq!(feed[0].meta["eventId"], "claude:s1-1");
    assert_eq!(feed[1].meta["eventId"], "claude:s1-2");
    assert_eq!(feed[1].meta["promptId"], "p-1");
    assert!(feed[1].meta["agentTimestampMs"].as_i64().unwrap() > 0);
}

#[test]
fn empty_thinking_keeps_its_duration_and_has_no_body() {
    let mut line = assistant(4, "m1", json!({ "type": "thinking", "thinking": "", "signature": "sig" }));
    line["thinkingDurationMs"] = json!(2600);
    let feed = run(&[user(0, "q"), line]);
    let Event::Acp(acp::SessionUpdate::AgentThoughtChunk(c)) = &feed[1].event else { panic!("thought") };
    let acp::ContentBlock::Text(t) = &c.content else { panic!("text") };
    assert!(t.text.trim().is_empty());
    let end = feed[1].meta["agentTimestampMs"].as_i64().unwrap();
    assert_eq!(end - feed[1].meta["streamStartMs"].as_i64().unwrap(), 2600);
}

#[test]
fn tools_map_to_grok_kinds() {
    let uses = [
        ("Read", json!({ "file_path": "/w/a.rs" })),
        ("Grep", json!({ "pattern": "fn main" })),
        ("Glob", json!({ "pattern": "**/*.rs" })),
        ("Edit", json!({ "file_path": "/w/a.rs", "old_string": "one\n", "new_string": "two\n" })),
        ("Write", json!({ "file_path": "/w/b.rs", "content": "fresh\n" })),
        ("WebFetch", json!({ "url": "https://example.test/x" })),
        ("WebSearch", json!({ "query": "rust sqlite" })),
        ("NotebookEdit", json!({ "notebook_path": "/w/n.ipynb" })),
    ];
    let lines: Vec<Value> = uses.iter().enumerate().map(|(i, (n, input))| tool_use(i as u64, &format!("t{i}"), n, input.clone())).collect();
    let feed = run(&lines);
    let got: Vec<_> = calls(&feed).iter().map(|c| c.kind).collect();
    use acp::ToolKind::*;
    assert_eq!(got, [Read, Search, Search, Edit, Edit, Fetch, Search, Other]);
    let c = calls(&feed);
    assert_eq!(c[5].title, "Fetch: https://example.test/x");
    assert_eq!(c[6].title, "Web search: rust sqlite");
    assert_eq!(c[7].title, "NotebookEdit: /w/n.ipynb", "an unmapped tool keeps its name and input");
    assert_eq!(c[7].raw_input.as_ref().unwrap()["notebook_path"], "/w/n.ipynb");
    assert!(c.iter().all(|c| c.status == acp::ToolCallStatus::InProgress), "no result yet");
}

#[test]
fn edits_carry_a_diff_and_a_write_is_creating() {
    let feed = run(&[
        tool_use(0, "e", "Edit", json!({ "file_path": "/w/a.rs", "old_string": "one\n", "new_string": "two\n" })),
        result(1, "e", "updated", false, Value::Null),
        tool_use(2, "w", "Write", json!({ "file_path": "/w/b.rs", "content": "fresh\n" })),
        result(3, "w", "created", false, Value::Null),
    ]);
    let c = calls(&feed);
    let acp::ToolCallContent::Diff(d) = &c[0].content[0] else { panic!("diff") };
    assert_eq!((d.old_text.as_deref(), d.new_text.as_str()), (Some("one\n"), "two\n"));
    let acp::ToolCallContent::Diff(d) = &c[1].content[0] else { panic!("diff") };
    assert_eq!((d.old_text.as_deref(), d.new_text.as_str()), (None, "fresh\n"));
    assert_eq!(c[1].raw_input.as_ref().unwrap()["variant"], "Write");
    assert!(c[0].raw_input.as_ref().unwrap().get("variant").is_none());
}

#[test]
fn multiedit_is_one_edit_per_replacement() {
    let feed = run(&[tool_use(
        0,
        "m",
        "MultiEdit",
        json!({ "file_path": "/w/a.rs", "edits": [{ "old_string": "a", "new_string": "b" }, { "old_string": "c", "new_string": "d" }] }),
    )]);
    let c = calls(&feed);
    assert_eq!(c.len(), 2);
    assert_ne!(c[0].tool_call_id, c[1].tool_call_id);
    assert!(c.iter().all(|c| c.kind == acp::ToolKind::Edit));
}

#[test]
fn an_error_result_fails_the_call_and_keeps_the_message() {
    let feed = run(&[tool_use(0, "t", "Bash", json!({ "command": "false" })), result(1, "t", "Exit code 1\nboom", true, Value::Null)]);
    let tc = calls(&feed)[0];
    assert_eq!(tc.status, acp::ToolCallStatus::Failed);
    assert!(crate::acp::tracker::content_blocks_text(&tc.content).contains("boom"));
}

#[test]
fn a_failed_edit_keeps_the_message_and_no_diff() {
    let feed = run(&[
        tool_use(0, "e", "Edit", json!({ "file_path": "/w/a.rs", "old_string": "x", "new_string": "y" })),
        result(1, "e", "String to replace not found", true, Value::Null),
    ]);
    let tc = calls(&feed)[0];
    assert!(tc.content.iter().all(|c| !matches!(c, acp::ToolCallContent::Diff(_))));
    assert_eq!(crate::acp::tracker::content_blocks_text(&tc.content), "String to replace not found");
}

#[test]
fn a_read_shows_the_file_text_and_its_range() {
    let tur = json!({ "file": { "filePath": "/w/a.rs", "content": "alpha\nbeta", "numLines": 2, "startLine": 10, "totalLines": 40 } });
    let feed = run(&[tool_use(0, "r", "Read", json!({ "file_path": "/w/a.rs", "offset": 10, "limit": 2 })), result(1, "r", "    10→alpha\n    11→beta", false, tur)]);
    let tc = calls(&feed)[0];
    assert_eq!(crate::acp::tracker::content_blocks_text(&tc.content), "alpha\nbeta");
    let raw = tc.raw_output.as_ref().unwrap();
    assert_eq!(raw["totalLines"], 40);
    assert_eq!((raw["range"]["start"].as_i64(), raw["range"]["end"].as_i64()), (Some(10), Some(11)));
}

#[test]
fn a_search_carries_its_files() {
    let tur = json!({ "filenames": ["/w/a.rs", "/w/b.rs"], "numFiles": 2 });
    let feed = run(&[tool_use(0, "g", "Grep", json!({ "pattern": "x" })), result(1, "g", "Found 2 files", false, tur)]);
    let raw = calls(&feed)[0].raw_output.as_ref().unwrap();
    assert_eq!(raw["type"], "GrepSearch");
    assert_eq!(raw["match_count"], 2);
}

#[test]
fn todowrite_is_a_plan() {
    let feed = run(&[tool_use(
        0,
        "p",
        "TodoWrite",
        json!({ "todos": [{ "content": "one", "status": "completed" }, { "content": "two", "status": "in_progress" }, { "content": "three", "status": "pending" }] }),
    )]);
    assert_eq!(shape(&feed), ["plan"]);
    let Event::Acp(acp::SessionUpdate::Plan(p)) = &feed[0].event else { panic!("plan") };
    let status: Vec<_> = p.entries.iter().map(|e| e.status.clone()).collect();
    assert_eq!(status, [acp::PlanEntryStatus::Completed, acp::PlanEntryStatus::InProgress, acp::PlanEntryStatus::Pending]);
}

#[test]
fn a_task_is_a_subagent_row_linked_by_its_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(
        tmp.path(),
        "s1",
        &[
            tool_use(0, "toolu_a", "Agent", json!({ "description": "scan src", "subagent_type": "explore", "prompt": "go" })),
            result(9, "toolu_a", "found three files", false, json!({ "status": "completed", "agentId": "zz9", "totalToolUseCount": 4, "totalDurationMs": 8000 })),
            tool_use(10, "toolu_b", "Task", json!({ "description": "still going", "prompt": "go" })),
            result(11, "toolu_b", "Async agent launched", false, json!({ "status": "async_launched", "agentId": "yy8" })),
        ],
    );
    let subs = tmp.path().join("proj/s1/subagents");
    fs::create_dir_all(&subs).unwrap();
    fs::write(subs.join("agent-aa1.meta.json"), json!({ "toolUseId": "toolu_a", "agentType": "explore", "description": "scan src", "model": "claude-x", "finished": true }).to_string()).unwrap();
    let feed = convert(&path, "claude:s1").unwrap();
    assert_eq!(shape(&feed), ["spawn", "finish", "spawn"], "a launched agent that never finished gets no finish");
    assert!(calls(&feed).is_empty(), "a subagent call is not a tool row");
    let Event::Xai(XaiSessionUpdate::SubagentSpawned { child_session_id, subagent_type, model, parent_session_id, .. }) = &feed[0].event else { panic!("spawn") };
    assert_eq!((child_session_id.as_str(), subagent_type.as_str(), model.as_deref(), parent_session_id.as_str()), ("agent-aa1", "explore", Some("claude-x"), "claude:s1"));
    let Event::Xai(XaiSessionUpdate::SubagentFinished { status, tool_calls, duration_ms, output, .. }) = &feed[1].event else { panic!("finish") };
    assert_eq!((status.as_str(), *tool_calls, *duration_ms, output.as_deref()), ("completed", 4, 8000, Some("found three files")));
    let Event::Xai(XaiSessionUpdate::SubagentSpawned { child_session_id, .. }) = &feed[2].event else { panic!("spawn") };
    assert_eq!(child_session_id, "agent-yy8", "no sidecar: the id comes from the result");
}

#[test]
fn a_large_output_is_read_from_tool_results() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(
        tmp.path(),
        "s1",
        &[
            tool_use(0, "t", "Bash", json!({ "command": "big" })),
            result(1, "t", "<persisted-output>\nOutput too large (9KB). Full output saved to: /elsewhere/s1/tool-results/t.txt\n\nPreview (first 2KB):\nhead", false, Value::Null),
        ],
    );
    let dir = tmp.path().join("proj/s1/tool-results");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("t.txt"), "the whole output").unwrap();
    let feed = convert(&path, "claude:s1").unwrap();
    assert_eq!(crate::acp::tracker::content_blocks_text(&calls(&feed)[0].content), "the whole output");
}

#[test]
fn a_missing_stub_file_keeps_the_stub_text() {
    let feed = run(&[tool_use(0, "t", "Bash", json!({ "command": "big" })), result(1, "t", "<persisted-output>\nFull output saved to: /nowhere/x.txt\nhead", false, Value::Null)]);
    assert!(crate::acp::tracker::content_blocks_text(&calls(&feed)[0].content).contains("head"));
}

#[test]
fn context_lines_are_skipped() {
    let mut meta = user(1, "injected context");
    meta["isMeta"] = json!(true);
    let mut summary = user(2, "a long compaction summary");
    summary["isCompactSummary"] = json!(true);
    let feed = run(&[
        user(0, "real prompt"),
        meta,
        summary,
        json!({ "type": "api-request-blob", "timestamp": at(3), "blob": "x".repeat(50) }),
        json!({ "type": "system", "subtype": "stop_hook_summary" }),
        json!({ "type": "attachment", "attachment": {} }),
        assistant(4, "m1", json!({ "type": "text", "text": "answer" })),
    ]);
    assert_eq!(shape(&feed), ["user", "text"]);
}

#[test]
fn a_garbled_line_does_not_stop_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s1.jsonl");
    fs::write(&path, format!("{}\n{{not json\n{}", user(0, "a"), user(1, "b"))).unwrap();
    assert_eq!(shape(&convert(&path, "claude:s1").unwrap()), ["user", "user"]);
}

#[test]
fn a_missing_transcript_is_an_error() {
    assert!(convert(Path::new("/definitely/not/here.jsonl"), "x").is_err());
}
