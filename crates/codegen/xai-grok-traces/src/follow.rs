//! `grok-traces follow`: a `tail -f` for one live session (and the subagents it dispatches).
//!
//! The formatters are pure; [`Follower::tick`] indexes just the followed files (the same single-file path
//! hooks use), reads the new rows and returns the blocks to print.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::index::session_id;
use crate::live::{Ev, LiveReceipt, norm};
use crate::normalize::{clean_text, normalize};
use crate::receipts::native_keys;
use crate::roots::resolve;
use crate::{ACTIVE_WINDOW_MS, EventFilter, EventRow, MAX_EVENTS_PER_FILE, Result, Root, SessionRow, Status, TraceIndex};

const WRAP: usize = 100;
const CLIP: usize = 120;
/// Rows shown for a session that is already running when followed (`tail`'s default).
const BACKLOG: usize = 10;
const IDLE_FIRST_S: u64 = 30;
const IDLE_EVERY_S: u64 = 60;

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}

fn first_line(s: &str) -> &str {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// `claude-sonnet-5-5` -> `sonnet-5-5`; a trailing `-YYYYMMDD` is dropped.
pub fn short_model(m: &str) -> String {
    let m = m.strip_prefix("claude-").unwrap_or(m);
    match m.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => head.to_string(),
        _ => m.to_string(),
    }
}

fn secs(s: u64) -> String {
    if s < 60 { format!("{s}s") } else { format!("{}m{:02}s", s / 60, s % 60) }
}

/// `0.4s`, `12s`, `2m05s`.
fn took(ms: i64) -> String {
    if ms < 10_000 { format!("{:.1}s", ms as f64 / 1000.0) } else { secs((ms as f64 / 1000.0).round() as u64) }
}

/// `<source> · <model> · <title> · <cwd>`, `home` shown as `~`.
pub fn header(s: &SessionRow, home: Option<&str>) -> String {
    let cwd = s.cwd.as_deref().map_or_else(
        || "-".to_string(),
        |c| match home.filter(|h| !h.is_empty()).and_then(|h| c.strip_prefix(h)) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
            _ => c.to_string(),
        },
    );
    let model = s.model.as_deref().map_or_else(|| "-".to_string(), short_model);
    format!("{} · {model} · {} · {cwd}", s.source, first_line(&s.title))
}

pub fn idle_line(s: u64) -> String {
    format!("… idle {}", secs(s))
}

pub fn ended_line(depth: usize) -> String {
    if depth == 0 { "■ ended".into() } else { format!("{}↳ ended", indent(depth)) }
}

/// The idle listing: one line per recent ended session, `<id>  <model>  <age> ago  <title>`.
pub fn recent_table(rows: &[crate::live::LiveRow], now_ms: i64) -> String {
    rows.iter()
        .map(|r| {
            let model = r.model.as_deref().map_or_else(|| "-".to_string(), short_model);
            let age = r.ended_at.map_or_else(|| "-".to_string(), |t| format!("{} ago", crate::live::fmt_age(now_ms - t)));
            format!("{}  {model}  {age}  {}", r.session_id, clip(first_line(&r.title), 60))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn started_line(model: Option<&str>, depth: usize) -> String {
    let model = model.map(|m| format!("{} ", short_model(m))).unwrap_or_default();
    format!("{}↳ {model}started", indent(depth))
}

/// Human text of one raw line: every `text`/`thinking` string and string `content`, tool inputs skipped.
fn raw_text(raw: &str) -> String {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Array(xs) => xs.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => {
                for (k, x) in m {
                    match (k.as_str(), x) {
                        ("input" | "arguments" | "usage", _) => {}
                        ("text" | "thinking" | "content", Value::String(s)) => {
                            let t = s.trim_end();
                            if !t.trim().is_empty() && out.last().is_none_or(|l| l != t) {
                                out.push(t.to_string());
                            }
                        }
                        _ => walk(x, out),
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        walk(&v, &mut out);
    }
    clean_text(&out.join("\n\n"))
}

/// The row's full text from its raw line, else its (clipped, flattened) preview.
fn full_text(ev: &EventRow, raw: Option<&str>) -> String {
    let t = raw.map(raw_text).unwrap_or_default();
    if !t.is_empty() {
        return t;
    }
    if ev.preview == ev.title { String::new() } else { ev.preview.trim().to_string() }
}

/// Word-wraps `text` to `width` chars per line; blank lines dropped, a word longer than `width` stays whole.
/// shortcut: width is counted in chars, not terminal cells; wide glyphs overshoot.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        let mut cur = String::new();
        for word in line.split(' ') {
            if !cur.trim().is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else if cur.is_empty() {
                cur = word.to_string();
            } else {
                cur.push(' ');
                cur.push_str(word);
            }
        }
        out.push(cur);
    }
    out
}

fn body(pfx: &str, mark: &str, text: &str) -> Option<String> {
    let lines = wrap(text, WRAP.saturating_sub(pfx.len() + 3).max(20));
    let (first, rest) = lines.split_first()?;
    let mut s = format!("{pfx}{mark}  {first}");
    for l in rest {
        s.push_str(&format!("\n{pfx}   {l}"));
    }
    Some(s)
}

/// A failed result's `(exit, message)` first lines; either may be empty.
fn error_parts(ev: &EventRow) -> (&str, &str) {
    let detail = ev.detail.as_deref().map(first_line).unwrap_or("");
    if detail.starts_with("exit ") { (detail, first_line(&ev.preview)) } else { ("", if detail.is_empty() { first_line(&ev.preview) } else { detail }) }
}

/// One printed block for a row, or `None` when the row is not shown. `raw` is the row's source line.
pub fn format_event(ev: &EventRow, raw: Option<&str>, depth: usize) -> Option<String> {
    if ev.noise {
        return None;
    }
    let pfx = indent(depth);
    let role = ev.role.as_deref();
    match (ev.kind.as_str(), role) {
        ("tool", Some("tool")) => {
            let what = [ev.intent.as_deref(), ev.detail.as_deref(), Some(ev.preview.as_str())]
                .into_iter()
                .flatten()
                .map(first_line)
                .find(|s| !s.is_empty() && *s != ev.title)
                .unwrap_or("");
            Some(format!("{pfx}→ {}  {}", ev.title, clip(what, CLIP)).trim_end().to_string())
        }
        ("tool", Some("tool_result")) if ev.is_error => {
            let (exit, msg) = error_parts(ev);
            let parts: Vec<String> = [exit, msg].into_iter().filter(|s| !s.is_empty()).map(|s| clip(s, CLIP)).collect();
            Some(format!("{pfx}   ✗ {}", parts.join("  ")).trim_end().to_string())
        }
        ("tool", Some("tool_result")) => Some(match ev.duration_ms {
            Some(ms) => format!("{pfx}   ✓ {}", took(ms)),
            None => format!("{pfx}   ✓"),
        }),
        ("reasoning", _) | (_, Some("reasoning")) => body(&pfx, "~", &full_text(ev, raw)),
        ("message", Some("assistant")) => body(&pfx, "·", &full_text(ev, raw)),
        ("message", Some("user")) => {
            let text = full_text(ev, raw);
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            let head = clip(lines.next()?, CLIP);
            let more = lines.count();
            let more = if more > 0 { format!(" (+{more} lines)") } else { String::new() };
            Some(format!("\n{pfx}▶ you  {head}{more}"))
        }
        _ => None,
    }
}

/// `follow --json` item wire version.
const ITEM_V: u32 = 1;

fn item(kind: &str, agent: &str, depth: usize, line: Option<i64>, ts: Option<i64>) -> Value {
    json!({"v": ITEM_V, "kind": kind, "agent": agent, "depth": depth, "line": line, "ts": ts})
}

/// `agent_start`, `agent_end` or `agent_resume` (the root gets `agent_end` only; its start and resume come from `live`).
pub fn agent_item(kind: &str, agent: &str, depth: usize, ts: Option<i64>, model: Option<&str>, label: Option<&str>) -> Value {
    let mut v = item(kind, agent, depth, None, ts);
    if kind == "agent_start" {
        v["model"] = json!(model);
        v["label"] = json!(label);
    }
    v
}

/// A `model` item: the agent's model changed (or, for the root, is first known).
pub fn model_item(agent: &str, depth: usize, line: Option<i64>, ts: Option<i64>, model: &str, label: Option<&str>) -> Value {
    let mut v = item("model", agent, depth, line, ts);
    v["model"] = json!(model);
    v["label"] = json!(label);
    v
}

/// The `--json` item for a row, shown exactly when [`format_event`] shows it; text is full and unwrapped.
/// `call_id` is the tool call id (`call`) or the id of the call answered (`result`).
pub fn json_event(ev: &EventRow, raw: Option<&str>, call_id: Option<&str>, agent: &str, depth: usize) -> Option<Value> {
    if ev.noise {
        return None;
    }
    let mut v = item("", agent, depth, Some(ev.line), ev.timestamp);
    let text = || Some(full_text(ev, raw)).filter(|t| !t.trim().is_empty());
    match (ev.kind.as_str(), ev.role.as_deref()) {
        ("tool", Some("tool")) => {
            let what = [ev.intent.as_deref(), ev.detail.as_deref()].into_iter().flatten().map(first_line).find(|s| !s.is_empty()).unwrap_or("");
            v["kind"] = json!("call");
            v["call_id"] = json!(call_id);
            v["tool"] = json!(ev.title);
            v["what"] = json!(what);
        }
        ("tool", Some("tool_result")) => {
            v["kind"] = json!("result");
            v["call_id"] = json!(call_id);
            v["ok"] = json!(!ev.is_error);
            // Additive (v stays 1): the call's duration, when the index has one.
            if let Some(ms) = ev.duration_ms {
                v["ms"] = json!(ms);
            }
            if ev.is_error {
                let (exit, msg) = error_parts(ev);
                v["error"] = json!([exit, msg].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  "));
            }
        }
        ("reasoning", _) | (_, Some("reasoning")) => {
            v["kind"] = json!("reasoning");
            v["text"] = json!(text()?);
        }
        ("message", Some("assistant")) => {
            v["kind"] = json!("text");
            v["text"] = json!(text()?);
        }
        ("message", Some("user")) => {
            v["kind"] = json!("user");
            v["text"] = json!(text()?);
        }
        _ => return None,
    }
    Some(v)
}

struct Followed {
    /// The agent's name in the output: its session row id, or for a child adopted from a receipt its `agent_id`.
    id: String,
    /// The session row to read files from; `None` while a receipt-born child has no transcript yet.
    row: Option<String>,
    /// The receipt `agent_id` of a child adopted from a receipt.
    agent_id: Option<String>,
    depth: usize,
    last_line: i64,
    ended: bool,
    /// A stop receipt ended this child: the file's freshness no longer says it is alive.
    stopped: bool,
    /// Highest receipt id read for this child's own session (grok and Pi write a child's receipts there).
    rcpt_after: i64,
    /// The child's model and label were sent after its row became known (receipt-born children start without).
    meta_sent: bool,
    /// Last model announced for this session (`--json` `model` items).
    model: Option<String>,
    label: Option<String>,
    /// Printed before this session's first row, so a subagent's start follows its parent's rows.
    announce: Option<String>,
    /// The session's first user row when the backlog cut it off: a subagent's brief is printed first, not lost.
    brief: Option<EventRow>,
    /// Tool call id -> line of calls with no result yet; the index puts a call's duration on the call row.
    calls: HashMap<String, i64>,
}

/// A call opened by a `Pre` receipt. Its file row is not printed again; `done` once its result was printed.
struct RcptCall {
    agent: String,
    depth: usize,
    at_ms: i64,
    done: bool,
    /// When a `Post` that does not say success arrived: the file's result prints it and takes its duration from this.
    post_ms: Option<i64>,
}

/// A file named one of `names` under `dir`, at most `levels` directories down (Claude: `subagents/` and `subagents/workflows/<wf>/`).
fn find_agent_file(dir: &Path, names: &[String], levels: usize) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| names.iter().any(|m| n == m.as_str())) {
            return Some(p);
        }
        if levels > 0 && p.is_dir() {
            if let Some(found) = find_agent_file(&p, names, levels - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// The receipt `agent_id` a child session is known by: Claude `agent-<id>.jsonl`, else its native id
/// (Cursor's file stem, grok's session dir, Pi's session id).
fn child_key(s: &SessionRow) -> Option<String> {
    if let Some(a) = s.file_path.file_stem().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("agent-")) {
        return Some(a.to_string());
    }
    let native = s.native_id.as_deref()?;
    Some(native_keys(&s.source, Some(native)).pop().unwrap_or_else(|| native.to_string()))
}

/// Sources that write a child's tool receipts under the child's own session (`native_session` = the `agent_id`
/// of the parent's `ChildStart`), not on the parent's. Claude's land on the parent's with an `agent_id`.
fn child_receipts_own_session(source: &str) -> bool {
    matches!(source, "grok" | "pi")
}

/// How a `Post` receipt closes its call: `Some(true)` success, `Some(false)` a failure or denial (closed now,
/// the file's result adds the output), `None` unknown (Pi, Cursor's `after*`: the file's result says).
fn post_outcome(event: &str) -> Option<bool> {
    match event {
        "PostToolUse" | "postToolUse" => Some(true),
        e if e.ends_with("Failure") || e == "PermissionDenied" => Some(false),
        _ => None,
    }
}

/// The session to attach to: `id` if given, else the newest active root; only if it is live now.
pub fn find_live_root(idx: &TraceIndex, id: Option<&str>) -> Result<Option<SessionRow>> {
    Ok(match id {
        Some(id) => idx.session(id)?.filter(|s| s.status == Status::Active),
        None => idx.list_sessions(&crate::SessionFilter { limit: 1, active_only: true, roots_only: true, ..Default::default() })?.pop(),
    })
}

pub struct Follower {
    idx: TraceIndex,
    roots: Vec<Root>,
    children: bool,
    home: Option<String>,
    sessions: Vec<Followed>,
    root: SessionRow,
    started: bool,
    last_activity_ms: i64,
    next_idle_s: u64,
    json: bool,
    backlog: usize,
    /// Highest receipt id read for the root.
    rcpt_after: i64,
    /// Calls opened from receipts, by tool-use id.
    rcalls: HashMap<String, RcptCall>,
    /// Tool-use ids of calls the files printed: a late `Pre` for one is not a new row.
    file_calls: HashSet<String>,
}

impl Follower {
    pub fn new(idx: TraceIndex, roots: Vec<Root>, root: SessionRow, children: bool, home: Option<String>) -> Self {
        Self { idx, roots, children, home, sessions: Vec::new(), root, started: false, last_activity_ms: 0, next_idle_s: IDLE_FIRST_S, json: false, backlog: BACKLOG, rcpt_after: 0, rcalls: HashMap::new(), file_calls: HashSet::new() }
    }

    /// Emit one JSON item per line (see [`json_event`]) instead of formatted text.
    pub fn with_json(mut self, json: bool) -> Self {
        self.json = json;
        self
    }

    /// Rows shown for sessions that are already running when followed.
    pub fn with_backlog(mut self, n: usize) -> Self {
        self.backlog = n;
        self
    }

    /// The followed root session has ended (its end line or item was emitted).
    pub fn root_ended(&self) -> bool {
        self.sessions.first().is_some_and(|f| f.ended)
    }

    /// What is printed before a subagent's first row: its start line, or its `agent_start` item.
    fn start_block(&self, kid: &SessionRow, depth: usize) -> String {
        if self.json {
            agent_item("agent_start", &kid.id, depth, kid.started_at, kid.model.as_deref(), kid.label.as_deref()).to_string()
        } else {
            started_line(kid.model.as_deref(), depth)
        }
    }

    /// Indexes the session's file alone (as a hook would); sources with no file (bb) are left to their hooks.
    fn index(&mut self, path: &Path) -> Result<()> {
        if let Some((root, spelled)) = resolve(path, &self.roots) {
            self.idx.index_file(root, &spelled)?;
        }
        Ok(())
    }

    /// Line to resume after so that `tail` rows are still to print; -1 (lines count from 0) prints everything.
    /// Also the first user message when it is at or before that line (cut off, so the caller can print it).
    fn resume_after(&self, id: &str, tail: Option<usize>) -> Result<(i64, Option<EventRow>)> {
        let Some(n) = tail else { return Ok((-1, None)) };
        let rows = self.idx.list_events(id, &EventFilter { limit: MAX_EVENTS_PER_FILE, ..Default::default() })?;
        // Never start on a result whose call is out of view.
        let mut from = rows.len().saturating_sub(n);
        while from > 0 && from < rows.len() && rows[from].role.as_deref() == Some("tool_result") {
            from -= 1;
        }
        let last_line = from.checked_sub(1).map_or(-1, |i| rows[i].line);
        let brief = rows.into_iter().find(|r| r.kind == "message" && r.role.as_deref() == Some("user")).filter(|r| r.line <= last_line);
        Ok((last_line, brief))
    }

    fn adopt(&mut self, s: &SessionRow, depth: usize, tail: Option<usize>, announce: Option<String>) -> Result<()> {
        let (last_line, brief) = self.resume_after(&s.id, tail)?;
        // A child found by its file is still known by the id its receipts carry, or they would adopt it twice.
        let agent_id = child_key(s).filter(|_| depth > 0);
        self.sessions.push(Followed {
            id: s.id.clone(),
            row: Some(s.id.clone()),
            agent_id,
            depth,
            last_line,
            ended: false,
            stopped: false,
            rcpt_after: self.rcpt_after,
            meta_sent: true,
            model: s.model.clone(),
            label: s.label.clone(),
            announce,
            // A root keeps today's backlog; a subagent (or any session with a parent) is read with its brief.
            brief: brief.filter(|_| depth > 0 || s.parent_id.is_some()),
            calls: HashMap::new(),
        });
        Ok(())
    }

    /// The followed agent for a receipt's `agent_id` (`None`: the root). A child not seen before is adopted
    /// now, without a file, and its start is printed. `None` when children are off.
    fn agent_at(&mut self, agent_id: Option<&str>, at_ms: i64, out: &mut Vec<String>) -> Option<usize> {
        let Some(a) = agent_id else { return Some(0) };
        if let Some(at) = self.sessions.iter().position(|f| f.agent_id.as_deref() == Some(a)) {
            return Some(at);
        }
        if !self.children {
            return None;
        }
        out.push(if self.json { agent_item("agent_start", a, 1, Some(at_ms), None, None).to_string() } else { started_line(None, 1) });
        self.sessions.push(Followed {
            id: a.to_string(),
            row: None,
            agent_id: Some(a.to_string()),
            depth: 1,
            last_line: -1,
            ended: false,
            stopped: false,
            rcpt_after: 0,
            meta_sent: false,
            model: None,
            label: None,
            announce: None,
            brief: None,
            calls: HashMap::new(),
        });
        Some(self.sessions.len() - 1)
    }

    /// The child's own grok session: `<sessions>/<bucket>/<id>/chat_history.jsonl`, the parent's bucket first.
    /// (A grok child is its own session, indexed with `native_id` = its dir name; see `stamp_grok_children`.)
    fn grok_child_file(&self, id: &str) -> Option<PathBuf> {
        let name = "chat_history.jsonl";
        let root = self.roots.iter().find(|r| r.source == "grok")?;
        let mut paths: Vec<PathBuf> = self.root.file_path.parent().and_then(Path::parent).map(|b| b.join(id).join(name)).into_iter().collect();
        paths.push(root.path.join(id).join(name));
        paths.extend(fs::read_dir(&root.path).ok()?.flatten().map(|e| e.path().join(id).join(name)));
        paths.into_iter().find(|p| p.is_file())
    }

    /// Binds a receipt-born child to its transcript: `hint` (the path a stop receipt carries), else the harness's
    /// layout for a child file (Claude `<stem>/subagents/**/agent-<id>.jsonl`, Cursor `subagents/<id>.jsonl` next to
    /// the parent, grok its own session dir), else a session already indexed under that id (Pi). Does nothing while
    /// there is no such file.
    fn attach(&mut self, at: usize, hint: Option<&str>) -> Result<()> {
        let Some(agent) = self.sessions[at].agent_id.clone() else { return Ok(()) };
        if agent.contains(['/', '\\']) || agent.contains("..") {
            return Ok(());
        }
        let names = [format!("agent-{agent}.jsonl"), format!("{agent}.jsonl")];
        let parent = &self.root.file_path;
        let mut paths: Vec<PathBuf> = hint.map(PathBuf::from).into_iter().collect();
        match self.root.source.as_str() {
            "claude" => paths.extend(find_agent_file(&parent.with_extension("").join("subagents"), &names[..1], 2)),
            "cursor" => paths.extend(parent.parent().and_then(|d| find_agent_file(&d.join("subagents"), &names, 0))),
            "grok" => paths.extend(self.grok_child_file(&agent)),
            _ => {}
        }
        for p in paths.into_iter().filter(|p| p.is_file()) {
            let Some((root, spelled)) = resolve(&p, &self.roots).map(|(r, s)| (r, s.to_path_buf())) else { continue };
            self.idx.index_file(root, &spelled)?;
            if let Some(kid) = self.idx.session(&session_id(&root.source, &spelled.to_string_lossy()))? {
                self.sessions[at].row = Some(kid.id);
                return Ok(());
            }
        }
        if child_receipts_own_session(&self.root.source) {
            self.sessions[at].row = self.idx.session_by_native(&self.root.source, &agent)?;
        }
        Ok(())
    }

    /// What was still in flight when the follow began: calls and children with no close yet, and recent.
    fn open_receipts(&self, recs: &[LiveReceipt], now_ms: i64) -> Vec<LiveReceipt> {
        let source = &self.root.source;
        let key = |r: &LiveReceipt| r.tool_use_id.clone().or_else(|| r.agent_id.clone());
        let closed: Vec<String> = recs
            .iter()
            .filter(|r| matches!(norm(source, &r.event, r.detail.as_deref()), Some(Ev::Post | Ev::ChildStop)))
            .filter_map(key)
            .collect();
        recs.iter()
            .filter(|r| matches!(norm(source, &r.event, r.detail.as_deref()), Some(Ev::Pre | Ev::ChildStart)))
            .filter(|r| now_ms - r.at_ms < ACTIVE_WINDOW_MS && !key(r).is_some_and(|k| closed.contains(&k)))
            .cloned()
            .collect()
    }

    /// The structure of the run, from receipts: children start and stop, calls open and close.
    /// Content (text, results, output) stays with the files. A `Pre` is printed now; its file row is skipped.
    /// `as_agent`: the receipts were written under that child's own session, so they are the child's whatever
    /// their `agent_id` says.
    fn receipts(&mut self, recs: &[LiveReceipt], as_agent: Option<&str>, out: &mut Vec<String>) -> Result<()> {
        for r in recs {
            let Some(ev) = norm(&self.root.source, &r.event, r.detail.as_deref()) else { continue };
            let call_id = r.tool_use_id.as_deref();
            let agent_id = as_agent.or(r.agent_id.as_deref());
            match ev {
                Ev::ChildStart => {
                    if as_agent.is_none() && agent_id.is_some() {
                        self.agent_at(agent_id, r.at_ms, out);
                    }
                }
                Ev::ChildStop => {
                    let Some(at) = self.sessions.iter().position(|f| f.agent_id.is_some() && f.agent_id.as_deref() == agent_id) else { continue };
                    if self.sessions[at].row.is_none() {
                        self.attach(at, r.detail.as_deref())?;
                    }
                    // The stop receipt says the child is done, whatever its file's freshness says.
                    let f = &mut self.sessions[at];
                    f.stopped = true;
                    if !f.ended {
                        f.ended = true;
                        out.push(if self.json { agent_item("agent_end", &f.id, f.depth, Some(r.at_ms), None, None).to_string() } else { ended_line(f.depth) });
                    }
                }
                Ev::Pre => {
                    let Some(id) = call_id.filter(|id| !self.rcalls.contains_key(*id) && !self.file_calls.contains(*id)) else { continue };
                    let Some(at) = self.agent_at(agent_id, r.at_ms, out) else { continue };
                    // A stopped child that calls again is running again.
                    let f = &mut self.sessions[at];
                    if std::mem::take(&mut f.stopped) && std::mem::take(&mut f.ended) && self.json {
                        out.push(agent_item("agent_resume", &f.id, f.depth, Some(r.at_ms), None, None).to_string());
                    }
                    let (agent, depth) = (self.sessions[at].id.clone(), self.sessions[at].depth);
                    let (tool, what) = (r.tool.as_deref().unwrap_or_default(), r.detail.as_deref().map(first_line).unwrap_or_default());
                    out.push(if self.json {
                        let mut v = item("call", &agent, depth, None, Some(r.at_ms));
                        (v["call_id"], v["tool"], v["what"]) = (json!(id), json!(tool), json!(what));
                        v.to_string()
                    } else {
                        format!("{}→ {tool}  {}", indent(depth), clip(what, CLIP)).trim_end().to_string()
                    });
                    self.rcalls.insert(id.to_string(), RcptCall { agent, depth, at_ms: r.at_ms, done: false, post_ms: None });
                }
                Ev::Post => {
                    let Some(c) = call_id.and_then(|id| self.rcalls.get_mut(id)).filter(|c| !c.done) else { continue };
                    let ms = (r.at_ms - c.at_ms).max(0);
                    c.post_ms = Some(r.at_ms);
                    // Only a success is closed here. A failure closes with a placeholder (the file's result links
                    // in the real error); anything else waits for the file's result. Text has no link: its failures
                    // wait for the file's result, which prints the real error.
                    let Some(ok) = post_outcome(&r.event) else { continue };
                    if !ok && !self.json {
                        continue;
                    }
                    c.done = true;
                    let why = if r.event == "PermissionDenied" { "denied" } else { "failed" };
                    out.push(if self.json {
                        let mut v = item("result", &c.agent, c.depth, None, Some(r.at_ms));
                        (v["call_id"], v["ok"], v["ms"]) = (json!(call_id), json!(ok), json!(ms));
                        if !ok {
                            v["error"] = json!(why);
                        }
                        v.to_string()
                    } else if ok {
                        format!("{}   ✓ {}", indent(c.depth), took(ms))
                    } else {
                        format!("{}   ✗ {why}", indent(c.depth))
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Receipts that grok and Pi write under a child's own session, read with the child's cursor and attributed to
    /// the child. `first`: the follow's first tick, so only what is still in flight is replayed.
    fn own_session_receipts(&mut self, first: Option<i64>, out: &mut Vec<String>) -> Result<()> {
        if !self.children || !child_receipts_own_session(&self.root.source) {
            return Ok(());
        }
        let mut at = 0;
        while at < self.sessions.len() {
            let Some(agent) = self.sessions[at].agent_id.clone() else {
                at += 1;
                continue;
            };
            // Not attached here: the tick's session loop already tried this tick.
            let (row, after) = (self.sessions[at].row.clone(), self.sessions[at].rcpt_after);
            let recs = self.idx.child_receipts_after(&self.root.source, row.as_deref(), &agent, after)?;
            let seen = recs.iter().map(|r| r.id).max().unwrap_or(after);
            self.sessions[at].rcpt_after = if first.is_some() { seen.max(self.rcpt_after) } else { seen };
            let recs = match first {
                Some(now) => self.open_receipts(&recs, now),
                None => recs,
            };
            self.receipts(&recs, Some(&agent), out)?;
            at += 1;
        }
        Ok(())
    }

    /// Duration of the call that `result_id` answers, from the call's row.
    fn call_duration(&mut self, at: usize, result_id: Option<&str>) -> Result<Option<i64>> {
        let Some(line) = result_id.and_then(|id| self.sessions[at].calls.remove(id)) else { return Ok(None) };
        let filter = EventFilter { since_line: Some(line - 1), limit: 1, include_noise: true, ..Default::default() };
        let Some(row) = self.sessions[at].row.clone() else { return Ok(None) };
        Ok(self.idx.list_events(&row, &filter)?.into_iter().find(|e| e.line == line).and_then(|e| e.duration_ms))
    }

    /// [`Self::tick`] as the structured items `--json` prints (see [`json_event`], [`agent_item`], [`model_item`]); needs [`Self::with_json`].
    /// shortcut: parses the JSON `tick` printed instead of threading `Value`s through it; upgrade if the reparse shows up in a profile.
    pub fn tick_items(&mut self, now_ms: i64) -> Result<Vec<Value>> {
        debug_assert!(self.json, "tick_items needs with_json(true)");
        Ok(self.tick(now_ms)?.iter().filter_map(|s| serde_json::from_str(s).ok()).collect())
    }

    /// A row's printed block: its `--json` item, or its text line. A child adopted from its receipt is named by
    /// its agent id (`id`); its lines are read from its row (`rid`).
    fn event_block(&self, ev: &EventRow, raw: Option<&str>, call_id: Option<&str>, id: &str, rid: &str, depth: usize) -> Option<String> {
        if !self.json {
            return format_event(ev, raw, depth);
        }
        json_event(ev, raw, call_id, id, depth).map(|mut v| {
            if rid != id {
                v["session"] = json!(rid);
            }
            v.to_string()
        })
    }

    /// One poll: index the followed files, find new subagents, return everything newly printable.
    pub fn tick(&mut self, now_ms: i64) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let first = !self.started;
        let (mut replay, mut head_len) = (Vec::new(), 0);
        if first {
            self.started = true;
            self.last_activity_ms = now_ms;
            let root = self.root.clone();
            if self.json {
                if let Some(m) = &root.model {
                    out.push(model_item(&root.id, 0, None, root.started_at, m, root.label.as_deref()).to_string());
                }
            } else {
                out.push(header(&root, self.home.as_deref()));
            }
            head_len = out.len();
            self.index(&root.file_path)?;
            self.adopt(&root, 0, Some(self.backlog), None)?;
            // Receipts from before the follow: only what is still in flight (a call or child with no close yet).
            // Children first (their rows come with their files); the calls after the rows, where they belong.
            self.rcpt_after = self.idx.receipts_watermark()?;
            let recs = self.idx.turn_receipts(&root.id)?;
            let (kids, calls): (Vec<_>, Vec<_>) = self.open_receipts(&recs, now_ms).into_iter().partition(|r| matches!(norm(&root.source, &r.event, r.detail.as_deref()), Some(Ev::ChildStart)));
            self.receipts(&kids, None, &mut out)?;
            replay = calls;
        }
        let mut printed = out.len() > head_len;
        let mut at = 0;
        while at < self.sessions.len() {
            let (id, depth) = (self.sessions[at].id.clone(), self.sessions[at].depth);
            if self.sessions[at].row.is_none() {
                self.attach(at, None)?;
                // A child already running when the follow began shows its backlog, not its whole file.
                if first && let Some(rid) = self.sessions[at].row.clone() {
                    (self.sessions[at].last_line, self.sessions[at].brief) = self.resume_after(&rid, Some(self.backlog))?;
                }
            }
            let Some(rid) = self.sessions[at].row.clone() else { at += 1; continue };
            let Some(row) = self.idx.session(&rid)? else { at += 1; continue };
            // A subagent's file is indexed here, so its status is read after, not before (the root's is a tick behind).
            let row = if depth > 0 {
                self.index(&row.file_path)?;
                self.idx.session(&rid)?.unwrap_or(row)
            } else {
                row
            };
            out.extend(self.sessions[at].announce.take());
            // A child born from a receipt had no model or label; send them now that its row is known.
            if !self.sessions[at].meta_sent && let Some(m) = row.model.clone() {
                let f = &mut self.sessions[at];
                (f.meta_sent, f.model, f.label) = (true, Some(m.clone()), row.label.clone());
                out.push(if self.json {
                    model_item(&id, depth, None, row.started_at, &m, row.label.as_deref()).to_string()
                } else {
                    format!("{}↳ {}{}", indent(depth), short_model(&m), row.label.as_deref().map(|l| format!("  {l}")).unwrap_or_default())
                });
            }
            self.index(&row.file_path)?;
            if self.children {
                for kid in self.idx.children(&rid)? {
                    if kid.status != Status::Active || self.sessions.iter().any(|f| f.row.as_deref() == Some(kid.id.as_str())) {
                        continue;
                    }
                    // A child already adopted from its start receipt: bind it to this file instead of adopting twice.
                    let key = child_key(&kid);
                    if let Some(i) = self.sessions.iter().position(|f| f.row.is_none() && f.agent_id.is_some() && f.agent_id == key) {
                        let (last_line, brief) = self.resume_after(&kid.id, first.then_some(self.backlog))?;
                        (self.sessions[i].row, self.sessions[i].last_line, self.sessions[i].brief) = (Some(kid.id.clone()), last_line, brief);
                        continue;
                    }
                    let start = self.start_block(&kid, depth + 1);
                    self.adopt(&kid, depth + 1, first.then_some(self.backlog), Some(start))?;
                }
            }
            // An ended session that is active again resumes before its new rows (a subagent's is silent in text).
            if self.sessions[at].ended && !self.sessions[at].stopped && row.status == Status::Active {
                if self.json {
                    out.push(agent_item("agent_resume", &id, depth, Some(now_ms), None, None).to_string());
                } else if depth == 0 {
                    out.push("↻ resumed".into());
                }
            }
            // The brief the backlog cut off goes out first, as any row would.
            if let Some(ev) = self.sessions[at].brief.take() {
                let raw = self.idx.raw_line(&rid, ev.line).ok().flatten();
                if let Some(block) = self.event_block(&ev, raw.as_deref(), None, &id, &rid, depth) {
                    out.push(block);
                    printed = true;
                }
            }
            let filter = EventFilter { since_line: Some(self.sessions[at].last_line), limit: MAX_EVENTS_PER_FILE, ..Default::default() };
            for mut ev in self.idx.list_events(&rid, &filter)? {
                self.sessions[at].last_line = ev.line;
                let raw = self.idx.raw_line(&rid, ev.line).ok().flatten();
                let ids = matches!(ev.role.as_deref(), Some("tool" | "tool_result"))
                    .then(|| raw.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok()).map(|v| normalize(&v)))
                    .flatten();
                let (call_id, result_id) = ids.map_or((None, None), |n| (n.tool_call_id, n.tool_result_id));
                match ev.role.as_deref() {
                    Some("tool") => {
                        if let Some(call_id) = call_id.clone() {
                            self.file_calls.insert(call_id.clone());
                            self.sessions[at].calls.insert(call_id, ev.line);
                        }
                    }
                    Some("tool_result") if ev.duration_ms.is_none() => {
                        ev.duration_ms = self.call_duration(at, result_id.as_deref())?;
                    }
                    _ => {}
                }
                if self.json && let Some(m) = ev.model.clone().filter(|m| self.sessions[at].model.as_ref() != Some(m)) {
                    out.push(model_item(&id, depth, Some(ev.line), ev.timestamp, &m, self.sessions[at].label.as_deref()).to_string());
                    self.sessions[at].model = Some(m);
                }
                // A call a `Pre` receipt already printed is not printed again; nor its result once a `Post` closed it.
                // What the file adds (where the call and its output are) goes out as a `link` item for the pane.
                let cid = call_id.clone().or_else(|| result_id.clone());
                let (skip, link) = match (ev.role.as_deref(), cid.as_deref().and_then(|i| self.rcalls.get_mut(i))) {
                    (Some("tool"), Some(_)) => (true, Some(("line", None))),
                    (Some("tool_result"), Some(c)) => {
                        ev.duration_ms = ev.duration_ms.or(c.post_ms.map(|p| p - c.at_ms));
                        let was_done = std::mem::replace(&mut c.done, true);
                        let error = ev.is_error.then(|| {
                            let (exit, msg) = error_parts(&ev);
                            [exit, msg].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  ")
                        });
                        (was_done, Some(("result_line", error)))
                    }
                    _ => (false, None),
                };
                let block = if skip {
                    if let (true, Some((key, error))) = (self.json, link) {
                        let mut v = item("link", &id, depth, None, ev.timestamp);
                        (v["call_id"], v["session"], v[key]) = (json!(cid), json!(rid), json!(ev.line));
                        if let Some(e) = error {
                            v["error"] = json!(e);
                        }
                        Some(v.to_string())
                    } else {
                        None
                    }
                } else {
                    self.event_block(&ev, raw.as_deref(), call_id.or(result_id).as_deref(), &id, &rid, depth)
                };
                if let Some(block) = block {
                    out.push(block);
                    printed = true;
                }
            }
            let live = row.status == Status::Active && !self.sessions[at].stopped;
            let f = &mut self.sessions[at];
            if live {
                f.ended = false;
            } else if !f.ended {
                f.ended = true;
                if !self.json {
                    out.push(ended_line(depth));
                } else {
                    out.push(agent_item("agent_end", &id, depth, Some(now_ms), None, None).to_string());
                }
            }
            at += 1;
        }
        // Receipts after the files' rows, so a call never prints above the text before it and a stop never above
        // the child's last rows. A call whose file row came first in this tick is skipped (`file_calls`).
        let before = out.len();
        if first {
            // Calls still open at the follow's start, after the backlog rows.
            self.receipts(&replay, None, &mut out)?;
            self.own_session_receipts(Some(now_ms), &mut out)?;
        } else {
            let recs = self.idx.receipts_after(&self.root.id, self.rcpt_after)?;
            self.rcpt_after = recs.iter().map(|r| r.id).fold(self.rcpt_after, i64::max);
            self.receipts(&recs, None, &mut out)?;
            self.own_session_receipts(None, &mut out)?;
        }
        printed |= out.len() > before;
        if printed {
            self.last_activity_ms = now_ms;
            self.next_idle_s = IDLE_FIRST_S;
        } else if !self.json && (now_ms - self.last_activity_ms) / 1000 >= self.next_idle_s as i64 {
            out.push(idle_line(self.next_idle_s));
            self.next_idle_s += IDLE_EVERY_S;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, role: Option<&str>, title: &str, preview: &str) -> EventRow {
        EventRow {
            line: 1,
            event_type: String::new(),
            kind: kind.into(),
            role: role.map(Into::into),
            title: title.into(),
            preview: preview.into(),
            timestamp: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: None,
            step: None,
            depth: 0,
            model: None,
            is_error: false,
            noise: false,
            detail: None,
            intent: None,
        }
    }

    fn user(text: &str) -> EventRow {
        ev("message", Some("user"), "User", text)
    }

    #[test]
    fn post_outcome_closes_only_what_says_success_or_failure() {
        assert_eq!(post_outcome("PostToolUse"), Some(true));
        assert_eq!(post_outcome("postToolUse"), Some(true));
        assert_eq!((post_outcome("PostToolUseFailure"), post_outcome("postToolUseFailure"), post_outcome("PermissionDenied")), (Some(false), Some(false), Some(false)));
        // Pi's end event and Cursor's `after*` do not say whether the call worked: the file's result does.
        assert_eq!((post_outcome("tool_execution_end"), post_outcome("afterShellExecution"), post_outcome("afterMCPExecution")), (None, None, None));
    }

    #[test]
    fn a_child_is_known_by_the_id_its_receipts_carry() {
        let row = |source: &str, path: &str, native: Option<&str>| SessionRow { source: source.into(), file_path: path.into(), native_id: native.map(String::from), ..Default::default() };
        assert_eq!(child_key(&row("claude", "/p/-w/s/subagents/agent-a1.jsonl", Some("-w/s/subagents/agent-a1"))).as_deref(), Some("a1"));
        assert_eq!(child_key(&row("grok", "/g/sessions/%2Fw/k1/chat_history.jsonl", Some("k1"))).as_deref(), Some("k1"));
        assert_eq!(child_key(&row("pi", "/pi/-w/s/Name.jsonl", Some("-w/2026-10-01T00-00-00_abc123"))).as_deref(), Some("abc123"));
        assert_eq!(child_key(&row("cursor", "/c/t/x/subagents/kid.jsonl", Some("kid"))).as_deref(), Some("kid"));
        assert!(child_key(&row("grok", "/g/x/chat_history.jsonl", None)).is_none());
    }

    #[test]
    fn header_line() {
        let s = SessionRow {
            source: "claude".into(),
            model: Some("claude-sonnet-5-5".into()),
            title: "Fix the build\nsecond line".into(),
            cwd: Some("/Users/me/repos/x".into()),
            ..Default::default()
        };
        assert_eq!(header(&s, Some("/Users/me")), "claude · sonnet-5-5 · Fix the build · ~/repos/x");
        assert_eq!(header(&s, Some("/Users/m")), "claude · sonnet-5-5 · Fix the build · /Users/me/repos/x");
        let s = SessionRow { source: "pi".into(), title: "t".into(), ..Default::default() };
        assert_eq!(header(&s, None), "pi · - · t · -");
        assert_eq!(short_model("claude-haiku-4-5-20251001"), "haiku-4-5");
        assert_eq!(short_model("gpt-5"), "gpt-5");
    }

    #[test]
    fn user_message() {
        assert_eq!(format_event(&user("fix it"), None, 0).as_deref(), Some("\n▶ you  fix it"));
        let raw = r#"{"type":"user","message":{"role":"user","content":"first\nsecond\n\nthird"}}"#;
        assert_eq!(format_event(&user("first second third"), Some(raw), 1).as_deref(), Some("\n  ▶ you  first (+2 lines)"));
        let long = "x".repeat(150);
        let want = format!("\n▶ you  {}…", "x".repeat(120));
        assert_eq!(format_event(&user(&long), None, 0), Some(want));
    }

    #[test]
    fn assistant_text_wraps_and_is_never_clipped() {
        let text = (1..=60).map(|i| format!("w{i:02}")).collect::<Vec<_>>().join(" ");
        let raw = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":text}]}}).to_string();
        let out = format_event(&ev("message", Some("assistant"), "Assistant", "w01 w02…"), Some(&raw), 0).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("·  w01 w02 w03 "));
        assert!(lines[1..].iter().all(|l| l.starts_with("   w")));
        assert!(lines.iter().all(|l| l.chars().count() <= 100));
        let words: Vec<&str> = out.split_whitespace().skip(1).collect();
        assert_eq!(words.len(), 60, "every word kept");
        assert_eq!(words[59], "w60");
        let nested = format_event(&ev("message", Some("assistant"), "Assistant", "hi"), Some(r#"{"message":{"content":"a\n\nb"}}"#), 2).unwrap();
        assert_eq!(nested, "    ·  a\n       b");
    }

    #[test]
    fn reasoning_uses_tilde() {
        let raw = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"consider the cache"}]}}"#;
        let r = ev("reasoning", Some("reasoning"), "Reasoning", "consider the…");
        assert_eq!(format_event(&r, Some(raw), 0).as_deref(), Some("~  consider the cache"));
        assert_eq!(format_event(&r, None, 1).as_deref(), Some("  ~  consider the…"));
    }

    #[test]
    fn tool_call_prefers_intent_then_detail_then_preview() {
        let mut c = ev("tool", Some("tool"), "Bash", "cargo test");
        c.detail = Some("cargo test -p x".into());
        c.intent = Some("run the crate tests".into());
        assert_eq!(format_event(&c, None, 0).as_deref(), Some("→ Bash  run the crate tests"));
        c.intent = None;
        assert_eq!(format_event(&c, None, 1).as_deref(), Some("  → Bash  cargo test -p x"));
        c.detail = None;
        assert_eq!(format_event(&c, None, 0).as_deref(), Some("→ Bash  cargo test"));
        c.preview = "Bash".into();
        assert_eq!(format_event(&c, None, 0).as_deref(), Some("→ Bash"));
    }

    #[test]
    fn tool_result_ok_durations() {
        let mut r = ev("tool", Some("tool_result"), "Tool Result", "ok");
        assert_eq!(format_event(&r, None, 0).as_deref(), Some("   ✓"));
        for (ms, want) in [(400, "   ✓ 0.4s"), (12_000, "   ✓ 12s"), (125_000, "   ✓ 2m05s")] {
            r.duration_ms = Some(ms);
            assert_eq!(format_event(&r, None, 0).as_deref(), Some(want));
        }
        assert_eq!(format_event(&r, None, 1).as_deref(), Some("     ✓ 2m05s"));
    }

    #[test]
    fn tool_result_error() {
        let mut r = ev("tool", Some("tool_result"), "Tool Result", "error: boom\nmore");
        r.is_error = true;
        r.detail = Some("error: boom".into());
        assert_eq!(format_event(&r, None, 0).as_deref(), Some("   ✗ error: boom"));
        r.detail = Some("exit 2".into());
        r.preview = "error: no such file".into();
        assert_eq!(format_event(&r, None, 0).as_deref(), Some("   ✗ exit 2  error: no such file"));
        r.detail = None;
        r.preview = "y".repeat(130);
        assert_eq!(format_event(&r, None, 1).as_deref(), Some(format!("     ✗ {}…", "y".repeat(120)).as_str()));
    }

    #[test]
    fn hidden_rows() {
        let mut n = user("x");
        n.noise = true;
        assert_eq!(format_event(&n, None, 0), None);
        for (k, r) in [("system", None), ("turn", None), ("telemetry", None), ("message", Some("system"))] {
            assert_eq!(format_event(&ev(k, r, "T", "p"), None, 0), None, "{k}");
        }
        assert_eq!(format_event(&ev("message", Some("user"), "User", "User"), None, 0), None, "empty prompt");
    }

    fn js(v: Option<Value>) -> String {
        v.unwrap().to_string()
    }

    #[test]
    fn json_user_text_reasoning() {
        let mut u = user("fix it");
        u.timestamp = Some(1000);
        let raw = r#"{"message":{"role":"user","content":"first\nsecond"}}"#;
        assert_eq!(js(json_event(&u, Some(raw), None, "a1", 0)), r#"{"v":1,"kind":"user","agent":"a1","depth":0,"line":1,"ts":1000,"text":"first\nsecond"}"#);
        let a = ev("message", Some("assistant"), "Assistant", "hi");
        assert_eq!(js(json_event(&a, None, None, "a1", 1)), r#"{"v":1,"kind":"text","agent":"a1","depth":1,"line":1,"ts":null,"text":"hi"}"#);
        let r = ev("reasoning", Some("reasoning"), "Reasoning", "hmm");
        assert_eq!(js(json_event(&r, None, None, "a1", 0)), r#"{"v":1,"kind":"reasoning","agent":"a1","depth":0,"line":1,"ts":null,"text":"hmm"}"#);
        assert_eq!(json_event(&user("User"), None, None, "a1", 0), None, "empty prompt is hidden, as in text");
    }

    #[test]
    fn json_call_and_result() {
        let mut c = ev("tool", Some("tool"), "Bash", "cargo test");
        c.detail = Some("cargo test -p x".into());
        assert_eq!(js(json_event(&c, None, Some("t1"), "a", 0)), r#"{"v":1,"kind":"call","agent":"a","depth":0,"line":1,"ts":null,"call_id":"t1","tool":"Bash","what":"cargo test -p x"}"#);
        c.intent = Some("run the tests".into());
        assert!(js(json_event(&c, None, Some("t1"), "a", 0)).ends_with(r#""what":"run the tests"}"#));
        c.intent = None;
        c.detail = None;
        assert!(js(json_event(&c, None, None, "a", 0)).ends_with(r#""call_id":null,"tool":"Bash","what":""}"#));
        let mut r = ev("tool", Some("tool_result"), "Tool Result", "ok");
        assert_eq!(js(json_event(&r, None, Some("t1"), "a", 0)), r#"{"v":1,"kind":"result","agent":"a","depth":0,"line":1,"ts":null,"call_id":"t1","ok":true}"#);
        r.is_error = true;
        r.detail = Some("exit 2".into());
        r.preview = "error: no such file\nmore".into();
        assert_eq!(js(json_event(&r, None, Some("t1"), "a", 0)), r#"{"v":1,"kind":"result","agent":"a","depth":0,"line":1,"ts":null,"call_id":"t1","ok":false,"error":"exit 2  error: no such file"}"#);
    }

    #[test]
    fn json_agent_and_model_items() {
        assert_eq!(
            agent_item("agent_start", "k", 1, Some(5), Some("claude-haiku-4-5"), Some("Check")).to_string(),
            r#"{"v":1,"kind":"agent_start","agent":"k","depth":1,"line":null,"ts":5,"model":"claude-haiku-4-5","label":"Check"}"#
        );
        assert_eq!(agent_item("agent_end", "k", 1, Some(6), None, None).to_string(), r#"{"v":1,"kind":"agent_end","agent":"k","depth":1,"line":null,"ts":6}"#);
        assert_eq!(agent_item("agent_resume", "k", 1, Some(7), None, None).to_string(), r#"{"v":1,"kind":"agent_resume","agent":"k","depth":1,"line":null,"ts":7}"#);
        assert_eq!(
            model_item("k", 1, Some(3), None, "gpt-5", None).to_string(),
            r#"{"v":1,"kind":"model","agent":"k","depth":1,"line":3,"ts":null,"model":"gpt-5","label":null}"#
        );
    }

    #[test]
    fn state_lines() {
        assert_eq!(ended_line(0), "■ ended");
        assert_eq!(ended_line(2), "    ↳ ended");
        assert_eq!(started_line(Some("claude-sonnet-5-5"), 1), "  ↳ sonnet-5-5 started");
        assert_eq!(started_line(None, 1), "  ↳ started");
        assert_eq!(idle_line(30), "… idle 30s");
        assert_eq!(idle_line(90), "… idle 1m30s");
        assert_eq!(idle_line(125), "… idle 2m05s");
    }
}
