//! Live Claude and Pi sessions: one row per call, filled in place, completed only with content.
//!
//! The worker owns the merger. The view drains `Tail` on the existing one-second poll.
//! Phoenix is fetched off the UI thread. A dead Phoenix leaves receipts and the transcript running.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agent_client_protocol as acp;
use serde_json::{Map, Value, json};
use xai_grok_traces::merge::{self, Merger, Piece};
use xai_grok_traces::{TraceIndex, default_db_path, live, phoenix, remote};

use super::claude_feed::{self, Event, Fed};

const PHOENIX_WAIT: Duration = Duration::from_millis(800);
const WAKE_WAIT: Duration = Duration::from_secs(1);

pub(crate) struct Update {
    pub events: Vec<Fed>,
    pub life: String,
    /// Call id and tool name.
    pub running: Option<(String, String)>,
    pub phoenix_offline: bool,
}

pub(crate) struct Tail {
    rx: mpsc::Receiver<Update>,
    stop: Arc<AtomicBool>,
    _join: Option<JoinHandle<()>>,
}

impl Tail {
    #[cfg(test)]
    pub(crate) fn from_rx(rx: mpsc::Receiver<Update>) -> Self {
        Tail { rx, stop: Arc::new(AtomicBool::new(false)), _join: None }
    }

    /// Last status, with every pending batch's events concatenated.
    pub(crate) fn try_recv(&self) -> Option<Update> {
        let mut last = None;
        let mut events = Vec::new();
        while let Ok(mut u) = self.rx.try_recv() {
            events.append(&mut u.events);
            last = Some(u);
        }
        last.map(|mut u| {
            u.events = events;
            u
        })
    }
}

impl Drop for Tail {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// What the status line shows: `alive · Running Bash … 3.2s · Phoenix offline`.
pub(crate) struct TraceLive {
    pub tail: Tail,
    pub session_id: String,
    pub life: String,
    /// Call id, tool name, and when that call was first seen running.
    pub running: Option<(String, String, Instant)>,
    pub phoenix_offline: bool,
}

impl TraceLive {
    pub(crate) fn line(&self) -> String {
        let mut s = self.life.clone();
        if let Some((_, name, at)) = &self.running {
            s.push_str(&format!(" · Running {name} … {:.1}s", at.elapsed().as_secs_f64()));
        }
        if self.phoenix_offline {
            s.push_str(" · Phoenix offline");
        }
        s
    }
}

pub(crate) struct Job {
    pub source: String,
    pub session_id: String,
    pub file_path: PathBuf,
    pub native_id: Option<String>,
    pub host: Option<String>,
    pub harness_pid: Option<i64>,
    pub harness_start: Option<i64>,
    pub merger: Merger,
    pub remote: bool,
    pub subs: SubCache,
}

/// Size and mtime of a file, for skipping a poll when nothing it reads has changed.
type FileStamp = (u64, Option<std::time::SystemTime>);

fn file_stamp(p: &std::path::Path) -> FileStamp {
    std::fs::metadata(p).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None))
}

/// Claude keeps a session's subagent transcripts in `<session>/subagents/agent-*.jsonl`.
fn subagent_files(transcript: &std::path::Path) -> Vec<PathBuf> {
    let dir = transcript.with_extension("").join("subagents");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Parsed calls and results of each subagent transcript, re-read only when the file changed.
#[derive(Default)]
pub(crate) struct SubCache(std::collections::HashMap<PathBuf, (FileStamp, Vec<merge::Item>)>);

impl SubCache {
    pub(crate) fn items(&mut self, transcript: &std::path::Path) -> Vec<merge::Item> {
        let mut out = Vec::new();
        for f in subagent_files(transcript) {
            let st = file_stamp(&f);
            let fresh = self.0.get(&f).is_none_or(|(have, _)| *have != st);
            if fresh {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                let items = merge::parse("claude", &text)
                    .into_iter()
                    .filter(|i| matches!(i.kind, merge::ItemKind::Call { .. } | merge::ItemKind::Result { .. }))
                    .collect();
                self.0.insert(f.clone(), (st, items));
            }
            out.extend(self.0[&f].1.iter().cloned());
        }
        out
    }
}

/// Everything a local poll reads: the transcript, the subagent transcripts, and this session's newest receipt.
fn local_stamp(source: &str, transcript: &std::path::Path, newest_receipt: Option<(i64, i64)>) -> (Vec<FileStamp>, Option<(i64, i64)>) {
    let mut v: Vec<FileStamp> = live::transcript_files(source, transcript).iter().map(|f| file_stamp(f)).collect();
    v.extend(subagent_files(transcript).iter().map(|f| file_stamp(f)));
    (v, newest_receipt)
}

pub(crate) fn start(job: Job) -> Tail {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let join = std::thread::Builder::new().name("traces-live".into()).spawn(move || run(job, tx, flag)).ok();
    Tail { rx, stop, _join: join }
}

fn run(mut job: Job, tx: mpsc::Sender<Update>, stop: Arc<AtomicBool>) {
    let waker = xai_grok_traces::wake::Waker::bind_default();
    let mut primed = false;
    let mut last: Option<(Vec<FileStamp>, Option<(i64, i64)>)> = None;
    let mut idx: Option<Option<TraceIndex>> = None;
    let (mut receipts_seen, mut life_sent, mut offline_sent) = (Vec::new(), String::new(), false);
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if primed {
            waker.wait(WAKE_WAIT);
            if stop.load(Ordering::Acquire) {
                break;
            }
        }
        primed = true;
        if !job.remote {
            // Nothing the poll reads changed: no transcript parse, no database, no Phoenix. Only the pid can end the session.
            let idx = idx.get_or_insert_with(|| TraceIndex::open_with(&default_db_path(), Some(Duration::from_millis(200))).ok());
            let newest = idx.as_ref().and_then(|i| i.last_receipt_key(&job.session_id).ok().flatten());
            let stamp = local_stamp(&job.source, &job.file_path, newest);
            if last.as_ref() == Some(&stamp) {
                let life = merge::session_life(&job.source, &receipts_seen, pid_alive(job.harness_pid, job.harness_start)).to_string();
                if life != life_sent {
                    life_sent = life.clone();
                    if tx.send(Update { events: Vec::new(), life, running: job.merger.running(), phoenix_offline: offline_sent }).is_err() {
                        break;
                    }
                }
                continue;
            }
            last = Some(stamp);
            let items = job.subs.items(&job.file_path);
            job.merger.set_subagent_items(&items);
        }
        let snap = if job.remote { remote_snapshot(&job) } else { Ok(local_snapshot(&mut job)) };
        let (transcript, receipts, pid_alive, file_name) = match snap {
            Ok(s) => s,
            Err(e) => {
                // Not empty data: an empty snapshot would read as "alive" with nothing new.
                let update = Update { events: Vec::new(), life: format!("remote fetch failed: {e}"), running: job.merger.running(), phoenix_offline: false };
                if tx.send(update).is_err() {
                    break;
                }
                continue;
            }
        };
        let spans = fetch_spans(&job.source, &file_name, job.native_id.as_deref());
        let offline = spans.is_err();
        let (pieces, _) = job.merger.ingest(&job.source, &transcript, &receipts, spans.as_deref().map_err(|_| ()));
        let life = merge::session_life(&job.source, &receipts, pid_alive).to_string();
        (receipts_seen, life_sent, offline_sent) = (receipts, life.clone(), offline);
        let running = job.merger.running();
        let events = to_feds(&job.session_id, &pieces);
        if tx.send(Update { events, life, running, phoenix_offline: offline }).is_err() {
            break;
        }
    }
}

fn pid_alive(pid: Option<i64>, start: Option<i64>) -> Option<bool> {
    pid.map(|pid| match start {
        Some(start) => live::alive(pid, start),
        None => live::alive_pid(pid),
    })
}

fn local_snapshot(job: &mut Job) -> (String, Vec<live::LiveReceipt>, Option<bool>, String) {
    // shortcut: re-reads the whole file each wake, switch to a byte offset if a session misses the one-second tail.
    let transcript = live::transcript(&job.source, &job.file_path);
    let file_name = job.file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let idx = TraceIndex::open_with(&default_db_path(), Some(Duration::from_millis(200))).ok();
    let receipts = idx.as_ref().and_then(|i| i.session_receipts(&job.session_id).ok()).unwrap_or_default();
    let row = idx.as_ref().and_then(|i| i.session(&job.session_id).ok().flatten());
    job.harness_pid = row.as_ref().and_then(|r| r.harness_pid).or(job.harness_pid);
    job.harness_start = row.as_ref().and_then(|r| r.harness_start).or(job.harness_start);
    (transcript, receipts, pid_alive(job.harness_pid, job.harness_start), file_name)
}

fn remote_snapshot(job: &Job) -> Result<(String, Vec<live::LiveReceipt>, Option<bool>, String), String> {
    let label = job.host.as_deref().ok_or("no host on the row")?;
    let target = remote::hosts().into_iter().find(|t| remote::label(t) == label).ok_or_else(|| format!("{label} is not in the hosts file"))?;
    // Exit 2 means the other machine's grok-traces is older than live-json.
    let d = remote::fetch_live_dump(&target, &job.session_id).map_err(|e| e.to_string())?;
    Ok((d.transcript, d.receipts, d.pid_alive, d.file_name))
}

pub(crate) fn fetch_spans(source: &str, file_name: &str, native_id: Option<&str>) -> Result<Vec<phoenix::PhoenixSpan>, ()> {
    // shortcut: dpi and Codex have no span emitter (Codex has no otel config here); key them when one exists.
    if source.is_empty() || matches!(source, "dpi" | "codex") {
        return Ok(Vec::new());
    }
    let sid = merge::phoenix_session_id(file_name, native_id);
    if sid.is_empty() {
        return Ok(Vec::new());
    }
    // Today's Pi emitter may stamp session.id as pi-<epoch>, so this key can miss. Receipts still open the rows.
    phoenix::fetch_session_spans(phoenix::DEFAULT_BASE, source, &sid, PHOENIX_WAIT).map_err(|_| ())
}

pub(crate) fn to_feds(session_id: &str, pieces: &[Piece]) -> Vec<Fed> {
    let mut out = Vec::new();
    for (n, piece) in pieces.iter().enumerate() {
        let mut stream_start = None;
        let (event, ts, dur) = match piece {
            Piece::User(t) => (Event::Acp(acp::SessionUpdate::UserMessageChunk(claude_feed::chunk(t))), None, None),
            Piece::Text(t) => (Event::Acp(acp::SessionUpdate::AgentMessageChunk(claude_feed::chunk(t))), None, None),
            Piece::Thinking { text, at_ms, dur_ms } => {
                // A new stream start splits it from the thought before; durationMs on the chunk times it (a live thought
                // otherwise runs its own wall-clock timer, which reads 0.0s for a thought that arrives whole).
                stream_start = dur_ms.map(|d| at_ms - d);
                let ts = stream_start.map(|_| *at_ms);
                // Always present: a null durationMs (Pi and dpi record none) still marks the thought as arriving whole.
                let chunk = claude_feed::chunk(text).meta(Some(Map::from_iter([("durationMs".to_string(), json!(dur_ms))])));
                (Event::Acp(acp::SessionUpdate::AgentThoughtChunk(chunk)), ts, None)
            }
            Piece::Open { id, tool, start_ms, input, label, .. } => {
                let raw = input_value(input.as_deref());
                let title = title(tool, &raw, label.as_deref());
                let tc = claude_feed::tool_call(id, kind_of(tool), title, raw, acp::ToolCallStatus::InProgress, Vec::new(), None, None);
                (Event::Acp(acp::SessionUpdate::ToolCall(tc)), Some(*start_ms), None)
            }
            Piece::Fill { id, tool, input, output } => {
                let raw = input.as_deref().map(|s| input_value(Some(s)));
                let content = output.as_deref().filter(|t| !t.is_empty()).map(|t| vec![claude_feed::text_content(t)]);
                let mut fields = update_fields(acp::ToolCallStatus::InProgress, None, content);
                if let Some(raw) = raw {
                    fields = fields.title(Some(title_of(tool, &raw))).raw_input(Some(raw));
                }
                let upd = acp::ToolCallUpdate::new(acp::ToolCallId::new(Arc::from(id.as_str())), fields);
                (Event::Acp(acp::SessionUpdate::ToolCallUpdate(upd)), None, None)
            }
            Piece::Complete { id, tool, start_ms, end_ms, error, input, label, output, fresh, .. } => {
                let raw = input_value(input.as_deref());
                let title = title(tool, &raw, label.as_deref());
                let status = if *error { acp::ToolCallStatus::Failed } else { acp::ToolCallStatus::Completed };
                let content = content_of(tool, &raw, output, *error);
                let dur = (*end_ms - *start_ms).max(0);
                let event = if *fresh {
                    let tc = claude_feed::tool_call(id, kind_of(tool), title, raw, status, content, None, Some(dur));
                    Event::Acp(acp::SessionUpdate::ToolCall(tc))
                } else {
                    // A null input would wipe what an earlier fill set; the title always goes, so a row never keeps "Bash call".
                    let raw = (!raw.is_null()).then_some(raw);
                    let fields = update_fields(status, raw, Some(content)).title(Some(title));
                    let upd = acp::ToolCallUpdate::new(acp::ToolCallId::new(Arc::from(id.as_str())), fields);
                    Event::Acp(acp::SessionUpdate::ToolCallUpdate(upd))
                };
                (event, Some(*end_ms), Some(dur))
            }
        };
        let mut m = meta(session_id, n, ts, dur);
        if let Some(s) = stream_start {
            m.insert("streamStartMs".into(), json!(s));
        }
        out.push(Fed { event, meta: m });
    }
    out
}

fn meta(session_id: &str, n: usize, ts: Option<i64>, dur: Option<i64>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("isReplay".into(), json!(true));
    m.insert("eventId".into(), json!(format!("{session_id}-live-{n}")));
    if let Some(ts) = ts {
        m.insert("agentTimestampMs".into(), json!(ts));
    }
    if let Some(d) = dur {
        m.insert("durationMs".into(), json!(d));
    }
    m
}

fn input_value(raw: Option<&str>) -> Value {
    match raw {
        Some(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({ "input": s })),
        None => Value::Null,
    }
}

fn kind_of(name: &str) -> acp::ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "bash" => acp::ToolKind::Execute,
        "read" => acp::ToolKind::Read,
        "grep" | "glob" | "find" | "websearch" => acp::ToolKind::Search,
        "edit" | "write" | "multiedit" => acp::ToolKind::Edit,
        "webfetch" => acp::ToolKind::Fetch,
        _ => acp::ToolKind::Other,
    }
}

fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The transcript's input names the call; before it has any, the receipt's detail does. Never the bare tool name when either exists.
fn title(name: &str, input: &Value, label: Option<&str>) -> String {
    match label.filter(|l| input.is_null() && !l.trim().is_empty()) {
        Some(l) => l.lines().next().unwrap_or(l).trim().to_string(),
        None => title_of(name, input),
    }
}

fn title_of(name: &str, input: &Value) -> String {
    let t = match name.to_ascii_lowercase().as_str() {
        "bash" => {
            let d = str_at(input, "description");
            let cmd = str_at(input, "command");
            let t = if d.is_empty() { cmd } else { d };
            t.lines().next().unwrap_or("").trim().to_string()
        }
        "read" => {
            let p = {
                let a = str_at(input, "file_path");
                if a.is_empty() { str_at(input, "path") } else { a }
            };
            if p.is_empty() { String::new() } else { format!("Read {p}") }
        }
        "edit" | "write" | "multiedit" => {
            let a = str_at(input, "file_path");
            if a.is_empty() { str_at(input, "path").to_string() } else { a.to_string() }
        }
        _ => claude_feed::one_line(name, input),
    };
    if t.is_empty() || t.eq_ignore_ascii_case("task") || t.eq_ignore_ascii_case("todowrite") || t == "Updating plan" {
        format!("{name} call")
    } else {
        t
    }
}

fn content_of(tool: &str, input: &Value, output: &str, failed: bool) -> Vec<acp::ToolCallContent> {
    if matches!(tool.to_ascii_lowercase().as_str(), "edit" | "write" | "multiedit") {
        let path = {
            let a = str_at(input, "file_path");
            if a.is_empty() { str_at(input, "path").to_string() } else { a.to_string() }
        };
        let (old, new) = if let Some(edits) = input.get("edits").and_then(Value::as_array) {
            let old = edits.iter().map(|e| str_at(e, "old_string").to_string() + str_at(e, "oldText")).collect::<Vec<_>>().join("\n");
            let new = edits.iter().map(|e| str_at(e, "new_string").to_string() + str_at(e, "newText")).collect::<Vec<_>>().join("\n");
            (Some(old), new)
        } else {
            let old = {
                let a = str_at(input, "old_string");
                let b = if a.is_empty() { str_at(input, "oldText") } else { a };
                if b.is_empty() { None } else { Some(b.to_string()) }
            };
            let new = {
                let a = str_at(input, "new_string");
                let b = if a.is_empty() { str_at(input, "newText") } else { a };
                let c = if b.is_empty() { str_at(input, "content") } else { b };
                c.to_string()
            };
            (old, new)
        };
        return claude_feed::edit_content(failed, output, &path, old, new);
    }
    if output.is_empty() { Vec::new() } else { vec![claude_feed::text_content(output)] }
}

fn update_fields(status: acp::ToolCallStatus, input: Option<Value>, content: Option<Vec<acp::ToolCallContent>>) -> acp::ToolCallUpdateFields {
    let mut f = acp::ToolCallUpdateFields::new().status(Some(status));
    if let Some(i) = input {
        f = f.raw_input(Some(i));
    }
    if let Some(c) = content {
        f = f.content(Some(c));
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::meta::NotificationMeta;
    use crate::acp::tracker::AcpUpdateTracker;
    use crate::scrollback::state::ScrollbackState;

    fn apply(pieces: &[Piece]) -> ScrollbackState {
        let mut tracker = AcpUpdateTracker::new();
        let mut sb = ScrollbackState::new();
        for fed in to_feds("s", pieces) {
            let meta = NotificationMeta::from_json(Some(&fed.meta));
            let Event::Acp(update) = fed.event else { continue };
            tracker.handle_update(update, &meta, &mut sb);
        }
        sb
    }

    #[test]
    fn open_then_complete_is_one_row() {
        let input = Some(r#"{"command":"ls"}"#.to_string());
        let open = Piece::Open { id: "c1".into(), tool: "Bash".into(), start_ms: 1_000, input: input.clone(), label: None, parent: None };
        let mut sb = apply(&[open]);
        assert_eq!(sb.len(), 1);
        assert!(sb.iter_entries().all(|(_, e)| e.is_running));
        let done = Piece::Complete {
            id: "c1".into(),
            tool: "Bash".into(),
            start_ms: 1_000,
            end_ms: 1_500,
            error: false,
            input,
            label: None,
            output: "ok".into(),
            fresh: false,
            parent: None,
        };
        sb = apply(&[
            Piece::Open { id: "c1".into(), tool: "Bash".into(), start_ms: 1_000, input: done_input(), label: None, parent: None },
            done,
        ]);
        assert_eq!(sb.len(), 1);
        assert!(sb.iter_entries().all(|(_, e)| !e.is_running));
        let fresh = Piece::Complete {
            id: "c2".into(),
            tool: "MultiEdit".into(),
            start_ms: 1,
            end_ms: 2,
            error: false,
            input: Some(r#"{"file_path":"a.rs","edits":[{"old_string":"a","new_string":"b"},{"old_string":"c","new_string":"d"}]}"#.into()),
            label: None,
            output: "ok".into(),
            fresh: true,
            parent: Some("c1".into()),
        };
        let feds = to_feds("s", std::slice::from_ref(&fresh));
        assert_eq!(feds.len(), 1, "MultiEdit is one row");
        let sb = apply(&[Piece::Complete {
            id: "c2".into(),
            tool: "MultiEdit".into(),
            start_ms: 1,
            end_ms: 2,
            error: false,
            input: Some(r#"{"file_path":"a.rs","edits":[{"old_string":"a","new_string":"b"}]}"#.into()),
            label: None,
            output: "ok".into(),
            fresh: true,
            parent: None,
        }]);
        assert_eq!(sb.len(), 1);
        assert!(sb.iter_entries().all(|(_, e)| !e.is_running));
    }

    #[test]
    fn live_remote_fetch_failure_is_an_error_not_empty_data() {
        let job = |host: Option<&str>| Job {
            source: "claude".into(),
            session_id: "s".into(),
            file_path: PathBuf::new(),
            native_id: None,
            host: host.map(str::to_string),
            harness_pid: None,
            harness_start: None,
            merger: Merger::default(),
            remote: true,
            subs: SubCache::default(),
        };
        assert!(remote_snapshot(&job(None)).is_err());
        let err = remote_snapshot(&job(Some("no-such-host-for-this-test"))).unwrap_err();
        assert!(err.contains("not in the hosts file"), "{err}");
    }

    /// An idle live view used 15-28% CPU: every second it re-read and re-parsed the transcript and all subagent
    /// transcripts and refetched Phoenix. The stamp it now checks first stays put until something it reads changes.
    #[test]
    fn live_poll_stamp_is_stable_while_idle_and_subagent_files_are_reparsed_only_when_changed() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("s.jsonl");
        std::fs::write(&main, "{}\n").unwrap();
        std::fs::create_dir_all(dir.path().join("s/subagents")).unwrap();
        let sub = dir.path().join("s/subagents/agent-a1.jsonl");
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_X","name":"Bash","input":{"command":"ls"}}]}}"#;
        std::fs::write(&sub, format!("{call}\n")).unwrap();
        let first = local_stamp("claude", &main, Some((1, 1)));
        assert_eq!(first, local_stamp("claude", &main, Some((1, 1))), "nothing changed, nothing to do");
        assert_ne!(first, local_stamp("claude", &main, Some((2, 2))), "a new receipt is a change");

        let mut cache = SubCache::default();
        assert_eq!(cache.items(&main).len(), 1);
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_X","content":"a"}]}}"#;
        std::fs::write(&sub, format!("{call}\n{result}\n")).unwrap();
        assert_ne!(first, local_stamp("claude", &main, Some((1, 1))), "a subagent transcript grew");
        assert_eq!(cache.items(&main).len(), 2, "the grown file is re-read");
    }

    fn done_input() -> Option<String> {
        Some(r#"{"command":"ls"}"#.into())
    }
}
