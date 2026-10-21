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

/// (ppid, comm, start in Unix seconds) of a process.
#[cfg(target_os = "macos")]
fn proc_info(pid: i64) -> Option<(i64, String, i64)> {
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
    Some((i64::from(info.pbi_ppid), comm, i64::try_from(info.pbi_start_tvsec).ok()?))
}

#[cfg(not(target_os = "macos"))]
fn proc_info(_pid: i64) -> Option<(i64, String, i64)> {
    None
}

/// Start time of a process in Unix seconds.
pub fn proc_start(pid: i64) -> Option<i64> {
    proc_info(pid).map(|(_, _, start)| start)
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
            let Some((ppid, comm, start)) = proc_info(pid) else { break };
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

/// The process exists and is the one that started at `start` (a reused pid has a different start).
pub fn alive(pid: i64, start: i64) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid32) = i32::try_from(pid) else { return false };
        if pid32 <= 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the process can be signalled.
        let exists = unsafe { libc::kill(pid32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        exists && proc_start(pid) == Some(start)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, start);
        false
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
    fn find_harness_resolves_something_above_the_test() {
        // Under cargo test the parent chain exists; only shape is asserted.
        if let Some((pid, start)) = find_harness() {
            assert!(pid > 1 && start > 0 && alive(pid, start));
        }
    }
}
