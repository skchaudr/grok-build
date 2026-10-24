//! Synthetic fixtures only; never real session content. `GROK_HOME` is a tempdir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use xai_grok_traces::{SessionFilter, TraceIndex, default_roots};

const PI: &str = r#"{"type":"session","version":3,"id":"p","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/pi"}
{"type":"message","timestamp":"2026-10-01T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"pi task"}]}}
{"type":"message","timestamp":"2026-10-01T00:00:02Z","message":{"role":"assistant","model":"pi-model","content":[{"type":"text","text":"ok"}]}}
"#;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

struct Fix {
    _tmp: tempfile::TempDir,
    /// `$GROK_HOME`. `default_db_path` is this joined with `traces/traces.sqlite`.
    grok: PathBuf,
    id: String,
}

fn fix() -> Fix {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let grok = tmp.path().join("grok");
    write(&home.join(".pi/agent/sessions/--w-pi--/p.jsonl"), PI);
    let db = grok.join("traces").join("traces.sqlite");
    let id = {
        let roots = default_roots(&home, &grok);
        let mut idx = TraceIndex::open(&db).unwrap();
        idx.scan(&roots).unwrap();
        let rows = idx.list_sessions(&SessionFilter { source: Some("pi".into()), ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 1, "pi: {rows:?}");
        let id = rows.into_iter().next().unwrap().id;
        drop(idx);
        id
    };
    Fix { _tmp: tmp, grok, id }
}

fn run(grok: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_grok-traces"))
        .args(args)
        .env("GROK_HOME", grok)
        .output()
        .unwrap()
}

fn lines_of(stdout: &[u8]) -> Vec<i64> {
    let text = std::str::from_utf8(stdout).unwrap();
    text.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["line"].as_i64().unwrap())
        .collect()
}

#[test]
fn after_flag_and_version_token() {
    let f = fix();
    let id = f.id.as_str();

    let since = run(&f.grok, &["events", id, "--json", "--since-line", "1", "--noise"]);
    assert_eq!(since.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&since.stderr));
    assert_eq!(lines_of(&since.stdout), vec![2]);

    let since_all = run(&f.grok, &["events", id, "--json", "--since-line", "-1", "--noise"]);
    assert_eq!(since_all.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&since_all.stderr));
    assert_eq!(lines_of(&since_all.stdout), vec![0, 1, 2]);

    let since_bad = run(&f.grok, &["events", id, "--json", "--since-line", "x"]);
    assert_eq!(since_bad.status.code(), Some(1));
    assert!(since_bad.stdout.is_empty(), "stdout: {:?}", String::from_utf8_lossy(&since_bad.stdout));
    assert!(
        String::from_utf8_lossy(&since_bad.stderr).contains("--since-line needs a number"),
        "stderr: {}",
        String::from_utf8_lossy(&since_bad.stderr)
    );

    let a = run(&f.grok, &["events", id, "--json", "--after", "1", "--noise"]);
    assert_eq!(a.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&a.stderr));
    assert_eq!(lines_of(&a.stdout), vec![2]);

    let b = run(&f.grok, &["events", id, "--json", "--after", "-1", "--noise"]);
    assert_eq!(b.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&b.stderr));
    assert_eq!(lines_of(&b.stdout), vec![0, 1, 2]);

    let c = run(&f.grok, &["events", id, "--json", "--after", "x"]);
    assert_eq!(c.status.code(), Some(1));
    assert!(c.stdout.is_empty(), "stdout: {:?}", String::from_utf8_lossy(&c.stdout));
    assert!(String::from_utf8_lossy(&c.stderr).contains("--after needs a number"), "stderr: {}", String::from_utf8_lossy(&c.stderr));

    let d = run(&f.grok, &["version"]);
    let ver = String::from_utf8(d.stdout).unwrap();
    assert!(ver.contains("after=1") && ver.contains("remote=1"), "{ver}");

    let e = run(&f.grok, &["events", id, "--json", "--noise"]);
    assert_eq!(e.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&e.stderr));
    assert_eq!(lines_of(&e.stdout), vec![0, 1, 2]);
}

// Contract: sab-air ~/repos/trace-deck/fixtures live-events.ndjson / live-list.ndjson (key order captured from the CLI)
const EVENT_KEYS: [&str; 17] = [
    "line",
    "type",
    "kind",
    "role",
    "title",
    "preview",
    "timestamp",
    "duration_ms",
    "input_tokens",
    "output_tokens",
    "turn",
    "step",
    "depth",
    "model",
    "is_error",
    "noise",
    "detail",
];
const SESSION_KEYS: [&str; 31] = [
    "id",
    "source",
    "title",
    "path",
    "model",
    "cwd",
    "started_at",
    "updated_at",
    "event_count",
    "user_count",
    "assistant_count",
    "tool_count",
    "error_count",
    "input_tokens",
    "output_tokens",
    "duration_ms",
    "status",
    "parent_id",
    "label",
    "agent_type",
    "now",
    "last_event_at",
    "children",
    "failed",
    "native_id",
    "mtime_ms",
    "harness_pid",
    "harness_start",
    "context_tokens",
    "context_window",
    "via",
];

#[test]
fn wire_shapes_match_trace_deck_fixtures() {
    // Cargo.toml enables serde_json preserve_order; insertion order must survive a parse.
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("preserve_order"), "serde_json preserve_order");
    let probe: serde_json::Value = serde_json::from_str(r#"{"z":1,"a":2}"#).unwrap();
    let probe_keys: Vec<&str> = probe.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(probe_keys, ["z", "a"], "serde_json key order");

    let f = fix();
    let events = run(&f.grok, &["events", f.id.as_str(), "--json", "--noise"]);
    assert_eq!(events.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&events.stderr));
    let event_text = std::str::from_utf8(&events.stdout).unwrap();
    let event_lines: Vec<&str> = event_text.lines().filter(|l| !l.is_empty()).collect();
    assert!(!event_lines.is_empty(), "events");
    for line in event_lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let obj = v.as_object().unwrap();
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(keys, EVENT_KEYS);
        assert!(obj["line"].is_i64(), "line");
        assert!(obj["noise"].is_boolean(), "noise");
        assert!(obj["is_error"].is_boolean(), "is_error");
    }

    let sessions = run(&f.grok, &["list", "--json"]);
    assert_eq!(sessions.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&sessions.stderr));
    let session_text = std::str::from_utf8(&sessions.stdout).unwrap();
    let session_lines: Vec<&str> = session_text.lines().filter(|l| !l.is_empty()).collect();
    assert!(!session_lines.is_empty(), "sessions");
    for line in session_lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let obj = v.as_object().unwrap();
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(keys, SESSION_KEYS);
        assert!(obj["children"].is_i64(), "children");
        assert!(obj["status"].is_string(), "status");
    }

    // sample-events.json hand-written subset. Order differs from EVENT_KEYS by design; containment only.
    const SAMPLE_EVENT_KEYS: [&str; 11] = [
        "line", "type", "kind", "role", "title", "preview", "detail", "depth", "noise", "is_error", "turn",
    ];
    assert!(SAMPLE_EVENT_KEYS.iter().all(|k| EVENT_KEYS.contains(k)));
}
