//! The sessions list drawn the way grok's Dashboard draws its agents: state icon, title, age, a dim line under it,
//! `▾ Group N ───` headers. It reads the ledger the modal already keeps (`shown_roots`, `shown_children`: this
//! machine and the hosts file's machines) and leads with what is running now, its running subagents under it.
//!
//! Views (Tab): Live (running, then by day), Longest (active time), Failed, Projects, Harness.
//! * Active time: the gaps between a session's consecutive events summed, each gap capped at five minutes, so
//!   waiting on the user does not count (`xai_grok_traces::active_sql`, run on each machine's own index).
//! * Failed: the index's `failed`: the session's last event is an error with nothing after it, or its harness
//!   recorded it finishing with an error.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::{SessionRow, Status, source_label};

use super::summaries::Summaries;
use super::{TracesModalState, clean_name, fmt_age, now_ms};
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::input::key::key;
use crate::theme::Theme;
use crate::views::shortcuts_bar::{HintItem, ShortcutsBar};

/// Children shown under an opened parent before `+N more`.
const KIDS: usize = 8;
/// Rows a group shows before `+N more` (Projects, Harness, the older days).
const GROUP: usize = 6;
/// How often active times are queried again.
const ACTIVE_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Live,
    Longest,
    Failed,
    Projects,
    Harness,
}

const VIEWS: [(View, &str); 5] = [
    (View::Live, "Live"),
    (View::Longest, "Longest"),
    (View::Failed, "Failed"),
    (View::Projects, "Projects"),
    (View::Harness, "Harness"),
];

pub struct Board {
    pub view: View,
    pub compact: bool,
    /// Key of the selected row (session id, or a `+N more` key).
    sel: Option<String>,
    scroll: usize,
    /// Parents whose subagents are all shown (`e`).
    open: HashSet<String>,
    /// Running parents whose running subagents the user folded away (`e`).
    shut: HashSet<String>,
    /// Groups and parents whose `+N more` was opened.
    more: HashSet<String>,
    active: HashMap<String, i64>,
    active_rx: Option<mpsc::Receiver<HashMap<String, i64>>>,
    active_at: Option<Instant>,
    sums: Summaries,
}

#[derive(Clone)]
enum It {
    Head(String, usize),
    /// `gap`: a blank line after it (comfortable mode), closing a parent and its subagents.
    Row { row: SessionRow, depth: u8, last: bool, kids: usize, parent: Option<String>, gap: bool },
    More { key: String, hidden: usize, depth: u8, gap: bool },
}

impl It {
    fn key(&self) -> Option<String> {
        match self {
            It::Row { row, .. } => Some(row.id.clone()),
            It::More { key, .. } => Some(key.clone()),
            It::Head(..) => None,
        }
    }
}

fn running(r: &SessionRow) -> bool {
    r.status == Status::Active
}

/// The repository a session ran in: its folder's name, with worktrees folded into their repository.
pub fn project(cwd: Option<&str>) -> String {
    let Some(c) = cwd.filter(|c| !c.is_empty()) else { return "no folder".into() };
    let c = c.split("/.claude/worktrees/").next().unwrap_or(c);
    let c = c.split("/.worktrees/").next().unwrap_or(c);
    let home = crate::views::dashboard::render::cached_home().unwrap_or("");
    if c == home {
        return "~".into();
    }
    Path::new(c).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| c.to_string())
}

/// `40s`, `14m`, `2h10m`.
pub fn active_label(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h{:02}m", s / 3600, (s / 60) % 60),
    }
}

fn last_at(r: &SessionRow) -> i64 {
    r.last_event_at.or(r.updated_at).unwrap_or(0)
}

/// `Today`, `Yesterday`, `This week`, `Earlier`, in local time.
fn day_bucket(at_ms: i64) -> &'static str {
    use chrono::{Local, TimeZone};
    let Some(at) = Local.timestamp_millis_opt(at_ms).single() else { return "Earlier" };
    let days = (Local::now().date_naive() - at.date_naive()).num_days();
    match days {
        ..=0 => "Today",
        1 => "Yesterday",
        2..=6 => "This week",
        _ => "Earlier",
    }
}

impl Board {
    pub fn new() -> Self {
        Self {
            view: View::Live,
            compact: false,
            sel: None,
            scroll: 0,
            open: HashSet::new(),
            shut: HashSet::new(),
            more: HashSet::new(),
            active: HashMap::new(),
            active_rx: None,
            active_at: None,
            sums: Summaries::load(),
        }
    }

    /// Collect worker results; start the next active-time query and summaries when due. True to redraw.
    pub fn poll(&mut self, state_rows: &[SessionRow], hosts: &[String], db: Option<&Path>) -> bool {
        let mut redraw = false;
        if let Some(rx) = &self.active_rx {
            for m in rx.try_iter() {
                self.active.extend(m);
                redraw = true;
            }
        }
        if self.active_at.is_none_or(|t| t.elapsed() >= ACTIVE_EVERY)
            && let Some(db) = db
        {
            self.active_at = Some(Instant::now());
            self.active_rx = Some(spawn_active(db.to_path_buf(), hosts.to_vec()));
        }
        redraw |= self.sums.poll();
        self.sums.want(state_rows);
        // The spinner on running rows turns once a poll.
        redraw | state_rows.iter().any(running)
    }

    fn active_of(&self, r: &SessionRow) -> Option<i64> {
        self.active.get(&r.id).copied()
    }

    /// Children of `r` the list shows under it, and how many more there are.
    fn kids_of<'a>(&self, r: &SessionRow, kids: &'a HashMap<String, Vec<SessionRow>>) -> (Vec<&'a SessionRow>, usize) {
        let all = kids.get(&r.id).map(Vec::as_slice).unwrap_or(&[]);
        if self.open.contains(&r.id) {
            let mut v: Vec<&SessionRow> = all.iter().collect();
            v.sort_by_key(|k| (!running(k), -last_at(k)));
            let cap = if self.more.contains(&format!("kids:{}", r.id)) { usize::MAX } else { KIDS };
            let hidden = v.len().saturating_sub(cap);
            v.truncate(cap);
            return (v, hidden);
        }
        if self.shut.contains(&r.id) {
            return (Vec::new(), 0);
        }
        (all.iter().filter(|k| running(k)).collect(), 0)
    }

    fn push_row(&self, out: &mut Vec<It>, r: &SessionRow, kids: &HashMap<String, Vec<SessionRow>>) {
        let n = kids.get(&r.id).map_or(0, Vec::len);
        let (shown, hidden) = self.kids_of(r, kids);
        out.push(It::Row { row: r.clone(), depth: 0, last: false, kids: n, parent: None, gap: shown.is_empty() });
        let count = shown.len();
        for (i, k) in shown.into_iter().enumerate() {
            let n = kids.get(&k.id).map_or(0, Vec::len);
            let last = i + 1 == count && hidden == 0;
            out.push(It::Row { row: k.clone(), depth: 1, last, kids: n, parent: Some(r.id.clone()), gap: last });
        }
        if hidden > 0 {
            out.push(It::More { key: format!("kids:{}", r.id), hidden, depth: 1, gap: true });
        }
    }

    fn group(&self, out: &mut Vec<It>, label: &str, rows: &[&SessionRow], kids: &HashMap<String, Vec<SessionRow>>, cap: usize) {
        if rows.is_empty() {
            return;
        }
        out.push(It::Head(label.to_string(), rows.len()));
        let key = format!("group:{label}");
        let cap = if self.more.contains(&key) { usize::MAX } else { cap };
        for r in rows.iter().take(cap) {
            self.push_row(out, r, kids);
        }
        if rows.len() > cap {
            out.push(It::More { key, hidden: rows.len() - cap, depth: 0, gap: true });
        }
    }

    fn items(&self, roots: &[SessionRow], kids: &HashMap<String, Vec<SessionRow>>) -> Vec<It> {
        let mut out = Vec::new();
        let live = |r: &SessionRow| running(r) || kids.get(&r.id).is_some_and(|k| k.iter().any(running));
        let mut by_recent: Vec<&SessionRow> = roots.iter().collect();
        by_recent.sort_by_key(|r| -last_at(r));
        match self.view {
            View::Live => {
                let now: Vec<&SessionRow> = by_recent.iter().copied().filter(|r| live(r)).collect();
                self.group(&mut out, "Running", &now, kids, usize::MAX);
                for day in ["Today", "Yesterday", "This week", "Earlier"] {
                    let rows: Vec<&SessionRow> = by_recent.iter().copied().filter(|r| !live(r) && day_bucket(last_at(r)) == day).collect();
                    let cap = if day == "Today" { usize::MAX } else { GROUP * 2 };
                    self.group(&mut out, day, &rows, kids, cap);
                }
            }
            View::Longest => {
                let mut rows = by_recent.clone();
                rows.sort_by_key(|r| -self.active_of(r).unwrap_or(-1));
                self.group(&mut out, "Most active time", &rows, kids, 30);
            }
            View::Failed => {
                let mut rows: Vec<&SessionRow> = roots.iter().chain(kids.values().flatten()).filter(|r| r.failed).collect();
                rows.sort_by_key(|r| -last_at(r));
                rows.dedup_by(|a, b| a.id == b.id);
                out.push(It::Head("Failed".into(), rows.len()));
                let cap = if self.more.contains("group:Failed") { usize::MAX } else { 40 };
                for r in rows.iter().take(cap) {
                    let parent = r.parent_id.clone();
                    out.push(It::Row { row: (*r).clone(), depth: 0, last: false, kids: kids.get(&r.id).map_or(0, Vec::len), parent, gap: true });
                }
                if rows.len() > cap {
                    out.push(It::More { key: "group:Failed".into(), hidden: rows.len() - cap, depth: 0, gap: true });
                }
            }
            View::Projects | View::Harness => {
                let name = |r: &SessionRow| {
                    if self.view == View::Projects { project(r.cwd.as_deref()) } else { source_label(&r.source).to_string() }
                };
                let mut order: Vec<String> = Vec::new();
                let mut groups: HashMap<String, Vec<&SessionRow>> = HashMap::new();
                for r in &by_recent {
                    let n = name(r);
                    if !groups.contains_key(&n) {
                        order.push(n.clone());
                    }
                    groups.entry(n).or_default().push(r);
                }
                for n in order {
                    self.group(&mut out, &n, &groups[&n], kids, GROUP);
                }
            }
        }
        out
    }

    /// Lines each item takes, its spacing included.
    fn height(&self, it: &It) -> usize {
        let gap = |g: bool| usize::from(g && !self.compact);
        match it {
            It::Head(..) => 2,
            It::Row { depth: 0, gap: g, .. } if !self.compact => 2 + gap(*g),
            It::Row { gap: g, .. } | It::More { gap: g, .. } => 1 + gap(*g),
        }
    }

    fn selectable(items: &[It]) -> Vec<usize> {
        items.iter().enumerate().filter(|(_, it)| !matches!(it, It::Head(..))).map(|(i, _)| i).collect()
    }

    fn sel_pos(&self, items: &[It], sel: &[usize]) -> usize {
        self.sel
            .as_ref()
            .and_then(|k| sel.iter().position(|&i| items[i].key().as_ref() == Some(k)))
            .unwrap_or(0)
    }

    fn move_by(&mut self, items: &[It], delta: isize) {
        let sel = Self::selectable(items);
        if sel.is_empty() {
            return;
        }
        let pos = self.sel_pos(items, &sel) as isize + delta;
        let pos = pos.clamp(0, sel.len() as isize - 1) as usize;
        self.sel = items[sel[pos]].key();
    }

    fn selected<'a>(&self, items: &'a [It]) -> Option<&'a It> {
        let sel = Self::selectable(items);
        sel.get(self.sel_pos(items, &sel)).map(|&i| &items[i])
    }
}

/// Active times from this machine's index and each host's, keyed as the ledger keys rows.
fn spawn_active(db: std::path::PathBuf, hosts: Vec<String>) -> mpsc::Receiver<HashMap<String, i64>> {
    let (tx, rx) = mpsc::channel();
    let since = now_ms() - 30 * 86_400_000;
    let local_tx = tx.clone();
    std::thread::spawn(move || {
        if let Ok(index) = xai_grok_traces::TraceIndex::open(&db)
            && let Ok(m) = index.active_ms(since)
        {
            let _ = local_tx.send(m);
        }
    });
    for target in hosts {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let sql = xai_grok_traces::active_sql(since);
            let out = std::process::Command::new("ssh")
                .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=3"])
                .arg(&target)
                .arg(format!("sqlite3 \"$HOME/.grok/traces/traces.sqlite\" '{sql}'"))
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            let Ok(out) = out else { return };
            let label = xai_grok_traces::remote::label(&target);
            let m = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| {
                    let (id, ms) = l.rsplit_once('|')?;
                    Some((format!("{label}:{id}"), ms.trim().parse().ok()?))
                })
                .collect();
            let _ = tx.send(m);
        });
    }
    rx
}

pub fn handle_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    let items = state.board.items(&state.shown_roots, &state.shown_children);
    let b = &mut state.board;
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => b.move_by(&items, 1),
        KeyCode::Up | KeyCode::Char('k') => b.move_by(&items, -1),
        KeyCode::PageDown => b.move_by(&items, 10),
        KeyCode::PageUp => b.move_by(&items, -10),
        KeyCode::Char('g') | KeyCode::Home => b.move_by(&items, isize::MIN / 2),
        KeyCode::Char('G') | KeyCode::End => b.move_by(&items, isize::MAX / 2),
        KeyCode::Tab | KeyCode::BackTab => {
            let i = VIEWS.iter().position(|(v, _)| *v == b.view).unwrap_or(0);
            let back = key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
            let n = VIEWS.len();
            b.view = VIEWS[if back { (i + n - 1) % n } else { (i + 1) % n }].0;
            b.scroll = 0;
        }
        KeyCode::Char(c @ '1'..='5') => {
            b.view = VIEWS[c as usize - '1' as usize].0;
            b.scroll = 0;
        }
        KeyCode::Char('c') => b.compact = !b.compact,
        KeyCode::Char('e') => match b.selected(&items) {
            Some(It::Row { row, kids, .. }) if *kids > 0 => {
                let id = row.id.clone();
                let showing_running = !b.open.contains(&id) && !b.shut.contains(&id) && running_kids(state, &id);
                let b = &mut state.board;
                if b.open.remove(&id) || showing_running {
                    b.shut.insert(id);
                } else {
                    b.shut.remove(&id);
                    b.open.insert(id);
                }
            }
            Some(It::Row { parent: Some(p), .. }) => {
                // `e` on a subagent folds its parent back, leaving the parent selected.
                let p = p.clone();
                b.open.remove(&p);
                b.shut.insert(p.clone());
                b.sel = Some(p);
            }
            Some(It::More { key, .. }) => {
                b.more.insert(key.clone());
            }
            _ => return InputOutcome::Unchanged,
        },
        KeyCode::Enter => match b.selected(&items).cloned() {
            Some(It::Row { row, .. }) => return open(state, row),
            Some(It::More { key, .. }) => {
                b.more.insert(key);
            }
            _ => return InputOutcome::Unchanged,
        },
        KeyCode::Char('r') => {
            let row = match b.selected(&items) {
                Some(It::Row { row, .. }) => Some(row.clone()),
                _ => None,
            };
            return state.resume(row);
        }
        KeyCode::Char('R') => {
            state.scanning = true;
            return InputOutcome::Action(Action::TracesRescan);
        }
        _ => return InputOutcome::Unchanged,
    }
    InputOutcome::Changed
}

fn running_kids(state: &TracesModalState, id: &str) -> bool {
    state.shown_children.get(id).is_some_and(|k| k.iter().any(running))
}

/// Enter: grok's own session view for every harness it can read (live ones keep updating); the trajectory otherwise.
fn open(state: &mut TracesModalState, row: SessionRow) -> InputOutcome {
    match row.source.as_str() {
        "grok" => InputOutcome::Action(Action::TracesOpenGrok(Box::new(row))),
        "claude" | "pi" | "dpi" | "codex" => InputOutcome::Action(Action::TracesOpenClaude(Box::new(row))),
        _ => {
            state.open_session(row);
            InputOutcome::Changed
        }
    }
}

pub fn handle_mouse(state: &mut TracesModalState, mouse: &MouseEvent) -> InputOutcome {
    let delta = match mouse.kind {
        MouseEventKind::ScrollDown => 3,
        MouseEventKind::ScrollUp => -3,
        _ => return InputOutcome::Unchanged,
    };
    let items = state.board.items(&state.shown_roots, &state.shown_children);
    state.board.move_by(&items, delta);
    InputOutcome::Changed
}

pub fn render(buf: &mut Buffer, area: Rect, state: &mut TracesModalState) {
    let theme = Theme::current();
    let base = Style::default().bg(theme.bg_base);
    // The agent view under the list (its composer) is already painted: wipe it.
    ratatui::widgets::Clear.render(area, buf);
    buf.set_style(area, base);
    if area.width < 30 || area.height < 6 {
        return;
    }
    let now = now_ms();
    let items = state.board.items(&state.shown_roots, &state.shown_children);
    // Content sits two columns in, as the Dashboard's does.
    let x = area.x + 2;
    let w = area.width.saturating_sub(4);
    render_header(buf, Rect::new(x, area.y, w, 1), state, &theme);
    let footer_y = area.y + area.height - 1;
    let status_h = u16::from(state.status.is_some() || state.scanning);
    let list = Rect::new(x, area.y + 2, w, area.height.saturating_sub(3 + status_h));
    if let Some(msg) = state.status.clone().or_else(|| state.scanning.then(|| "indexing…".to_string())) {
        buf.set_line(x, footer_y - 1, &Line::from(Span::styled(msg, theme.dim().bg(theme.bg_base))), w);
    }
    render_footer(buf, Rect::new(area.x, footer_y, area.width, 1), &theme);

    let b = &mut state.board;
    if items.is_empty() {
        let msg = match b.view {
            View::Failed => "No failed sessions.",
            _ if state.scanning => "Indexing sessions…",
            _ => "No sessions yet.",
        };
        buf.set_line(x, list.y, &Line::from(Span::styled(msg, theme.dim().bg(theme.bg_base))), w);
        return;
    }
    let sel = Board::selectable(&items);
    let pos = b.sel_pos(&items, &sel);
    let sel_idx = sel.get(pos).copied().unwrap_or(0);
    b.sel = items[sel_idx].key();
    // Scroll so the selected item and the header above it stay in view.
    let tops: Vec<usize> = items.iter().scan(0, |acc, it| {
        let t = *acc;
        *acc += b.height(it);
        Some(t)
    }).collect();
    let h = list.height as usize;
    let sel_top = tops[sel_idx];
    let sel_bot = sel_top + b.height(&items[sel_idx]);
    if sel_top < b.scroll {
        b.scroll = if sel_idx > 0 && matches!(items[sel_idx - 1], It::Head(..)) { tops[sel_idx - 1] } else { sel_top };
    } else if sel_bot > b.scroll + h {
        b.scroll = sel_bot.saturating_sub(h);
    }
    for (i, it) in items.iter().enumerate() {
        let top = tops[i];
        let ht = b.height(it);
        if top + ht <= b.scroll || top >= b.scroll + h {
            continue;
        }
        let y0 = list.y as isize + top as isize - b.scroll as isize;
        let selected = i == sel_idx;
        match it {
            It::Head(label, n) => {
                if y0 >= list.y as isize {
                    head(buf, x, y0 as u16, w, label, *n, &theme);
                }
            }
            It::Row { row, depth, last, kids, parent, .. } => {
                let lines = row_lines(b, row, *depth, *last, *kids, parent.as_deref(), state_parent_name(&state.shown_roots, parent.as_deref(), b.view), now, w, &theme);
                let bg = if selected { theme.bg_highlight } else { theme.bg_base };
                for (dy, line) in lines.into_iter().enumerate() {
                    let y = y0 + dy as isize;
                    if y < list.y as isize || y >= (list.y + list.height) as isize {
                        continue;
                    }
                    let y = y as u16;
                    buf.set_style(Rect::new(x, y, w, 1), Style::default().bg(bg));
                    let bar = if selected { crate::glyphs::selection_bar() } else { " " };
                    buf.set_string(x, y, bar, Style::default().fg(theme.accent_user).bg(bg).add_modifier(Modifier::BOLD));
                    buf.set_line(x + 1, y, &line, w.saturating_sub(1));
                    for cx in x..x + w {
                        if let Some(cell) = buf.cell_mut((cx, y)) {
                            cell.set_bg(bg);
                        }
                    }
                }
            }
            It::More { hidden, depth, .. } => {
                if y0 >= list.y as isize {
                    let bg = if selected { theme.bg_highlight } else { theme.bg_base };
                    let y = y0 as u16;
                    buf.set_style(Rect::new(x, y, w, 1), Style::default().bg(bg));
                    let bar = if selected { crate::glyphs::selection_bar() } else { " " };
                    buf.set_string(x, y, bar, Style::default().fg(theme.accent_user).bg(bg));
                    let indent = if *depth > 0 { "    └ " } else { "  " };
                    let text = format!("{indent}{hidden} more");
                    buf.set_string(x + 1, y, text, theme.dim().bg(bg));
                }
            }
        }
    }
}

/// The parent's name, shown beside a failed subagent in the Failed view.
fn state_parent_name(roots: &[SessionRow], parent: Option<&str>, view: View) -> Option<String> {
    if view != View::Failed {
        return None;
    }
    let p = parent?;
    roots.iter().find(|r| r.id == p).map(clean_name)
}

fn head(buf: &mut Buffer, x: u16, y: u16, w: u16, label: &str, n: usize, theme: &Theme) {
    let bg = theme.bg_base;
    let lead = format!("{} {label}", crate::glyphs::disclosure_open());
    let count = format!(" {n} ");
    let used = lead.width() + count.width();
    let rule = "─".repeat((w as usize).saturating_sub(used));
    let line = Line::from(vec![
        Span::styled(lead, theme.muted().bg(bg).add_modifier(Modifier::BOLD)),
        Span::styled(count, theme.dim().bg(bg)),
        Span::styled(rule, Style::default().fg(theme.selection_border).bg(bg)),
    ]);
    buf.set_line(x, y, &line, w);
}

/// Icon and its color: running spins, failed and finished are filled, idle is hollow (the Dashboard's set).
fn icon(row: &SessionRow, theme: &Theme) -> (&'static str, ratatui::style::Color) {
    if running(row) {
        let frames = crate::glyphs::dot_spinner_frames();
        let i = (now_ms() / 1000) as usize % frames.len().max(1);
        (frames.get(i).copied().unwrap_or("●"), theme.accent_running)
    } else if row.failed {
        (crate::glyphs::diamond_filled(), theme.accent_error)
    } else if row.parent_id.is_some() {
        (crate::glyphs::diamond_filled(), theme.accent_success)
    } else {
        (crate::glyphs::diamond_hollow(), theme.gray_dim)
    }
}

/// `Running Bash: cargo build`, from the index's last meaningful event.
fn doing(row: &SessionRow) -> Option<String> {
    let now = row.now.as_deref()?.trim();
    if now.is_empty() {
        return None;
    }
    Some(if running(row) { format!("▸ {now}") } else { now.to_string() })
}

#[allow(clippy::too_many_arguments)]
fn row_lines(
    b: &Board,
    row: &SessionRow,
    depth: u8,
    last: bool,
    kids: usize,
    _parent: Option<&str>,
    parent_name: Option<String>,
    now: i64,
    w: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let w = w.saturating_sub(1) as usize;
    let (glyph, color) = icon(row, theme);
    let title = clean_name(row);
    let age = if running(row) { "now".to_string() } else { fmt_age(now - last_at(row)) };
    let mut meta: Vec<String> = Vec::new();
    if let Some(h) = &row.host {
        meta.push(h.clone());
    }
    if depth == 0 {
        meta.push(source_label(&row.source).to_string());
        if b.view != View::Projects {
            meta.push(project(row.cwd.as_deref()));
        }
    } else if let Some(t) = row.agent_type.as_deref().filter(|t| !t.is_empty() && *t != "general-purpose") {
        meta.push(t.to_string());
    }
    if let Some(a) = b.active_of(row) {
        meta.push(active_label(a));
    }
    let meta = meta.join(" · ");
    let right = format!("{meta}  {age}");
    let summary = if running(row) { doing(row) } else { b.sums.get(row).or_else(|| doing(row)) };
    let title_style = Style::default().fg(theme.text_primary).add_modifier(if depth == 0 { Modifier::BOLD } else { Modifier::empty() });
    let dim = theme.dim();
    let lead = if depth > 0 { format!("   {} ", if last { "└" } else { "├" }) } else { " ".to_string() };
    let lead_w = lead.width() + glyph.width() + 1;
    let right_w = right.width();
    // Narrow: the meta gives way to the title, the age stays.
    let (right, right_w) = if lead_w + 12 + right_w + 2 > w { (format!("  {age}"), age.width() + 2) } else { (right, right_w) };
    let room = w.saturating_sub(lead_w + right_w + 1);
    let mut first = vec![Span::styled(lead, dim), Span::styled(format!("{glyph} "), Style::default().fg(color))];
    let one_line = depth > 0 || b.compact;
    if one_line {
        let (t, _) = split_at_width(&title, room);
        let tw = t.width();
        first.push(Span::styled(t, title_style));
        let left = room.saturating_sub(tw + 3);
        if let Some(s) = summary.as_deref().filter(|_| left > 6) {
            first.push(Span::styled(format!(" — {}", split_at_width(s, left).0), dim));
        }
    } else {
        let (t, _) = split_at_width(&title, room);
        first.push(Span::styled(t, title_style));
    }
    let used: usize = first.iter().map(|s| s.content.width()).sum();
    first.push(Span::raw(" ".repeat(w.saturating_sub(used + right_w))));
    first.push(Span::styled(right, dim));
    let mut lines = vec![Line::from(first)];
    if !one_line {
        let mut second = String::new();
        if let Some(p) = parent_name {
            second.push_str(&format!("in {p} · "));
        }
        match summary.as_deref() {
            Some(s) => second.push_str(s),
            None => second.push_str(&format!("{} calls · {} events", row.tool_count, row.event_count)),
        }
        let badge = if kids > 0 {
            let open = b.open.contains(&row.id);
            format!("{kids} subagent{}{}", if kids == 1 { "" } else { "s" }, if open { "" } else { " (e)" })
        } else {
            String::new()
        };
        let indent = 1 + glyph.width() + 1;
        let room = w.saturating_sub(indent + badge.width() + 2);
        let (s, _) = split_at_width(&second, room);
        let used = indent + s.width();
        let badge_style = Style::default().fg(theme.accent_user);
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(indent)),
            Span::styled(s, dim),
            Span::raw(" ".repeat(w.saturating_sub(used + badge.width()))),
            Span::styled(badge, badge_style),
        ]));
    }
    lines
}

/// The first `w` columns of `s` on one line (`…` when cut), and whether it was cut.
fn split_at_width(s: &str, w: usize) -> (String, bool) {
    let s = s.replace(['\n', '\r', '\t'], " ");
    if s.width() <= w {
        return (s, false);
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw + 1 > w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out = out.trim_end().to_string();
    out.push('…');
    (out, true)
}

fn render_header(buf: &mut Buffer, area: Rect, state: &TracesModalState, theme: &Theme) {
    let bg = theme.bg_base;
    let b = &state.board;
    let mut spans = vec![Span::styled("traces ", Style::default().fg(theme.text_primary).bg(bg).add_modifier(Modifier::BOLD))];
    for (v, name) in VIEWS {
        let style = if v == b.view {
            Style::default().fg(theme.accent_user).bg(bg).add_modifier(Modifier::BOLD)
        } else {
            theme.dim().bg(bg)
        };
        spans.push(Span::styled(format!(" {name} "), style));
    }
    let left = Line::from(spans);
    let all: Vec<&SessionRow> = state.shown_roots.iter().chain(state.shown_children.values().flatten()).collect();
    let run = all.iter().filter(|r| running(r)).count();
    let today = state.shown_roots.iter().filter(|r| day_bucket(last_at(r)) == "Today").count();
    let failed = all.iter().filter(|r| r.failed && day_bucket(last_at(r)) == "Today").count();
    let frames = crate::glyphs::dot_spinner_frames();
    let spin = frames.get((now_ms() / 1000) as usize % frames.len().max(1)).copied().unwrap_or("●");
    let mut chips: Vec<Span> = Vec::new();
    let mut chip = |glyph: &'static str, color, n: usize, label: &str| {
        if !chips.is_empty() {
            chips.push(Span::styled("  ", Style::default().bg(bg)));
        }
        chips.push(Span::styled(format!("{glyph} "), Style::default().fg(color).bg(bg)));
        chips.push(Span::styled(format!("{n} {label}"), theme.dim().bg(bg)));
    };
    chip(spin, theme.accent_running, run, "running");
    chip(crate::glyphs::diamond_hollow(), theme.gray_dim, today, "today");
    if failed > 0 {
        chip(crate::glyphs::diamond_filled(), theme.accent_error, failed, "failed today");
    }
    let right = Line::from(chips);
    buf.set_line(area.x, area.y, &left, area.width);
    let rw = right.width() as u16;
    if left.width() as u16 + rw + 2 <= area.width {
        buf.set_line(area.x + area.width - rw, area.y, &right, rw);
    }
}

fn render_footer(buf: &mut Buffer, area: Rect, theme: &Theme) {
    let _ = theme;
    // Enter, Esc and the arrows are obvious and get no hint.
    let hints = vec![
        HintItem::new(key!('e'), "subagents"),
        HintItem::new(key!(Tab), "view"),
        HintItem::new(key!('c'), "compact"),
        HintItem::new(key!('r'), "resume"),
    ];
    let inner = Rect { x: area.x + 2, width: area.width.saturating_sub(2), ..area };
    ShortcutsBar::new(&hints).render(inner, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, cwd: &str, active: bool, failed: bool, at: i64) -> SessionRow {
        SessionRow {
            id: id.into(),
            source: "claude".into(),
            title: format!("title {id}"),
            cwd: Some(cwd.into()),
            status: if active { Status::Active } else { Status::Completed },
            failed,
            last_event_at: Some(at),
            ..Default::default()
        }
    }

    #[test]
    fn project_folds_worktrees_into_their_repository() {
        assert_eq!(project(Some("/r/grok-build/.claude/worktrees/traces-proto-c")), "grok-build");
        assert_eq!(project(Some("/r/trace-deck")), "trace-deck");
        assert_eq!(project(None), "no folder");
    }

    #[test]
    fn live_view_leads_with_running_and_its_running_subagents() {
        let now = now_ms();
        let roots = vec![row("a", "/r/x", false, false, now - 1000), row("b", "/r/y", true, false, now - 5000)];
        let mut kids = HashMap::new();
        let mut done = row("b1", "/r/y", false, false, now - 9000);
        done.parent_id = Some("b".into());
        let mut live = row("b2", "/r/y", true, false, now);
        live.parent_id = Some("b".into());
        kids.insert("b".to_string(), vec![done, live]);
        let mut board = Board::new();
        let keys: Vec<String> = board.items(&roots, &kids).iter().map(|i| match i {
            It::Head(l, _) => format!("#{l}"),
            other => other.key().unwrap_or_default(),
        }).collect();
        assert_eq!(keys, ["#Running", "b", "b2", "#Today", "a"], "only the running subagent shows under its running parent");
        board.open.insert("b".into());
        let keys: Vec<String> = board.items(&roots, &kids).iter().filter_map(It::key).collect();
        assert_eq!(keys, ["b", "b2", "b1", "a"], "e shows every subagent, running first");
    }

    #[test]
    fn failed_view_lists_failed_subagents_too() {
        let now = now_ms();
        let roots = vec![row("a", "/r/x", false, true, now), row("b", "/r/x", false, false, now)];
        let mut k = row("b1", "/r/x", false, true, now - 10);
        k.parent_id = Some("b".into());
        let kids = HashMap::from([("b".to_string(), vec![k])]);
        let mut board = Board::new();
        board.view = View::Failed;
        let keys: Vec<String> = board.items(&roots, &kids).iter().filter_map(It::key).collect();
        assert_eq!(keys, ["a", "b1"]);
    }
}
