//! `bb thread` for the traces row: tell, stop, approve, deny.
//!
//! Each call runs on its own thread. The waiter polls the child once a second and kills it at a
//! 4s wall clock, the same shape as [`crate::remote`]. The thread id is the part after `#` in the
//! session's `file_path` (`<db>#<thread_id>`). This process never acts on `BB_THREAD_ID`.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const WALL: Duration = Duration::from_secs(4);
pub const POLL: Duration = Duration::from_secs(1);
const MAX_OUTPUT: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Tell { thread: String, text: String },
    Stop { thread: String },
    Approve { thread: String },
    Deny { thread: String },
}

impl Op {
    fn thread(&self) -> &str {
        match self {
            Op::Tell { thread, .. } | Op::Stop { thread } | Op::Approve { thread } | Op::Deny { thread } => thread,
        }
    }
}

/// How to run `bb`. Tests point `bin` at a script and shorten the clock.
#[derive(Clone, Debug)]
pub struct Runner {
    pub bin: String,
    pub timeout: Duration,
    pub poll: Duration,
    /// Thread this process must not tell, stop, approve, or deny.
    pub self_id: Option<String>,
}

impl Default for Runner {
    fn default() -> Self {
        Runner {
            bin: "bb".into(),
            timeout: WALL,
            poll: POLL,
            self_id: std::env::var("BB_THREAD_ID").ok().filter(|s| !s.is_empty()),
        }
    }
}

/// `thr_` plus ascii letters and digits. Anything else is not a bb thread id.
pub fn valid_thread(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("thr_") else { return false };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Thread id stored in a session `file_path` of `<db>#<thread_id>`.
pub fn thread_id(file_path: &str) -> Option<&str> {
    let (_, id) = crate::bb::split_key(file_path)?;
    valid_thread(id).then_some(id)
}

pub fn run(op: &Op) -> Result<String, String> {
    run_with(&Runner::default(), op)
}

pub fn run_with(r: &Runner, op: &Op) -> Result<String, String> {
    let thread = op.thread();
    if !valid_thread(thread) {
        return Err("invalid thread id".into());
    }
    if r.self_id.as_deref() == Some(thread) {
        return Err(format!("refusing {thread}"));
    }
    match op {
        Op::Tell { text, .. } => {
            let text = text.trim();
            if text.is_empty() {
                return Err("empty tell".into());
            }
            exec(r, &["thread", "tell", "--json", thread, "--", text])?;
            Ok(format!("told {thread}"))
        }
        Op::Stop { .. } => {
            exec(r, &["thread", "stop", "--json", thread])?;
            Ok(format!("stopped {thread}"))
        }
        Op::Approve { .. } | Op::Deny { .. } => {
            let id = pending_with(r, thread)?.into_iter().next().ok_or_else(|| "no pending interaction".to_string())?;
            let approve = matches!(op, Op::Approve { .. });
            let verb = if approve { "approve" } else { "deny" };
            exec(r, &["thread", "interactions", verb, "--json", &id, thread])?;
            Ok(format!("{} {thread}", if approve { "approved" } else { "denied" }))
        }
    }
}

pub fn pending(thread: &str) -> Result<Vec<String>, String> {
    pending_with(&Runner::default(), thread)
}

pub fn pending_with(r: &Runner, thread: &str) -> Result<Vec<String>, String> {
    if !valid_thread(thread) {
        return Err("invalid thread id".into());
    }
    let out = exec(r, &["thread", "interactions", "list", "--json", thread])?;
    parse_pending(&out)
}

fn parse_pending(stdout: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(stdout.trim()).map_err(|e| format!("bad interactions ({e})"))?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        return Err(error_message(&v, stdout));
    }
    let arr = match &v {
        Value::Array(a) => a,
        Value::Object(m) => m
            .get("interactions")
            .or_else(|| m.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| "bad interactions".to_string())?,
        _ => return Err("bad interactions".into()),
    };
    let mut ids = Vec::new();
    for item in arr {
        let Some(obj) = item.as_object() else { continue };
        if let Some(status) = obj.get("status").and_then(Value::as_str)
            && !matches!(status.to_ascii_lowercase().as_str(), "pending" | "open" | "awaiting")
        {
            continue;
        }
        let Some(id) = obj.get("id").or_else(|| obj.get("interactionId")).and_then(Value::as_str) else { continue };
        if valid_interaction(id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn valid_interaction(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn exec(r: &Runner, args: &[&str]) -> Result<String, String> {
    let o = spawn(r, args)?;
    if let Ok(v) = serde_json::from_str::<Value>(o.stdout.trim())
        && v.get("ok") == Some(&Value::Bool(false))
    {
        return Err(error_message(&v, &o.stderr));
    }
    if o.code != 0 {
        return Err(fail_text(&o));
    }
    Ok(o.stdout)
}

fn error_message(v: &Value, fallback: &str) -> String {
    v.pointer("/error/message")
        .and_then(Value::as_str)
        .map(clip)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| clip(fallback))
}

fn fail_text(o: &Out) -> String {
    let stderr = clip(&o.stderr);
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = clip(&o.stdout);
    if !stdout.is_empty() {
        return stdout;
    }
    format!("bb exit {}", o.code)
}

fn clip(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(160).collect()
}

fn spawn(r: &Runner, args: &[&str]) -> Result<Out, String> {
    let mut child = Command::new(&r.bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("bb: {e}"))?;
    fn drain(reader: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = reader.take(MAX_OUTPUT).read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        rx
    }
    let out_rx = drain(child.stdout.take().expect("piped"));
    let err_rx = drain(child.stderr.take().expect("piped"));
    let deadline = Instant::now() + r.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", r.timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(r.poll),
            Err(e) => return Err(format!("bb: {e}")),
        }
    };
    let grab = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        rx.recv_timeout(Duration::from_millis(500)).map_err(|e| format!("bb: output not read: {e}"))
    };
    let stdout = String::from_utf8_lossy(&grab(out_rx)?).into_owned();
    let stderr = String::from_utf8_lossy(&grab(err_rx)?).into_owned();
    Ok(Out { code: status.code().unwrap_or(-1), stdout, stderr })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake(dir: &std::path::Path, body: &str) -> Runner {
        let p = dir.join("bb");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        Runner { bin: p.to_string_lossy().into(), timeout: Duration::from_secs(2), poll: Duration::from_millis(20), self_id: None }
    }

    #[test]
    fn thread_id_is_the_key_after_hash() {
        assert_eq!(thread_id("/h/.bb/bb.db#thr_probe1"), Some("thr_probe1"));
        assert_eq!(thread_id("/h/.bb/bb.db#thr_x"), Some("thr_x"));
        assert_eq!(thread_id("/h/session.jsonl"), None);
        assert_eq!(thread_id("/h/.bb/bb.db#not_a_thread"), None);
        assert_eq!(thread_id("/h/.bb/bb.db#thr_"), None);
        assert_eq!(thread_id("/h/.bb/bb.db#thr_bad-id"), None);
    }

    #[test]
    fn tell_stop_and_approve_shell_out_without_a_shell() {
        let d = tempfile::tempdir().unwrap();
        let args = d.path().join("args");
        let r = fake(
            d.path(),
            &format!(
                r#"printf '%s\n' "$@" >> '{}'
echo '---' >> '{}'
case "$*" in
  *"interactions list"*) echo '[{{"id":"int_old","status":"resolved"}},{{"id":"int_ok","status":"pending"}}]' ;;
  *"interactions deny"*) echo '{{"ok":false,"error":{{"message":"nope"}}}}' ;;
  *) echo '{{"ok":true}}' ;;
esac"#,
                args.display(),
                args.display()
            ),
        );
        let told = run_with(&r, &Op::Tell { thread: "thr_probe1".into(), text: "reply $(with) `ticks`".into() }).unwrap();
        assert_eq!(told, "told thr_probe1");
        assert_eq!(run_with(&r, &Op::Stop { thread: "thr_probe1".into() }).unwrap(), "stopped thr_probe1");
        assert_eq!(run_with(&r, &Op::Approve { thread: "thr_probe1".into() }).unwrap(), "approved thr_probe1");
        let err = run_with(&r, &Op::Deny { thread: "thr_probe1".into() }).unwrap_err();
        assert!(err.contains("nope"), "{err}");
        let log = std::fs::read_to_string(&args).unwrap();
        assert!(log.contains("reply $(with) `ticks`"), "message is an argument, not a shell: {log}");
        assert!(log.contains("thread\ntell\n--json\nthr_probe1\n--\nreply $(with) `ticks`"), "{log}");
        assert!(log.contains("interactions\napprove\n--json\nint_ok\nthr_probe1"), "{log}");
        assert!(run_with(&r, &Op::Tell { thread: "thr_probe1".into(), text: "  ".into() }).is_err());
        assert!(run_with(&r, &Op::Stop { thread: "nope".into() }).is_err());
    }

    #[test]
    fn refuses_the_process_thread() {
        let d = tempfile::tempdir().unwrap();
        let mut r = fake(d.path(), "echo '{\"ok\":true}'");
        r.self_id = Some("thr_probe1".into());
        let err = run_with(&r, &Op::Stop { thread: "thr_probe1".into() }).unwrap_err();
        assert!(err.contains("refusing thr_probe1"), "{err}");
        let log = d.path().join("args");
        assert!(!log.exists(), "refused before spawning");
    }

    #[test]
    fn hung_bb_dies_at_the_wall_clock() {
        let d = tempfile::tempdir().unwrap();
        let mut r = fake(d.path(), "exec sleep 30");
        r.timeout = WALL;
        r.poll = POLL;
        let t = Instant::now();
        let err = run_with(&r, &Op::Stop { thread: "thr_probe1".into() }).unwrap_err();
        let elapsed = t.elapsed();
        assert!(elapsed < Duration::from_secs(6), "{elapsed:?} {err}");
        assert!(elapsed + Duration::from_millis(200) >= WALL, "{elapsed:?}");
        assert!(err.contains("timed out"), "{err}");
    }
}
