//! Live view of running sessions, built on hook receipts. Process identity (`alive`, the
//! ancestry walk) comes first; the receipt queries and the state machine follow.

/// Interpreters a hook is run through; the harness is the first ancestor that is not one of these.
const SHELLS: [&str; 5] = ["sh", "bash", "zsh", "dash", "fish"];
/// Ancestors examined before giving up.
const MAX_WALK: usize = 64;

/// One process of an ancestry chain: `(pid, ppid, comm)`.
pub type Proc = (i64, i64, String);

fn is_shell(comm: &str) -> bool {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    SHELLS.contains(&base.strip_prefix('-').unwrap_or(base))
}

/// The first process of `chain` (nearest ancestor first) whose `comm` is not a shell.
/// A chain of shells only gives `None`.
pub fn harness_in_chain(chain: &[Proc]) -> Option<i64> {
    chain.iter().find(|(_, _, comm)| !is_shell(comm)).map(|(pid, _, _)| *pid)
}

/// (ppid, comm, start in Unix seconds, is zombie) of a process.
#[cfg(target_os = "macos")]
fn proc_info(pid: i64) -> Option<(i64, String, i64, bool)> {
    let pid32 = i32::try_from(pid).ok().filter(|p| *p > 0)?;
    // SAFETY: `proc_pidinfo` fills at most `size` bytes of the zeroed struct we pass.
    let info = unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = libc::proc_pidinfo(pid32, libc::PROC_PIDTBSDINFO, 0, std::ptr::addr_of_mut!(info).cast(), size);
        if got != size {
            return None;
        }
        info
    };
    let text = |raw: &[libc::c_char]| {
        let bytes: Vec<u8> = raw.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let name = text(&info.pbi_name);
    let comm = if name.is_empty() { text(&info.pbi_comm) } else { name };
    Some((i64::from(info.pbi_ppid), comm, i64::try_from(info.pbi_start_tvsec).ok()?, info.pbi_status == libc::SZOMB))
}

#[cfg(not(target_os = "macos"))]
fn proc_info(_pid: i64) -> Option<(i64, String, i64, bool)> {
    None
}

/// Start time of a process in Unix seconds.
pub fn proc_start(pid: i64) -> Option<i64> {
    proc_info(pid).map(|(_, _, start, _)| start)
}

/// The harness above the hook: walks up from this process's parent, past shells.
/// Returns `(pid, start)`; `None` when the chain is unreadable or all shells.
pub fn find_harness() -> Option<(i64, i64)> {
    #[cfg(unix)]
    {
        // SAFETY: getppid has no preconditions.
        let mut pid = i64::from(unsafe { libc::getppid() });
        let mut chain: Vec<Proc> = Vec::new();
        let mut starts = Vec::new();
        while pid > 1 && chain.len() < MAX_WALK && !chain.iter().any(|(p, _, _)| *p == pid) {
            let Some((ppid, comm, start, _)) = proc_info(pid) else { break };
            chain.push((pid, ppid, comm));
            starts.push(start);
            pid = ppid;
        }
        let harness = harness_in_chain(&chain)?;
        let i = chain.iter().position(|(p, _, _)| *p == harness)?;
        Some((harness, starts[i]))
    }
    #[cfg(not(unix))]
    None
}

/// The process exists, is not a zombie, and is the one that started at `start` (a reused pid has a different start).
pub fn alive(pid: i64, start: i64) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid32) = i32::try_from(pid) else { return false };
        if pid32 <= 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the process can be signalled.
        let exists = unsafe { libc::kill(pid32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        // A zombie has exited; its pid and start linger until the parent reaps it.
        exists && proc_info(pid).is_some_and(|(_, _, s, zombie)| s == start && !zombie)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, start);
        false
    }
}

// ---- receipts as the live signal ----

/// One hook receipt as the live screen reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveReceipt {
    pub id: i64,
    pub session_id: Option<String>,
    pub native_session: Option<String>,
    pub agent_id: Option<String>,
    pub event: String,
    pub tool: Option<String>,
    pub detail: Option<String>,
    pub tool_use_id: Option<String>,
    pub at_ms: i64,
}

/// A turn boundary: what the session did when a receipt of this kind arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Idle,
    Prompt,
    End,
}

/// Subagent lifetime events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    Start,
    Stop,
}

/// Every event name `boundary` can map; the SQL prefilter (it still decides on `detail`).
pub const BOUNDARY_EVENTS: &[&str] = &[
    "SessionStart",
    "Stop",
    "StopFailure",
    "Notification",
    "UserPromptSubmit",
    "SessionEnd",
    "session_start",
    "agent_end",
    "agent_start",
    "session_shutdown",
    "sessionStart",
    "stop",
    "beforeSubmitPrompt",
    "sessionEnd",
];

/// Every event name `child_kind` can map.
pub const CHILD_EVENTS: &[&str] = &["SubagentStart", "SubagentStop", "subagentStart", "subagentStop"];

/// The turn boundary an event is, per source. Event names are case-sensitive per source.
/// A Claude `SessionStart` with detail `compact` fires mid-turn and is no boundary;
/// a Claude `Notification` is one only as `idle_prompt`.
pub fn boundary(source: &str, event: &str, detail: Option<&str>) -> Option<Boundary> {
    use Boundary::{End, Idle, Prompt};
    match (source, event) {
        ("claude", "SessionStart") => (detail != Some("compact")).then_some(Idle),
        ("claude", "Stop" | "StopFailure") => Some(Idle),
        ("claude", "Notification") => (detail == Some("idle_prompt")).then_some(Idle),
        ("claude", "UserPromptSubmit") => Some(Prompt),
        ("claude", "SessionEnd") => Some(End),
        ("pi", "session_start" | "agent_end") => Some(Idle),
        ("pi", "agent_start") => Some(Prompt),
        ("pi", "session_shutdown") => Some(End),
        ("cursor", "sessionStart" | "stop") => Some(Idle),
        ("cursor", "beforeSubmitPrompt") => Some(Prompt),
        ("cursor", "sessionEnd") => Some(End),
        _ => None,
    }
}

/// Subagent start/stop events: Claude `SubagentStart/Stop`, Cursor `subagentStart/Stop`.
pub fn child_kind(source: &str, event: &str) -> Option<Child> {
    match (source, event) {
        ("claude", "SubagentStart") | ("cursor", "subagentStart") => Some(Child::Start),
        ("claude", "SubagentStop") | ("cursor", "subagentStop") => Some(Child::Stop),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(comms: &[&str]) -> Vec<Proc> {
        comms.iter().enumerate().map(|(i, c)| (100 + i as i64, 101 + i as i64, (*c).to_string())).collect()
    }

    #[test]
    fn walk_skips_shells_by_basename() {
        // The real Claude chain: sh, /bin/zsh, claude, -zsh, herdr.
        assert_eq!(harness_in_chain(&chain(&["sh", "/bin/zsh", "claude", "-zsh", "herdr"])), Some(102));
        assert_eq!(harness_in_chain(&chain(&["sh", "-zsh", "Cursor Helper (Plugin)"])), Some(102));
        assert_eq!(harness_in_chain(&chain(&["claude"])), Some(100));
        assert_eq!(harness_in_chain(&chain(&["sh", "-bash", "fish"])), None);
        assert_eq!(harness_in_chain(&[]), None);
    }

    #[test]
    fn boundary_table() {
        use Boundary::*;
        let b = boundary;
        assert_eq!(b("claude", "SessionStart", Some("startup")), Some(Idle));
        assert_eq!(b("claude", "SessionStart", None), Some(Idle));
        assert_eq!(b("claude", "SessionStart", Some("compact")), None, "mid-turn after auto-compaction");
        assert_eq!(b("claude", "Notification", Some("idle_prompt")), Some(Idle));
        assert_eq!(b("claude", "Notification", Some("permission_prompt")), None);
        assert_eq!(b("claude", "Notification", None), None);
        assert_eq!((b("claude", "Stop", None), b("claude", "StopFailure", None)), (Some(Idle), Some(Idle)));
        assert_eq!((b("claude", "UserPromptSubmit", None), b("claude", "SessionEnd", None)), (Some(Prompt), Some(End)));
        assert_eq!((b("pi", "agent_end", None), b("pi", "agent_start", None), b("pi", "session_shutdown", None)), (Some(Idle), Some(Prompt), Some(End)));
        assert_eq!(b("pi", "session_start", None), Some(Idle));
        assert_eq!((b("cursor", "stop", None), b("cursor", "beforeSubmitPrompt", None), b("cursor", "sessionEnd", None)), (Some(Idle), Some(Prompt), Some(End)));
        assert_eq!(b("cursor", "sessionStart", None), Some(Idle));
        // Case-sensitive per source; tools and ticks are no boundary.
        assert_eq!((b("cursor", "Stop", None), b("claude", "stop", None), b("claude", "PreToolUse", None), b("codex", "Stop", None)), (None, None, None, None));
        assert_eq!(b("cursor", "afterAgentResponse", None), None);
    }

    #[test]
    fn boundary_events_cover_the_table() {
        for (source, event) in [("claude", "Notification"), ("claude", "Stop"), ("pi", "agent_end"), ("cursor", "sessionEnd")] {
            assert!(BOUNDARY_EVENTS.contains(&event), "{source} {event}");
        }
        // Every name the table can map is prefiltered in.
        for source in ["claude", "pi", "cursor"] {
            for event in BOUNDARY_EVENTS {
                let _ = boundary(source, event, Some("idle_prompt"));
            }
        }
        let mapped: Vec<&str> = BOUNDARY_EVENTS
            .iter()
            .copied()
            .filter(|e| ["claude", "pi", "cursor"].iter().any(|s| boundary(s, e, Some("idle_prompt")).is_some()))
            .collect();
        assert_eq!(mapped.len(), BOUNDARY_EVENTS.len(), "no dead names in the prefilter");
    }

    #[test]
    fn child_kinds_are_case_sensitive_per_source() {
        assert_eq!(child_kind("claude", "SubagentStart"), Some(Child::Start));
        assert_eq!(child_kind("claude", "SubagentStop"), Some(Child::Stop));
        assert_eq!(child_kind("cursor", "subagentStart"), Some(Child::Start));
        assert_eq!(child_kind("cursor", "subagentStop"), Some(Child::Stop));
        assert_eq!((child_kind("claude", "subagentStart"), child_kind("cursor", "SubagentStop"), child_kind("pi", "SubagentStart")), (None, None, None));
        for e in CHILD_EVENTS {
            assert!(["claude", "cursor"].iter().any(|s| child_kind(s, e).is_some()), "{e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn alive_checks_pid_and_start() {
        let me = i64::from(std::process::id());
        let start = proc_start(me);
        if cfg!(target_os = "macos") {
            let start = start.expect("own start");
            assert!(alive(me, start));
            assert!(!alive(me, start + 1000), "start mismatch = reused pid");
        }
        assert!(!alive(0, 0));
        assert!(!alive(-5, 0));
        assert!(!alive(i64::from(i32::MAX), 1));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_zombie_is_not_alive() {
        let mut child = std::process::Command::new("sh").args(["-c", "sleep 0.3"]).spawn().unwrap();
        let pid = i64::from(child.id());
        let start = proc_start(pid).expect("child start");
        assert!(alive(pid, start));
        // Exited but not yet waited for: the pid is still in the process table (a zombie, or already unreadable).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while alive(pid, start) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive(pid, start), "an exited, unreaped child is not alive");
        child.wait().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn find_harness_resolves_something_above_the_test() {
        // Under cargo test the parent chain exists; only shape is asserted.
        if let Some((pid, start)) = find_harness() {
            assert!(pid > 1 && start > 0 && alive(pid, start));
        }
    }
}
