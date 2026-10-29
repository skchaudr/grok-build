//! `/traces` follow screen: one live session (and its subagents) as a tail you can pause, filter and read.
//!
//! A `Follower` (the library behind `grok-traces follow`) runs on a worker thread and is ticked from the
//! modal's 1 s poll; its `--json` items become the [`Row`]s here. Nothing here touches the index except the
//! raw-line reads that fill the selected call's body.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::follow::{Follower, header, short_model};
use xai_grok_traces::{SessionRow, TraceIndex, user_roots};

use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window;
use crate::views::traces_body::{self, INLINE_MAX_LINES};
use crate::views::traces_modal::{compose_body, continuation, continuation_source, split_result};

/// Rows replayed when the screen opens (lookout's respawn replays the same).
const BACKLOG: usize = 200;
/// Reasoning is dimmed context, not the thing being followed: this many lines, then `… +N lines`.
const REASONING_LINES: usize = 6;
const MAX_INDENT: usize = 8;

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
                }),
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
                    let had = self.agent_mut(&agent).model.replace(model.clone());
                    if had.is_none_or(|m| m == model) {
                        continue;
                    }
                    Body::Model(model)
                }
                _ => continue,
            };
            self.rows.push(Row { agent, depth, body });
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

    /// The highlighted row's call (with its agent's session id), when it is one.
    pub fn selected_call(&self) -> Option<(&str, &Call)> {
        let row = self.rows.get(self.cursor?)?;
        match &row.body {
            Body::Call(c) => Some((row.agent.as_str(), c)),
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
            _ => false,
        }
    }

    // ------------------------------------------------------------------ render

    /// The selected call's inline body: command, separator, result. Cached once complete.
    fn body_rows(&mut self, index: Option<&TraceIndex>, width: usize) -> Option<Vec<String>> {
        let (agent, c) = self.selected_call()?;
        let (agent, c) = (agent.to_string(), c.clone());
        let key = (agent.clone(), c.line, c.result.as_ref().map(|r| r.line));
        let text = match self.bodies.get(&key) {
            Some(t) => t.clone(),
            None => {
                let index = index?;
                let fetch = |l: i64| index.raw_line(&agent, l).map_err(|e| e.to_string());
                match compose_body(fetch(c.line), c.result.as_ref().map(|r| (fetch(r.line), !r.ok))) {
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
        let plain = |text: String, style: Style| Line::from(Span::styled(format!("{ind}{text}"), style));
        let wrapped = |mark: &str, text: &str, cap: usize, style: Style| -> Vec<Line<'static>> {
            let hang = " ".repeat(mark.width());
            let avail = w.saturating_sub(mark.width()).max(1);
            let all: Vec<&str> = text
                .lines()
                .map(str::trim_end)
                .filter(|l| !l.trim().is_empty())
                .flat_map(|l| modal_window::word_wrap(l, avail))
                .collect();
            let mut out: Vec<Line<'static>> =
                all.iter().take(cap).enumerate().map(|(n, l)| plain(format!("{}{l}", if n == 0 { mark } else { hang.as_str() }), style)).collect();
            if all.len() > cap {
                out.push(plain(format!("{hang}… +{} lines", all.len() - cap), Style::default().fg(theme.gray)));
            }
            out
        };
        match &row.body {
            Body::User(t) => {
                let mut lines = t.lines().map(str::trim).filter(|l| !l.is_empty());
                let head = lines.next().unwrap_or_default();
                let more = lines.count();
                let more = if more > 0 { format!(" (+{more} lines)") } else { String::new() };
                let text = truncate_str(&format!("{ind}▶ you  {head}{more}"), width);
                vec![Line::from(Span::styled(text, Style::default().fg(theme.accent_user).add_modifier(Modifier::BOLD)))]
            }
            Body::Text(t) => wrapped("· ", t, INLINE_MAX_LINES, Style::default().fg(theme.text_primary)),
            Body::Reasoning(t) => wrapped("~ ", t, REASONING_LINES, Style::default().fg(theme.gray).add_modifier(Modifier::DIM)),
            Body::Call(c) => {
                let failed = c.result.as_ref().is_some_and(|r| !r.ok);
                let (status, color) = match (&c.result, c.no_result) {
                    (Some(r), _) if r.ok => (format!("✓ {}", r.ms.map_or_else(String::new, took)).trim_end().to_string(), theme.accent_assistant),
                    (Some(r), _) => (format!("✗ {}", first_line(&r.error)).trim_end().to_string(), theme.accent_error),
                    (None, true) => ("no result".to_string(), theme.gray),
                    (None, false) => (format!("… {}", span_text((now_ms - c.started_ms) / 1000)), theme.gray),
                };
                let status = truncate_str(&status, w / 2);
                let head = format!("→ {}  ", c.tool);
                let what = truncate_str(first_line(&c.what), w.saturating_sub(head.width() + status.width() + 2));
                let pad = w.saturating_sub(head.width() + what.width() + status.width()).max(1);
                let tool = if failed { theme.accent_error } else { theme.accent_tool };
                let mut lines = vec![Line::from(vec![
                    Span::raw(ind.clone()),
                    Span::styled(head, Style::default().fg(tool).add_modifier(Modifier::BOLD)),
                    Span::styled(what, Style::default().fg(theme.gray)),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(status, Style::default().fg(color)),
                ])];
                if let Some(rows) = body {
                    lines.extend(body_lines(rows, &ind, &c.tool, failed, theme));
                }
                lines
            }
            Body::Start { model, label } => {
                let model = model.as_deref().map(|m| format!("{} ", short_model(m))).unwrap_or_default();
                let label = label.as_deref().filter(|l| !l.is_empty()).map(|l| format!("  {l}")).unwrap_or_default();
                vec![plain(format!("↳ {model}started{label}"), Style::default().fg(theme.accent_user))]
            }
            Body::End if row.agent == self.root.id => vec![plain("■ ended".to_string(), Style::default().fg(theme.gray).add_modifier(Modifier::BOLD))],
            Body::End => vec![plain("↳ ended".to_string(), Style::default().fg(theme.gray))],
            Body::Resume => vec![plain("↻ resumed".to_string(), Style::default().fg(theme.gray))],
            Body::Model(m) => vec![plain(format!("model → {}", short_model(m)), Style::default().fg(theme.accent_user))],
        }
    }

    /// Header line, the tail (or the paused window) of the rows, and a status line.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, index: Option<&TraceIndex>, theme: &Theme, now_ms: i64) {
        if area.height < 3 {
            return;
        }
        let width = area.width as usize;
        let state = if self.ended { "■ ended" } else { "● live" };
        let state_color = if self.ended { theme.gray } else { theme.accent_running };
        buf.set_span(area.x, area.y, &Span::styled(state, Style::default().fg(state_color).add_modifier(Modifier::BOLD)), area.width);
        let title = truncate_str(&header(&self.root, None), width.saturating_sub(state.width() + 2));
        buf.set_span(area.x + state.width() as u16 + 2, area.y, &Span::styled(title, Style::default().fg(theme.text_primary)), area.width);

        let paused = self.paused().then(|| format!("paused · {} new · G live", self.rows.len().saturating_sub(self.paused_len)));
        let status = [Some(paused.unwrap_or_else(|| "live tail".to_string())), self.filter_name().map(|n| format!("agent: {n}")), self.err.as_ref().map(|e| format!("index: {e}"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        buf.set_span(area.x, area.y + area.height - 1, &Span::styled(truncate_str(&status, width), Style::default().fg(theme.gray)), area.width);

        let body = Rect { y: area.y + 1, height: area.height - 2, ..area };
        let hgt = body.height as usize;
        let vis = self.visible();
        if vis.is_empty() {
            let msg = if self.connected { "no activity yet" } else { "connecting…" };
            buf.set_span(body.x, body.y, &Span::styled(msg, Style::default().fg(theme.gray)), body.width);
            return;
        }
        let cursor_depth = self.cursor.and_then(|c| self.rows.get(c)).map_or(0, |r| r.depth.min(MAX_INDENT));
        let cursor_body = self.body_rows(index, width.saturating_sub(2 * cursor_depth));
        let lines_at = |me: &Self, pos: usize| {
            let i = vis[pos];
            let b = if me.cursor == Some(i) { cursor_body.as_deref() } else { None };
            me.row_lines(i, width, now_ms, theme, b)
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
            for line in lines_at(self, pos) {
                if y >= body.y + body.height {
                    return;
                }
                if selected {
                    buf.set_style(Rect { x: body.x, y, width: body.width, height: 1 }, Style::default().bg(theme.bg_highlight));
                }
                buf.set_line(body.x, y, &line, body.width);
                y += 1;
            }
        }
    }
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
            "● live",
            "▶ you  fix the parser (+1 lines)",
            "~ the tokenizer",
            "· I will patch",
            "→ Bash  run the tests",
            "✓ 0.4s",
            "  ↳ haiku-4-5 started  Check the lexer",
            "  → Read  src/lexer.rs",
            "✗ exit 1  no such file",
            "… 12s",
        ] {
            assert!(screen.contains(want), "missing {want:?}\n{screen}");
        }
        assert!(screen.lines().any(|l| l.starts_with("  → Read")), "the subagent's rows sit two columns in\n{screen}");
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
        assert!(screen(&mut f, 70, 16).contains("live tail"));
        let short = screen(&mut f, 70, 5);
        assert!(short.contains("later still") && !short.contains("▶ you"), "a pinned short screen shows the newest rows\n{short}");
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
