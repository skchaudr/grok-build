//! Hook-to-viewer wake signal: the hook sends one empty datagram to `<traces dir>/wake*.sock`, viewers block on it.
//! A datagram socket has one receiver, so a second `Waker` binds `wake.<pid>.sock`; `notify` sends to every `wake*.sock`.
//! Best effort by design: viewers keep a slow timer as fallback, and `notify` swallows every error.

use std::io::ErrorKind;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::default_db_path;

fn is_wake_sock(name: &str) -> bool {
    name.starts_with("wake") && name.ends_with(".sock")
}

/// Directory holding the sockets: next to the index.
pub fn dir() -> PathBuf {
    default_db_path().parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Sends to every `wake*.sock` in `dir`, deleting dead ones; returns how many were reached.
fn send_all(dir: &Path) -> usize {
    let Ok(sender) = UnixDatagram::unbound() else { return 0 };
    let _ = sender.set_nonblocking(true);
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut reached = 0;
    for e in entries.flatten().filter(|e| e.file_name().to_str().is_some_and(is_wake_sock)) {
        match sender.send_to(&[], e.path()) {
            Ok(_) => reached += 1,
            Err(err) if matches!(err.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound) => {
                let _ = std::fs::remove_file(e.path());
            }
            Err(_) => {} // full buffer (WouldBlock): the listener has a wake pending already
        }
    }
    reached
}

/// Wakes every listening viewer. Never blocks, never creates a socket, ignores all errors.
pub fn notify() {
    send_all(&dir());
}

/// Number of live listeners (doctor).
pub fn listeners() -> usize {
    send_all(&dir())
}

pub struct Waker {
    sock: Option<UnixDatagram>,
    path: PathBuf,
}

impl Waker {
    /// Binds `wake.sock`, or `wake.<pid>.sock` when a live process holds it. Without a socket, `wait` just sleeps.
    pub fn bind(dir: &Path) -> Waker {
        let _ = std::fs::create_dir_all(dir);
        let primary = dir.join("wake.sock");
        let own = dir.join(format!("wake.{}.sock", std::process::id()));
        let sock = match UnixDatagram::bind(&primary) {
            Ok(s) => Some((s, primary)),
            Err(e) if e.kind() == ErrorKind::AddrInUse => {
                // Live holder: step aside. Dead holder: the probe removes its file, so take it over.
                let _ = std::fs::remove_file(&own);
                if UnixDatagram::unbound().and_then(|s| s.send_to(&[], &primary)).is_ok() {
                    UnixDatagram::bind(&own).ok().map(|s| (s, own))
                } else {
                    let _ = std::fs::remove_file(&primary);
                    UnixDatagram::bind(&primary).ok().map(|s| (s, primary))
                }
            }
            Err(_) => None,
        };
        match sock {
            Some((s, path)) => Waker { sock: Some(s), path },
            None => Waker { sock: None, path: PathBuf::new() },
        }
    }

    pub fn bind_default() -> Waker {
        Self::bind(&dir())
    }

    /// True when a datagram arrived (a burst counts once), false on timeout.
    pub fn wait(&self, timeout: Duration) -> bool {
        let Some(s) = &self.sock else {
            std::thread::sleep(timeout);
            return false;
        };
        let _ = s.set_nonblocking(false);
        let _ = s.set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
        let woke = s.recv(&mut [0u8; 8]).is_ok();
        if woke {
            self.drain();
        }
        woke
    }

    /// Discards every queued datagram.
    pub fn drain(&self) {
        if let Some(s) = &self.sock {
            let _ = s.set_nonblocking(true);
            while s.recv(&mut [0u8; 8]).is_ok() {}
        }
    }
}

impl Drop for Waker {
    fn drop(&mut self) {
        if self.sock.is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const T: Duration = Duration::from_millis(500);

    #[test]
    fn round_trip_and_burst() {
        let d = tempfile::tempdir().unwrap();
        let w = Waker::bind(d.path());
        assert_eq!(send_all(d.path()), 1);
        send_all(d.path());
        send_all(d.path());
        assert!(w.wait(T));
        assert!(!w.wait(Duration::from_millis(50)), "burst drained");
    }

    #[test]
    fn timeout_is_false() {
        let d = tempfile::tempdir().unwrap();
        let w = Waker::bind(d.path());
        let t = Instant::now();
        assert!(!w.wait(Duration::from_millis(80)));
        assert!(t.elapsed() >= Duration::from_millis(70));
    }

    #[test]
    fn two_wakers_both_wake() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (Waker::bind(d.path()), Waker::bind(d.path()));
        assert_ne!(a.path, b.path);
        assert_eq!(send_all(d.path()), 2);
        assert!(a.wait(T) && b.wait(T));
    }

    #[test]
    fn stale_socket_is_replaced_and_swept() {
        let d = tempfile::tempdir().unwrap();
        // A bound-then-leaked socket file with no process behind it.
        drop(UnixDatagram::bind(d.path().join("wake.sock")).unwrap());
        drop(UnixDatagram::bind(d.path().join("wake.99999.sock")).unwrap());
        assert_eq!(send_all(d.path()), 0);
        assert!(!d.path().join("wake.99999.sock").exists(), "stale sibling swept");
        drop(UnixDatagram::bind(d.path().join("wake.sock")).unwrap());
        let w = Waker::bind(d.path());
        assert_eq!(w.path, d.path().join("wake.sock"), "stale primary taken over");
        send_all(d.path());
        assert!(w.wait(T));
    }

    #[test]
    fn notify_never_creates_a_socket() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(send_all(d.path()), 0);
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }
}
