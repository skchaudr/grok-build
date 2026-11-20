```rust
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

pub use xai_grok_pager::acpx::bundle::{fmt_duration, parse_ts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Acp,
    Shell,
    Compute,
    Checkpoint,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchNode {
    pub id: String,
    pub kind: StepKind,
    pub summary: Option<String>,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchEdge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowSketch {
    pub name: String,
    pub start_at: String,
    pub nodes: Vec<SketchNode>,
    pub edges: Vec<SketchEdge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateChoice {
    pub label: String,
    pub flow_file: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    RunStarted {
        run_id: String,
        run_dir: PathBuf,
        title: String,
        flow_path: String,
        flow: FlowSketch,
        at_ms: i64,
    },
    StepStarted {
        node: String,
        attempt: String,
        kind: StepKind,
        at_ms: i64,
    },
    /// The prompt text acpx sent to the step's agent (outbound session/prompt, text blocks joined by "\n").
    AgentPrompt {
        node: String,
        attempt: String,
        text: String,
        at_ms: i64,
    },
    /// One inbound `session/update` of the step's agent: `update` is the JSON object at message.params.update, verbatim.
    AgentUpdate {
        node: String,
        attempt: String,
        update: Value,
        at_ms: i64,
    },
    ShellPrepared {
        node: String,
        attempt: String,
        command_line: String,
        cwd: Option<String>,
        at_ms: i64,
    },
    StepFinished {
        node: String,
        attempt: String,
        kind: StepKind,
        outcome: String,
        duration_ms: u64,
        error: Option<String>,
        output: Value,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i64>,
        at_ms: i64,
    },
    /// A checkpoint the run now waits on. `question` = the flow node summary, else the node id.
    Waiting {
        node: String,
        attempt: String,
        question: String,
        choices: Vec<GateChoice>,
        at_ms: i64,
    },
    RunFinished {
        status: String,
        error: Option<String>,
        at_ms: i64,
    },
}

#[derive(Debug, Clone)]
pub struct DriverConfig {
    pub acpx_bin: PathBuf,
    pub runs_dir: PathBuf,
    pub cwd: PathBuf,
    pub extra_path: Vec<PathBuf>,
    pub poll_ms: u64,
}

pub struct RunHandle {
    pid: Option<u32>,
    run_dir: Arc<Mutex<Option<PathBuf>>>,
    cancelled: Arc<AtomicBool>,
}

impl RunHandle {
    pub fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid
            && let Ok(ip) = i32::try_from(pid)
            && ip > 0
        {
            // SAFETY: ip is a valid positive PID created by our child process.
            unsafe {
                libc::kill(ip, libc::SIGTERM);
            }
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(5));
                // SAFETY: Checking if the process is still alive via signal 0, and if so, killing its process group.
                unsafe {
                    if libc::kill(ip, 0) == 0 {
                        libc::kill(-ip, libc::SIGKILL);
                    }
                }
            });
        }
    }

    pub fn run_dir(&self) -> Option<PathBuf> {
        self.run_dir.lock().ok().and_then(|g| g.clone())
    }
}

pub fn start(
    cfg: &DriverConfig,
    flow_file: &Path,
    input_json: &str,
    tx: tokio::sync::mpsc::UnboundedSender<DriverEvent>,
) -> anyhow::Result<RunHandle> {
    let run_dir_slot: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let cancelled = Arc::new(AtomicBool::new(false));

    let resolved_flow = if flow_file.is_relative() {
        cfg.cwd.join(flow_file)
    } else {
        flow_file.to_path_buf()
    };
    let canon_flow = fs::canonicalize(&resolved_flow).unwrap_or_else(|_| resolved_flow.clone());

    let mut existing_runs = HashSet::new();
    if let Ok(entries) = fs::read_dir(&cfg.runs_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                existing_runs.insert(name.to_string());
            }
        }
    }

    let tmp_path = make_temp_input_file(input_json)?;

    let mut cmd = tokio::process::Command::new(&cfg.acpx_bin);
    cmd.arg("--approve-all")
        .arg("--format")
        .arg("json")
        .arg("flow")
        .arg("run")
        .arg(&resolved_flow)
        .arg("--input-file")
        .arg(&tmp_path)
        .current_dir(&cfg.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    if !cfg.extra_path.is_empty() {
        let mut paths = cfg.extra_path.clone();
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            let _ = tx.send(DriverEvent::RunFinished {
                status: "failed".to_string(),
                error: Some(e.to_string()),
                at_ms: 0,
            });
            return Ok(RunHandle {
                pid: None,
                run_dir: run_dir_slot,
                cancelled,
            });
        }
    };

    let pid = child.id();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));

    if let Some(mut out) = stdout_pipe {
        let buf_clone = Arc::clone(&stdout_buf);
        tokio::task::spawn_local(async move {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = tokio::io::AsyncReadExt::read(&mut out, &mut chunk).await {
                if n == 0 {
                    break;
                }
                if let Ok(mut b) = buf_clone.lock()
                    && b.len() < 256 * 1024
                {
                    let space = (256 * 1024) - b.len();
                    if let Some(slice) = chunk.get(..n.min(space)) {
                        b.extend_from_slice(slice);
                    }
                }
            }
        });
    }

    if let Some(mut err) = stderr_pipe {
        let buf_clone = Arc::clone(&stderr_buf);
        tokio::task::spawn_local(async move {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = tokio::io::AsyncReadExt::read(&mut err, &mut chunk).await {
                if n == 0 {
                    break;
                }
                if let Ok(mut b) = buf_clone.lock() {
                    if let Some(slice) = chunk.get(..n) {
                        b.extend_from_slice(slice);
                    }
                    if b.len() > 8192 {
                        let drop_len = b.len() - 4096;
                        b.drain(..drop_len);
                    }
                }
            }
        });
    }

    let cfg_clone = cfg.clone();
    let run_dir_slot_clone = Arc::clone(&run_dir_slot);

    tokio::task::spawn_local(async move {
        let poll_dur = Duration::from_millis(cfg_clone.poll_ms.max(10));
        let start_inst = Instant::now();
        let mut discovered_dir: Option<PathBuf> = None;
        let mut child_exited_at: Option<Instant> = None;

        while discovered_dir.is_none() {
            if child_exited_at.is_none()
                && let Ok(Some(_)) = child.try_wait()
            {
                child_exited_at = Some(Instant::now());
            }

            if let Some(dir) =
                find_matching_run_dir(&cfg_clone.runs_dir, &existing_runs, &canon_flow)
            {
                discovered_dir = Some(dir);
                break;
            }

            if let Some(exited_at) = child_exited_at
                && exited_at.elapsed() >= Duration::from_millis(300)
            {
                break;
            }

            if start_inst.elapsed() >= Duration::from_secs(15) {
                break;
            }

            tokio::time::sleep(poll_dur).await;
        }

        let Some(run_dir) = discovered_dir else {
            let _ = fs::remove_file(&tmp_path);
            let err_tail = extract_stderr_tail(&stderr_buf);
            let error_msg = if err_tail.is_empty() {
                "acpx did not create a run folder".to_string()
            } else {
                err_tail
            };
            let _ = tx.send(DriverEvent::RunFinished {
                status: "failed".to_string(),
                error: Some(error_msg),
                at_ms: 0,
            });
            return;
        };

        let mut tailer = Tailer::new(run_dir.clone());

        loop {
            for ev in tailer.tick() {
                if matches!(ev, DriverEvent::RunStarted { .. })
                    && let Ok(mut g) = run_dir_slot_clone.lock()
                {
                    *g = Some(run_dir.clone());
                }
                let _ = tx.send(ev);
            }

            if child_exited_at.is_none()
                && let Ok(Some(_)) = child.try_wait()
            {
                child_exited_at = Some(Instant::now());
            }

            if let Some(exited_at) = child_exited_at
                && exited_at.elapsed() >= Duration::from_millis(300)
            {
                break;
            }

            tokio::time::sleep(poll_dur).await;
        }

        for ev in tailer.tick() {
            if matches!(ev, DriverEvent::RunStarted { .. })
                && let Ok(mut g) = run_dir_slot_clone.lock()
            {
                *g = Some(run_dir.clone());
            }
            let _ = tx.send(ev);
        }

        let stdout_fallback = parse_stdout_json(&stdout_buf);
        for ev in tailer.finish(true, stdout_fallback.as_ref()) {
            let _ = tx.send(ev);
        }

        let _ = fs::remove_file(&tmp_path);
    });

    Ok(RunHandle {
        pid,
        run_dir: run_dir_slot,
        cancelled,
    })
}

pub fn replay(run_dir: &Path) -> anyhow::Result<Vec<DriverEvent>> {
    let mut tailer = Tailer::new(run_dir.to_path_buf());
    let mut events = tailer.tick();
    events.extend(tailer.finish(false, None));
    Ok(events)
}

pub fn list_runs(runs_dir: &Path, limit: usize) -> Vec<(String, String, String, String, i64)> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let live = read_json_file(&path.join("projections").join("live.json"));

        let run_id = json_str(&manifest, "runId")
            .or_else(|| entry.file_name().to_str().map(ToString::to_string))
            .unwrap_or_default();
        if run_id.is_empty() {
            continue;
        }
        let flow_name = json_str(&manifest, "flowName").unwrap_or_default();
        let title = json_str(&manifest, "runTitle")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                live.as_ref()
                    .and_then(|l| json_str(l, "runTitle"))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| flow_name.clone());
        let status = live
            .as_ref()
            .and_then(|l| json_str(l, "status"))
            .filter(|s| !s.is_empty())
            .or_else(|| json_str(&manifest, "status"))
            .unwrap_or_else(|| "unknown".to_string());
        let started_ms = json_str(&manifest, "startedAt")
            .and_then(|s| parse_ts(&s))
            .unwrap_or(0);

        runs.push((run_id, flow_name, title, status, started_ms));
    }

    runs.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| b.0.cmp(&a.0)));
    runs.truncate(limit);
    runs
}

fn make_temp_input_file(input_json: &str) -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..100u32 {
        let name = format!("grok-acpx-input-{pid}-{nanos}-{attempt}.json");
        let path = dir.join(name);
        if !path.exists() {
            fs::write(&path, input_json.as_bytes())?;
            return Ok(path);
        }
    }
    anyhow::bail!("failed to create temporary input file")
}

fn find_matching_run_dir(
    runs_dir: &Path,
    existing: &HashSet<String>,
    canon_flow: &Path,
) -> Option<PathBuf> {
    let entries = fs::read_dir(runs_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_str()?.to_string();
        if existing.contains(&name) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let Some(fp_str) = json_str(&manifest, "flowPath") else {
            continue;
        };
        let fp = PathBuf::from(&fp_str);
        let canon_fp = fs::canonicalize(&fp).unwrap_or(fp);
        if canon_fp == canon_flow {
            return Some(path);
        }
    }
    None
}

fn extract_stderr_tail(stderr_buf: &Arc<Mutex<Vec<u8>>>) -> String {
    let Ok(guard) = stderr_buf.lock() else {
        return String::new();
    };
    let len = guard.len();
    let start = len.saturating_sub(2048);
    let slice = guard.get(start..).unwrap_or(&[]);
    String::from_utf8_lossy(slice).trim().to_string()
}

fn parse_stdout_json(stdout_buf: &Arc<Mutex<Vec<u8>>>) -> Option<Value> {
    let guard = stdout_buf.lock().ok()?;
    let text = String::from_utf8_lossy(&guard);
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        {
            return Some(val);
        }
    }
    serde_json::from_str::<Value>(text.trim()).ok()
}

fn read_json_file(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(ToString::to_string)
}

#[derive(Default)]
struct LineReader {
    offset: u64,
    buf: Vec<u8>,
}

impl LineReader {
    fn read_new_lines(&mut self, path: &Path) -> Vec<String> {
        let Ok(mut file) = fs::File::open(path) else {
            return Vec::new();
        };
        if std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut chunk = Vec::new();
        if file.read_to_end(&mut chunk).is_err() || chunk.is_empty() {
            return Vec::new();
        }
        self.offset += chunk.len() as u64;
        self.buf.extend_from_slice(&chunk);
        self.drain_complete_lines()
    }

    #[cfg(test)]
    fn feed_str(&mut self, s: &str) -> Vec<String> {
        self.buf.extend_from_slice(s.as_bytes());
        self.drain_complete_lines()
    }

    fn drain_complete_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.buf.drain(..=pos).collect();
            let mut s = String::from_utf8_lossy(&line_bytes).into_owned();
            if s.ends_with('\n') {
                s.pop();
            }
            if s.ends_with('\r') {
                s.pop();
            }
            if !s.is_empty() {
                lines.push(s);
            }
        }
        lines
    }
}

#[derive(Debug, Clone)]
enum ParsedSessionItem {
    Prompt {
        seq: u64,
        at_ms: i64,
        rpc_id: Option<Value>,
        text: String,
    },
    Update {
        seq: u64,
        at_ms: i64,
        update: Value,
    },
    Response {
        seq: u64,
        rpc_id: Value,
    },
}

#[derive(Default)]
struct PromptGrouping {
    prompt_seq: u64,
    prompt_at_ms: i64,
    rpc_id: Option<Value>,
    prompt_text: String,
    updates: VecDeque<(i64, Value)>,
    response_received: bool,
    prompt_emitted: bool,
}

#[derive(Default)]
struct SessionTracker {
    reader: LineReader,
    items: Vec<ParsedSessionItem>,
    groupings: Vec<PromptGrouping>,
    prepared_attempts: Vec<(String, String)>,
}

impl SessionTracker {
    fn ingest_lines(&mut self, lines: Vec<String>) {
        for line in lines {
            let Ok(val) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let seq = val.get("seq").and_then(Value::as_u64).unwrap_or(0);
            let at_ms = val
                .get("at")
                .and_then(Value::as_str)
                .and_then(parse_ts)
                .unwrap_or(0);
            let direction = val.get("direction").and_then(Value::as_str).unwrap_or("");
            let Some(msg) = val.get("message") else {
                continue;
            };

            if let Some(method) = msg.get("method").and_then(Value::as_str) {
                if method.starts_with("_auth") {
                    continue;
                }
                if direction == "outbound" && method == "session/prompt" {
                    let rpc_id = msg.get("id").cloned();
                    let text = extract_prompt_text(msg);
                    self.items.push(ParsedSessionItem::Prompt {
                        seq,
                        at_ms,
                        rpc_id,
                        text,
                    });
                } else if direction == "inbound" && method == "session/update" {
                    if let Some(update) = msg.get("params").and_then(|p| p.get("update")).cloned() {
                        self.items
                            .push(ParsedSessionItem::Update { seq, at_ms, update });
                    }
                }
            } else if direction == "inbound"
                && (msg.get("result").is_some() || msg.get("error").is_some())
                && let Some(id) = msg.get("id").cloned()
            {
                self.items.push(ParsedSessionItem::Response {
                    seq,
                    rpc_id: id,
                });
            }
        }
        self.rebuild_groupings();
    }

    fn rebuild_groupings(&mut self) {
        self.items.sort_by_key(|item| match item {
            ParsedSessionItem::Prompt { seq, .. }
            | ParsedSessionItem::Update { seq, .. }
            | ParsedSessionItem::Response { seq, .. } => *seq,
        });

        let old_emitted: Vec<(bool, usize)> = self
            .groupings
            .iter()
            .map(|g| (g.prompt_emitted, 0))
            .collect();

        let mut new_groupings: Vec<PromptGrouping> = Vec::new();
        for item in &self.items {
            match item {
                ParsedSessionItem::Prompt {
                    seq,
                    at_ms,
                    rpc_id,
                    text,
                } => {
                    new_groupings.push(PromptGrouping {
                        prompt_seq: *seq,
                        prompt_at_ms: *at_ms,
                        rpc_id: rpc_id.clone(),
                        prompt_text: text.clone(),
                        updates: VecDeque::new(),
                        response_received: false,
                        prompt_emitted: false,
                    });
                }
                ParsedSessionItem::Update { seq, at_ms, update } => {
                    if let Some(last) = new_groupings.last_mut()
                        && *seq > last.prompt_seq
                        && !last.response_received
                    {
                        last.updates.push_back((*at_ms, update.clone()));
                    }
                }
                ParsedSessionItem::Response { seq, rpc_id } => {
                    if let Some(last) = new_groupings.last_mut()
                        && *seq > last.prompt_seq
                        && !last.response_received
                        && last.rpc_id.as_ref() == Some(rpc_id)
                    {
                        last.response_received = true;
                    }
                }
            }
        }

        // Preserve emission state from previous groupings
        for (idx, g) in new_groupings.iter_mut().enumerate() {
            if let Some((was_emitted, _)) = old_emitted.get(idx) {
                g.prompt_emitted = *was_emitted;
            }
            if let Some(old_g) = self.groupings.get(idx) {
                let already_drained = old_g.total_seen.saturating_sub(old_g.updates.len());
                for _ in 0..already_drained {
                    g.updates.pop_front();
                }
                g.total_seen = already_drained + g.updates.len();
            } else {
                g.total_seen = g.updates.len();
            }
        }

        self.groupings = new_groupings;
    }
}

// Add `total_seen` field to PromptGrouping to track how many updates were already drained
#[derive(Default)]
struct ActionCompletedInfo {
    exit_code: Option<i64>,
    stdout_path: Option<String>,
    stderr_path: Option<String>,
}

// Redefine SessionTracker with explicit cursor tracking so rebuild is not needed
struct SessionState {
    reader: LineReader,
    prompts: Vec<PromptSlot>,
    prepared_attempts: Vec<(String, String)>,
}

struct PromptSlot {
    prompt_seq: u64,
    prompt_at_ms: i64,
    rpc_id: Option<Value>,
    prompt_text: String,
    prompt_emitted: bool,
    updates: VecDeque<(i64, Value)>,
    response_received: bool,
}

impl SessionState {
    fn new() -> Self {
        Self {
            reader: LineReader::default(),
            prompts: Vec::new(),
            prepared_attempts: Vec::new(),
        }
    }

    fn ingest_lines(&mut self, lines: Vec<String>) {
        for line in lines {
            let Ok(val) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let seq = val.get("seq").and_then(Value::as_u64).unwrap_or(0);
            let at_ms = val
                .get("at")
                .and_then(Value::as_str)
                .and_then(parse_ts)
                .unwrap_or(0);
            let direction = val.get("direction").and_then(Value::as_str).unwrap_or("");
            let Some(msg) = val.get("message") else {
                continue;
            };

            if let Some(method) = msg.get("method").and_then(Value::as_str) {
                if method.starts_with("_auth") {
                    continue;
                }
                if direction == "outbound" && method == "session/prompt" {
                    let rpc_id = msg.get("id").cloned();
                    let text = extract_prompt_text(msg);
                    self.prompts.push(PromptSlot {
                        prompt_seq: seq,
                        prompt_at_ms: at_ms,
                        rpc_id,
                        prompt_text: text,
                        prompt_emitted: false,
                        updates: VecDeque::new(),
                        response_received: false,
                    });
                } else if direction == "inbound" && method == "session/update" {
                    if let Some(update) = msg.get("params").and_then(|p| p.get("update")).cloned()
                        && let Some(slot) = self.prompts.last_mut()
                        && seq >= slot.prompt_seq
                        && !slot.response_received
                    {
                        slot.updates.push_back((at_ms, update));
                    }
                }
            } else if direction == "inbound"
                && (msg.get("result").is_some() || msg.get("error").is_some())
                && let Some(id) = msg.get("id")
                && let Some(slot) = self.prompts.last_mut()
                && seq > slot.prompt_seq
                && !slot.response_received
                && slot.rpc_id.as_ref() == Some(id)
            {
                slot.response_received = true;
            }
        }
    }
}

fn extract_prompt_text(msg: &Value) -> String {
    let Some(params) = msg.get("params") else {
        return String::new();
    };
    if let Some(arr) = params
        .get("prompt")
        .or_else(|| params.get("content"))
        .and_then(Value::as_array)
    {
        let mut parts = Vec::new();
        for block in arr {
            if let Some(s) = block.as_str() {
                parts.push(s.to_string());
            } else if let Some(t) = block.get("text").and_then(Value::as_str) {
                parts.push(t.to_string());
            } else if let Some(t) = block
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(Value::as_str)
            {
                parts.push(t.to_string());
            }
        }
        return parts.join("\n");
    }
    if let Some(s) = params
        .get("prompt")
        .or_else(|| params.get("text"))
        .and_then(Value::as_str)
    {
        return s.to_string();
    }
    String::new()
}

struct Tailer {
    run_dir: PathBuf,
    trace_reader: LineReader,
    sessions: HashMap<String, SessionState>,
    run_started_emitted: bool,
    flow_sketch: Option<FlowSketch>,
    node_kinds: HashMap<String, StepKind>,
    node_summaries: HashMap<String, String>,
    started_attempts: HashSet<(String, String)>,
    finished_attempts: HashSet<(String, String)>,
    waiting_emitted_attempts: HashSet<(String, String)>,
    last_attempt_for_node: HashMap<String, String>,
    action_completed: HashMap<(String, String), ActionCompletedInfo>,
    trace_terminal_status: Option<(String, Option<String>, i64)>,
    last_at_ms: i64,
}

impl Tailer {
    fn new(run_dir: PathBuf) -> Self {
        Self {
            run_dir,
            trace_reader: LineReader::default(),
            sessions: HashMap::new(),
            run_started_emitted: false,
            flow_sketch: None,
            node_kinds: HashMap::new(),
            node_summaries: HashMap::new(),
            started_attempts: HashSet::new(),
            finished_attempts: HashSet::new(),
            waiting_emitted_attempts: HashSet::new(),
            last_attempt_for_node: HashMap::new(),
            action_completed: HashMap::new(),
            trace_terminal_status: None,
            last_at_ms: 0,
        }
    }

    fn tick(&mut self) -> Vec<DriverEvent> {
        self.read_all_session_files();
        let trace_path = self.run_dir.join("trace.ndjson");
        let trace_lines = self.trace_reader.read_new_lines(&trace_path);
        self.process_trace_lines(trace_lines)
    }

    #[cfg(test)]
    fn feed_manual(
        &mut self,
        session_chunks: &[(&str, &str)],
        trace_chunk: &str,
    ) -> Vec<DriverEvent> {
        for (sid, chunk) in session_chunks {
            let state = self
                .sessions
                .entry((*sid).to_string())
                .or_insert_with(SessionState::new);
            let lines = state.reader.feed_str(chunk);
            state.ingest_lines(lines);
        }
        let trace_lines = self.trace_reader.feed_str(trace_chunk);
        self.process_trace_lines(trace_lines)
    }

    fn read_all_session_files(&mut self) {
        let sessions_dir = self.run_dir.join("sessions");
        if let Ok(entries) = fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && let Some(sid) = entry.file_name().to_str()
                {
                    let state = self
                        .sessions
                        .entry(sid.to_string())
                        .or_insert_with(SessionState::new);
                    let events_path = path.join("events.ndjson");
                    let lines = state.reader.read_new_lines(&events_path);
                    if !lines.is_empty() {
                        state.ingest_lines(lines);
                    }
                }
            }
        }
    }

    fn flush_active_sessions(&mut self, out: &mut Vec<DriverEvent>) {
        let sids: Vec<String> = self.sessions.keys().cloned().collect();
        for sid in sids {
            self.flush_session(&sid, None, out);
        }
    }

    fn flush_session(
        &mut self,
        sid: &str,
        only_attempt: Option<&(String, String)>,
        out: &mut Vec<DriverEvent>,
    ) {
        let Some(state) = self.sessions.get_mut(sid) else {
            return;
        };
        let count = state.prepared_attempts.len().min(state.prompts.len());
        for idx in 0..count {
            let Some(attempt_key) = state.prepared_attempts.get(idx).cloned() else {
                continue;
            };
            if let Some(filter) = only_attempt
                && &attempt_key != filter
            {
                continue;
            }
            if !self.started_attempts.contains(&attempt_key) {
                continue;
            }
            let is_finished = self.finished_attempts.contains(&attempt_key);
            let Some(slot) = state.prompts.get_mut(idx) else {
                continue;
            };
            if is_finished {
                slot.prompt_emitted = true;
                slot.updates.clear();
                continue;
            }
            if !slot.prompt_emitted {
                slot.prompt_emitted = true;
                if slot.prompt_at_ms > self.last_at_ms {
                    self.last_at_ms = slot.prompt_at_ms;
                }
                out.push(DriverEvent::AgentPrompt {
                    node: attempt_key.0.clone(),
                    attempt: attempt_key.1.clone(),
                    text: slot.prompt_text.clone(),
                    at_ms: slot.prompt_at_ms,
                });
            }
            while let Some((u_at, u_val)) = slot.updates.pop_front() {
                if u_at > self.last_at_ms {
                    self.last_at_ms = u_at;
                }
                out.push(DriverEvent::AgentUpdate {
                    node: attempt_key.0.clone(),
                    attempt: attempt_key.1.clone(),
                    update: u_val,
                    at_ms: u_at,
                });
            }
        }
    }

    fn flush_attempt(&mut self, attempt_key: &(String, String), out: &mut Vec<DriverEvent>) {
        let sids: Vec<String> = self.sessions.keys().cloned().collect();
        for sid in sids {
            self.flush_session(&sid, Some(attempt_key), out);
        }
    }

    fn ensure_run_started(&mut self, trace_at_ms: Option<i64>, out: &mut Vec<DriverEvent>) -> bool {
        if self.run_started_emitted {
            return true;
        }
        let Some(manifest) = read_json_file(&self.run_dir.join("manifest.json")) else {
            return false;
        };
        let Some(flow_val) = read_json_file(&self.run_dir.join("flow.json")) else {
            return false;
        };
        let live = read_json_file(&self.run_dir.join("projections").join("live.json"));

        let run_id = json_str(&manifest, "runId")
            .or_else(|| {
                self.run_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_default();
        let flow_name = json_str(&manifest, "flowName")
            .or_else(|| json_str(&flow_val, "name"))
            .unwrap_or_default();
        let title = json_str(&manifest, "runTitle")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                live.as_ref()
                    .and_then(|l| json_str(l, "runTitle"))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| flow_name.clone());
        let flow_path = json_str(&manifest, "flowPath").unwrap_or_default();
        let at_ms = trace_at_ms
            .or_else(|| json_str(&manifest, "startedAt").and_then(|s| parse_ts(&s)))
            .unwrap_or(0);
        if at_ms > self.last_at_ms {
            self.last_at_ms = at_ms;
        }

        let (sketch, kinds, summaries) = build_flow_sketch(&flow_val, &flow_name);
        self.node_kinds = kinds;
        self.node_summaries = summaries;
        self.flow_sketch = Some(sketch.clone());
        self.run_started_emitted = true;

        out.push(DriverEvent::RunStarted {
            run_id,
            run_dir: self.run_dir.clone(),
            title,
            flow_path,
            flow: sketch,
            at_ms,
        });
        true
    }

    fn process_trace_lines(&mut self, lines: Vec<String>) -> Vec<DriverEvent> {
        let mut out = Vec::new();

        // First flush any newly arrived session updates for already-prepared active attempts
        self.flush_active_sessions(&mut out);

        let mut parsed_lines: Vec<(u64, Value)> = Vec::new();
        for line in lines {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let seq = v.get("seq").and_then(Value::as_u64).unwrap_or(0);
                parsed_lines.push((seq, v));
            }
        }
        parsed_lines.sort_by_key(|(seq, _)| *seq);

        for (_, v) in parsed_lines {
            let ev_type = v.get("type").and_then(Value::as_str).unwrap_or("");
            let at_ms = v
                .get("at")
                .and_then(Value::as_str)
                .and_then(parse_ts)
                .unwrap_or(0);
            if at_ms > self.last_at_ms {
                self.last_at_ms = at_ms;
            }
            let node_id = v
                .get("nodeId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let attempt_id = v
                .get("attemptId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let session_id = v
                .get("sessionId")
                .and_then(Value::as_str)
                .or_else(|| {
                    v.get("payload")
                        .and_then(|p| p.get("sessionId"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("")
                .to_string();
            let payload = v.get("payload").cloned().unwrap_or(Value::Null);

            match ev_type {
                "run_started" => {
                    self.ensure_run_started(Some(at_ms), &mut out);
                }
                "node_started" => {
                    self.ensure_run_started(None, &mut out);
                    let kind = self.resolve_step_kind(&node_id, &payload);
                    let key = (node_id.clone(), attempt_id.clone());
                    self.started_attempts.insert(key);
                    self.last_attempt_for_node
                        .insert(node_id.clone(), attempt_id.clone());
                    out.push(DriverEvent::StepStarted {
                        node: node_id,
                        attempt: attempt_id,
                        kind,
                        at_ms,
                    });
                    self.flush_active_sessions(&mut out);
                }
                "session_bound" => {
                    if !session_id.is_empty() {
                        self.sessions
                            .entry(session_id)
                            .or_insert_with(SessionState::new);
                    }
                }
                "acp_prompt_prepared" => {
                    self.ensure_run_started(None, &mut out);
                    if !session_id.is_empty() && !node_id.is_empty() {
                        let key = (node_id.clone(), attempt_id.clone());
                        let state = self
                            .sessions
                            .entry(session_id.clone())
                            .or_insert_with(SessionState::new);
                        state.prepared_attempts.push(key.clone());
                        self.flush_session(&session_id, Some(&key), &mut out);
                    }
                }
                "action_prepared" => {
                    self.ensure_run_started(None, &mut out);
                    if let Some(action) = payload.get("action")
                        && action.get("actionType").and_then(Value::as_str) == Some("shell")
                    {
                        let cmd = action.get("command").and_then(Value::as_str).unwrap_or("");
                        let args: Vec<&str> = action
                            .get("args")
                            .and_then(Value::as_array)
                            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        let command_line = format_shell_command(cmd, &args);
                        let cwd = action
                            .get("cwd")
                            .and_then(Value::as_str)
                            .map(ToString::to_string);
                        out.push(DriverEvent::ShellPrepared {
                            node: node_id,
                            attempt: attempt_id,
                            command_line,
                            cwd,
                            at_ms,
                        });
                    }
                }
                "action_completed" => {
                    let exit_code = payload
                        .get("action")
                        .and_then(|a| a.get("exitCode"))
                        .and_then(Value::as_i64);
                    let stdout_path = payload
                        .get("stdoutArtifact")
                        .and_then(|a| a.get("path"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let stderr_path = payload
                        .get("stderrArtifact")
                        .and_then(|a| a.get("path"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    self.action_completed.insert(
                        (node_id, attempt_id),
                        ActionCompletedInfo {
                            exit_code,
                            stdout_path,
                            stderr_path,
                        },
                    );
                }
                "node_outcome" => {
                    self.ensure_run_started(None, &mut out);
                    let key = (node_id.clone(), attempt_id.clone());
                    self.last_attempt_for_node
                        .insert(node_id.clone(), attempt_id.clone());
                    let kind = self.resolve_step_kind(&node_id, &payload);
                    if kind == StepKind::Acp {
                        self.flush_attempt(&key, &mut out);
                    }
                    self.finished_attempts.insert(key.clone());

                    let outcome = json_str(&payload, "outcome").unwrap_or_default();
                    let duration_ms = payload
                        .get("durationMs")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let error = extract_error_string(payload.get("error"));
                    let output = payload.get("outputInline").cloned().unwrap_or(Value::Null);

                    let (stdout, stderr, exit_code) = if kind == StepKind::Shell {
                        let ac = self.action_completed.get(&key);
                        let stdout_str = ac
                            .and_then(|i| i.stdout_path.as_deref())
                            .or_else(|| {
                                payload
                                    .get("stdoutArtifact")
                                    .and_then(|a| a.get("path"))
                                    .and_then(Value::as_str)
                            })
                            .and_then(|rel| read_artifact_bounded(&self.run_dir, rel, 64 * 1024));
                        let stderr_str = ac
                            .and_then(|i| i.stderr_path.as_deref())
                            .or_else(|| {
                                payload
                                    .get("stderrArtifact")
                                    .and_then(|a| a.get("path"))
                                    .and_then(Value::as_str)
                            })
                            .and_then(|rel| read_artifact_bounded(&self.run_dir, rel, 16 * 1024));
                        let code = ac.and_then(|i| i.exit_code).or_else(|| {
                            payload
                                .get("action")
                                .and_then(|a| a.get("exitCode"))
                                .and_then(Value::as_i64)
                        });
                        (stdout_str, stderr_str, code)
                    } else {
                        (None, None, None)
                    };

                    out.push(DriverEvent::StepFinished {
                        node: node_id.clone(),
                        attempt: attempt_id.clone(),
                        kind,
                        outcome,
                        duration_ms,
                        error,
                        output: output.clone(),
                        stdout,
                        stderr,
                        exit_code,
                        at_ms,
                    });

                    if kind == StepKind::Checkpoint
                        && let Some(next_obj) = output.get("next").and_then(Value::as_object)
                        && self.waiting_emitted_attempts.insert(key)
                    {
                        let mut choices: Vec<GateChoice> = next_obj
                            .iter()
                            .filter_map(|(k, v)| {
                                v.as_str().map(|ff| GateChoice {
                                    label: k.clone(),
                                    flow_file: ff.to_string(),
                                })
                            })
                            .collect();
                        choices.sort_by(|a, b| a.label.cmp(&b.label));
                        let question = self
                            .node_summaries
                            .get(&node_id)
                            .cloned()
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| node_id.clone());
                        out.push(DriverEvent::Waiting {
                            node: node_id,
                            attempt: attempt_id,
                            question,
                            choices,
                            at_ms,
                        });
                    }
                }
                "run_completed" => {
                    let st =
                        json_str(&payload, "status").unwrap_or_else(|| "completed".to_string());
                    let err = extract_error_string(payload.get("error"));
                    self.trace_terminal_status = Some((st, err, at_ms));
                }
                "run_failed" => {
                    let st = json_str(&payload, "status").unwrap_or_else(|| "failed".to_string());
                    let err = extract_error_string(payload.get("error"));
                    self.trace_terminal_status = Some((st, err, at_ms));
                }
                _ => {}
            }
        }

        out
    }

    fn finish(
        &mut self,
        child_exited: bool,
        stdout_fallback: Option<&Value>,
    ) -> Vec<DriverEvent> {
        let mut out = Vec::new();
        self.ensure_run_started(None, &mut out);
        self.flush_active_sessions(&mut out);

        let live = read_json_file(&self.run_dir.join("projections").join("live.json"));
        let live_status = live.as_ref().and_then(|l| json_str(l, "status"));
        let live_error = live
            .as_ref()
            .and_then(|l| extract_error_string(l.get("error")));
        let live_at_ms = live
            .as_ref()
            .and_then(|l| {
                json_str(l, "finishedAt")
                    .or_else(|| json_str(l, "updatedAt"))
                    .and_then(|s| parse_ts(&s))
            })
            .unwrap_or(self.last_at_ms);

        if live_status.as_deref() == Some("waiting") {
            let waiting_node = live
                .as_ref()
                .and_then(|l| json_str(l, "waitingOn"))
                .or_else(|| live.as_ref().and_then(|l| json_str(l, "currentNode")))
                .unwrap_or_default();
            if !waiting_node.is_empty() {
                let attempt = self
                    .last_attempt_for_node
                    .get(&waiting_node)
                    .cloned()
                    .unwrap_or_default();
                let key = (waiting_node.clone(), attempt.clone());
                if self.waiting_emitted_attempts.insert(key) {
                    let question = self
                        .node_summaries
                        .get(&waiting_node)
                        .cloned()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| waiting_node.clone());
                    out.push(DriverEvent::Waiting {
                        node: waiting_node,
                        attempt,
                        question,
                        choices: Vec::new(),
                        at_ms: live_at_ms,
                    });
                }
            }
        }

        let (status, error, at_ms) = if let Some((st, err, at)) = self.trace_terminal_status.clone()
        {
            (st, err.or(live_error), at)
        } else if live_status.as_deref() == Some("waiting") {
            ("waiting".to_string(), live_error, live_at_ms)
        } else if let Some(ref lst) = live_status
            && lst != "running"
        {
            (lst.clone(), live_error, live_at_ms)
        } else if !child_exited && live_status.as_deref() == Some("running") {
            ("running".to_string(), live_error, live_at_ms)
        } else if let Some(fb) = stdout_fallback
            && let Some(fb_status) = json_str(fb, "status").filter(|s| s != "running")
        {
            let fb_err = extract_error_string(fb.get("error")).or(live_error);
            (fb_status, fb_err, live_at_ms)
        } else {
            (
                "failed".to_string(),
                Some("acpx exited without finishing".to_string()),
                live_at_ms,
            )
        };

        out.push(DriverEvent::RunFinished {
            status,
            error,
            at_ms,
        });
        out
    }

    fn resolve_step_kind(&self, node_id: &str, payload: &Value) -> StepKind {
        let node_type = payload.get("nodeType").and_then(Value::as_str);
        match node_type {
            Some("acp") => StepKind::Acp,
            Some("compute") => StepKind::Compute,
            Some("checkpoint") => StepKind::Checkpoint,
            Some("action") => {
                if let Some(at) = payload
                    .get("action")
                    .and_then(|a| a.get("actionType"))
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("actionExecution").and_then(Value::as_str))
                {
                    match at {
                        "shell" => return StepKind::Shell,
                        "function" => return StepKind::Compute,
                        _ => return StepKind::Other,
                    }
                }
                if let Some(&k) = self.node_kinds.get(node_id) {
                    return k;
                }
                StepKind::Shell
            }
            Some(_) => StepKind::Other,
            None => self
                .node_kinds
                .get(node_id)
                .copied()
                .unwrap_or(StepKind::Other),
        }
    }
}

fn extract_error_string(err_val: Option<&Value>) -> Option<String> {
    let v = err_val?;
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        if s.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    if let Some(msg) = v.get("message").and_then(Value::as_str) {
        return Some(msg.to_string());
    }
    Some(v.to_string())
}

fn format_shell_command(command: &str, args: &[&str]) -> String {
    let mut parts = Vec::with_capacity(1 + args.len());
    if !command.is_empty() {
        parts.push(shell_quote_if_needed(command));
    }
    for &arg in args {
        parts.push(shell_quote_if_needed(arg));
    }
    parts.join(" ")
}

fn shell_quote_if_needed(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .any(|c| c.is_whitespace() || c == '\'' || c == '"')
    {
        let escaped = s.replace('\'', "'\\''");
        format!("'{escaped}'")
    } else {
        s.to_string()
    }
}

fn read_artifact_bounded(run_dir: &Path, rel_path: &str, max_bytes: usize) -> Option<String> {
    if rel_path.is_empty() {
        return None;
    }
    let rel = Path::new(rel_path);
    if rel.is_absolute() {
        return None;
    }
    for comp in rel.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let full = run_dir.join(rel);
    let mut file = fs::File::open(full).ok()?;
    let mut buf = vec![0u8; max_bytes];
    let mut total = 0;
    while total < max_bytes {
        let Some(slice) = buf.get_mut(total..) else {
            break;
        };
        match file.read(slice) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => return None,
        }
    }
    buf.truncate(total);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn classify_flow_node_kind(node_val: &Value) -> StepKind {
    match node_val.get("nodeType").and_then(Value::as_str) {
        Some("acp") => StepKind::Acp,
        Some("compute") => StepKind::Compute,
        Some("checkpoint") => StepKind::Checkpoint,
        Some("action") => match node_val.get("actionExecution").and_then(Value::as_str) {
            Some("shell") => StepKind::Shell,
            Some("function") => StepKind::Compute,
            _ => StepKind::Other,
        },
        _ => StepKind::Other,
    }
}

fn build_flow_sketch(
    flow_val: &Value,
    fallback_name: &str,
) -> (
    FlowSketch,
    HashMap<String, StepKind>,
    HashMap<String, String>,
) {
    let name = json_str(flow_val, "name").unwrap_or_else(|| fallback_name.to_string());
    let start_at = json_str(flow_val, "startAt").unwrap_or_default();

    let mut nodes_map: BTreeMap<String, SketchNode> = BTreeMap::new();
    let mut kinds_map: HashMap<String, StepKind> = HashMap::new();
    let mut summaries_map: HashMap<String, String> = HashMap::new();

    if let Some(obj) = flow_val.get("nodes").and_then(Value::as_object) {
        for (id, nval) in obj {
            let kind = classify_flow_node_kind(nval);
            let summary = json_str(nval, "summary");
            let profile = json_str(nval, "profile");
            let model = json_str(nval, "modelId");
            let effort = nval
                .get("configOptions")
                .and_then(Value::as_array)
                .and_then(|arr| {
                    arr.iter().find_map(|entry| {
                        if entry.get("configId").and_then(Value::as_str) == Some("reasoning_effort")
                        {
                            json_str(entry, "value")
                        } else {
                            None
                        }
                    })
                });
            if let Some(ref s) = summary {
                summaries_map.insert(id.clone(), s.clone());
            }
            kinds_map.insert(id.clone(), kind);
            nodes_map.insert(
                id.clone(),
                SketchNode {
                    id: id.clone(),
                    kind,
                    summary,
                    profile,
                    model,
                    effort,
                },
            );
        }
    }

    let mut edges: Vec<SketchEdge> = Vec::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    if let Some(edge_arr) = flow_val.get("edges").and_then(Value::as_array) {
        for eval in edge_arr {
            let Some(from) = json_str(eval, "from") else {
                continue;
            };
            if let Some(to) = json_str(eval, "to") {
                adj.entry(from.clone()).or_default().push(to.clone());
                edges.push(SketchEdge {
                    from,
                    to,
                    label: None,
                });
            } else if let Some(cases) = eval
                .get("switch")
                .and_then(|s| s.get("cases"))
                .and_then(Value::as_object)
            {
                for (label, target_val) in cases {
                    if let Some(to) = target_val.as_str() {
                        adj.entry(from.clone()).or_default().push(to.to_string());
                        edges.push(SketchEdge {
                            from: from.clone(),
                            to: to.to_string(),
                            label: Some(label.clone()),
                        });
                    }
                }
            }
        }
    }

    let mut ordered_nodes = Vec::with_capacity(nodes_map.len());
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    if !start_at.is_empty() && nodes_map.contains_key(&start_at) {
        visited.insert(start_at.clone());
        queue.push_back(start_at.clone());
    }

    while let Some(curr) = queue.pop_front() {
        if let Some(node) = nodes_map.get(&curr) {
            ordered_nodes.push(node.clone());
        }
        if let Some(neighbors) = adj.get(&curr) {
            for nb in neighbors {
                if nodes_map.contains_key(nb) && visited.insert(nb.clone()) {
                    queue.push_back(nb.clone());
                }
            }
        }
    }

    for (id, node) in &nodes_map {
        if !visited.contains(id) {
            ordered_nodes.push(node.clone());
        }
    }

    (
        FlowSketch {
            name,
            start_at,
            nodes: ordered_nodes,
            edges,
        },
        kinds_map,
        summaries_map,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(p, content.as_bytes());
    }

    fn base_manifest(run_id: &str, flow_name: &str, title: Option<&str>) -> String {
        let mut obj = json!({
            "runId": run_id,
            "flowName": flow_name,
            "flowPath": "/tmp/flow.json",
            "startedAt": "2026-11-20T01:14:34.000Z",
            "status": "completed"
        });
        if let Some(t) = title
            && let Some(map) = obj.as_object_mut()
        {
            map.insert("runTitle".to_string(), json!(t));
        }
        obj.to_string()
    }

    #[test]
    fn test_replay_acp_step_and_auth_filter() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(
            root,
            "manifest.json",
            &base_manifest("run-1", "test-flow", Some("My Run")),
        );
        write_file(
            root,
            "flow.json",
            &json!({
                "name": "test-flow",
                "startAt": "step1",
                "nodes": {
                    "step1": {
                        "nodeType": "acp",
                        "profile": "grok-build",
                        "modelId": "grok-4",
                        "configOptions": [{"configId": "reasoning_effort", "value": "high"}]
                    }
                },
                "edges": []
            })
            .to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({
                "status": "completed",
                "updatedAt": "2026-11-20T01:14:36.000Z"
            })
            .to_string(),
        );

        let session_lines = [
            json!({
                "seq": 1,
                "at": "2026-11-20T01:14:34.100Z",
                "direction": "outbound",
                "message": {"jsonrpc": "2.0", "id": 1, "method": "_auth/login", "params": {"email": "secret@x.ai"}}
            }),
            json!({
                "seq": 2,
                "at": "2026-11-20T01:14:34.200Z",
                "direction": "outbound",
                "message": {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "session/prompt",
                    "params": {"prompt": [{"type": "text", "text": "Hello"}, {"type": "text", "text": "World"}]}
                }
            }),
            json!({
                "seq": 3,
                "at": "2026-11-20T01:14:34.300Z",
                "direction": "inbound",
                "message": {
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {"update": {"sessionUpdate": "agent_message_chunk", "text": "chunk1"}}
                }
            }),
            json!({
                "seq": 4,
                "at": "2026-11-20T01:14:34.400Z",
                "direction": "inbound",
                "message": {
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {"update": {"sessionUpdate": "agent_message_chunk", "text": "chunk2"}}
                }
            }),
            json!({
                "seq": 5,
                "at": "2026-11-20T01:14:34.500Z",
                "direction": "inbound",
                "message": {"jsonrpc": "2.0", "id": 2, "result": {"stopReason": "end_turn"}}
            }),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "sessions/s1/events.ndjson", &session_lines);

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.050Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.150Z", "type": "node_started", "nodeId": "step1", "attemptId": "a1", "payload": {"nodeType": "acp"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.180Z", "type": "acp_prompt_prepared", "nodeId": "step1", "attemptId": "a1", "sessionId": "s1", "payload": {"sessionId": "s1"}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.600Z", "type": "node_outcome", "nodeId": "step1", "attemptId": "a1", "payload": {"nodeType": "acp", "outcome": "ok", "durationMs": 450, "outputInline": {"done": true}}}),
            json!({"seq": 5, "at": "2026-11-20T01:14:34.700Z", "type": "run_completed", "payload": {"status": "completed"}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        assert_eq!(events.len(), 7);
        assert!(matches!(&events[0], DriverEvent::RunStarted { title, flow, .. }
            if title == "My Run"
            && flow.nodes.len() == 1
            && flow.nodes[0].profile.as_deref() == Some("grok-build")
            && flow.nodes[0].model.as_deref() == Some("grok-4")
            && flow.nodes[0].effort.as_deref() == Some("high")
        ));
        assert!(
            matches!(&events[1], DriverEvent::StepStarted { node, attempt, kind: StepKind::Acp, .. } if node == "step1" && attempt == "a1")
        );
        assert!(
            matches!(&events[2], DriverEvent::AgentPrompt { text, .. } if text == "Hello\nWorld")
        );
        assert!(
            matches!(&events[3], DriverEvent::AgentUpdate { update, .. } if update == &json!({"sessionUpdate": "agent_message_chunk", "text": "chunk1"}))
        );
        assert!(
            matches!(&events[4], DriverEvent::AgentUpdate { update, .. } if update == &json!({"sessionUpdate": "agent_message_chunk", "text": "chunk2"}))
        );
        assert!(
            matches!(&events[5], DriverEvent::StepFinished { outcome, duration_ms: 450, output, .. } if outcome == "ok" && output == &json!({"done": true}))
        );
        assert!(
            matches!(&events[6], DriverEvent::RunFinished { status, error: None, .. } if status == "completed")
        );
    }

    #[test]
    fn test_session_shared_by_two_steps_and_repeated_rpc_ids() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(
            root,
            "manifest.json",
            &base_manifest("run-2", "two-step", None),
        );
        write_file(
            root,
            "flow.json",
            &json!({
                "name": "two-step",
                "startAt": "n1",
                "nodes": {
                    "n1": {"nodeType": "acp"},
                    "n2": {"nodeType": "acp"}
                },
                "edges": [{"from": "n1", "to": "n2"}]
            })
            .to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "completed"}).to_string(),
        );

        let session_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.100Z", "direction": "outbound", "message": {"id": 1, "method": "session/prompt", "params": {"prompt": [{"text": "P1"}]}}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.200Z", "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"u": 1}}}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.300Z", "direction": "inbound", "message": {"id": 1, "result": {}}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.350Z", "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"u": "ignored_between_prompts"}}}}),
            json!({"seq": 5, "at": "2026-11-20T01:14:34.400Z", "direction": "outbound", "message": {"id": 1, "method": "session/prompt", "params": {"prompt": [{"text": "P2"}]}}}),
            json!({"seq": 6, "at": "2026-11-20T01:14:34.500Z", "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"u": 2}}}}),
            json!({"seq": 7, "at": "2026-11-20T01:14:34.600Z", "direction": "inbound", "message": {"id": 1, "result": {}}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "sessions/s1/events.ndjson", &session_lines);

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.050Z", "type": "node_started", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "acp"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.080Z", "type": "acp_prompt_prepared", "nodeId": "n1", "attemptId": "a1", "sessionId": "s1", "payload": {}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.320Z", "type": "node_outcome", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "acp", "outcome": "ok", "durationMs": 100}}),
            json!({"seq": 5, "at": "2026-11-20T01:14:34.360Z", "type": "node_started", "nodeId": "n2", "attemptId": "a2", "payload": {"nodeType": "acp"}}),
            json!({"seq": 6, "at": "2026-11-20T01:14:34.380Z", "type": "acp_prompt_prepared", "nodeId": "n2", "attemptId": "a2", "sessionId": "s1", "payload": {}}),
            json!({"seq": 7, "at": "2026-11-20T01:14:34.650Z", "type": "node_outcome", "nodeId": "n2", "attemptId": "a2", "payload": {"nodeType": "acp", "outcome": "ok", "durationMs": 100}}),
            json!({"seq": 8, "at": "2026-11-20T01:14:34.700Z", "type": "run_completed", "payload": {"status": "completed"}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        let prompts: Vec<(&str, &str)> = events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::AgentPrompt { node, text, .. } => Some((node.as_str(), text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(prompts, vec![("n1", "P1"), ("n2", "P2")]);

        let updates: Vec<(&str, &Value)> = events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::AgentUpdate { node, update, .. } => Some((node.as_str(), update)),
                _ => None,
            })
            .collect();
        assert_eq!(
            updates,
            vec![("n1", &json!({"u": 1})), ("n2", &json!({"u": 2}))]
        );
    }

    #[test]
    fn test_session_lines_before_trace_two_halves_equals_replay() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(root, "manifest.json", &base_manifest("run-3", "half", None));
        write_file(
            root,
            "flow.json",
            &json!({"name": "half", "startAt": "n1", "nodes": {"n1": {"nodeType": "acp"}}, "edges": []}).to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "completed"}).to_string(),
        );

        let s_all = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.100Z", "direction": "outbound", "message": {"id": 1, "method": "session/prompt", "params": {"prompt": [{"text": "Hi"}]}}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.200Z", "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"k": "v"}}}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.300Z", "direction": "inbound", "message": {"id": 1, "result": {}}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";

        let t_half1 = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";

        let t_half2 = [
            json!({"seq": 2, "at": "2026-11-20T01:14:34.050Z", "type": "node_started", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "acp"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.080Z", "type": "acp_prompt_prepared", "nodeId": "n1", "attemptId": "a1", "sessionId": "s1", "payload": {}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.400Z", "type": "node_outcome", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "acp", "outcome": "ok", "durationMs": 50}}),
            json!({"seq": 5, "at": "2026-11-20T01:14:34.500Z", "type": "run_completed", "payload": {"status": "completed"}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";

        let mut incremental_tailer = Tailer::new(root.to_path_buf());
        let mut inc_events = Vec::new();
        inc_events.extend(incremental_tailer.feed_manual(&[("s1", &s_all)], &t_half1));
        inc_events.extend(incremental_tailer.feed_manual(&[], &t_half2));
        inc_events.extend(incremental_tailer.finish(false, None));

        write_file(root, "sessions/s1/events.ndjson", &s_all);
        write_file(root, "trace.ndjson", &(t_half1 + &t_half2));
        let replay_events = replay(root).unwrap_or_default();

        assert_eq!(inc_events, replay_events);
    }

    #[test]
    fn test_shell_step_with_artifacts_and_exit_code() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(root, "manifest.json", &base_manifest("run-4", "sh", None));
        write_file(
            root,
            "flow.json",
            &json!({
                "name": "sh",
                "startAt": "sh1",
                "nodes": {"sh1": {"nodeType": "action", "actionExecution": "shell"}},
                "edges": []
            })
            .to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "completed"}).to_string(),
        );
        write_file(root, "artifacts/out.txt", "hello stdout\n");
        write_file(root, "artifacts/err.txt", "hello stderr\n");

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.100Z", "type": "node_started", "nodeId": "sh1", "attemptId": "a1", "payload": {"nodeType": "action"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.150Z", "type": "action_prepared", "nodeId": "sh1", "attemptId": "a1", "payload": {"action": {"actionType": "shell", "command": "echo", "args": ["hello world", "plain"], "cwd": "/tmp"}}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.200Z", "type": "action_completed", "nodeId": "sh1", "attemptId": "a1", "payload": {"action": {"exitCode": 0, "durationMs": 50}, "stdoutArtifact": {"path": "artifacts/out.txt"}, "stderrArtifact": {"path": "artifacts/err.txt"}}}),
            json!({"seq": 5, "at": "2026-11-20T01:14:34.250Z", "type": "node_outcome", "nodeId": "sh1", "attemptId": "a1", "payload": {"nodeType": "action", "outcome": "ok", "durationMs": 50}}),
            json!({"seq": 6, "at": "2026-11-20T01:14:34.300Z", "type": "run_completed", "payload": {"status": "completed"}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        assert!(
            matches!(&events[2], DriverEvent::ShellPrepared { command_line, cwd, .. }
            if command_line == "echo 'hello world' plain" && cwd.as_deref() == Some("/tmp"))
        );
        assert!(matches!(
            &events[3],
            DriverEvent::StepFinished {
                kind: StepKind::Shell,
                stdout: Some(out),
                stderr: Some(err),
                exit_code: Some(0),
                ..
            } if out == "hello stdout\n" && err == "hello stderr\n"
        ));
    }

    #[test]
    fn test_artifact_path_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path().join("run");
        let _ = fs::create_dir_all(&root);
        write_file(tmp.path(), "secret.txt", "top secret");

        write_file(&root, "manifest.json", &base_manifest("run-5", "sh", None));
        write_file(
            &root,
            "flow.json",
            &json!({"name": "sh", "startAt": "sh1", "nodes": {"sh1": {"nodeType": "action", "actionExecution": "shell"}}, "edges": []}).to_string(),
        );
        write_file(
            &root,
            "projections/live.json",
            &json!({"status": "completed"}).to_string(),
        );

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.100Z", "type": "node_started", "nodeId": "sh1", "attemptId": "a1", "payload": {"nodeType": "action"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.200Z", "type": "action_completed", "nodeId": "sh1", "attemptId": "a1", "payload": {"action": {"exitCode": 0}, "stdoutArtifact": {"path": "../secret.txt"}, "stderrArtifact": {"path": "/etc/passwd"}}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.250Z", "type": "node_outcome", "nodeId": "sh1", "attemptId": "a1", "payload": {"nodeType": "action", "outcome": "ok", "durationMs": 10}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(&root, "trace.ndjson", &trace_lines);

        let events = replay(&root).unwrap_or_default();
        assert!(matches!(
            &events[2],
            DriverEvent::StepFinished {
                stdout: None,
                stderr: None,
                ..
            }
        ));
    }

    #[test]
    fn test_checkpoint_with_next_emits_waiting_with_sorted_choices() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(root, "manifest.json", &base_manifest("run-6", "cp", None));
        write_file(
            root,
            "flow.json",
            &json!({
                "name": "cp",
                "startAt": "gate",
                "nodes": {"gate": {"nodeType": "checkpoint", "summary": "Proceed with deploy?"}},
                "edges": []
            })
            .to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "waiting", "waitingOn": "gate"}).to_string(),
        );

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.100Z", "type": "node_started", "nodeId": "gate", "attemptId": "a1", "payload": {"nodeType": "checkpoint"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.200Z", "type": "node_outcome", "nodeId": "gate", "attemptId": "a1", "payload": {
                "nodeType": "checkpoint",
                "outcome": "ok",
                "durationMs": 5,
                "outputInline": {"next": {"zelda": "z.json", "alpha": "a.json"}}
            }}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        let waiting_events: Vec<&DriverEvent> = events
            .iter()
            .filter(|e| matches!(e, DriverEvent::Waiting { .. }))
            .collect();
        assert_eq!(waiting_events.len(), 1);
        assert!(matches!(
            waiting_events[0],
            DriverEvent::Waiting { node, attempt, question, choices, .. }
            if node == "gate"
                && attempt == "a1"
                && question == "Proceed with deploy?"
                && choices == &vec![
                    GateChoice { label: "alpha".to_string(), flow_file: "a.json".to_string() },
                    GateChoice { label: "zelda".to_string(), flow_file: "z.json".to_string() },
                ]
        ));
        assert!(
            matches!(events.last(), Some(DriverEvent::RunFinished { status, .. }) if status == "waiting")
        );
    }

    #[test]
    fn test_waiting_run_without_next() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(root, "manifest.json", &base_manifest("run-7", "cp2", None));
        write_file(
            root,
            "flow.json",
            &json!({
                "name": "cp2",
                "startAt": "gate2",
                "nodes": {"gate2": {"nodeType": "checkpoint"}},
                "edges": []
            })
            .to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "waiting", "waitingOn": "gate2", "updatedAt": "2026-11-20T01:14:35.000Z"}).to_string(),
        );

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.100Z", "type": "node_started", "nodeId": "gate2", "attemptId": "att-9", "payload": {"nodeType": "checkpoint"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.200Z", "type": "node_outcome", "nodeId": "gate2", "attemptId": "att-9", "payload": {
                "nodeType": "checkpoint",
                "outcome": "ok",
                "durationMs": 5
            }}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        assert!(matches!(
            &events[3],
            DriverEvent::Waiting { node, attempt, question, choices, .. }
            if node == "gate2" && attempt == "att-9" && question == "gate2" && choices.is_empty()
        ));
    }

    #[test]
    fn test_torn_last_line_ignored_then_completed() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(root, "manifest.json", &base_manifest("run-8", "torn", None));
        write_file(
            root,
            "flow.json",
            &json!({"name": "torn", "startAt": "n1", "nodes": {"n1": {"nodeType": "compute"}}, "edges": []}).to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "completed"}).to_string(),
        );

        let line1 = json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}).to_string() + "\n";
        let line2_full = json!({"seq": 2, "at": "2026-11-20T01:14:34.100Z", "type": "node_started", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "compute"}}).to_string() + "\n";
        let (part_a, part_b) = line2_full.split_at(25);

        let mut tailer = Tailer::new(root.to_path_buf());
        let ev1 = tailer.feed_manual(&[], &(line1 + part_a));
        assert_eq!(ev1.len(), 1);
        assert!(matches!(ev1[0], DriverEvent::RunStarted { .. }));

        let ev2 = tailer.feed_manual(&[], part_b);
        assert_eq!(ev2.len(), 1);
        assert!(matches!(&ev2[0], DriverEvent::StepStarted { node, .. } if node == "n1"));
    }

    #[test]
    fn test_switch_edges_and_bfs_order_with_unreachable_node() {
        let flow = json!({
            "name": "graph-flow",
            "startAt": "start",
            "nodes": {
                "z_unreach": {"nodeType": "compute"},
                "a_unreach": {"nodeType": "compute"},
                "start": {"nodeType": "compute"},
                "branch_a": {"nodeType": "acp"},
                "branch_b": {"nodeType": "action", "actionExecution": "function"}
            },
            "edges": [
                {
                    "from": "start",
                    "switch": {
                        "on": "$.res",
                        "cases": {
                            "yes": "branch_a",
                            "no": "branch_b"
                        }
                    }
                }
            ]
        });

        let (sketch, _, _) = build_flow_sketch(&flow, "fallback");
        let ids: Vec<&str> = sketch.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids[0], "start");
        assert!(ids[1..3].contains(&"branch_a"));
        assert!(ids[1..3].contains(&"branch_b"));
        assert_eq!(&ids[3..], &["a_unreach", "z_unreach"]);

        let fn_node = sketch
            .nodes
            .iter()
            .find(|n| n.id == "branch_b")
            .unwrap_or_else(|| unreachable!());
        assert_eq!(fn_node.kind, StepKind::Compute);

        assert_eq!(sketch.edges.len(), 2);
        assert!(sketch.edges.contains(&SketchEdge {
            from: "start".to_string(),
            to: "branch_a".to_string(),
            label: Some("yes".to_string()),
        }));
        assert!(sketch.edges.contains(&SketchEdge {
            from: "start".to_string(),
            to: "branch_b".to_string(),
            label: Some("no".to_string()),
        }));
    }

    #[test]
    fn test_list_runs_order_and_title_fallback() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let runs_dir = tmp.path();

        let r1 = runs_dir.join("r1");
        write_file(
            &r1,
            "manifest.json",
            &json!({
                "runId": "r1",
                "flowName": "flow-one",
                "startedAt": "2026-11-20T01:00:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r2 = runs_dir.join("r2");
        write_file(
            &r2,
            "manifest.json",
            &json!({
                "runId": "r2",
                "flowName": "flow-two",
                "runTitle": "Explicit Title",
                "startedAt": "2026-11-20T02:00:00.000Z",
                "status": "running"
            })
            .to_string(),
        );
        write_file(
            &r2,
            "projections/live.json",
            &json!({"status": "failed"}).to_string(),
        );

        let list = list_runs(runs_dir, 10);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "r2");
        assert_eq!(list[0].2, "Explicit Title");
        assert_eq!(list[0].3, "failed");
        assert_eq!(list[1].0, "r1");
        assert_eq!(list[1].2, "flow-one");
        assert_eq!(list[1].3, "completed");

        let limited = list_runs(runs_dir, 1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].0, "r2");
    }

    #[test]
    fn test_failed_acp_step_without_response_and_run_failed() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(
            root,
            "manifest.json",
            &base_manifest("run-9", "fail-flow", None),
        );
        write_file(
            root,
            "flow.json",
            &json!({"name": "fail-flow", "startAt": "n1", "nodes": {"n1": {"nodeType": "acp"}}, "edges": []}).to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "timed_out"}).to_string(),
        );

        let session_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.100Z", "direction": "outbound", "message": {"id": 1, "method": "session/prompt", "params": {"prompt": [{"text": "Go"}]}}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.200Z", "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"partial": true}}}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "sessions/s1/events.ndjson", &session_lines);

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.050Z", "type": "node_started", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "acp"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.080Z", "type": "acp_prompt_prepared", "nodeId": "n1", "attemptId": "a1", "sessionId": "s1", "payload": {}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:39.000Z", "type": "node_outcome", "nodeId": "n1", "attemptId": "a1", "payload": {"nodeType": "acp", "outcome": "timed_out", "durationMs": 5000, "error": "step timed out"}}),
            json!({"seq": 5, "at": "2026-11-20T01:14:39.100Z", "type": "run_failed", "payload": {"status": "timed_out", "error": "flow timed out"}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        assert_eq!(events.len(), 6);
        assert!(matches!(
            &events[3],
            DriverEvent::AgentUpdate { update, .. } if update == &json!({"partial": true})
        ));
        assert!(matches!(
            &events[4],
            DriverEvent::StepFinished { outcome, error: Some(err), .. }
            if outcome == "timed_out" && err == "step timed out"
        ));
        assert!(matches!(
            &events[5],
            DriverEvent::RunFinished { status, error: Some(err), .. }
            if status == "timed_out" && err == "flow timed out"
        ));
    }

    #[test]
    fn test_action_function_in_trace_is_compute() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let root = tmp.path();

        write_file(
            root,
            "manifest.json",
            &base_manifest("run-10", "fn-flow", None),
        );
        write_file(
            root,
            "flow.json",
            &json!({
                "name": "fn-flow",
                "startAt": "fn1",
                "nodes": {"fn1": {"nodeType": "action", "actionExecution": "function"}},
                "edges": []
            })
            .to_string(),
        );
        write_file(
            root,
            "projections/live.json",
            &json!({"status": "completed"}).to_string(),
        );

        let trace_lines = [
            json!({"seq": 1, "at": "2026-11-20T01:14:34.000Z", "type": "run_started", "payload": {}}),
            json!({"seq": 2, "at": "2026-11-20T01:14:34.100Z", "type": "node_started", "nodeId": "fn1", "attemptId": "a1", "payload": {"nodeType": "action"}}),
            json!({"seq": 3, "at": "2026-11-20T01:14:34.200Z", "type": "node_outcome", "nodeId": "fn1", "attemptId": "a1", "payload": {"nodeType": "action", "action": {"actionType": "function"}, "outcome": "ok", "durationMs": 12}}),
            json!({"seq": 4, "at": "2026-11-20T01:14:34.300Z", "type": "run_completed", "payload": {"status": "completed"}}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        write_file(root, "trace.ndjson", &trace_lines);

        let events = replay(root).unwrap_or_default();
        assert!(matches!(
            &events[1],
            DriverEvent::StepStarted {
                kind: StepKind::Compute,
                ..
            }
        ));
        assert!(matches!(
            &events[2],
            DriverEvent::StepFinished {
                kind: StepKind::Compute,
                ..
            }
        ));
    }
}
```