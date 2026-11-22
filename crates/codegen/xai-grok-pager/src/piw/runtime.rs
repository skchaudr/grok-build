//! The pager's process-wide pi-workflows attachment: one shared client, one
//! mirror, and the merged run list the flow screens read.
//!
//! Nothing here blocks or errors on screen. When the machine has no
//! pi-workflows server socket, [`mirror`] stays `None` and every caller keeps
//! the plain acpx behaviour. [`ensure_mirror_started`] is called from the
//! app's one-second tick, so a server that appears later is picked up without
//! a pager restart; it is the only function that starts anything.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::acpx::bundle::{self, RunSummary};
use crate::piw::client::PiwClient;
use crate::piw::mirror::PiwMirror;

/// Where the pager writes its mirrored pi-workflows run folders:
/// `<grok home>/cache/pi-workflows-mirror`. Read by the flow screens through
/// the ordinary acpx readers, so nothing but this path is new to them.
pub fn mirror_root() -> Option<PathBuf> {
    let home = xai_grok_shell::util::grok_home::grok_home();
    (home.as_os_str() != ".").then(|| home.join("cache").join("pi-workflows-mirror"))
}

/// The shared client of this process, connected lazily to the machine's
/// default server socket. `None` when the socket does not exist: acpx keeps
/// working exactly as before. A client, once made, is never remade.
pub fn shared_client() -> Option<Arc<PiwClient>> {
    shared_client_at(PiwClient::default_socket_path().as_deref())
}

/// The same for an explicit socket path, so tests can pass a path that
/// cannot exist.
pub fn shared_client_at(socket: Option<&Path>) -> Option<Arc<PiwClient>> {
    static CLIENT: OnceLock<Option<Arc<PiwClient>>> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return client.clone();
    }
    let socket = socket?;
    if !socket.exists() {
        return None;
    }
    let client = Arc::new(PiwClient::connect(socket).ok()?);
    // A racing first caller keeps its own client; its drop closes it.
    let _ = CLIENT.set(Some(Arc::clone(&client)));
    Some(client)
}

/// The mirror this process is running, if it was ever started. Never starts
/// anything, so callers (and tests) can poll it freely.
pub fn mirror() -> Option<Arc<PiwMirror>> {
    MIRROR.get().cloned().flatten()
}

static MIRROR: OnceLock<Option<Arc<PiwMirror>>> = OnceLock::new();

/// Start the mirror if the server socket has appeared. Cheap when the mirror
/// already runs: one lock read. Called from the app's one-second tick.
pub fn ensure_mirror_started() -> Option<Arc<PiwMirror>> {
    if let Some(mirror) = mirror() {
        return Some(mirror);
    }
    let client = shared_client()?;
    let root = mirror_root()?;
    match PiwMirror::start(client, root) {
        Ok(mirror) => {
            let mirror = Arc::new(mirror);
            let _ = MIRROR.set(Some(Arc::clone(&mirror)));
            Some(mirror)
        }
        Err(err) => {
            tracing::debug!(%err, "failed to start the pi-workflows mirror");
            None
        }
    }
}

/// The run list the flow screens show: acpx runs and mirrored pi-workflows
/// runs together, newest first. Without a running mirror this is exactly
/// `bundle::list_runs`.
pub fn list_runs_with_mirror(base: Option<&Path>) -> Vec<RunSummary> {
    let mirror = mirror();
    match (base, mirror) {
        (Some(base), Some(mirror)) => list_runs_merged(Some(base), Some(mirror.root())),
        (Some(base), None) => bundle::list_runs(base),
        (None, Some(mirror)) => bundle::list_runs(mirror.root()),
        (None, None) => Vec::new(),
    }
}

/// Pure merge of two run directories, newest first (`bundle::list_runs` order).
pub fn list_runs_merged(
    acpx_base: Option<&Path>,
    mirror_root: Option<&Path>,
) -> Vec<RunSummary> {
    let mut runs = Vec::new();
    if let Some(base) = acpx_base {
        runs.extend(bundle::list_runs(base));
    }
    if let Some(root) = mirror_root {
        runs.extend(bundle::list_runs(root));
    }
    runs.sort_by(|a, b| {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_client_is_none_without_a_socket() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("no-server.sock");
        assert!(shared_client_at(Some(&missing)).is_none());
        assert!(shared_client_at(None).is_none());
    }

    #[test]
    fn mirror_never_started_keeps_plain_acpx_listing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("runs");
        std::fs::create_dir_all(&base).expect("create base");
        // No mirror was started in this test process, so the merged listing
        // must be exactly the acpx listing, whatever the machine's real
        // mirror directory holds.
        assert_eq!(list_runs_with_mirror(Some(&base)), bundle::list_runs(&base));
    }

    #[test]
    fn list_runs_merged_orders_newest_first_across_both_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let acpx_base = dir.path().join("acpx-runs");
        let mirror = dir.path().join("mirror-runs");
        std::fs::create_dir_all(acpx_base.join("older/projections")).expect("create older");
        std::fs::create_dir_all(mirror.join("newer/projections")).expect("create newer");
        std::fs::write(
            acpx_base.join("older/projections/live.json"),
            r#"{"runId":"older","flowName":"old-flow","status":"completed","startedAt":"2025-03-01T10:00:00Z","updatedAt":"2025-03-01T10:01:00Z"}"#,
        )
        .expect("write older live.json");
        std::fs::write(
            mirror.join("newer/projections/live.json"),
            r#"{"runId":"newer","flowName":"piw-flow","status":"running","startedAt":"2025-03-02T10:00:00Z","updatedAt":"2025-03-02T10:00:30Z"}"#,
        )
        .expect("write newer live.json");

        let runs = list_runs_merged(Some(&acpx_base), Some(&mirror));
        let ids: Vec<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(ids, vec!["newer", "older"]);
        assert_eq!(runs[0].flow_name, "piw-flow");
    }
}
