//! `grok traces`: the pager starts straight into the traces modal with no agent session behind it.
//! The mode is process-wide and set once by the CLI, so a static is enough.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use agent_client_protocol as acp;
use xai_acp_lib::AcpClientMessage;
use xai_grok_shell::extensions::notification::SessionNotification;
use xai_grok_shell::session::storage::{
    ReplayEmission, ReplayLookupFallback, ReplayPathHint, ReplayedUpdate, stream_replay_updates_at_hinted,
};

use crate::acp::claude_feed::{self, Event, Fed};
use crate::acp::pi_feed;
use crate::acp::codex_feed;
use crate::acp::tracker::AcpUpdateTracker;
use crate::app::actions::{Action, Effect};
use crate::app::agent::{AgentId, AgentSession, AgentState};
use crate::app::agent_view::AgentView;
use crate::app::app_view::{ActiveView, AppView, AuthState, TrustState};
use crate::scrollback::state::ScrollbackState;
use crate::app::consent::ConsentState;
use crate::views::modal::ActiveModal;

static STANDALONE: AtomicBool = AtomicBool::new(false);
/// Session id (prefix) still to open once the index can answer for it.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

pub const TITLE: &str = "traces";

pub fn enable(session: Option<String>) {
    STANDALONE.store(true, Ordering::Release);
    *PENDING.lock().unwrap() = session.filter(|s| !s.trim().is_empty());
}

pub fn is_standalone() -> bool {
    STANDALONE.load(Ordering::Acquire)
}

/// Build the agent view the modal lives on without sending `session/new`, then open the modal on it.
/// Trust, consent and login gate sessions; the traces modal reads local files only, so none applies.
pub(crate) fn start(app: &mut AppView) -> Vec<Effect> {
    app.trust_state = TrustState::Done;
    app.consent_state = ConsentState::Done;
    app.auth_state = AuthState::Done;
    let (agent_id, _create_session) =
        super::session::lifecycle::dispatch_new_session_inner_with_id(app, None, false);
    if let Some(agent) = app.agents.get_mut(&agent_id) {
        agent.session_starting_since = None;
        agent.session.prompt_history_loading = false;
    }
    let effects = super::dispatch(Action::OpenTraces, app);
    open_pending(app, false);
    effects
}

/// Open the requested session once it is indexed. After the scan finished, a miss is reported instead.
pub(crate) fn open_pending(app: &mut AppView, scan_done: bool) {
    let mut pending = PENDING.lock().unwrap();
    let Some(prefix) = pending.clone() else { return };
    let opened = {
        let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
        let Some(ActiveModal::Traces { state }) = agent.active_modal.as_mut() else { return };
        match state.open_by_prefix(&prefix) {
            Ok(row) => {
                *pending = None;
                Some(row)
            }
            Err(msg) if scan_done => {
                state.status = Some(msg);
                *pending = None;
                None
            }
            Err(_) => None,
        }
    };
    drop(pending);
    let Some(row) = opened else { return };
    if row.source == "grok" {
        open_grok(app, row);
    } else {
        open_claude(app, row);
    }
}

/// Enter on a Claude, Pi or Codex session in the sessions list: the transcript, converted, goes through the session-event handler into a
/// read-only view (no composer) as a replay; the sessions list waits in `trace_return` for Esc.
pub(crate) fn open_claude(app: &mut AppView, row: xai_grok_traces::SessionRow) {
    open_claude_in(app, row, is_standalone());
}

/// `grok traces` replays into its own view. Inside a grok session the replay gets a new agent view, so the user's session
/// is left as it was. A row from another machine is copied here first; a live row keeps being followed.
pub(crate) fn open_claude_in(app: &mut AppView, row: xai_grok_traces::SessionRow, standalone: bool) {
    let local = match local_copy(&row) {
        Ok(p) => p,
        Err(e) => return report(app, &row, e),
    };
    let feed = feed_for(&row, &local);
    open_feed_in(app, row.id.clone(), row, local, feed, standalone);
}

/// Enter on a past grok session: same read-only view as Claude.
pub(crate) fn open_grok(app: &mut AppView, row: xai_grok_traces::SessionRow) {
    open_grok_in(app, row, is_standalone());
}

/// grok's `updates.jsonl` is already grok's events: no converter, replayed with the xAI updates kept so subagents survive.
/// The row's file is `<grok home>/sessions/<cwd>/<session id>/chat_history.jsonl`.
pub(crate) fn open_grok_in(app: &mut AppView, row: xai_grok_traces::SessionRow, standalone: bool) {
    // Bound to the index's `grok:` id, not grok's own: a tab with this session open must not get the replay.
    // A row from another machine is copied first (session dir, so `updates.jsonl` comes with it).
    let local = match local_copy(&row) {
        Ok(p) => p,
        Err(e) => return report(app, &row, e),
    };
    let feed = feed_for(&row, &local);
    open_feed_in(app, row.id.clone(), row, local, feed, standalone);
}

/// The row's events, converted from its transcript at `path` (the row's own file, or the local copy of a remote one).
fn feed_for(row: &xai_grok_traces::SessionRow, path: &Path) -> io::Result<Vec<Fed>> {
    match row.source.as_str() {
        "pi" => pi_feed::convert(path, &row.id),
        "dpi" => pi_feed::convert_dpi(&path.to_string_lossy(), &row.id),
        "codex" => codex_feed::convert(path, &row.id),
        "grok" => {
            let dir = path.parent();
            let sid = dir.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
            let home = dir
                .and_then(|d| d.ancestors().find(|a| a.file_name().is_some_and(|n| n == "sessions")))
                .and_then(Path::parent);
            // The session folder's own cwd finds `updates.jsonl` without scanning every session.
            let folder_cwd = dir.and_then(Path::parent).and_then(xai_grok_config::decode_cwd_from_dirname);
            match (&sid, home) {
                (Some(sid), Some(home)) => grok_feed(sid, home, folder_cwd.as_deref()),
                _ => Err(io::Error::other("not in a grok sessions directory")),
            }
        }
        "claude" => claude_feed::convert(path, &row.id),
        _ => plain_events(row, path),
    }
}

/// Harnesses without their own converter: the index (and the raw line behind each event) on this machine,
/// the copied transcript when it is JSONL, otherwise one `events` read over ssh.
fn plain_events(row: &xai_grok_traces::SessionRow, path: &Path) -> io::Result<Vec<Fed>> {
    let live = row.status == xai_grok_traces::Status::Active;
    if row.host.is_none() {
        let idx = xai_grok_traces::TraceIndex::open(&xai_grok_traces::default_db_path()).map_err(|e| io::Error::other(e.to_string()))?;
        // shortcut: the first 20_000 events; a longer session's tail is not followed, raise this if that shows up
        let events = idx
            .list_events(&row.id, &xai_grok_traces::EventFilter { limit: 20_000, ..Default::default() })
            .map_err(|e| io::Error::other(e.to_string()))?;
        let texts: Vec<String> = events.iter().map(|e| event_text(&idx, &row.id, e)).collect();
        return Ok(crate::acp::plain_feed::from_events(&events, &texts, live));
    }
    if looks_like_jsonl(path) {
        let raw = std::fs::read_to_string(path)?;
        return Ok(crate::acp::plain_feed::from_jsonl(&raw, live));
    }
    let target = target_of(row).ok_or_else(|| io::Error::other("no ssh target for this host in ~/.grok/traces/hosts"))?;
    let events = xai_grok_traces::remote::fetch_events(
        &target,
        &row.id,
        &xai_grok_traces::remote::EventQuery { limit: 20_000, ..Default::default() },
    )
    .map_err(|e| io::Error::other(e.to_string()))?;
    let texts: Vec<String> = events.iter().map(|e| {
        let detail = e.detail.as_deref().unwrap_or("");
        if e.preview.is_empty() { e.title.clone() } else if detail.is_empty() { e.preview.clone() } else { format!("{} · {detail}", e.preview) }
    }).collect();
    Ok(crate::acp::plain_feed::from_events(&events, &texts, live))
}

fn event_text(idx: &xai_grok_traces::TraceIndex, id: &str, e: &xai_grok_traces::EventRow) -> String {
    idx.raw_line(id, e.line)
        .ok()
        .flatten()
        .map(|raw| crate::views::traces_modal::body_text(&raw))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| if e.preview.is_empty() { e.title.clone() } else { e.preview.clone() })
}

fn looks_like_jsonl(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    let mut line = String::new();
    if std::io::BufRead::read_line(&mut std::io::BufReader::new(file), &mut line).is_err() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(line.trim()).is_ok()
}

/// The footer names the harness (and the other machine) when the transcript itself names no model.
fn trace_label(row: &xai_grok_traces::SessionRow) -> String {
    let mut parts = vec![xai_grok_traces::source_label(&row.source).to_string()];
    if let Some(m) = row.model.as_deref().filter(|m| !m.is_empty()) {
        parts.push(m.to_string());
    }
    if let Some(h) = row.host.as_deref() {
        parts.push(h.to_string());
    }
    parts.join(" · ")
}

/// The ssh target (`user@host`) for a remote row's host label.
fn target_of(row: &xai_grok_traces::SessionRow) -> Option<String> {
    let host = row.host.as_deref()?;
    xai_grok_traces::remote::hosts().into_iter().find(|t| xai_grok_traces::remote::label(t) == host)
}

/// A local file to convert: the row's own, or for a row from another machine a copy fetched over ssh (rsync) into the temp dir.
/// A Claude transcript brings its `<session>/` folder too, for subagents and stored tool results.
/// grok copies the session directory (it holds `updates.jsonl`); dpi and bb copy the sqlite and keep the `#id` key.
fn local_copy(row: &xai_grok_traces::SessionRow) -> io::Result<PathBuf> {
    if row.host.is_none() {
        return Ok(row.file_path.clone());
    }
    let target = target_of(row).ok_or_else(|| io::Error::other("no ssh target for this host in ~/.grok/traces/hosts"))?;
    let host = row.host.clone().unwrap_or_default();
    let root = std::env::temp_dir().join("grok-traces-remote").join(&host);
    if row.file_path.as_os_str().is_empty() {
        return Ok(root.join(format!("remote-{}", row.id.replace('/', "_"))));
    }
    match row.source.as_str() {
        "grok" => copy_grok_dir(&target, &host, &root, row),
        "dpi" => copy_keyed(&target, &host, &root.join("dpi"), &row.file_path.to_string_lossy(), xai_grok_traces::dpi::split_key),
        "bb" => copy_bb(&target, &host, &root.join("bb"), &row.file_path.to_string_lossy()),
        _ => copy_file(&target, &host, &root, row),
    }
}

fn rsync(srcs: &[String], dest: &Path, host: &str) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let out = std::process::Command::new("rsync")
        .args(["-a", "--timeout=8", "-e", "ssh -o BatchMode=yes -o ConnectTimeout=3"])
        .args(srcs)
        .arg(format!("{}/", dest.display()))
        .stdin(std::process::Stdio::null())
        .output()?;
    if out.status.success() || out.status.code() == Some(23) {
        return Ok(());
    }
    Err(io::Error::other(format!("{host} unreachable: {}", String::from_utf8_lossy(&out.stderr).trim())))
}

fn copy_file(target: &str, host: &str, root: &Path, row: &xai_grok_traces::SessionRow) -> io::Result<PathBuf> {
    let dest = root.join(row.file_path.parent().map(|p| p.to_string_lossy().trim_start_matches('/').replace('/', "_")).unwrap_or_default());
    let remote = row.file_path.to_string_lossy().into_owned();
    let mut srcs = vec![format!("{target}:{remote}")];
    if row.source == "claude" {
        srcs.push(format!("{target}:{}", row.file_path.with_extension("").to_string_lossy()));
    }
    rsync(&srcs, &dest, host)?;
    let local = dest.join(row.file_path.file_name().unwrap_or_default());
    if !local.exists() {
        return Err(io::Error::other(format!("{host} unreachable: transcript did not copy")));
    }
    Ok(local)
}

/// The session directory, under a `sessions/` ancestor, so grok's replay finds `updates.jsonl` and the cwd slug.
fn copy_grok_dir(target: &str, host: &str, root: &Path, row: &xai_grok_traces::SessionRow) -> io::Result<PathBuf> {
    let session_dir = row.file_path.parent().ok_or_else(|| io::Error::other("grok session has no directory"))?;
    let slug = session_dir.parent().and_then(|p| p.file_name()).unwrap_or(std::ffi::OsStr::new("cwd"));
    let sid = session_dir.file_name().unwrap_or(std::ffi::OsStr::new("session"));
    let dest = root.join("sessions").join(slug).join(sid);
    rsync(&[format!("{target}:{}/", session_dir.display())], &dest, host)?;
    let local = dest.join(row.file_path.file_name().unwrap_or_default());
    if !local.exists() {
        return Err(io::Error::other(format!("{host} unreachable: grok session did not copy")));
    }
    Ok(local)
}

fn copy_keyed(target: &str, host: &str, dest: &Path, key: &str, split: fn(&str) -> Option<(&str, i64)>) -> io::Result<PathBuf> {
    let (db, conv) = split(key).ok_or_else(|| io::Error::other(format!("not a session key: {key}")))?;
    let name = Path::new(db).file_name().unwrap_or_default();
    rsync(&[format!("{target}:{db}")], dest, host)?;
    let local = dest.join(name);
    if !local.exists() {
        return Err(io::Error::other(format!("{host} unreachable: database did not copy")));
    }
    Ok(PathBuf::from(format!("{}#{conv}", local.display())))
}

fn copy_bb(target: &str, host: &str, dest: &Path, key: &str) -> io::Result<PathBuf> {
    let (db, thread) = xai_grok_traces::bb::split_key(key).ok_or_else(|| io::Error::other(format!("not a bb key: {key}")))?;
    let name = Path::new(db).file_name().unwrap_or_default();
    rsync(&[format!("{target}:{db}")], dest, host)?;
    let local = dest.join(name);
    if !local.exists() {
        return Err(io::Error::other(format!("{host} unreachable: database did not copy")));
    }
    Ok(PathBuf::from(format!("{}#{thread}", local.display())))
}

fn report(app: &mut AppView, row: &xai_grok_traces::SessionRow, e: io::Error) {
    if let Some(agent) = super::ctx::get_active_agent_mut(app)
        && let Some(ActiveModal::Traces { state }) = agent.active_modal.as_mut()
    {
        state.status = Some(format!("can't open {}: {e}", row.title));
    }
}

fn grok_feed(sid: &str, home: &Path, cwd: Option<&str>) -> io::Result<Vec<Fed>> {
    let hint = ReplayPathHint {
        parent_cwd: cwd.map(Path::new),
        child_cwd: None,
        // The folder is known, so a miss means no `updates.jsonl`; scan only when its cwd can't be read.
        fallback: if cwd.is_some() { ReplayLookupFallback::HintedOnly } else { ReplayLookupFallback::Relocation },
    };
    let mut out = Vec::new();
    let emission = stream_replay_updates_at_hinted(sid, home, hint, |update| {
        let (event, meta) = match update {
            ReplayedUpdate::Acp(update, meta) => (Event::Acp(update), meta.unwrap_or_default()),
            ReplayedUpdate::Xai(update) => (Event::Xai(update), Default::default()),
        };
        let mut meta = meta;
        meta.insert("isReplay".into(), true.into());
        out.push(Fed { event, meta });
    })?;
    if emission == ReplayEmission::Empty {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no updates.jsonl to replay"));
    }
    Ok(out)
}

fn open_feed_in(
    app: &mut AppView,
    session_id: String,
    row: xai_grok_traces::SessionRow,
    local: PathBuf,
    feed: io::Result<Vec<Fed>>,
    standalone: bool,
) {
    let ActiveView::Agent(origin) = app.active_view else { return };
    let Some(agent) = app.agents.get_mut(&origin) else { return };
    let events = match feed {
        Ok(events) => events,
        Err(e) => {
            if let Some(ActiveModal::Traces { state }) = agent.active_modal.as_mut() {
                state.status = Some(format!("can't read {}: {e}", row.file_path.display()));
            }
            return;
        }
    };
    let Some(ActiveModal::Traces { state }) = agent.active_modal.take() else { return };
    let agent = if standalone {
        agent.clear_trace_content();
        agent
    } else {
        let id = AgentId(app.next_agent_id);
        app.next_agent_id += 1;
        let view = read_only_view(app, id);
        app.agents.insert(id, view);
        super::ctx::switch_to_agent(app, id, super::ctx::SwitchCause::Load);
        let agent = app.agents.get_mut(&id).expect("just inserted");
        agent.trace_origin = Some(origin);
        agent
    };
    agent.trace_return = Some(state);
    // Esc back to the list needs the scrollback focused; in simple mode nothing else moves it there.
    agent.set_active_pane(crate::app::agent_view::AgentPane::Scrollback, true);
    agent.bind_session_id(acp::SessionId::new(session_id.clone()));
    if let Some(cwd) = row.cwd.as_deref() {
        agent.session.cwd = cwd.into();
        agent.session.tracker.set_session_cwd(cwd);
    }
    agent.begin_replay_window();
    agent.scrollback.begin_batch();
    let live = row.status == xai_grok_traces::Status::Active;
    let follow = live.then(|| Follow::new(&row, &local, &session_id, &events));
    for fed in events {
        super::super::acp_handler::handle(message(&session_id, fed), app);
    }
    let ActiveView::Agent(view) = app.active_view else { return };
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.scrollback.end_batch();
    agent.session.loading_replay = false;
    if agent.session.models.footer_label().is_none() {
        agent.session.models.set_served_model_name(Some(trace_label(&row)));
    }
    *FOLLOW.lock().unwrap() = follow.map(|f| Follow { agent: view, ..f });
    if live {
        // Still running: its last calls stay open, and grok's own follow keeps the tail in view until you scroll up.
        agent.scrollback.enable_follow_mode();
        return;
    }
    agent.scrollback.clear_running_clocks();
    agent.session.finish_turn(&mut agent.scrollback);
    agent.scrollback.finish_all_running();
    agent.mark_turn_finished(crate::app::cancel_latency::TurnEnd::Aborted);
}

/// The live session open in the read-only view. Each poll re-reads its transcript when the file grew and feeds what is new
/// through the handler: events past `fed`, and calls fed as running that now have their result (the tracker updates those in place).
struct Follow {
    agent: AgentId,
    session_id: String,
    row: xai_grok_traces::SessionRow,
    local: PathBuf,
    fed: usize,
    running: HashSet<String>,
    size: u64,
    /// No local transcript to watch: each finished copy re-reads the remote event list.
    events_over_ssh: bool,
    /// A remote row: the copy refreshing in the background, and when the last one finished.
    sync: Option<mpsc::Receiver<io::Result<PathBuf>>>,
    synced_at: Instant,
}

static FOLLOW: Mutex<Option<Follow>> = Mutex::new(None);
const REMOTE_SYNC_EVERY: Duration = Duration::from_secs(2);

impl Follow {
    fn new(row: &xai_grok_traces::SessionRow, local: &Path, session_id: &str, events: &[Fed]) -> Self {
        Follow {
            agent: AgentId(0),
            session_id: session_id.to_string(),
            row: row.clone(),
            local: local.to_path_buf(),
            fed: events.len(),
            running: events.iter().filter_map(running_call).collect(),
            size: watched_size(row, local),
            events_over_ssh: ssh_events(row, local),
            sync: None,
            synced_at: Instant::now(),
        }
    }
}

/// Remote sessions whose transcript is not a file we can re-read (a sqlite key, or no file at all).
fn ssh_events(row: &xai_grok_traces::SessionRow, local: &Path) -> bool {
    row.host.is_some() && !matches!(row.source.as_str(), "claude" | "pi" | "dpi" | "codex" | "grok") && !looks_like_jsonl(local)
}

/// A tool call still waiting for its result.
fn running_call(fed: &Fed) -> Option<String> {
    match &fed.event {
        Event::Acp(acp::SessionUpdate::ToolCall(tc))
            if matches!(tc.status, acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress) =>
        {
            Some(tc.tool_call_id.0.to_string())
        }
        _ => None,
    }
}

fn call_id(fed: &Fed) -> Option<String> {
    match &fed.event {
        Event::Acp(acp::SessionUpdate::ToolCall(tc)) => Some(tc.tool_call_id.0.to_string()),
        _ => None,
    }
}

/// Bytes of what the converter reads: the transcript, plus `updates.jsonl` beside a grok one.
fn watched_size(row: &xai_grok_traces::SessionRow, local: &Path) -> u64 {
    let spec = local.to_string_lossy();
    let file: &Path = match spec.split_once('#') {
        Some((db, _)) => Path::new(db),
        None => local,
    };
    let len = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let extra = if row.source == "grok" { file.parent().map_or(0, |d| len(&d.join("updates.jsonl"))) } else { 0 };
    len(file) + extra
}

/// Whether a live session is open, so the app keeps polling.
pub(crate) fn following() -> bool {
    FOLLOW.lock().unwrap().is_some()
}

/// The app's traces poll while a live session is open: feed what the transcript gained. Returns whether to redraw.
pub(crate) fn poll_follow(app: &mut AppView) -> bool {
    let mut guard = FOLLOW.lock().unwrap();
    let Some(f) = guard.as_mut() else { return false };
    let bound = app.agents.get(&f.agent).and_then(|a| a.session.session_id.as_ref()).is_some_and(|s| *s.0 == *f.session_id);
    if !bound {
        *guard = None;
        return false;
    }
    if app.active_view != ActiveView::Agent(f.agent) {
        return false;
    }
    if f.row.host.is_some() {
        match f.sync.as_ref().map(mpsc::Receiver::try_recv) {
            None if f.synced_at.elapsed() >= REMOTE_SYNC_EVERY => {
                let (tx, rx) = mpsc::channel();
                let row = f.row.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(local_copy(&row));
                });
                f.sync = Some(rx);
                return false;
            }
            Some(Ok(Ok(path))) => {
                f.local = path;
                f.sync = None;
                f.synced_at = Instant::now();
            }
            Some(Ok(Err(_))) | Some(Err(mpsc::TryRecvError::Disconnected)) => {
                f.sync = None;
                f.synced_at = Instant::now();
                return false;
            }
            _ => return false,
        }
    }
    let pull = f.events_over_ssh;
    let size = watched_size(&f.row, &f.local);
    if !pull && size == f.size {
        return false;
    }
    f.size = size;
    let Ok(events) = feed_for(&f.row, &f.local) else { return false };
    let total = events.len();
    let mut send = Vec::new();
    for (i, fed) in events.into_iter().enumerate() {
        let id = call_id(&fed);
        let still = running_call(&fed).is_some();
        if i >= f.fed {
            if still && let Some(id) = &id {
                f.running.insert(id.clone());
            }
            send.push(fed);
        } else if !still && id.as_ref().is_some_and(|id| f.running.remove(id)) {
            send.push(fed);
        }
    }
    f.fed = total;
    let session_id = f.session_id.clone();
    drop(guard);
    if send.is_empty() {
        return false;
    }
    if let Some(agent) = super::ctx::get_active_agent_mut(app) {
        agent.session.loading_replay = true;
    }
    for fed in send {
        super::super::acp_handler::handle(message(&session_id, fed), app);
    }
    if let Some(agent) = super::ctx::get_active_agent_mut(app) {
        agent.session.loading_replay = false;
    }
    true
}

/// Esc in the read-only view: put the sessions list back, on the session it was opened from when there is one.
pub(crate) fn leave_view(app: &mut AppView) {
    let ActiveView::Agent(id) = app.active_view else { return };
    if FOLLOW.lock().unwrap().as_ref().is_some_and(|f| f.agent == id) {
        *FOLLOW.lock().unwrap() = None;
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    let Some(state) = agent.trace_return.take() else { return };
    if let Some(origin) = agent.trace_origin {
        // The origin may have been closed meanwhile: another session takes the list, else the welcome screen.
        let back = Some(origin)
            .filter(|o| app.agents.contains_key(o))
            .or_else(|| app.agents.keys().copied().find(|k| *k != id));
        match back {
            Some(back) => {
                app.agents[&back].active_modal = Some(ActiveModal::Traces { state });
                super::ctx::switch_to_agent(app, back, super::ctx::SwitchCause::Picker);
            }
            None => super::ctx::show_welcome(app),
        }
        super::session::modal::remove_agent_and_cleanup(app, id);
        return;
    }
    let Some(agent) = super::ctx::get_active_agent_mut(app) else { return };
    agent.clear_trace_content();
    agent.active_modal = Some(ActiveModal::Traces { state });
}

/// A blank top-level view for a past session; nothing is sent for it, and it has no session until the replay binds one.
fn read_only_view(app: &AppView, id: AgentId) -> AgentView {
    let mut scrollback = ScrollbackState::new();
    scrollback.set_appearance(app.appearance.clone());
    AgentView::from_app(
        app,
        AgentSession {
            id,
            acp_tx: app.acp_tx.clone(),
            session_id: None,
            models: app.models.clone(),
            state: AgentState::Idle,
            tracker: AcpUpdateTracker::new(),
            cwd: app.cwd.clone(),
            is_worktree: false,
            forked_from: None,
            pending_prompts: std::collections::VecDeque::new(),
            next_queue_id: 0,
            yolo_mode: false,
            auto_mode: false,
            prompt_history: Vec::new(),
            prompt_history_loading: false,
            loading_replay: false,
            restore_degree: None,
            rate_limited: false,
            model_incompatible: false,
            credit_limit_blocked: false,
            free_usage_blocked: false,
            available_commands: Vec::new(),
            available_commands_generation: 0,
            available_tools: None,
            model_switch_pending: false,
            hook_block_hold: false,
            blocked_prompt: None,
            user_model_preference: None,
            deferred_model_switch: None,
            bg_tasks: std::collections::BTreeMap::new(),
            bg_tool_call_to_task: std::collections::HashMap::new(),
            scheduled_tasks: std::collections::HashMap::new(),
            in_flight_prompt: None,
            forwarded_slash: None,
            turn_saw_agent_text: false,
            compact_held_prompt: None,
            current_prompt_id: None,
            created_via_new: false,
        },
        scrollback,
    )
}

/// One converted event as the message the handler takes off the wire.
fn message(session_id: &str, fed: Fed) -> AcpClientMessage {
    let (response_tx, _rx) = tokio::sync::oneshot::channel();
    match fed.event {
        Event::Acp(update) => AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
            request: acp::SessionNotification::new(acp::SessionId::new(session_id), update).meta(Some(fed.meta)),
            response_tx,
        }),
        Event::Xai(update) => {
            let payload = SessionNotification {
                session_id: acp::SessionId::new(session_id),
                update,
                meta: Some(serde_json::Value::Object(fed.meta)),
            };
            let params = serde_json::value::to_raw_value(&payload).expect("session notification serializes");
            AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                request: acp::ExtNotification::new("x.ai/session/update", params.into()),
                response_tx,
            })
        }
    }
}

/// Whether the active view is a read-only past session, or a subagent opened from one (the parent holds `trace_return`).
pub(crate) fn in_trace_view(app: &mut AppView) -> bool {
    let crate::app::app_view::ActiveView::Agent(id) = app.active_view else { return false };
    app.agents.get(&id).is_some_and(|a| a.trace_return.is_some())
}
