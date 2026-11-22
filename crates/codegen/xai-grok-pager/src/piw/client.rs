//! Non-blocking local-socket client for the `pi-workflows` server.
//!
//! A single background worker thread runs a current-thread Tokio runtime that
//! owns the Unix-domain socket (or Windows named pipe), performs the protocol
//! handshake, maintains active subscriptions across reconnects, streams
//! chunked `$artifact` content, and routes request/response envelopes. Every
//! method on [`PiwClient`] except [`PiwClient::call`] reads or mutates
//! in-memory state immediately and never performs I/O on the calling thread.

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

const PROTOCOL_ID: &str = "pi-workflows.client.v1";
const STEP_ACTIVITY_SCHEMA: &str = "pi-workflows.step-activity.v1";
const PIW_SOCKET_ENV: &str = "PIW_SOCKET";
const MAX_PROTOCOL_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_OPAQUE_ID_BYTES: usize = 256;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const RECONNECT_BASE_MS: u64 = 250;
const RECONNECT_MAX_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Text,
    Thought,
    Tool,
    Status,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolActivity {
    pub tool_call_id: String,
    pub title: String,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepActivity {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub seq: u64,
    pub at: String,
    pub kind: ActivityKind,
    pub text: String,
    pub tool: Option<ToolActivity>,
}

impl StepActivity {
    /// None for anything that is not a well-formed StepActivity.
    pub fn from_json(value: &Value) -> Option<StepActivity> {
        let obj = value.as_object()?;
        if obj.get("schema").and_then(Value::as_str)? != STEP_ACTIVITY_SCHEMA {
            return None;
        }
        let run_id = obj.get("runId").and_then(Value::as_str)?.to_string();
        let node_id = obj.get("nodeId").and_then(Value::as_str)?.to_string();
        let attempt_id = obj.get("attemptId").and_then(Value::as_str)?.to_string();
        if run_id.is_empty() || node_id.is_empty() || attempt_id.is_empty() {
            return None;
        }
        let seq = obj.get("seq").and_then(Value::as_u64)?;
        let at = obj.get("at").and_then(Value::as_str)?.to_string();
        if at.is_empty() {
            return None;
        }
        let kind = match obj.get("kind").and_then(Value::as_str)? {
            "text" => ActivityKind::Text,
            "thought" => ActivityKind::Thought,
            "tool" => ActivityKind::Tool,
            "status" => ActivityKind::Status,
            _ => return None,
        };
        let text = obj.get("text").and_then(Value::as_str)?.to_string();
        let tool = match kind {
            ActivityKind::Tool => {
                let tool_obj = obj.get("tool")?.as_object()?;
                Some(parse_tool_activity(tool_obj)?)
            }
            ActivityKind::Text | ActivityKind::Thought | ActivityKind::Status => {
                if obj.get("tool").is_some_and(|v| !v.is_null()) {
                    return None;
                }
                None
            }
        };
        Some(StepActivity {
            run_id,
            node_id,
            attempt_id,
            seq,
            at,
            kind,
            text,
            tool,
        })
    }
}

fn parse_tool_activity(obj: &Map<String, Value>) -> Option<ToolActivity> {
    let tool_call_id = obj.get("toolCallId").and_then(Value::as_str)?.to_string();
    if tool_call_id.is_empty() {
        return None;
    }
    let title = obj.get("title").and_then(Value::as_str)?.to_string();
    let status = match obj.get("status") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return None,
    };
    let kind = match obj.get("kind") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return None,
    };
    let input = obj.get("input").filter(|v| !v.is_null()).cloned();
    let output = obj.get("output").filter(|v| !v.is_null()).cloned();
    let truncated = match obj.get("truncated") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return None,
    };
    Some(ToolActivity {
        tool_call_id,
        title,
        status,
        kind,
        input,
        output,
        truncated,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiwResponse {
    pub request_id: String,
    pub operation: String,
    pub run_id: Option<String>,
    pub outcome: String,
    pub revision: Option<u64>,
    pub receipt: Option<Value>,
    pub error: Option<Value>,
}

impl PiwResponse {
    /// True when the server accepted the request (see the outcome words in src/client/protocol.ts).
    pub fn ok(&self) -> bool {
        matches!(self.outcome.as_str(), "accepted" | "adopted")
    }

    /// A one-line human message for a refusal, taken from `error`; None when ok().
    pub fn error_text(&self) -> Option<String> {
        if self.ok() {
            return None;
        }
        let extracted = match self.error.as_ref() {
            Some(Value::String(s)) => Some(collapse_one_line(s)),
            Some(Value::Object(map)) => map
                .get("message")
                .and_then(Value::as_str)
                .map(collapse_one_line)
                .filter(|s| !s.is_empty())
                .or_else(|| Some(collapse_one_line(&Value::Object(map.clone()).to_string()))),
            Some(Value::Null) | None => None,
            Some(other) => Some(collapse_one_line(&other.to_string())),
        };
        match extracted {
            Some(text) if !text.is_empty() => Some(text),
            _ => Some(format!("workflow request was {}", self.outcome)),
        }
    }
}

fn collapse_one_line(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_space = false;
    for ch in input.trim().chars() {
        if matches!(ch, '\r' | '\n' | '\t') {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = ch == ' ';
        }
    }
    out.trim().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Connecting,
    Connected,
    Reconnecting,
    Closed,
}

/// `PiwResponse::outcome` of a request the client gave up on locally because the connection dropped.
pub const CONNECTION_LOST: &str = "connection_lost";

#[derive(Debug, Clone, PartialEq)]
pub struct RunView {
    pub revision: u64,
    pub run_revision: u64,
    pub view: Value,
}

/// A value the server replaced by a `$artifact` content reference, fetched in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Pending,
    Ready(String),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ContentKey {
    run_id: String,
    path: String,
    media_type: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone)]
enum ContentEntry {
    Loading {
        buffer: Vec<u8>,
        next_offset: u64,
        in_flight: bool,
    },
    Ready(String),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum WatchKey {
    Runs,
    Run(String),
    Activity(String),
    Session(String),
}

impl WatchKey {
    fn describe(&self) -> String {
        match self {
            WatchKey::Runs => "runs watch".to_string(),
            WatchKey::Run(run_id) => format!("run watch ({run_id})"),
            WatchKey::Activity(run_id) => format!("activity watch ({run_id})"),
            WatchKey::Session(session_id) => format!("session watch ({session_id})"),
        }
    }
}

#[derive(Debug, Clone)]
struct QueuedRequest {
    request_id: String,
    operation: String,
    run_id: Option<String>,
    frame: String,
}

#[derive(Debug, Clone)]
struct InFlightRequest {
    request_id: String,
    operation: String,
    run_id: Option<String>,
}

#[derive(Debug, Default)]
struct ActivityStream {
    last_seq: u64,
    items: Vec<StepActivity>,
}

#[derive(Debug)]
struct Shared {
    client_id: String,
    request_counter: u64,
    conn_state: ConnState,
    closed: bool,
    last_error: Option<String>,
    last_error_sticky: bool,
    server_version: Option<String>,
    connection_count: u64,
    generation: u64,

    runs_watched: bool,
    runs: Vec<Value>,

    watched_runs: BTreeSet<String>,
    run_views: HashMap<String, RunView>,

    watched_activities: BTreeSet<String>,
    activities: HashMap<String, ActivityStream>,

    watched_sessions: BTreeMap<String, bool>,
    sessions: HashMap<String, Value>,

    disabled_watches: HashSet<WatchKey>,

    contents: HashMap<ContentKey, ContentEntry>,

    queued_requests: VecDeque<QueuedRequest>,
    responses: HashMap<String, PiwResponse>,
    discarded_requests: HashSet<String>,
}

impl Shared {
    fn new(client_id: String) -> Self {
        Self {
            client_id,
            request_counter: 0,
            conn_state: ConnState::Connecting,
            closed: false,
            last_error: None,
            last_error_sticky: false,
            server_version: None,
            connection_count: 0,
            generation: 0,
            runs_watched: false,
            runs: Vec::new(),
            watched_runs: BTreeSet::new(),
            run_views: HashMap::new(),
            watched_activities: BTreeSet::new(),
            activities: HashMap::new(),
            watched_sessions: BTreeMap::new(),
            sessions: HashMap::new(),
            disabled_watches: HashSet::new(),
            contents: HashMap::new(),
            queued_requests: VecDeque::new(),
            responses: HashMap::new(),
            discarded_requests: HashSet::new(),
        }
    }

    fn next_request_id(&mut self) -> String {
        self.request_counter = self.request_counter.wrapping_add(1);
        format!("{}-req-{}", self.client_id, self.request_counter)
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn set_transient_error(&mut self, message: String) {
        if !self.last_error_sticky {
            self.last_error = Some(message);
        }
    }

    fn set_sticky_error(&mut self, message: String) {
        self.last_error = Some(message);
        self.last_error_sticky = true;
    }

    fn clear_transient_error(&mut self) {
        if !self.last_error_sticky {
            self.last_error = None;
        }
    }
}

struct Inner {
    shared: Mutex<Shared>,
    condvar: Condvar,
    wake_tx: mpsc::UnboundedSender<()>,
    shutdown_tx: watch::Sender<bool>,
}

/// Send + Sync. Share it with `Arc<PiwClient>`. Every method except `call` returns immediately and
/// never does I/O on the calling thread: a worker thread owns the socket, reconnects, and re-sends
/// the watches after a reconnect.
pub struct PiwClient {
    inner: Arc<Inner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl PiwClient {
    /// The server socket of this machine's default state folder, by the same rule as
    /// `src/client/protocol.ts`.
    pub fn default_socket_path() -> Option<PathBuf> {
        let pinned = std::env::var_os(PIW_SOCKET_ENV);
        let home = user_home_dir();
        default_socket_path_from(pinned.as_deref(), home.as_deref())
    }

    /// Returns at once; the worker connects in the background. Err only if the worker thread cannot
    /// be started.
    pub fn connect(socket: &Path) -> Result<PiwClient> {
        let client_id = format!("grok-piw-{}", Uuid::new_v4());
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let inner = Arc::new(Inner {
            shared: Mutex::new(Shared::new(client_id)),
            condvar: Condvar::new(),
            wake_tx,
            shutdown_tx,
        });
        let worker_inner = Arc::clone(&inner);
        let socket_path = socket.to_path_buf();
        let handle = std::thread::Builder::new()
            .name("piw-client".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(error) => {
                        let mut shared = lock_unpoisoned(&worker_inner.shared);
                        shared.set_sticky_error(format!(
                            "failed to start pi-workflows client runtime: {error}"
                        ));
                        shared.conn_state = ConnState::Closed;
                        shared.closed = true;
                        worker_inner.condvar.notify_all();
                        return;
                    }
                };
                runtime.block_on(run_worker(socket_path, worker_inner, wake_rx, shutdown_rx));
            })
            .context("spawn pi-workflows client worker thread")?;

        let _ = inner.wake_tx.send(());
        Ok(PiwClient {
            inner,
            worker: Mutex::new(Some(handle)),
        })
    }

    pub fn state(&self) -> ConnState {
        lock_unpoisoned(&self.inner.shared).conn_state
    }

    pub fn last_error(&self) -> Option<String> {
        lock_unpoisoned(&self.inner.shared).last_error.clone()
    }

    /// The `clientId` this client puts on every request.
    pub fn client_id(&self) -> String {
        lock_unpoisoned(&self.inner.shared).client_id.clone()
    }

    /// The server's `packageVersion` from its hello, once connected.
    pub fn server_version(&self) -> Option<String> {
        lock_unpoisoned(&self.inner.shared).server_version.clone()
    }

    /// How many times the connection has been established (1 after the first connect). The
    /// coordinator role and its epoch belong to one connection, so users of session commands
    /// compare this to notice a reconnect.
    pub fn connection_count(&self) -> u64 {
        lock_unpoisoned(&self.inner.shared).connection_count
    }

    /// Bumps every time any cached list, view, session snapshot or activity changes. Lets pollers
    /// skip work.
    pub fn generation(&self) -> u64 {
        lock_unpoisoned(&self.inner.shared).generation
    }

    pub fn watch_runs(&self) {
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if shared.closed {
            return;
        }
        if !shared.runs_watched {
            shared.runs_watched = true;
            drop(shared);
            self.wake();
        }
    }

    /// Latest run summaries (WorkflowRunSummary JSON values), in the server's order. Empty until
    /// the first event.
    pub fn runs(&self) -> Vec<Value> {
        lock_unpoisoned(&self.inner.shared).runs.clone()
    }

    pub fn watch_run(&self, run_id: &str) {
        if !valid_opaque_id(run_id) {
            return;
        }
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if shared.closed {
            return;
        }
        if shared.watched_runs.insert(run_id.to_string()) {
            drop(shared);
            self.wake();
        }
    }

    pub fn unwatch_run(&self, run_id: &str) {
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if shared.closed {
            return;
        }
        let removed_watch = shared.watched_runs.remove(run_id);
        let removed_activity_watch = shared.watched_activities.remove(run_id);
        let removed_view = shared.run_views.remove(run_id).is_some();
        let removed_activity = shared.activities.remove(run_id).is_some();
        shared
            .disabled_watches
            .remove(&WatchKey::Run(run_id.to_string()));
        shared
            .disabled_watches
            .remove(&WatchKey::Activity(run_id.to_string()));
        if removed_view || removed_activity {
            shared.bump_generation();
        }
        drop(shared);
        if removed_watch || removed_activity_watch {
            self.wake();
        }
    }

    pub fn run_view(&self, run_id: &str) -> Option<RunView> {
        lock_unpoisoned(&self.inner.shared)
            .run_views
            .get(run_id)
            .cloned()
    }

    /// Starts (once) the activity watch of the run from seq 0 and keeps everything received.
    pub fn watch_activity(&self, run_id: &str) {
        if !valid_opaque_id(run_id) {
            return;
        }
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if shared.closed {
            return;
        }
        if shared.watched_activities.insert(run_id.to_string()) {
            drop(shared);
            self.wake();
        }
    }

    /// Received activity of the run with seq > after_seq, in seq order.
    pub fn activity(&self, run_id: &str, after_seq: u64) -> Vec<StepActivity> {
        let shared = lock_unpoisoned(&self.inner.shared);
        let Some(stream) = shared.activities.get(run_id) else {
            return Vec::new();
        };
        stream
            .items
            .iter()
            .filter(|item| item.seq > after_seq)
            .cloned()
            .collect()
    }

    /// Watch a session's snapshot; with `coordinator` true the watch also claims the coordinator
    /// role.
    pub fn watch_session(&self, session_id: &str, coordinator: bool) {
        if !valid_opaque_id(session_id) {
            return;
        }
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if shared.closed {
            return;
        }
        let previous = shared
            .watched_sessions
            .insert(session_id.to_string(), coordinator);
        if previous != Some(coordinator) {
            drop(shared);
            self.wake();
        }
    }

    /// Latest session snapshot payload (`interaction`, `coordinatorEpoch`, `coordinatorActive`,
    /// `branchReportRequired`, ...), verbatim. After a reconnect it is None until the new snapshot
    /// arrives.
    pub fn session(&self, session_id: &str) -> Option<Value> {
        lock_unpoisoned(&self.inner.shared)
            .sessions
            .get(session_id)
            .cloned()
    }

    /// End the session watch (and with it this connection's hold on the coordinator role, as the
    /// server defines).
    pub fn unwatch_session(&self, session_id: &str) {
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if shared.closed {
            return;
        }
        let removed_watch = shared.watched_sessions.remove(session_id).is_some();
        let removed_snapshot = shared.sessions.remove(session_id).is_some();
        shared
            .disabled_watches
            .remove(&WatchKey::Session(session_id.to_string()));
        if removed_snapshot {
            shared.bump_generation();
        }
        drop(shared);
        if removed_watch {
            self.wake();
        }
    }

    /// The text behind a `$artifact` content reference found in a view of `run_id`: starts the
    /// fetch (`view.content`, as `piw` does) on first call and returns Pending until it is
    /// complete. Cached per reference. A value that is not a reference gives Invalid.
    pub fn content(&self, run_id: &str, reference: &Value) -> Content {
        if !valid_opaque_id(run_id) {
            return Content::Invalid("run_id is invalid".to_string());
        }
        let Some(key) = parse_artifact_reference(run_id, reference) else {
            return Content::Invalid("value is not a valid $artifact reference".to_string());
        };
        let mut shared = lock_unpoisoned(&self.inner.shared);
        match shared.contents.get(&key) {
            Some(ContentEntry::Ready(text)) => Content::Ready(text.clone()),
            Some(ContentEntry::Invalid(reason)) => Content::Invalid(reason.clone()),
            Some(ContentEntry::Loading { .. }) => Content::Pending,
            None => {
                if shared.closed {
                    return Content::Invalid("client is closed".to_string());
                }
                shared.contents.insert(
                    key,
                    ContentEntry::Loading {
                        buffer: Vec::new(),
                        next_offset: 0,
                        in_flight: false,
                    },
                );
                drop(shared);
                self.wake();
                Content::Pending
            }
        }
    }

    /// Queue a request with a fresh idempotency key; returns its requestId. It is sent when
    /// connected (and kept until then).
    pub fn send(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
    ) -> String {
        let idempotency_key = format!("idem-{}", Uuid::new_v4());
        self.send_keyed(operation, run_id, expected_revision, payload, &idempotency_key)
    }

    /// The same with the caller's idempotency key, so a caller can repeat a request the server may
    /// already have applied (the server deduplicates on the key).
    pub fn send_keyed(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
        idempotency_key: &str,
    ) -> String {
        let mut shared = lock_unpoisoned(&self.inner.shared);
        let request_id = shared.next_request_id();
        if shared.closed {
            return request_id;
        }
        let client_id = shared.client_id.clone();
        match build_request_frame(
            &request_id,
            &client_id,
            operation,
            idempotency_key,
            run_id,
            expected_revision,
            &payload,
        ) {
            Ok(frame) => {
                shared.queued_requests.push_back(QueuedRequest {
                    request_id: request_id.clone(),
                    operation: operation.to_string(),
                    run_id: run_id.map(str::to_string),
                    frame,
                });
                drop(shared);
                self.wake();
            }
            Err(error) => {
                shared.responses.insert(
                    request_id.clone(),
                    PiwResponse {
                        request_id: request_id.clone(),
                        operation: operation.to_string(),
                        run_id: run_id.map(str::to_string),
                        outcome: "rejected".to_string(),
                        revision: None,
                        receipt: None,
                        error: Some(Value::String(error)),
                    },
                );
                drop(shared);
                self.inner.condvar.notify_all();
            }
        }
        request_id
    }

    /// The response to a request, once; None while it has not arrived. A request that was on the
    /// wire when the connection dropped is NOT re-sent: it completes with
    /// `outcome == CONNECTION_LOST` (ok() false) and the caller decides whether to repeat it with
    /// `send_keyed`.
    pub fn take_response(&self, request_id: &str) -> Option<PiwResponse> {
        lock_unpoisoned(&self.inner.shared)
            .responses
            .remove(request_id)
    }

    /// `send` and wait. Blocks the calling thread up to `timeout`; never call it on the UI thread.
    pub fn call(
        &self,
        operation: &str,
        run_id: Option<&str>,
        expected_revision: Option<u64>,
        payload: Value,
        timeout: Duration,
    ) -> Result<PiwResponse> {
        {
            let shared = lock_unpoisoned(&self.inner.shared);
            if shared.closed {
                anyhow::bail!("pi-workflows client is closed while calling {operation}");
            }
        }
        let request_id = self.send(operation, run_id, expected_revision, payload);
        let deadline = Instant::now() + timeout;
        let mut shared = lock_unpoisoned(&self.inner.shared);
        if let Some(response) = shared.responses.remove(&request_id) {
            return Ok(response);
        }
        loop {
            if shared.closed {
                anyhow::bail!("pi-workflows client is closed while calling {operation}");
            }
            if let Some(response) = shared.responses.remove(&request_id) {
                return Ok(response);
            }
            let now = Instant::now();
            if now >= deadline {
                shared
                    .queued_requests
                    .retain(|req| req.request_id != request_id);
                shared.discarded_requests.insert(request_id);
                anyhow::bail!(
                    "pi-workflows request '{operation}' timed out after {}ms",
                    timeout.as_millis()
                );
            }
            let remaining = deadline.saturating_duration_since(now);
            let (guard, _) = match self.inner.condvar.wait_timeout(shared, remaining) {
                Ok(pair) => pair,
                Err(poisoned) => poisoned.into_inner(),
            };
            shared = guard;
        }
    }

    /// Stop the worker and close the socket. Later calls are no-ops; `state()` is Closed.
    pub fn close(&self) {
        {
            let mut shared = lock_unpoisoned(&self.inner.shared);
            if shared.closed {
                return;
            }
            shared.closed = true;
            shared.conn_state = ConnState::Closed;
            shared.queued_requests.clear();
            for entry in shared.contents.values_mut() {
                if matches!(entry, ContentEntry::Loading { .. }) {
                    *entry = ContentEntry::Invalid("client closed".to_string());
                }
            }
        }
        let _ = self.inner.shutdown_tx.send(true);
        self.inner.condvar.notify_all();

        let handle = lock_unpoisoned(&self.worker).take();
        if let Some(handle) = handle
            && handle.thread().id() != std::thread::current().id()
        {
            let _ = handle.join();
        }
    }

    fn wake(&self) {
        let _ = self.inner.wake_tx.send(());
    }
}

impl Drop for PiwClient {
    fn drop(&mut self) {
        self.close();
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn user_home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
}

fn default_socket_path_from(piw_socket: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(pinned) = piw_socket.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(pinned));
    }
    let state_directory = home?.join(".pi").join("agent").join("workflows");
    #[cfg(windows)]
    {
        let digest = sha256_hex(state_directory.to_string_lossy().as_bytes());
        let suffix: String = digest.chars().take(24).collect();
        Some(PathBuf::from(format!(r"\\.\pipe\pi-workflows-{suffix}")))
    }
    #[cfg(not(windows))]
    {
        Some(state_directory.join("server").join("server.sock"))
    }
}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_OPAQUE_ID_BYTES
}

fn parse_artifact_reference(run_id: &str, value: &Value) -> Option<ContentKey> {
    let outer = value.as_object()?;
    if outer.len() != 1 {
        return None;
    }
    let artifact = outer.get("$artifact")?.as_object()?;
    let path = artifact.get("path").and_then(Value::as_str)?;
    let media_type = artifact.get("mediaType").and_then(Value::as_str)?;
    let bytes = artifact.get("bytes").and_then(Value::as_u64)?;
    let sha256 = artifact.get("sha256").and_then(Value::as_str)?;
    if path.is_empty() || media_type.is_empty() || sha256.is_empty() {
        return None;
    }
    if let Some(opaque) = artifact.get("opaque")
        && !opaque.is_boolean()
    {
        return None;
    }
    Some(ContentKey {
        run_id: run_id.to_string(),
        path: path.to_string(),
        media_type: media_type.to_string(),
        bytes,
        sha256: sha256.to_string(),
    })
}

fn build_request_frame(
    request_id: &str,
    client_id: &str,
    operation: &str,
    idempotency_key: &str,
    run_id: Option<&str>,
    expected_revision: Option<u64>,
    payload: &Value,
) -> std::result::Result<String, String> {
    if !valid_opaque_id(request_id) {
        return Err("requestId must be nonempty text of at most 256 bytes".to_string());
    }
    if !valid_opaque_id(client_id) {
        return Err("clientId must be nonempty text of at most 256 bytes".to_string());
    }
    if !valid_opaque_id(idempotency_key) {
        return Err("idempotencyKey must be nonempty text of at most 256 bytes".to_string());
    }
    if operation.is_empty() {
        return Err("operation must be nonempty text".to_string());
    }
    if let Some(run_id) = run_id
        && !valid_opaque_id(run_id)
    {
        return Err("runId must be nonempty text of at most 256 bytes".to_string());
    }

    let mut obj = Map::new();
    obj.insert("schema".to_string(), Value::String(PROTOCOL_ID.to_string()));
    obj.insert("type".to_string(), Value::String("request".to_string()));
    obj.insert(
        "requestId".to_string(),
        Value::String(request_id.to_string()),
    );
    obj.insert("clientId".to_string(), Value::String(client_id.to_string()));
    obj.insert(
        "operation".to_string(),
        Value::String(operation.to_string()),
    );
    obj.insert(
        "idempotencyKey".to_string(),
        Value::String(idempotency_key.to_string()),
    );
    if let Some(run_id) = run_id {
        obj.insert("runId".to_string(), Value::String(run_id.to_string()));
    }
    if let Some(rev) = expected_revision {
        obj.insert("expectedRevision".to_string(), Value::from(rev));
    }
    obj.insert("payload".to_string(), payload.clone());

    let encoded = canonical_json(&Value::Object(obj))?;
    if encoded.len() + 1 > MAX_PROTOCOL_MESSAGE_BYTES {
        return Err("client protocol message exceeds 1 MiB".to_string());
    }
    Ok(encoded)
}

/// Encode a JSON value as canonical JSON (UTF-16 key order, compact, integer-only numbers).
/// Any non-integer number is refused locally.
fn canonical_json(value: &Value) -> std::result::Result<String, String> {
    let mut output = String::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn write_canonical_json(value: &Value, output: &mut String) -> std::result::Result<(), String> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(b) => output.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(v) = n.as_i64() {
                output.push_str(&v.to_string());
            } else if let Some(v) = n.as_u64() {
                output.push_str(&v.to_string());
            } else {
                return Err("payload contains a non-integer number".to_string());
            }
        }
        Value::String(s) => {
            let escaped =
                serde_json::to_string(s).map_err(|err| format!("invalid JSON string: {err}"))?;
            output.push_str(&escaped);
        }
        Value::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(item, output)?;
            }
            output.push(']');
        }
        Value::Object(map) => {
            output.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                let escaped = serde_json::to_string(key)
                    .map_err(|err| format!("invalid JSON key: {err}"))?;
                output.push_str(&escaped);
                output.push(':');
                if let Some(member) = map.get(key) {
                    write_canonical_json(member, output)?;
                }
            }
            output.push('}');
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LastWritten {
    Watch(WatchKey),
    Other,
}

#[derive(Debug, Clone)]
struct PendingWatch {
    request_id: String,
    subscription_id: String,
    key: WatchKey,
}

struct ConnTracking {
    subscribed_runs: bool,
    subscribed_run_ids: HashSet<String>,
    subscribed_activity_ids: HashSet<String>,
    subscribed_sessions: HashMap<String, bool>,
    pending_watch: Option<PendingWatch>,
    in_flight_requests: HashMap<String, InFlightRequest>,
    in_flight_content: HashMap<String, ContentKey>,
    in_flight_unwatches: HashSet<String>,
    last_written: Option<(String, LastWritten)>,
    saw_post_hello_message: bool,
}

impl ConnTracking {
    fn new() -> Self {
        Self {
            subscribed_runs: false,
            subscribed_run_ids: HashSet::new(),
            subscribed_activity_ids: HashSet::new(),
            subscribed_sessions: HashMap::new(),
            pending_watch: None,
            in_flight_requests: HashMap::new(),
            in_flight_content: HashMap::new(),
            in_flight_unwatches: HashSet::new(),
            last_written: None,
            saw_post_hello_message: false,
        }
    }
}

enum StreamOutcome {
    Shutdown,
    Disconnected {
        error: String,
        after_hello: bool,
        saw_post_hello_message: bool,
        last_written: Option<LastWritten>,
        in_flight_requests: Vec<InFlightRequest>,
        confirmed_watches: Vec<WatchKey>,
    },
}

enum LineRead {
    Line(Vec<u8>),
    Oversized,
    Eof,
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<LineRead> {
    let mut buf = Vec::new();
    let mut discarding = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if discarding {
                return Ok(LineRead::Oversized);
            }
            if buf.is_empty() {
                return Ok(LineRead::Eof);
            }
            return Ok(LineRead::Line(buf));
        }
        if let Some(newline_idx) = available.iter().position(|&b| b == b'\n') {
            if discarding {
                reader.consume(newline_idx + 1);
                return Ok(LineRead::Oversized);
            }
            if buf.len().saturating_add(newline_idx).saturating_add(1) > MAX_PROTOCOL_MESSAGE_BYTES
            {
                reader.consume(newline_idx + 1);
                return Ok(LineRead::Oversized);
            }
            buf.extend_from_slice(&available[..newline_idx]);
            reader.consume(newline_idx + 1);
            if buf.is_empty() {
                continue;
            }
            return Ok(LineRead::Line(buf));
        }
        let n = available.len();
        if !discarding {
            if buf.len().saturating_add(n) > MAX_PROTOCOL_MESSAGE_BYTES {
                discarding = true;
                buf.clear();
            } else {
                buf.extend_from_slice(available);
            }
        }
        reader.consume(n);
    }
}

async fn run_worker(
    socket_path: PathBuf,
    inner: Arc<Inner>,
    mut wake_rx: mpsc::UnboundedReceiver<()>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut attempt: u32 = 0;
    let mut last_drop_watch: Option<(WatchKey, u32)> = None;

    loop {
        if *shutdown_rx.borrow() {
            return;
        }
        let outcome = connect_and_run(&socket_path, &inner, &mut wake_rx, &mut shutdown_rx).await;
        match outcome {
            StreamOutcome::Shutdown => return,
            StreamOutcome::Disconnected {
                error,
                after_hello,
                saw_post_hello_message,
                last_written,
                in_flight_requests,
                confirmed_watches,
            } => {
                if *shutdown_rx.borrow() {
                    return;
                }
                if saw_post_hello_message {
                    attempt = 0;
                }
                if let Some((ref tracked_key, _)) = last_drop_watch
                    && confirmed_watches.contains(tracked_key)
                {
                    last_drop_watch = None;
                }

                let mut disabled_now: Option<WatchKey> = None;
                if after_hello {
                    match last_written {
                        Some(LastWritten::Watch(key)) => {
                            let next_count = match &last_drop_watch {
                                Some((prev, count)) if prev == &key => count.saturating_add(1),
                                _ => 1,
                            };
                            if next_count >= 2 {
                                disabled_now = Some(key);
                                last_drop_watch = None;
                            } else {
                                last_drop_watch = Some((key, next_count));
                            }
                        }
                        _ => {
                            last_drop_watch = None;
                        }
                    }
                }

                {
                    let mut shared = lock_unpoisoned(&inner.shared);
                    if shared.closed {
                        return;
                    }
                    shared.conn_state = ConnState::Reconnecting;
                    if let Some(key) = disabled_now {
                        let desc = key.describe();
                        shared.disabled_watches.insert(key);
                        shared.set_sticky_error(format!(
                            "disabled {desc} after repeated connection drops"
                        ));
                    } else {
                        shared.set_transient_error(error.clone());
                    }

                    if !shared.sessions.is_empty() {
                        shared.sessions.clear();
                        shared.bump_generation();
                    }

                    for entry in shared.contents.values_mut() {
                        if let ContentEntry::Loading { in_flight, .. } = entry {
                            *in_flight = false;
                        }
                    }

                    for req in in_flight_requests {
                        if shared.discarded_requests.remove(&req.request_id) {
                            continue;
                        }
                        shared.responses.insert(
                            req.request_id.clone(),
                            PiwResponse {
                                request_id: req.request_id,
                                operation: req.operation,
                                run_id: req.run_id,
                                outcome: CONNECTION_LOST.to_string(),
                                revision: None,
                                receipt: None,
                                error: Some(Value::String(error.clone())),
                            },
                        );
                    }
                }
                inner.condvar.notify_all();

                let delay_ms = (RECONNECT_BASE_MS.saturating_mul(1u64 << attempt.min(5)))
                    .min(RECONNECT_MAX_MS);
                attempt = attempt.saturating_add(1);

                tokio::select! {
                    biased;
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                }
            }
        }
    }
}

async fn connect_and_run(
    socket_path: &Path,
    inner: &Arc<Inner>,
    wake_rx: &mut mpsc::UnboundedReceiver<()>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> StreamOutcome {
    #[cfg(unix)]
    {
        let connect_fut = UnixStream::connect(socket_path);
        let stream = tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return StreamOutcome::Shutdown;
                }
                return StreamOutcome::Disconnected {
                    error: "shutdown signalled".to_string(),
                    after_hello: false,
                    saw_post_hello_message: false,
                    last_written: None,
                    in_flight_requests: Vec::new(),
                    confirmed_watches: Vec::new(),
                };
            }
            res = connect_fut => match res {
                Ok(stream) => stream,
                Err(err) => {
                    return StreamOutcome::Disconnected {
                        error: format!("connecting to {}: {err}", socket_path.display()),
                        after_hello: false,
                        saw_post_hello_message: false,
                        last_written: None,
                        in_flight_requests: Vec::new(),
                        confirmed_watches: Vec::new(),
                    };
                }
            }
        };
        run_stream(stream, inner, wake_rx, shutdown_rx).await
    }
    #[cfg(windows)]
    {
        match ClientOptions::new().open(socket_path) {
            Ok(stream) => run_stream(stream, inner, wake_rx, shutdown_rx).await,
            Err(err) => StreamOutcome::Disconnected {
                error: format!("connecting to {}: {err}", socket_path.display()),
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            },
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (socket_path, inner, wake_rx, shutdown_rx);
        StreamOutcome::Disconnected {
            error: "local workflow server transport is not supported on this platform".to_string(),
            after_hello: false,
            saw_post_hello_message: false,
            last_written: None,
            in_flight_requests: Vec::new(),
            confirmed_watches: Vec::new(),
        }
    }
}

async fn run_stream<S>(
    stream: S,
    inner: &Arc<Inner>,
    wake_rx: &mut mpsc::UnboundedReceiver<()>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> StreamOutcome
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    // 1. Handshake: wait for server hello.
    let hello_read = tokio::select! {
        biased;
        _ = shutdown_rx.changed() => {
            if *shutdown_rx.borrow() {
                return StreamOutcome::Shutdown;
            }
            return StreamOutcome::Disconnected {
                error: "connection aborted".to_string(),
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            };
        }
        res = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_bounded_line(&mut reader)) => res,
    };

    let hello_bytes = match hello_read {
        Ok(Ok(LineRead::Line(bytes))) => bytes,
        Ok(Ok(LineRead::Oversized)) => {
            return StreamOutcome::Disconnected {
                error: "server hello exceeds 1 MiB".to_string(),
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            };
        }
        Ok(Ok(LineRead::Eof)) => {
            return StreamOutcome::Disconnected {
                error: "server closed connection before hello".to_string(),
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            };
        }
        Ok(Err(err)) => {
            return StreamOutcome::Disconnected {
                error: format!("reading server hello: {err}"),
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            };
        }
        Err(_) => {
            return StreamOutcome::Disconnected {
                error: "server hello timed out".to_string(),
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            };
        }
    };

    let hello_version = match parse_hello(&hello_bytes) {
        Ok(version) => version,
        Err(err) => {
            return StreamOutcome::Disconnected {
                error: err,
                after_hello: false,
                saw_post_hello_message: false,
                last_written: None,
                in_flight_requests: Vec::new(),
                confirmed_watches: Vec::new(),
            };
        }
    };

    {
        let mut shared = lock_unpoisoned(&inner.shared);
        if shared.closed {
            return StreamOutcome::Shutdown;
        }
        shared.conn_state = ConnState::Connected;
        shared.connection_count = shared.connection_count.saturating_add(1);
        shared.server_version = hello_version;
        shared.clear_transient_error();
    }

    let mut tracking = ConnTracking::new();
    let mut confirmed_watches: Vec<WatchKey> = Vec::new();

    // 2. Active connection loop.
    loop {
        if *shutdown_rx.borrow() {
            return StreamOutcome::Shutdown;
        }

        if let Err(err) =
            reconcile_and_write(&mut write_half, inner, &mut tracking, shutdown_rx).await
        {
            if *shutdown_rx.borrow() {
                return StreamOutcome::Shutdown;
            }
            return disconnected_outcome(err, tracking, confirmed_watches);
        }

        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return StreamOutcome::Shutdown;
                }
            }
            read_result = read_bounded_line(&mut reader) => {
                match read_result {
                    Ok(LineRead::Line(bytes)) => {
                        handle_server_frame(&bytes, inner, &mut tracking, &mut confirmed_watches);
                    }
                    Ok(LineRead::Oversized) => {
                        let mut shared = lock_unpoisoned(&inner.shared);
                        shared.set_sticky_error(
                            "skipped server frame exceeding 1 MiB limit".to_string(),
                        );
                    }
                    Ok(LineRead::Eof) => {
                        return disconnected_outcome(
                            "connection closed".to_string(),
                            tracking,
                            confirmed_watches,
                        );
                    }
                    Err(err) => {
                        return disconnected_outcome(
                            format!("socket read error: {err}"),
                            tracking,
                            confirmed_watches,
                        );
                    }
                }
            }
            wake_msg = wake_rx.recv() => {
                if wake_msg.is_none() {
                    return StreamOutcome::Shutdown;
                }
                // Drain coalesced wakes before reconciling.
                while wake_rx.try_recv().is_ok() {}
            }
        }
    }
}

fn disconnected_outcome(
    error: String,
    tracking: ConnTracking,
    confirmed_watches: Vec<WatchKey>,
) -> StreamOutcome {
    let last_written = tracking.last_written.map(|(_, kind)| kind);
    let in_flight_requests = tracking.in_flight_requests.into_values().collect();
    StreamOutcome::Disconnected {
        error,
        after_hello: true,
        saw_post_hello_message: tracking.saw_post_hello_message,
        last_written,
        in_flight_requests,
        confirmed_watches,
    }
}

fn parse_hello(bytes: &[u8]) -> std::result::Result<Option<String>, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|err| format!("invalid server hello JSON: {err}"))?;
    let Some(obj) = value.as_object() else {
        return Err("server hello must be a JSON object".to_string());
    };
    if obj.get("schema").and_then(Value::as_str) != Some(PROTOCOL_ID) {
        return Err("server hello has unexpected protocol schema".to_string());
    }
    if obj.get("type").and_then(Value::as_str) != Some("hello") {
        return Err("server did not send hello first".to_string());
    }
    let package_version = obj
        .get("packageVersion")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(package_version)
}

struct OutboundFrame {
    request_id: String,
    frame: String,
    kind: OutboundKind,
}

enum OutboundKind {
    Watch {
        subscription_id: String,
        key: WatchKey,
    },
    Unwatch,
    UserRequest(InFlightRequest),
    ContentRequest(ContentKey),
}

async fn reconcile_and_write<W>(
    writer: &mut W,
    inner: &Arc<Inner>,
    tracking: &mut ConnTracking,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> std::result::Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    let frames = collect_outbound_frames(inner, tracking);
    for outbound in frames {
        let last_written_kind = match &outbound.kind {
            OutboundKind::Watch {
                subscription_id,
                key,
            } => {
                tracking.pending_watch = Some(PendingWatch {
                    request_id: outbound.request_id.clone(),
                    subscription_id: subscription_id.clone(),
                    key: key.clone(),
                });
                LastWritten::Watch(key.clone())
            }
            OutboundKind::Unwatch => {
                tracking
                    .in_flight_unwatches
                    .insert(outbound.request_id.clone());
                LastWritten::Other
            }
            OutboundKind::UserRequest(req) => {
                tracking
                    .in_flight_requests
                    .insert(req.request_id.clone(), req.clone());
                LastWritten::Other
            }
            OutboundKind::ContentRequest(key) => {
                tracking
                    .in_flight_content
                    .insert(outbound.request_id.clone(), key.clone());
                LastWritten::Other
            }
        };
        tracking.last_written = Some((outbound.request_id.clone(), last_written_kind));

        let mut wire = Vec::with_capacity(outbound.frame.len() + 1);
        wire.extend_from_slice(outbound.frame.as_bytes());
        wire.push(b'\n');

        let write_res = tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return Err("client closed".to_string());
                }
                Ok(())
            }
            res = async {
                writer.write_all(&wire).await?;
                writer.flush().await
            } => res,
        };
        if let Err(err) = write_res {
            return Err(format!("socket write error: {err}"));
        }
    }
    Ok(())
}

fn collect_outbound_frames(inner: &Arc<Inner>, tracking: &mut ConnTracking) -> Vec<OutboundFrame> {
    let mut shared = lock_unpoisoned(&inner.shared);
    if shared.closed {
        return Vec::new();
    }
    let client_id = shared.client_id.clone();
    let mut out = Vec::new();

    // 1. Send unwatches for removed subscriptions.
    let run_removals: Vec<String> = tracking
        .subscribed_run_ids
        .iter()
        .filter(|run_id| !shared.watched_runs.contains(*run_id))
        .cloned()
        .collect();
    for run_id in run_removals {
        tracking.subscribed_run_ids.remove(&run_id);
        if tracking
            .pending_watch
            .as_ref()
            .is_some_and(|pw| pw.key == WatchKey::Run(run_id.clone()))
        {
            tracking.pending_watch = None;
        }
        let request_id = shared.next_request_id();
        let sub_id = format!("run:{run_id}");
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.run.unwatch",
            &request_id,
            Some(&run_id),
            None,
            &json!({ "subscriptionId": sub_id }),
        ) {
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Unwatch,
            });
        }
    }

    let activity_removals: Vec<String> = tracking
        .subscribed_activity_ids
        .iter()
        .filter(|run_id| !shared.watched_activities.contains(*run_id))
        .cloned()
        .collect();
    for run_id in activity_removals {
        tracking.subscribed_activity_ids.remove(&run_id);
        if tracking
            .pending_watch
            .as_ref()
            .is_some_and(|pw| pw.key == WatchKey::Activity(run_id.clone()))
        {
            tracking.pending_watch = None;
        }
        let request_id = shared.next_request_id();
        let sub_id = format!("activity:{run_id}");
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.run.activity.unwatch",
            &request_id,
            None,
            None,
            &json!({ "subscriptionId": sub_id }),
        ) {
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Unwatch,
            });
        }
    }

    let session_removals: Vec<String> = tracking
        .subscribed_sessions
        .keys()
        .filter(|sid| !shared.watched_sessions.contains_key(*sid))
        .cloned()
        .collect();
    for session_id in session_removals {
        tracking.subscribed_sessions.remove(&session_id);
        if tracking
            .pending_watch
            .as_ref()
            .is_some_and(|pw| pw.key == WatchKey::Session(session_id.clone()))
        {
            tracking.pending_watch = None;
        }
        let request_id = shared.next_request_id();
        let sub_id = format!("session:{session_id}");
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.run.unwatch",
            &request_id,
            None,
            None,
            &json!({ "subscriptionId": sub_id }),
        ) {
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Unwatch,
            });
        }
    }

    // 2. If a watch is already awaiting its response, wait before sending the next watch or
    //    queued request so a failing watch is attributed accurately and cannot fail queued requests.
    if tracking.pending_watch.is_some() {
        return out;
    }

    // 3. Send at most one watch at a time, starting with standard watches and then activity watches.
    if shared.runs_watched
        && !tracking.subscribed_runs
        && !shared.disabled_watches.contains(&WatchKey::Runs)
    {
        let request_id = shared.next_request_id();
        let sub_id = "runs".to_string();
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.runs.watch",
            &request_id,
            None,
            None,
            &json!({ "subscriptionId": sub_id }),
        ) {
            tracking.subscribed_runs = true;
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Watch {
                    subscription_id: sub_id,
                    key: WatchKey::Runs,
                },
            });
            return out;
        }
    }

    let next_run = shared
        .watched_runs
        .iter()
        .find(|run_id| {
            !tracking.subscribed_run_ids.contains(*run_id)
                && !shared
                    .disabled_watches
                    .contains(&WatchKey::Run((*run_id).clone()))
        })
        .cloned();
    if let Some(run_id) = next_run {
        let request_id = shared.next_request_id();
        let sub_id = format!("run:{run_id}");
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.run.watch",
            &request_id,
            Some(&run_id),
            None,
            &json!({ "subscriptionId": sub_id }),
        ) {
            tracking.subscribed_run_ids.insert(run_id.clone());
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Watch {
                    subscription_id: sub_id,
                    key: WatchKey::Run(run_id),
                },
            });
            return out;
        }
    }

    let next_session = shared
        .watched_sessions
        .iter()
        .find(|(sid, coord)| {
            tracking.subscribed_sessions.get(*sid) != Some(*coord)
                && !shared
                    .disabled_watches
                    .contains(&WatchKey::Session((*sid).clone()))
        })
        .map(|(sid, coord)| (sid.clone(), *coord));
    if let Some((session_id, coordinator)) = next_session {
        let request_id = shared.next_request_id();
        let sub_id = format!("session:{session_id}");
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.session.watch",
            &request_id,
            None,
            None,
            &json!({
                "subscriptionId": sub_id,
                "sessionId": session_id,
                "coordinator": coordinator,
            }),
        ) {
            tracking
                .subscribed_sessions
                .insert(session_id.clone(), coordinator);
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Watch {
                    subscription_id: sub_id,
                    key: WatchKey::Session(session_id),
                },
            });
            return out;
        }
    }

    let next_activity = shared
        .watched_activities
        .iter()
        .find(|run_id| {
            !tracking.subscribed_activity_ids.contains(*run_id)
                && !shared
                    .disabled_watches
                    .contains(&WatchKey::Activity((*run_id).clone()))
        })
        .cloned();
    if let Some(run_id) = next_activity {
        let after_seq = shared
            .activities
            .get(&run_id)
            .map_or(0, |stream| stream.last_seq);
        let request_id = shared.next_request_id();
        let sub_id = format!("activity:{run_id}");
        if let Ok(frame) = build_request_frame(
            &request_id,
            &client_id,
            "view.run.activity.watch",
            &request_id,
            Some(&run_id),
            None,
            &json!({
                "subscriptionId": sub_id,
                "afterSeq": after_seq,
            }),
        ) {
            tracking.subscribed_activity_ids.insert(run_id.clone());
            out.push(OutboundFrame {
                request_id,
                frame,
                kind: OutboundKind::Watch {
                    subscription_id: sub_id,
                    key: WatchKey::Activity(run_id),
                },
            });
            return out;
        }
    }

    // 4. All active watches are established; send queued user requests.
    while let Some(queued) = shared.queued_requests.pop_front() {
        out.push(OutboundFrame {
            request_id: queued.request_id.clone(),
            frame: queued.frame,
            kind: OutboundKind::UserRequest(InFlightRequest {
                request_id: queued.request_id,
                operation: queued.operation,
                run_id: queued.run_id,
            }),
        });
    }

    // 5. Send pending `$artifact` content chunk fetches.
    let pending_content: Vec<(ContentKey, u64)> = shared
        .contents
        .iter()
        .filter_map(|(key, entry)| match entry {
            ContentEntry::Loading {
                next_offset,
                in_flight: false,
                ..
            } => Some((key.clone(), *next_offset)),
            _ => None,
        })
        .collect();
    for (key, offset) in pending_content {
        let request_id = shared.next_request_id();
        match build_request_frame(
            &request_id,
            &client_id,
            "view.content",
            &request_id,
            Some(&key.run_id),
            None,
            &json!({
                "path": key.path,
                "offset": offset,
            }),
        ) {
            Ok(frame) => {
                if let Some(ContentEntry::Loading { in_flight, .. }) = shared.contents.get_mut(&key)
                {
                    *in_flight = true;
                }
                out.push(OutboundFrame {
                    request_id,
                    frame,
                    kind: OutboundKind::ContentRequest(key),
                });
            }
            Err(err) => {
                shared.contents.insert(key, ContentEntry::Invalid(err));
                shared.bump_generation();
            }
        }
    }

    out
}

fn handle_server_frame(
    bytes: &[u8],
    inner: &Arc<Inner>,
    tracking: &mut ConnTracking,
    confirmed_watches: &mut Vec<WatchKey>,
) {
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(err) => {
            let mut shared = lock_unpoisoned(&inner.shared);
            shared.set_sticky_error(format!("skipped non-JSON server frame: {err}"));
            return;
        }
    };
    let Some(obj) = value.as_object() else {
        let mut shared = lock_unpoisoned(&inner.shared);
        shared.set_sticky_error("skipped server frame: expected JSON object".to_string());
        return;
    };
    let Some(msg_type) = obj.get("type").and_then(Value::as_str) else {
        let mut shared = lock_unpoisoned(&inner.shared);
        shared.set_sticky_error("skipped server frame: missing message type".to_string());
        return;
    };

    match msg_type {
        "response" => {
            tracking.saw_post_hello_message = true;
            handle_response_frame(obj, inner, tracking, confirmed_watches);
        }
        "event" => {
            tracking.saw_post_hello_message = true;
            handle_event_frame(obj, inner, tracking, confirmed_watches);
        }
        other => {
            let mut shared = lock_unpoisoned(&inner.shared);
            shared.set_sticky_error(format!("skipped unexpected server message type: {other}"));
        }
    }
}

fn handle_response_frame(
    obj: &Map<String, Value>,
    inner: &Arc<Inner>,
    tracking: &mut ConnTracking,
    confirmed_watches: &mut Vec<WatchKey>,
) {
    let Some(request_id) = obj.get("requestId").and_then(Value::as_str) else {
        lock_unpoisoned(&inner.shared)
            .set_sticky_error("skipped server response without requestId".to_string());
        return;
    };
    let Some(outcome) = obj.get("outcome").and_then(Value::as_str) else {
        lock_unpoisoned(&inner.shared)
            .set_sticky_error("skipped server response without outcome".to_string());
        return;
    };
    let revision = obj.get("revision").and_then(Value::as_u64);
    let receipt = obj.get("receipt").filter(|v| !v.is_null()).cloned();
    let error = obj.get("error").filter(|v| !v.is_null()).cloned();

    if tracking
        .last_written
        .as_ref()
        .is_some_and(|(id, _)| id == request_id)
    {
        tracking.last_written = None;
    }

    if let Some(pending) = tracking.pending_watch.as_ref()
        && pending.request_id == request_id
    {
        let pending = tracking.pending_watch.take();
        if let Some(pending) = pending {
            confirmed_watches.push(pending.key.clone());
            if !matches!(outcome, "accepted" | "adopted") {
                let err_msg = extract_error_string(error.as_ref())
                    .unwrap_or_else(|| format!("{} was {outcome}", pending.key.describe()));
                lock_unpoisoned(&inner.shared).set_sticky_error(err_msg);
            }
        }
        return;
    }

    if tracking.in_flight_unwatches.remove(request_id) {
        return;
    }

    if let Some(req) = tracking.in_flight_requests.remove(request_id) {
        let mut shared = lock_unpoisoned(&inner.shared);
        if shared.discarded_requests.remove(request_id) {
            return;
        }
        shared.responses.insert(
            req.request_id.clone(),
            PiwResponse {
                request_id: req.request_id,
                operation: req.operation,
                run_id: req.run_id,
                outcome: outcome.to_string(),
                revision,
                receipt,
                error,
            },
        );
        drop(shared);
        inner.condvar.notify_all();
        return;
    }

    if let Some(content_key) = tracking.in_flight_content.remove(request_id) {
        handle_content_response(inner, content_key, outcome, receipt, error);
    }
}

fn handle_content_response(
    inner: &Arc<Inner>,
    key: ContentKey,
    outcome: &str,
    receipt: Option<Value>,
    error: Option<Value>,
) {
    let mut shared = lock_unpoisoned(&inner.shared);
    if !matches!(outcome, "accepted" | "adopted") {
        let msg = extract_error_string(error.as_ref())
            .unwrap_or_else(|| format!("content fetch failed: {outcome}"));
        shared.contents.insert(key, ContentEntry::Invalid(msg));
        shared.bump_generation();
        return;
    }
    let Some(receipt_obj) = receipt.as_ref().and_then(Value::as_object) else {
        shared.contents.insert(
            key,
            ContentEntry::Invalid("content chunk receipt is missing or invalid".to_string()),
        );
        shared.bump_generation();
        return;
    };

    let Some(entry) = shared.contents.remove(&key) else {
        return;
    };
    let ContentEntry::Loading {
        mut buffer,
        next_offset: expected_offset,
        ..
    } = entry
    else {
        shared.contents.insert(key, entry);
        return;
    };

    let chunk_result = parse_and_validate_chunk(receipt_obj, &key, expected_offset);
    match chunk_result {
        Err(reason) => {
            shared.contents.insert(key, ContentEntry::Invalid(reason));
            shared.bump_generation();
        }
        Ok((chunk_bytes, next_offset, complete)) => {
            buffer.extend_from_slice(&chunk_bytes);
            if complete {
                if buffer.len() as u64 != key.bytes || next_offset != key.bytes {
                    shared.contents.insert(
                        key,
                        ContentEntry::Invalid("content length does not match reference".to_string()),
                    );
                } else if sha256_hex(&buffer) != key.sha256 {
                    shared.contents.insert(
                        key,
                        ContentEntry::Invalid("content digest does not match reference".to_string()),
                    );
                } else {
                    match String::from_utf8(buffer) {
                        Ok(text) => {
                            shared.contents.insert(key, ContentEntry::Ready(text));
                        }
                        Err(_) => {
                            shared.contents.insert(
                                key,
                                ContentEntry::Invalid("content is not valid UTF-8".to_string()),
                            );
                        }
                    }
                }
                shared.bump_generation();
            } else {
                shared.contents.insert(
                    key,
                    ContentEntry::Loading {
                        buffer,
                        next_offset,
                        in_flight: false,
                    },
                );
            }
        }
    }
}

fn parse_and_validate_chunk(
    receipt: &Map<String, Value>,
    key: &ContentKey,
    expected_offset: u64,
) -> std::result::Result<(Vec<u8>, u64, bool), String> {
    let path = receipt
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "content chunk missing path".to_string())?;
    if path != key.path {
        return Err("content chunk path does not match reference".to_string());
    }
    let media_type = receipt
        .get("mediaType")
        .and_then(Value::as_str)
        .ok_or_else(|| "content chunk missing mediaType".to_string())?;
    if media_type != key.media_type {
        return Err("content chunk mediaType does not match reference".to_string());
    }
    let bytes = receipt
        .get("bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "content chunk missing bytes".to_string())?;
    if bytes != key.bytes {
        return Err("content chunk byte total does not match reference".to_string());
    }
    let sha256 = receipt
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "content chunk missing sha256".to_string())?;
    if sha256 != key.sha256 {
        return Err("content chunk sha256 does not match reference".to_string());
    }
    let offset = receipt
        .get("offset")
        .and_then(Value::as_u64)
        .ok_or_else(|| "content chunk missing offset".to_string())?;
    let next_offset = receipt
        .get("nextOffset")
        .and_then(Value::as_u64)
        .ok_or_else(|| "content chunk missing nextOffset".to_string())?;
    let complete = receipt
        .get("complete")
        .and_then(Value::as_bool)
        .ok_or_else(|| "content chunk missing complete".to_string())?;
    let data_b64 = receipt
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "content chunk missing data".to_string())?;
    let chunk = BASE64
        .decode(data_b64)
        .map_err(|err| format!("content chunk is not valid base64: {err}"))?;
    if offset != expected_offset
        || offset.checked_add(chunk.len() as u64) != Some(next_offset)
        || next_offset > key.bytes
    {
        return Err("content chunk offset is invalid".to_string());
    }
    if !complete && chunk.is_empty() {
        return Err("content chunk made no progress".to_string());
    }
    Ok((chunk, next_offset, complete))
}

fn handle_event_frame(
    obj: &Map<String, Value>,
    inner: &Arc<Inner>,
    tracking: &mut ConnTracking,
    confirmed_watches: &mut Vec<WatchKey>,
) {
    let Some(event_name) = obj.get("event").and_then(Value::as_str) else {
        lock_unpoisoned(&inner.shared)
            .set_sticky_error("skipped server event without event name".to_string());
        return;
    };
    let subscription_id = obj.get("subscriptionId").and_then(Value::as_str);
    let envelope_run_id = obj.get("runId").and_then(Value::as_str);
    let payload = obj.get("payload").cloned().unwrap_or(Value::Null);

    // Receiving an event for a pending watch also confirms that the watch did not kill the socket.
    if let (Some(sub_id), Some(pending)) = (subscription_id, tracking.pending_watch.as_ref())
        && pending.subscription_id == sub_id
    {
        if let Some(pending) = tracking.pending_watch.take() {
            confirmed_watches.push(pending.key);
            if tracking
                .last_written
                .as_ref()
                .is_some_and(|(id, _)| id == &pending.request_id)
            {
                tracking.last_written = None;
            }
        }
    }

    match event_name {
        "runs" => {
            let items = payload
                .get("items")
                .and_then(Value::as_array)
                .or_else(|| payload.as_array())
                .cloned();
            let mut shared = lock_unpoisoned(&inner.shared);
            match items {
                Some(list) => {
                    shared.runs = list;
                    shared.bump_generation();
                }
                None => {
                    shared.set_sticky_error("skipped runs event without items array".to_string());
                }
            }
        }
        "run_snapshot" => {
            let run_id = envelope_run_id
                .or_else(|| payload.get("runId").and_then(Value::as_str))
                .or_else(|| subscription_id.and_then(|s| s.strip_prefix("run:")));
            let Some(run_id) = run_id else {
                lock_unpoisoned(&inner.shared)
                    .set_sticky_error("skipped run_snapshot event without runId".to_string());
                return;
            };
            let mut shared = lock_unpoisoned(&inner.shared);
            if !shared.watched_runs.contains(run_id) {
                return;
            }
            let revision = payload.get("revision").and_then(Value::as_u64).unwrap_or(0);
            let run_revision = payload
                .get("runRevision")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            shared.run_views.insert(
                run_id.to_string(),
                RunView {
                    revision,
                    run_revision,
                    view: payload,
                },
            );
            shared.bump_generation();
        }
        "session_snapshot" => {
            let session_id = payload
                .get("sessionId")
                .and_then(Value::as_str)
                .or_else(|| subscription_id.and_then(|s| s.strip_prefix("session:")));
            let Some(session_id) = session_id else {
                lock_unpoisoned(&inner.shared).set_sticky_error(
                    "skipped session_snapshot event without sessionId".to_string(),
                );
                return;
            };
            let mut shared = lock_unpoisoned(&inner.shared);
            if !shared.watched_sessions.contains_key(session_id) {
                return;
            }
            shared.sessions.insert(session_id.to_string(), payload);
            shared.bump_generation();
        }
        "step_activity" => {
            let run_id = envelope_run_id
                .or_else(|| subscription_id.and_then(|s| s.strip_prefix("activity:")));
            let Some(run_id) = run_id else {
                lock_unpoisoned(&inner.shared)
                    .set_sticky_error("skipped step_activity event without runId".to_string());
                return;
            };
            let mut shared = lock_unpoisoned(&inner.shared);
            if !shared.watched_activities.contains(run_id) {
                return;
            }
            let mut changed = false;
            let reset = payload
                .get("reset")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if reset {
                if shared.activities.remove(run_id).is_some() {
                    changed = true;
                }
            }
            if let Some(items) = payload.get("activities").and_then(Value::as_array) {
                let mut malformed_count = 0usize;
                let stream = shared.activities.entry(run_id.to_string()).or_default();
                for item in items {
                    match StepActivity::from_json(item) {
                        Some(activity) => {
                            if activity.seq > stream.last_seq {
                                stream.last_seq = activity.seq;
                                stream.items.push(activity);
                                changed = true;
                            }
                        }
                        None => {
                            malformed_count += 1;
                        }
                    }
                }
                if malformed_count > 0 {
                    shared.set_sticky_error(format!(
                        "skipped {malformed_count} malformed step_activity item(s) for {run_id}"
                    ));
                }
            } else if !reset {
                shared.set_sticky_error(
                    "skipped step_activity event without activities array".to_string(),
                );
            }
            if changed {
                shared.bump_generation();
            }
        }
        "unavailable" => {
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .map(collapse_one_line)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    payload
                        .get("reasonCode")
                        .and_then(Value::as_str)
                        .map(|code| format!("subscription unavailable: {code}"))
                })
                .unwrap_or_else(|| "subscription unavailable".to_string());
            let mut shared = lock_unpoisoned(&inner.shared);
            shared.set_sticky_error(message);

            let run_id = envelope_run_id
                .or_else(|| subscription_id.and_then(|s| s.strip_prefix("run:")));
            if let Some(run_id) = run_id {
                if shared.run_views.remove(run_id).is_some() {
                    shared.bump_generation();
                }
            }
            if let Some(session_id) = subscription_id.and_then(|s| s.strip_prefix("session:")) {
                if shared.sessions.remove(session_id).is_some() {
                    shared.bump_generation();
                }
            }
        }
        other => {
            let mut shared = lock_unpoisoned(&inner.shared);
            shared.set_sticky_error(format!("skipped unknown server event kind: {other}"));
        }
    }
}

fn extract_error_string(error: Option<&Value>) -> Option<String> {
    match error {
        Some(Value::String(s)) => {
            let line = collapse_one_line(s);
            if line.is_empty() { None } else { Some(line) }
        }
        Some(Value::Object(map)) => map
            .get("message")
            .and_then(Value::as_str)
            .map(collapse_one_line)
            .filter(|s| !s.is_empty()),
        Some(Value::Null) | None => None,
        Some(other) => {
            let line = collapse_one_line(&other.to_string());
            if line.is_empty() { None } else { Some(line) }
        }
    }
}

/// Pure-Rust FIPS 180-4 SHA-256 returning 64 lowercase hex characters.
fn sha256_hex(data: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0x00);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = INITIAL;
    for block in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (slot, word_bytes) in w.iter_mut().take(16).zip(block.chunks_exact(4)) {
            if let Ok(arr) = <[u8; 4]>::try_from(word_bytes) {
                *slot = u32::from_be_bytes(arr);
            }
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut hex = String::with_capacity(64);
    for word in hash {
        use std::fmt::Write as _;
        let _ = write!(hex, "{word:08x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PiwClient>();
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn default_socket_path_honours_env_and_home() {
        let pinned = default_socket_path_from(
            Some(OsStr::new("/tmp/custom-piw.sock")),
            Some(Path::new("/home/alice")),
        );
        assert_eq!(pinned, Some(PathBuf::from("/tmp/custom-piw.sock")));

        let from_home =
            default_socket_path_from(Some(OsStr::new("")), Some(Path::new("/home/alice")));
        #[cfg(not(windows))]
        assert_eq!(
            from_home,
            Some(PathBuf::from(
                "/home/alice/.pi/agent/workflows/server/server.sock"
            ))
        );
        #[cfg(windows)]
        assert!(
            from_home
                .as_ref()
                .is_some_and(|p| p.to_string_lossy().starts_with(r"\\.\pipe\pi-workflows-"))
        );

        assert_eq!(default_socket_path_from(None, None), None);
    }

    #[test]
    fn canonical_json_sorts_keys_by_utf16_order_and_rejects_floats() {
        // U+E000 (UTF-16: 0xE000) vs U+10000 (UTF-16: 0xD800, 0xDC00).
        // Code-point order puts U+E000 first; UTF-16 order puts U+10000 first.
        let payload = json!({
            "\u{e000}": 1,
            "\u{10000}": 2,
            "b": true,
            "a": null,
        });
        let encoded = build_request_frame(
            "req-1",
            "client-1",
            "run.status",
            "idem-1",
            Some("run-1"),
            Some(3),
            &payload,
        )
        .expect("canonical frame");

        // Verify top-level key order and UTF-16 payload key order.
        assert!(encoded.contains(r#""payload":{"a":null,"b":true,"𐀀":2,"":1}"#));
        let keys_in_order = [
            "\"clientId\":",
            "\"expectedRevision\":",
            "\"idempotencyKey\":",
            "\"operation\":",
            "\"payload\":",
            "\"requestId\":",
            "\"runId\":",
            "\"schema\":",
            "\"type\":",
        ];
        let mut last_pos = 0;
        for marker in keys_in_order {
            let pos = encoded
                .find(marker)
                .unwrap_or_else(|| panic!("missing {marker} in {encoded}"));
            assert!(
                pos >= last_pos,
                "key {marker} out of order in {encoded}"
            );
            last_pos = pos;
        }

        // Non-integer numbers must be refused.
        assert!(canonical_json(&json!({"ratio": 1.5})).is_err());
        assert!(canonical_json(&json!({"nested": [1, 2.0]})).is_err());
    }

    #[test]
    fn step_activity_from_json_validates_all_kinds_and_fields() {
        let text = json!({
            "schema": STEP_ACTIVITY_SCHEMA,
            "runId": "run-1",
            "nodeId": "plan",
            "attemptId": "plan#1",
            "seq": 1,
            "at": "2026-01-01T00:00:00.000Z",
            "kind": "text",
            "text": "hello"
        });
        let parsed = StepActivity::from_json(&text).expect("valid text activity");
        assert_eq!(parsed.kind, ActivityKind::Text);
        assert_eq!(parsed.text, "hello");
        assert!(parsed.tool.is_none());

        let thought = json!({
            "schema": STEP_ACTIVITY_SCHEMA,
            "runId": "run-1",
            "nodeId": "plan",
            "attemptId": "plan#1",
            "seq": 2,
            "at": "2026-01-01T00:00:01.000Z",
            "kind": "thought",
            "text": "thinking"
        });
        assert_eq!(
            StepActivity::from_json(&thought).map(|a| a.kind),
            Some(ActivityKind::Thought)
        );

        let status = json!({
            "schema": STEP_ACTIVITY_SCHEMA,
            "runId": "run-1",
            "nodeId": "plan",
            "attemptId": "plan#1",
            "seq": 3,
            "at": "2026-01-01T00:00:02.000Z",
            "kind": "status",
            "text": "running tests"
        });
        assert_eq!(
            StepActivity::from_json(&status).map(|a| a.kind),
            Some(ActivityKind::Status)
        );

        // Tool with truncated absent, false, and true.
        for (truncated_field, expected) in [
            (None, false),
            (Some(json!(false)), false),
            (Some(json!(true)), true),
        ] {
            let mut tool_obj = Map::new();
            tool_obj.insert("toolCallId".into(), json!("tc-1"));
            tool_obj.insert("title".into(), json!("read file"));
            tool_obj.insert("status".into(), json!("in_progress"));
            tool_obj.insert("kind".into(), json!("read"));
            tool_obj.insert("input".into(), json!({"path": "a.rs"}));
            tool_obj.insert("output".into(), json!("contents"));
            if let Some(val) = truncated_field {
                tool_obj.insert("truncated".into(), val);
            }
            let tool_event = json!({
                "schema": STEP_ACTIVITY_SCHEMA,
                "runId": "run-1",
                "nodeId": "plan",
                "attemptId": "plan#1",
                "seq": 4,
                "at": "2026-01-01T00:00:03.000Z",
                "kind": "tool",
                "text": "",
                "tool": Value::Object(tool_obj)
            });
            let act = StepActivity::from_json(&tool_event).expect("valid tool activity");
            assert_eq!(act.kind, ActivityKind::Tool);
            let tool = act.tool.expect("tool present");
            assert_eq!(tool.tool_call_id, "tc-1");
            assert_eq!(tool.title, "read file");
            assert_eq!(tool.status.as_deref(), Some("in_progress"));
            assert_eq!(tool.kind.as_deref(), Some("read"));
            assert_eq!(tool.input, Some(json!({"path": "a.rs"})));
            assert_eq!(tool.output, Some(json!("contents")));
            assert_eq!(tool.truncated, expected);
        }

        // Reject missing required fields.
        for field in [
            "schema",
            "runId",
            "nodeId",
            "attemptId",
            "seq",
            "at",
            "kind",
            "text",
        ] {
            let mut missing = text.clone();
            if let Some(map) = missing.as_object_mut() {
                map.remove(field);
            }
            assert!(
                StepActivity::from_json(&missing).is_none(),
                "expected None when missing {field}"
            );
        }

        // Reject tool kind without tool field.
        let tool_without_tool = json!({
            "schema": STEP_ACTIVITY_SCHEMA,
            "runId": "run-1",
            "nodeId": "plan",
            "attemptId": "plan#1",
            "seq": 5,
            "at": "2026-01-01T00:00:04.000Z",
            "kind": "tool",
            "text": ""
        });
        assert!(StepActivity::from_json(&tool_without_tool).is_none());

        // Reject non-tool kind with tool field present.
        let mut text_with_tool = text.clone();
        text_with_tool["tool"] = json!({"toolCallId": "tc-1", "title": "x"});
        assert!(StepActivity::from_json(&text_with_tool).is_none());
    }

    #[cfg(unix)]
    mod unix_tests {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::net::UnixListener;

        struct FakeServer {
            socket_path: PathBuf,
            shutdown_tx: watch::Sender<bool>,
            thread: Option<JoinHandle<()>>,
            _dir: tempfile::TempDir,
        }

        impl FakeServer {
            fn start<F, Fut>(handler: F) -> Self
            where
                F: Fn(usize, tokio::net::UnixStream) -> Fut + Send + Sync + 'static,
                Fut: std::future::Future<Output = ()> + Send + 'static,
            {
                let dir = tempfile::tempdir().expect("tempdir");
                let socket_path = dir.path().join("server.sock");
                Self::start_at(dir, socket_path, handler)
            }

            fn start_at<F, Fut>(dir: tempfile::TempDir, socket_path: PathBuf, handler: F) -> Self
            where
                F: Fn(usize, tokio::net::UnixStream) -> Fut + Send + Sync + 'static,
                Fut: std::future::Future<Output = ()> + Send + 'static,
            {
                let _ = std::fs::remove_file(&socket_path);
                let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
                let (ready_tx, ready_rx) = std::sync::mpsc::channel();
                let bind_path = socket_path.clone();
                let handler = Arc::new(handler);

                let thread = std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("test runtime");
                    rt.block_on(async move {
                        let listener = UnixListener::bind(&bind_path).expect("bind unix socket");
                        let _ = ready_tx.send(());
                        let conn_num = Arc::new(AtomicUsize::new(0));
                        loop {
                            tokio::select! {
                                biased;
                                _ = shutdown_rx.changed() => {
                                    if *shutdown_rx.borrow() {
                                        break;
                                    }
                                }
                                accepted = listener.accept() => {
                                    match accepted {
                                        Ok((stream, _)) => {
                                            let idx = conn_num.fetch_add(1, Ordering::SeqCst) + 1;
                                            let handler = Arc::clone(&handler);
                                            tokio::spawn(async move {
                                                handler(idx, stream).await;
                                            });
                                        }
                                        Err(_) => break,
                                    }
                                }
                            }
                        }
                    });
                });
                ready_rx.recv().expect("server bound");
                FakeServer {
                    socket_path,
                    shutdown_tx,
                    thread: Some(thread),
                    _dir: dir,
                }
            }

            fn stop(&mut self) {
                let _ = self.shutdown_tx.send(true);
                if let Some(handle) = self.thread.take() {
                    let _ = handle.join();
                }
            }
        }

        impl Drop for FakeServer {
            fn drop(&mut self) {
                self.stop();
            }
        }

        async fn send_json<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) {
            let line = format!("{}\n", canonical_json(value).expect("canonical"));
            let _ = writer.write_all(line.as_bytes()).await;
            let _ = writer.flush().await;
        }

        async fn send_hello<W: AsyncWrite + Unpin>(writer: &mut W, version: &str) {
            send_json(
                writer,
                &json!({
                    "schema": PROTOCOL_ID,
                    "type": "hello",
                    "connectionId": format!("conn-{}", Uuid::new_v4()),
                    "packageVersion": version,
                }),
            )
            .await;
        }

        fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                if pred() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("condition did not become true within {timeout:?}");
        }

        fn activity_json(run_id: &str, seq: u64, kind: &str, text: &str) -> Value {
            json!({
                "schema": STEP_ACTIVITY_SCHEMA,
                "runId": run_id,
                "nodeId": "work",
                "attemptId": "work#1",
                "seq": seq,
                "at": "2026-01-01T00:00:00.000Z",
                "kind": kind,
                "text": text,
            })
        }

        #[test]
        fn handshake_accepts_other_package_version_and_delivers_runs_and_run_snapshots() {
            let server = FakeServer::start(|_conn, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "99.88.77-other").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                    let op = req["operation"].as_str().unwrap_or("");
                    let sub_id = req["payload"]["subscriptionId"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    match op {
                        "view.runs.watch" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "runs",
                                    "revision": 1,
                                    "payload": {
                                        "schema": "pi-workflows.run-list-page.v1",
                                        "revision": "1:0:0",
                                        "start": 0,
                                        "total": 2,
                                        "items": [
                                            {"runId": "r1", "workflowName": "alpha"},
                                            {"runId": "r2", "workflowName": "beta"}
                                        ]
                                    }
                                }),
                            )
                            .await;
                        }
                        "view.run.watch" => {
                            let run_id = req["runId"].as_str().unwrap_or("").to_string();
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "run_snapshot",
                                    "revision": 1,
                                    "runId": run_id,
                                    "payload": {
                                        "schema": "pi-workflows.run-view.v1",
                                        "runId": run_id,
                                        "revision": 10,
                                        "runRevision": 2,
                                        "display": {"status": "running"}
                                    }
                                }),
                            )
                            .await;
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "run_snapshot",
                                    "revision": 2,
                                    "runId": run_id,
                                    "payload": {
                                        "schema": "pi-workflows.run-view.v1",
                                        "runId": run_id,
                                        "revision": 11,
                                        "runRevision": 3,
                                        "display": {"status": "completed"}
                                    }
                                }),
                            )
                            .await;
                        }
                        _ => {}
                    }
                }
            });

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            client.watch_runs();
            client.watch_run("r1");

            wait_until(Duration::from_secs(3), || {
                client.state() == ConnState::Connected
                    && client.runs().len() == 2
                    && client
                        .run_view("r1")
                        .is_some_and(|v| v.revision == 11 && v.run_revision == 3)
            });

            assert_eq!(client.server_version().as_deref(), Some("99.88.77-other"));
            assert_eq!(client.connection_count(), 1);
            assert!(client.generation() >= 3);
            let runs = client.runs();
            assert_eq!(runs[0]["runId"], "r1");
            assert_eq!(runs[1]["runId"], "r2");
            let view = client.run_view("r1").expect("run view");
            assert_eq!(view.view["display"]["status"], "completed");

            client.unwatch_run("r1");
            assert!(client.run_view("r1").is_none());
            client.close();
        }

        #[test]
        fn send_take_response_and_call_success_refusal_and_timeout() {
            let server = FakeServer::start(|_conn, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "1.0.0").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                    let op = req["operation"].as_str().unwrap_or("");
                    match op {
                        "run.cancel" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"cancelled": true}
                                }),
                            )
                            .await;
                        }
                        "decision.answer" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "refused",
                                    "error": {"message": "no coordinator role held"}
                                }),
                            )
                            .await;
                        }
                        // "run.pause" gets no answer: `call` must time out on it.
                        _ => {}
                    }
                }
            });

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            wait_until(Duration::from_secs(5), || client.state() == ConnState::Connected);

            // `send` + `take_response` success.
            let req = client.send("run.cancel", Some("r1"), Some(3), json!({"why": "user"}));
            let response = loop {
                if let Some(response) = client.take_response(&req) {
                    break response;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(response.ok());
            assert_eq!(response.operation, "run.cancel");
            assert_eq!(response.run_id.as_deref(), Some("r1"));
            assert_eq!(response.receipt.as_ref().and_then(|r| r["cancelled"].as_bool()), Some(true));
            assert!(client.take_response(&req).is_none(), "response is delivered once");

            // `call` success.
            let ok = client
                .call("run.cancel", Some("r1"), Some(4), json!({}), Duration::from_secs(2))
                .expect("call succeeds");
            assert!(ok.ok());

            // `call` refusal surfaces `error_text`.
            let refused = client
                .call("decision.answer", None, None, json!({}), Duration::from_secs(2))
                .expect("refusal is a response, not an error");
            assert!(!refused.ok());
            assert_eq!(refused.error_text().as_deref(), Some("no coordinator role held"));

            // `call` timeout.
            let timed_out = client.call(
                "run.pause",
                Some("r1"),
                None,
                json!({}),
                Duration::from_millis(300),
            );
            assert!(timed_out.is_err());
            assert!(timed_out.unwrap_err().to_string().contains("run.pause"));

            // A payload with a non-integer number is refused locally and never sent.
            let bad = client.send("run.cancel", Some("r1"), None, json!({"ratio": 1.5}));
            let bad_response = client.take_response(&bad).expect("local refusal is immediate");
            assert!(!bad_response.ok());
            assert!(bad_response.error_text().is_some_and(|t| t.contains("non-integer")));

            client.close();
        }

        #[test]
        fn request_queued_while_disconnected_is_sent_after_connect() {
            let dir = tempfile::tempdir().expect("tempdir");
            let socket_path = dir.path().join("late.sock");

            let client = PiwClient::connect(&socket_path).expect("connect");
            client.watch_runs();
            wait_until(Duration::from_secs(5), || client.state() == ConnState::Reconnecting);

            let req = client.send("run.status", Some("r1"), None, json!({}));
            assert!(client.take_response(&req).is_none(), "queued, not refused");

            let server = FakeServer::start_at(dir, socket_path, |_conn, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "1.0.0").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let reqv: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = reqv["requestId"].as_str().unwrap_or("").to_string();
                    let op = reqv["operation"].as_str().unwrap_or("");
                    let sub_id = reqv["payload"]["subscriptionId"].as_str().unwrap_or("").to_string();
                    match op {
                        "view.runs.watch" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "runs",
                                    "revision": 1,
                                    "payload": {
                                        "schema": "pi-workflows.run-list-page.v1",
                                        "revision": "1:0:0",
                                        "start": 0,
                                        "total": 1,
                                        "items": [{"runId": "r1", "workflowName": "late"}]
                                    }
                                }),
                            )
                            .await;
                        }
                        "run.status" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"status": "running"}
                                }),
                            )
                            .await;
                        }
                        _ => {}
                    }
                }
            });

            wait_until(Duration::from_secs(15), || {
                client.state() == ConnState::Connected && client.runs().len() == 1
            });
            let response = loop {
                if let Some(response) = client.take_response(&req) {
                    break response;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(response.ok());
            assert_eq!(response.operation, "run.status");
            assert!(client.last_error().is_none(), "no error for a late server: {:?}", client.last_error());
            client.close();
        }

        #[test]
        fn reconnect_resends_watches_completes_unanswered_request_and_clears_session() {
            let server = FakeServer::start(|conn_idx, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "1.0.0").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                    let op = req["operation"].as_str().unwrap_or("");
                    let sub_id = req["payload"]["subscriptionId"].as_str().unwrap_or("").to_string();
                    match op {
                        "view.runs.watch" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "runs",
                                    "revision": 1,
                                    "payload": {
                                        "schema": "pi-workflows.run-list-page.v1",
                                        "revision": "1:0:0",
                                        "start": 0,
                                        "total": 1,
                                        "items": [{"runId": "r1", "workflowName": "alpha"}]
                                    }
                                }),
                            )
                            .await;
                        }
                        "view.session.watch" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "session_snapshot",
                                    "payload": {
                                        "sessionId": "sess-1",
                                        "coordinatorEpoch": if conn_idx == 1 { Value::Null } else { json!("session-epoch-2") },
                                        "coordinatorActive": false,
                                        "branchReportRequired": false
                                    }
                                }),
                            )
                            .await;
                        }
                        "decision.answer" if conn_idx == 1 => {
                            // The first connection reads the answer and drops without answering.
                            return;
                        }
                        "decision.answer" => {
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"answered": true}
                                }),
                            )
                            .await;
                        }
                        _ => {}
                    }
                }
            });

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            client.watch_runs();
            client.watch_session("sess-1", false);
            wait_until(Duration::from_secs(5), || {
                client.state() == ConnState::Connected
                    && client.runs().len() == 1
                    && client.session("sess-1").is_some()
            });
            assert_eq!(client.connection_count(), 1);

            let req = client.send("decision.answer", Some("r1"), Some(7), json!({"choice": "go"}));
            // The drop completes the in-flight request with CONNECTION_LOST and clears the session.
            let response = loop {
                if let Some(response) = client.take_response(&req) {
                    break response;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(response.outcome, CONNECTION_LOST);
            assert!(!response.ok());
            assert!(client.session("sess-1").is_none(), "session snapshot died with the connection");

            // Reconnect re-sends both watches; the session snapshot comes back.
            wait_until(Duration::from_secs(10), || {
                client.connection_count() == 2
                    && client.session("sess-1").is_some()
                    && client
                        .session("sess-1")
                        .is_some_and(|s| s["coordinatorEpoch"] == json!("session-epoch-2"))
            });
            assert!(client.runs().len() == 1, "runs cache survived the reconnect");
            client.close();
        }

        #[test]
        fn activity_in_order_deduplicates_and_resets_after_restart() {
            let after_seqs_server: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
            let server = {
                let after_seqs = Arc::clone(&after_seqs_server);
                FakeServer::start(move |conn_idx, stream| {
                    let after_seqs = Arc::clone(&after_seqs);
                    async move {
                        let (r, mut w) = tokio::io::split(stream);
                        send_hello(&mut w, "1.0.0").await;
                        let mut lines = BufReader::new(r).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            let req: Value = serde_json::from_str(&line).expect("req json");
                            let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                            let op = req["operation"].as_str().unwrap_or("");
                            let sub_id = req["payload"]["subscriptionId"]
                                .as_str()
                                .unwrap_or("")
                                .to_string();
                            if op == "view.run.activity.watch" {
                                let after_seq = req["payload"]["afterSeq"].as_u64().unwrap_or(999);
                                lock_unpoisoned(&after_seqs).push(after_seq);
                                send_json(
                                    &mut w,
                                    &json!({
                                        "schema": PROTOCOL_ID,
                                        "type": "response",
                                        "requestId": req_id,
                                        "outcome": "accepted",
                                        "receipt": {"subscribed": true}
                                    }),
                                )
                                .await;
                                if conn_idx == 1 {
                                    // Seq 1 and 2, then a duplicate seq 2 and a new seq 3, then a
                                    // simulated server restart (drop).
                                    send_json(
                                        &mut w,
                                        &json!({
                                            "schema": PROTOCOL_ID,
                                            "type": "event",
                                            "subscriptionId": sub_id,
                                            "event": "step_activity",
                                            "runId": "r1",
                                            "payload": {"activities": [
                                                activity_json("r1", 1, "text", "a"),
                                                activity_json("r1", 2, "text", "b")
                                            ]}
                                        }),
                                    )
                                    .await;
                                    send_json(
                                        &mut w,
                                        &json!({
                                            "schema": PROTOCOL_ID,
                                            "type": "event",
                                            "subscriptionId": sub_id,
                                            "event": "step_activity",
                                            "runId": "r1",
                                            "payload": {"activities": [
                                                activity_json("r1", 2, "text", "b-dup"),
                                                activity_json("r1", 3, "text", "c")
                                            ]}
                                        }),
                                    )
                                    .await;
                                    // Drop: the server "restarts".
                                    return;
                                }
                                // Second connection: renumbered stream with a reset marker.
                                send_json(
                                    &mut w,
                                    &json!({
                                        "schema": PROTOCOL_ID,
                                        "type": "event",
                                        "subscriptionId": sub_id,
                                        "event": "step_activity",
                                        "runId": "r1",
                                        "payload": {
                                            "reset": true,
                                            "activities": [
                                                activity_json("r1", 1, "text", "x"),
                                                activity_json("r1", 2, "text", "y")
                                            ]
                                        }
                                    }),
                                )
                                .await;
                            }
                        }
                    }
                })
            };

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            client.watch_activity("r1");
            wait_until(Duration::from_secs(5), || {
                client.activity("r1", 0).len() == 3
                    && client.activity("r1", 0)[2].text == "c"
            });
            let activity = client.activity("r1", 0);
            assert_eq!(
                activity.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(),
                ["a", "b", "c"],
                "the duplicate seq 2 was dropped"
            );

            wait_until(Duration::from_secs(10), || {
                client.connection_count() == 2 && client.activity("r1", 0).len() == 2
            });
            let restarted = client.activity("r1", 0);
            assert_eq!(
                restarted.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(),
                ["x", "y"],
                "the reset discarded the pre-restart activity"
            );
            assert_eq!(*lock_unpoisoned(&after_seqs_server), vec![0, 3]);
            client.close();
        }

        #[test]
        fn activity_watch_dropping_connection_twice_is_switched_off_while_other_watches_deliver() {
            let server = FakeServer::start(|conn_idx, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "1.0.0").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                    let op = req["operation"].as_str().unwrap_or("");
                    let sub_id = req["payload"]["subscriptionId"].as_str().unwrap_or("").to_string();
                    match op {
                        "view.runs.watch" => {
                            let name = match conn_idx {
                                1 => "alpha",
                                2 => "beta",
                                _ => "gamma",
                            };
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "runs",
                                    "revision": 1,
                                    "payload": {
                                        "schema": "pi-workflows.run-list-page.v1",
                                        "revision": "1:0:0",
                                        "start": 0,
                                        "total": 1,
                                        "items": [{"runId": "r1", "workflowName": name}]
                                    }
                                }),
                            )
                            .await;
                        }
                        "view.run.watch" => {
                            let run_id = req["runId"].as_str().unwrap_or("").to_string();
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "response",
                                    "requestId": req_id,
                                    "outcome": "accepted",
                                    "receipt": {"subscribed": true}
                                }),
                            )
                            .await;
                            send_json(
                                &mut w,
                                &json!({
                                    "schema": PROTOCOL_ID,
                                    "type": "event",
                                    "subscriptionId": sub_id,
                                    "event": "run_snapshot",
                                    "revision": 1,
                                    "runId": run_id,
                                    "payload": {
                                        "schema": "pi-workflows.run-view.v1",
                                        "runId": run_id,
                                        "revision": 10,
                                        "runRevision": 1,
                                        "display": {"status": "running"}
                                    }
                                }),
                            )
                            .await;
                        }
                        // A server without the activity operations closes the connection.
                        "view.run.activity.watch" => {
                            return;
                        }
                        _ => {}
                    }
                }
            });

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            client.watch_runs();
            client.watch_run("r1");
            client.watch_activity("r1");
            wait_until(Duration::from_secs(10), || {
                client.connection_count() == 3
                    && client.runs().len() == 1
                    && client.runs()[0]["workflowName"] == "gamma"
                    && client
                        .last_error()
                        .as_deref()
                        .is_some_and(|e| e.contains("disabled activity watch"))
            });
            assert_eq!(client.state(), ConnState::Connected);
            assert!(
                client.run_view("r1").is_some_and(|v| v.view["display"]["status"] == "running"),
                "the run watch kept working after the activity watch was disabled"
            );
            client.close();
        }

        #[test]
        fn send_keyed_puts_the_callers_idempotency_key_on_the_wire() {
            let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let server = {
                let captured = Arc::clone(&captured);
                FakeServer::start(move |_conn, stream| {
                    let captured = Arc::clone(&captured);
                    async move {
                        let (r, mut w) = tokio::io::split(stream);
                        send_hello(&mut w, "1.0.0").await;
                        let mut lines = BufReader::new(r).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            lock_unpoisoned(&captured).push(line.clone());
                            let req: Value = serde_json::from_str(&line).expect("req json");
                            let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                            let op = req["operation"].as_str().unwrap_or("");
                            if op == "workflowMessage.reportBranch" {
                                send_json(
                                    &mut w,
                                    &json!({
                                        "schema": PROTOCOL_ID,
                                        "type": "response",
                                        "requestId": req_id,
                                        "outcome": "accepted",
                                        "receipt": {"reported": true}
                                    }),
                                )
                                .await;
                            }
                        }
                    }
                })
            };

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            wait_until(Duration::from_secs(5), || client.state() == ConnState::Connected);
            let req = client.send_keyed(
                "workflowMessage.reportBranch",
                None,
                None,
                json!({"branch": "main"}),
                "stable-key-1",
            );
            let response = loop {
                if let Some(response) = client.take_response(&req) {
                    break response;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(response.ok());
            let frames = lock_unpoisoned(&captured);
            let frame = frames
                .iter()
                .find(|f| f.contains("workflowMessage.reportBranch"))
                .expect("request reached the wire");
            assert!(frame.contains(r#""idempotencyKey":"stable-key-1""#));
            client.close();
        }

        #[test]
        fn content_reference_is_fetched_in_two_chunks() {
            let full_text = "hello workflow world";
            let reference = json!({
                "$artifact": {
                    "path": "steps/plan/output.txt",
                    "mediaType": "text/plain",
                    "bytes": full_text.len() as u64,
                    "sha256": sha256_hex(full_text.as_bytes())
                }
            });
            let server = FakeServer::start(move |_conn, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "1.0.0").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                    let op = req["operation"].as_str().unwrap_or("");
                    if op == "view.content" {
                        let offset = req["payload"]["offset"].as_u64().unwrap_or(0);
                        let path = req["payload"]["path"].as_str().unwrap_or("").to_string();
                        let chunk_end = (offset as usize + 11).min(full_text.len());
                        let chunk = &full_text[offset as usize..chunk_end];
                        let complete = chunk_end == full_text.len();
                        send_json(
                            &mut w,
                            &json!({
                                "schema": PROTOCOL_ID,
                                "type": "response",
                                "requestId": req_id,
                                "outcome": "accepted",
                                "receipt": {
                                    "path": path,
                                    "mediaType": "text/plain",
                                    "bytes": full_text.len() as u64,
                                    "sha256": sha256_hex(full_text.as_bytes()),
                                    "offset": offset,
                                    "nextOffset": chunk_end as u64,
                                    "complete": complete,
                                    "data": BASE64.encode(chunk.as_bytes())
                                }
                            }),
                        )
                        .await;
                    }
                }
            });

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            wait_until(Duration::from_secs(5), || client.state() == ConnState::Connected);
            assert_eq!(client.content("r1", &reference), Content::Pending);
            wait_until(Duration::from_secs(5), || {
                matches!(client.content("r1", &reference), Content::Ready(_))
            });
            assert_eq!(
                client.content("r1", &reference),
                Content::Ready(full_text.to_string())
            );
            // A value that is not a reference is Invalid.
            assert!(matches!(
                client.content("r1", &json!({"text": "plain"})),
                Content::Invalid(_)
            ));
            client.close();
        }

        #[test]
        fn oversized_nonjson_and_unknown_frames_are_skipped_and_connection_survives() {
            let server = FakeServer::start(|_conn, stream| async move {
                let (r, mut w) = tokio::io::split(stream);
                send_hello(&mut w, "1.0.0").await;
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).expect("req json");
                    let req_id = req["requestId"].as_str().unwrap_or("").to_string();
                    let op = req["operation"].as_str().unwrap_or("");
                    let sub_id = req["payload"]["subscriptionId"].as_str().unwrap_or("").to_string();
                    if op == "view.runs.watch" {
                        send_json(
                            &mut w,
                            &json!({
                                "schema": PROTOCOL_ID,
                                "type": "response",
                                "requestId": req_id,
                                "outcome": "accepted",
                                "receipt": {"subscribed": true}
                            }),
                        )
                        .await;

                        // A frame that is not JSON.
                        let _ = w
                            .write_all(b"this is not json at all\n")
                            .await;
                        let _ = w.flush().await;

                        // A frame over the 1 MiB protocol limit.
                        let mut oversized = vec![b'x'; MAX_PROTOCOL_MESSAGE_BYTES + 64];
                        oversized.push(b'\n');
                        let _ = w.write_all(&oversized).await;
                        let _ = w.flush().await;

                        // An event of an unknown kind (delivered last so the sticky error is
                        // deterministic).
                        send_json(
                            &mut w,
                            &json!({
                                "schema": PROTOCOL_ID,
                                "type": "event",
                                "subscriptionId": sub_id,
                                "event": "mystery",
                                "payload": {}
                            }),
                        )
                        .await;

                        // The connection keeps working afterwards.
                        send_json(
                            &mut w,
                            &json!({
                                "schema": PROTOCOL_ID,
                                "type": "event",
                                "subscriptionId": sub_id,
                                "event": "runs",
                                "revision": 1,
                                "payload": {
                                    "schema": "pi-workflows.run-list-page.v1",
                                    "revision": "1:0:0",
                                    "start": 0,
                                    "total": 1,
                                    "items": [{"runId": "r1", "workflowName": "skipped"}]
                                }
                            }),
                        )
                        .await;
                    }
                }
            });

            let client = PiwClient::connect(&server.socket_path).expect("connect");
            client.watch_runs();
            wait_until(Duration::from_secs(5), || {
                client.runs().len() == 1 && client.runs()[0]["workflowName"] == "skipped"
            });
            assert_eq!(client.state(), ConnState::Connected);
            assert_eq!(client.connection_count(), 1, "no reconnect was needed");
            assert!(
                client
                    .last_error()
                    .as_deref()
                    .is_some_and(|e| e.contains("skipped unknown server event kind: mystery")),
                "unexpected last error: {:?}",
                client.last_error()
            );
            client.close();
        }

}
}
