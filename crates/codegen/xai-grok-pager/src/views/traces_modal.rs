//! `/traces`: ledger of local agent sessions → trajectory → raw-line inspector.
//!
//! Queries run synchronously against the local index (cheap); discovery and
//! parsing run in a background effect and refresh the ledger when done.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::{EventFilter, EventRow, RECEIPT_WINDOW_MS, SOURCES, SessionFilter, SessionRow, Sort, Status, TraceIndex, source_label};

use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowState, Shortcut};

pub const TITLE: &str = "Traces";
const PAGE: usize = 500;
const EVENT_PAGE: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    List,
    Trajectory,
    Inspector,
}

/// Which line editor has the keyboard, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Typing {
    No,
    Ledger,
    Events,
}

pub struct TracesModalState {
    pub window: ModalWindowState,
    pub mode: Mode,
    index: Option<TraceIndex>,
    pub scanning: bool,
    pub status: Option<String>,
    /// Index health problem (over cap, ...); shown red under the tabs.
    pub health: Option<String>,
    /// Hook receipts arrived within `RECEIPT_WINDOW_MS`.
    pub live: bool,
    /// Highest receipt id seen; a change means the index moved under us.
    watermark: i64,
    typing: Typing,

    pub source_tab: usize,
    pub query: String,
    deep: bool,
    pub errors_only: bool,
    pub active_only: bool,
    pub sort: Sort,
    pub sessions: Vec<SessionRow>,
    pub selected: usize,
    scroll: usize,

    pub session: Option<SessionRow>,
    pub events: Vec<EventRow>,
    pub event_selected: usize,
    event_scroll: usize,
    pub event_errors_only: bool,
    pub event_query: String,
    tools: Vec<String>,
    pub tool_filter: Option<usize>,

    pub raw: Option<String>,
    raw_scroll: usize,
}

/// What `r` resumes: the harness and its own session id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeTarget {
    pub source: String,
    pub native_id: String,
}

pub fn resume_target(row: &SessionRow) -> Option<ResumeTarget> {
    let stem = row.file_path.file_stem()?.to_str()?;
    let native_id = match row.source.as_str() {
        "claude" => stem.to_string(),
        // rollout-<timestamp>-<uuid>.jsonl
        "codex" => stem.get(stem.len().checked_sub(36)?..)?.to_string(),
        "grok" => row.file_path.parent()?.file_name()?.to_str()?.to_string(),
        _ => return None,
    };
    Some(ResumeTarget { source: row.source.clone(), native_id })
}

fn tab_source(tab: usize) -> Option<&'static str> {
    tab.checked_sub(1).and_then(|i| SOURCES.get(i).copied())
}

impl TracesModalState {
    pub fn open(index: Option<TraceIndex>) -> Self {
        let mut s = Self {
            window: ModalWindowState::new(),
            mode: Mode::List,
            status: index.is_none().then(|| "trace index unavailable".to_string()),
            index,
            health: None,
            live: false,
            watermark: 0,
            scanning: true,
            typing: Typing::No,
            source_tab: 0,
            query: String::new(),
            deep: false,
            errors_only: false,
            active_only: false,
            sort: Sort::Updated,
            sessions: Vec::new(),
            selected: 0,
            scroll: 0,
            session: None,
            events: Vec::new(),
            event_selected: 0,
            event_scroll: 0,
            event_errors_only: false,
            event_query: String::new(),
            tools: Vec::new(),
            tool_filter: None,
            raw: None,
            raw_scroll: 0,
        };
        s.watermark = s.index.as_ref().and_then(|i| i.receipts_watermark().ok()).unwrap_or(0);
        s.refresh();
        s.poll_live();
        s
    }

    /// Live mode, driven by the app's 1s poll while the modal is open. Returns whether to redraw.
    pub fn poll_live(&mut self) -> bool {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        self.poll_live_at(now)
    }

    fn poll_live_at(&mut self, now_ms: i64) -> bool {
        let Some(index) = &self.index else { return false };
        let mark = index.receipts_watermark().unwrap_or(self.watermark);
        let live = index.last_receipt_at().ok().flatten().is_some_and(|at| now_ms - at < RECEIPT_WINDOW_MS);
        let health = index.health();
        let changed = live != self.live || health != self.health;
        (self.live, self.health) = (live, health);
        if mark == self.watermark {
            return changed;
        }
        self.watermark = mark;
        // Ledger: requery in place, following the selected session wherever it moved.
        let keep = self.sessions.get(self.selected).map(|s| s.id.clone());
        self.reload_sessions();
        if let Some(i) = keep.and_then(|id| self.sessions.iter().position(|s| s.id == id)) {
            self.selected = i;
        }
        // Trajectory: requery only when the open session grew; follow if the user was at the bottom.
        if self.mode != Mode::List
            && let (Some(index), Some(open)) = (&self.index, &self.session)
            && let Ok(Some(now)) = index.session(&open.id)
            && now.event_count != open.event_count
        {
            let at_bottom = self.event_selected + 1 >= self.events.len();
            self.load_events();
            if at_bottom {
                self.event_selected = self.events.len().saturating_sub(1);
            }
        }
        true
    }

    /// Re-query the ledger (and the open trajectory) from the index.
    pub fn refresh(&mut self) {
        self.reload_sessions();
        self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
        if self.mode != Mode::List {
            self.load_events();
        }
    }

    fn reload_sessions(&mut self) {
        let Some(index) = &self.index else { return };
        self.health = index.health();
        let filter = SessionFilter {
            source: tab_source(self.source_tab).map(str::to_string),
            text: (!self.query.is_empty()).then(|| self.query.clone()),
            deep: self.deep,
            errors_only: self.errors_only,
            active_only: self.active_only,
            sort: self.sort,
            limit: PAGE,
            ..Default::default()
        };
        match index.list_sessions(&filter) {
            Ok(rows) => self.sessions = rows,
            Err(e) => self.status = Some(format!("query failed: {e}")),
        }
        self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
    }

    pub fn finish_scan(&mut self, summary: String) {
        self.scanning = false;
        self.status = Some(summary);
        self.refresh();
    }

    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
        self.status = Some(delivery.toast_message().into_owned());
    }

    fn open_selected(&mut self) {
        let Some(row) = self.sessions.get(self.selected).cloned() else { return };
        self.session = Some(row);
        self.event_selected = 0;
        self.event_scroll = 0;
        self.event_errors_only = false;
        self.event_query.clear();
        self.tool_filter = None;
        self.tools = match (&self.index, &self.session) {
            (Some(i), Some(s)) => i.facets(&s.id).map(|f| f.tools.into_iter().map(|(t, _)| t).collect()).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.mode = Mode::Trajectory;
        self.load_events();
    }

    fn load_events(&mut self) {
        let (Some(index), Some(session)) = (&self.index, &self.session) else { return };
        let filter = EventFilter {
            text: (!self.event_query.is_empty()).then(|| self.event_query.clone()),
            errors_only: self.event_errors_only,
            tool: self.tool_filter.and_then(|i| self.tools.get(i).cloned()),
            include_noise: false,
            limit: EVENT_PAGE,
            offset: 0,
        };
        match index.list_events(&session.id, &filter) {
            Ok(rows) => self.events = rows,
            Err(e) => self.status = Some(format!("query failed: {e}")),
        }
        if let Ok(Some(row)) = index.session(&session.id) {
            self.session = Some(row);
        }
        self.event_selected = self.event_selected.min(self.events.len().saturating_sub(1));
    }

    fn open_inspector(&mut self) {
        let (Some(index), Some(session), Some(ev)) = (&self.index, &self.session, self.events.get(self.event_selected)) else {
            return;
        };
        self.raw = Some(match index.raw_line(&session.id, ev.line) {
            Ok(Some(line)) => serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or(line),
            Ok(None) => "(line no longer in the source file)".to_string(),
            Err(e) => format!("(could not read source: {e})"),
        });
        self.raw_scroll = 0;
        self.mode = Mode::Inspector;
    }

    /// The raw source line for the inspected event, unformatted.
    fn raw_source_line(&self) -> Option<String> {
        let (index, session, ev) = (self.index.as_ref()?, self.session.as_ref()?, self.events.get(self.event_selected)?);
        index.raw_line(&session.id, ev.line).ok().flatten()
    }

    fn resume(&self, row: Option<&SessionRow>) -> InputOutcome {
        let Some(row) = row else { return InputOutcome::Unchanged };
        match resume_target(row) {
            Some(t) => InputOutcome::Action(Action::TracesResume { source: t.source, native_id: t.native_id }),
            None => InputOutcome::Action(Action::TracesNotice { text: format!("no resume for {}", source_label(&row.source)) }),
        }
    }
}

fn move_sel(sel: &mut usize, len: usize, delta: isize) {
    if len == 0 {
        *sel = 0;
        return;
    }
    *sel = (*sel as isize + delta).clamp(0, len as isize - 1) as usize;
}

/// Returns true when the key was consumed by the line editor.
fn edit_line(text: &mut String, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            text.push(c);
            true
        }
        KeyCode::Backspace => {
            text.pop();
            true
        }
        _ => false,
    }
}

/// Esc in a sub-view or an editor is handled here; only the ledger's Esc closes the modal.
pub fn wants_esc(state: &TracesModalState) -> bool {
    state.typing != Typing::No || state.mode != Mode::List
}

pub fn handle_traces_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    if key.kind == KeyEventKind::Release {
        return InputOutcome::Unchanged;
    }
    match state.typing {
        Typing::Ledger => {
            match key.code {
                KeyCode::Esc => {
                    state.typing = Typing::No;
                    state.query.clear();
                    state.deep = false;
                }
                KeyCode::Enter => {
                    state.typing = Typing::No;
                    state.deep = true;
                }
                _ if edit_line(&mut state.query, key) => state.deep = false,
                _ => return InputOutcome::Unchanged,
            }
            state.selected = 0;
            state.refresh();
            return InputOutcome::Changed;
        }
        Typing::Events => {
            match key.code {
                KeyCode::Esc => {
                    state.typing = Typing::No;
                    state.event_query.clear();
                }
                KeyCode::Enter => state.typing = Typing::No,
                _ if edit_line(&mut state.event_query, key) => {}
                _ => return InputOutcome::Unchanged,
            }
            state.event_selected = 0;
            state.load_events();
            return InputOutcome::Changed;
        }
        Typing::No => {}
    }
    let page = 10;
    match state.mode {
        Mode::List => {
            let len = state.sessions.len();
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => move_sel(&mut state.selected, len, 1),
                KeyCode::Char('k') | KeyCode::Up => move_sel(&mut state.selected, len, -1),
                KeyCode::PageDown => move_sel(&mut state.selected, len, page),
                KeyCode::PageUp => move_sel(&mut state.selected, len, -page),
                KeyCode::Char('g') | KeyCode::Home => state.selected = 0,
                KeyCode::Char('G') | KeyCode::End => move_sel(&mut state.selected, len, isize::MAX / 2),
                KeyCode::Tab | KeyCode::BackTab => {
                    let n = SOURCES.len() + 1;
                    state.source_tab =
                        if key.code == KeyCode::Tab { (state.source_tab + 1) % n } else { (state.source_tab + n - 1) % n };
                    state.selected = 0;
                    state.refresh();
                }
                KeyCode::Char('/') => state.typing = Typing::Ledger,
                KeyCode::Char('e') => {
                    state.errors_only = !state.errors_only;
                    state.refresh();
                }
                KeyCode::Char('a') => {
                    state.active_only = !state.active_only;
                    state.refresh();
                }
                KeyCode::Char('s') => {
                    state.sort = match state.sort {
                        Sort::Updated => Sort::Started,
                        Sort::Started => Sort::Events,
                        Sort::Events => Sort::Duration,
                        Sort::Duration => Sort::Errors,
                        Sort::Errors => Sort::Updated,
                    };
                    state.refresh();
                }
                KeyCode::Char('R') => {
                    state.scanning = true;
                    return InputOutcome::Action(Action::TracesRescan);
                }
                KeyCode::Char('r') => return state.resume(state.sessions.get(state.selected)),
                KeyCode::Enter => state.open_selected(),
                _ => return InputOutcome::Unchanged,
            }
        }
        Mode::Trajectory => {
            let len = state.events.len();
            match key.code {
                KeyCode::Esc => {
                    state.mode = Mode::List;
                    state.session = None;
                    state.events.clear();
                }
                KeyCode::Char('j') | KeyCode::Down => move_sel(&mut state.event_selected, len, 1),
                KeyCode::Char('k') | KeyCode::Up => move_sel(&mut state.event_selected, len, -1),
                KeyCode::PageDown => move_sel(&mut state.event_selected, len, page),
                KeyCode::PageUp => move_sel(&mut state.event_selected, len, -page),
                KeyCode::Char('g') | KeyCode::Home => state.event_selected = 0,
                KeyCode::Char('G') | KeyCode::End => move_sel(&mut state.event_selected, len, isize::MAX / 2),
                KeyCode::Char('e') => {
                    state.event_errors_only = !state.event_errors_only;
                    state.event_selected = 0;
                    state.load_events();
                }
                KeyCode::Char('t') => {
                    state.tool_filter = match state.tool_filter {
                        _ if state.tools.is_empty() => None,
                        None => Some(0),
                        Some(i) if i + 1 < state.tools.len() => Some(i + 1),
                        Some(_) => None,
                    };
                    state.event_selected = 0;
                    state.load_events();
                }
                KeyCode::Char('/') => state.typing = Typing::Events,
                KeyCode::Char('r') => return state.resume(state.session.as_ref()),
                KeyCode::Enter => state.open_inspector(),
                _ => return InputOutcome::Unchanged,
            }
        }
        Mode::Inspector => match key.code {
            KeyCode::Esc => {
                state.mode = Mode::Trajectory;
                state.raw = None;
            }
            KeyCode::Char('j') | KeyCode::Down => state.raw_scroll += 1,
            KeyCode::Char('k') | KeyCode::Up => state.raw_scroll = state.raw_scroll.saturating_sub(1),
            KeyCode::PageDown => state.raw_scroll += page as usize,
            KeyCode::PageUp => state.raw_scroll = state.raw_scroll.saturating_sub(page as usize),
            KeyCode::Char('g') | KeyCode::Home => state.raw_scroll = 0,
            KeyCode::Char('y') => {
                if let Some(text) = state.raw_source_line() {
                    return InputOutcome::Action(Action::TracesCopy { text });
                }
                return InputOutcome::Unchanged;
            }
            _ => return InputOutcome::Unchanged,
        },
    }
    InputOutcome::Changed
}

// ---------------------------------------------------------------- render

fn source_color(source: &str, theme: &Theme) -> Color {
    match source {
        "claude" => theme.accent_assistant,
        "codex" => theme.accent_success,
        "pi" => theme.accent_thinking,
        "omp" => theme.accent_plan,
        "dsh" => theme.accent_user,
        "grok" => theme.accent_model,
        _ => theme.gray_bright,
    }
}

pub fn fmt_duration(ms: Option<i64>) -> String {
    match ms {
        None => "-".into(),
        Some(ms) if ms < 1000 => format!("{ms}ms"),
        Some(ms) if ms < 60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) if ms < 3_600_000 => format!("{}m{:02}s", ms / 60_000, (ms / 1000) % 60),
        Some(ms) => format!("{}h{:02}m", ms / 3_600_000, (ms / 60_000) % 60),
    }
}

pub fn fmt_tokens(n: Option<i64>) -> String {
    match n {
        None => "-".into(),
        Some(n) if n < 1000 => n.to_string(),
        Some(n) if n < 1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        Some(n) => format!("{:.1}m", n as f64 / 1e6),
    }
}

fn fmt_time(ms: Option<i64>, with_date: bool) -> String {
    let Some(dt) = ms.and_then(chrono::DateTime::from_timestamp_millis) else { return "-".into() };
    let local = dt.with_timezone(&chrono::Local);
    local.format(if with_date { "%b %d %H:%M" } else { "%H:%M:%S" }).to_string()
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Draws `cells` left to right, each truncated to its width, separated by one space.
fn put_row(buf: &mut Buffer, area: Rect, y: u16, cells: &[(String, usize, Style)]) {
    let mut x = area.x;
    let right = area.x + area.width;
    for (text, width, style) in cells {
        if x >= right {
            break;
        }
        let w = (*width).min((right - x) as usize);
        let t = truncate_str(text, w);
        buf.set_span(x, y, &Span::styled(t.as_str(), *style), w as u16);
        x = x.saturating_add(w as u16 + 1);
    }
}

fn fill(buf: &mut Buffer, area: Rect, y: u16, style: Style) {
    for x in area.x..area.x + area.width {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
}

fn keep_visible(sel: usize, scroll: &mut usize, height: usize) {
    if height == 0 {
        return;
    }
    if sel < *scroll {
        *scroll = sel;
    } else if sel >= *scroll + height {
        *scroll = sel + 1 - height;
    }
}

fn shortcuts(state: &TracesModalState) -> Vec<Shortcut<'static>> {
    let plain = |label: &'static str| Shortcut { label, clickable: false, id: 0 };
    if state.typing != Typing::No {
        return vec![plain("type to filter"), plain("Enter search events"), plain("Esc clear")];
    }
    match state.mode {
        Mode::List => vec![
            plain("j/k move"),
            plain("Tab source"),
            plain("/ search"),
            plain("e errors"),
            plain("a active"),
            plain("s sort"),
            plain("Enter open"),
            plain("r resume"),
            plain("R rescan"),
            plain("Esc close"),
        ],
        Mode::Trajectory => vec![
            plain("j/k move"),
            plain("Enter inspect"),
            plain("e errors"),
            plain("t tool"),
            plain("/ search"),
            plain("r resume"),
            plain("Esc back"),
        ],
        Mode::Inspector => vec![plain("j/k scroll"), plain("y copy line"), plain("Esc back")],
    }
}

pub fn render_traces_modal(buf: &mut Buffer, full_area: Rect, state: &mut TracesModalState, compact: bool) {
    let theme = Theme::current();
    let shortcuts = shortcuts(state);
    let config = ModalWindowConfig {
        title: TITLE,
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing { width_pct: 1.0, max_width: u16::MAX, min_width: 44, v_margin: 0, h_pad: 2, v_pad: 0, footer_lines: 2 }
            .with_compact(compact),
        fold_info: None,
    };
    let Some(ModalContentArea { content, .. }) = modal_window::render_modal_window(buf, full_area, &mut state.window, &config, &theme)
    else {
        return;
    };
    if content.height < 4 || content.width < 20 {
        return;
    }
    match state.mode {
        Mode::List => render_ledger(buf, content, state, &theme),
        Mode::Trajectory => render_trajectory(buf, content, state, &theme),
        Mode::Inspector => render_inspector(buf, content, state, &theme),
    }
}

fn render_ledger(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let dim = Style::default().fg(theme.gray);
    // Tabs.
    let mut x = area.x;
    for tab in 0..=SOURCES.len() {
        let label = tab_source(tab).map_or("All", source_label);
        let style = if tab == state.source_tab {
            Style::default().fg(theme.bg_base).bg(tab_source(tab).map_or(theme.accent_user, |s| source_color(s, theme))).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        };
        let text = format!(" {label} ");
        if x + text.width() as u16 > area.x + area.width {
            break;
        }
        buf.set_span(x, area.y, &Span::styled(text.as_str(), style), text.width() as u16);
        x += text.width() as u16 + 1;
    }
    let mut flags = Vec::new();
    if state.live {
        flags.push("● live".to_string());
    }
    if state.scanning {
        flags.push("indexing…".to_string());
    }
    if state.errors_only {
        flags.push("errors".into());
    }
    if state.active_only {
        flags.push("active".into());
    }
    flags.push(format!("sort:{:?}", state.sort).to_lowercase());
    flags.push(format!("{} shown", state.sessions.len()));
    let right = flags.join(" · ");
    let rw = right.width() as u16;
    if area.width > rw && x < area.x + area.width - rw {
        buf.set_span(area.x + area.width - rw, area.y, &Span::styled(right.as_str(), dim), rw);
    }

    // Search line.
    let search_y = area.y + 1;
    let (text, style) = match (state.typing == Typing::Ledger, state.query.is_empty()) {
        (true, _) => (format!("/{}▏", state.query), Style::default().fg(theme.text_primary)),
        (false, true) => ("/ to search titles, cwd, model · Enter searches events too".into(), dim),
        (false, false) => (format!("/{}{}", state.query, if state.deep { "  (events)" } else { "" }), Style::default().fg(theme.fuzzy_accent)),
    };
    // A red health line takes the right of the search row; the search hint yields to it.
    let health_w = state.health.as_deref().map_or(0, |h| h.width().min(area.width as usize * 2 / 3)) as u16;
    if let Some(health) = &state.health {
        let red = Style::default().fg(theme.accent_error).add_modifier(Modifier::BOLD);
        buf.set_span(area.x + area.width - health_w, search_y, &Span::styled(truncate_str(health, health_w as usize).as_str(), red), health_w);
    }
    let search_w = area.width.saturating_sub(health_w + u16::from(health_w > 0));
    buf.set_span(area.x, search_y, &Span::styled(truncate_str(&text, search_w as usize).as_str(), style), search_w);

    let status_rows = u16::from(state.status.is_some());
    let list = Rect { y: area.y + 3, height: area.height.saturating_sub(3 + status_rows), ..area };
    let wide = area.width >= 140;
    // Fixed columns plus one separator each, and room for the trailing error count.
    let fixed: usize = [8, 7, 5, 5, 6, 12, 12].iter().map(|w| w + 1).sum::<usize>() + if wide { 29 } else { 0 } + 9;
    let title_w = (area.width as usize).saturating_sub(fixed + 1);
    let cols = |title: String, s: &SessionRow, base: Style| -> Vec<(String, usize, Style)> {
        let mut v = vec![
            (source_label(&s.source).to_string(), 8, base.fg(source_color(&s.source, theme))),
            (title, title_w, base),
            (s.status.as_str().to_string(), 7, base.fg(if s.status == Status::Active { theme.accent_running } else { theme.gray })),
            (format!("{:>5}", s.event_count), 5, base.fg(theme.text_secondary)),
            (format!("{:>5}", s.tool_count), 5, base.fg(theme.accent_tool)),
            (format!("{:>6}", fmt_duration(s.duration_ms)), 6, base.fg(theme.text_secondary)),
            (format!("{:>5}/{:<6}", fmt_tokens(s.input_tokens), fmt_tokens(s.output_tokens)), 12, base.fg(theme.gray_bright)),
            (fmt_time(s.updated_at, true), 12, base.fg(theme.text_secondary)),
        ];
        if wide {
            v.push((s.cwd.clone().or_else(|| s.model.clone()).unwrap_or_default(), 28, base.fg(theme.path)));
        }
        if s.error_count > 0 {
            v.push((format!("{} err", s.error_count), 8, base.fg(theme.accent_error)));
        }
        v
    };
    put_row(
        buf,
        area,
        area.y + 2,
        &[
            ("SOURCE".into(), 8, dim),
            ("TITLE".into(), title_w, dim),
            ("STATUS".into(), 7, dim),
            ("  EVT".into(), 5, dim),
            ("TOOLS".into(), 5, dim),
            ("   DUR".into(), 6, dim),
            ("   IN/OUT".into(), 12, dim),
            ("UPDATED".into(), 12, dim),
            (if wide { "CWD" } else { "" }.into(), if wide { 28 } else { 0 }, dim),
        ],
    );
    if state.sessions.is_empty() {
        let msg = if state.scanning { "Indexing local sessions…" } else { "No sessions match." };
        buf.set_span(list.x, list.y, &Span::styled(msg, dim), list.width);
    }
    keep_visible(state.selected, &mut state.scroll, list.height as usize);
    for (i, s) in state.sessions.iter().enumerate().skip(state.scroll).take(list.height as usize) {
        let y = list.y + (i - state.scroll) as u16;
        let selected = i == state.selected;
        let base = if selected { Style::default().bg(theme.bg_highlight).fg(theme.text_primary) } else { Style::default().fg(theme.text_primary) };
        if selected {
            fill(buf, list, y, base);
        }
        put_row(buf, list, y, &cols(one_line(&s.title), s, base));
    }
    render_status(buf, area, state, theme);
}

fn render_status(buf: &mut Buffer, area: Rect, state: &TracesModalState, theme: &Theme) {
    if let Some(status) = &state.status {
        let y = area.y + area.height - 1;
        buf.set_span(
            area.x,
            y,
            &Span::styled(truncate_str(status, area.width as usize).as_str(), Style::default().fg(theme.accent_user).add_modifier(Modifier::BOLD)),
            area.width,
        );
    }
}

fn role_chip(e: &EventRow) -> &'static str {
    match (e.kind.as_str(), e.role.as_deref()) {
        (_, Some("tool_result")) => "RESULT",
        ("tool", _) => "TOOL",
        (_, Some("user")) => "USER",
        (_, Some("assistant")) => "ASSIST",
        ("reasoning", _) => "THINK",
        ("system", _) => "SYSTEM",
        ("step", _) => "STEP",
        ("turn", _) => "TURN",
        ("telemetry", _) => "TOKENS",
        _ => "EVENT",
    }
}

/// 0 = input, 1 = model, 2 = tools (the bb Traces lanes).
fn lane(e: &EventRow) -> usize {
    if e.kind == "tool" {
        2
    } else if e.role.as_deref() == Some("user") || e.kind == "system" || e.kind == "reasoning" {
        0
    } else {
        1
    }
}

fn render_timeline(buf: &mut Buffer, area: Rect, events: &[EventRow], selected: usize, theme: &Theme) {
    const LABELS: [&str; 3] = ["INPUT", "MODEL", "TOOLS"];
    let gutter = 7u16;
    let width = area.width.saturating_sub(gutter) as usize;
    let dim = Style::default().fg(theme.gray_dim);
    for (row, label) in LABELS.iter().enumerate() {
        let y = area.y + row as u16;
        buf.set_span(area.x, y, &Span::styled(*label, Style::default().fg(theme.gray)), gutter);
        buf.set_span(area.x + gutter, y, &Span::styled("·".repeat(width).as_str(), dim), width as u16);
    }
    if width == 0 || events.is_empty() {
        return;
    }
    let stamps: Vec<i64> = events.iter().filter_map(|e| e.timestamp).collect();
    let (start, end) = match (stamps.iter().min(), events.iter().filter_map(|e| e.timestamp.map(|t| t + e.duration_ms.unwrap_or(1))).max()) {
        (Some(&s), Some(e)) => (s, e),
        _ => (0, 0),
    };
    let span = (end - start).max(1) as f64;
    let col = |i: usize, e: &EventRow| -> usize {
        let frac = match e.timestamp {
            Some(t) if !stamps.is_empty() => (t - start) as f64 / span,
            _ => i as f64 / events.len().max(1) as f64,
        };
        ((frac * (width - 1) as f64).round() as usize).min(width - 1)
    };
    let colors = [theme.accent_user, theme.accent_assistant, theme.accent_tool];
    for (i, e) in events.iter().enumerate() {
        let l = lane(e);
        let c = col(i, e);
        let fg = if e.is_error { theme.accent_error } else { colors[l] };
        buf.set_span(area.x + gutter + c as u16, area.y + l as u16, &Span::styled("●", Style::default().fg(fg)), 1);
    }
    if let Some(e) = events.get(selected) {
        let c = col(selected, e);
        buf.set_span(area.x + gutter + c as u16, area.y + lane(e) as u16, &Span::styled("◆", Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), 1);
    }
}

fn render_trajectory(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let Some(session) = state.session.clone() else { return };
    let dim = Style::default().fg(theme.gray);
    let chip = format!(" {} ", source_label(&session.source));
    let cw = chip.width() as u16;
    buf.set_span(area.x, area.y, &Span::styled(chip.as_str(), Style::default().fg(theme.bg_base).bg(source_color(&session.source, theme)).add_modifier(Modifier::BOLD)), cw);
    let meta = format!(
        "  {} events · {} tools · {} · {} · {}",
        session.event_count,
        session.tool_count,
        fmt_duration(session.duration_ms),
        session.status.as_str(),
        session.model.as_deref().unwrap_or("-")
    );
    let title_w = (area.width.saturating_sub(cw + 1) as usize).saturating_sub(meta.width());
    buf.set_span(area.x + cw + 1, area.y, &Span::styled(truncate_str(&one_line(&session.title), title_w).as_str(), Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), title_w as u16);
    let mw = meta.width() as u16;
    if area.width > mw {
        buf.set_span(area.x + area.width - mw, area.y, &Span::styled(meta.as_str(), dim), mw);
    }
    render_timeline(buf, Rect { y: area.y + 1, height: 3, ..area }, &state.events, state.event_selected, theme);

    let mut filters = Vec::new();
    if state.event_errors_only {
        filters.push("errors only".to_string());
    }
    if let Some(t) = state.tool_filter.and_then(|i| state.tools.get(i)) {
        filters.push(format!("tool:{t}"));
    }
    let search = match state.typing {
        Typing::Events => format!("/{}▏", state.event_query),
        _ if !state.event_query.is_empty() => format!("/{}", state.event_query),
        _ => String::new(),
    };
    if !search.is_empty() {
        filters.push(search);
    }
    let line = if filters.is_empty() {
        session.cwd.clone().unwrap_or_else(|| session.file_path.display().to_string())
    } else {
        format!("{} · {} matching", filters.join(" · "), state.events.len())
    };
    buf.set_span(area.x, area.y + 4, &Span::styled(truncate_str(&line, area.width as usize).as_str(), Style::default().fg(theme.fuzzy_accent)), area.width);

    let status_rows = u16::from(state.status.is_some());
    let list = Rect { y: area.y + 5, height: area.height.saturating_sub(5 + status_rows), ..area };
    if state.events.is_empty() {
        buf.set_span(list.x, list.y, &Span::styled("No events match.", dim), list.width);
    }
    keep_visible(state.event_selected, &mut state.event_scroll, list.height as usize);
    let summary_w = (list.width as usize).saturating_sub(7 + 7 + 9 + 2);
    let mut prev_turn = None;
    for (i, e) in state.events.iter().enumerate().skip(state.event_scroll).take(list.height as usize) {
        let y = list.y + (i - state.event_scroll) as u16;
        let selected = i == state.event_selected;
        let base = if selected { Style::default().bg(theme.bg_highlight).fg(theme.text_primary) } else { Style::default().fg(theme.text_primary) };
        if selected {
            fill(buf, list, y, base);
        }
        let chip_fg = if e.is_error {
            theme.accent_error
        } else {
            match lane(e) {
                0 => theme.accent_user,
                1 => theme.accent_assistant,
                _ => theme.accent_tool,
            }
        };
        let indent = "  ".repeat(e.depth.clamp(0, 2) as usize);
        let body = if e.kind == "tool" && e.role.as_deref() == Some("tool") {
            format!("{indent}{}  {}", e.title, one_line(&e.preview))
        } else {
            format!("{indent}{}", one_line(&e.preview))
        };
        let turn_mark = if e.turn.is_some() && e.turn != prev_turn { "┊" } else { " " };
        prev_turn = e.turn.or(prev_turn);
        put_row(
            buf,
            list,
            y,
            &[
                (format!("{:>6}", format!("#{}", e.line)), 6, base.fg(theme.gray)),
                (role_chip(e).to_string(), 6, base.fg(chip_fg).add_modifier(Modifier::BOLD)),
                (format!("{turn_mark}{body}"), summary_w, base),
                (fmt_time(e.timestamp, false), 8, base.fg(theme.text_secondary)),
            ],
        );
    }
    render_status(buf, area, state, theme);
}

fn render_inspector(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let dim = Style::default().fg(theme.gray);
    if let Some(e) = state.events.get(state.event_selected) {
        let head = format!("{} · line {} · {} · {}", e.event_type, e.line, fmt_time(e.timestamp, true), fmt_duration(e.duration_ms));
        buf.set_span(area.x, area.y, &Span::styled(truncate_str(&head, area.width as usize).as_str(), Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), area.width);
    }
    let Some(raw) = &state.raw else { return };
    let lines: Vec<&str> = raw.lines().collect();
    let status_rows = u16::from(state.status.is_some());
    let body = Rect { y: area.y + 2, height: area.height.saturating_sub(2 + status_rows), ..area };
    state.raw_scroll = state.raw_scroll.min(lines.len().saturating_sub(body.height as usize));
    for (i, line) in lines.iter().skip(state.raw_scroll).take(body.height as usize).enumerate() {
        let style = if line.trim_start().starts_with('"') { Style::default().fg(theme.text_primary) } else { dim };
        buf.set_span(body.x, body.y + i as u16, &Span::styled(truncate_str(line, body.width as usize).as_str(), style), body.width);
    }
    render_status(buf, area, state, theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source: &str, path: &str) -> SessionRow {
        SessionRow {
            id: "x".into(),
            source: source.into(),
            title: "t".into(),
            file_path: path.into(),
            model: None,
            cwd: None,
            started_at: None,
            updated_at: None,
            event_count: 0,
            user_count: 0,
            assistant_count: 0,
            tool_count: 0,
            error_count: 0,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            status: Status::Unknown,
            parent_id: None,
            label: None,
            agent_type: None,
            now: None,
            last_event_at: None,
            children: 0,
            failed: false,
        }
    }

    #[test]
    fn resume_targets() {
        let c = resume_target(&row("claude", "/h/.claude/projects/-w/3a680ac6-2551-4c67-88d3-a18d8b3bbe73.jsonl")).unwrap();
        assert_eq!(c.native_id, "3a680ac6-2551-4c67-88d3-a18d8b3bbe73");
        let x = resume_target(&row("codex", "/h/.codex/sessions/2026/06/06/rollout-2026-06-06T16-07-29-019e9f31-031e-7fb0-9562-b793939a558a.jsonl")).unwrap();
        assert_eq!(x.native_id, "019e9f31-031e-7fb0-9562-b793939a558a");
        let g = resume_target(&row("grok", "/h/.grok/sessions/%2Fw/01a10b35-4b73-7903-a589-181aba7b40f5/chat_history.jsonl")).unwrap();
        assert_eq!(g.native_id, "01a10b35-4b73-7903-a589-181aba7b40f5");
        assert_eq!(resume_target(&row("dsh", "/h/.dsh/sessions/a/b/session.v2.jsonl.zstd")), None);
        assert_eq!(resume_target(&row("bb", "/h/.bb/bb.db#thr_x")), None);
        assert_eq!(source_label("bb"), "bb");
    }

    fn line(ts: &str, text: &str) -> String {
        format!("{{\"type\":\"message\",\"timestamp\":\"{ts}\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n")
    }

    /// Three pi sessions (a < b < c by time), indexed, with a modal and a writer handle on one db.
    fn live_fixture() -> (tempfile::TempDir, [xai_grok_traces::Root; 1], TraceIndex, TracesModalState) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pi");
        std::fs::create_dir_all(&dir).unwrap();
        for (n, ts) in [("a", "2026-10-01T00:00:01Z"), ("b", "2026-10-01T00:00:02Z"), ("c", "2026-10-01T00:00:03Z")] {
            std::fs::write(dir.join(format!("{n}.jsonl")), line(ts, &format!("task {n}"))).unwrap();
        }
        let roots = [xai_grok_traces::Root {
            id: "pi".into(),
            source: "pi".into(),
            path: dir,
            format: xai_grok_traces::Format::Jsonl,
            file_name: None,
        }];
        let db = tmp.path().join("idx/t.sqlite");
        let mut writer = TraceIndex::open(&db).unwrap();
        writer.scan(&roots).unwrap();
        let state = TracesModalState::open(Some(TraceIndex::open(&db).unwrap()));
        (tmp, roots, writer, state)
    }

    fn hook(writer: &mut TraceIndex, roots: &[xai_grok_traces::Root], file: &std::path::Path, append: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(file).unwrap().write_all(append.as_bytes()).unwrap();
        let payload = format!(r#"{{"hook_event_name":"PostToolUse","transcript_path":"{}"}}"#, file.display());
        xai_grok_traces::hook::ingest(writer, roots, xai_grok_traces::hook::extract("pi", payload.as_bytes())).unwrap();
    }

    fn titles(state: &TracesModalState) -> Vec<&str> {
        state.sessions.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn live_poll_keeps_selection_by_session_id() {
        let (_tmp, roots, mut writer, mut state) = live_fixture();
        assert_eq!(titles(&state), ["task c", "task b", "task a"]);
        assert!(!state.live);
        state.selected = 1;
        assert!(!state.poll_live(), "nothing changed");

        let a = roots[0].path.join("a.jsonl");
        hook(&mut writer, &roots, &a, &line("2026-10-01T00:00:09Z", "later"));
        assert!(state.poll_live());
        assert_eq!(titles(&state), ["task a", "task c", "task b"], "a moved to the top");
        assert_eq!((state.selected, state.sessions[state.selected].title.as_str()), (2, "task b"));
        assert!(state.live, "fresh receipt shows the live marker");
        assert!(state.poll_live_at(i64::MAX / 2), "marker flips off after the window");
        assert!(!state.live);
    }

    #[test]
    fn live_poll_appends_events_and_follows_the_bottom() {
        let (_tmp, roots, mut writer, mut state) = live_fixture();
        let a = roots[0].path.join("a.jsonl");
        state.selected = 2; // task a
        state.open_selected();
        assert_eq!(state.events.len(), 1);

        hook(&mut writer, &roots, &a, &line("2026-10-01T00:00:05Z", "second"));
        assert!(state.poll_live());
        assert_eq!((state.events.len(), state.event_selected), (2, 1), "at the bottom: follows");

        state.event_selected = 0;
        hook(&mut writer, &roots, &a, &line("2026-10-01T00:00:06Z", "third"));
        assert!(state.poll_live());
        assert_eq!((state.events.len(), state.event_selected), (3, 0), "mid-list: stays put");
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_duration(Some(61_000)), "1m01s");
        assert_eq!(fmt_tokens(Some(12_345)), "12.3k");
    }
}
