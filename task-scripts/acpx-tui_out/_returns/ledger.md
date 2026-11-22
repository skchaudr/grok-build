```rust
use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{NodeState, Outcome, RunStatus, RunSummary, fmt_duration};
use crate::acpx::sessions::SessionSummary;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerTab {
    Flows,
    Sessions,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LedgerOutcome {
    Unchanged,
    Changed,
    OpenRun(PathBuf),
    OpenSession(SessionSummary),
    Copy(String),
    Rescan,
    Close,
}

/// Per-run facts the parent loads (cached) so the ledger does no I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowOrder {
    Grouped,
    Recent,
    Name,
}

impl FlowOrder {
    fn label(self) -> &'static str {
        match self {
            Self::Grouped => "grouped",
            Self::Recent => "recent",
            Self::Name => "name",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Grouped => Self::Recent,
            Self::Recent => Self::Name,
            Self::Name => Self::Grouped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionOrder {
    Recent,
    Name,
}

impl SessionOrder {
    fn label(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Name => "name",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Recent => Self::Name,
            Self::Name => Self::Recent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    NeedsYou,
    Running,
    Finished,
}

impl SectionKind {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "NEEDS YOU",
            Self::Running => "RUNNING",
            Self::Finished => "FINISHED",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::NeedsYou => theme.warning,
            Self::Running => theme.accent_running,
            Self::Finished => theme.gray_dim,
        }
    }
}

#[derive(Debug, Clone)]
enum FlowListRow {
    Header { kind: SectionKind, count: usize },
    Blank,
    Run(usize), // index in `runs`
}

impl FlowListRow {
    fn is_selectable(&self) -> bool {
        matches!(self, Self::Run(_))
    }
}

pub struct LedgerState {
    tab: LedgerTab,
    runs: Vec<RunSummary>,
    extras: HashMap<String, RowExtra>,
    sessions: Vec<SessionSummary>,

    // Per-tab filter, order, active-only, selection & scroll
    flow_filter: String,
    flow_active_only: bool,
    flow_order: FlowOrder,
    flow_selected_run_id: Option<String>,
    flow_scroll: usize,

    session_filter: String,
    session_active_only: bool,
    session_order: SessionOrder,
    session_selected_id: Option<String>,
    session_scroll: usize,

    filter_open: bool,
    last_render_height: usize,
    last_render_width: usize,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerState {
    pub fn new() -> Self {
        Self {
            tab: LedgerTab::Flows,
            runs: Vec::new(),
            extras: HashMap::new(),
            sessions: Vec::new(),

            flow_filter: String::new(),
            flow_active_only: false,
            flow_order: FlowOrder::Grouped,
            flow_selected_run_id: None,
            flow_scroll: 0,

            session_filter: String::new(),
            session_active_only: false,
            session_order: SessionOrder::Recent,
            session_selected_id: None,
            session_scroll: 0,

            filter_open: false,
            last_render_height: 14,
            last_render_width: 100,
        }
    }

    pub fn set_runs(&mut self, runs: Vec<RunSummary>) {
        self.runs = runs;
        self.reconcile_flow_selection();
    }

    pub fn set_extras(&mut self, extras: HashMap<String, RowExtra>) {
        self.extras = extras;
        self.reconcile_flow_selection();
    }

    pub fn selected_run(&self) -> Option<&RunSummary> {
        if self.tab != LedgerTab::Flows {
            return None;
        }
        let id = self.flow_selected_run_id.as_deref()?;
        let indices = self.filtered_flow_indices();
        indices
            .into_iter()
            .find_map(|idx| self.runs.get(idx).filter(|r| r.run_id == id))
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.sessions = sessions;
        self.reconcile_session_selection();
    }

    pub fn tab(&self) -> LedgerTab {
        self.tab
    }

    pub fn any_live(&self) -> bool {
        self.runs
            .iter()
            .any(|r| r.status.is_live() && !self.is_stalled(r))
    }

    pub fn captures_text(&self) -> bool {
        self.filter_open
    }

    pub fn esc_closes(&self) -> bool {
        !self.filter_open && self.active_filter().is_empty()
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        if self.filter_open {
            vec!["Enter apply", "Esc clear"]
        } else {
            vec![
                "Enter open",
                "Tab switch",
                "/ filter",
                "a active",
                "s sort",
                "y copy",
                "R rescan",
                "Esc close",
            ]
        }
    }

    fn is_stalled(&self, run: &RunSummary) -> bool {
        self.extras
            .get(&run.run_id)
            .map(|e| e.stalled)
            .unwrap_or(false)
    }

    fn active_filter(&self) -> &str {
        match self.tab {
            LedgerTab::Flows => &self.flow_filter,
            LedgerTab::Sessions => &self.session_filter,
        }
    }

    fn active_filter_mut(&mut self) -> &mut String {
        match self.tab {
            LedgerTab::Flows => &mut self.flow_filter,
            LedgerTab::Sessions => &mut self.session_filter,
        }
    }

    fn is_run_active(&self, run: &RunSummary) -> bool {
        matches!(run.status, RunStatus::Running | RunStatus::Waiting) && !self.is_stalled(run)
    }

    fn matches_flow_filter(&self, run: &RunSummary) -> bool {
        if self.flow_active_only && !self.is_run_active(run) {
            return false;
        }
        let q = self.flow_filter.trim();
        if q.is_empty() {
            return true;
        }
        let q_lower = q.to_lowercase();
        let agents = self
            .extras
            .get(&run.run_id)
            .map(|e| e.agents.as_str())
            .unwrap_or("");
        run.title.to_lowercase().contains(&q_lower)
            || run.flow_name.to_lowercase().contains(&q_lower)
            || run.status.label().to_lowercase().contains(&q_lower)
            || run.run_id.to_lowercase().contains(&q_lower)
            || agents.to_lowercase().contains(&q_lower)
    }

    fn matches_session_filter(&self, s: &SessionSummary) -> bool {
        if self.session_active_only && s.closed {
            return false;
        }
        let q = self.session_filter.trim();
        if q.is_empty() {
            return true;
        }
        let q_lower = q.to_lowercase();
        s.title.to_lowercase().contains(&q_lower)
            || s.agent.to_lowercase().contains(&q_lower)
            || s.cwd.to_lowercase().contains(&q_lower)
            || s.name.to_lowercase().contains(&q_lower)
    }

    fn run_section(&self, run: &RunSummary) -> SectionKind {
        let stalled = self.is_stalled(run);
        if run.status == RunStatus::Waiting {
            SectionKind::NeedsYou
        } else if run.status == RunStatus::Running && !stalled {
            SectionKind::Running
        } else {
            SectionKind::Finished
        }
    }

    /// Returns indices into `self.runs` in display order.
    fn filtered_flow_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .runs
            .iter()
            .enumerate()
            .filter_map(|(i, r)| self.matches_flow_filter(r).then_some(i))
            .collect();

        match self.flow_order {
            FlowOrder::Grouped => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    let sa = match self.run_section(ra) {
                        SectionKind::NeedsYou => 0,
                        SectionKind::Running => 1,
                        SectionKind::Finished => 2,
                    };
                    let sb = match self.run_section(rb) {
                        SectionKind::NeedsYou => 0,
                        SectionKind::Running => 1,
                        SectionKind::Finished => 2,
                    };
                    sa.cmp(&sb)
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Recent => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    rb.updated_ms
                        .cmp(&ra.updated_ms)
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Name => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    ra.title
                        .to_lowercase()
                        .cmp(&rb.title.to_lowercase())
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
        }
        indices
    }

    /// Build visual rows for the Flows tab. `list_height` is used to decide whether blank rows
    /// separate sections when the list is shorter than the area.
    fn build_flow_rows(&self, list_height: usize) -> Vec<FlowListRow> {
        let indices = self.filtered_flow_indices();
        if indices.is_empty() {
            return Vec::new();
        }
        if self.flow_order != FlowOrder::Grouped {
            return indices.into_iter().map(FlowListRow::Run).collect();
        }

        let mut needs_you = Vec::new();
        let mut running = Vec::new();
        let mut finished = Vec::new();
        for idx in indices {
            if let Some(r) = self.runs.get(idx) {
                match self.run_section(r) {
                    SectionKind::NeedsYou => needs_you.push(idx),
                    SectionKind::Running => running.push(idx),
                    SectionKind::Finished => finished.push(idx),
                }
            }
        }

        let sections: Vec<(SectionKind, Vec<usize>)> = [
            (SectionKind::NeedsYou, needs_you),
            (SectionKind::Running, running),
            (SectionKind::Finished, finished),
        ]
        .into_iter()
        .filter(|(_, items)| !items.is_empty())
        .collect();

        let non_blank_rows: usize = sections.iter().map(|(_, items)| 1 + items.len()).sum();
        let blank_count = sections.len().saturating_sub(1);
        let insert_blanks = non_blank_rows + blank_count <= list_height;

        let mut out = Vec::with_capacity(non_blank_rows + blank_count);
        for (i, (kind, items)) in sections.into_iter().enumerate() {
            if i > 0 && insert_blanks {
                out.push(FlowListRow::Blank);
            }
            out.push(FlowListRow::Header {
                kind,
                count: items.len(),
            });
            for idx in items {
                out.push(FlowListRow::Run(idx));
            }
        }
        out
    }

    fn filtered_session_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| self.matches_session_filter(s).then_some(i))
            .collect();

        match self.session_order {
            SessionOrder::Recent => {
                indices.sort_by(|&a, &b| {
                    let sa = &self.sessions[a];
                    let sb = &self.sessions[b];
                    sb.last_used_ms
                        .cmp(&sa.last_used_ms)
                        .then_with(|| sb.created_ms.cmp(&sa.created_ms))
                        .then_with(|| sa.record_id.cmp(&sb.record_id))
                });
            }
            SessionOrder::Name => {
                indices.sort_by(|&a, &b| {
                    let sa = &self.sessions[a];
                    let sb = &self.sessions[b];
                    sa.title
                        .to_lowercase()
                        .cmp(&sb.title.to_lowercase())
                        .then_with(|| sb.last_used_ms.cmp(&sa.last_used_ms))
                        .then_with(|| sa.record_id.cmp(&sb.record_id))
                });
            }
        }
        indices
    }

    fn reconcile_flow_selection(&mut self) {
        let indices = self.filtered_flow_indices();
        if indices.is_empty() {
            self.flow_selected_run_id = None;
            self.flow_scroll = 0;
            return;
        }
        if let Some(ref cur_id) = self.flow_selected_run_id
            && indices
                .iter()
                .any(|&i| self.runs.get(i).map(|r| &r.run_id) == Some(cur_id))
        {
            return;
        }
        if let Some(&first_idx) = indices.first()
            && let Some(r) = self.runs.get(first_idx)
        {
            self.flow_selected_run_id = Some(r.run_id.clone());
        }
    }

    fn reconcile_session_selection(&mut self) {
        let indices = self.filtered_session_indices();
        if indices.is_empty() {
            self.session_selected_id = None;
            self.session_scroll = 0;
            return;
        }
        if let Some(ref cur_id) = self.session_selected_id
            && indices
                .iter()
                .any(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(cur_id))
        {
            return;
        }
        if let Some(&first_idx) = indices.first()
            && let Some(s) = self.sessions.get(first_idx)
        {
            self.session_selected_id = Some(s.record_id.clone());
        }
    }

    fn move_selection(&mut self, delta: isize) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let indices = self.filtered_flow_indices();
                if indices.is_empty() {
                    return false;
                }
                let cur_pos = self
                    .flow_selected_run_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.runs.get(i).map(|r| &r.run_id) == Some(id))
                    })
                    .unwrap_or(0);
                let max_pos = indices.len().saturating_sub(1);
                let new_pos = (cur_pos as isize + delta).clamp(0, max_pos as isize) as usize;
                let new_id = self.runs.get(indices[new_pos]).map(|r| r.run_id.clone());
                if new_id != self.flow_selected_run_id {
                    self.flow_selected_run_id = new_id;
                    true
                } else {
                    false
                }
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                if indices.is_empty() {
                    return false;
                }
                let cur_pos = self
                    .session_selected_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(id))
                    })
                    .unwrap_or(0);
                let max_pos = indices.len().saturating_sub(1);
                let new_pos = (cur_pos as isize + delta).clamp(0, max_pos as isize) as usize;
                let new_id = self
                    .sessions
                    .get(indices[new_pos])
                    .map(|s| s.record_id.clone());
                if new_id != self.session_selected_id {
                    self.session_selected_id = new_id;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn jump_start(&mut self) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let indices = self.filtered_flow_indices();
                let first_id = indices
                    .first()
                    .and_then(|&i| self.runs.get(i))
                    .map(|r| r.run_id.clone());
                if first_id.is_some() && first_id != self.flow_selected_run_id {
                    self.flow_selected_run_id = first_id;
                    true
                } else {
                    false
                }
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                let first_id = indices
                    .first()
                    .and_then(|&i| self.sessions.get(i))
                    .map(|s| s.record_id.clone());
                if first_id.is_some() && first_id != self.session_selected_id {
                    self.session_selected_id = first_id;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn jump_end(&mut self) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let indices = self.filtered_flow_indices();
                let last_id = indices
                    .last()
                    .and_then(|&i| self.runs.get(i))
                    .map(|r| r.run_id.clone());
                if last_id.is_some() && last_id != self.flow_selected_run_id {
                    self.flow_selected_run_id = last_id;
                    true
                } else {
                    false
                }
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                let last_id = indices
                    .last()
                    .and_then(|&i| self.sessions.get(i))
                    .map(|s| s.record_id.clone());
                if last_id.is_some() && last_id != self.session_selected_id {
                    self.session_selected_id = last_id;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        self.move_selection(delta)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        if key.kind == KeyEventKind::Release {
            return LedgerOutcome::Unchanged;
        }

        if self.filter_open {
            return self.handle_filter_key(key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half_page = (self.last_render_height.saturating_sub(1) / 2).max(1) as isize;

        match key.code {
            KeyCode::Char('u') if ctrl => {
                if self.move_selection(-half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                if self.move_selection(half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.move_selection(1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.move_selection(-1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                if self.move_selection(half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                if self.move_selection(-half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.jump_start() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.jump_end() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.tab = match self.tab {
                    LedgerTab::Flows => LedgerTab::Sessions,
                    LedgerTab::Sessions => LedgerTab::Flows,
                };
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('/') => {
                self.filter_open = true;
                LedgerOutcome::Changed
            }
            KeyCode::Char('a') => {
                match self.tab {
                    LedgerTab::Flows => {
                        self.flow_active_only = !self.flow_active_only;
                        self.reconcile_flow_selection();
                    }
                    LedgerTab::Sessions => {
                        self.session_active_only = !self.session_active_only;
                        self.reconcile_session_selection();
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('s') => {
                match self.tab {
                    LedgerTab::Flows => {
                        self.flow_order = self.flow_order.next();
                        self.reconcile_flow_selection();
                    }
                    LedgerTab::Sessions => {
                        self.session_order = self.session_order.next();
                        self.reconcile_session_selection();
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Enter => match self.tab {
                LedgerTab::Flows => {
                    if let Some(run) = self.selected_run() {
                        LedgerOutcome::OpenRun(run.dir.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
                LedgerTab::Sessions => {
                    if let Some(sess) = self.selected_session() {
                        LedgerOutcome::OpenSession(sess.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('y') => match self.tab {
                LedgerTab::Flows => {
                    if let Some(run) = self.selected_run() {
                        LedgerOutcome::Copy(run.dir.to_string_lossy().into_owned())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
                LedgerTab::Sessions => {
                    if let Some(sess) = self.selected_session() {
                        LedgerOutcome::Copy(sess.record_id.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('R') => LedgerOutcome::Rescan,
            KeyCode::Esc => {
                if !self.active_filter().is_empty() {
                    self.active_filter_mut().clear();
                    match self.tab {
                        LedgerTab::Flows => self.reconcile_flow_selection(),
                        LedgerTab::Sessions => self.reconcile_session_selection(),
                    }
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Close
                }
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn handle_filter_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        match key.code {
            KeyCode::Enter => {
                self.filter_open = false;
                LedgerOutcome::Changed
            }
            KeyCode::Esc => {
                self.filter_open = false;
                self.active_filter_mut().clear();
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Backspace => {
                self.active_filter_mut().pop();
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.active_filter_mut().push(c);
                    match self.tab {
                        LedgerTab::Flows => self.reconcile_flow_selection(),
                        LedgerTab::Sessions => self.reconcile_session_selection(),
                    }
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn selected_session(&self) -> Option<&SessionSummary> {
        if self.tab != LedgerTab::Sessions {
            return None;
        }
        let id = self.session_selected_id.as_deref()?;
        let indices = self.filtered_session_indices();
        indices
            .into_iter()
            .find_map(|idx| self.sessions.get(idx).filter(|s| s.record_id == id))
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.last_render_height = area.height as usize;
        self.last_render_width = area.width as usize;

        // Row 0: Tab header
        let list_area = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        };

        let overflow_counter = self.compute_overflow_counter(list_area.height as usize);
        self.render_tab_row(
            buf,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            theme,
            overflow_counter.as_deref(),
        );

        if list_area.height == 0 {
            return;
        }

        match self.tab {
            LedgerTab::Flows => self.render_flows(buf, list_area, theme, now_ms),
            LedgerTab::Sessions => self.render_sessions(buf, list_area, theme, now_ms),
        }
    }

    fn compute_overflow_counter(&self, list_height: usize) -> Option<String> {
        if list_height == 0 {
            return None;
        }
        match self.tab {
            LedgerTab::Flows => {
                let rows = self.build_flow_rows(list_height);
                if rows.len() <= list_height {
                    return None;
                }
                let indices = self.filtered_flow_indices();
                let total = indices.len();
                if total == 0 {
                    return None;
                }
                let pos = self
                    .flow_selected_run_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.runs.get(i).map(|r| &r.run_id) == Some(id))
                    })
                    .unwrap_or(0)
                    + 1;
                Some(format!("{pos}/{total}"))
            }
            LedgerTab::Sessions => {
                let indices = self.filtered_session_indices();
                // 1 header row + indices.len() rows
                let total_rows = if indices.is_empty() {
                    0
                } else {
                    1 + indices.len()
                };
                if total_rows <= list_height {
                    return None;
                }
                let total = indices.len();
                let pos = self
                    .session_selected_id
                    .as_ref()
                    .and_then(|id| {
                        indices
                            .iter()
                            .position(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(id))
                    })
                    .unwrap_or(0)
                    + 1;
                Some(format!("{pos}/{total}"))
            }
        }
    }

    fn render_tab_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        theme: &Theme,
        overflow_counter: Option<&str>,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }

        let flow_count = self.runs.len();
        let sess_count = self.sessions.len();

        let flows_style = if self.tab == LedgerTab::Flows {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        let sess_style = if self.tab == LedgerTab::Sessions {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        let sep_style = Style::default().fg(theme.gray_dim);

        let left_flows = format!(" Flows {flow_count} ");
        let left_sep = "│";
        let left_sess = format!(" Sessions {sess_count} ");
        let left_width = left_flows.width() + left_sep.width() + left_sess.width();

        let left_line = Line::from(vec![
            Span::styled(left_flows, flows_style),
            Span::styled(left_sep, sep_style),
            Span::styled(left_sess, sess_style),
        ]);
        buf.set_line(row.x, row.y, &left_line, row.width);

        // Build right-aligned components:
        // Summary, Filter/active, [order], overflow_counter
        // Drop order when overlapping tabs: [order], then filter text, then summary.
        let summary_spans = self.build_summary_spans(theme);
        let filter_spans = self.build_filter_spans(theme);
        let order_label = match self.tab {
            LedgerTab::Flows => self.flow_order.label(),
            LedgerTab::Sessions => self.session_order.label(),
        };
        let order_spans = vec![Span::styled(
            format!("[{order_label}]"),
            Style::default().fg(theme.gray_dim),
        )];
        let counter_spans: Vec<Span<'static>> = overflow_counter
            .map(|c| vec![Span::styled(c.to_string(), Style::default().fg(theme.gray))])
            .unwrap_or_default();

        // Try combinations in priority order:
        // 1. summary + filter + order + counter
        // 2. summary + filter + counter (drop [order])
        // 3. summary + counter (drop filter)
        // 4. counter only (drop summary)
        let candidates: [&[&Vec<Span<'static>>]; 4] = [
            &[&summary_spans, &filter_spans, &order_spans, &counter_spans],
            &[&summary_spans, &filter_spans, &counter_spans],
            &[&summary_spans, &counter_spans],
            &[&counter_spans],
        ];

        let avail = w.saturating_sub(left_width + 1);
        for group in candidates {
            let joined = join_span_groups(group, theme);
            let jw: usize = joined.iter().map(|s| s.content.width()).sum();
            if jw > 0 && jw <= avail {
                let rx = row.x.saturating_add((w - jw) as u16);
                let line = Line::from(joined);
                buf.set_line(rx, row.y, &line, jw as u16);
                break;
            }
        }
    }

    fn build_summary_spans(&self, theme: &Theme) -> Vec<Span<'static>> {
        let waiting = self
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Waiting)
            .count();
        let running = self
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Running && !self.is_stalled(r))
            .count();

        let mut spans = Vec::new();
        if waiting > 0 {
            spans.push(Span::styled(
                format!("{waiting} needs you"),
                Style::default().fg(theme.warning),
            ));
        }
        if running > 0 {
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", Style::default().fg(theme.gray_dim)));
            }
            spans.push(Span::styled(
                format!("{running} running"),
                Style::default().fg(theme.accent_running),
            ));
        }
        spans
    }

    fn build_filter_spans(&self, theme: &Theme) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let filter = self.active_filter();
        if self.filter_open || !filter.is_empty() {
            let text = if self.filter_open {
                format!("/{filter}▏")
            } else {
                format!("/{filter}")
            };
            spans.push(Span::styled(text, Style::default().fg(theme.command)));
        }
        let active_only = match self.tab {
            LedgerTab::Flows => self.flow_active_only,
            LedgerTab::Sessions => self.session_active_only,
        };
        if active_only {
            if !spans.is_empty() {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                "active",
                Style::default().fg(theme.accent_running),
            ));
        }
        spans
    }

    fn render_flows(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        let list_h = area.height as usize;
        let rows = self.build_flow_rows(list_h);

        if rows.is_empty() {
            let msg = if !self.flow_filter.is_empty() || self.flow_active_only {
                "No matching flow runs"
            } else {
                "No acpx flow runs found"
            };
            render_empty_centered(buf, area, msg, theme);
            return;
        }

        // Find visual row index of the selected run
        let selected_visual_idx = self.flow_selected_run_id.as_ref().and_then(|sel_id| {
            rows.iter().position(|row| match row {
                FlowListRow::Run(idx) => self.runs.get(*idx).map(|r| &r.run_id) == Some(sel_id),
                _ => false,
            })
        });

        // Adjust scroll to keep selection visible (and include header row immediately above if at top)
        if let Some(sel_v) = selected_visual_idx {
            if sel_v < self.flow_scroll {
                self.flow_scroll = sel_v;
                // If the row right above `sel_v` is a section header, show it too when possible
                if sel_v > 0
                    && matches!(rows.get(sel_v - 1), Some(FlowListRow::Header { .. }))
                    && list_h > 1
                {
                    self.flow_scroll = sel_v - 1;
                }
            } else if sel_v >= self.flow_scroll + list_h {
                self.flow_scroll = (sel_v + 1).saturating_sub(list_h);
            }
        }
        let max_scroll = rows.len().saturating_sub(list_h);
        if self.flow_scroll > max_scroll {
            self.flow_scroll = max_scroll;
        }

        for (line_offset, row) in rows
            .iter()
            .skip(self.flow_scroll)
            .take(list_h)
            .enumerate()
        {
            let y = area.y.saturating_add(line_offset as u16);
            let row_rect = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            match row {
                FlowListRow::Blank => {}
                FlowListRow::Header { kind, count } => {
                    self.render_section_header(buf, row_rect, *kind, *count, theme);
                }
                FlowListRow::Run(idx) => {
                    if let Some(run) = self.runs.get(*idx) {
                        let is_selected =
                            self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
                        self.render_flow_row(buf, row_rect, run, is_selected, theme, now_ms);
                    }
                }
            }
        }
    }

    fn render_section_header(
        &self,
        buf: &mut Buffer,
        row: Rect,
        kind: SectionKind,
        count: usize,
        theme: &Theme,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }
        let label = format!("{} {} ", kind.label(), count);
        let label_cut = truncate_str(&label, w);
        let lw = label_cut.width();
        let rule_len = w.saturating_sub(lw);
        let rule = "─".repeat(rule_len);

        let line = Line::from(vec![
            Span::styled(
                label_cut,
                Style::default()
                    .fg(kind.color(theme))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rule, Style::default().fg(theme.gray_dim)),
        ]);
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn render_flow_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        run: &RunSummary,
        selected: bool,
        theme: &Theme,
        now_ms: i64,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }

        let base_mod = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        if selected {
            buf.set_style(row, Style::default().bg(theme.bg_highlight));
        }

        let extra = self.extras.get(&run.run_id);
        let stalled = extra.map(|e| e.stalled).unwrap_or(false);

        // Status glyph
        let (glyph, glyph_color) = if stalled {
            ("⊘", theme.gray)
        } else {
            let c = match run.status {
                RunStatus::Running => theme.accent_running,
                RunStatus::Waiting => theme.warning,
                RunStatus::Completed => theme.accent_success,
                RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
                RunStatus::Unknown => theme.gray_dim,
            };
            (run.status.glyph(), c)
        };

        // Optional right/middle columns dropped by width thresholds:
        // agents below 120, duration below 96, <done>/<total> below 80, age below 64, rail below 48.
        let show_rail = w >= 48;
        let show_age = w >= 64;
        let show_rail_count = w >= 80;
        let show_duration = w >= 96;
        let show_agents = w >= 120;

        // Build rail spans + width
        let mut rail_spans: Vec<Span<'static>> = Vec::new();
        let mut rail_width: usize = 0;
        if show_rail
            && let Some(ex) = extra
            && !ex.rail.is_empty()
        {
            for &ns in &ex.rail {
                let g = ns.glyph();
                let color = match ns {
                    NodeState::Running => theme.accent_running,
                    NodeState::Waiting => theme.warning,
                    NodeState::Done(Outcome::Ok) => theme.accent_success,
                    NodeState::Done(Outcome::Failed | Outcome::TimedOut) => theme.accent_error,
                    _ => theme.gray_dim,
                };
                rail_width += g.width();
                rail_spans.push(Span::styled(
                    g.to_string(),
                    Style::default().fg(color).add_modifier(base_mod),
                ));
            }
            if show_rail_count {
                let done = ex
                    .rail
                    .iter()
                    .filter(|s| matches!(s, NodeState::Done(_)))
                    .count();
                let total = ex.rail.len();
                let cnt = format!(" {done}/{total}");
                rail_width += cnt.width();
                rail_spans.push(Span::styled(
                    cnt,
                    Style::default().fg(theme.gray).add_modifier(base_mod),
                ));
            }
        }

        let agents_str = if show_agents {
            extra
                .map(|e| e.agents.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| truncate_str(s, 26))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let age_str = if show_age {
            fmt_age(now_ms.saturating_sub(run.updated_ms))
        } else {
            String::new()
        };

        let dur_str = if show_duration {
            let end_ms = if run.status.is_live() && !stalled {
                now_ms
            } else {
                run.finished_ms.unwrap_or(run.updated_ms)
            };
            let dur_ms = (end_ms - run.started_ms).max(0) as u64;
            fmt_duration(dur_ms)
        } else {
            String::new()
        };

        // Raw state text before cutting
        let (raw_state_text, state_color) = self.flow_state_text(run, extra, stalled, theme);

        // Calculate fixed columns width:
        // "<glyph> " (2 cols) + optional gaps & columns
        let prefix_w = glyph.width() + 1;
        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
        let agents_col_w = if !agents_str.is_empty() {
            1 + agents_str.width()
        } else {
            0
        };
        let age_col_w = if !age_str.is_empty() {
            1 + age_str.width()
        } else {
            0
        };
        let dur_col_w = if !dur_str.is_empty() {
            1 + dur_str.width()
        } else {
            0
        };

        let fixed_w = prefix_w + rail_col_w + agents_col_w + age_col_w + dur_col_w;
        let flex_budget = w.saturating_sub(fixed_w);

        // Title and state text share what is left, title first (at least 16 columns), state text cut to fit.
        let title_str;
        let state_str;
        if flex_budget == 0 {
            title_str = String::new();
            state_str = String::new();
        } else if raw_state_text.is_empty() || flex_budget <= 17 {
            title_str = truncate_str(&run.title, flex_budget);
            state_str = String::new();
        } else {
            // Need 1 space separator between title and state
            let avail_pair = flex_budget - 1;
            let raw_title_w = run.title.width();
            let raw_state_w = raw_state_text.width();
            if raw_title_w + raw_state_w <= avail_pair {
                title_str = run.title.clone();
                state_str = raw_state_text;
            } else {
                // Title gets at least 16 columns (or half of avail_pair if larger, capped at raw_title_w)
                let half = (avail_pair / 2).max(16).min(avail_pair);
                let title_max = if raw_state_w < avail_pair.saturating_sub(half) {
                    avail_pair.saturating_sub(raw_state_w)
                } else {
                    half.min(raw_title_w.max(16)).min(avail_pair)
                };
                title_str = truncate_str(&run.title, title_max);
                let used_title = title_str.width();
                let state_budget = avail_pair.saturating_sub(used_title);
                state_str = if state_budget > 0 {
                    truncate_str(&raw_state_text, state_budget)
                } else {
                    String::new()
                };
            }
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled(
            format!("{glyph} "),
            Style::default().fg(glyph_color).add_modifier(base_mod),
        ));
        if !title_str.is_empty() {
            spans.push(Span::styled(
                title_str.clone(),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(base_mod),
            ));
        }
        if ! rail_spans.is_empty() {
            spans.push(Span::raw(" "));
            spans.extend(rail_spans);
        }
        if !state_str.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                state_str,
                Style::default().fg(state_color).add_modifier(base_mod),
            ));
        }

        // Right-aligned metadata columns (agents, age, duration) so rows align cleanly
        let mut right_spans: Vec<Span<'static>> = Vec::new();
        if !agents_str.is_empty() {
            right_spans.push(Span::raw(" "));
            right_spans.push(Span::styled(
                agents_str,
                Style::default().fg(theme.gray).add_modifier(base_mod),
            ));
        }
        if !age_str.is_empty() {
            right_spans.push(Span::raw(" "));
            right_spans.push(Span::styled(
                age_str,
                Style::default().fg(theme.gray).add_modifier(base_mod),
            ));
        }
        if !dur_str.is_empty() {
            right_spans.push(Span::raw(" "));
            right_spans.push(Span::styled(
                dur_str,
                Style::default().fg(theme.gray_dim).add_modifier(base_mod),
            ));
        }

        let left_w: usize = spans.iter().map(|s| s.content.width()).sum();
        let right_w: usize = right_spans.iter().map(|s| s.content.width()).sum();
        if right_w > 0 && left_w + right_w < w {
            spans.push(Span::raw(" ".repeat(w - left_w - right_w)));
        }
        spans.extend(right_spans);

        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn flow_state_text(
        &self,
        run: &RunSummary,
        extra: Option<&RowExtra>,
        stalled: bool,
        theme: &Theme,
    ) -> (String, Color) {
        if run.status == RunStatus::Waiting {
            if let Some(q) = extra.and_then(|e| e.question.as_deref()).filter(|q| !q.is_empty()) {
                return (q.to_string(), theme.warning);
            }
            let wait_target = run.waiting_on.as_deref().unwrap_or("checkpoint");
            return (format!("waiting: {wait_target}"), theme.warning);
        }
        if stalled {
            let node = run.current_node.as_deref().unwrap_or("unknown");
            return (format!("stalled at {node}"), theme.gray);
        }
        match run.status {
            RunStatus::Running => {
                let node = run.current_node.as_deref().unwrap_or("running");
                let text = match run.status_detail.as_deref().filter(|d| !d.is_empty()) {
                    Some(detail) => format!("{node} · {detail}"),
                    None => node.to_string(),
                };
                (text, theme.accent_running)
            }
            RunStatus::Failed | RunStatus::TimedOut => {
                let err = run
                    .error
                    .as_deref()
                    .or(run.status_detail.as_deref())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| run.status.label());
                (err.to_string(), theme.accent_error)
            }
            RunStatus::Completed => ("done".to_string(), theme.gray),
            RunStatus::Waiting => unreachable!(),
            RunStatus::Unknown => (run.status.label().to_string(), theme.gray_dim),
        }
    }

    fn render_sessions(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        let indices = self.filtered_session_indices();
        if indices.is_empty() {
            let msg = if !self.session_filter.is_empty() || self.session_active_only {
                "No matching sessions"
            } else {
                "No acpx sessions found"
            };
            render_empty_centered(buf, area, msg, theme);
            return;
        }

        // Row 0 of `area` is the column header row in `theme.gray_dim`
        self.render_session_header(
            buf,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            theme,
        );

        let rows_h = area.height.saturating_sub(1) as usize;
        if rows_h == 0 {
            return;
        }

        let selected_pos = self
            .session_selected_id
            .as_ref()
            .and_then(|id| {
                indices
                    .iter()
                    .position(|&i| self.sessions.get(i).map(|s| &s.record_id) == Some(id))
            })
            .unwrap_or(0);

        if selected_pos < self.session_scroll {
            self.session_scroll = selected_pos;
        } else if selected_pos >= self.session_scroll + rows_h {
            self.session_scroll = (selected_pos + 1).saturating_sub(rows_h);
        }
        let max_scroll = indices.len().saturating_sub(rows_h);
        if self.session_scroll > max_scroll {
            self.session_scroll = max_scroll;
        }

        for (line_offset, &sess_idx) in indices
            .iter()
            .skip(self.session_scroll)
            .take(rows_h)
            .enumerate()
        {
            if let Some(sess) = self.sessions.get(sess_idx) {
                let y = area.y.saturating_add(1 + line_offset as u16);
                let row_rect = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                };
                let is_selected =
                    self.session_selected_id.as_deref() == Some(sess.record_id.as_str());
                self.render_session_row(buf, row_rect, sess, is_selected, theme, now_ms);
            }
        }
    }

    fn render_session_header(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }
        let layout = SessionColumns::for_width(w);
        let style = Style::default().fg(theme.gray_dim);
        let line_str = layout.format_row(" ", "SESSION", "AGENT", "CWD", "MSGS", "TOKENS", "COST", "AGE");
        let line = Line::from(Span::styled(truncate_str(&line_str, w), style));
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn render_session_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        sess: &SessionSummary,
        selected: bool,
        theme: &Theme,
        now_ms: i64,
    ) {
        let w = row.width as usize;
        if w == 0 {
            return;
        }
        let base_mod = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        if selected {
            buf.set_style(row, Style::default().bg(theme.bg_highlight));
        }

        let layout = SessionColumns::for_width(w);
        let (glyph, glyph_color) = if sess.closed {
            ("○", theme.gray_dim)
        } else {
            ("●", theme.accent_running)
        };

        let cwd_last = last_path_component(&sess.cwd);
        let msgs = sess.messages.to_string();
        let tokens = fmt_tokens(sess.tokens);
        let cost = sess
            .cost_usd
            .map(|c| format!("${c:.2}"))
            .unwrap_or_default();
        let age = fmt_age(now_ms.saturating_sub(sess.last_used_ms));

        let spans = layout.build_spans(
            glyph,
            glyph_color,
            &sess.title,
            &sess.agent,
            cwd_last,
            &msgs,
            &tokens,
            &cost,
            &age,
            base_mod,
            theme,
        );
        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, row.width);
    }
}

struct SessionColumns {
    width: usize,
    show_agent: bool,
    show_cwd: bool,
    show_msgs: bool,
    show_tokens: bool,
    show_cost: bool,
    show_age: bool,
}

impl SessionColumns {
    const AGENT_W: usize = 12;
    const CWD_W: usize = 14;
    const MSGS_W: usize = 5;
    const TOKENS_W: usize = 7;
    const COST_W: usize = 7;
    const AGE_W: usize = 5;

    fn for_width(width: usize) -> Self {
        // Drop columns from the right as width shrinks:
        // Order from left to right: glyph(2) + title + agent + cwd + msgs + tokens + cost + age
        // Drop from right: age (<56), cost (<110), tokens (<96), msgs (<84), cwd (<70), agent (<54)
        let show_agent = width >= 46;
        let show_cwd = width >= 64;
        let show_msgs = width >= 74;
        let show_tokens = width >= 86;
        let show_cost = width >= 98;
        let show_age = width >= 108;
        Self {
            width,
            show_agent,
            show_cwd,
            show_msgs,
            show_tokens,
            show_cost,
            show_age,
        }
    }

    fn title_width(&self) -> usize {
        let mut fixed = 2; // glyph + space
        if self.show_agent {
            fixed += 1 + Self::AGENT_W;
        }
        if self.show_cwd {
            fixed += 1 + Self::CWD_W;
        }
        if self.show_msgs {
            fixed += 1 + Self::MSGS_W;
        }
        if self.show_tokens {
            fixed += 1 + Self::TOKENS_W;
        }
        if self.show_cost {
            fixed += 1 + Self::COST_W;
        }
        if self.show_age {
            fixed += 1 + Self::AGE_W;
        }
        self.width.saturating_sub(fixed).max(1)
    }

    #[allow(clippy::too_many_arguments)]
    fn format_row(
        &self,
        glyph: &str,
        title: &str,
        agent: &str,
        cwd: &str,
        msgs: &str,
        tokens: &str,
        cost: &str,
        age: &str,
    ) -> String {
        let tw = self.title_width();
        let mut out = format!("{} {}", glyph, pad_right(&truncate_str(title, tw), tw));
        if self.show_agent {
            out.push(' ');
            out.push_str(&pad_right(&truncate_str(agent, Self::AGENT_W), Self::AGENT_W));
        }
        if self.show_cwd {
            out.push(' ');
            out.push_str(&pad_right(&truncate_str(cwd, Self::CWD_W), Self::CWD_W));
        }
        if self.show_msgs {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(msgs, Self::MSGS_W), Self::MSGS_W));
        }
        if self.show_tokens {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(tokens, Self::TOKENS_W), Self::TOKENS_W));
        }
        if self.show_cost {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(cost, Self::COST_W), Self::COST_W));
        }
        if self.show_age {
            out.push(' ');
            out.push_str(&pad_left(&truncate_str(age, Self::AGE_W), Self::AGE_W));
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn build_spans(
        &self,
        glyph: &str,
        glyph_color: Color,
        title: &str,
        agent: &str,
        cwd: &str,
        msgs: &str,
        tokens: &str,
        cost: &str,
        age: &str,
        base_mod: Modifier,
        theme: &Theme,
    ) -> Vec<Span<'static>> {
        let tw = self.title_width();
        let mut spans = Vec::new();
        spans.push(Span::styled(
            format!("{glyph} "),
            Style::default().fg(glyph_color).add_modifier(base_mod),
        ));
        spans.push(Span::styled(
            pad_right(&truncate_str(title, tw), tw),
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(base_mod),
        ));
        let sec_style = Style::default().fg(theme.gray).add_modifier(base_mod);
        if self.show_agent {
            spans.push(Span::styled(
                format!(" {}", pad_right(&truncate_str(agent, Self::AGENT_W), Self::AGENT_W)),
                sec_style,
            ));
        }
        if self.show_cwd {
            spans.push(Span::styled(
                format!(" {}", pad_right(&truncate_str(cwd, Self::CWD_W), Self::CWD_W)),
                sec_style,
            ));
        }
        if self.show_msgs {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(msgs, Self::MSGS_W), Self::MSGS_W)),
                sec_style,
            ));
        }
        if self.show_tokens {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(tokens, Self::TOKENS_W), Self::TOKENS_W)),
                sec_style,
            ));
        }
        if self.show_cost {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(cost, Self::COST_W), Self::COST_W)),
                sec_style,
            ));
        }
        if self.show_age {
            spans.push(Span::styled(
                format!(" {}", pad_left(&truncate_str(age, Self::AGE_W), Self::AGE_W)),
                sec_style,
            ));
        }
        spans
    }
}

fn pad_right(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - w))
    }
}

fn last_path_component(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return path;
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
}

fn fmt_tokens(tokens: u64) -> String {
    if tokens == 0 {
        return "0".to_string();
    }
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn join_span_groups(groups: &[&Vec<Span<'static>>], theme: &Theme) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for g in groups {
        if g.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(Span::styled("  ", Style::default().fg(theme.gray_dim)));
        }
        out.extend((*g).iter().cloned());
    }
    out
}

fn render_empty_centered(buf: &mut Buffer, area: Rect, msg: &str, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let y = area.y.saturating_add(area.height / 2);
    let cut = truncate_str(msg, area.width as usize);
    let mw = cut.width() as u16;
    let x = area.x.saturating_add(area.width.saturating_sub(mw) / 2);
    let line = Line::from(Span::styled(cut, Style::default().fg(theme.gray_dim)));
    buf.set_line(x, y, &line, area.width.saturating_sub(x - area.x));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_run(id: &str, title: &str, status: RunStatus, updated_ms: i64) -> RunSummary {
        RunSummary {
            run_id: id.to_string(),
            dir: PathBuf::from(format!("/tmp/runs/{id}")),
            flow_name: format!("flow-{id}"),
            title: title.to_string(),
            status,
            started_ms: updated_ms - 5_000,
            updated_ms,
            finished_ms: matches!(
                status,
                RunStatus::Completed | RunStatus::Failed | RunStatus::TimedOut
            )
            .then_some(updated_ms),
            current_node: Some("build_node".to_string()),
            waiting_on: (status == RunStatus::Waiting).then(|| "approve_step".to_string()),
            status_detail: None,
            error: (status == RunStatus::Failed).then(|| "exit code 1".to_string()),
        }
    }

    fn make_session(id: &str, title: &str, closed: bool, last_used_ms: i64) -> SessionSummary {
        SessionSummary {
            record_id: id.to_string(),
            acp_session_id: format!("acp-{id}"),
            name: format!("name-{id}"),
            title: title.to_string(),
            agent: "grok".to_string(),
            cwd: "/workspace/my-repo".to_string(),
            created_ms: last_used_ms - 10_000,
            last_used_ms,
            closed,
            messages: 12,
            tokens: 12_300,
            cost_usd: Some(0.12),
            stream_path: PathBuf::from(format!("/tmp/sessions/{id}.stream.ndjson")),
        }
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        let w = buf.area.width;
        let mut s = String::new();
        for x in 0..w {
            s.push_str(buf[(x, y)].symbol());
        }
        s.trim_end().to_string()
    }

    fn all_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn grouped_order_and_navigation_skips_headers_and_clamps() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-done", "Done Run", RunStatus::Completed, 3000),
            make_run("r-wait", "Waiting Run", RunStatus::Waiting, 1000),
            make_run("r-live", "Running Run", RunStatus::Running, 2000),
        ]);

        // Waiting run is selected first even though Completed is newer
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Moving up at top clamps
        assert_eq!(state.handle_key(&key(KeyCode::Up)), LedgerOutcome::Unchanged);
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Moving down skips RUNNING header and lands on r-live
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('j'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-live")
        );

        // Moving down again skips FINISHED header and lands on r-done
        assert_eq!(
            state.handle_key(&key(KeyCode::Down)),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );

        // Clamps at bottom
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('j'))),
            LedgerOutcome::Unchanged
        );

        // Jump start and end
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('g'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('G'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );
    }

    #[test]
    fn stalled_running_run_is_placed_under_finished_with_stalled_at() {
        let mut state = LedgerState::new();
        let mut stalled_run = make_run("r-stalled", "Stalled Run", RunStatus::Running, 5000);
        stalled_run.current_node = Some("deploy_step".to_string());
        let done_run = make_run("r-done", "Older Done", RunStatus::Completed, 2000);
        state.set_runs(vec![done_run, stalled_run]);

        let mut extras = HashMap::new();
        extras.insert(
            "r-stalled".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: true,
                question: None,
            },
        );
        state.set_extras(extras);

        // Stalled run is not considered live
        assert!(!state.any_live());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        state.render(&mut buf, buf.area, &theme, 10_000);
        let text = all_text(&buf);
        assert!(!text.contains("RUNNING 1"));
        assert!(text.contains("FINISHED 2"));
        assert!(text.contains("⊘ Stalled Run"));
        assert!(text.contains("stalled at deploy_step"));
    }

    #[test]
    fn tab_switching_keeps_per_tab_selection_and_selected_run_behavior() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "First Run", RunStatus::Completed, 2000),
            make_run("r2", "Second Run", RunStatus::Completed, 1000),
        ]);
        state.set_sessions(vec![
            make_session("s1", "First Session", false, 2000),
            make_session("s2", "Second Session", false, 1000),
        ]);

        // Select second run on Flows tab
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Switch to Sessions tab
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), LedgerOutcome::Changed);
        assert_eq!(state.tab(), LedgerTab::Sessions);
        assert!(state.selected_run().is_none());

        // Move to second session
        state.handle_key(&key(KeyCode::Down));

        // Switch back to Flows tab: r2 is still selected
        state.handle_key(&key(KeyCode::Char('h')));
        assert_eq!(state.tab(), LedgerTab::Flows);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Switch back to Sessions tab: s2 is still selected
        state.handle_key(&key(KeyCode::Right));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s2", "Second Session", false, 1000))
        );
    }

    #[test]
    fn filter_typing_enter_and_esc_ordering() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Alpha deploy", RunStatus::Completed, 2000),
            make_run("r2", "Beta test", RunStatus::Completed, 1000),
        ]);

        assert!(state.esc_closes());
        assert!(!state.captures_text());

        // Open filter with `/`
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('/'))),
            LedgerOutcome::Changed
        );
        assert!(state.captures_text());
        assert!(!state.esc_closes());

        // Type "beta" (note: 'a' would toggle active-only if filter weren't capturing)
        for ch in ['b', 'e', 't', 'a', 'x'] {
            state.handle_key(&key(KeyCode::Char(ch)));
        }
        state.handle_key(&key(KeyCode::Backspace));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Enter closes input but keeps filter
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::Changed
        );
        assert!(!state.captures_text());
        assert!(!state.esc_closes());
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // First Esc clears the non-empty filter (Changed)
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), LedgerOutcome::Changed);
        assert!(state.esc_closes());
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));

        // Second Esc returns Close
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), LedgerOutcome::Close);
    }

    #[test]
    fn active_only_toggle_filters_runs_and_sessions() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-done", "Done", RunStatus::Completed, 3000),
            make_run("r-live", "Live", RunStatus::Running, 1000),
        ]);
        state.set_sessions(vec![
            make_session("s-closed", "Closed Sess", true, 3000),
            make_session("s-open", "Open Sess", false, 1000),
        ]);

        // Toggle active-only on Flows
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('a'))),
            LedgerOutcome::Changed
        );
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-live")
        );
        // Moving down clamps because r-done is filtered out
        assert_eq!(
            state.handle_key(&key(KeyCode::Down)),
            LedgerOutcome::Unchanged
        );

        // Switch to Sessions and toggle active-only
        state.handle_key(&key(KeyCode::Tab));
        state.handle_key(&key(KeyCode::Char('a')));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s-open", "Open Sess", false, 1000))
        );
    }

    #[test]
    fn order_cycling_on_flows_and_sessions() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait", "Zeta Waiting", RunStatus::Waiting, 1000),
            make_run("r-done", "Alpha Done", RunStatus::Completed, 3000),
        ]);
        state.set_sessions(vec![
            make_session("s1", "Zeta Session", false, 3000),
            make_session("s2", "Alpha Session", false, 1000),
        ]);

        // Grouped: Waiting first
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Cycle to Recent: r-done (3000) is above r-wait (1000), jump_start selects r-done
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );

        // Cycle to Name: Alpha Done first
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-done")
        );

        // Cycle back to Grouped: Waiting first
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-wait")
        );

        // Sessions tab cycles Recent -> Name -> Recent
        state.handle_key(&key(KeyCode::Tab));
        state.handle_key(&key(KeyCode::Char('s'))); // Name
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s2", "Alpha Session", false, 1000))
        );
    }

    #[test]
    fn enter_y_and_rescan_outcomes_on_both_tabs() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run("r1", "Run 1", RunStatus::Completed, 1000)]);
        state.set_sessions(vec![make_session("s1", "Sess 1", false, 1000)]);

        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenRun(PathBuf::from("/tmp/runs/r1"))
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("/tmp/runs/r1".to_string())
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('R'))),
            LedgerOutcome::Rescan
        );

        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(make_session("s1", "Sess 1", false, 1000))
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("s1".to_string())
        );
    }

    #[test]
    fn set_runs_preserves_selected_run_id_and_any_live() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Run 1", RunStatus::Completed, 2000),
            make_run("r2", "Run 2", RunStatus::Completed, 1000),
        ]);
        assert!(!state.any_live());

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Refresh runs with a new live run at the top; r2 stays selected
        state.set_runs(vec![
            make_run("r3", "Run 3", RunStatus::Running, 3000),
            make_run("r1", "Run 1", RunStatus::Completed, 2000),
            make_run("r2", "Run 2", RunStatus::Completed, 1000),
        ]);
        assert!(state.any_live());
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
    }

    #[test]
    fn render_at_120x14_shows_tabs_needs_you_question_rail_and_count() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait", "Release pipeline", RunStatus::Waiting, 9_000),
            make_run("r-run", "Nightly bench", RunStatus::Running, 8_000),
            make_run("r-done", "Unit suite", RunStatus::Completed, 5_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Failed),
                    NodeState::Waiting,
                    NodeState::NotRun,
                ],
                agents: "grok-build · grok-4".to_string(),
                stalled: false,
                question: Some("Ship to prod?".to_string()),
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        state.render(&mut buf, buf.area, &theme, 10_000);

        let r0 = row_text(&buf, 0);
        assert!(r0.contains("Flows 3"));
        assert!(r0.contains("Sessions 0"));
        assert!(r0.contains("1 needs you · 1 running"));
        assert!(r0.contains("[grouped]"));

        let full = all_text(&buf);
        assert!(full.contains("NEEDS YOU 1"));
        assert!(full.contains("Release pipeline"));
        assert!(full.contains("✓✓✓✗◐○ 4/6"));
        assert!(full.contains("Ship to prod?"));
        assert!(full.contains("grok-build · grok-4"));
    }

    #[test]
    fn render_at_50x8_keeps_title_and_does_not_panic() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run(
            "r-wait",
            "Compact Title Check",
            RunStatus::Waiting,
            9_000,
        )]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Done(Outcome::Ok), NodeState::Waiting],
                agents: "grok-agent".to_string(),
                stalled: false,
                question: Some("Approve?".to_string()),
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 8));
        state.render(&mut buf, buf.area, &theme, 10_000);

        let full = all_text(&buf);
        assert!(full.contains("Compact Title"));
        // Below 80 columns, <done>/<total> is dropped
        assert!(!full.contains("1/2"));
    }

    #[test]
    fn render_at_0x0_and_empty_state_do_not_panic() {
        let mut state = LedgerState::new();
        let theme = Theme::default();

        let mut zero_buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        state.render(&mut zero_buf, zero_buf.area, &theme, 10_000);

        let mut empty_buf = Buffer::empty(Rect::new(0, 0, 80, 10));
        state.render(&mut empty_buf, empty_buf.area, &theme, 10_000);
        let full = all_text(&empty_buf);
        assert!(full.contains("No acpx flow runs found"));
    }
}
```