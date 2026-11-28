Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, ledger routing, or release behavior.

=== REPOSITORY CONTEXT (16 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 1-315 of 2990 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 317-528 of 2990 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 550-940 of 2990 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 943-1036 of 2990 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 1120-1696 of 2990 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2124-2278 of 2990 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2279-2421 of 2990 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2499-2710 of 2990 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2756-2813 of 2990 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1-103 of 1621 (repository grok-build)
````
pub(crate) mod detail;
pub(crate) mod inspector;
pub(crate) mod ledger;
pub(crate) mod modal_view;
pub(crate) mod run;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{self, Run, RunSummary};
use crate::acpx::sessions::{self, SessionSummary};
use crate::piw::gate::{self as piw_gate, GateAnswer as PiwGateAnswer, GateAnswerer};
use crate::piw::mirror;
use crate::piw::runtime;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{
    self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowOutcome, ModalWindowState,
    Shortcut,
};

use self::detail::{DetailPanel, DetailTarget};
use self::inspector::{InspectorOutcome, InspectorState};
use self::ledger::{GateAnswer, LedgerOutcome, LedgerState, RowExtra};
use self::run::{RunOutcome, RunViewState};

const MAX_TITLE_WIDTH: usize = 72;
/// Below this content width the runs list has no room for a preview beside it.
const PREVIEW_MIN_WIDTH: u16 = 120;
/// Rails and agents are loaded for this many of the newest runs.
const EXTRAS_LIMIT: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ledger,
    Run,
    Step,
}

/// A conversation to replay as a transcript; the wiring feeds it to `crate::acp::acpx_feed::convert`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayRequest {
    pub events_path: PathBuf,
    pub range: Option<(u64, u64)>,
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AcpxOutcome {
    Unchanged,
    Changed,
    Close,
    Copy(String),
    Replay(ReplayRequest),
    /// A gate was answered: start this flow with this input.
    RunFlow { flow_file: PathBuf, input_json: String },
}

struct ModalNotice {
    text: String,
    warning: bool,
}

impl ModalNotice {
    fn warning(text: String) -> Self {
        Self { text, warning: true }
    }
}

pub struct AcpxModalState {
    pub window: ModalWindowState,
    runs_base: Option<PathBuf>,
    sessions_base: Option<PathBuf>,
    runs_snapshot: Vec<RunSummary>,
    ledger: LedgerState,
    run_view: Option<RunViewState>,
    inspector: Option<(usize, InspectorState)>,
    mode: Mode,
    help_open: bool,
    notice: Option<ModalNotice>,
    /// The right-hand panel: the selected run beside the list, the selected stage beside a run.
    detail: DetailPanel,
    /// The run selected in the list, loaded for the preview.
    preview: Option<Run>,
    /// Rail and agents per run id, with the `updated_ms` they were read at.
    extras: HashMap<String, (i64, RowExtra)>,
    /// Gate answers per run id for runs beyond `EXTRAS_LIMIT`.
    answers: HashMap<String, Option<GateAnswer>>,
    /// Answers being sent to pi-workflows gates, polled on the one-second tick.
    answerers: Vec<GateAnswerer>,
    /// The mirrored run whose folder the run view is open on, so closing the
    /// view can release it.
    open_mirror_run: Option<String>,
    /// Where the detail panel was last drawn, for the mouse wheel.
    detail_rect: Option<Rect>,
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 311-368 of 1621 (repository grok-build)
````
    fn rescan_runs(&mut self) -> bool {
        let runs = runtime::list_runs_with_mirror(self.runs_base.as_deref());
        let changed = runs != self.runs_snapshot;
        self.runs_snapshot = runs.clone();
        self.load_extras(&runs);
        self.ledger.set_runs(runs);
        if let Some(rv) = self.run_view.as_mut() {
            let run_id = rv.run().summary.run_id.clone();
            rv.set_answered(self.ledger.answered_choice(&run_id).map(str::to_string));
        }
        changed
    }

    /// Rail, agents and question for the newest runs. A run is re-read only when acpx has written it since.
    fn load_extras(&mut self, runs: &[RunSummary]) {
        let now = now_ms();
        let mut kept = HashMap::new();
        for summary in runs.iter().take(EXTRAS_LIMIT) {
            let cached = self.extras.remove(&summary.run_id).filter(|(at, _)| *at == summary.updated_ms);
            let (at, mut extra) = cached.unwrap_or_else(|| (summary.updated_ms, row_extra(summary)));
            extra.stalled = bundle::is_stalled(summary, now);
            kept.insert(summary.run_id.clone(), (at, extra));
        }
        let mut all_extras: HashMap<String, RowExtra> = kept
            .iter()
            .map(|(id, (_, extra))| (id.clone(), extra.clone()))
            .collect();
        for summary in runs.iter().skip(EXTRAS_LIMIT) {
            let answers = self
                .answers
                .entry(summary.run_id.clone())
                .or_insert_with(|| read_run_answers(&summary.dir))
                .clone();
            all_extras.insert(
                summary.run_id.clone(),
                RowExtra { answers, ..RowExtra::default() },
            );
        }
        self.ledger.set_extras(all_extras);
        self.extras = kept;
    }

    /// Keep `preview` on the run selected in the list.
    fn sync_preview(&mut self) {
        let Some(selected) = self.ledger.selected_run() else {
            self.preview = None;
            return;
        };
        let fresh = self
            .preview
            .as_ref()
            .is_some_and(|p| p.summary.dir == selected.dir && p.summary.updated_ms == selected.updated_ms);
        if !fresh {
            self.preview = bundle::load_run(&selected.dir).ok();
        }
    }

    fn rescan_sessions(&mut self) {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 695-741 of 1621 (repository grok-build)
````
fn row_extra(summary: &RunSummary) -> RowExtra {
    let Ok(run) = bundle::load_run(&summary.dir) else { return RowExtra::default() };
    let steps = bundle::step_settings(&summary.dir);
    let nodes = bundle::node_settings(&summary.dir);
    // The agent that did the most recent agent step, else the first one the flow declares.
    let agents = run
        .steps
        .iter()
        .rev()
        .filter_map(|step| steps.get(&step.attempt_id))
        .chain(run.flow.nodes.iter().filter_map(|node| nodes.get(&node.id)))
        .map(bundle::AgentSettings::label)
        .find(|label| !label.is_empty())
        .unwrap_or_default();
    RowExtra {
        rail: run.flow.nodes.iter().map(|node| run.node_state(&node.id)).collect(),
        agents,
        stalled: false,
        question: (run.summary.status == bundle::RunStatus::Waiting).then(|| run.summary.status_detail.clone()).flatten(),
        answers: parse_gate_answer(&run.input),
    }
}

fn parse_gate_answer(input: &serde_json::Value) -> Option<GateAnswer> {
    let gate = input.get("gate")?.as_object()?;
    let run_id = gate.get("runId")?.as_str()?;
    if run_id.is_empty() {
        return None;
    }
    let choice = gate
        .get("choice")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(GateAnswer {
        run_id: run_id.to_string(),
        choice,
    })
}

fn read_run_answers(dir: &Path) -> Option<GateAnswer> {
    let text = std::fs::read_to_string(dir.join("projections").join("run.json")).ok()?;
    let val = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let input = val.get("input")?;
    parse_gate_answer(input)
}

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1432-1510 of 1621 (repository grok-build)
````
    fn row_extra_reads_gate_answer_and_handles_missing_or_malformed_gate() {
        let fx = write_fixture("completed");
        let run_json_path = fx.run_dir.join("projections").join("run.json");

        fs::write(
            &run_json_path,
            r#"{"input":{"gate":{"runId":"r1","node":"g","choice":"approve"}}}"#,
        )
        .expect("write run.json");
        let runs = bundle::list_runs(&fx.runs_base);
        let summary = runs.first().expect("one run");
        let extra = row_extra(summary);
        assert_eq!(
            extra.answers,
            Some(GateAnswer {
                run_id: "r1".into(),
                choice: "approve".into(),
            })
        );

        fs::write(&run_json_path, r#"{"input":{"other":1}}"#).expect("write run.json without gate");
        let extra = row_extra(summary);
        assert_eq!(extra.answers, None);

        fs::write(&run_json_path, r#"{"input":{"gate":"not-an-object"}}"#)
            .expect("write run.json with string gate");
        let extra = row_extra(summary);
        assert_eq!(extra.answers, None);
    }

    #[test]
    fn run_outside_extras_limit_gets_answers_with_empty_rail() {
        let fx = write_fixture("completed");
        fs::write(
            fx.run_dir.join("projections").join("run.json"),
            r#"{"input":{"gate":{"runId":"waiting-run","node":"g","choice":"ship"}}}"#,
        )
        .expect("write old run.json");

        for i in 0..EXTRAS_LIMIT {
            let dir = fx.runs_base.join(format!("newer-{i:03}"));
            let proj = dir.join("projections");
            fs::create_dir_all(&proj).expect("create newer proj");
            let live = format!(
                r#"{{
                    "runId": "newer-{i:03}",
                    "flowName": "deploy-flow",
                    "runTitle": "Newer {i}",
                    "status": "completed",
                    "startedAt": "2025-03-02T10:{:02}:{:02}Z",
                    "updatedAt": "2025-03-02T10:{:02}:{:02}Z"
                }}"#,
                i / 60,
                i % 60,
                i / 60,
                i % 60
            );
            fs::write(proj.join("live.json"), live).expect("write newer live.json");
            fs::write(
                dir.join("flow.json"),
                r#"{"name":"deploy-flow","startAt":"plan","nodes":{"plan":{"nodeType":"acp"}}}"#,
            )
            .expect("write newer flow.json");
        }

        let state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.extras.contains_key("run-001"));
        assert_eq!(
            state.ledger.answered_choice("waiting-run"),
            Some("ship")
        );
    }
}

#[cfg(test)]
mod gate_answer_target_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 1-50 of 1936 (repository grok-build)
````
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use chrono::DateTime;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus {
    Running,
    Waiting,
    Completed,
    Failed,
    TimedOut,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeKind {
    Acp,
    Action,
    Compute,
    Checkpoint,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    TimedOut,
    Failed,
    Cancelled,
    Unknown,
}

/// What a node looks like right now, derived from steps + live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    NotRun,
    Running,
    Waiting,
    Done(Outcome),
}

impl RunStatus {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "running" => Self::Running,
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 150-187 of 1936 (repository grok-build)
````
pub struct ActionReceipt {
    pub action_type: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}

/// One attempt of one node, from projections/steps.json, in execution order.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub attempt_id: String,
    pub node_id: String,
    pub kind: NodeKind,
    pub outcome: Outcome,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub prompt_text: Option<String>,
    pub raw_text: Option<String>,
    pub output: Value,
    pub error: Option<String>,
    pub agent_name: Option<String>,
    pub action: Option<ActionReceipt>,
    pub conversation: Option<Conversation>,
    pub stdout: Option<ArtifactRef>,
    pub stderr: Option<ArtifactRef>,
    pub output_artifact: Option<ArtifactRef>,
}

impl Step {
    pub fn duration_ms(&self) -> u64 {
        let diff = self.finished_ms.saturating_sub(self.started_ms);
        if diff <= 0 { 0 } else { diff as u64 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 261-350 of 1936 (repository grok-build)
````
pub struct SessionRef {
    pub id: String,
    pub handle: String,
    pub events_path: PathBuf,
    pub agent_name: Option<String>,
}

/// One row of the runs ledger; cheap to load (projections/live.json, falling back to manifest.json).
#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub dir: PathBuf,
    pub flow_name: String,
    pub title: String,
    pub status: RunStatus,
    pub started_ms: i64,
    pub updated_ms: i64,
    pub finished_ms: Option<i64>,
    pub current_node: Option<String>,
    pub waiting_on: Option<String>,
    pub status_detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub summary: RunSummary,
    pub flow: FlowDef,
    pub steps: Vec<Step>,
    pub events: Vec<TraceEvent>,
    pub input: Value,
    pub sessions: Vec<SessionRef>,
}

impl Run {
    pub fn node_state(&self, node_id: &str) -> NodeState {
        if self.summary.waiting_on.as_deref() == Some(node_id) {
            return NodeState::Waiting;
        }
        if self.summary.status == RunStatus::Running
            && self.summary.current_node.as_deref() == Some(node_id)
        {
            return NodeState::Running;
        }
        if let Some(step) = self.steps.iter().rev().find(|s| s.node_id == node_id) {
            return NodeState::Done(step.outcome);
        }
        NodeState::NotRun
    }

    /// `node_state`, except that a gate which has been answered is done: the
    /// checkpoint is no longer waiting on anyone.
    pub fn node_state_answered(&self, node_id: &str, answered: bool) -> NodeState {
        match self.node_state(node_id) {
            NodeState::Waiting if answered => NodeState::Done(Outcome::Ok),
            state => state,
        }
    }

    pub fn taken_path(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.node_id.clone()).collect()
    }

    pub fn edge_taken(&self, from: &str, to: &str) -> bool {
        self.steps
            .windows(2)
            .any(|pair| pair[0].node_id == from && pair[1].node_id == to)
    }

    pub fn step_events(&self, attempt_id: &str) -> Vec<&TraceEvent> {
        self.events
            .iter()
            .filter(|e| e.attempt_id.as_deref() == Some(attempt_id))
            .collect()
    }

    pub fn progress(&self) -> (usize, usize) {
        let distinct: HashSet<&str> = self.steps.iter().map(|s| s.node_id.as_str()).collect();
        (distinct.len(), self.flow.nodes.len())
    }

    pub fn duration_ms(&self, now_ms: i64) -> u64 {
        let end_ms = if let Some(fin) = self.summary.finished_ms {
            fin
        } else if self.summary.status == RunStatus::Running {
            now_ms.max(self.summary.updated_ms)
        } else {
            self.summary.updated_ms
        };
        let diff = end_ms.saturating_sub(self.summary.started_ms);
````

=== TASK SPECIFICATION ===

Slice ID: finished-run-ledger

Own Flows row grouping, selection and progress in crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs and its
unit tests. In crates/codegen/xai-grok-pager/src/views/acpx_modal.rs edit only the row_extra producer/cache and its tests
if needed for executed counts. Do not change preview or modal route. Keep
LedgerState::selected_run() and LedgerOutcome routing compatible.

Group two or more actual finished runs with the same nonempty flow_name.
The newest member represents a collapsed group: the greatest started_ms, and on
a tie the greatest run_id (run IDs begin with a sortable timestamp). Keep
active/waiting runs individually selectable. Do not group any run still marked
Running or Waiting by its actual summary, even if the legacy section label puts
it in Finished (a stalled Running run, an answered Waiting run). Preserve legacy
gate classification for now; call no answer operation. Leave empty workflow
names as individual runs. Keep total counts in units of runs. Preserve sort and
active filters.

Space toggles the selected group open or closed in Flows; it is unbound in the
attached Flows dispatcher. Tab, BackTab, Left, Right, h and l keep switching
tabs. Add the Space label to LedgerState::shortcuts(); the coordinator adds the
help-table row in crates/codegen/xai-grok-pager/src/views/acpx_modal.rs. Space on a row that is not a group is Unchanged.
Enter still opens the selected run. Expanding reveals older member runs in
newest-first order by the same rule. When search matches an older ID, keep it
selectable even if previously folded. Refresh keeps selection by run ID, even
if a newer member arrives. On explicit collapse of a selected child, move
selection to the visible representative. Leave the Agent Turns tab
(LedgerTab::Sessions; its label now reads Agent Turns) behavior alone.

Each Flows row shows three things that answer different questions: the
existing dot rail (the run's shape: one mark per flow node, unrun nodes
included), '<N> steps ran' (how much happened), and the executed path (what
happened, in order), e.g. `✓ cargo_check → ✗ repair_compile → ✓ cargo_check`.
The count and the path come only from recorded Step entries in run.steps
order, repeat attempts included, each glyph from that step's outcome; never
from flow nodes. The ' done/total' suffix after the rail is replaced by the
count. RowExtra contract: add `pub executed: Option<Vec<(String, Outcome)>>`
(node_id and outcome per recorded step, in order) and keep `rail` (literals
outside the excerpts still set it). None means unknown and renders
'steps unknown', never 0; Some(empty) renders 'no steps ran'. row_extra in
crates/codegen/xai-grok-pager/src/views/acpx_modal.rs fills executed from the run it already loads. At full width show
rail, count and path together, truncating the path with … to the space left.
At narrower widths keep the row readable: drop the path from the row first,
then follow the existing width thresholds, and give the selected run its full
executed path on wrapped lines below the row, the way flow_question_lines adds
a waiting run's question (keep row-height and scroll math consistent for both).
Update every RowExtra literal inside the attached excerpts, and update the
attached probe test's rail assertions (the 5/262 count and the check marks) to
the new text. The coordinator adds `executed: None` to the RowExtra literals
outside the excerpts. Reuse the parent's cached bundle read. Do not
add I/O to render or widen the cache/read limit. Keep existing styles. For a
concrete Flows selection, y copies the canonical '/open <run_id>' command;
on the Agent Turns tab y still copies the selected record ID. A confirmed copy
already shows "Copied!" (fixed on main). Chat /open <id> already resolves
mirrored PIW runs read-only (resolve_saved_run); the coordinator still checks a
live paste of the copied command before shipping it.

Tests: three finished runs of one name plus a same-name pending run; folded
and expanded navigation with Space; newest tie by run_id; matching and
preserving an older ID on refresh; a stalled Running run and an answered
Waiting run of the grouped name staying individual; a group under the Recent
and Name orders; count and path including a retry (cargo_check, repair_compile, cargo_check in
order) vs missing extras ('steps unknown') vs no steps; the rail still showing
unrun nodes while count and path exclude them; full-width row with all three;
a narrow selected row showing its full path below it and a narrow unselected
row without it; exact y
behavior on both tabs. Stay out of unrelated existing gate-status tests.

Edit list format. The file is Markdown. List the edits in application order;
each edit has exactly this shape:

## Edit <N>: <short purpose>
Path: <repository-relative path>
OLD:
~~~rust
<existing lines copied verbatim from one attached excerpt>
~~~
NEW:
~~~rust
<complete replacement>
~~~

Rules:
- Copy OLD character for character from an attached excerpt, including
  indentation, blank lines and comments. Never retype, reformat, abbreviate or
  elide it. OLD must occur exactly once in the file: include enough lines to be
  unique, normally the whole function, impl item, struct or test.
- Every OLD quotes the original attached source, never the result of an
  earlier edit. No two OLD blocks overlap.
- To insert new code (a field, function or test), quote a nearby existing
  anchor as OLD and give the anchor plus the new code as NEW.
- NEW is complete code with no "...", "unchanged" or elided bodies.
- Prefer replacing whole functions and whole test functions to small fragments.
- Inside the file use only ~~~ fences, never triple backticks.
- Never edit code outside the attached excerpts. If the change needs that, add
  a final "## Coordinator follow-ups" section naming each location and reason.

The coordinator matches each OLD exactly once on the pinned base and stops on
zero or several matches, then applies the edits and runs format, compile and
the targeted tests.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-returns/finished-run-ledger.edits.md ===
```markdown
<complete Markdown edit list>
```
