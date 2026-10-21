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
