//! Synthetic transcripts only. A tool call at T and its result at T+1500ms
//! stores `duration_ms = 1500` on the call row.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};
use xai_grok_traces::{EventFilter, EventRow, SessionFilter, TraceIndex, default_roots};

const T: i64 = 1_700_000_000_000;

fn write_lines(path: &Path, lines: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = lines.iter().map(|v| serde_json::to_string(v).unwrap()).collect::<Vec<_>>().join("\n") + "\n";
    fs::write(path, text).unwrap();
}

fn zstd_lines(path: &Path, lines: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = lines.iter().map(|v| serde_json::to_string(v).unwrap()).collect::<Vec<_>>().join("\n") + "\n";
    fs::write(path, zstd::encode_all(text.as_bytes(), 3).unwrap()).unwrap();
}

fn events(idx: &TraceIndex, source: &str) -> Vec<EventRow> {
    let rows = idx.list_sessions(&SessionFilter { source: Some(source.into()), ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1, "{source}: {rows:?}");
    idx.list_events(&rows[0].id, &EventFilter { include_noise: true, ..Default::default() }).unwrap()
}

fn tool<'a>(events: &'a [EventRow], title: &str) -> &'a EventRow {
    events.iter().find(|e| e.title == title && e.role.as_deref() == Some("tool")).unwrap_or_else(|| panic!("no {title} call in {events:?}"))
}

#[test]
fn call_to_result_fills_duration_ms() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let grok = home.join(".grok");

    // Results are not adjacent to their calls, and one call already carries a duration.
    write_lines(
        &home.join(".cursor/projects/w-proj/agent-transcripts/abc/abc.jsonl"),
        &[
            json!({"role":"user","message":{"content":[{"type":"text","text":"<user_query>\nfix it\n</user_query>"}]}}),
            json!({"role":"assistant","timestamp":T,"message":{"content":[{"type":"tool_use","id":"sh","name":"Shell","input":{"command":"ls"}}]}}),
            json!({"role":"assistant","timestamp":T + 100,"message":{"content":[{"type":"tool_use","id":"rd","name":"Read","input":{"path":"a.rs"}}]}}),
            json!({"role":"user","timestamp":T + 100 + 3100,"message":{"content":[{"type":"tool_result","tool_use_id":"rd","content":"file"}]}}),
            json!({"role":"user","timestamp":T + 1500,"message":{"content":[{"type":"tool_result","tool_use_id":"sh","content":"ok"}]}}),
            json!({"role":"assistant","timestamp":T,"duration_ms":42,"message":{"content":[{"type":"tool_use","id":"keep","name":"Bash","input":{"command":"true"}}]}}),
            json!({"role":"user","timestamp":T + 9000,"message":{"content":[{"type":"tool_result","tool_use_id":"keep","content":"ok"}]}}),
            json!({"role":"assistant","timestamp":T,"message":{"content":[{"type":"tool_use","id":"open","name":"Grep","input":{"pattern":"x"}}]}}),
            json!({"role":"assistant","timestamp":T + 5000,"message":{"content":[{"type":"tool_use","id":"neg","name":"Edit","input":{"file_path":"b.rs"}}]}}),
            json!({"role":"user","timestamp":T + 4000,"message":{"content":[{"type":"tool_result","tool_use_id":"neg","content":"no"}]}}),
        ],
    );
    // A result for a different id sits before the call; it must not become the call's duration.
    write_lines(
        &grok.join("sessions/g/g1/chat_history.jsonl"),
        &[
            json!({"type":"user","content":[{"type":"text","text":"grok task"}]}),
            json!({"type":"tool_result","timestamp":T + 500,"tool_call_id":"other","content":"no"}),
            json!({"type":"assistant","timestamp":T,"content":"x","model_id":"grok-5","tool_calls":[{"id":"t1","name":"run_terminal_command","arguments":{"command":"ls"}}]}),
            json!({"type":"tool_result","timestamp":T + 1500,"tool_call_id":"t1","content":"ok"}),
        ],
    );
    zstd_lines(
        &home.join(".dsh/sessions/w/u/session.jsonl.zstd"),
        &[
            json!({"type":"session","version":2,"id":"d","cwd":"/w/dsh"}),
            json!({"type":"tool/call","seq":2,"time":T,"data":{"name":"read_file","arguments":{"path":"a.rs"},"callId":"c1"}}),
            json!({"type":"tool/result","seq":3,"time":T + 1500,"data":{"callId":"c1","message":"done"}}),
        ],
    );
    // Codex already records some durations; a recorded one stays, a missing one is filled.
    write_lines(
        &home.join(".codex/sessions/2026/10/01/rollout-d.jsonl"),
        &[
            json!({"timestamp":T,"type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"c","arguments":"{\"cmd\":\"ls\"}"}}),
            json!({"timestamp":T + 1500,"type":"response_item","payload":{"type":"function_call_output","call_id":"c","output":"ok"}}),
            json!({"timestamp":T,"duration_ms":7,"type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"keep","arguments":"{\"cmd\":\"true\"}"}}),
            json!({"timestamp":T + 1500,"type":"response_item","payload":{"type":"function_call_output","call_id":"keep","output":"ok"}}),
        ],
    );
    // Pi records `durationMs` on the result. That value stays; the call still gets the timestamp span.
    write_lines(
        &home.join(".pi/agent/sessions/--w--/p.jsonl"),
        &[
            json!({"type":"message","timestamp":T,"message":{"role":"assistant","content":[{"type":"toolCall","id":"p1","name":"bash","arguments":{"command":"ls"}}]}}),
            json!({"type":"message","timestamp":T + 1500,"message":{"role":"toolResult","toolCallId":"p1","toolName":"bash","durationMs":9,"content":[{"type":"text","text":"ok"}]}}),
        ],
    );

    let roots = default_roots(&home, &grok);
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    let report = idx.scan(&roots).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let cursor = events(&idx, "cursor");
    assert_eq!(tool(&cursor, "Shell").duration_ms, Some(1500), "shell span is the shell result, not the read that sits next to the call");
    assert_eq!(tool(&cursor, "Read").duration_ms, Some(3100));
    assert_eq!(tool(&cursor, "Bash").duration_ms, Some(42), "a duration the harness wrote is kept");
    assert_eq!(tool(&cursor, "Grep").duration_ms, None, "no result, no duration");
    assert_eq!(tool(&cursor, "Edit").duration_ms, None, "a result timestamp before the call is not a duration");
    assert!(cursor.iter().filter(|e| e.role.as_deref() == Some("tool_result")).all(|e| e.duration_ms.is_none()));

    let grok_ev = events(&idx, "grok");
    let call = grok_ev.iter().find(|e| e.role.as_deref() == Some("assistant")).unwrap();
    assert_eq!(call.duration_ms, Some(1500));
    assert!(grok_ev.iter().filter(|e| e.role.as_deref() == Some("tool_result")).all(|e| e.duration_ms.is_none()));

    let dsh = events(&idx, "dsh");
    assert_eq!(tool(&dsh, "read_file").duration_ms, Some(1500));
    assert!(dsh.iter().filter(|e| e.role.as_deref() == Some("tool_result")).all(|e| e.duration_ms.is_none()));

    let codex = events(&idx, "codex");
    let by_detail = |d: &str| codex.iter().find(|e| e.role.as_deref() == Some("tool") && e.detail.as_deref() == Some(d)).unwrap();
    assert_eq!(by_detail("ls").duration_ms, Some(1500));
    assert_eq!(by_detail("true").duration_ms, Some(7));

    let pi = events(&idx, "pi");
    assert_eq!(tool(&pi, "bash").duration_ms, Some(1500));
    let pi_result = pi.iter().find(|e| e.role.as_deref() == Some("tool_result")).unwrap();
    assert_eq!(pi_result.duration_ms, Some(9), "durationMs on the result stays on the result");
}

#[test]
fn an_appended_result_fills_the_call_indexed_earlier() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let path = home.join(".cursor/projects/w-proj/agent-transcripts/late/late.jsonl");
    write_lines(
        &path,
        &[json!({"role":"assistant","timestamp":T,"message":{"content":[{"type":"tool_use","id":"late","name":"Shell","input":{"command":"pwd"}}]}})],
    );

    let roots = default_roots(&home, &home.join(".grok"));
    let cursor = roots.iter().find(|r| r.source == "cursor").unwrap().clone();
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    idx.scan(&roots).unwrap();
    let before = events(&idx, "cursor");
    assert_eq!(tool(&before, "Shell").duration_ms, None);

    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(
        f,
        "{}",
        serde_json::to_string(&json!({"role":"user","timestamp":T + 1500,"message":{"content":[{"type":"tool_result","tool_use_id":"late","content":"ok"}]}})).unwrap()
    )
    .unwrap();
    drop(f);
    assert!(idx.index_file(&cursor, &path).unwrap(), "the appended result is indexed");

    let after = events(&idx, "cursor");
    assert_eq!(after.len(), 2, "the call line is not parsed again: {after:?}");
    assert_eq!(tool(&after, "Shell").duration_ms, Some(1500));
    assert_eq!(tool(&after, "Shell").line, 0);
}
