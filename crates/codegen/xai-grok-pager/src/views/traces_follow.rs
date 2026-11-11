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
use crate::views::traces_modal::{compose_body, continuation, continuation_source, split_result};

/// Rows replayed when the screen opens (lookout's respawn replays the same).
const BACKLOG: usize = 200;
/// Reasoning is dimmed context, not the thing being followed: this many lines, then `… +N lines`.
const REASONING_LINES: usize = 6;
const MAX_INDENT: usize = 8;
/// A prompt shows this many wrapped lines, then `… +N lines`.
const USER_LINES: usize = 3;
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
            agents: vec![Agent { id: root.id.clone(), model: root.model.clone(), label: None }],
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

    fn has_open_call(&self) -> bool {
        !self.ended && self.rows.iter().rev().take(64).any(|r| matches!(&r.body, Body::Call(c) if c.result.is_none() && !c.no_result))
    }

    fn agent_mut(&mut self, id: &str) -> &mut Agent {
        let at = self.agents.iter().position(|a| a.id == id).unwrap_or_else(|| {
            self.agents.push(Agent { id: id.to_string(), model: None, label: None });
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
                    Body::Start { model: a.model.clone(), label: a.label.clone() }
                }
                "agent_end" => {
                    for r in self.rows.iter_mut().filter(|r| r.agent == agent) {
                        if let Body::Call(c) = &mut r.body {
                            c.no_result |= c.result.is_none();
                        }
                    }
                    self.ended |= is_root;
                    Body::End
                }
                "agent_resume" => {
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

    /// Basic key handling; Enter, Esc and `o` are the modal's.
    pub fn key(&mut self, code: KeyCode) -> bool {
        match code {
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
        }
    }

    /// `l` / `h`: open or close the highlighted call's body inline. False when nothing changed.
    pub fn fold(&mut self, open: bool) -> bool {
        let Some(i) = self.cursor.filter(|&i| self.call_at(i).is_some()) else { return false };
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

    fn row_lines(&self, i: usize, width: usize, now_ms: i64, theme: &Theme, body: Option<&[String]>) -> Vec<Line<'static>> {
        let row = &self.rows[i];
        let ind = " ".repeat(2 * row.depth.min(MAX_INDENT));
        let w = width.saturating_sub(ind.width()).max(8);
        let gray = Style::default().fg(theme.gray);
        let faint = Style::default().fg(theme.gray_dim);
        let plain = |spans: Vec<Span<'static>>| {
            let mut all = vec![Span::raw(ind.clone())];
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
                let (lines, more) = wrap(t, w.saturating_sub(4 + beside), USER_LINES);
                let mut out: Vec<Line<'static>> = lines
                    .into_iter()
                    .enumerate()
                    .map(|(n, l)| {
                        let lead = if n == 0 { Span::styled(" › ", band.fg(theme.gray)) } else { Span::styled("   ", band) };
                        Line::from(vec![Span::raw(ind.clone()), lead, Span::styled(l, band.fg(theme.text_primary))])
                    })
                    .collect();
                if more > 0 {
                    out.push(Line::from(vec![Span::raw(ind.clone()), Span::styled(format!("   … +{more} lines"), band.fg(theme.gray))]));
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
                let more = lines.len().saturating_sub(INLINE_MAX_LINES);
                lines.truncate(INLINE_MAX_LINES);
                let mut out: Vec<Line<'static>> = lines
                    .into_iter()
                    .map(|l| {
                        let style = l.style;
                        let mut line = plain(l.spans);
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
                let more = all.len().saturating_sub(REASONING_LINES);
                let text = gray.add_modifier(Modifier::ITALIC);
                let hang = " ".repeat(head.width());
                let mut out: Vec<Line<'static>> = all
                    .into_iter()
                    .take(REASONING_LINES)
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
                let (diamond, status, status_style) = match (&c.result, c.no_result) {
                    (Some(r), _) if r.ok => (
                        blend_color(theme.bg_base, theme.accent_success, DIM_ACCENT).unwrap_or(theme.accent_success),
                        r.ms.map_or_else(String::new, took),
                        faint,
                    ),
                    (Some(r), _) => (theme.accent_error, format!("✗ {}", first_line(&r.error)).trim_end().to_string(), Style::default().fg(theme.accent_error)),
                    (None, true) => (theme.gray_dim, "no result".to_string(), faint),
                    (None, false) => (theme.accent_running, format!("… {}", span_text((now_ms - c.started_ms) / 1000)), gray),
                };
                let status = truncate_str(&status, w / 2);
                let name = format!("{}  ", c.tool);
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
                let mut spans = vec![Span::styled("↳ ", faint)];
                if let Some(m) = model {
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
        let status = [paused, self.filter_name().map(|n| format!("agent: {n}")), self.err.as_ref().map(|e| format!("index: {e}"))]
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
        let open: Vec<usize> = vis.iter().copied().filter(|i| self.open.contains(i)).collect();
        let bodies: HashMap<usize, Vec<String>> = open
            .into_iter()
            .filter_map(|i| {
                let depth = self.rows[i].depth.min(MAX_INDENT);
                Some((i, self.body_rows(i, index, width.saturating_sub(2 * depth))?))
            })
            .collect();
        // A row's lines, its leading gap (if any) as an empty first line.
        let lines_at = |me: &Self, pos: usize| {
            let i = vis[pos];
            let mut lines = me.row_lines(i, width, now_ms, theme, bodies.get(&i).map(Vec::as_slice));
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
                if selected {
                    buf.set_style(Rect { x: body.x, y, width: body.width, height: 1 }, Style::default().bg(theme.bg_highlight));
                }
                buf.set_line(body.x, y, &truncate_line(line, width), body.width);
                y += 1;
            }
        }
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
            "  ↳ haiku-4-5 started  Check the lexer",
            "  ◆ Read  src/lexer.rs",
            "✗ exit 1  no such file",
            "… 12s",
        ] {
            assert!(screen.contains(want), "missing {want:?}\n{screen}");
        }
        assert!(screen.lines().any(|l| l.starts_with("  ◆ Read")), "the subagent's rows sit two columns in\n{screen}");
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
    fn agent_text_is_rendered_markdown_not_source() {
        let mut f = Follow::bare(session());
        let md = "**Done.** Two things:\n\n- first\n- second\n\n| Work | State |\n|---|---|\n| A1 | Landed |";
        f.apply(&[item("text", "root", 0, json!({ "text": md })), item("reasoning", "root", 0, json!({"text": "check `lexer.rs` **first**"}))], 0);
        let shown = screen(&mut f, 60, 18);
        assert!(shown.contains("◆ Thought  check lexer.rs first"), "reasoning is markdown too\n{shown}");
        assert!(shown.contains("Done. Two things:") && !shown.contains("**"), "{shown}");
        assert!(shown.contains("• first"), "{shown}");
        assert!(!shown.contains("|---|") && shown.contains("Landed"), "a table, not its pipes\n{shown}");
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
