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
