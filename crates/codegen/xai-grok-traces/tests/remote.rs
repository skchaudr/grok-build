//! Remote fetch against fake `ssh` scripts (no network).

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use xai_grok_traces::Status;
use xai_grok_traces::remote::*;

const ROW: &str = r#"{"id":"claude:abc","source":"claude","title":"t","path":"/x","updated_at":2000,"status":"active","parent_id":"claude:p","label":"L","agent_type":"A","now":"Bash: ls","last_event_at":1999,"children":2,"failed":true,"event_count":5}"#;
const ROW2: &str = r#"{"id":"codex:def","source":"codex","updated_at":1000,"status":"idle"}"#;

fn fake(dir: &tempfile::TempDir, body: &str) -> Ssh {
    let p = dir.path().join("ssh");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    Ssh { bin: p.to_string_lossy().into(), timeout: WALL_TIMEOUT }
}

#[test]
fn hosts_file_parse() {
    assert_eq!(parse_hosts("# c\nsab-mini@sab-air # air\n\n-oProxy=x\nbad host\nsab-mini@sab-air\nbox\n"), ["sab-mini@sab-air", "box"]);
    assert_eq!(label("sab-mini@sab-air"), "sab-air");
    assert_eq!(label("box"), "box");
}

#[test]
fn parse_tag_and_merge() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake(&d, &format!("echo '{ROW}'\necho '{ROW2}'"));
    let f = fetch_sessions_with(&ssh, "sab-mini@sab-air", &RemoteFilter { active_only: true, limit: 20, ..Default::default() }).unwrap();
    assert!(f.note.is_none());
    assert_eq!(f.rows.len(), 2);
    let r = &f.rows[0];
    assert_eq!((r.id.as_str(), r.host.as_deref(), r.parent_id.as_deref()), ("sab-air:claude:abc", Some("sab-air"), Some("sab-air:claude:p")));
    assert_eq!((r.status, r.children, r.failed, r.now.as_deref()), (Status::Active, 2, true, Some("Bash: ls")));
    assert_eq!(remote_id("sab-mini@sab-air", &r.id), "claude:abc");
    assert_eq!(f.rows[1].status, Status::Idle);
    assert_eq!(cached_sessions("sab-mini@sab-air").unwrap().1.len(), 2);

    let local = vec![{
        let mut l = f.rows[1].clone();
        l.id = "claude:local".into();
        l.host = None;
        l.updated_at = Some(1500);
        l
    }];
    let m = merge(local, vec![f.rows.clone()], 0);
    let ids: Vec<_> = m.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["sab-air:claude:abc", "claude:local", "sab-air:codex:def"]);
    assert_eq!(merge(vec![], vec![f.rows], 1).len(), 1);
}

#[test]
fn events_and_raw() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake(
        &d,
        r#"case "$*" in *" events claude:abc --json --noise"*) echo '{"line":3,"type":"tool_use","kind":"tool","role":"tool","title":"Bash","preview":"p","depth":1,"is_error":true,"noise":false,"detail":"ls"}';; *" raw claude:abc 9"*) exit 3;; *" raw claude:abc 3"*) echo '{"raw":1}';; *) exit 9;; esac"#,
    );
    let ev = fetch_events_with(&ssh, "u@h", "h:claude:abc", &EventQuery { include_noise: true, limit: 0 }).unwrap();
    assert_eq!((ev[0].line, ev[0].is_error, ev[0].detail.as_deref()), (3, true, Some("ls")));
    assert_eq!(fetch_raw_with(&ssh, "u@h", "h:claude:abc", 3).unwrap().as_deref(), Some(r#"{"raw":1}"#));
    assert_eq!(fetch_raw_with(&ssh, "u@h", "h:claude:abc", 9).unwrap(), None);
    assert!(fetch_events_with(&ssh, "u@h", "x; rm -rf ~", &EventQuery::default()).is_err(), "ids are validated");
}

#[test]
fn hung_ssh_returns_error_within_wall_timeout() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake(&d, "sleep 30");
    let t = Instant::now();
    let e = fetch_sessions_with(&ssh, "sab-mini@sab-air", &RemoteFilter::default()).unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(6), "{:?}", t.elapsed());
    assert!(matches!(e, Error::Unreachable { .. }));
    assert!(e.to_string().starts_with("sab-air unreachable"), "{e}");
}

#[test]
fn ssh_failure_and_missing_binary_are_values() {
    let d = tempfile::tempdir().unwrap();
    let e = fetch_sessions_with(&fake(&d, "echo 'ssh: Could not resolve hostname' >&2; exit 255"), "u@gone", &RemoteFilter::default()).unwrap_err();
    assert!(e.to_string().starts_with("gone unreachable"), "{e}");
    let e = fetch_sessions_with(&fake(&d, "exit 127"), "u@h", &RemoteFilter::default()).unwrap_err();
    assert!(e.to_string().contains("not installed"), "{e}");
    let e = fetch_sessions_with(&Ssh { bin: "/no/such/ssh".into(), timeout: WALL_TIMEOUT }, "u@h", &RemoteFilter::default()).unwrap_err();
    assert!(matches!(e, Error::Unreachable { .. }));
    let e = fetch_sessions_with(&fake(&d, "echo not json"), "u@h", &RemoteFilter::default()).unwrap_err();
    assert!(matches!(e, Error::Parse { .. }));
}

#[test]
fn old_remote_degrades_and_filters_locally() {
    let d = tempfile::tempdir().unwrap();
    // Old binary: rejects any flag but --json/--limit; knows no `version`.
    let ssh = fake(
        &d,
        &format!(
            r#"case "$*" in *--since*|*--active*|*--roots*|*" version"*) echo 'grok-traces: unknown flag' >&2; exit 1;; *) echo '{ROW}'; echo '{ROW2}'; echo '{{"id":"claude:kid","parent_id":"claude:abc","updated_at":3000,"status":"active"}}';; esac"#
        ),
    );
    let f = fetch_sessions_with(&ssh, "u@sab-air", &RemoteFilter { active_only: true, roots_only: false, since_ms: Some(1500), limit: 10 }).unwrap();
    assert_eq!(f.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["sab-air:claude:abc", "sab-air:claude:kid"]);
    let note = f.note.unwrap();
    assert!(note.contains("older") && note.contains("version unknown"), "{note}");
}

// ---- live (tests 40-45, 53 of SPEC-live.md) ----

use xai_grok_traces::live::{
    LIVE_SCHEMA, LiveEnvelope, LiveRow, LiveState, age_ms, fmt_age, order, render,
};

fn envelope(rows: &[LiveRow]) -> String {
    serde_json::to_string(&LiveEnvelope {
        live_schema: LIVE_SCHEMA,
        rows: rows.to_vec(),
    })
    .unwrap()
}

fn lrow(id: &str, state: LiveState, age: i64) -> LiveRow {
    LiveRow {
        session_id: id.into(),
        state,
        age_ms: age,
        text: "t".into(),
        ..Default::default()
    }
}

fn serve(dir: &tempfile::TempDir, json: &str) -> Ssh {
    fake(dir, &format!("echo '{json}'"))
}

#[test]
fn live_40_tag_prefix_age_and_order() {
    let d = tempfile::tempdir().unwrap();
    let mut w = lrow("w1", LiveState::Waiting, 5000);
    w.native_id = Some("nat".into());
    let f = fetch_live_with(&serve(&d, &envelope(&[w])), "t40@sab-air").unwrap();
    assert!(f.note.is_none());
    let r = &f.rows[0];
    assert_eq!(
        (
            r.session_id.as_str(),
            r.host.as_deref(),
            r.parent_id.as_deref(),
            r.native_id.as_deref()
        ),
        ("sab-air:w1", Some("sab-air"), None, Some("nat"))
    );
    assert_eq!(fmt_age(age_ms(r, 3000)), "8s");
    assert_eq!(render(r, 3000).glyph, "⚠");
    let mut all = vec![lrow("local", LiveState::Working, 1000), r.clone()];
    order(&mut all);
    assert_eq!(
        all.iter()
            .map(|r| r.session_id.as_str())
            .collect::<Vec<_>>(),
        ["sab-air:w1", "local"]
    );
}

#[test]
fn live_41_negative_age_clamps() {
    let d = tempfile::tempdir().unwrap();
    let f = fetch_live_with(
        &serve(&d, &envelope(&[lrow("n", LiveState::Working, -400)])),
        "t41@sab-air",
    )
    .unwrap();
    assert_eq!(age_ms(&f.rows[0], 0), 0);
    assert_eq!(fmt_age(age_ms(&f.rows[0], 0)), "0s");
}

#[test]
fn live_42_gate_exit2_and_schema_mismatch() {
    let d = tempfile::tempdir().unwrap();
    let note = |n: &str| format!("sab-air: update grok-traces (live_schema {n})");
    for (body, n) in [
        ("exit 2", "none"),
        ("echo ''", "none"),
        ("echo not json", "none"),
        (r#"echo '{"rows":[]}'"#, "none"),
        (r#"echo '{"live_schema":"1","rows":[]}'"#, "none"),
        (r#"echo '{"live_schema":0,"rows":[]}'"#, "0"),
        (r#"echo '{"live_schema":99,"rows":[{"bogus":1}]}'"#, "99"),
    ] {
        let f = fetch_live_with(&fake(&d, body), "t42@sab-air").unwrap();
        assert!(f.rows.is_empty(), "{body}");
        assert_eq!(f.note, Some(note(n)), "{body}");
    }
    assert!(cached_live("t42@sab-air").is_none(), "a gate never caches");
    assert!(matches!(
        fetch_live_with(&fake(&d, "exit 127"), "t42@sab-air").unwrap_err(),
        Error::Remote { .. }
    ));
    assert!(matches!(
        fetch_live_with(&fake(&d, "exit 1"), "t42@sab-air").unwrap_err(),
        Error::Remote { .. }
    ));
    let bad = r#"echo '{"live_schema":1,"rows":[{"state":"nope"}]}'"#;
    assert!(matches!(
        fetch_live_with(&fake(&d, bad), "t42@sab-air").unwrap_err(),
        Error::Parse { .. }
    ));
}

#[test]
fn live_43_unreachable_falls_back_to_cache() {
    let d = tempfile::tempdir().unwrap();
    let mut w = lrow("a", LiveState::Working, 1000);
    w.quiet_ms = 1000;
    fetch_live_with(&serve(&d, &envelope(&[w])), "t43@sab-air").unwrap();
    let down = fake(&d, "echo 'ssh: connect refused' >&2; exit 255");
    assert!(matches!(
        fetch_live_with(&down, "t43@sab-air").unwrap_err(),
        Error::Unreachable { .. }
    ));
    let (age, rows) = cached_live("t43@sab-air").unwrap();
    assert!(age < Duration::from_secs(5));
    assert_eq!(rows[0].session_id, "sab-air:a");
    // The pager passes the cache age as `extra`; the stale glyph in the header is its job (step 2).
    assert_eq!(fmt_age(age_ms(&rows[0], 12_000)), "13s");
    assert!(render(&rows[0], 12_000).text.contains('t'));
}

#[test]
fn live_44_child_nests_under_remote_parent() {
    let d = tempfile::tempdir().unwrap();
    let parent = lrow("p", LiveState::Working, 9000);
    let mut kid = lrow("k", LiveState::Waiting, 100);
    kid.parent_id = Some("p".into());
    let f = fetch_live_with(&serve(&d, &envelope(&[kid, parent])), "t44@sab-air").unwrap();
    assert_eq!(f.rows[0].parent_id.as_deref(), Some("sab-air:p"));
    let mut all = f.rows;
    all.push(lrow("local", LiveState::Prompt, 50));
    order(&mut all);
    let ids: Vec<_> = all.iter().map(|r| r.session_id.as_str()).collect();
    assert_eq!(ids, ["local", "sab-air:p", "sab-air:k"]);
    // An orphan (parent in no fetch) is a root.
    let mut o = lrow("sab-air:o", LiveState::Waiting, 1);
    o.parent_id = Some("sab-air:gone".into());
    let mut all = vec![lrow("x", LiveState::Working, 5), o];
    order(&mut all);
    assert_eq!(all[0].session_id, "sab-air:o");
}

#[test]
fn live_45_serde_roundtrip_and_defaults() {
    let mut r = lrow("a", LiveState::Compacting, 7);
    r.parent_id = Some("p".into());
    r.since_ms = Some(3);
    r.bg = 2;
    r.bg_quiet = true;
    r.failed = true;
    r.context_window = Some(200_000);
    r.host = Some("h".into());
    let env = LiveEnvelope {
        live_schema: LIVE_SCHEMA,
        rows: vec![r.clone(), lrow("b", LiveState::Tool, 1)],
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains(r#""state":"compacting""#));
    assert_eq!(serde_json::from_str::<LiveEnvelope>(&json).unwrap(), env);
    let mut v: serde_json::Value = serde_json::to_value(&r).unwrap();
    v.as_object_mut().unwrap().remove("context_window");
    v.as_object_mut().unwrap().remove("bg");
    let back: LiveRow = serde_json::from_value(v).unwrap();
    assert_eq!((back.context_window, back.bg), (None, 0));
    assert_eq!(back.session_id, "a");
}

#[test]
fn live_53_cached_tool_row_goes_stale() {
    let d = tempfile::tempdir().unwrap();
    let mut t = lrow("t", LiveState::Tool, 0);
    t.text = "Bash".into();
    t.since_ms = Some(25_000);
    fetch_live_with(&serve(&d, &envelope(&[t])), "t53@sab-air").unwrap();
    let down = fake(&d, "exit 255");
    assert!(fetch_live_with(&down, "t53@sab-air").is_err());
    let (_, rows) = cached_live("t53@sab-air").unwrap();
    assert_eq!(render(&rows[0], 0).glyph, "▶");
    assert_eq!(render(&rows[0], 6000).glyph, "⏳");
}
