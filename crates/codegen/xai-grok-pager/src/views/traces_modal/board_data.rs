//! What the sessions board needs beyond the index rows: each session's active time, and a generated line on where it ended up.
//! Both read the traces index with the `sqlite3` CLI, locally or on another machine over ssh, so one query serves both.
//!
//! * Active time: the sum of the gaps between a session's consecutive events, counting a gap only when it is at most
//!   [`IDLE_GAP_MS`]. A longer gap is the agent waiting (on the user, or on a subagent), not working.
//! * Summary: `claude -p --model haiku` on the session's last prompts and replies, at most [`MAX_SUMMARIES`] per run,
//!   cached in `<traces dir>/board-summaries.json` and redone only when the session changed. Live sessions are not summarized.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use xai_grok_traces::SessionRow;
use xai_grok_traces::remote;

pub const IDLE_GAP_MS: i64 = 5 * 60 * 1000;
pub const MAX_SUMMARIES: usize = 30;
const WORKERS: usize = 4;
const ACTIVE_EVERY: Duration = Duration::from_secs(60);
/// Sessions updated within this window get an active time.
const ACTIVE_SINCE_MS: i64 = 60 * 86_400_000;

fn traces_dir() -> PathBuf {
    xai_grok_traces::default_db_path().parent().map(PathBuf::from).unwrap_or_default()
}

/// `sqlite3 -json` on this machine's index (`host == None`) or another's over ssh. `None` on any failure.
fn query(target: Option<&str>, sql: &str, timeout: Duration) -> Option<Vec<Value>> {
    let mut cmd = match target {
        None => {
            let mut c = Command::new("sqlite3");
            c.arg("-json").arg(xai_grok_traces::default_db_path()).arg(sql);
            c
        }
        Some(t) => {
            let quoted = shlex::try_quote(sql).ok()?.into_owned();
            let mut c = Command::new("ssh");
            c.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=3", t])
                .arg(format!("sqlite3 -json ~/.grok/traces/traces.sqlite {quoted}"));
            c
        }
    };
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let out = child.stdout.take()?;
    let reader = std::thread::spawn(move || std::io::read_to_string(out).ok());
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let text = reader.join().ok()??;
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    serde_json::from_str::<Vec<Value>>(&text).ok()
}

fn active_sql(now_ms: i64) -> String {
    format!(
        "SELECT s.id AS i, SUM(CASE WHEN g BETWEEN 0 AND {IDLE_GAP_MS} THEN g ELSE 0 END) AS a FROM \
         (SELECT sk, timestamp - LAG(timestamp) OVER (PARTITION BY sk ORDER BY line_number) AS g FROM trace_events \
          WHERE timestamp IS NOT NULL AND sk IN (SELECT sk FROM trace_sessions WHERE updated_at > {})) e \
         JOIN trace_sessions s ON s.sk = e.sk GROUP BY e.sk",
        now_ms - ACTIVE_SINCE_MS
    )
}

/// `(id, active ms)` rows into a map; remote ids get the `label:` prefix the board's rows carry.
fn active_map(rows: Vec<Value>, label: Option<&str>) -> HashMap<String, i64> {
    rows.into_iter()
        .filter_map(|r| {
            let id = r.get("i")?.as_str()?;
            let ms = r.get("a")?.as_i64().unwrap_or(0);
            Some((label.map_or_else(|| id.to_string(), |l| format!("{l}:{id}")), ms))
        })
        .collect()
}

/// Active time per session id, refreshed in the background.
pub struct ActiveTimes {
    pub ms: HashMap<String, i64>,
    rx: Option<mpsc::Receiver<HashMap<String, i64>>>,
    at: Option<Instant>,
}

impl ActiveTimes {
    pub fn new() -> Self {
        ActiveTimes { ms: HashMap::new(), rx: None, at: None }
    }

    /// Starts a refresh when due; takes a finished one. Returns whether new times arrived.
    pub fn poll(&mut self, hosts: &[String], now_ms: i64) -> bool {
        if let Some(rx) = &self.rx {
            return match rx.try_recv() {
                Ok(map) => {
                    self.ms.extend(map);
                    true
                }
                Err(mpsc::TryRecvError::Empty) => false,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    false
                }
            };
        }
        if self.at.is_some_and(|t| t.elapsed() < ACTIVE_EVERY) {
            return false;
        }
        self.at = Some(Instant::now());
        let (tx, rx) = mpsc::channel();
        let hosts = hosts.to_vec();
        std::thread::spawn(move || {
            let sql = active_sql(now_ms);
            if let Some(rows) = query(None, &sql, Duration::from_secs(10)) {
                let _ = tx.send(active_map(rows, None));
            }
            for t in hosts {
                if let Some(rows) = query(Some(&t), &sql, Duration::from_secs(15)) {
                    let _ = tx.send(active_map(rows, Some(remote::label(&t))));
                }
            }
        });
        self.rx = Some(rx);
        false
    }
}

/// When a summary was made: the session's last activity then. A later activity makes it stale.
fn stamp(row: &SessionRow) -> i64 {
    row.last_event_at.or(row.updated_at).unwrap_or(0)
}

struct Job {
    id: String,
    stamp: i64,
    target: Option<String>,
    native: String,
    title: String,
    failed: bool,
}

/// Generated "where it ended up" lines, by session id.
pub struct Summaries {
    pub text: HashMap<String, String>,
    stamps: HashMap<String, i64>,
    queued: HashSet<String>,
    started: usize,
    queue: Arc<Mutex<VecDeque<Job>>>,
    tx: mpsc::Sender<(String, i64, String)>,
    rx: mpsc::Receiver<(String, i64, String)>,
    workers: usize,
    path: PathBuf,
    /// Off in tests and wherever `claude` is not wanted.
    pub enabled: bool,
}

impl Summaries {
    pub fn load(enabled: bool) -> Self {
        let path = traces_dir().join("board-summaries.json");
        let (mut text, mut stamps) = (HashMap::new(), HashMap::new());
        if let Some(Value::Object(map)) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            for (id, v) in map {
                if let (Some(t), Some(at)) = (v.get("text").and_then(Value::as_str), v.get("at").and_then(Value::as_i64)) {
                    text.insert(id.clone(), t.to_string());
                    stamps.insert(id, at);
                }
            }
        }
        let (tx, rx) = mpsc::channel();
        Summaries {
            text,
            stamps,
            queued: HashSet::new(),
            started: 0,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            tx,
            rx,
            workers: 0,
            path,
            enabled,
        }
    }

    /// Queues the most recent ended sessions whose summary is missing or stale, up to the per-run cap.
    pub fn request(&mut self, rows: &[&SessionRow], hosts: &[String]) {
        if !self.enabled {
            return;
        }
        let mut ended: Vec<&&SessionRow> = rows.iter().filter(|r| r.status != xai_grok_traces::Status::Active).collect();
        ended.sort_by_key(|r| std::cmp::Reverse(stamp(r)));
        for r in ended.into_iter().take(MAX_SUMMARIES) {
            if self.started >= MAX_SUMMARIES {
                break;
            }
            if self.stamps.get(&r.id) == Some(&stamp(r)) || self.queued.contains(&r.id) {
                continue;
            }
            let target = match r.host.as_deref() {
                None => None,
                Some(h) => match hosts.iter().find(|t| remote::label(t) == h) {
                    Some(t) => Some(t.clone()),
                    None => continue,
                },
            };
            let native = match &target {
                Some(t) => remote::remote_id(t, &r.id).to_string(),
                None => r.id.clone(),
            };
            self.queued.insert(r.id.clone());
            self.started += 1;
            let job = Job { id: r.id.clone(), stamp: stamp(r), target, native, title: super::clean_name(r), failed: r.failed };
            self.queue.lock().unwrap().push_back(job);
        }
        while self.workers < WORKERS && !self.queue.lock().unwrap().is_empty() {
            self.workers += 1;
            let (queue, tx) = (self.queue.clone(), self.tx.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(job) = queue.lock().unwrap().pop_front() else { break };
                    if let Some(text) = summarize(&job) {
                        let _ = tx.send((job.id, job.stamp, text));
                    }
                }
            });
        }
    }

    /// Takes finished summaries and saves the cache. Returns whether any arrived.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        while let Ok((id, at, text)) = self.rx.try_recv() {
            self.queued.remove(&id);
            self.text.insert(id.clone(), text);
            self.stamps.insert(id, at);
            got = true;
        }
        if got {
            let map: serde_json::Map<String, Value> = self
                .text
                .iter()
                .map(|(id, t)| (id.clone(), serde_json::json!({ "text": t, "at": self.stamps.get(id) })))
                .collect();
            if let Ok(bytes) = serde_json::to_vec_pretty(&Value::Object(map)) {
                let tmp = self.path.with_extension("json.tmp");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &self.path);
                }
            }
        }
        got
    }
}

/// The session's last prompts and replies (index previews), oldest first.
fn tail_text(job: &Job) -> Option<String> {
    let id = job.native.replace('\'', "''");
    let sql = format!(
        "SELECT e.role AS r, e.preview AS p, e.is_error AS x FROM trace_events e WHERE e.sk = \
         (SELECT sk FROM trace_sessions WHERE id = '{id}') AND e.noise = 0 AND (e.role IN (0, 1) OR e.is_error = 1) \
         AND e.preview != '' ORDER BY e.line_number DESC LIMIT 16"
    );
    let rows = query(job.target.as_deref(), &sql, Duration::from_secs(10))?;
    let mut lines: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            let who = match (r.get("r").and_then(Value::as_i64), r.get("x").and_then(Value::as_i64)) {
                (_, Some(1)) => "error",
                (Some(0), _) => "user",
                _ => "agent",
            };
            Some(format!("{who}: {}", r.get("p")?.as_str()?))
        })
        .collect();
    lines.reverse();
    // Nothing to go on: no call, rather than a summary that says so.
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn summarize(job: &Job) -> Option<String> {
    let tail = tail_text(job)?;
    let prompt = format!(
        "This is the end of an AI coding agent session, as short previews.\nTask: {}\n{}\n{}\n\n\
         In one plain line of at most 14 words, say where the session ended up: what got done, or the state it was left in. \
         No preamble, no quotes, no markdown.",
        job.title,
        if job.failed { "It ended on an error." } else { "" },
        tail
    );
    let dir = traces_dir().join("board-summarize");
    std::fs::create_dir_all(&dir).ok()?;
    // No session file, no user hooks or MCP servers: the call must not show up as a session of its own.
    let mut child = Command::new("claude")
        .args(["-p", "--model", "haiku", "--no-session-persistence", "--setting-sources", "project", "--strict-mcp-config"])
        .args(["--disable-slash-commands", "--tools", ""])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(prompt.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().lines().next()?.trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_map_prefixes_remote_ids() {
        let rows = vec![serde_json::json!({"i": "claude:1", "a": 61000}), serde_json::json!({"i": "pi:2", "a": null})];
        let local = active_map(rows.clone(), None);
        assert_eq!(local.get("claude:1"), Some(&61000));
        let air = active_map(rows, Some("sab-air"));
        assert_eq!(air.get("sab-air:claude:1"), Some(&61000));
        assert_eq!(air.get("sab-air:pi:2"), Some(&0));
    }
}
