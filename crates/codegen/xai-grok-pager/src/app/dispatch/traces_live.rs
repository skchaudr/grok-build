//! Live follow in grok's own session view. A running session's transcript is converted again whenever it changes
//! (another machine's is copied over first), and only events the view has not had go through the session-event
//! handler. A call that was pending and has finished comes again under its id; grok's tracker upserts it in place.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use agent_client_protocol as acp;
use xai_grok_shell::extensions::notification::SessionUpdate as XaiSessionUpdate;
use xai_grok_traces::SessionRow;

use crate::acp::claude_feed::{Event, Fed};
use crate::app::agent::AgentId;
use crate::app::app_view::AppView;

struct Tail {
    agent: AgentId,
    session_id: String,
    rx: mpsc::Receiver<Vec<Fed>>,
    /// How many events with each signature the view has had.
    seen: HashMap<u64, usize>,
    stop: Arc<AtomicBool>,
}

static TAILS: Mutex<Vec<Tail>> = Mutex::new(Vec::new());

/// What makes an event the same event in the next conversion. A tool call is its id and status, so a finished call
/// is new; a spawn is its time, so a child id that resolves later does not draw a second subagent row.
fn sig(f: &Fed) -> u64 {
    let mut h = DefaultHasher::new();
    let ts = f.meta.get("agentTimestampMs").map(|v| v.to_string());
    match &f.event {
        Event::Acp(acp::SessionUpdate::ToolCall(tc)) => {
            ("tool", tc.tool_call_id.0.as_ref(), format!("{:?}", tc.status)).hash(&mut h)
        }
        Event::Xai(XaiSessionUpdate::SubagentSpawned { .. }) => ("spawn", &ts).hash(&mut h),
        Event::Xai(XaiSessionUpdate::SubagentFinished { subagent_id, .. }) => ("done", subagent_id).hash(&mut h),
        Event::Acp(u) => (serde_json::to_string(u).unwrap_or_default(), &ts).hash(&mut h),
        Event::Xai(u) => (serde_json::to_string(u).unwrap_or_default(), &ts).hash(&mut h),
    }
    h.finish()
}

/// Events of `batch` the view has not had yet, in order; `seen` counts them in.
fn unseen(seen: &mut HashMap<u64, usize>, batch: Vec<Fed>) -> Vec<Fed> {
    let mut counts: HashMap<u64, usize> = HashMap::new();
    let mut out = Vec::new();
    for f in batch {
        let k = sig(&f);
        let n = counts.entry(k).or_default();
        *n += 1;
        if *n > seen.get(&k).copied().unwrap_or(0) {
            seen.insert(k, *n);
            out.push(f);
        }
    }
    out
}

pub(crate) fn signatures(events: &[Fed]) -> HashMap<u64, usize> {
    let mut seen = HashMap::new();
    for f in events {
        *seen.entry(sig(f)).or_default() += 1;
    }
    seen
}

/// Start tailing `row` into `agent`, which already shows the events counted in `seen`.
pub(crate) fn start(agent: AgentId, session_id: String, row: SessionRow, seen: HashMap<u64, usize>) {
    stop(agent);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let flag = stop_flag.clone();
    std::thread::spawn(move || {
        let every = if row.host.is_some() { Duration::from_secs(3) } else { Duration::from_secs(1) };
        let mut last = stamp(&row.file_path, &row);
        while !flag.load(Ordering::Acquire) {
            std::thread::sleep(every);
            let path = match local_copy(&row) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let now = stamp(&path, &row);
            if now == last {
                continue;
            }
            last = now;
            if let Ok(feed) = super::standalone_traces::feed_for(&row, &path)
                && tx.send(feed).is_err()
            {
                break;
            }
        }
    });
    TAILS.lock().unwrap().push(Tail { agent, session_id, rx, seen, stop: stop_flag });
}

/// Size and mtime of the transcript and, for Claude, of its `subagents/` folder.
fn stamp(path: &Path, row: &SessionRow) -> Vec<(u64, Option<SystemTime>)> {
    let of = |p: &Path| std::fs::metadata(p).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
    let mut s = vec![of(path)];
    if row.source == "claude" {
        s.push(of(&path.with_extension("").join("subagents")));
    }
    s
}

pub(crate) fn stop(agent: AgentId) {
    TAILS.lock().unwrap().retain(|t| {
        let keep = t.agent != agent;
        if !keep {
            t.stop.store(true, Ordering::Release);
        }
        keep
    });
}

pub(crate) fn tailing() -> bool {
    !TAILS.lock().unwrap().is_empty()
}

/// Feed every tail's newest conversion. True when anything reached a view.
pub(crate) fn tick(app: &mut AppView) -> bool {
    let mut batches = Vec::new();
    {
        let mut tails = TAILS.lock().unwrap();
        tails.retain(|t| {
            let alive = app.agents.get(&t.agent).is_some_and(|a| {
                a.session.session_id.as_ref().is_some_and(|s| s.0.as_ref() == t.session_id)
            });
            if !alive {
                t.stop.store(true, Ordering::Release);
            }
            alive
        });
        for t in tails.iter_mut() {
            let Some(latest) = t.rx.try_iter().last() else { continue };
            let fresh = unseen(&mut t.seen, latest);
            if !fresh.is_empty() {
                batches.push((t.agent, t.session_id.clone(), fresh));
            }
        }
    }
    let any = !batches.is_empty();
    for (agent, sid, events) in batches {
        if let Some(a) = app.agents.get_mut(&agent) {
            a.scrollback.begin_batch();
        }
        for mut fed in events {
            // As live updates: a replay after the load window is dropped, and so is an event id at or below the last one.
            fed.meta.remove("isReplay");
            fed.meta.remove("eventId");
            crate::app::acp_handler::handle(super::standalone_traces::message(&sid, fed), app);
        }
        if let Some(a) = app.agents.get_mut(&agent) {
            a.scrollback.end_batch();
        }
    }
    any
}

/// The transcript as a local file. Another machine's is copied (with Claude's `subagents/` and `tool-results/`, or
/// grok's whole session folder) under `<traces dir>/remote/<host>/`, mirroring its path there.
pub(crate) fn local_copy(row: &SessionRow) -> io::Result<PathBuf> {
    let Some(host) = row.host.as_deref() else { return Ok(row.file_path.clone()) };
    let target = xai_grok_traces::remote::hosts()
        .into_iter()
        .find(|t| xai_grok_traces::remote::label(t) == host)
        .ok_or_else(|| io::Error::other(format!("{host} is not in the traces hosts file")))?;
    let base = xai_grok_traces::default_db_path().parent().map(Path::to_path_buf).unwrap_or_default().join("remote").join(host);
    let local = base.join(row.file_path.strip_prefix("/").unwrap_or(&row.file_path));
    let parent = local.parent().map(Path::to_path_buf).unwrap_or_else(|| base.clone());
    std::fs::create_dir_all(&parent)?;
    let rsync = |src: &Path, dir: bool, dst: &Path| -> io::Result<bool> {
        let src = format!("{target}:{}{}", src.display(), if dir { "/" } else { "" });
        let st = Command::new("rsync")
            .args(["-az", "-s", "--timeout=10", "-e", "ssh -o BatchMode=yes -o ConnectTimeout=3"])
            .arg(src)
            .arg(dst)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        Ok(st.success())
    };
    if row.source == "grok" {
        if let Some(dir) = row.file_path.parent() {
            rsync(dir, true, &parent)?;
        }
    } else {
        rsync(&row.file_path, false, &local)?;
        if row.source == "claude" {
            // Absent for a session with no subagents or stored outputs; that is not a failure.
            let _ = rsync(&row.file_path.with_extension(""), true, &local.with_extension(""));
        }
    }
    if local.exists() { Ok(local) } else { Err(io::Error::other(format!("could not copy it from {host}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn call(id: &str, status: acp::ToolCallStatus) -> Fed {
        let tc = acp::ToolCall::new(acp::ToolCallId::new(id), "Bash").status(status);
        Fed { event: Event::Acp(acp::SessionUpdate::ToolCall(tc)), meta: Map::new() }
    }

    fn text(t: &str) -> Fed {
        let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(t.to_string())));
        Fed { event: Event::Acp(acp::SessionUpdate::AgentMessageChunk(chunk)), meta: Map::new() }
    }

    #[test]
    fn only_new_events_and_finished_calls_pass() {
        let first = vec![text("a"), call("t1", acp::ToolCallStatus::InProgress)];
        let mut seen = signatures(&first);
        let next = vec![text("a"), call("t1", acp::ToolCallStatus::Completed), text("a"), text("b")];
        let fresh = unseen(&mut seen, next);
        let kinds: Vec<String> = fresh
            .iter()
            .map(|f| match &f.event {
                Event::Acp(acp::SessionUpdate::ToolCall(tc)) => format!("{:?}", tc.status),
                Event::Acp(acp::SessionUpdate::AgentMessageChunk(_)) => "text".into(),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(kinds, ["Completed", "text", "text"], "the finished call and the two new texts, the repeated 'a' once");
        assert!(unseen(&mut seen, vec![text("a"), text("a"), text("b")]).is_empty());
    }
}
