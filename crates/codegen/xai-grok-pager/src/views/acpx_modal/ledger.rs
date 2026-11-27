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

/// The gate this run answered: read from this run's own input, written by `bundle::gate_input` when a gate answer
/// starts the next flow file. `projections/run.json` -> `input.gate = { "runId": <the waiting run>, "node": .., "choice": .. }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateAnswer {
    pub run_id: String,   // input.gate.runId: the run that was waiting at the gate
    pub choice: String,   // input.gate.choice: the label that was picked
}

/// How many step marks a runs-list row shows at most. A workflow with
/// hundreds of nodes would otherwise stretch the marks column past every
/// other column; the run view keeps the full picture.
const RAIL_MARKS_CAP: usize = 24;

/// Per-run facts the parent loads (cached) so the ledger does no I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
    pub answers: Option<GateAnswer>,
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
    Continuation { indent: usize, text: String },
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
    answered: HashMap<String, String>,
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
            answered: HashMap::new(),
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
        self.rebuild_answered();
        self.reconcile_flow_selection();
    }

    pub fn set_extras(&mut self, extras: HashMap<String, RowExtra>) {
        self.extras = extras;
        self.rebuild_answered();
        self.reconcile_flow_selection();
    }

    pub fn answered_choice(&self, run_id: &str) -> Option<&str> {
        if self
            .runs
            .iter()
            .find(|r| r.run_id == run_id)
            .is_some_and(|r| r.status != RunStatus::Waiting)
        {
            return None;
        }
        self.answered.get(run_id).map(String::as_str)
    }

    fn rebuild_answered(&mut self) {
        let mut winners: HashMap<String, (&str, Option<i64>, &str)> = HashMap::new();
        let mut keys: Vec<&String> = self.extras.keys().collect();
        keys.sort();
        for answering_id in keys {
            if let Some(extra) = self.extras.get(answering_id)
                && let Some(ref ans) = extra.answers
            {
                let started = self
                    .runs
                    .iter()
                    .find(|r| &r.run_id == answering_id)
                    .map(|r| r.started_ms);
                let replace = match winners.get(&ans.run_id) {
                    None => true,
                    Some(&(prev_id, prev_started, _)) => match (prev_started, started) {
                        (Some(ps), Some(ns)) => (ns, answering_id.as_str()) > (ps, prev_id),
                        _ => answering_id.as_str() > prev_id,
                    },
                };
                if replace {
                    winners.insert(
                        ans.run_id.clone(),
                        (answering_id.as_str(), started, ans.choice.as_str()),
                    );
                }
            }
        }
        self.answered = winners
            .into_iter()
            .map(|(k, (_, _, choice))| (k, choice.to_string()))
            .collect();
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

    fn is_answered(&self, run: &RunSummary) -> bool {
        run.status == RunStatus::Waiting && self.answered.contains_key(&run.run_id)
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
        matches!(run.status, RunStatus::Running | RunStatus::Waiting)
            && !self.is_stalled(run)
            && !self.is_answered(run)
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
        if run.status == RunStatus::Waiting && !self.is_answered(run) {
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
    fn build_flow_rows(&self, list_height: usize, now_ms: i64) -> Vec<FlowListRow> {
        let indices = self.filtered_flow_indices();
        if indices.is_empty() {
            return Vec::new();
        }
        if self.flow_order != FlowOrder::Grouped {
            let mut out = Vec::new();
            for idx in indices {
                out.push(FlowListRow::Run(idx));
                self.append_continuations(&mut out, idx, now_ms);
            }
            return out;
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

        let extra_rows: usize = sections
            .iter()
            .flat_map(|(_, items)| items.iter())
            .map(|&idx| self.continuation_lines_for(idx, now_ms).len())
            .sum();
        let non_blank_rows: usize = sections
            .iter()
            .map(|(_, items)| 1 + items.len())
            .sum::<usize>()
            + extra_rows;
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
                self.append_continuations(&mut out, idx, now_ms);
            }
        }
        out
    }

    fn append_continuations(&self, out: &mut Vec<FlowListRow>, idx: usize, now_ms: i64) {
        for (indent, text) in self.continuation_lines_for(idx, now_ms) {
            out.push(FlowListRow::Continuation { indent, text });
        }
    }

    fn continuation_lines_for(&self, idx: usize, now_ms: i64) -> Vec<(usize, String)> {
        let Some(run) = self.runs.get(idx) else {
            return Vec::new();
        };
        let is_selected = self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
        if !is_selected || run.status != RunStatus::Waiting || self.is_answered(run) {
            return Vec::new();
        }
        let (_, indent, lines) = self.flow_question_lines(run, self.last_render_width, now_ms);
        lines.into_iter().skip(1).map(|t| (indent, t)).collect()
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

        let overflow_counter = self.compute_overflow_counter(list_area.height as usize, now_ms);
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

    fn compute_overflow_counter(&self, list_height: usize, now_ms: i64) -> Option<String> {
        if list_height == 0 {
            return None;
        }
        match self.tab {
            LedgerTab::Flows => {
                let rows = self.build_flow_rows(list_height, now_ms);
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
        let left_sess = format!(" Agent Turns {sess_count} ");
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
            .filter(|r| r.status == RunStatus::Waiting && !self.is_answered(r))
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
        let rows = self.build_flow_rows(list_h, now_ms);

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
            let mut sel_end = sel_v + 1;
            while matches!(rows.get(sel_end), Some(FlowListRow::Continuation { .. })) {
                sel_end += 1;
            }
            let sel_h = sel_end - sel_v;
            if sel_v < self.flow_scroll {
                self.flow_scroll = sel_v;
                // If the row right above `sel_v` is a section header, show it too when possible
                if sel_v > 0
                    && matches!(rows.get(sel_v - 1), Some(FlowListRow::Header { .. }))
                    && list_h > sel_h
                {
                    self.flow_scroll = sel_v - 1;
                }
            } else if sel_end > self.flow_scroll + list_h {
                self.flow_scroll = sel_end.saturating_sub(list_h).min(sel_v);
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
                FlowListRow::Continuation { indent, text } => {
                    buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
                    let content = format!("{}{}", " ".repeat(*indent), text);
                    let line = Line::from(Span::styled(
                        truncate_str(&content, row_rect.width as usize),
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ));
                    buf.set_line(row_rect.x, row_rect.y, &line, row_rect.width);
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
        let answered = self.is_answered(run);

        // Status glyph
        let (glyph, glyph_color) = if stalled {
            ("⊘", theme.gray)
        } else if answered {
            (RunStatus::Completed.glyph(), theme.accent_success)
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
            // An answered gate is done, so the ticks and the count below read the same states.
            let rail: Vec<NodeState> = ex
                .rail
                .iter()
                .map(|&ns| {
                    if answered && ns == NodeState::Waiting {
                        NodeState::Done(Outcome::Ok)
                    } else {
                        ns
                    }
                })
                .collect();
            for &ns in rail.iter().take(RAIL_MARKS_CAP) {
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
                let done = rail
                    .iter()
                    .filter(|s| matches!(s, NodeState::Done(_)))
                    .count();
                let total = rail.len();
                let cnt = format!(" {done}/{total}");
                rail_width += cnt.width();
                rail_spans.push(Span::styled(
                    cnt,
                    Style::default().fg(theme.gray).add_modifier(base_mod),
                ));
            }
        }

        // Pad the rail to the longest one shown, so the state text starts in one column.
        if show_rail {
            let count_w = |n: usize| if show_rail_count { format!(" {n}/{n}").width() } else { 0 };
            let widest = self
                .extras
                .values()
                .map(|e| {
                    if e.rail.is_empty() {
                        0
                    } else {
                        e.rail.len().min(RAIL_MARKS_CAP) + count_w(e.rail.len())
                    }
                })
                .max()
                .unwrap_or(0);
            if widest > rail_width {
                rail_spans.push(Span::raw(" ".repeat(widest - rail_width)));
                rail_width = widest;
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

        // Title and state text share what is left. Every row uses the same title width, so the columns line up.
        let longest_title = self.runs.iter().map(|r| r.title.width()).max().unwrap_or(0);
        let wrap_selected_waiting =
            selected && run.status == RunStatus::Waiting && !answered && !raw_state_text.is_empty();
        let (title_str, state_str) = if flex_budget <= 17 || raw_state_text.is_empty() && rail_width == 0 {
            let first = if wrap_selected_waiting {
                let (first, _, _) = self.flow_question_lines(run, w, now_ms);
                first
            } else {
                String::new()
            };
            (truncate_str(&run.title, flex_budget), first)
        } else {
            let half = (flex_budget - 1).div_euclid(2);
            let title_col = longest_title.clamp(16, half.max(16));
            let title = truncate_str(&run.title, title_col);
            let pad = title_col.saturating_sub(title.width());
            let state = if wrap_selected_waiting {
                let (first, _, _) = self.flow_question_lines(run, w, now_ms);
                first
            } else {
                truncate_str(&raw_state_text, flex_budget - 1 - title_col)
            };
            (format!("{title}{}", " ".repeat(pad)), state)
        };

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

    fn flow_question_lines(
        &self,
        run: &RunSummary,
        w: usize,
        now_ms: i64,
    ) -> (String, usize, Vec<String>) {
        let extra = self.extras.get(&run.run_id);
        let stalled = extra.map(|e| e.stalled).unwrap_or(false);
        let (raw_state_text, _) = self.flow_state_text(run, extra, stalled, &Theme::default());
        if raw_state_text.is_empty() || w == 0 {
            return (String::new(), 0, Vec::new());
        }

        let show_rail = w >= 48;
        let show_age = w >= 64;
        let show_rail_count = w >= 80;
        let show_duration = w >= 96;
        let show_agents = w >= 120;

        let rail_width = if show_rail {
            let count_w = |n: usize| {
                if show_rail_count {
                    format!(" {n}/{n}").width()
                } else {
                    0
                }
            };
            self.extras
                .values()
                .map(|e| {
                    if e.rail.is_empty() {
                        0
                    } else {
                        e.rail.len().min(RAIL_MARKS_CAP) + count_w(e.rail.len())
                    }
                })
                .max()
                .unwrap_or(0)
        } else {
            0
        };

        let agents_w = if show_agents {
            extra
                .map(|e| e.agents.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| 1 + truncate_str(s, 26).width())
                .unwrap_or(0)
        } else {
            0
        };
        let age_w = if show_age {
            1 + fmt_age(now_ms.saturating_sub(run.updated_ms)).width()
        } else {
            0
        };
        let dur_w = if show_duration {
            let end_ms = if run.status.is_live() && !stalled {
                now_ms
            } else {
                run.finished_ms.unwrap_or(run.updated_ms)
            };
            let dur_ms = (end_ms - run.started_ms).max(0) as u64;
            1 + fmt_duration(dur_ms).width()
        } else {
            0
        };

        let prefix_w = run.status.glyph().width() + 1;
        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
        let fixed_w = prefix_w + rail_col_w + agents_w + age_w + dur_w;
        let flex_budget = w.saturating_sub(fixed_w);
        let longest_title = self.runs.iter().map(|r| r.title.width()).max().unwrap_or(0);

        if flex_budget <= 17 {
            let indent = prefix_w;
            let wrap_w = w.saturating_sub(indent).max(1);
            let mut lines = vec![String::new()];
            lines.extend(wrap_words(&raw_state_text, wrap_w));
            (String::new(), indent, lines)
        } else {
            let half = (flex_budget - 1).div_euclid(2);
            let title_col = longest_title.clamp(16, half.max(16));
            let indent = prefix_w + title_col + rail_col_w + 1;
            let state_w = (flex_budget - 1 - title_col).max(1);
            let lines = wrap_words(&raw_state_text, state_w);
            let first = lines.first().cloned().unwrap_or_default();
            (first, indent, lines)
        }
    }

    fn flow_state_text(
        &self,
        run: &RunSummary,
        extra: Option<&RowExtra>,
        stalled: bool,
        theme: &Theme,
    ) -> (String, Color) {
        if run.status == RunStatus::Waiting {
            if let Some(choice) = self.answered.get(&run.run_id) {
                let text = if choice.is_empty() {
                    "answered".to_string()
                } else {
                    format!("answered: {choice}")
                };
                return (text, theme.gray);
            }
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
                "No matching agent turns"
            } else {
                "No agent turns found"
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

fn wrap_words(text: &str, max_w: usize) -> Vec<String> {
    let max_w = max_w.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for word in text.split_whitespace() {
        let ww = word.width();
        if cur.is_empty() {
            if ww <= max_w {
                cur.push_str(word);
                cur_w = ww;
            } else {
                for ch in word.chars() {
                    let cw = ch.to_string().width();
                    if cur_w + cw > max_w && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
            }
        } else if cur_w + 1 + ww <= max_w {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.clear();
            cur_w = 0;
            if ww <= max_w {
                cur.push_str(word);
                cur_w = ww;
            } else {
                for ch in word.chars() {
                    let cw = ch.to_string().width();
                    if cur_w + cw > max_w && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
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
    fn probe_title_state_age_render_at_live_width() {
        let mut state = LedgerState::new();
        let mut wait = make_run("r-wait", "ship: slugify", RunStatus::Waiting, 10_000);
        wait.waiting_on = Some("accept".to_string());
        let done = make_run("r-done", "mixed-talk", RunStatus::Completed, 9_000);
        state.set_runs(vec![wait.clone(), done.clone()]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Ok),
                    NodeState::NotRun,
                    NodeState::Done(Outcome::Ok),
                    NodeState::NotRun,
                    NodeState::Waiting,
                ],
                agents: String::new(),
                stalled: false,
                question: Some("tests and review are in: land it, or send it back?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-done".to_string(),
            RowExtra {
                rail: vec![NodeState::Done(Outcome::Ok); 6],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: None,
            },
        );
        // A workflow with hundreds of nodes (the monitor run: 262) must not
        // squeeze the title and state out of every row.
        let mut huge = make_run("r-huge", "monitor", RunStatus::Running, 8_000);
        huge.current_node = Some("step-200".to_string());
        state.set_runs(vec![wait.clone(), done.clone(), huge]);
        let mut huge_rail = Vec::with_capacity(262);
        for i in 0..262 {
            huge_rail.push(if i < 5 { NodeState::Done(Outcome::Ok) } else { NodeState::NotRun });
        }
        extras.insert(
            "r-huge".to_string(),
            RowExtra {
                rail: huge_rail,
                agents: String::new(),
                stalled: false,
                question: None,
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        for w in [77u16, 114] {
            let mut buf = Buffer::empty(Rect::new(0, 0, w, 12));
            let area_ = buf.area;
            state.render(&mut buf, area_, &theme, 20_000);
            let text = all_text(&buf);
            assert!(
                text.contains("ship: slugify"),
                "title missing at w={w}: {text:?}"
            );
            assert!(
                text.contains("mixed-talk"),
                "finished title missing at w={w}: {text:?}"
            );
            assert!(text.contains("done"), "state missing at w={w}: {text:?}");
            assert!(
                text.contains("monitor"),
                "huge-rail run title missing at w={w}: {text:?}"
            );
            if w >= 80 {
                assert!(
                    text.contains("5/262"),
                    "huge-rail count must read the whole run at w={w}: {text:?}"
                );
            }
            let marks: String = text.chars().filter(|c| *c == '✓').collect();
            assert!(
                marks.len() < 60,
                "a row must not draw hundreds of marks at w={w}"
            );
        }
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
                answers: None,
            },
        );
        state.set_extras(extras);

        // Stalled run is not considered live
        assert!(!state.any_live());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);
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
        // Clearing the filter keeps the selected run selected.
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

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
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);

        let r0 = row_text(&buf, 0);
        assert!(r0.contains("Flows 3"));
        assert!(r0.contains("Agent Turns 0"));
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
    fn answered_gate_row_count_includes_the_answered_gate_and_matches_the_rail() {
        let rail = vec![
            NodeState::Done(Outcome::Ok),
            NodeState::Done(Outcome::Ok),
            NodeState::Done(Outcome::Ok),
            NodeState::Waiting,
            NodeState::NotRun,
            NodeState::NotRun,
        ];
        let render = |answered: bool| {
            let mut state = LedgerState::new();
            state.set_runs(vec![
                make_run("r-wait", "Ship flow", RunStatus::Waiting, 9_000),
                make_run("r-next", "Fix flow", RunStatus::Completed, 8_000),
            ]);
            let mut extras = HashMap::new();
            extras.insert(
                "r-wait".to_string(),
                RowExtra {
                    rail: rail.clone(),
                    agents: String::new(),
                    stalled: false,
                    question: Some("Ship it?".to_string()),
                    answers: None,
                },
            );
            if answered {
                extras.insert(
                    "r-next".to_string(),
                    RowExtra {
                        rail: vec![],
                        agents: String::new(),
                        stalled: false,
                        question: None,
                        answers: Some(GateAnswer {
                            run_id: "r-wait".to_string(),
                            choice: "Accept".to_string(),
                        }),
                    },
                );
            }
            state.set_extras(extras);
            let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
            let area_ = buf.area;
            state.render(&mut buf, area_, &Theme::default(), 10_000);
            all_text(&buf)
        };

        let answered = render(true);
        assert!(answered.contains("✓✓✓✓○○ 4/6"), "answered row: {answered}");
        assert!(!answered.contains("3/6"));

        let waiting = render(false);
        assert!(waiting.contains("✓✓✓◐○○ 3/6"), "waiting row: {waiting}");
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
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 8));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);

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
        let area_ = zero_buf.area;
        state.render(&mut zero_buf, area_, &theme, 10_000);

        let mut empty_buf = Buffer::empty(Rect::new(0, 0, 80, 10));
        let area_ = empty_buf.area;
        state.render(&mut empty_buf, area_, &theme, 10_000);
        let full = all_text(&empty_buf);
        assert!(full.contains("No acpx flow runs found"));
    }

    #[test]
    fn answered_gate_counts_only_unanswered_and_moves_answered_to_finished() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-old-wait", "Old Gate", RunStatus::Waiting, 1_000),
            make_run("r-new-wait", "New Gate", RunStatus::Waiting, 2_000),
            make_run("r-next", "Next Flow", RunStatus::Completed, 3_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-old-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Continue old?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-new-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Continue new?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-next".to_string(),
            RowExtra {
                rail: vec![NodeState::Done(Outcome::Ok)],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-old-wait".to_string(),
                    choice: "Approve".to_string(),
                }),
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 14));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let full = all_text(&buf);
        assert!(full.contains("1 needs you"));
        assert!(full.contains("NEEDS YOU 1"));
        assert!(full.contains("FINISHED 2"));
        let fin_pos = full.find("FINISHED 2").unwrap_or(0);
        let old_pos = full.find("Old Gate").unwrap_or(0);
        assert!(old_pos > fin_pos);
    }

    #[test]
    fn unanswered_waiting_run_remains_in_needs_you_and_reverts_when_answer_removed() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait", "Gate Run", RunStatus::Waiting, 1_000),
            make_run("r-ans", "Answering Run", RunStatus::Failed, 2_000),
        ]);
        let mut extras_unanswered = HashMap::new();
        extras_unanswered.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Proceed?".to_string()),
                answers: None,
            },
        );
        state.set_extras(extras_unanswered.clone());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 12));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let full = all_text(&buf);
        assert!(full.contains("1 needs you"));
        assert!(full.contains("NEEDS YOU 1"));

        let mut extras_answered = extras_unanswered.clone();
        extras_answered.insert(
            "r-ans".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-wait".to_string(),
                    choice: "Yes".to_string(),
                }),
            },
        );
        state.set_extras(extras_answered);
        let mut buf2 = Buffer::empty(Rect::new(0, 0, 100, 12));
        state.render(&mut buf2, area, &theme, 10_000);
        assert!(!all_text(&buf2).contains("needs you"));

        // Replace extras without the answering run: counts as needing you again
        state.set_extras(extras_unanswered);
        let mut buf3 = Buffer::empty(Rect::new(0, 0, 100, 12));
        state.render(&mut buf3, area, &theme, 10_000);
        let full3 = all_text(&buf3);
        assert!(full3.contains("1 needs you"));
        assert!(full3.contains("NEEDS YOU 1"));
    }

    #[test]
    fn set_extras_before_set_runs_matches_reverse_and_latest_started_wins() {
        let runs = vec![
            make_run("r-wait", "Waiting Gate", RunStatus::Waiting, 1_000),
            make_run("r-ans-1", "Answer 1", RunStatus::Completed, 2_000),
            make_run("r-ans-2", "Answer 2", RunStatus::Completed, 5_000),
        ];
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Which?".to_string()),
                answers: None,
            },
        );
        extras.insert(
            "r-ans-1".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-wait".to_string(),
                    choice: "OldChoice".to_string(),
                }),
            },
        );
        extras.insert(
            "r-ans-2".to_string(),
            RowExtra {
                rail: vec![],
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-wait".to_string(),
                    choice: "NewChoice".to_string(),
                }),
            },
        );

        let mut s1 = LedgerState::new();
        s1.set_extras(extras.clone());
        s1.set_runs(runs.clone());

        let mut s2 = LedgerState::new();
        s2.set_runs(runs);
        s2.set_extras(extras);

        assert_eq!(s1.answered_choice("r-wait"), Some("NewChoice"));
        assert_eq!(s2.answered_choice("r-wait"), Some("NewChoice"));
        assert_eq!(s1.answered_choice("r-ans-1"), None);
        assert_eq!(s1.answered_choice("nonexistent"), None);

        let theme = Theme::default();
        let mut b1 = Buffer::empty(Rect::new(0, 0, 100, 12));
        let mut b2 = Buffer::empty(Rect::new(0, 0, 100, 12));
        let area = b1.area;
        s1.render(&mut b1, area, &theme, 10_000);
        s2.render(&mut b2, area, &theme, 10_000);
        let t1 = all_text(&b1);
        assert_eq!(t1, all_text(&b2));
        assert!(t1.contains("answered: NewChoice"));
        assert!(!t1.contains("Which?"));
    }

    #[test]
    fn selected_unanswered_waiting_run_wraps_200_char_question_and_unselected_stays_one_row() {
        let words: Vec<String> = (1..=28).map(|i| format!("word{i:02}")).collect();
        let question = format!("{} tailend", words.join(" "));
        assert_eq!(question.len(), 203);

        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-wait-1", "First Gate", RunStatus::Waiting, 2_000),
            make_run("r-wait-2", "Second Gate", RunStatus::Waiting, 1_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait-1".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some(question.clone()),
                answers: None,
            },
        );
        extras.insert(
            "r-wait-2".to_string(),
            RowExtra {
                rail: vec![NodeState::Waiting],
                agents: String::new(),
                stalled: false,
                question: Some("Short?".to_string()),
                answers: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let text = all_text(&buf);
        for w in question.split_whitespace() {
            assert!(text.contains(w), "missing word {w} in:\n{text}");
        }

        // Move selection to r-wait-2 so r-wait-1 is unselected and takes a single row (row 2)
        state.handle_key(&key(KeyCode::Down));
        let mut buf2 = Buffer::empty(Rect::new(0, 0, 100, 20));
        state.render(&mut buf2, area, &theme, 10_000);
        assert!(row_text(&buf2, 2).contains("First Gate"));
        assert!(row_text(&buf2, 3).contains("Second Gate"));
    }
}
