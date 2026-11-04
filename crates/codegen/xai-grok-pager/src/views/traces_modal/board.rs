//! The sessions board: grok's Dashboard look over the traces index. Views pick how sessions are grouped and sorted
//! (live then recent, longest, failed, by project, by harness); density switches between two-line and one-line rows;
//! `e` opens a session in place to show its details and subagents; Enter opens it in grok's session view.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Local, TimeZone};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::{SessionRow, Status, source_label};

use super::board_data::{ActiveTimes, Summaries};
use super::{TracesModalState, Typing, clean_name, fmt_age, now_ms, one_line};
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    #[default]
    Recent,
    Longest,
    Failed,
    Projects,
    Harness,
}

impl View {
    pub const ALL: [View; 5] = [View::Recent, View::Longest, View::Failed, View::Projects, View::Harness];

    pub fn label(self) -> &'static str {
        match self {
            View::Recent => "Live · Recent",
            View::Longest => "Longest",
            View::Failed => "Failed",
            View::Projects => "Projects",
            View::Harness => "Harness",
        }
    }
}

pub struct Board {
    pub view: View,
    pub compact: bool,
    /// Selected line: a group key or a session id.
    sel: Option<String>,
    scroll: usize,
    /// Sessions opened in place with `e`.
    open: HashSet<String>,
    /// Collapsed groups, by `view/label`.
    collapsed: HashSet<String>,
    /// Group keys that already got their default (the old "Earlier" bucket starts closed).
    seeded: HashSet<String>,
    pub active: ActiveTimes,
    pub summaries: Summaries,
}

impl Board {
    pub fn new(summaries: bool) -> Self {
        Board {
            view: View::Recent,
            compact: false,
            sel: None,
            scroll: 0,
            open: HashSet::new(),
            collapsed: HashSet::new(),
            seeded: HashSet::new(),
            active: ActiveTimes::new(),
            summaries: Summaries::load(summaries),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Line {
    Header { key: String, label: String, count: usize, collapsed: bool },
    Row { row: SessionRow, depth: usize, kids: usize },
    Detail(String),
    Gap,
}

impl Line {
    fn key(&self) -> Option<&str> {
        match self {
            Line::Header { key, .. } => Some(key),
            Line::Row { row, .. } => Some(&row.id),
            _ => None,
        }
    }
}

/// Ended subagents listed under an open session. Every running one is listed; the rest are a count.
const MAX_SHOWN_KIDS: usize = 12;
/// Roots kept per machine. The header says so when a list is this long.
pub(super) const BOARD_LIMIT: usize = 2000;

fn last_at(r: &SessionRow) -> i64 {
    r.last_event_at.or(r.updated_at).unwrap_or(0)
}

/// Running now: the session, or one of its subagents.
fn is_live(r: &SessionRow, kids: &HashMap<String, Vec<SessionRow>>) -> bool {
    r.status == Status::Active || kids.get(&r.id).is_some_and(|k| k.iter().any(|c| c.status == Status::Active))
}

/// Ended on an error (the index's `failed`: a trailing error with nothing after it), itself or in a subagent.
fn is_failed(r: &SessionRow, kids: &HashMap<String, Vec<SessionRow>>) -> bool {
    r.failed || kids.get(&r.id).is_some_and(|k| k.iter().any(|c| c.failed))
}

/// The repo a session ran in: the folder name, with worktree paths folded into their repo, `~` for home.
pub(super) fn project(cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.map(|c| c.trim_end_matches('/')).filter(|c| !c.is_empty()) else { return "no folder".into() };
    let cwd = ["/.claude/worktrees/", "/.worktrees/", "/worktrees/"].iter().fold(cwd, |c, w| c.split(w).next().unwrap_or(c));
    let home = std::env::var("HOME").unwrap_or_default();
    if cwd == home.trim_end_matches('/') || cwd.rsplit('/').nth(1) == Some("Users") {
        return "~".into();
    }
    cwd.rsplit('/').next().unwrap_or(cwd).to_string()
}

/// Running subagents, then up to [`MAX_SHOWN_KIDS`] ended ones. The count is how many ended ones were left out.
pub(super) fn pick_kids(kids: &[SessionRow]) -> (Vec<&SessionRow>, usize) {
    let mut live = Vec::new();
    let mut rest = Vec::new();
    for c in kids {
        if c.status == Status::Active {
            live.push(c);
        } else {
            rest.push(c);
        }
    }
    live.sort_by_key(|c| std::cmp::Reverse(last_at(c)));
    rest.sort_by_key(|c| std::cmp::Reverse(last_at(c)));
    let more = rest.len().saturating_sub(MAX_SHOWN_KIDS);
    live.extend(rest.into_iter().take(MAX_SHOWN_KIDS));
    (live, more)
}

/// "4h05m", "12m", "40s", or "–" when unknown.
pub(super) fn fmt_span(ms: Option<i64>) -> String {
    let Some(ms) = ms.filter(|m| *m > 0) else { return "–".into() };
    let s = ms / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
    }
}

/// Local-day bucket of a time, newest first.
fn day_bucket(at: i64, now: i64) -> (u8, &'static str) {
    let (Some(t), Some(n)) = (Local.timestamp_millis_opt(at).single(), Local.timestamp_millis_opt(now).single()) else {
        return (4, "Earlier");
    };
    let days = n.date_naive().signed_duration_since(t.date_naive()).num_days();
    match days {
        ..=0 => (1, "Today"),
        1 => (2, "Yesterday"),
        _ if days < 7 && t.weekday().num_days_from_monday() < n.weekday().num_days_from_monday() => (3, "Earlier this week"),
        _ => (4, "Earlier"),
    }
}

fn longest_bucket(ms: i64) -> (u8, &'static str) {
    match ms / 60_000 {
        120.. => (0, "Over 2 hours"),
        30.. => (1, "30 minutes to 2 hours"),
        5.. => (2, "5 to 30 minutes"),
        _ => (3, "Under 5 minutes"),
    }
}

impl TracesModalState {
    pub(super) fn active_ms(&self, id: &str) -> Option<i64> {
        self.board.active.ms.get(id).copied()
    }

    /// Roots grouped for the current view, each group's rows in view order.
    fn groups(&self) -> Vec<(String, Vec<&SessionRow>)> {
        let kids = &self.shown_children;
        let now = now_ms();
        let view = self.board.view;
        let roots = self.shown_roots.iter().filter(|r| view != View::Failed || is_failed(r, kids));
        let mut keyed: Vec<((u8, String), &SessionRow)> = roots
            .map(|r| {
                let key = match view {
                    View::Recent | View::Failed if is_live(r, kids) => (0, "Live".to_string()),
                    View::Recent | View::Failed => {
                        let (n, l) = day_bucket(last_at(r), now);
                        (n, l.to_string())
                    }
                    View::Longest => {
                        let (n, l) = longest_bucket(self.active_ms(&r.id).unwrap_or(0));
                        (n, l.to_string())
                    }
                    View::Projects => (0, project(r.cwd.as_deref())),
                    View::Harness => (0, source_label(&r.source).to_string()),
                };
                (key, r)
            })
            .collect();
        match view {
            View::Longest => keyed.sort_by_key(|(_, r)| std::cmp::Reverse(self.active_ms(&r.id).unwrap_or(0))),
            _ => keyed.sort_by_key(|(_, r)| std::cmp::Reverse(last_at(r))),
        }
        // Groups in order of their key, then (for name groups) by their newest session.
        let mut groups: Vec<((u8, String), Vec<&SessionRow>)> = Vec::new();
        for (key, r) in keyed {
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, rows)) => rows.push(r),
                None => groups.push((key, vec![r])),
            }
        }
        groups.sort_by_key(|((n, _), _)| *n);
        groups.into_iter().map(|((_, label), rows)| (label, rows)).collect()
    }

    pub(super) fn board_lines(&self) -> Vec<Line> {
        let b = &self.board;
        let mut out = Vec::new();
        for (label, rows) in self.groups() {
            let key = format!("{:?}/{label}", b.view);
            let collapsed = b.collapsed.contains(&key);
            if !out.is_empty() && !matches!(out.last(), Some(Line::Gap)) {
                out.push(Line::Gap);
            }
            out.push(Line::Header { key, label, count: rows.len(), collapsed });
            if collapsed {
                continue;
            }
            if !b.compact {
                out.push(Line::Gap);
            }
            for r in rows {
                self.push_row(&mut out, r, 0);
                if !b.compact {
                    out.push(Line::Gap);
                }
            }
        }
        if matches!(out.last(), Some(Line::Gap)) {
            out.pop();
        }
        out
    }

    fn push_row(&self, out: &mut Vec<Line>, r: &SessionRow, depth: usize) {
        let kids = self.shown_children.get(&r.id).map(Vec::as_slice).unwrap_or(&[]);
        out.push(Line::Row { row: r.clone(), depth, kids: kids.len().max(r.children as usize) });
        if !self.board.open.contains(&r.id) {
            return;
        }
        for d in self.details(r, kids) {
            out.push(Line::Detail(d));
        }
        // Every running subagent, then the most recent ended ones. A running one is never hidden behind the cap.
        let (shown, more) = pick_kids(kids);
        for c in shown {
            self.push_row(out, c, depth + 1);
        }
        if more > 0 {
            out.push(Line::Detail(format!("{more} ended subagents — Enter opens the session, which lists them")));
        }
    }

    /// What `e` shows under a session: where it ran, how long it worked, what it did, and how it ended.
    fn details(&self, r: &SessionRow, kids: &[SessionRow]) -> Vec<String> {
        let mut where_ = vec![source_label(&r.source).to_string()];
        where_.extend(r.model.clone());
        where_.extend(r.cwd.as_deref().map(|c| c.replace(&std::env::var("HOME").unwrap_or_default(), "~")));
        where_.push(r.host.clone().unwrap_or_else(|| "this Mac".into()));
        let wall = r.last_event_at.or(r.updated_at).zip(r.started_at).map(|(e, s)| e - s);
        let mut work = vec![format!("Worked {} over {}", fmt_span(self.active_ms(&r.id)), fmt_span(wall))];
        work.push(format!("{} calls", r.tool_count));
        if r.error_count > 0 {
            work.push(format!("{} errors", r.error_count));
        }
        if !kids.is_empty() {
            let sub: i64 = kids.iter().filter_map(|c| self.active_ms(&c.id)).sum();
            work.push(format!("{} subagents worked {}", kids.len(), fmt_span(Some(sub))));
        }
        let mut lines = vec![where_.join(" · "), work.join(" · ")];
        if let Some(s) = self.board.summaries.text.get(&r.id) {
            lines.push(format!("Ended: {s}"));
        }
        if let Some(now) = r.now.as_deref().filter(|n| !n.trim().is_empty()) {
            lines.push(format!("Last: {}", one_line(now)));
        }
        lines
    }

    fn sel_index(&self, lines: &[Line]) -> usize {
        let key = self.board.sel.as_deref();
        key.and_then(|k| lines.iter().position(|l| l.key() == Some(k)))
            .or_else(|| lines.iter().position(|l| matches!(l, Line::Row { .. })))
            .or_else(|| lines.iter().position(|l| l.key().is_some()))
            .unwrap_or(0)
    }

    fn board_move(&mut self, delta: isize) {
        let lines = self.board_lines();
        let picks: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.key().is_some()).map(|(i, _)| i).collect();
        if picks.is_empty() {
            return;
        }
        let at = self.sel_index(&lines);
        let pos = picks.iter().position(|&i| i == at).unwrap_or(0) as isize;
        let next = (pos + delta).clamp(0, picks.len() as isize - 1) as usize;
        self.board.sel = picks.get(next).and_then(|&i| lines.get(i)).and_then(Line::key).map(str::to_string);
    }

    fn selected_line(&self) -> Option<Line> {
        let lines = self.board_lines();
        let i = self.sel_index(&lines);
        lines.into_iter().nth(i)
    }

    /// The board's poll: active times and summaries. Returns whether to redraw.
    pub(super) fn poll_board(&mut self, now_ms: i64) -> bool {
        let hosts = self.hosts.clone();
        let mut redraw = self.board.active.poll(&hosts, now_ms);
        redraw |= self.board.summaries.poll();
        if !self.scanning {
            let rows: Vec<&SessionRow> = self.shown_roots.iter().collect();
            self.board.summaries.request(&rows, &hosts);
        }
        redraw
    }

    fn set_view(&mut self, view: View) {
        self.board.view = view;
        self.board.scroll = 0;
        self.board.sel = None;
    }

    fn toggle_open(&mut self) -> bool {
        match self.selected_line() {
            Some(Line::Header { key, .. }) => {
                if !self.board.collapsed.remove(&key) {
                    self.board.collapsed.insert(key);
                }
                true
            }
            Some(Line::Row { row, .. }) => {
                if !self.board.open.remove(&row.id) {
                    self.board.open.insert(row.id);
                }
                true
            }
            _ => false,
        }
    }

    /// `h`: close the open session, else step to its parent row or group.
    fn board_close(&mut self) -> bool {
        let lines = self.board_lines();
        let at = self.sel_index(&lines);
        let Some(Line::Row { row, depth, .. }) = lines.get(at) else { return false };
        if self.board.open.remove(&row.id) {
            return true;
        }
        let parent = lines[..at].iter().rev().find(|l| match l {
            Line::Row { depth: d, .. } => d < depth,
            Line::Header { .. } => true,
            _ => false,
        });
        self.board.sel = parent.and_then(Line::key).map(str::to_string);
        true
    }

    /// The old "Earlier" bucket starts closed, once. Opening it stays open.
    pub(super) fn prepare_board(&mut self) {
        let key = format!("{:?}/Earlier", self.board.view);
        if self.board.seeded.insert(key.clone()) {
            self.board.collapsed.insert(key);
        }
    }

    /// Enter opens the session in grok's session view. A live one keeps following.
    fn board_enter(&mut self) -> InputOutcome {
        match self.selected_line() {
            Some(Line::Header { .. }) => {
                self.toggle_open();
                InputOutcome::Changed
            }
            Some(Line::Row { row, .. }) => {
                let row = Box::new(row);
                let open = if row.source == "grok" { Action::TracesOpenGrok(row) } else { Action::TracesOpenClaude(row) };
                InputOutcome::Action(open)
            }
            _ => InputOutcome::Unchanged,
        }
    }
}

pub(super) fn board_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    state.prepare_board();
    let page = 10;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => state.board_move(1),
        KeyCode::Char('k') | KeyCode::Up => state.board_move(-1),
        KeyCode::PageDown => state.board_move(page),
        KeyCode::PageUp => state.board_move(-page),
        KeyCode::Char('g') | KeyCode::Home => state.board_move(isize::MIN / 2),
        KeyCode::Char('G') | KeyCode::End => state.board_move(isize::MAX / 2),
        KeyCode::Tab | KeyCode::BackTab => {
            let n = View::ALL.len();
            let i = View::ALL.iter().position(|v| *v == state.board.view).unwrap_or(0);
            let next = if key.code == KeyCode::Tab { (i + 1) % n } else { (i + n - 1) % n };
            state.set_view(View::ALL[next]);
        }
        KeyCode::Char(c @ '1'..='5') => state.set_view(View::ALL[c as usize - '1' as usize]),
        KeyCode::Char('c') => state.board.compact = !state.board.compact,
        KeyCode::Char('e' | ' ') | KeyCode::Right | KeyCode::Char('l') => {
            if !state.toggle_open() {
                return InputOutcome::Unchanged;
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if !state.board_close() {
                return InputOutcome::Unchanged;
            }
        }
        KeyCode::Char('/') => state.typing = Typing::Ledger,
        KeyCode::Esc if !state.query.is_empty() => {
            state.query.clear();
            state.deep = false;
            state.refresh();
        }
        KeyCode::Char('H') if !state.hosts.is_empty() => {
            state.host_filter = (state.host_filter + 1) % (state.hosts.len() + 2);
            state.rebuild_ledger();
        }
        KeyCode::Char('R') => {
            state.scanning = true;
            return InputOutcome::Action(Action::TracesRescan);
        }
        KeyCode::Char('r') => {
            let row = match state.selected_line() {
                Some(Line::Row { row, .. }) => Some(row),
                _ => None,
            };
            return state.resume(row);
        }
        KeyCode::Enter => return state.board_enter(),
        _ => return InputOutcome::Unchanged,
    }
    InputOutcome::Changed
}

pub(super) fn board_wheel(state: &mut TracesModalState, down: bool) -> InputOutcome {
    state.prepare_board();
    state.board_move(if down { 3 } else { -3 });
    InputOutcome::Changed
}

fn line_height(l: &Line, compact: bool) -> usize {
    match l {
        Line::Row { depth: 0, .. } if !compact => 2,
        _ => 1,
    }
}

fn host_label(state: &TracesModalState) -> String {
    match state.host_filter {
        0 if state.hosts.is_empty() => "this Mac".into(),
        0 => "all machines".into(),
        1 => "this Mac".into(),
        n => state.hosts.get(n - 2).map(|t| xai_grok_traces::remote::label(t).to_string()).unwrap_or_default(),
    }
}

pub(super) fn render_board(buf: &mut Buffer, area: Rect, state: &mut TracesModalState) {
    let theme = Theme::current();
    let base = Style::default().bg(theme.bg_base);
    for y in area.y..area.y + area.height {
        buf.set_string(area.x, y, " ".repeat(area.width as usize), base);
    }
    if area.height < 6 || area.width < 30 {
        return;
    }
    let pad = 2u16;
    let inner = Rect::new(area.x + pad, area.y, area.width.saturating_sub(pad * 2), area.height);
    let w = inner.width as usize;
    let dim = theme.dim().bg(theme.bg_base);
    let gray = Style::default().fg(theme.gray).bg(theme.bg_base);

    // Top line: what is shown, and the counts on the right (the Dashboard's `◇ 4 idle`).
    state.prepare_board();
    let kids = &state.shown_children;
    let all: Vec<&SessionRow> = state.shown_roots.iter().collect();
    let live = all.iter().filter(|r| is_live(r, kids)).count();
    let failed = all.iter().filter(|r| is_failed(r, kids)).count();
    let capped = state.sessions.len() >= BOARD_LIMIT || state.remote_rows.values().any(|v| v.len() >= BOARD_LIMIT);
    let left = format!(
        "all agents · {}{}{}",
        host_label(state),
        if capped { format!(" · newest {BOARD_LIMIT}") } else { String::new() },
        if state.scanning { " · indexing…" } else { "" }
    );
    buf.set_string(inner.x, inner.y + 1, truncate_str(&left, w), gray);
    let mut parts: Vec<(String, Color)> = Vec::new();
    if live > 0 {
        parts.push((format!("{} {live} live", crate::glyphs::dot_spinner_frames()[0]), theme.accent_running));
    }
    if failed > 0 {
        parts.push((format!("{} {failed} failed", crate::glyphs::diamond_filled()), theme.accent_error));
    }
    parts.push((format!("{} {}", crate::glyphs::diamond_hollow(), all.len()), theme.gray));
    let right_w: usize = parts.iter().map(|(s, _)| s.width()).sum::<usize>() + 3 * parts.len().saturating_sub(1);
    if right_w + left.width() + 2 <= w {
        let mut x = inner.x + (w - right_w) as u16;
        for (i, (s, c)) in parts.iter().enumerate() {
            if i > 0 {
                buf.set_string(x, inner.y + 1, " │ ", gray);
                x += 3;
            }
            buf.set_string(x, inner.y + 1, s, Style::default().fg(*c).bg(theme.bg_base));
            x += s.width() as u16;
        }
    }

    // Views strip, then the search line when searching.
    let mut x = inner.x;
    let vy = inner.y + 3;
    for v in View::ALL {
        let s = v.label();
        let style = if v == state.board.view {
            Style::default().fg(theme.accent_user).bg(theme.bg_base).add_modifier(Modifier::BOLD)
        } else {
            gray
        };
        if (x - inner.x) as usize + s.width() > w {
            break;
        }
        buf.set_string(x, vy, s, style);
        x += s.width() as u16 + 3;
    }
    let mut top = vy + 2;
    if state.typing == Typing::Ledger || !state.query.is_empty() {
        let cursor = if state.typing == Typing::Ledger { "▏" } else { "" };
        let q = format!("/ {}{cursor}", state.query);
        buf.set_string(inner.x, top - 1, truncate_str(&q, w), Style::default().fg(theme.text_primary).bg(theme.bg_base));
        top += 1;
    }

    // Footer: the keys that are not obvious.
    let fy = area.y + area.height - 1;
    let footer = if let Some(msg) = state.status.as_deref() {
        msg.to_string()
    } else {
        let mut keys = vec!["Tab:view".to_string(), format!("c:{}", if state.board.compact { "comfortable" } else { "compact" })];
        if matches!(state.selected_line(), Some(Line::Row { kids, .. }) if kids > 0) {
            keys.push("e:subagents".into());
        } else {
            keys.push("e:details".into());
        }
        if !state.hosts.is_empty() {
            keys.push("H:machine".into());
        }
        keys.push("/:search".into());
        keys.join("  │  ")
    };
    buf.set_string(inner.x, fy, truncate_str(&footer, w), dim);

    // Rows.
    let list = Rect::new(inner.x, top, inner.width, fy.saturating_sub(top + 1));
    let lines = state.board_lines();
    if lines.is_empty() {
        let msg = if state.scanning { "Reading sessions…" } else if state.board.view == View::Failed { "No session ended on an error." } else { "No sessions." };
        buf.set_string(list.x, list.y, msg, dim);
        return;
    }
    let compact = state.board.compact;
    let heights: Vec<usize> = lines.iter().map(|l| line_height(l, compact)).collect();
    let sel = state.sel_index(&lines);
    if state.board.sel.is_none() {
        state.board.sel = lines.get(sel).and_then(Line::key).map(str::to_string);
    }
    // Keep the selected line (and the group header right above a first row) in view.
    let start_of = |i: usize| heights[..i].iter().sum::<usize>();
    let sel_top = start_of(sel);
    let sel_bot = sel_top + heights.get(sel).copied().unwrap_or(1);
    let h = list.height as usize;
    if sel_top < state.board.scroll {
        state.board.scroll = sel_top.saturating_sub(2);
    } else if sel_bot > state.board.scroll + h {
        state.board.scroll = sel_bot - h;
    }
    let now = now_ms();
    let tick = (now / 250) as usize;
    let mut y = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let lh = heights[i];
        if y + lh <= state.board.scroll {
            y += lh;
            continue;
        }
        if y >= state.board.scroll + h {
            break;
        }
        let sy = list.y + (y - state.board.scroll) as u16;
        let selected = i == sel;
        match line {
            Line::Header { label, count, collapsed, .. } => header(buf, Rect::new(list.x, sy, list.width, 1), &theme, label, *count, *collapsed, selected),
            Line::Row { row, depth, kids } => {
                let rows_left = (list.y + list.height).saturating_sub(sy);
                let rect = Rect::new(list.x, sy, list.width, (lh as u16).min(rows_left));
                session_row(buf, rect, state, &theme, row, *depth, *kids, selected, tick, now);
            }
            Line::Detail(text) => {
                let x = list.x + 6;
                buf.set_string(x, sy, truncate_str(text, list.width.saturating_sub(7) as usize), dim);
            }
            Line::Gap => {}
        }
        y += lh;
    }
}

#[allow(clippy::too_many_arguments)]
fn header(buf: &mut Buffer, rect: Rect, theme: &Theme, label: &str, count: usize, collapsed: bool, selected: bool) {
    let style = if selected { Style::default().fg(theme.accent_user) } else { theme.muted() }.bg(theme.bg_base).add_modifier(Modifier::BOLD);
    let glyph = if collapsed { crate::glyphs::disclosure_closed() } else { crate::glyphs::disclosure_open() };
    let head = format!("{glyph} {label}");
    let count = format!(" {count} ");
    let w = rect.width as usize;
    buf.set_string(rect.x, rect.y, truncate_str(&head, w), style);
    let mut x = head.width();
    if x + count.width() < w {
        buf.set_string(rect.x + x as u16, rect.y, &count, theme.dim().bg(theme.bg_base));
        x += count.width();
        buf.set_string(rect.x + x as u16, rect.y, "─".repeat(w - x), Style::default().fg(theme.selection_border).bg(theme.bg_base));
    }
}

#[allow(clippy::too_many_arguments)]
fn session_row(
    buf: &mut Buffer,
    rect: Rect,
    state: &TracesModalState,
    theme: &Theme,
    row: &SessionRow,
    depth: usize,
    kids: usize,
    selected: bool,
    tick: usize,
    now: i64,
) {
    let bg = if selected { theme.bg_highlight } else { theme.bg_base };
    let fill = " ".repeat(rect.width as usize);
    for dy in 0..rect.height {
        buf.set_string(rect.x, rect.y + dy, &fill, Style::default().bg(bg));
    }
    if selected {
        let bar = Style::default().fg(theme.accent_user).bg(bg).add_modifier(Modifier::BOLD);
        for dy in 0..rect.height {
            buf.set_string(rect.x, rect.y + dy, crate::glyphs::selection_bar(), bar);
        }
    }
    let (icon, color) = if row.status == Status::Active {
        let f = crate::glyphs::dot_spinner_frames();
        (f[tick % f.len()], theme.accent_running)
    } else if row.failed {
        (crate::glyphs::diamond_filled(), theme.accent_error)
    } else if row.status == Status::Completed {
        (crate::glyphs::diamond_filled(), theme.accent_success)
    } else {
        (crate::glyphs::diamond_hollow(), theme.gray_dim)
    };
    let indent = 2 + 2 * depth as u16;
    let x0 = rect.x + indent;
    buf.set_string(x0, rect.y, icon, Style::default().fg(color).bg(bg));
    let tx = x0 + 2;
    let right_edge = rect.x + rect.width;
    let dim = theme.dim().bg(bg);
    let text = Style::default().fg(theme.text_primary).bg(bg);

    // Right: the view's measure. Longest shows active time; the others show how long ago.
    let age = row.last_event_at.or(row.updated_at).map_or("–".into(), |t| fmt_age(now - t));
    let active = fmt_span(state.active_ms(&row.id));
    let meta = match (state.board.view, state.board.compact) {
        (View::Longest, _) => active.clone(),
        (_, true) => age.clone(),
        (_, false) => format!("{active} · {age}"),
    };
    let meta_x = right_edge.saturating_sub(meta.width() as u16 + 1);
    buf.set_string(meta_x, rect.y, &meta, Style::default().fg(theme.gray).bg(bg));
    let room = meta_x.saturating_sub(tx + 2) as usize;

    let name = clean_name(row);
    let mut tail: Vec<String> = Vec::new();
    if kids > 0 {
        tail.push(format!("{kids} subagent{}", if kids == 1 { "" } else { "s" }));
    }
    if depth == 0 {
        tail.extend(row.host.clone());
        if state.board.view != View::Projects {
            tail.push(project(row.cwd.as_deref()));
        }
        if state.board.view != View::Harness {
            tail.push(source_label(&row.source).to_string());
        }
    } else {
        tail.extend(row.agent_type.clone().filter(|t| t != "general-purpose" && !name.contains(t.as_str())));
    }
    let tail = if tail.is_empty() { String::new() } else { format!(" · {}", tail.join(" · ")) };
    let summary = state.board.summaries.text.get(&row.id).cloned().or_else(|| row.now.as_deref().map(one_line)).unwrap_or_default();
    let name_w = name.width().min(room.saturating_sub(tail.width().min(room / 2)));
    let shown = truncate_str(&name, name_w);
    buf.set_string(tx, rect.y, &shown, text);
    let mut x = tx + shown.width() as u16;
    let left = room.saturating_sub(shown.width());
    if left > 0 {
        let t = truncate_str(&tail, left);
        buf.set_string(x, rect.y, &t, dim);
        x += t.width() as u16;
    }
    if state.board.compact && depth == 0 && !summary.is_empty() {
        let left = (tx as usize + room).saturating_sub(x as usize + 3);
        if left > 8 {
            buf.set_string(x, rect.y, "   ", dim);
            buf.set_string(x + 3, rect.y, truncate_str(&summary, left), dim);
        }
    }
    if rect.height >= 2 && !summary.is_empty() {
        let s = if selected { Style::default().fg(theme.text_secondary).bg(bg) } else { dim };
        buf.set_string(tx, rect.y + 1, truncate_str(&summary, right_edge.saturating_sub(tx + 1) as usize), s);
    }
    if theme.is_bandless() && selected {
        crate::render::color::force_area_fg(buf, rect, Color::Reset);
        buf.set_style(rect, Style::default().add_modifier(Modifier::REVERSED));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_folds_worktrees_and_home() {
        assert_eq!(project(Some("/Volumes/x/repos/grok-build/.claude/worktrees/traces-proto-b")), "grok-build");
        assert_eq!(project(Some("/Users/someone")), "~");
        assert_eq!(project(Some("/Users/someone/repos/pi-lab/")), "pi-lab");
        assert_eq!(project(None), "no folder");
    }

    #[test]
    fn spans_read_as_words() {
        assert_eq!(fmt_span(None), "–");
        assert_eq!(fmt_span(Some(42_000)), "42s");
        assert_eq!(fmt_span(Some(12 * 60_000)), "12m");
        assert_eq!(fmt_span(Some(4 * 3_600_000 + 5 * 60_000)), "4h05m");
    }

    #[test]
    fn running_subagents_are_all_listed() {
        let rows = vec![
            kid("a", true, 1),
            kid("b", true, 2),
            kid("c", false, 3),
            kid("d", false, 4),
        ];
        let mut many: Vec<SessionRow> = (0..MAX_SHOWN_KIDS + 2).map(|i| kid(&format!("e{i}"), false, i as i64)).collect();
        many.push(kid("live", true, 99));
        let (shown, more) = pick_kids(&many);
        assert!(shown.iter().any(|r| r.id == "live"));
        assert_eq!(shown.iter().filter(|r| r.status == Status::Active).count(), 1);
        assert_eq!(more, 2);
        let (shown, more) = pick_kids(&rows);
        assert_eq!(more, 0);
        assert_eq!(shown[0].id, "b");
    }

    fn kid(id: &str, active: bool, at: i64) -> SessionRow {
        SessionRow {
            id: id.into(),
            status: if active { Status::Active } else { Status::Completed },
            last_event_at: Some(at),
            ..Default::default()
        }
    }

    #[test]
    fn longest_buckets_by_active_time() {
        assert_eq!(longest_bucket(3 * 3_600_000).1, "Over 2 hours");
        assert_eq!(longest_bucket(45 * 60_000).1, "30 minutes to 2 hours");
        assert_eq!(longest_bucket(60_000).1, "Under 5 minutes");
    }
}
