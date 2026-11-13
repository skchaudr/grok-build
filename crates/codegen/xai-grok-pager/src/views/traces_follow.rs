//! `/traces` follow screen: one live session (and its subagents) as a tail you can pause, filter and read.
//!
//! A `Follower` (the library behind `grok-traces follow`) runs on a worker thread and is ticked from the
//! modal's 1 s poll; its `--json` items become the [`Row`]s here. Nothing here touches the index except the
//! raw-line reads that fill the selected call's body.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::follow::{Follower, short_model};
use xai_grok_traces::{SessionRow, TraceIndex, user_roots};

use crate::render::color::blend_color;
use crate::render::line_utils::{truncate_line, truncate_str};
use crate::scrollback::blocks::markdown_content::MarkdownContent;
use crate::theme::Theme;
use crate::views::modal_window;
use crate::views::traces_body::{self, INLINE_MAX_LINES};
use crate::views::traces_modal::{compose_body, continuation, continuation_source, norm, split_result};

/// Rows replayed when the screen opens (lookout's respawn replays the same).
const BACKLOG: usize = 200;
/// Reasoning is dimmed context, not the thing being followed: this many lines, then `… +N lines`.
const REASONING_LINES: usize = 6;
const MAX_INDENT: usize = 8;
/// A prompt shows this many wrapped lines, then `… +N lines`.
const USER_LINES: usize = 3;
/// An agent message shows this many lines until `l` opens it (to `INLINE_MAX_LINES`).
const TEXT_LINES: usize = 6;
/// How far a finished call's diamond is dimmed toward the background, as grok's collapsed tool rows do
/// (`scrollback.display.dim_accent` default).
const DIM_ACCENT: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Res {
    pub ok: bool,
    pub error: String,
    pub ms: Option<i64>,
    pub line: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Call {
    pub line: i64,
    pub call_id: Option<String>,
    pub tool: String,
    pub what: String,
    /// When the call started (item time, else arrival): the in-flight timer counts from here.
    pub started_ms: i64,
    pub result: Option<Res>,
    /// Its agent ended with the call still open.
    pub no_result: bool,
    /// The session whose file holds `line`, when it is not the agent's own name (a child born from a receipt).
    pub session: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Body {
    User(String),
    Text(String),
    Reasoning(String),
    Call(Call),
    Start { model: Option<String>, label: Option<String> },
    End,
    Resume,
    Model(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub agent: String,
    pub depth: usize,
    pub body: Body,
    /// The item's time, ms; prompts and agent text show it the way grok's view does.
    pub ts: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Agent {
    pub id: String,
    pub model: Option<String>,
    pub label: Option<String>,
    /// The `agent_start` item's time, ms.
    pub started: Option<i64>,
    pub ended: bool,
    /// The `agent_end` item's time, ms.
    pub end_ms: Option<i64>,
}

impl Agent {
    fn name(&self, root: bool) -> String {
        match (&self.label, &self.model) {
            _ if root => "root".to_string(),
            (Some(l), _) if !l.is_empty() => l.clone(),
            (_, Some(m)) => short_model(m),
            _ => self.id.clone(),
        }
    }
}

type Tick = Result<Vec<Value>, String>;

pub(super) struct Follow {
    pub root: SessionRow,
    rows: Vec<Row>,
    /// Root first, then subagents in order of appearance.
    agents: Vec<Agent>,
    /// 0 = every agent; n = `agents[n - 1]` alone.
    filter: usize,
    /// The highlighted row (index into `rows`) while paused; `None` pins the tail.
    cursor: Option<usize>,
    /// `rows.len()` when the tail was last pinned, for the `N new` count.
    paused_len: usize,
    /// First drawn row, as a position in the visible rows.
    top: usize,
    ended: bool,
    connected: bool,
    err: Option<String>,
    tx: mpsc::Sender<i64>,
    rx: mpsc::Receiver<Tick>,
    inflight: bool,
    /// Composed bodies by (agent, call line, result line); only complete reads are kept.
    bodies: HashMap<(String, i64, Option<i64>), String>,
    /// Call rows (indices into `rows`) shown with their body inline: `l` opens, `h` closes.
    open: HashSet<usize>,
    /// Text and reasoning rows rendered as markdown by grok's own renderer, wrap cached per width.
    md: HashMap<usize, MarkdownContent>,
    /// A line for the status row (`no transcript yet`, `spawn row not loaded`); cleared by the next key.
    pub(super) note: Option<String>,
    /// (label, start time) of the subagent this follow is of, to put the cursor on the `Agent` row that spawned it
    /// once the first rows arrive.
    pub(super) want: Option<(String, Option<i64>)>,
}

/// `0.4s`, `12s`, `2m05s`.
fn took(ms: i64) -> String {
    if ms < 10_000 { format!("{:.1}s", ms as f64 / 1000.0) } else { span_text(ms / 1000) }
}

fn span_text(s: i64) -> String {
    let s = s.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, s / 60 % 60)
    }
}

fn first_line(s: &str) -> &str {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

impl Follow {
    /// Follows `root`, ticking a `Follower` on a worker thread over its own handle on the index at `db`.
    pub fn open(db: PathBuf, root: SessionRow) -> Self {
        let (tx, tick_rx) = mpsc::channel::<i64>();
        let (out_tx, rx) = mpsc::channel::<Tick>();
        let seed = root.clone();
        std::thread::spawn(move || {
            let mut follower = match TraceIndex::open_with(&db, Some(std::time::Duration::from_secs(2))) {
                Ok(idx) => Follower::new(idx, user_roots(), seed, true, None).with_json(true).with_backlog(BACKLOG),
                Err(e) => {
                    let _ = out_tx.send(Err(e.to_string()));
                    return;
                }
            };
            while let Ok(now) = tick_rx.recv() {
                if out_tx.send(follower.tick_items(now).map_err(|e| e.to_string())).is_err() {
                    break;
                }
            }
        });
        let mut f = Self::bare(root);
        f.tx = tx;
        f.rx = rx;
        // The first tick (the backlog) goes out now rather than at the next poll.
        f.inflight = f.tx.send(now_ms()).is_ok();
        f
    }

    /// A screen with no worker behind it.
    pub fn bare(root: SessionRow) -> Self {
        let (tx, _) = mpsc::channel();
        let (_, rx) = mpsc::channel();
        Self {
            agents: vec![Agent { id: root.id.clone(), model: root.model.clone(), label: None, started: None, ended: false, end_ms: None }],
            root,
            rows: Vec::new(),
            filter: 0,
            cursor: None,
            paused_len: 0,
            top: 0,
            ended: false,
            connected: false,
            err: None,
            tx,
            rx,
            inflight: false,
            bodies: HashMap::new(),
            open: HashSet::new(),
            md: HashMap::new(),
            note: None,
            want: None,
        }
    }

    /// The modal's 1 s poll: take what the worker found, ask for the next tick. True when the screen changed
    /// (new rows, or an open call's timer moved).
    pub fn poll(&mut self, now_ms: i64) -> bool {
        let mut redraw = false;
        while let Ok(tick) = self.rx.try_recv() {
            self.inflight = false;
            redraw |= !self.connected;
            self.connected = true;
            match tick {
                Ok(items) => {
                    redraw |= self.err.take().is_some() || !items.is_empty();
                    self.apply(&items, now_ms);
                    redraw |= self.want.is_some();
                    self.resolve_want();
                }
                // A busy index is transient; the next tick retries.
                Err(e) => redraw |= self.err.replace(e.clone()).as_deref() != Some(e.as_str()),
            }
        }
        if !self.inflight {
            // shortcut: the tick rides the modal's 1 s poll; the `traces-wake` socket would wake it here.
            self.inflight = self.tx.send(now_ms).is_ok();
        }
        redraw || self.has_open_call()
    }

    /// A call with no result, or a subagent still running (its `Agent` row counts up).
    fn has_open_call(&self) -> bool {
        !self.ended
            && (self.rows.iter().rev().take(64).any(|r| matches!(&r.body, Body::Call(c) if c.result.is_none() && !c.no_result))
                || self.agents.iter().skip(1).any(|a| !a.ended))
    }

    fn agent_mut(&mut self, id: &str) -> &mut Agent {
        let at = self.agents.iter().position(|a| a.id == id).unwrap_or_else(|| {
            self.agents.push(Agent { id: id.to_string(), model: None, label: None, started: None, ended: false, end_ms: None });
            self.agents.len() - 1
        });
        &mut self.agents[at]
    }

    /// Items to rows. A result merges into its call (one whose call is not shown is dropped); an agent's end
    /// marks its still-open calls; a `model` item is a row only when the agent already had a model.
    /// shortcut: rows keep arrival order; lookout also sorts by timestamp, which only matters across agents.
    pub fn apply(&mut self, items: &[Value], now_ms: i64) {
        let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
        let mut last_ts = now_ms;
        for v in items {
            let (agent, depth) = (text(v, "agent"), v["depth"].as_u64().unwrap_or(0) as usize);
            let is_root = agent == self.root.id;
            self.agent_mut(&agent);
            if let Some(ts) = v["ts"].as_i64() {
                last_ts = ts;
            }
            let body = match v["kind"].as_str().unwrap_or_default() {
                "user" => Body::User(text(v, "text")),
                "text" => Body::Text(text(v, "text")),
                "reasoning" => Body::Reasoning(text(v, "text")),
                "call" => Body::Call(Call {
                    line: v["line"].as_i64().unwrap_or(-1),
                    call_id: v["call_id"].as_str().map(str::to_string),
                    tool: text(v, "tool"),
                    what: text(v, "what"),
                    started_ms: last_ts,
                    result: None,
                    no_result: false,
                    session: v["session"].as_str().map(str::to_string),
                }),
                // The transcript caught up with a call a receipt opened: where its command and output are.
                "link" => {
                    let id = v["call_id"].as_str();
                    let at = self.rows.iter().rposition(|r| r.agent == agent && matches!(&r.body, Body::Call(c) if c.call_id.as_deref() == id));
                    if let Some(Body::Call(c)) = at.map(|i| &mut self.rows[i].body) {
                        if let Some(l) = v["line"].as_i64() {
                            c.line = l;
                        }
                        c.session = v["session"].as_str().map(str::to_string).or(c.session.take());
                        if let (Some(l), Some(r)) = (v["result_line"].as_i64(), c.result.as_mut()) {
                            r.line = l;
                            if !r.ok && let Some(e) = v["error"].as_str() {
                                r.error = e.to_string();
                            }
                        }
                    }
                    continue;
                }
                "result" => {
                    let id = v["call_id"].as_str();
                    // A result with no id (some harnesses omit it) answers the agent's oldest open call without one.
                    let open = |r: &Row| matches!(&r.body, Body::Call(c) if c.result.is_none() && c.call_id.as_deref() == id) && r.agent == agent;
                    let at = if id.is_some() { self.rows.iter().rposition(open) } else { self.rows.iter().position(open) };
                    let call = at.and_then(|i| match &mut self.rows[i].body {
                        Body::Call(c) => Some(c),
                        _ => None,
                    });
                    if let Some(c) = call {
                        c.result = Some(Res {
                            ok: v["ok"].as_bool().unwrap_or(true),
                            error: text(v, "error"),
                            ms: v["ms"].as_i64(),
                            line: v["line"].as_i64().unwrap_or(-1),
                        });
                    }
                    continue;
                }
                "agent_start" => {
                    let a = self.agent_mut(&agent);
                    a.model = v["model"].as_str().map(str::to_string);
                    a.label = v["label"].as_str().map(str::to_string);
                    a.started = v["ts"].as_i64();
                    Body::Start { model: a.model.clone(), label: a.label.clone() }
                }
                "agent_end" => {
                    let a = self.agent_mut(&agent);
                    (a.ended, a.end_ms) = (true, v["ts"].as_i64());
                    for r in self.rows.iter_mut().filter(|r| r.agent == agent) {
                        if let Body::Call(c) = &mut r.body {
                            c.no_result |= c.result.is_none();
                        }
                    }
                    self.ended |= is_root;
                    Body::End
                }
                "agent_resume" => {
                    self.agent_mut(&agent).ended = false;
                    self.ended &= !is_root;
                    Body::Resume
                }
                "model" => {
                    let model = text(v, "model");
                    let label = v["label"].as_str().filter(|l| !l.is_empty()).map(str::to_string);
                    // A child started from a receipt has a start row with no model yet: name it now.
                    let start = self.rows.iter_mut().rev().find(|r| r.agent == agent && matches!(r.body, Body::Start { .. }));
                    if let Some(Body::Start { model: m, label: l }) = start.map(|r| &mut r.body)
                        && m.is_none()
                    {
                        *m = Some(model.clone());
                        if label.is_some() {
                            l.clone_from(&label);
                        }
                    }
                    let a = self.agent_mut(&agent);
                    if let Some(l) = label {
                        a.label = Some(l);
                    }
                    let had = a.model.replace(model.clone());
                    if had.is_none_or(|m| m == model) {
                        continue;
                    }
                    Body::Model(model)
                }
                _ => continue,
            };
            if let Body::Text(t) | Body::Reasoning(t) = &body {
                self.md.insert(self.rows.len(), MarkdownContent::new(t.as_str()));
            }
            self.rows.push(Row { agent, depth, body, ts: v["ts"].as_i64() });
        }
    }

    /// Indices of the rows the current agent filter shows.
    pub fn visible(&self) -> Vec<usize> {
        let only = self.filter.checked_sub(1).and_then(|i| self.agents.get(i)).map(|a| a.id.as_str());
        (0..self.rows.len()).filter(|&i| only.is_none_or(|id| self.rows[i].agent == id)).collect()
    }

    pub fn is_live(&self) -> bool {
        !self.ended
    }

    pub fn paused(&self) -> bool {
        self.cursor.is_some()
    }

    /// The highlighted row's call (with the session id its lines are read from), when it is one.
    pub fn selected_call(&self) -> Option<(&str, &Call)> {
        self.call_at(self.cursor?)
    }

    /// The highlighted row's prompt text and time, when it is one.
    pub fn selected_prompt(&self) -> Option<(&str, Option<i64>)> {
        let row = self.rows.get(self.cursor?)?;
        match &row.body {
            Body::User(t) => Some((t.as_str(), row.ts)),
            _ => None,
        }
    }

    fn call_at(&self, i: usize) -> Option<(&str, &Call)> {
        let row = self.rows.get(i)?;
        match &row.body {
            Body::Call(c) => Some((c.session.as_deref().unwrap_or(row.agent.as_str()), c)),
            _ => None,
        }
    }

    /// Position in `vis` of the highlighted row, or of the tail when pinned.
    fn at(&self, vis: &[usize]) -> Option<usize> {
        self.cursor.and_then(|c| vis.iter().position(|&i| i == c)).or_else(|| vis.len().checked_sub(1))
    }

    fn pause_at(&mut self, vis: &[usize], pos: usize) {
        if self.cursor.is_none() {
            self.paused_len = self.rows.len();
        }
        self.cursor = vis.get(pos).copied();
    }

    /// Moves the highlighted row by `delta` visible rows; the first move pauses the tail.
    pub fn step(&mut self, delta: isize) -> bool {
        let vis = self.visible();
        let Some(at) = self.at(&vis) else { return false };
        let to = (at as isize + delta).clamp(0, vis.len() as isize - 1) as usize;
        self.pause_at(&vis, to);
        true
    }

    /// `G`: pin the tail again.
    pub fn resume(&mut self) -> bool {
        self.top = 0;
        self.cursor.take().is_some()
    }

    pub fn first(&mut self) -> bool {
        let vis = self.visible();
        self.pause_at(&vis, 0);
        !vis.is_empty()
    }

    /// `e`: the next failed call after the highlighted row (or the tail), wrapping.
    pub fn next_error(&mut self) -> bool {
        let vis = self.visible();
        let Some(at) = self.at(&vis) else { return false };
        let bad = |i: usize| matches!(&self.rows[vis[i]].body, Body::Call(c) if c.result.as_ref().is_some_and(|r| !r.ok));
        match (at + 1..vis.len()).chain(0..=at).find(|&i| bad(i)) {
            Some(i) => {
                self.pause_at(&vis, i);
                true
            }
            None => false,
        }
    }

    /// `Tab`: every agent, then each agent alone (root first), then every agent again.
    pub fn cycle_agent(&mut self, back: bool) -> bool {
        let n = self.agents.len() + 1;
        if n <= 2 {
            return false;
        }
        self.filter = if back { (self.filter + n - 1) % n } else { (self.filter + 1) % n };
        self.cursor = None;
        self.top = 0;
        true
    }

    fn filter_name(&self) -> Option<String> {
        let i = self.filter.checked_sub(1)?;
        Some(self.agents.get(i)?.name(i == 0))
    }

    /// Basic key handling; Enter, Esc, `o`, `f` and `p` are the modal's.
    pub fn key(&mut self, code: KeyCode) -> bool {
        let cleared = self.note.take().is_some();
        let changed = match code {
            KeyCode::Char('j') | KeyCode::Down => self.step(1),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1),
            KeyCode::PageDown => self.step(10),
            KeyCode::PageUp => self.step(-10),
            KeyCode::Char('g') | KeyCode::Home => self.first(),
            KeyCode::Char('G') | KeyCode::End => self.resume(),
            KeyCode::Tab => self.cycle_agent(false),
            KeyCode::BackTab => self.cycle_agent(true),
            KeyCode::Char('e') => self.next_error(),
            KeyCode::Char('l') | KeyCode::Right => self.fold(true),
            KeyCode::Char('h') | KeyCode::Left => self.fold(false),
            _ => false,
        };
        changed || cleared
    }

    /// The subagent `f` opens: the highlighted row's agent when it is not the root, else the child a highlighted
    /// `Agent` call dispatched. `None` while the tail is pinned (nothing is highlighted) and on a root row.
    pub fn cursor_agent(&self) -> Option<&str> {
        let i = self.cursor?;
        let row = self.rows.get(i)?;
        if row.agent != self.root.id {
            return Some(row.agent.as_str());
        }
        Some(self.agents[*self.links().get(&i)?].id.as_str())
    }

    /// The session whose file holds one of agent `id`'s calls, for a child born from a receipt (no row under its own id).
    pub fn session_of(&self, id: &str) -> Option<String> {
        self.rows.iter().filter(|r| r.agent == id).find_map(|r| match &r.body {
            Body::Call(c) => c.session.clone(),
            _ => None,
        })
    }

    /// The `Agent`/`Task` call row that dispatched a subagent called `label` and started at `started`: its label
    /// matches the call's target (equal or a prefix either way), the call is no later than 5 s after the start,
    /// and an exact match, then the latest, wins. The label match is the same the trajectory screen uses; no item carries an id.
    fn spawn_row(&self, label: &str, started: Option<i64>) -> Option<usize> {
        let want = norm(label);
        if want.is_empty() {
            return None;
        }
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match &r.body {
                Body::Call(c) if matches!(c.tool.to_ascii_lowercase().as_str(), "agent" | "task") => Some((i, c)),
                _ => None,
            })
            .filter(|(_, c)| {
                let what = norm(&c.what);
                !what.is_empty() && (what == want || what.starts_with(&want) || want.starts_with(&what)) && started.is_none_or(|s| c.started_ms <= s + 5_000)
            })
            .max_by_key(|(_, c)| (norm(&c.what) == want, c.started_ms))
            .map(|(i, _)| i)
    }

    /// Row index to agent index for every dispatch call a labelled subagent matches. A later agent wins a shared call.
    fn links(&self) -> HashMap<usize, usize> {
        let mut out = HashMap::new();
        for (at, a) in self.agents.iter().enumerate().skip(1) {
            if let Some(i) = a.label.as_deref().and_then(|l| self.spawn_row(l, a.started)) {
                out.insert(i, at);
            }
        }
        out
    }

    /// After `p`: put the cursor on the `Agent` row that spawned the subagent this screen replaced, or say it is not loaded.
    fn resolve_want(&mut self) {
        let Some((label, started)) = self.want.take() else { return };
        match self.spawn_row(&label, started) {
            Some(i) => {
                let vis = self.visible();
                if let Some(pos) = vis.iter().position(|&v| v == i) {
                    self.pause_at(&vis, pos);
                }
            }
            None => self.note = Some("spawn row not loaded".to_string()),
        }
    }

    /// `l` / `h`: open or close the highlighted call's body, prompt, message or reasoning inline. False when nothing changed.
    pub fn fold(&mut self, open: bool) -> bool {
        let foldable = |i: usize| matches!(self.rows[i].body, Body::Call(_) | Body::Text(_) | Body::Reasoning(_) | Body::User(_));
        let Some(i) = self.cursor.filter(|&i| foldable(i)) else { return false };
        if open { self.open.insert(i) } else { self.open.remove(&i) }
    }

    // ------------------------------------------------------------------ render

    /// Row `i`'s inline call body: command, separator, result. Cached once complete.
    fn body_rows(&mut self, i: usize, index: Option<&TraceIndex>, width: usize) -> Option<Vec<String>> {
        let (agent, c) = self.call_at(i)?;
        let (agent, c) = (agent.to_string(), c.clone());
        if c.line < 0 {
            // Opened by a receipt; the transcript has not reached it yet. The row above is its command.
            return Some(traces_body::note("(waiting for the transcript)", width));
        }
        let key = (agent.clone(), c.line, c.result.as_ref().map(|r| r.line));
        let text = match self.bodies.get(&key) {
            Some(t) => t.clone(),
            None => {
                let index = index?;
                let fetch = |l: i64| index.raw_line(&agent, l).map_err(|e| e.to_string());
                match compose_body(fetch(c.line), c.result.as_ref().filter(|r| r.line >= 0).map(|r| (fetch(r.line), !r.ok))) {
                    Ok(t) => {
                        self.bodies.insert(key, t.clone());
                        t
                    }
                    Err(e) => return Some(traces_body::note(&format!("(could not read #{}: {e})", c.line), width)),
                }
            }
        };
        let (own, result) = split_result(&text);
        let rest = continuation(&c.what, &continuation_source(own));
        Some(traces_body::call_rows(&rest, traces_body::section(result), width))
    }

    /// Subagent `at`'s name for its tag: `Agent::name` minus the leading words it shares with another subagent's.
    fn tag_name(&self, at: usize) -> String {
        let name = self.agents[at].name(false);
        let words: Vec<&str> = name.split_whitespace().collect();
        let shared = (1..self.agents.len())
            .filter(|&j| j != at)
            .map(|j| {
                let other = self.agents[j].name(false);
                words.iter().copied().zip(other.split_whitespace()).take_while(|&(a, b)| a == b).count()
            })
            .max()
            .unwrap_or(0);
        if shared == 0 || shared >= words.len() { name } else { words[shared..].join(" ") }
    }

    /// Subagent `at`'s tag as `(head, model)`: `↳ 1 UI/UX lens` and its short model id. An unlabelled agent's name
    /// already is its model, and an agent with no model yet has none, so `model` is then empty.
    fn tag_text(&self, at: usize) -> (String, String) {
        let a = &self.agents[at];
        let labelled = a.label.as_deref().is_some_and(|l| !l.is_empty());
        let model = a.model.as_deref().filter(|_| labelled).map(short_model).unwrap_or_default();
        (format!("↳ {at} {}", self.tag_name(at)), model)
    }

    /// The tag column: the widest full tag of any subagent (not only the visible ones, so it holds still while Tab
    /// filters) plus 2, at most a third of the screen.
    fn tag_w(&self, width: usize) -> usize {
        let widest = (1..self.agents.len())
            .map(|at| {
                let (head, model) = self.tag_text(at);
                head.width() + if model.is_empty() { 0 } else { 3 + model.width() }
            })
            .max()
            .unwrap_or(0);
        (widest + 2).clamp(6, (width / 3).max(6))
    }

    /// Subagent `at`'s tag fitted to the column, leaving 2 spaces before the row: `(head, tail)` as `↳ 2 name` and
    /// ` · model`; else the model dropped (`↳ 2 name`), the name cut with `…` if it is still too long.
    fn tag_fit(&self, at: usize, width: usize) -> (String, String) {
        let avail = self.tag_w(width).saturating_sub(2);
        let (head, model) = self.tag_text(at);
        if !model.is_empty() && head.width() + 3 + model.width() <= avail {
            return (head, format!(" · {model}"));
        }
        (truncate_str(&head, avail), String::new())
    }

    /// What starts every line of row `i`: its depth indent, or for a subagent's row its tag (`↳ 1 UI/UX lens ·
    /// haiku-4-5`) padded to the tag column, then the indent below its first level.
    fn lead(&self, i: usize, width: usize) -> String {
        let row = &self.rows[i];
        let indent = |depth: usize| "  ".repeat(depth.min(MAX_INDENT));
        let at = self.agents.iter().position(|a| a.id == row.agent).filter(|_| row.agent != self.root.id);
        let Some(at) = at else { return indent(row.depth) };
        let (head, tail) = self.tag_fit(at, width);
        let tag = format!("{head}{tail}");
        format!("{tag}{}{}", " ".repeat(self.tag_w(width).saturating_sub(tag.width())), indent(row.depth.saturating_sub(1)))
    }

    /// `lead` in colour: `↳ n` bold in the model accent, the name gray, ` · model` in the accent, the rest dim.
    fn lead_spans(&self, i: usize, width: usize, theme: &Theme) -> Vec<Span<'static>> {
        let row = &self.rows[i];
        let at = self.agents.iter().position(|a| a.id == row.agent).filter(|_| row.agent != self.root.id);
        let Some(at) = at else { return vec![Span::styled(self.lead(i, width), Style::default().fg(theme.gray_dim))] };
        let (head, tail) = self.tag_fit(at, width);
        let accent = Style::default().fg(theme.accent_model);
        // A tag cut below its number is all accent.
        let cut = if head.starts_with(&format!("↳ {at}")) { format!("↳ {at}").len() } else { head.len() };
        let (num, name) = (head[..cut].to_string(), head[cut..].to_string());
        let tag = format!("{num}{name}{tail}");
        let mut out = vec![Span::styled(num, accent.add_modifier(Modifier::BOLD)), Span::styled(name, Style::default().fg(theme.gray))];
        if !tail.is_empty() {
            out.push(Span::styled(tail, accent));
        }
        // The padding and the indent below the first level.
        let lead = self.lead(i, width);
        out.push(Span::styled(lead.strip_prefix(tag.as_str()).unwrap_or_default().to_string(), Style::default().fg(theme.gray_dim)));
        out
    }

    /// `links` is [`Self::links`], computed once per frame.
    fn row_lines(&self, i: usize, width: usize, now_ms: i64, theme: &Theme, body: Option<&[String]>, links: &HashMap<usize, usize>) -> Vec<Line<'static>> {
        let row = &self.rows[i];
        let ind = self.lead(i, width);
        let lead = self.lead_spans(i, width, theme);
        let w = width.saturating_sub(ind.width()).max(8);
        let gray = Style::default().fg(theme.gray);
        let faint = Style::default().fg(theme.gray_dim);
        let plain = |spans: Vec<Span<'static>>| {
            let mut all = lead.clone();
            all.extend(spans);
            Line::from(all)
        };
        // Word-wrapped at `avail` with blank lines dropped: the first `cap` lines and how many were cut.
        let wrap = |text: &str, avail: usize, cap: usize| -> (Vec<String>, usize) {
            let all: Vec<String> = text
                .lines()
                .map(str::trim_end)
                .filter(|l| !l.trim().is_empty())
                .flat_map(|l| modal_window::word_wrap(l, avail.max(1)).into_iter().map(str::to_string).collect::<Vec<_>>())
                .collect();
            let more = all.len().saturating_sub(cap);
            (all.into_iter().take(cap).collect(), more)
        };
        // Prose wraps short of the time column so the time always fits on its first line, as in grok's view.
        let time = clock(row.ts);
        let beside = time.as_ref().map_or(0, |t| t.width() + 2);
        match &row.body {
            // grok's prompt: a lifted band with the time on the right.
            Body::User(t) => {
                let band = Style::default().bg(theme.bg_light);
                // Under a tag the `›` takes the tag column, as `◆` does; the root's keeps grok's leading space.
                let (first, rest, cut, pad) = if row.agent == self.root.id { (" › ", "   ", "   … +", 4) } else { ("› ", "  ", "  … +", 3) };
                let (lines, more) = wrap(t, w.saturating_sub(pad + beside), if self.open.contains(&i) { INLINE_MAX_LINES } else { USER_LINES });
                let mut out: Vec<Line<'static>> = lines
                    .into_iter()
                    .enumerate()
                    .map(|(n, l)| {
                        let mark = if n == 0 { Span::styled(first, band.fg(theme.gray)) } else { Span::styled(rest, band) };
                        let mut spans = lead.clone();
                        spans.extend([mark, Span::styled(l, band.fg(theme.text_primary))]);
                        Line::from(spans)
                    })
                    .collect();
                if more > 0 {
                    let mut spans = lead.clone();
                    spans.push(Span::styled(format!("{cut}{more} lines"), band.fg(theme.gray)));
                    out.push(Line::from(spans));
                }
                for line in &mut out {
                    let used = line.width();
                    line.spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), band));
                }
                if let Some(first) = out.first_mut() {
                    right_align(first, time, width, band.fg(theme.gray));
                }
                out
            }
            // Agent text: grok's markdown renderer at this width, paragraphs and tables included.
            Body::Text(t) => {
                let mut lines: Vec<Line<'static>> = match self.md.get(&i) {
                    Some(md) => md.output(w.saturating_sub(beside).max(8)).lines.into_iter().map(|l| l.content).collect(),
                    None => t.lines().map(|l| Line::from(l.to_string())).collect(),
                };
                // Blank edges come from the markdown; the gap between rows is the stream's.
                while lines.last().is_some_and(blank) {
                    lines.pop();
                }
                let lead = lines.iter().take_while(|l| blank(l)).count();
                lines.drain(..lead);
                lines.dedup_by(|a, b| blank(a) && blank(b));
                let cap = if self.open.contains(&i) { INLINE_MAX_LINES } else { TEXT_LINES };
                let more = lines.len().saturating_sub(cap);
                lines.truncate(cap);
                let mut out: Vec<Line<'static>> = lines
                    .into_iter()
                    .map(|l| {
                        let style = l.style;
                        // Prose is the brightest thing in the stream: markdown body text (`md_text`, secondary in
                        // some themes) is lifted to `text_primary`; headings, code and links keep their colours.
                        let spans = l.spans.into_iter().map(|mut s| {
                            if s.style.fg == Some(theme.md_text) {
                                s.style.fg = Some(theme.text_primary);
                            }
                            s
                        });
                        let mut line = plain(spans.collect());
                        line.style = style;
                        line
                    })
                    .collect();
                if more > 0 {
                    out.push(plain(vec![Span::styled(format!("… +{more} lines"), gray)]));
                }
                if let Some(first) = out.first_mut() {
                    right_align(first, time, width, gray);
                }
                out
            }
            // grok's "◆ Thought": a quiet row of the same family as the calls.
            Body::Reasoning(t) => {
                let head = "◆ Thought  ";
                let avail = w.saturating_sub(head.width());
                // Markdown, compacted (no blank lines) and recoloured muted italic; bold survives.
                let all: Vec<Line<'static>> = match self.md.get(&i) {
                    Some(md) => md.output(avail).lines.into_iter().map(|l| l.content).filter(|l| !blank(l)).collect(),
                    None => wrap(t, avail, usize::MAX).0.into_iter().map(Line::from).collect(),
                };
                let cap = if self.open.contains(&i) { INLINE_MAX_LINES } else { REASONING_LINES };
                let more = all.len().saturating_sub(cap);
                let text = gray.add_modifier(Modifier::ITALIC);
                let hang = " ".repeat(head.width());
                let mut out: Vec<Line<'static>> = all
                    .into_iter()
                    .take(cap)
                    .enumerate()
                    .map(|(n, l)| {
                        let mut spans = if n == 0 {
                            vec![Span::styled("◆ ", gray), Span::styled("Thought  ", gray.add_modifier(Modifier::BOLD))]
                        } else {
                            vec![Span::raw(hang.clone())]
                        };
                        spans.extend(l.spans.into_iter().map(|s| Span::styled(s.content, s.style.patch(text))));
                        plain(spans)
                    })
                    .collect();
                if more > 0 {
                    out.push(plain(vec![Span::raw(hang), Span::styled(format!("… +{more} lines"), faint)]));
                }
                out
            }
            // grok's tool row: the diamond carries the outcome, the name and target stay quiet.
            Body::Call(c) => {
                let failed = c.result.as_ref().is_some_and(|r| !r.ok);
                let name = format!("{}  ", c.tool);
                let kid = links.get(&i).map(|&at| &self.agents[at]).filter(|_| !failed);
                let (diamond, status, status_style) = match (kid, &c.result, c.no_result) {
                    // A dispatch call: the child's model, calls and state. A background agent's call has its result
                    // at launch, so only the child's end makes it `✓`.
                    (Some(a), _, _) => {
                        let calls = self.rows.iter().filter(|r| r.agent == a.id && matches!(r.body, Body::Call(_))).count();
                        let state = if a.ended {
                            a.end_ms.map_or_else(|| "✓".to_string(), |e| format!("✓ {}", took((e - c.started_ms).max(0))))
                        } else {
                            format!("● {}", span_text((now_ms - c.started_ms) / 1000))
                        };
                        let model = a.model.as_deref().map(short_model);
                        let calls = format!("{calls} call{}", if calls == 1 { "" } else { "s" });
                        let join = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
                        let options = [
                            join(vec![model.clone(), Some(calls), Some(state.clone())]),
                            join(vec![model, Some(state.clone())]),
                            state,
                        ];
                        // The label keeps at least 12 columns: drop `N calls`, then the model.
                        let fits = |s: &str| w.saturating_sub(2 + name.width() + s.width() + 2) >= 12;
                        let pick = options.iter().find(|s| fits(s.as_str())).unwrap_or(&options[2]).clone();
                        let diamond = if a.ended {
                            blend_color(theme.bg_base, theme.accent_success, DIM_ACCENT).unwrap_or(theme.accent_success)
                        } else {
                            theme.accent_running
                        };
                        (diamond, pick, if a.ended { faint } else { gray })
                    }
                    (None, Some(r), _) if r.ok => (
                        blend_color(theme.bg_base, theme.accent_success, DIM_ACCENT).unwrap_or(theme.accent_success),
                        r.ms.map_or_else(String::new, took),
                        faint,
                    ),
                    (None, Some(r), _) => (theme.accent_error, format!("✗ {}", first_line(&r.error)).trim_end().to_string(), Style::default().fg(theme.accent_error)),
                    (None, None, true) => (theme.gray_dim, "no result".to_string(), faint),
                    (None, None, false) => (theme.accent_running, format!("… {}", span_text((now_ms - c.started_ms) / 1000)), gray),
                };
                let status = truncate_str(&status, w / 2);
                let name_style = if failed { Style::default().fg(theme.accent_error) } else { gray }.add_modifier(Modifier::BOLD);
                let room = w.saturating_sub(2 + name.width() + status.width() + 2);
                let what = truncate_str(first_line(&c.what), room);
                // Muted even when it is a command, as grok's collapsed rows are: prose stays the brightest thing.
                let mut line = plain(vec![Span::styled("◆ ", Style::default().fg(diamond)), Span::styled(name, name_style), Span::styled(what, gray)]);
                right_align(&mut line, Some(status).filter(|s| !s.is_empty()), width, status_style);
                let mut lines = vec![line];
                if let Some(rows) = body {
                    lines.extend(body_lines(rows, &ind, &c.tool, failed, theme));
                }
                lines
            }
            Body::Start { model, label } => {
                // Under a subagent's tag the tag already names the agent and its model.
                let tagged = row.agent != self.root.id && self.agents.iter().any(|a| a.id == row.agent);
                let mut spans = if tagged { vec![] } else { vec![Span::styled("↳ ", faint)] };
                if let Some(m) = model.as_ref().filter(|_| !tagged) {
                    spans.push(Span::styled(format!("{} ", short_model(m)), Style::default().fg(theme.accent_model)));
                }
                spans.push(Span::styled("started", gray));
                if let Some(l) = label.as_deref().filter(|l| !l.is_empty()) {
                    spans.push(Span::styled(format!("  {}", truncate_str(first_line(l), w.saturating_sub(24))), Style::default().fg(theme.text_primary)));
                }
                vec![plain(spans)]
            }
            Body::End if row.agent == self.root.id => vec![plain(vec![Span::styled("■ ended", gray.add_modifier(Modifier::BOLD))])],
            Body::End => vec![plain(vec![Span::styled("↳ ended", faint)])],
            Body::Resume => vec![plain(vec![Span::styled("↻ resumed", gray)])],
            Body::Model(m) => vec![plain(vec![Span::styled("model → ", gray), Span::styled(short_model(m), Style::default().fg(theme.accent_model))])],
        }
    }

    /// A blank line before the row at `pos` when it, or the visible row before it, is a prompt or agent text:
    /// calls stay packed together and prose gets air, as in grok's view.
    fn gap_before(&self, vis: &[usize], pos: usize) -> bool {
        let spaced = |i: usize| matches!(self.rows[i].body, Body::User(_) | Body::Text(_));
        pos > 0 && (spaced(vis[pos]) || spaced(vis[pos - 1]))
    }

    /// The tail (or the paused window) of the rows, and a status line when paused, filtered or failing.
    /// The deck above draws the session's name and state.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, index: Option<&TraceIndex>, theme: &Theme, now_ms: i64) {
        if area.height < 3 {
            return;
        }
        let width = area.width as usize;
        let paused = self.paused().then(|| format!("paused · {} new · G live", self.rows.len().saturating_sub(self.paused_len)));
        let status = [paused, self.filter_name().map(|n| format!("agent: {n}")), self.note.clone(), self.err.as_ref().map(|e| format!("index: {e}"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let mut body = area;
        if !status.is_empty() {
            body.height -= 1;
            buf.set_span(area.x, area.y + area.height - 1, &Span::styled(truncate_str(&status, width), Style::default().fg(theme.gray)), area.width);
        }

        let hgt = body.height as usize;
        let vis = self.visible();
        if vis.is_empty() {
            let msg = if self.connected { "no activity yet" } else { "connecting…" };
            buf.set_span(body.x, body.y, &Span::styled(msg, Style::default().fg(theme.gray)), body.width);
            return;
        }
        let links = self.links();
        let open: Vec<usize> = vis.iter().copied().filter(|i| self.open.contains(i)).collect();
        let bodies: HashMap<usize, Vec<String>> = open
            .into_iter()
            .filter_map(|i| {
                let lead = self.lead(i, width).width();
                Some((i, self.body_rows(i, index, width.saturating_sub(lead))?))
            })
            .collect();
        // A row's lines, its leading gap (if any) as an empty first line.
        let lines_at = |me: &Self, pos: usize| {
            let i = vis[pos];
            let mut lines = me.row_lines(i, width, now_ms, theme, bodies.get(&i).map(Vec::as_slice), &links);
            if me.gap_before(&vis, pos) {
                lines.insert(0, Line::default());
            }
            lines
        };
        let last = vis.len() - 1;
        match self.cursor.and_then(|c| vis.iter().position(|&i| i == c)) {
            Some(p) => {
                self.top = self.top.min(p);
                let mut used: usize = (self.top..=p).map(|q| lines_at(self, q).len()).sum();
                while used > hgt && self.top < p {
                    used -= lines_at(self, self.top).len();
                    self.top += 1;
                }
            }
            None => {
                // Pinned: the last rows that fit.
                let (mut used, mut pos) = (lines_at(self, last).len(), last);
                while pos > 0 && used + lines_at(self, pos - 1).len() <= hgt {
                    pos -= 1;
                    used += lines_at(self, pos).len();
                }
                self.top = pos;
            }
        }
        let mut y = body.y;
        for pos in self.top..vis.len() {
            let selected = self.cursor == Some(vis[pos]);
            let gap = self.gap_before(&vis, pos);
            for (n, line) in lines_at(self, pos).into_iter().enumerate() {
                if gap && n == 0 {
                    // The window's first row needs no gap above it.
                    if pos != self.top {
                        y += 1;
                    }
                    continue;
                }
                if y >= body.y + body.height {
                    return;
                }
                buf.set_line(body.x, y, &truncate_line(line, width), body.width);
                if selected {
                    select_row(buf, Rect { x: body.x, y, width: body.width, height: 1 }, theme);
                }
                y += 1;
            }
        }
    }
}

/// grok's selection on a drawn row: the theme's selection band (reverse video when bandless), bright bold text.
/// A call's diamond keeps its outcome colour at full strength, as grok undims a selected row's bullet.
fn select_row(buf: &mut Buffer, row: Rect, theme: &Theme) {
    let dimmed = blend_color(theme.bg_base, theme.accent_success, DIM_ACCENT);
    let diamonds: Vec<(u16, ratatui::style::Color)> =
        (row.x..row.x + row.width).filter(|&x| buf[(x, row.y)].symbol() == "◆").map(|x| (x, buf[(x, row.y)].fg)).collect();
    buf.set_style(row, theme.selection_overlay().fg(theme.text_primary).add_modifier(Modifier::BOLD));
    for (x, fg) in diamonds {
        buf[(x, row.y)].fg = if Some(fg) == dimmed { theme.accent_success } else { fg };
    }
}

fn blank(l: &Line<'_>) -> bool {
    l.spans.iter().all(|s| s.content.trim().is_empty())
}

/// `h:mm AM`, grok's scrollback timestamp.
fn clock(ms: Option<i64>) -> Option<String> {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms?).single().map(|t| t.format("%-I:%M %p").to_string())
}

/// Puts `right` at the line's right edge, inside any trailing fill, when it fits with two columns to spare.
fn right_align(line: &mut Line<'static>, right: Option<String>, width: usize, style: Style) {
    let Some(right) = right else { return };
    while line.spans.last().is_some_and(|s| !s.content.is_empty() && s.content.trim().is_empty()) && line.spans.len() > 1 {
        line.spans.pop();
    }
    let used = line.width();
    if used + right.width() + 2 > width {
        // No room: put the fill back.
        line.spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), style));
        return;
    }
    line.spans.push(Span::styled(" ".repeat(width - used - right.width()), style));
    line.spans.push(Span::styled(right, style));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Which highlighter token a tool's command gets, if any.
fn language(tool: &str) -> Option<&'static str> {
    let t = tool.to_ascii_lowercase();
    if t.contains("python") {
        Some("python")
    } else if t == "sh" || ["bash", "shell", "exec", "command"].iter().any(|k| t.contains(k)) {
        Some("sh")
    } else {
        None
    }
}

/// `call_rows` output as styled lines: command rows highlighted when the tool has a language, then the
/// separator and result dimmed (red after a failure).
/// shortcut: a wrapped command row is highlighted on its own line, so a construct split by the wrap loses its colour;
/// the narrow gutter's `│` also eats one column of indent.
fn body_lines(rows: &[String], ind: &str, tool: &str, failed: bool, theme: &Theme) -> Vec<Line<'static>> {
    let syntect = language(tool).map(|_| crate::syntax::get_syntect());
    let mut hl = syntect.zip(language(tool)).and_then(|(s, token)| s.highlight_lines_for_token(token));
    let mut in_result = false;
    rows.iter()
        .map(|row| {
            let rest = row.strip_prefix('│').unwrap_or(row);
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            let gutter = &row[..row.len() - rest.len()];
            let (cont, text) = rest.strip_prefix('↪').map_or(("", rest), |t| ("↪", t));
            let sep = traces_body::is_sep(row);
            in_result |= sep;
            let dim = Style::default().fg(theme.gray_dim);
            let mut spans = vec![Span::styled(format!("{ind}{gutter}{cont}"), dim)];
            match (&mut hl, in_result) {
                (Some(h), false) => spans.extend(highlight(h, text, &syntect.expect("set with hl").syntax_set)),
                (_, false) => spans.push(Span::styled(text.to_string(), Style::default().fg(theme.text_primary))),
                (_, true) if sep => spans.push(Span::styled(text.to_string(), Style::default().fg(theme.gray))),
                (_, true) => spans.push(Span::styled(text.to_string(), Style::default().fg(if failed { theme.accent_error } else { theme.text_secondary }))),
            }
            Line::from(spans)
        })
        .collect()
}

fn highlight(hl: &mut syntect::easy::HighlightLines<'_>, text: &str, set: &syntect::parsing::SyntaxSet) -> Vec<Span<'static>> {
    match hl.highlight_line(&format!("{text}\n"), set) {
        Ok(parts) => parts
            .into_iter()
            .map(|(style, seg)| {
                let fg = crate::syntax::syntect_rgb_to_fg(style.foreground.r, style.foreground.g, style.foreground.b);
                Span::styled(seg.trim_end_matches(['\n', '\r']).to_string(), Style::default().fg(fg))
            })
            .filter(|s| !s.content.is_empty())
            .collect(),
        Err(_) => vec![Span::raw(text.to_string())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> SessionRow {
        SessionRow {
            id: "root".into(),
            source: "claude".into(),
            title: "fix the parser".into(),
            model: Some("claude-sonnet-5-5".into()),
            ..Default::default()
        }
    }

    fn item(kind: &str, agent: &str, depth: usize, extra: Value) -> Value {
        let mut v = json!({"v": 1, "kind": kind, "agent": agent, "depth": depth, "line": 0, "ts": 1_000});
        v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        v
    }

    fn call(agent: &str, depth: usize, line: i64, id: &str, tool: &str, what: &str) -> Value {
        item("call", agent, depth, json!({"line": line, "call_id": id, "tool": tool, "what": what}))
    }

    fn done(agent: &str, depth: usize, line: i64, id: &str, ok: bool, error: &str, ms: i64) -> Value {
        item("result", agent, depth, json!({"line": line, "call_id": id, "ok": ok, "error": error, "ms": ms}))
    }

    /// `now` is 12 s after every fixture item.
    fn draw(f: &mut Follow, w: u16, h: u16) -> Vec<String> {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        f.render(&mut buf, area, None, &Theme::current(), 1_000 + 12_000);
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    fn screen(f: &mut Follow, w: u16, h: u16) -> String {
        draw(f, w, h).join("\n")
    }

    fn stream() -> Vec<Value> {
        vec![
            item("user", "root", 0, json!({"text": "fix the parser\nthen run the tests"})),
            item("reasoning", "root", 0, json!({"text": "the tokenizer is off by one"})),
            item("text", "root", 0, json!({"text": "I will patch the tokenizer."})),
            call("root", 0, 3, "t1", "Bash", "run the tests"),
            done("root", 0, 4, "t1", true, "", 400),
            call("root", 0, 5, "t2", "Agent", "Check the lexer"),
            item("agent_start", "kid", 1, json!({"model": "claude-haiku-4-5", "label": "Check the lexer"})),
            call("kid", 1, 0, "c1", "Read", "src/lexer.rs"),
            done("kid", 1, 1, "c1", false, "exit 1  no such file", 100),
            call("root", 0, 6, "t3", "Bash", "sleep 30"),
        ]
    }

    #[test]
    fn items_become_rows_and_results_merge_into_their_call() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        assert_eq!(f.rows.len(), 8, "results are not rows of their own");
        let Body::Call(c) = &f.rows[3].body else { panic!("{:?}", f.rows[3]) };
        assert_eq!(c.result, Some(Res { ok: true, error: String::new(), ms: Some(400), line: 4 }));
        let Body::Call(open) = &f.rows[7].body else { panic!() };
        assert!(open.result.is_none() && !open.no_result);
        assert_eq!(f.agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["root", "kid"]);

        let screen = screen(&mut f, 70, 16);
        for want in [
            " › fix the parser",
            "   then run the tests",
            "◆ Thought  the tokenizer",
            "I will patch",
            "◆ Bash  run the tests",
            "0.4s",
            "✗ exit 1  no such file",
            "… 12s",
        ] {
            assert!(screen.contains(want), "missing {want:?}\n{screen}");
        }
        assert!(
            screen.lines().any(|l| l.contains("◆ Read  src/lexer.rs") && l.starts_with("↳ 1 Check")),
            "the subagent's rows start with its numbered name\n{screen}"
        );
        let lines: Vec<&str> = screen.lines().collect();
        let text = lines.iter().position(|l| l.starts_with("I will patch")).unwrap();
        assert!(lines[text - 1].is_empty() && lines[text + 1].is_empty(), "agent text has air around it\n{screen}");
        let bash = lines.iter().position(|l| l.starts_with("◆ Bash  run the tests")).unwrap();
        assert!(lines[bash + 1].starts_with("◆ Agent"), "calls stay packed together\n{screen}");
    }

    #[test]
    fn a_link_gives_a_receipt_opened_call_the_transcript_lines_to_read() {
        let mut f = Follow::bare(session());
        f.apply(
            &[
                item("agent_start", "kid", 1, json!({})),
                item("call", "kid", 1, json!({"line": null, "call_id": "k1", "tool": "Bash", "what": "ls"})),
                item("result", "kid", 1, json!({"line": null, "call_id": "k1", "ok": false, "error": "failed", "ms": 100})),
            ],
            2_000,
        );
        f.cursor = Some(1);
        let (read_from, c) = f.selected_call().unwrap();
        assert_eq!((read_from, c.line), ("kid", -1), "no transcript line yet: nothing to read");
        assert!(f.body_rows(1, None, 60).unwrap()[0].contains("waiting for the transcript"));

        f.apply(
            &[
                item("link", "kid", 1, json!({"call_id": "k1", "session": "kidrow", "line": 7})),
                item("link", "kid", 1, json!({"call_id": "k1", "session": "kidrow", "line": null, "result_line": 8, "error": "exit 1  boom"})),
            ],
            2_100,
        );
        assert_eq!(f.rows.len(), 2, "a link is not a row");
        let (read_from, c) = f.selected_call().unwrap();
        assert_eq!((read_from, c.line), ("kidrow", 7), "lines are read from the child's own session");
        assert_eq!(c.result, Some(Res { ok: false, error: "exit 1  boom".into(), ms: Some(100), line: 8 }));
    }

    #[test]
    fn a_model_item_fills_in_a_label_the_start_did_not_have() {
        let mut f = Follow::bare(session());
        f.apply(&[item("agent_start", "kid", 1, json!({})), item("model", "kid", 1, json!({"model": "claude-haiku-4-5", "label": "Check the lexer"}))], 2_000);
        assert_eq!(f.rows.len(), 1, "the first model of an agent is not a row");
        let kid = f.agents.iter().find(|a| a.id == "kid").unwrap();
        assert_eq!((kid.model.as_deref(), kid.label.as_deref()), (Some("claude-haiku-4-5"), Some("Check the lexer")));
        assert_eq!(f.rows[0].body, Body::Start { model: Some("claude-haiku-4-5".into()), label: Some("Check the lexer".into()) }, "the start row is named");
    }

    #[test]
    fn an_end_marks_open_calls_and_resume_flips_it_back() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        assert!(f.is_live());
        f.apply(&[item("agent_end", "root", 0, json!({}))], 14_000);
        assert!(!f.is_live());
        let Body::Call(open) = &f.rows[7].body else { panic!() };
        assert!(open.no_result);
        let shown = screen(&mut f, 70, 16);
        assert!(shown.contains("no result") && shown.contains("■ ended"), "{shown}");
        assert!(!shown.contains("… 12s"), "no timer on a call of an ended agent\n{shown}");
        f.apply(&[item("agent_resume", "root", 0, json!({}))], 15_000);
        assert!(f.is_live());
        assert!(screen(&mut f, 70, 16).contains("↻ resumed"));
    }

    #[test]
    fn a_model_item_is_a_row_only_when_the_model_changed() {
        let mut f = Follow::bare(session());
        f.apply(&[item("model", "root", 0, json!({"model": "claude-sonnet-5-5"}))], 0);
        assert!(f.rows.is_empty(), "the root already had this model");
        f.apply(&[item("model", "root", 0, json!({"model": "claude-opus-5-5"}))], 0);
        assert_eq!(f.rows[0].body, Body::Model("claude-opus-5-5".into()));
    }

    #[test]
    fn k_pauses_the_tail_counts_new_rows_and_g_resumes() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        assert!(!f.paused());
        assert!(f.step(-1));
        assert_eq!(f.cursor, Some(6), "k from the tail lands one above the last row");
        assert!(f.paused());
        f.apply(&[item("text", "root", 0, json!({"text": "later"})), item("text", "root", 0, json!({"text": "later still"}))], 14_000);
        assert!(screen(&mut f, 70, 16).contains("paused · 2 new · G live"));
        assert!(f.key(KeyCode::Char('G')));
        assert!(!f.paused());
        assert!(!screen(&mut f, 70, 16).contains("paused"));
        let short = screen(&mut f, 70, 5);
        assert!(short.contains("later still") && !short.contains("fix the parser"), "a pinned short screen shows the newest rows\n{short}");
    }

    #[test]
    fn a_paused_window_keeps_the_cursor_row_on_screen() {
        let mut f = Follow::bare(session());
        let many: Vec<Value> = (0..40).map(|n| item("text", "root", 0, json!({"text": format!("row {n}")}))).collect();
        f.apply(&many, 0);
        f.first();
        let top = screen(&mut f, 40, 6);
        assert!(top.contains("row 0") && !top.contains("row 39"), "{top}");
        f.step(30);
        let mid = screen(&mut f, 40, 6);
        assert!(mid.contains("row 30") && !mid.contains("row 0\n"), "{mid}");
    }

    #[test]
    fn tab_cycles_every_agent_then_each_alone_and_back() {
        let mut f = Follow::bare(session());
        assert!(!f.cycle_agent(false), "one agent: nothing to cycle");
        f.apply(&stream(), 13_000);
        assert_eq!((f.filter, f.visible().len()), (0, 8));
        assert!(f.key(KeyCode::Tab));
        assert_eq!((f.filter, f.visible().len(), f.filter_name().as_deref()), (1, 6, Some("root")), "root alone");
        f.key(KeyCode::Tab);
        assert_eq!((f.filter, f.visible().len(), f.filter_name().as_deref()), (2, 2, Some("Check the lexer")), "the subagent alone");
        assert!(screen(&mut f, 70, 8).contains("agent: Check the lexer"));
        f.key(KeyCode::Tab);
        assert_eq!((f.filter, f.visible().len()), (0, 8), "all again");
        f.key(KeyCode::BackTab);
        assert_eq!(f.filter, 2);
    }

    #[test]
    fn e_jumps_to_the_next_failed_call_and_wraps() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        f.apply(&[call("root", 0, 7, "t4", "Bash", "false"), done("root", 0, 8, "t4", false, "exit 2", 5)], 13_000);
        assert!(f.next_error());
        assert_eq!(f.cursor, Some(6), "from the tail it wraps to the first error: the subagent's");
        assert!(f.next_error());
        assert_eq!(f.cursor, Some(8), "then the last call's");
        assert!(f.next_error());
        assert_eq!(f.cursor, Some(6));
        let mut ok = Follow::bare(session());
        ok.apply(&[call("root", 0, 1, "a", "Bash", "ls"), done("root", 0, 2, "a", true, "", 1)], 0);
        assert!(!ok.next_error());
    }

    #[test]
    fn the_inspect_target_is_the_highlighted_call_only() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        assert!(f.selected_call().is_none(), "pinned: nothing is highlighted");
        f.step(-1);
        assert_eq!(f.selected_call().map(|(a, c)| (a, c.line)), Some(("kid", 0)));
        f.key(KeyCode::Char('g'));
        assert!(f.selected_call().is_none(), "the first row is the prompt");
    }

    #[test]
    fn subagent_rows_start_with_a_numbered_tag_without_the_words_they_share() {
        let mut f = Follow::bare(session());
        f.apply(
            &[
                item("agent_start", "a", 1, json!({"model": "claude-haiku-4-5", "label": "Audit traces: UI/UX lens"})),
                item("agent_start", "b", 1, json!({"model": "claude-haiku-4-5", "label": "Audit traces: observability lens"})),
                call("a", 1, 0, "c1", "Read", "x.rs"),
                call("b", 1, 0, "c2", "Read", "y.rs"),
                call("root", 0, 1, "t1", "Bash", "ls"),
            ],
            2_000,
        );
        let lines = draw(&mut f, 120, 16);
        assert!(lines.iter().any(|l| l.starts_with("↳ 1 U") && l.contains("◆ Read")), "{lines:#?}");
        assert!(lines.iter().any(|l| l.starts_with("↳ 2 o") && l.contains("◆ Read")), "{lines:#?}");
        assert!(lines.iter().any(|l| l.starts_with("◆ Bash")), "root rows have no tag\n{lines:#?}");
        assert!(!lines.iter().any(|l| l.starts_with("0 ")), "{lines:#?}");
    }

    /// Two labelled subagents of the root: `a` finished (its `agent_end` at 135 s), `b` live. Both `Agent` calls have
    /// their result at launch, as a background agent's do. Prompt times are nulled (the clock column is local time);
    /// everything else keeps its item time, and `draw`'s `now` is 12 s after it.
    fn two_lenses() -> Follow {
        let mut f = Follow::bare(session());
        let mut items = vec![
            item("user", "root", 0, json!({"text": "audit the traces screens with two lenses", "ts": null})),
            call("root", 0, 1, "t1", "Agent", "Audit traces: UI/UX lens"),
            done("root", 0, 2, "t1", true, "", 340),
            call("root", 0, 3, "t2", "Agent", "Audit traces: observability lens"),
            done("root", 0, 4, "t2", true, "", 340),
        ];
        for (agent, label, line) in [("a", "Audit traces: UI/UX lens", 5), ("b", "Audit traces: observability lens", 7)] {
            items.extend([
                item("agent_start", agent, 1, json!({"model": "claude-haiku-4-5", "label": label})),
                item("user", agent, 1, json!({"text": format!("You are one of two auditors. Your lens: {label}"), "ts": null})),
                call(agent, 1, line, &format!("r{line}"), "Read", "src/views/traces_follow.rs"),
                done(agent, 1, line + 1, &format!("r{line}"), true, "", 100),
            ]);
        }
        items.push(item("agent_end", "a", 1, json!({"ts": 135_000})));
        items.push(call("root", 0, 9, "t3", "Bash", "cargo test"));
        items.push(done("root", 0, 10, "t3", true, "", 400));
        f.apply(&items, 13_000);
        f
    }

    /// One subagent under one `Agent` call, as a single line of setup for the odd tag states.
    fn one_kid(label: Option<&str>, model: Option<&str>, dispatch: &str) -> Follow {
        let mut f = Follow::bare(session());
        f.apply(
            &[
                item("user", "root", 0, json!({"text": "look into it", "ts": null})),
                call("root", 0, 1, "t1", "Agent", dispatch),
                done("root", 0, 2, "t1", true, "", 340),
                item("agent_start", "k", 1, json!({"model": model, "label": label})),
                item("user", "k", 1, json!({"text": "You are the auditor.", "ts": null})),
                call("k", 1, 3, "r3", "Read", "src/views/traces_follow.rs"),
                done("k", 1, 4, "r3", true, "", 100),
            ],
            13_000,
        );
        f
    }

    /// The subagent look, for Sab to see before install: tag, aligned prompt band, the `Agent` call's summary.
    #[test]
    fn the_subagent_rows_at_two_widths() {
        let mut f = two_lenses();
        insta::assert_snapshot!("subagent_rows_width_120", screen(&mut f, 120, 20));
        insta::assert_snapshot!("subagent_rows_width_60", screen(&mut f, 60, 24));
    }

    #[test]
    fn an_unlabelled_subagent_shows_its_model_once() {
        let mut f = one_kid(None, Some("claude-haiku-4-5"), "Explore the index");
        insta::assert_snapshot!("subagent_unlabelled_width_60", screen(&mut f, 60, 12));
    }

    #[test]
    fn a_subagent_with_no_model_yet_has_a_tag_without_one() {
        let mut f = one_kid(Some("Check the lexer"), None, "Check the lexer");
        insta::assert_snapshot!("subagent_no_model_width_60", screen(&mut f, 60, 12));
    }

    #[test]
    fn a_subagent_name_over_the_cap_is_cut_with_an_ellipsis() {
        let label = "Audit the observability of the traces follow screen end to end";
        let mut f = one_kid(Some(label), Some("claude-haiku-4-5"), label);
        insta::assert_snapshot!("subagent_long_name_width_60", screen(&mut f, 60, 12));
    }

    #[test]
    fn links_pair_each_labelled_subagent_with_its_dispatch_call() {
        let f = two_lenses();
        let links = f.links();
        assert_eq!(links.len(), 2, "{links:?}");
        for (row, at) in links {
            let Body::Call(c) = &f.rows[row].body else { panic!("row {row} is not a call") };
            assert_eq!(Some(c.what.as_str()), f.agents[at].label.as_deref());
        }
        let unlabelled = one_kid(None, Some("claude-haiku-4-5"), "Explore the index");
        assert!(unlabelled.links().is_empty(), "an unlabelled child links nothing");
    }

    #[test]
    fn a_finished_agent_call_shows_a_tick_and_a_live_one_a_dot_whatever_its_own_result() {
        let mut f = two_lenses();
        let lines = draw(&mut f, 120, 20);
        let row = |label: &str| lines.iter().find(|l| l.contains(&format!("◆ Agent  {label}"))).unwrap_or_else(|| panic!("{label}\n{lines:#?}")).clone();
        let done = row("Audit traces: UI/UX lens");
        assert!(done.ends_with("haiku-4-5 · 1 call · ✓ 2m14s"), "{done}");
        assert!(!done.contains('…'), "no running timer on a finished call\n{done}");
        let live = row("Audit traces: observability lens");
        assert!(live.ends_with("haiku-4-5 · 1 call · ● 12s"), "its call has a result, the child has not ended\n{live}");
        f.apply(&[item("agent_resume", "a", 1, json!({}))], 14_000);
        assert!(draw(&mut f, 120, 20).iter().any(|l| l.contains("◆ Agent  Audit traces: UI/UX lens") && l.contains('●')), "a resumed child is live again");
    }

    #[test]
    fn an_agent_call_drops_its_calls_then_its_model_before_its_label_gets_narrow() {
        let mut f = two_lenses();
        let tight = |f: &mut Follow, w: u16| draw(f, w, 24).into_iter().find(|l| l.contains("◆ Agent  Audit")).unwrap();
        let wide = tight(&mut f, 120);
        assert!(wide.contains("1 call") && wide.contains("haiku-4-5"), "{wide}");
        let mid = tight(&mut f, 46);
        assert!(!mid.contains("call") && mid.contains("haiku-4-5") && mid.contains("✓ 2m14s"), "{mid}");
        let narrow = tight(&mut f, 36);
        assert!(!narrow.contains("haiku") && narrow.contains("✓ 2m14s"), "{narrow}");
    }

    #[test]
    fn a_prompt_band_on_a_tagged_row_starts_at_the_tag_column() {
        let mut f = one_kid(Some("Check the lexer"), Some("claude-haiku-4-5"), "Check the lexer");
        f.apply(&[item("user", "k", 1, json!({"text": "word ".repeat(40), "ts": null}))], 13_000);
        let lines = draw(&mut f, 60, 20);
        let col = |l: &str, c: char| l.chars().position(|x| x == c).unwrap();
        let read = lines.iter().find(|l| l.contains("◆ Read")).unwrap();
        let prompt = lines.iter().find(|l| l.contains("› You are the auditor.")).unwrap();
        assert_eq!(col(prompt, '›'), col(read, '◆'), "{lines:#?}");
        let at = lines.iter().position(|l| l.contains("› word word")).unwrap();
        let (first, second) = (&lines[at], &lines[at + 1]);
        assert_eq!(col(first, '›'), col(read, '◆'));
        assert!(second.starts_with("↳ 1") && second.chars().skip(col(read, '◆')).take(2).all(|c| c == ' '), "wrapped lines keep the tag and start at the column\n{second:?}");
        assert!(lines.iter().any(|l| l.starts_with(" › look into it")), "a root prompt keeps grok's leading space\n{lines:#?}");
    }

    #[test]
    fn f_targets_the_highlighted_subagent_row_or_its_agent_call_and_nothing_on_a_root_row() {
        let mut f = two_lenses();
        assert_eq!(f.cursor_agent(), None, "pinned: nothing is highlighted");
        let row = |f: &Follow, pred: &dyn Fn(&Row) -> bool| f.rows.iter().position(|r| pred(r)).unwrap();
        let is_agent_call = |r: &Row, what: &str| matches!(&r.body, Body::Call(c) if c.tool == "Agent" && c.what == what);
        let read_a = row(&f, &|r| r.agent == "a" && matches!(&r.body, Body::Call(_)));
        let dispatch_b = row(&f, &|r| is_agent_call(r, "Audit traces: observability lens"));
        let root_bash = row(&f, &|r| matches!(&r.body, Body::Call(c) if c.tool == "Bash"));
        for (cursor, want) in [(read_a, Some("a")), (dispatch_b, Some("b")), (root_bash, None), (0, None)] {
            f.cursor = Some(cursor);
            assert_eq!(f.cursor_agent(), want, "row {cursor}");
        }
    }

    #[test]
    fn an_exact_label_beats_a_prefix_match() {
        let mut f = Follow::bare(session());
        f.apply(&[call("root", 0, 1, "t1", "Agent", "Explore"), call("root", 0, 2, "t2", "Agent", "Explore the cache")], 13_000);
        assert_eq!(f.spawn_row("Explore", Some(1_000)), Some(0), "not the later, longer call that merely starts with it");
        assert_eq!(f.spawn_row("Explore the cache", Some(1_000)), Some(1));
    }

    #[test]
    fn the_spawn_row_is_found_by_label_and_a_missing_one_is_said() {
        let mut f = Follow::bare(session());
        f.apply(&[call("root", 0, 1, "t1", "Bash", "ls"), call("root", 0, 2, "t2", "Agent", "Check the lexer"), call("root", 0, 3, "t3", "Bash", "pwd")], 13_000);
        f.want = Some(("Check the lexer".into(), Some(1_000)));
        f.resolve_want();
        assert_eq!((f.cursor, f.note.as_deref(), f.want.is_none()), (Some(1), None, true));
        f.resume();
        f.want = Some(("Something never dispatched".into(), Some(1_000)));
        f.resolve_want();
        assert_eq!((f.cursor, f.note.as_deref(), f.want.is_none()), (None, Some("spawn row not loaded"), true), "the tail stays pinned");
        assert!(screen(&mut f, 70, 8).contains("spawn row not loaded"));
        assert!(f.key(KeyCode::Char('k')) && f.note.is_none(), "the next key clears it");
    }

    #[test]
    fn l_opens_a_long_prompt_in_full_and_h_folds_it_back() {
        let mut f = Follow::bare(session());
        let brief: Vec<String> = (0..10).map(|n| format!("brief line {n}")).collect();
        f.apply(&[item("user", "root", 0, json!({ "text": brief.join("\n") }))], 0);
        let shown = screen(&mut f, 60, 20);
        assert!(shown.contains("brief line 2") && !shown.contains("brief line 3") && shown.contains("… +7 lines"), "{shown}");
        assert!(f.key(KeyCode::Char('g')));
        let all = brief.join("\n");
        assert_eq!(f.selected_prompt(), Some((all.as_str(), Some(1_000))));
        assert!(f.key(KeyCode::Char('l')));
        let shown = screen(&mut f, 60, 20);
        assert!(shown.contains("brief line 9") && !shown.contains("… +"), "{shown}");
        assert!(f.key(KeyCode::Char('h')));
        assert!(!screen(&mut f, 60, 20).contains("brief line 3"));
    }

    #[test]
    fn selected_prompt_is_the_highlighted_prompt_only() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        assert!(f.selected_prompt().is_none(), "pinned: nothing is highlighted");
        f.key(KeyCode::Char('g'));
        assert_eq!(f.selected_prompt().map(|(t, _)| t), Some("fix the parser\nthen run the tests"));
        f.step(3);
        assert!(f.selected_prompt().is_none(), "a call is not a prompt");
    }

    #[test]
    fn agent_text_is_rendered_markdown_not_source() {
        let mut f = Follow::bare(session());
        let md = "**Done.** Two things:\n\n- first\n- second\n\n| Work | State |\n|---|---|\n| A1 | Landed |";
        f.apply(&[item("text", "root", 0, json!({ "text": md })), item("reasoning", "root", 0, json!({"text": "check `lexer.rs` **first**"}))], 0);
        assert!(f.step(-1) && f.fold(true), "the table is past the folded message's first lines");
        let shown = screen(&mut f, 60, 18);
        assert!(shown.contains("◆ Thought  check lexer.rs first"), "reasoning is markdown too\n{shown}");
        assert!(shown.contains("Done. Two things:") && !shown.contains("**"), "{shown}");
        assert!(shown.contains("• first"), "{shown}");
        assert!(!shown.contains("|---|") && shown.contains("Landed"), "a table, not its pipes\n{shown}");
    }

    #[test]
    fn a_long_message_starts_folded_and_l_opens_it() {
        let mut f = Follow::bare(session());
        let text: Vec<String> = (0..20).map(|n| format!("- para {n}")).collect();
        f.apply(&[item("text", "root", 0, json!({ "text": text.join("\n") }))], 0);
        let shown = screen(&mut f, 60, 40);
        assert!(shown.contains("para 5") && !shown.contains("para 6") && shown.contains("… +14 lines"), "{shown}");
        assert!(f.step(-1) && f.fold(true));
        let shown = screen(&mut f, 60, 40);
        assert!(shown.contains("para 19") && !shown.contains("… +"), "{shown}");
        assert!(f.fold(false));
        assert!(!screen(&mut f, 60, 40).contains("para 6"));
    }

    #[test]
    fn long_text_keeps_its_time_on_the_first_line() {
        let mut f = Follow::bare(session());
        f.apply(&[item("text", "root", 0, json!({ "text": "word ".repeat(60) }))], 0);
        let first = draw(&mut f, 60, 12)[0].clone();
        assert!(first.ends_with("AM") || first.ends_with("PM"), "{first:?}");
    }

    #[test]
    fn the_diamond_carries_the_outcome_and_prose_outranks_calls() {
        let mut f = Follow::bare(session());
        f.apply(&stream(), 13_000);
        let theme = Theme::current();
        let area = Rect::new(0, 0, 70, 16);
        let mut buf = Buffer::empty(area);
        f.render(&mut buf, area, None, &theme, 13_000);
        let at = |needle: &str| {
            (0..16u16)
                .find_map(|y| {
                    let row: String = (0..70u16).map(|x| buf[(x, y)].symbol().to_string()).collect();
                    row.find(needle).map(|b| (row[..b].chars().count() as u16, y))
                })
                .unwrap_or_else(|| panic!("{needle:?} not drawn"))
        };
        let fg = |needle: &str| {
            let (x, y) = at(needle);
            buf[(x, y)].fg
        };
        let done = blend_color(theme.bg_base, theme.accent_success, DIM_ACCENT).unwrap();
        assert_eq!(fg("◆ Bash  run"), done, "a finished call's diamond is dimmed green");
        assert_eq!(fg("◆ Read"), theme.accent_error, "a failed call's diamond is red");
        assert_eq!(fg("I will patch"), theme.text_primary, "agent text is the brightest thing in the stream");
        let (x, y) = at("◆ Bash  run the tests");
        assert_eq!(buf[(x + 8, y)].fg, theme.gray, "a call's target is muted");
    }

    /// Every theme grok ships: the selected row is grok's selection band (reverse video when bandless) with
    /// bright bold text, apart from the prompt band; a failed call's diamond stays red on it.
    #[test]
    fn the_selected_row_is_grok_selection_in_every_theme() {
        let themes = [
            Theme::groknight(),
            Theme::grokday(),
            Theme::tokyonight(),
            Theme::rosepine_moon(),
            Theme::oscura_midnight(),
            Theme::kanso_mist(),
            Theme::kanagawa_wave(),
            Theme::kanagawa(),
            Theme::jetbrains_islands_dark(),
            Theme::night_owl(),
            Theme::one_dark_pro(),
            Theme::nstlgy_dark(),
            Theme::severance_dark(),
            Theme::quasi_monochrome_dark(),
            Theme::jetbrains_islands_light(),
            Theme::monosami_light(),
            Theme::monosami_ink_light(),
            Theme::dracula(),
            Theme::ayu_darker(),
            Theme::fleet_dark(),
            Theme::carbonfox(),
            Theme::kanagawa_dragon(),
            Theme::kanso_ink(),
            Theme::fleet_light(),
            Theme::dayfox(),
            Theme::kanagawa_lotus(),
            Theme::kanso_pearl(),
            Theme::terminal(),
        ];
        for theme in themes {
            for (row, needle) in [(0, " › fix the parser"), (6, "◆ Read")] {
                let mut f = Follow::bare(session());
                f.apply(&stream(), 13_000);
                f.cursor = Some(row);
                let area = Rect::new(0, 0, 70, 18);
                let mut buf = Buffer::empty(area);
                f.render(&mut buf, area, None, &theme, 13_000);
                let y = (0..18u16)
                    .find(|&y| (0..70u16).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().contains(needle))
                    .unwrap_or_else(|| panic!("{needle:?} not drawn"));
                let cell = &buf[(10, y)];
                match theme.selection_overlay().bg {
                    Some(sel) => {
                        assert_eq!(cell.bg, sel, "{needle:?} in {:?}", theme.bg_base);
                        assert_ne!(sel, theme.bg_light, "selection must differ from the prompt band");
                    }
                    None => assert!(cell.modifier.contains(Modifier::REVERSED), "bandless selects in reverse video"),
                }
                assert!(cell.modifier.contains(Modifier::BOLD), "selected text is bold");
                if needle == "◆ Read" {
                    let x = (0..70u16).find(|&x| buf[(x, y)].symbol() == "◆").unwrap();
                    assert_eq!(buf[(x, y)].fg, theme.accent_error, "the outcome survives the selection");
                }
            }
        }
    }

    #[test]
    fn tool_languages() {
        assert_eq!(language("Bash"), Some("sh"));
        assert_eq!(language("exec_command"), Some("sh"));
        assert_eq!(language("python3"), Some("python"));
        assert_eq!(language("Read"), None);
    }

    #[test]
    fn a_bash_body_is_highlighted_and_its_result_is_not() {
        let theme = Theme::current();
        let rows = traces_body::call_rows("echo \"hi\" | wc -l\nls -la", Some(("2", false)), 60);
        let lines = body_lines(&rows, "", "Bash", false, &theme);
        assert_eq!(lines.len(), rows.len());
        assert!(lines[0].spans.len() > 2, "a gutter plus several coloured tokens, got {:?}", lines[0]);
        assert_eq!(lines.last().unwrap().spans.last().unwrap().content, "2");
        assert_eq!(body_lines(&rows, "", "Read", false, &theme)[0].spans.len(), 2, "no language: gutter and one plain span");
    }
}
