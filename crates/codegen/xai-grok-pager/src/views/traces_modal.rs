//! `/traces`: ledger of local agent sessions → trajectory → raw-line inspector.
//!
//! Queries run synchronously against the local index (cheap); discovery and
//! parsing run in a background effect and refresh the ledger when done.

use std::collections::{HashMap, HashSet};

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
/// Children shown under a parent before `+N more`.
const MAX_KIDS: usize = 6;

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
    /// `a`: every session, flat (children and trivial ones included).
    pub show_all: bool,
    /// Root sessions as queried.
    pub sessions: Vec<SessionRow>,
    children: HashMap<String, Vec<SessionRow>>,
    expanded: HashSet<String>,
    /// What the ledger draws: roots with their children beneath. `selected` indexes this.
    ledger: Vec<Item>,
    pub selected: usize,
    scroll: usize,

    pub session: Option<SessionRow>,
    pub events: Vec<EventRow>,
    pub event_selected: usize,
    event_scroll: usize,
    pub event_errors_only: bool,
    pub event_query: String,
    /// `n`: show system/bookkeeping rows too, dimmed.
    pub include_noise: bool,
    /// Per event: the tool result folded into that call row.
    folded: Vec<Option<Fold>>,
    /// Children of the open session (dispatch targets), oldest first.
    session_children: Vec<SessionRow>,
    parent_name: Option<String>,
    /// Sessions we descended from with Enter on a dispatch row, and the row we were on.
    trail: Vec<(SessionRow, usize)>,
    tools: Vec<String>,
    pub tool_filter: Option<usize>,

    pub raw: Option<String>,
    raw_scroll: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Branch {
    Root,
    Mid,
    Last,
}

#[derive(Clone, Debug)]
enum Item {
    Row { row: SessionRow, branch: Branch },
    More { parent: String, hidden: usize },
}

/// A tool result folded into the call row before it.
#[derive(Clone, Debug, PartialEq)]
struct Fold {
    error: bool,
    detail: Option<String>,
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
            show_all: false,
            sessions: Vec::new(),
            children: HashMap::new(),
            expanded: HashSet::new(),
            ledger: Vec::new(),
            selected: 0,
            scroll: 0,
            session: None,
            events: Vec::new(),
            event_selected: 0,
            event_scroll: 0,
            event_errors_only: false,
            event_query: String::new(),
            include_noise: false,
            folded: Vec::new(),
            session_children: Vec::new(),
            parent_name: None,
            trail: Vec::new(),
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
        let keep = self.selected_session().map(|s| s.id.clone());
        self.reload_sessions();
        if let Some(i) = keep.and_then(|id| self.ledger.iter().position(|it| matches!(it, Item::Row { row, .. } if row.id == id))) {
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
        if self.mode != Mode::List {
            self.load_events();
        }
    }

    /// Roots-only, trivial-hidden tree view; searching or `a` flattens to every session.
    fn browse(&self) -> bool {
        !self.show_all && self.query.is_empty()
    }

    fn reload_sessions(&mut self) {
        let Some(index) = &self.index else { return };
        self.health = index.health();
        let browse = self.browse();
        let filter = SessionFilter {
            source: tab_source(self.source_tab).map(str::to_string),
            text: (!self.query.is_empty()).then(|| self.query.clone()),
            deep: self.deep,
            errors_only: self.errors_only,
            active_only: self.active_only,
            sort: self.sort,
            limit: PAGE,
            roots_only: browse,
            hide_trivial: browse,
            ..Default::default()
        };
        match index.list_sessions(&filter) {
            Ok(rows) => self.sessions = rows,
            Err(e) => self.status = Some(format!("query failed: {e}")),
        }
        self.children.clear();
        if browse {
            for r in self.sessions.iter().filter(|r| r.children > 0) {
                if let Ok(kids) = index.children(&r.id) {
                    self.children.insert(r.id.clone(), kids);
                }
            }
            if self.sort == Sort::Updated {
                // Running work first (a finished parent with a running child counts), then by recency.
                let running = |r: &SessionRow, kids: Option<&Vec<SessionRow>>| {
                    r.status == Status::Active || kids.is_some_and(|k| k.iter().any(|c| c.status == Status::Active))
                };
                let flags: HashMap<String, bool> =
                    self.sessions.iter().map(|r| (r.id.clone(), running(r, self.children.get(&r.id)))).collect();
                self.sessions.sort_by_key(|r| !flags[&r.id]);
            }
        }
        self.rebuild_ledger();
    }

    fn rebuild_ledger(&mut self) {
        let mut out = Vec::new();
        for r in &self.sessions {
            out.push(Item::Row { row: r.clone(), branch: Branch::Root });
            let Some(kids) = self.children.get(&r.id) else { continue };
            let shown = if kids.len() > MAX_KIDS && !self.expanded.contains(&r.id) { MAX_KIDS } else { kids.len() };
            for (i, k) in kids.iter().take(shown).enumerate() {
                let last = i + 1 == kids.len();
                out.push(Item::Row { row: k.clone(), branch: if last { Branch::Last } else { Branch::Mid } });
            }
            if shown < kids.len() {
                out.push(Item::More { parent: r.id.clone(), hidden: kids.len() - shown });
            }
        }
        self.ledger = out;
        self.selected = self.selected.min(self.ledger.len().saturating_sub(1));
    }

    pub fn selected_session(&self) -> Option<&SessionRow> {
        match self.ledger.get(self.selected)? {
            Item::Row { row, .. } => Some(row),
            Item::More { .. } => None,
        }
    }

    /// Enter on `+N more` expands it; on anything else it does nothing here.
    fn expand_selected_more(&mut self) -> bool {
        matches!(self.ledger.get(self.selected), Some(Item::More { .. })) && self.expand_selected()
    }

    /// `→` on `+N more` or on a root with hidden children: show every child of that parent.
    fn expand_selected(&mut self) -> bool {
        let parent = match self.ledger.get(self.selected) {
            Some(Item::More { parent, .. }) => parent.clone(),
            Some(Item::Row { row, branch: Branch::Root }) if self.children.get(&row.id).is_some_and(|k| k.len() > MAX_KIDS) => row.id.clone(),
            _ => return false,
        };
        if !self.expanded.insert(parent.clone()) {
            return false;
        }
        // Keep the cursor on the row it was on (a root) or on the first revealed child.
        self.rebuild_ledger();
        true
    }

    /// (active, failed, total) over everything loaded, children included.
    fn counts(&self) -> (usize, usize, usize) {
        let all = || self.sessions.iter().chain(self.children.values().flatten());
        (all().filter(|s| s.status == Status::Active).count(), all().filter(|s| s.failed).count(), all().count())
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
        let Some(row) = self.selected_session().cloned() else { return };
        self.trail.clear();
        self.open_session(row);
    }

    fn open_session(&mut self, row: SessionRow) {
        self.parent_name = match (&self.index, &row.parent_id) {
            (Some(i), Some(pid)) => i.session(pid).ok().flatten().map(|p| clean_name(&p)),
            _ => None,
        };
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

    /// Esc in a trajectory: back up the dispatch trail, else to the ledger.
    fn leave_trajectory(&mut self) {
        if let Some((parent, selected)) = self.trail.pop() {
            self.open_session(parent);
            self.event_selected = selected.min(self.events.len().saturating_sub(1));
        } else {
            self.mode = Mode::List;
            self.session = None;
            self.events.clear();
            self.folded.clear();
        }
    }

    fn load_events(&mut self) {
        let listed = {
            let (Some(index), Some(session)) = (&self.index, &self.session) else { return };
            let filter = EventFilter {
                text: (!self.event_query.is_empty()).then(|| self.event_query.clone()),
                errors_only: self.event_errors_only,
                tool: self.tool_filter.and_then(|i| self.tools.get(i).cloned()),
                include_noise: self.include_noise,
                limit: EVENT_PAGE,
                offset: 0,
            };
            index.list_events(&session.id, &filter)
        let (Some(index), Some(session)) = (&self.index, &self.session) else { return };
        let filter = EventFilter {
            text: (!self.event_query.is_empty()).then(|| self.event_query.clone()),
            errors_only: self.event_errors_only,
            tool: self.tool_filter.and_then(|i| self.tools.get(i).cloned()),
            limit: EVENT_PAGE,
            offset: 0,
        };
        match listed {
            Ok(rows) => self.set_events(rows),
            Err(e) => self.status = Some(format!("query failed: {e}")),
        }
        let fresh = match (&self.index, &self.session) {
            (Some(i), Some(s)) => i.session(&s.id).ok().flatten(),
            _ => None,
        };
        if fresh.is_some() {
            self.session = fresh;
        }
        self.session_children = match (&self.index, &self.session) {
            (Some(i), Some(s)) if s.children > 0 => i.children(&s.id).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.session_children.sort_by_key(|c| c.started_at);
        self.event_selected = self.event_selected.min(self.events.len().saturating_sub(1));
    }

    /// Folds each tool result into the call row right before it (unless noise is shown).
    fn set_events(&mut self, rows: Vec<EventRow>) {
        let mut events: Vec<EventRow> = Vec::with_capacity(rows.len());
        let mut folded: Vec<Option<Fold>> = Vec::with_capacity(rows.len());
        for e in rows {
            if !self.include_noise
                && is_result(&e)
                && events.last().is_some_and(is_call)
                && folded.last().is_some_and(Option::is_none)
            {
                *folded.last_mut().unwrap() = Some(Fold { error: e.is_error, detail: e.detail.clone() });
                continue;
            }
            events.push(e);
            folded.push(None);
        }
        (self.events, self.folded) = (events, folded);
    }

    /// The child session a dispatch row (Agent/Task tool call) started, if one matches.
    fn child_of(&self, i: usize) -> Option<&SessionRow> {
        let e = self.events.get(i)?;
        if !is_dispatch(e) || self.session_children.is_empty() {
            return None;
        }
        let want = norm(e.detail.as_deref().unwrap_or(&e.preview));
        if !want.is_empty()
            && let Some(c) = self.session_children.iter().find(|c| {
                let l = norm(c.label.as_deref().unwrap_or_default());
                !l.is_empty() && (l == want || l.starts_with(&want) || want.starts_with(&l))
            })
        {
            return Some(c);
        }
        // No label match: pair the nth dispatch row with the nth child when the counts agree.
        if !self.event_query.is_empty() || self.event_errors_only {
            return None;
        }
        let dispatches: Vec<usize> = (0..self.events.len()).filter(|&j| is_dispatch(&self.events[j])).collect();
        if dispatches.len() != self.session_children.len() {
            return None;
        }
        self.session_children.get(dispatches.iter().position(|&j| j == i)?)
    }

    fn enter_child(&mut self, child: SessionRow) {
        if let Some(open) = self.session.clone() {
            self.trail.push((open, self.event_selected));
        }
        self.open_session(child);
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

fn is_result(e: &EventRow) -> bool {
    e.role.as_deref() == Some("tool_result")
}

fn is_call(e: &EventRow) -> bool {
    e.kind == "tool" && e.role.as_deref() == Some("tool")
}

/// An Agent/Task tool call: a subagent dispatch.
fn is_dispatch(e: &EventRow) -> bool {
    is_call(e) && matches!(e.title.to_ascii_lowercase().as_str(), "agent" | "task")
}

fn norm(s: &str) -> String {
    one_line(s).to_lowercase()
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
            let len = state.ledger.len();
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
                    state.show_all = !state.show_all;
                    state.selected = 0;
                    state.refresh();
                }
                KeyCode::Char('A') => {
                    state.active_only = !state.active_only;
                    state.refresh();
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if !state.expand_selected() {
                        return InputOutcome::Unchanged;
                    }
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
                KeyCode::Char('r') => return state.resume(state.selected_session()),
                KeyCode::Enter => {
                    if !state.expand_selected_more() {
                        state.open_selected();
                    }
                }
                _ => return InputOutcome::Unchanged,
            }
        }
        Mode::Trajectory => {
            let len = state.events.len();
            match key.code {
                KeyCode::Esc => state.leave_trajectory(),
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
                KeyCode::Char('n') => {
                    state.include_noise = !state.include_noise;
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
                KeyCode::Enter => match state.child_of(state.event_selected).cloned() {
                    Some(child) => state.enter_child(child),
                    None => state.open_inspector(),
                },
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
            plain("a all"),
            plain("A active"),
            plain("e errors"),
            plain("s sort"),
            plain("Enter open"),
            plain("→ expand"),
            plain("r resume"),
            plain("R rescan"),
            plain("Esc close"),
        ],
        Mode::Trajectory => vec![
            plain("j/k move"),
            plain("Enter inspect/open agent"),
            plain("n noise"),
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

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// "now", "42s", "3m", "2h", "5d".
pub fn fmt_age(ago_ms: i64) -> String {
    let s = ago_ms.max(0) / 1000;
    match s {
        0..10 => "now".into(),
        10..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

fn age_of(s: &SessionRow, now: i64) -> String {
    s.last_event_at.or(s.updated_at).map_or("-".into(), |t| fmt_age(now - t))
}

/// Dispatch description (else title) without leading markup or a bare file name.
pub fn clean_name(s: &SessionRow) -> String {
    let raw = s.label.as_deref().filter(|l| !l.trim().is_empty()).unwrap_or(&s.title);
    let mut t = one_line(raw);
    loop {
        let before = t.len();
        t = t.trim_start_matches(|c: char| matches!(c, '#' | '*' | '>' | '"' | '\'' | '`') || c.is_whitespace()).to_string();
        // A wrapper tag such as `<command-message>`: drop it and its closing twin.
        if t.starts_with('<')
            && let Some(end) = t.find('>')
        {
            let tag = t[1..end].to_string();
            if !tag.is_empty() && !tag.starts_with('/') && tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
                t = t[end + 1..].replacen(&format!("</{tag}>"), " ", 1);
            }
        }
        if t.len() == before {
            break;
        }
    }
    t = t.trim().to_string();
    if t.is_empty() {
        t = one_line(&s.title);
    }
    if !t.contains(' ') && (t.ends_with(".jsonl") || t.ends_with(".json")) {
        let dir = s.cwd.as_deref().and_then(|c| c.trim_end_matches('/').rsplit('/').next()).filter(|d| !d.is_empty());
        t = format!("{} session", dir.unwrap_or(source_label(&s.source)));
    }
    t
}

/// ● running, ✗ failed, ✓ done, ○ idle.
fn status_glyph(s: &SessionRow, theme: &Theme) -> (&'static str, Color) {
    match s.status {
        Status::Active => ("●", theme.accent_running),
        _ if s.failed => ("✗", theme.accent_error),
        Status::Completed => ("✓", theme.gray),
        _ => ("○", theme.gray_dim),
    }
}

struct Cols {
    name: usize,
    now: usize,
    counts: bool,
    cwd: bool,
    tokens: bool,
}

/// Fixed columns by priority; NAME and NOW share what is left.
fn ledger_cols(width: usize) -> Cols {
    let (counts, cwd, tokens) = (width >= 100, width >= 130, width >= 160);
    // glyph, source, age, tools (+1 separator each), plus the NAME and NOW separators.
    let mut fixed = (1 + 1) + (7 + 1) + (4 + 1) + (5 + 1) + 2;
    if counts {
        fixed += (5 + 1) + (4 + 1) + (6 + 1);
    }
    if cwd {
        fixed += 22 + 1;
    }
    if tokens {
        fixed += 12 + 1;
    }
    let flex = width.saturating_sub(fixed);
    let name = (flex * 2 / 5).max(12).min(flex);
    Cols { name, now: flex - name, counts, cwd, tokens }
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
        flags.push("active only".into());
    }
    if state.show_all {
        flags.push("all".into());
    }
    flags.push(format!("sort:{:?}", state.sort).to_lowercase());
    let (active, failed, total) = state.counts();
    let mut counts = Vec::new();
    if active > 0 {
        counts.push(format!("{active} active"));
    }
    if failed > 0 {
        counts.push(format!("{failed} failed"));
    }
    counts.push(format!("{total} sessions"));
    flags.push(counts.join(" · "));
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
    let c = ledger_cols(area.width as usize);
    let mut head: Vec<(String, usize, Style)> = vec![
        (" ".into(), 1, dim),
        ("SOURCE".into(), 7, dim),
        ("NAME".into(), c.name, dim),
        ("NOW".into(), c.now, dim),
        ("AGE".into(), 4, dim),
        ("TOOLS".into(), 5, dim),
    ];
    if c.counts {
        head.extend([("  EVT".into(), 5, dim), (" ERR".into(), 4, dim), ("   DUR".into(), 6, dim)]);
    }
    if c.cwd {
        head.push(("CWD".into(), 22, dim));
    }
    if c.tokens {
        head.push(("   IN/OUT".into(), 12, dim));
    }
    put_row(buf, area, area.y + 2, &head);
    if state.ledger.is_empty() {
        let msg = if state.scanning { "Indexing local sessions…" } else { "No sessions match. (a shows every session)" };
        buf.set_span(list.x, list.y, &Span::styled(msg, dim), list.width);
    }
    keep_visible(state.selected, &mut state.scroll, list.height as usize);
    let now = now_ms();
    for (i, item) in state.ledger.iter().enumerate().skip(state.scroll).take(list.height as usize) {
        let y = list.y + (i - state.scroll) as u16;
        let selected = i == state.selected;
        let base = if selected { Style::default().bg(theme.bg_highlight).fg(theme.text_primary) } else { Style::default().fg(theme.text_primary) };
        if selected {
            fill(buf, list, y, base);
        }
        match item {
            Item::More { hidden, .. } => put_row(
                buf,
                list,
                y,
                &[(" ".into(), 1, base), ("".into(), 7, base), (format!("└─ +{hidden} more (→)"), c.name, base.fg(theme.gray))],
            ),
            Item::Row { row: s, branch } => {
                let (glyph, glyph_color) = status_glyph(s, theme);
                let prefix = match branch {
                    Branch::Root => "",
                    Branch::Mid => "├─ ",
                    Branch::Last => "└─ ",
                };
                let running = s.status == Status::Active;
                let now_text = one_line(s.now.as_deref().unwrap_or_default());
                let mut v = vec![
                    (glyph.to_string(), 1, base.fg(glyph_color)),
                    (source_label(&s.source).to_string(), 7, base.fg(source_color(&s.source, theme))),
                    (format!("{prefix}{}", clean_name(s)), c.name, base),
                    (now_text, c.now, base.fg(if running { theme.text_secondary } else { theme.gray })),
                    (format!("{:>4}", age_of(s, now)), 4, base.fg(theme.text_secondary)),
                    (format!("{:>5}", s.tool_count), 5, base.fg(theme.accent_tool)),
                ];
                if c.counts {
                    v.push((format!("{:>5}", s.event_count), 5, base.fg(theme.text_secondary)));
                    v.push((if s.error_count > 0 { format!("{:>4}", s.error_count) } else { String::new() }, 4, base.fg(theme.accent_error)));
                    v.push((format!("{:>6}", fmt_duration(s.duration_ms)), 6, base.fg(theme.text_secondary)));
                }
                if c.cwd {
                    v.push((s.cwd.clone().or_else(|| s.model.clone()).unwrap_or_default(), 22, base.fg(theme.path)));
                }
                if c.tokens {
                    v.push((format!("{:>5}/{:<6}", fmt_tokens(s.input_tokens), fmt_tokens(s.output_tokens)), 12, base.fg(theme.gray_bright)));
                }
                put_row(buf, list, y, &v);
            }
        }
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

fn chip_color(e: &EventRow, theme: &Theme) -> Color {
    if e.is_error {
        theme.accent_error
    } else if e.kind == "tool" {
        theme.accent_tool
    } else if e.role.as_deref() == Some("user") || e.kind == "system" || e.kind == "reasoning" {
        theme.accent_user
    } else {
        theme.accent_assistant
    }
}

const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];

/// Bucket of event `i` out of `n`: by time when the span has one, else by position.
fn bucket_of(events: &[EventRow], i: usize, n: usize) -> usize {
    let stamps = events.iter().filter_map(|e| e.timestamp);
    let frac = match (events[i].timestamp, stamps.clone().min(), stamps.max()) {
        (Some(t), Some(a), Some(b)) if b > a => (t - a) as f64 / (b - a) as f64,
        _ => i as f64 / (events.len().max(2) - 1) as f64,
    };
    ((frac * (n - 1) as f64).round() as usize).min(n - 1)
}

/// Per-bucket (event count, has error, turn starts here) over `n` buckets.
fn bucket_events(events: &[EventRow], n: usize) -> Vec<(u32, bool, bool)> {
    let mut out = vec![(0u32, false, false); n];
    if n == 0 {
        return out;
    }
    let mut prev_turn = None;
    for (i, e) in events.iter().enumerate() {
        let b = &mut out[bucket_of(events, i, n)];
        b.0 += 1;
        b.1 |= e.is_error;
        if e.turn.is_some() && e.turn != prev_turn {
            b.2 = true;
        }
        prev_turn = e.turn.or(prev_turn);
    }
    out
}

/// Two lines: shaded activity per bucket (red where something failed), then turn starts `│` and the cursor `▲`.
fn render_strip(buf: &mut Buffer, area: Rect, events: &[EventRow], selected: usize, theme: &Theme) {
    let gutter = 9u16;
    let n = area.width.saturating_sub(gutter) as usize;
    buf.set_span(area.x, area.y, &Span::styled("ACTIVITY", Style::default().fg(theme.gray)), gutter);
    if n == 0 || events.is_empty() {
        return;
    }
    let buckets = bucket_events(events, n);
    let max = buckets.iter().map(|b| b.0).max().unwrap_or(0).max(1) as usize;
    for (i, (count, err, turn)) in buckets.iter().enumerate() {
        let x = area.x + gutter + i as u16;
        if *count > 0 {
            let level = (*count as usize * 4).div_ceil(max).clamp(1, 4);
            let fg = if *err { theme.accent_error } else { theme.accent_assistant };
            buf.set_span(x, area.y, &Span::styled(SHADES[level].to_string().as_str(), Style::default().fg(fg)), 1);
        }
        if *turn {
            buf.set_span(x, area.y + 1, &Span::styled("│", Style::default().fg(theme.gray_dim)), 1);
        }
    }
    if selected < events.len() {
        let at = area.x + gutter + bucket_of(events, selected, n) as u16;
        buf.set_span(at, area.y + 1, &Span::styled("▲", Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), 1);
    }
}

fn render_trajectory(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let Some(session) = state.session.clone() else { return };
    let dim = Style::default().fg(theme.gray);
    let chip = format!(" {} ", source_label(&session.source));
    let cw = chip.width() as u16;
    buf.set_span(area.x, area.y, &Span::styled(chip.as_str(), Style::default().fg(theme.bg_base).bg(source_color(&session.source, theme)).add_modifier(Modifier::BOLD)), cw);
    let title_w = area.width.saturating_sub(cw + 1) as usize;
    buf.set_span(area.x + cw + 1, area.y, &Span::styled(truncate_str(&clean_name(&session), title_w).as_str(), Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), title_w as u16);

    // Meta line: status, agent type / model, age, counts, parent.
    let (glyph, glyph_color) = status_glyph(&session, theme);
    let mut segs: Vec<(String, Style)> = vec![(format!("{glyph} {}", if session.failed && session.status != Status::Active { "failed" } else { session.status.as_str() }), Style::default().fg(glyph_color))];
    for t in [session.agent_type.clone(), session.model.clone()].into_iter().flatten() {
        segs.push((t, dim));
    }
    let age = age_of(&session, now_ms());
    segs.push((if age == "now" || age == "-" { age } else { format!("{age} ago") }, dim));
    segs.push((format!("{} evt · {} tools · {}", session.event_count, session.tool_count, fmt_duration(session.duration_ms)), dim));
    if let Some(p) = &state.parent_name {
        segs.push((format!("↑ {p}"), Style::default().fg(theme.accent_user)));
    }
    let mut x = area.x;
    let right = area.x + area.width;
    for (i, (text, style)) in segs.iter().enumerate() {
        if i > 0 {
            if x + 3 >= right {
                break;
            }
            buf.set_span(x, area.y + 1, &Span::styled(" · ", dim), 3);
            x += 3;
        }
        let w = (text.width() as u16).min(right - x);
        buf.set_span(x, area.y + 1, &Span::styled(truncate_str(text, w as usize).as_str(), *style), w);
        x += w;
    }
    render_strip(buf, Rect { y: area.y + 2, height: 2, ..area }, &state.events, state.event_selected, theme);

    let mut filters = Vec::new();
    if state.event_errors_only {
        filters.push("errors only".to_string());
    }
    if state.include_noise {
        filters.push("noise shown".to_string());
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
        // Noise (only present when `n` is on) is dimmed.
        let (text_style, chip_fg) = if e.noise { (base.fg(theme.gray_dim), theme.gray_dim) } else { (base, chip_color(e, theme)) };
        let indent = "  ".repeat(e.depth.clamp(0, 2) as usize);
        let one = one_line(e.detail.as_deref().filter(|d| !d.is_empty()).unwrap_or(&e.preview));
        let body = if is_call(e) { format!("{indent}{}  {}", e.title, one) } else { format!("{indent}{one}") };
        // Suffix: where a dispatch went, and how the call ended.
        let mut suffix = String::new();
        if let Some(c) = state.child_of(i) {
            suffix.push_str(&format!("→ {} {}", truncate_str(&clean_name(c), 28), status_glyph(c, theme).0));
        }
        let mut failed = e.is_error;
        if let Some(Some(f)) = state.folded.get(i) {
            failed |= f.error;
            if !suffix.is_empty() {
                suffix.push(' ');
            }
            suffix.push_str(if f.error { "✗" } else { "✓" });
            if f.error && let Some(d) = f.detail.as_deref().filter(|d| !d.is_empty()) {
                suffix.push_str(&format!(" {}", truncate_str(&one_line(d), 24)));
            }
        }
        let sw = suffix.width().min(summary_w / 2);
        let body_w = summary_w.saturating_sub(if sw > 0 { sw + 1 } else { 0 });
        let turn_mark = if e.turn.is_some() && e.turn != prev_turn { "┊" } else { " " };
        prev_turn = e.turn.or(prev_turn);
        let body_style = if is_result(e) && e.is_error { text_style.fg(theme.accent_error) } else { text_style };
        let mut cells = vec![
            (format!("{:>6}", format!("#{}", e.line)), 6, base.fg(theme.gray)),
            (role_chip(e).to_string(), 6, base.fg(chip_fg).add_modifier(Modifier::BOLD)),
            (format!("{turn_mark}{body}"), body_w, body_style),
        ];
        if sw > 0 {
            cells.push((suffix, sw, base.fg(if failed { theme.accent_error } else { theme.accent_success })));
        }
        cells.push((fmt_time(e.timestamp, false), 8, base.fg(theme.text_secondary)));
        put_row(buf, list, y, &cells);
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
        let mut state = TracesModalState::open(Some(TraceIndex::open(&db).unwrap()));
        // One-line fixtures count as trivial; this fixture is about ordering and live updates.
        state.show_all = true;
        state.refresh();
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
        assert_eq!((state.selected, state.selected_session().unwrap().title.as_str()), (2, "task b"));
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
        assert_eq!(
            [3_000, 42_000, 180_000, 7_200_000, 5 * 86_400_000].map(fmt_age),
            ["now", "42s", "3m", "2h", "5d"]
        );
    }

    #[test]
    fn names_lose_wrapper_noise() {
        let mut r = row("claude", "/p/a.jsonl");
        r.title = "<command-message>## Fix the index</command-message>".into();
        assert_eq!(clean_name(&r), "Fix the index");
        r.label = Some("  Explore index code ".into());
        assert_eq!(clean_name(&r), "Explore index code", "dispatch label beats title");
        r.label = None;
        r.title = "chat_history.jsonl".into();
        r.cwd = Some("/w/proj/".into());
        assert_eq!(clean_name(&r), "proj session");
    }

    const PID: &str = "0b7e1c2a-0000-4000-8000-0000000000aa";

    fn jl(lines: &[&str]) -> String {
        lines.join("\n") + "\n"
    }

    /// A Claude parent that dispatched one Agent, its subagent transcript and meta, indexed; a modal on the same db.
    fn claude_fixture() -> (tempfile::TempDir, TracesModalState) {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("claude/-w");
        let subs = proj.join(PID).join("subagents");
        std::fs::create_dir_all(&subs).unwrap();
        std::fs::write(
            proj.join(format!("{PID}.jsonl")),
            jl(&[
                r#"{"type":"file-history-snapshot","timestamp":"2026-10-01T00:00:00Z","snapshot":{}}"#,
                r#"{"type":"user","timestamp":"2026-10-01T00:00:01Z","cwd":"/w","message":{"role":"user","content":"index everything please"}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:02Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Agent","input":{"description":"Explore index code","subagent_type":"Explore","prompt":"look at the index"}}]}}"#,
                r#"{"type":"user","timestamp":"2026-10-01T00:00:09Z","message":{"role":"user","content":[{"type":"tool_result","content":"found it"}]}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:10Z","message":{"role":"assistant","content":"all done here"}}"#,
            ]),
        )
        .unwrap();
        std::fs::write(
            subs.join("agent-x.jsonl"),
            jl(&[
                r#"{"type":"user","timestamp":"2026-10-01T00:00:03Z","message":{"role":"user","content":"look at the index and report"}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:04Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"rg index"}}]}}"#,
                r#"{"type":"user","timestamp":"2026-10-01T00:00:05Z","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:08Z","message":{"role":"assistant","content":"the index lives in index.rs"}}"#,
            ]),
        )
        .unwrap();
        std::fs::write(subs.join("agent-x.meta.json"), r#"{"agentType":"Explore","description":"Explore index code"}"#).unwrap();
        let roots = [xai_grok_traces::Root {
            id: "claude-projects".into(),
            source: "claude".into(),
            path: tmp.path().join("claude"),
            format: xai_grok_traces::Format::Jsonl,
            file_name: None,
        }];
        let db = tmp.path().join("idx/t.sqlite");
        TraceIndex::open(&db).unwrap().scan(&roots).unwrap();
        let state = TracesModalState::open(Some(TraceIndex::open(&db).unwrap()));
        (tmp, state)
    }

    fn press(state: &mut TracesModalState, code: KeyCode) {
        handle_traces_key(state, &KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn draw(state: &mut TracesModalState, w: u16, h: u16) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = Theme::current();
        match state.mode {
            Mode::Trajectory => render_trajectory(&mut buf, area, state, &theme),
            _ => render_ledger(&mut buf, area, state, &theme),
        }
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn ledger_nests_subagents_under_their_parent() {
        let (_tmp, mut state) = claude_fixture();
        assert_eq!(state.sessions.len(), 1, "roots only by default");
        assert_eq!(state.ledger.len(), 2, "parent then its child");
        assert!(matches!(&state.ledger[1], Item::Row { row, branch: Branch::Last } if row.parent_id.is_some()));
        let screen = draw(&mut state, 140, 20);
        assert!(screen.contains("└─ Explore index code"), "{screen}");
        assert!(screen.contains("index everything please"), "{screen}");
        assert!(screen.contains("2 sessions"), "{screen}");

        // `a` flattens: no tree prefixes, both sessions are plain rows.
        press(&mut state, KeyCode::Char('a'));
        assert!(state.show_all);
        assert_eq!(state.sessions.len(), 2);
        assert!(!draw(&mut state, 140, 20).contains("└─"));
    }

    #[test]
    fn many_children_collapse_until_expanded() {
        let (_tmp, mut state) = claude_fixture();
        let kid = match &state.ledger[1] {
            Item::Row { row, .. } => row.clone(),
            _ => unreachable!(),
        };
        let parent = state.sessions[0].id.clone();
        state.children.insert(parent.clone(), (0..9).map(|i| SessionRow { id: format!("k{i}"), ..kid.clone() }).collect());
        state.rebuild_ledger();
        assert_eq!(state.ledger.len(), 1 + MAX_KIDS + 1);
        assert!(matches!(state.ledger.last(), Some(Item::More { hidden: 3, .. })));
        assert!(draw(&mut state, 140, 20).contains("+3 more"));
        state.selected = state.ledger.len() - 1;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::List, "Enter on +N more expands instead of opening");
        assert_eq!(state.ledger.len(), 1 + 9);
    }

    #[test]
    fn noise_is_hidden_until_n() {
        let (_tmp, mut state) = claude_fixture();
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::Trajectory);
        assert!(state.events.iter().all(|e| !e.noise), "no noise rows by default");
        let quiet = state.events.len();
        press(&mut state, KeyCode::Char('n'));
        assert!(state.include_noise);
        assert!(state.events.len() > quiet && state.events.iter().any(|e| e.noise), "n shows the snapshot row");
        assert!(draw(&mut state, 140, 24).contains("noise shown"));
        press(&mut state, KeyCode::Char('n'));
        assert_eq!(state.events.len(), quiet);
    }

    #[test]
    fn results_fold_into_their_call_and_agent_rows_open_the_child() {
        let (_tmp, mut state) = claude_fixture();
        let parent = state.selected_session().unwrap().id.clone();
        press(&mut state, KeyCode::Enter);
        assert!(state.events.iter().all(|e| !is_result(e)), "adjacent results are folded away");
        let at = state.events.iter().position(is_dispatch).expect("an Agent row");
        assert!(matches!(&state.folded[at], Some(Fold { error: false, .. })));
        state.event_selected = at;
        let screen = draw(&mut state, 140, 24);
        assert!(screen.contains("Agent  Explore index code"), "{screen}");
        assert!(screen.contains("→ Explore index code") && screen.contains("✓"), "{screen}");

        press(&mut state, KeyCode::Enter);
        let child = state.session.clone().unwrap();
        assert_eq!((state.mode, child.parent_id.as_deref()), (Mode::Trajectory, Some(parent.as_str())));
        let screen = draw(&mut state, 140, 24);
        assert!(screen.contains("rg index") && screen.contains("↑ "), "child shows its tool and its parent\n{screen}");

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.session.as_ref().unwrap().id, parent, "Esc returns to the parent");
        assert_eq!(state.event_selected, at, "at the row we left");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::List);
    }

    #[test]
    fn activity_strip_buckets_events() {
        let mk = |t: i64, err: bool, turn: i64| EventRow {
            line: 0,
            event_type: "x".into(),
            kind: "tool".into(),
            role: Some("tool".into()),
            title: "Bash".into(),
            preview: String::new(),
            timestamp: Some(t),
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: Some(turn),
            step: None,
            depth: 0,
            model: None,
            is_error: err,
            noise: false,
            detail: None,
        };
        let events = [mk(0, false, 1), mk(10, false, 1), mk(11, false, 1), mk(100, true, 2)];
        let b = bucket_events(&events, 11);
        assert_eq!((b[0], b[1], b[10]), ((1, false, true), (2, false, false), (1, true, true)));
    }
}
