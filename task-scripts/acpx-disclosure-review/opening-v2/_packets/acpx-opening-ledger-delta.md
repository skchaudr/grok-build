Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, or release behavior. Change ledger routing only
where the task explicitly owns a reviewed presentation or focus change.

=== REPOSITORY CONTEXT (18 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 1-445 of 3849 (repository grok-build)
````
use std::collections::{HashMap, HashSet};
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
    pub executed: Option<Vec<(String, Outcome)>>,
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
struct FlowGroupEntry {
    /// Representative run index in `runs` (newest member for a group).
    rep_idx: usize,
    /// Member indices in `runs` ordered newest-first (`members[0] == rep_idx`).
    members: Vec<usize>,
    /// Non-empty `flow_name` when `members.len() >= 2`, else `None`.
    group_key: Option<String>,
}

#[derive(Debug, Clone)]
struct VisibleFlowItem {
    run_idx: usize,
    /// When this row represents a group of 2+ finished runs: `(member_count, expanded)`.
    group_rep: Option<(usize, bool)>,
    /// True when this row is an older member inside an expanded group.
    is_child: bool,
    /// When this row belongs to a group (rep or child): `(flow_name, rep_run_id)`.
    group_ref: Option<(String, String)>,
}

#[derive(Debug, Clone)]
enum FlowListRow {
    Header { kind: SectionKind, count: usize },
    Blank,
    Run {
        idx: usize,
        group_rep: Option<(usize, bool)>,
        is_child: bool,
    },
    Continuation {
        indent: usize,
        text: String,
        is_warning: bool,
    },
}

impl FlowListRow {
    fn is_selectable(&self) -> bool {
        matches!(self, Self::Run { .. })
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
    flow_expanded_groups: HashSet<String>,

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
            flow_expanded_groups: HashSet::new(),

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
        } else if self.tab == LedgerTab::Flows {
            vec![
                "Enter open",
                "Space group",
                "Tab switch",
                "/ filter",
                "a active",
                "s sort",
                "y copy",
                "R rescan",
                "Esc close",
            ]
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

    fn is_groupable_finished(run: &RunSummary) -> bool {
        !run.flow_name.is_empty()
            && matches!(
                run.status,
                RunStatus::Completed | RunStatus::Failed | RunStatus::TimedOut
            )
    }

    fn cmp_group_newest_first(a: &RunSummary, b: &RunSummary) -> std::cmp::Ordering {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 446-747 of 3849 (repository grok-build)
````
    fn build_flow_entries(&self) -> Vec<FlowGroupEntry> {
        let mut grouped: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut entries: Vec<FlowGroupEntry> = Vec::new();

        for (i, run) in self.runs.iter().enumerate() {
            if !self.matches_flow_filter(run) {
                continue;
            }
            if Self::is_groupable_finished(run) {
                grouped.entry(run.flow_name.as_str()).or_default().push(i);
            } else {
                entries.push(FlowGroupEntry {
                    rep_idx: i,
                    members: vec![i],
                    group_key: None,
                });
            }
        }

        for (flow_name, mut members) in grouped {
            if members.len() >= 2 {
                members.sort_by(|&a, &b| Self::cmp_group_newest_first(&self.runs[a], &self.runs[b]));
                let rep_idx = members[0];
                entries.push(FlowGroupEntry {
                    rep_idx,
                    members,
                    group_key: Some(flow_name.to_string()),
                });
            } else if let Some(&idx) = members.first() {
                entries.push(FlowGroupEntry {
                    rep_idx: idx,
                    members: vec![idx],
                    group_key: None,
                });
            }
        }

        match self.flow_order {
            FlowOrder::Grouped => {
                entries.sort_by(|a, b| {
                    let ra = &self.runs[a.rep_idx];
                    let rb = &self.runs[b.rep_idx];
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
                entries.sort_by(|a, b| {
                    let ra = &self.runs[a.rep_idx];
                    let rb = &self.runs[b.rep_idx];
                    rb.updated_ms
                        .cmp(&ra.updated_ms)
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Name => {
                entries.sort_by(|a, b| {
                    let ra = &self.runs[a.rep_idx];
                    let rb = &self.runs[b.rep_idx];
                    ra.title
                        .to_lowercase()
                        .cmp(&rb.title.to_lowercase())
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
        }
        entries
    }

    fn expand_entry(&self, entry: &FlowGroupEntry) -> Vec<VisibleFlowItem> {
        if let Some(ref key) = entry.group_key {
            let expanded = self.flow_expanded_groups.contains(key);
            let rep_id = self
                .runs
                .get(entry.rep_idx)
                .map(|r| r.run_id.clone())
                .unwrap_or_default();
            let mut items = vec![VisibleFlowItem {
                run_idx: entry.rep_idx,
                group_rep: Some((entry.members.len(), expanded)),
                is_child: false,
                group_ref: Some((key.clone(), rep_id.clone())),
            }];
            if expanded {
                for &child_idx in &entry.members[1..] {
                    items.push(VisibleFlowItem {
                        run_idx: child_idx,
                        group_rep: None,
                        is_child: true,
                        group_ref: Some((key.clone(), rep_id.clone())),
                    });
                }
            }
            items
        } else {
            vec![VisibleFlowItem {
                run_idx: entry.rep_idx,
                group_rep: None,
                is_child: false,
                group_ref: None,
            }]
        }
    }

    fn visible_flow_items(&self) -> Vec<VisibleFlowItem> {
        self.build_flow_entries()
            .iter()
            .flat_map(|entry| self.expand_entry(entry))
            .collect()
    }

    fn has_any_flow_group(&self) -> bool {
        self.build_flow_entries()
            .iter()
            .any(|entry| entry.group_key.is_some())
    }

    fn toggle_selected_flow_group(&mut self) -> bool {
        if self.tab != LedgerTab::Flows {
            return false;
        }
        let Some(sel_id) = self.flow_selected_run_id.as_deref() else {
            return false;
        };
        let items = self.visible_flow_items();
        let Some(item) = items
            .into_iter()
            .find(|it| self.runs.get(it.run_idx).map(|r| r.run_id.as_str()) == Some(sel_id))
        else {
            return false;
        };
        let Some((key, rep_id)) = item.group_ref else {
            return false;
        };
        if self.flow_expanded_groups.contains(&key) {
            self.flow_expanded_groups.remove(&key);
            if item.is_child {
                self.flow_selected_run_id = Some(rep_id);
            }
        } else {
            self.flow_expanded_groups.insert(key);
        }
        true
    }

    /// Returns visible selectable indices into `self.runs` in display order.
    fn filtered_flow_indices(&self) -> Vec<usize> {
        self.visible_flow_items()
            .into_iter()
            .map(|item| item.run_idx)
            .collect()
    }

    /// Build visual rows for the Flows tab. `list_height` is used to decide whether blank rows
    /// separate sections when the list is shorter than the area.
    fn build_flow_rows(&self, list_height: usize, now_ms: i64) -> Vec<FlowListRow> {
        let entries = self.build_flow_entries();
        if entries.is_empty() {
            return Vec::new();
        }
        if self.flow_order != FlowOrder::Grouped {
            let mut out = Vec::new();
            for entry in &entries {
                for item in self.expand_entry(entry) {
                    out.push(FlowListRow::Run {
                        idx: item.run_idx,
                        group_rep: item.group_rep,
                        is_child: item.is_child,
                    });
                    self.append_continuations(&mut out, item.run_idx, item.is_child, now_ms);
                }
            }
            return out;
        }

        let mut needs_you = Vec::new();
        let mut running = Vec::new();
        let mut finished = Vec::new();
        for entry in entries {
            if let Some(r) = self.runs.get(entry.rep_idx) {
                match self.run_section(r) {
                    SectionKind::NeedsYou => needs_you.push(entry),
                    SectionKind::Running => running.push(entry),
                    SectionKind::Finished => finished.push(entry),
                }
            }
        }

        let sections: Vec<(SectionKind, usize, Vec<VisibleFlowItem>)> = [
            (SectionKind::NeedsYou, needs_you),
            (SectionKind::Running, running),
            (SectionKind::Finished, finished),
        ]
        .into_iter()
        .filter(|(_, sec_entries)| !sec_entries.is_empty())
        .map(|(kind, sec_entries)| {
            let run_count: usize = sec_entries.iter().map(|e| e.members.len()).sum();
            let visible: Vec<VisibleFlowItem> = sec_entries
                .iter()
                .flat_map(|e| self.expand_entry(e))
                .collect();
            (kind, run_count, visible)
        })
        .collect();

        let extra_rows: usize = sections
            .iter()
            .flat_map(|(_, _, items)| items.iter())
            .map(|item| self.continuation_lines_for(item.run_idx, item.is_child, now_ms).len())
            .sum();
        let non_blank_rows: usize = sections
            .iter()
            .map(|(_, _, items)| 1 + items.len())
            .sum::<usize>()
            + extra_rows;
        let blank_count = sections.len().saturating_sub(1);
        let insert_blanks = non_blank_rows + blank_count <= list_height;

        let mut out = Vec::with_capacity(non_blank_rows + blank_count);
        for (i, (kind, run_count, items)) in sections.into_iter().enumerate() {
            if i > 0 && insert_blanks {
                out.push(FlowListRow::Blank);
            }
            out.push(FlowListRow::Header {
                kind,
                count: run_count,
            });
            for item in items {
                out.push(FlowListRow::Run {
                    idx: item.run_idx,
                    group_rep: item.group_rep,
                    is_child: item.is_child,
                });
                self.append_continuations(&mut out, item.run_idx, item.is_child, now_ms);
            }
        }
        out
    }

    fn append_continuations(
        &self,
        out: &mut Vec<FlowListRow>,
        idx: usize,
        is_child: bool,
        now_ms: i64,
    ) {
        for (indent, text, is_warning) in self.continuation_lines_for(idx, is_child, now_ms) {
            out.push(FlowListRow::Continuation {
                indent,
                text,
                is_warning,
            });
        }
    }

    fn continuation_lines_for(
        &self,
        idx: usize,
        is_child: bool,
        now_ms: i64,
    ) -> Vec<(usize, String, bool)> {
        let Some(run) = self.runs.get(idx) else {
            return Vec::new();
        };
        let is_selected = self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
        if !is_selected {
            return Vec::new();
        }
        let mut out = Vec::new();
        if run.status == RunStatus::Waiting && !self.is_answered(run) {
            let (_, indent, lines) =
                self.flow_question_lines(run, self.last_render_width, now_ms);
            out.extend(lines.into_iter().skip(1).map(|t| (indent, t, true)));
        }
        if !self.can_show_inline_path(run, self.last_render_width, now_ms) {
            let extra = self.extras.get(&run.run_id);
            let path = format_executed_path(extra.and_then(|e| e.executed.as_deref()));
            if !path.is_empty() && self.last_render_width > 0 {
                let indent = (if is_child { 2 } else { 0 }) + run.status.glyph().width() + 1;
                let wrap_w = self.last_render_width.saturating_sub(indent).max(1);
                for line in wrap_words(&path, wrap_w) {
                    out.push((indent, line, false));
                }
            }
        }
        out
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 748-1177 of 3849 (repository grok-build)
````
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
        let entries = self.build_flow_entries();
        if entries.is_empty() {
            self.flow_selected_run_id = None;
            self.flow_scroll = 0;
            return;
        }
        if let Some(ref cur_id) = self.flow_selected_run_id {
            for entry in &entries {
                if entry
                    .members
                    .iter()
                    .any(|&idx| self.runs.get(idx).map(|r| &r.run_id) == Some(cur_id))
                {
                    if let Some(ref key) = entry.group_key
                        && self.runs.get(entry.rep_idx).map(|r| &r.run_id) != Some(cur_id)
                    {
                        self.flow_expanded_groups.insert(key.clone());
                    }
                    return;
                }
            }
        }
        if let Some(first) = entries.first()
            && let Some(r) = self.runs.get(first.rep_idx)
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
            KeyCode::Char(' ') => {
                if self.toggle_selected_flow_group() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
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
                        LedgerOutcome::Copy(format!("/open {}", run.run_id))
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 1178-1412 of 3849 (repository grok-build)
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 1413-1637 of 3849 (repository grok-build)
````
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
                FlowListRow::Run { idx, .. } => {
                    self.runs.get(*idx).map(|r| &r.run_id) == Some(sel_id)
                }
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
                FlowListRow::Run {
                    idx,
                    group_rep,
                    is_child,
                } => {
                    if let Some(run) = self.runs.get(*idx) {
                        let is_selected =
                            self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
                        self.render_flow_row(
                            buf,
                            row_rect,
                            run,
                            *group_rep,
                            *is_child,
                            is_selected,
                            theme,
                            now_ms,
                        );
                    }
                }
                FlowListRow::Continuation {
                    indent,
                    text,
                    is_warning,
                } => {
                    buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
                    let content = format!("{}{}", " ".repeat(*indent), text);
                    let fg = if *is_warning {
                        theme.warning
                    } else {
                        theme.gray
                    };
                    let line = Line::from(Span::styled(
                        truncate_str(&content, row_rect.width as usize),
                        Style::default().fg(fg).add_modifier(Modifier::BOLD),
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

    fn rail_column_width_for(&self, run_id: &str, show_rail: bool, show_rail_count: bool) -> usize {
        if !show_rail {
            return 0;
        }
        let extra = self.extras.get(run_id);
        let marks_w = extra
            .map(|e| e.rail.len().min(RAIL_MARKS_CAP))
            .unwrap_or(0);
        if !show_rail_count {
            return marks_w;
        }
        let count_w = executed_count_label(extra.and_then(|e| e.executed.as_deref())).width();
        if marks_w > 0 {
            marks_w + 1 + count_w
        } else {
            count_w
        }
    }

    fn widest_rail_column(&self, show_rail: bool, show_rail_count: bool) -> usize {
        if !show_rail {
            return 0;
        }
        self.runs
            .iter()
            .map(|r| self.rail_column_width_for(&r.run_id, show_rail, show_rail_count))
            .max()
            .unwrap_or(0)
    }

    fn flow_row_base_flex_budget(&self, run: &RunSummary, w: usize, now_ms: i64) -> usize {
        let extra = self.extras.get(&run.run_id);
        let stalled = extra.map(|e| e.stalled).unwrap_or(false);
        let show_rail = w >= 48;
        let show_age = w >= 64;
        let show_rail_count = w >= 80;
        let show_duration = w >= 96;
        let show_agents = w >= 120;

        let rail_width = self.widest_rail_column(show_rail, show_rail_count);
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

        let glyph = if stalled { "⊘" } else { run.status.glyph() };
        let prefix_w = glyph.width() + 1;
        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
        let fixed_w = prefix_w + rail_col_w + agents_w + age_w + dur_w;
        w.saturating_sub(fixed_w)
    }

    fn can_show_inline_path(&self, run: &RunSummary, w: usize, now_ms: i64) -> bool {
        if w < 120 {
            return false;
        }
        let extra = self.extras.get(&run.run_id);
        let path = format_executed_path(extra.and_then(|e| e.executed.as_deref()));
        if path.is_empty() {
            return false;
        }
        self.flow_row_base_flex_budget(run, w, now_ms) > 17
    }

    #[allow(clippy::too_many_arguments)]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 1638-2057 of 3849 (repository grok-build)
````
    fn render_flow_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        run: &RunSummary,
        group_rep: Option<(usize, bool)>,
        is_child: bool,
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
        // path & agents below 120, duration below 96, step count below 80, age below 64, rail below 48.
        let show_rail = w >= 48;
        let show_age = w >= 64;
        let show_rail_count = w >= 80;
        let show_duration = w >= 96;
        let show_agents = w >= 120;
        let show_inline_path = w >= 120;

        // Build rail + executed count spans
        let mut rail_spans: Vec<Span<'static>> = Vec::new();
        let mut rail_width: usize = 0;
        if show_rail {
            if let Some(ex) = extra
                && !ex.rail.is_empty()
            {
                // An answered gate is done, so the ticks read the same state.
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
            }
            if show_rail_count {
                let count_label =
                    executed_count_label(extra.and_then(|e| e.executed.as_deref()));
                let cnt = if rail_width > 0 {
                    format!(" {count_label}")
                } else {
                    count_label
                };
                rail_width += cnt.width();
                rail_spans.push(Span::styled(
                    cnt,
                    Style::default().fg(theme.gray).add_modifier(base_mod),
                ));
            }

            // Pad the rail+count column to the longest one shown, so the state text starts in one column.
            let widest = self.widest_rail_column(show_rail, show_rail_count);
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
        let child_indent = if is_child { 2 } else { 0 };
        let base_prefix_w = glyph.width() + 1;
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

        let fixed_w = base_prefix_w + rail_col_w + agents_col_w + age_col_w + dur_col_w;
        let flex_budget = w.saturating_sub(fixed_w);

        let wrap_selected_waiting =
            selected && run.status == RunStatus::Waiting && !answered && !raw_state_text.is_empty();
        let executed_path = format_executed_path(extra.and_then(|e| e.executed.as_deref()));
        let inline_path_available =
            show_inline_path && !executed_path.is_empty() && flex_budget > 17;
        let (display_state_text, display_state_color) = if inline_path_available {
            if run.status == RunStatus::Completed || raw_state_text.is_empty() {
                (executed_path, theme.gray)
            } else if wrap_selected_waiting {
                (raw_state_text, state_color)
            } else {
                (format!("{raw_state_text} · {executed_path}"), state_color)
            }
        } else {
            (raw_state_text, state_color)
        };

        let (group_marker, group_badge) = match group_rep {
            Some((count, expanded)) => (
                if expanded { "▾ " } else { "▸ " },
                format!(" ({count})"),
            ),
            None => ("", String::new()),
        };

        // Title and state text share what is left. Every row uses the same title width, so the columns line up.
        let longest_title = self
            .runs
            .iter()
            .map(|r| r.title.width())
            .max()
            .unwrap_or(0)
            + if self.has_any_flow_group() { 6 } else { 0 };

        let (title_spans, state_str) =
            if flex_budget <= 17 || display_state_text.is_empty() && rail_width == 0 {
                let first = if wrap_selected_waiting {
                    let (first, _, _) = self.flow_question_lines(run, w, now_ms);
                    first
                } else {
                    String::new()
                };
                let avail = flex_budget.saturating_sub(child_indent);
                let combined = format!("{group_marker}{}{group_badge}", run.title);
                let cut = truncate_str(&combined, avail);
                let spans = if cut.is_empty() {
                    Vec::new()
                } else {
                    vec![Span::styled(
                        cut,
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(base_mod),
                    )]
                };
                (spans, first)
            } else {
                let half = (flex_budget - 1).div_euclid(2);
                let title_col = longest_title.clamp(16, half.max(16));
                let effective_title_col = title_col.saturating_sub(child_indent);
                let mut t_spans: Vec<Span<'static>> = Vec::new();
                let marker_w = group_marker.width();
                let badge_w = group_badge.width();
                let name_budget = effective_title_col.saturating_sub(marker_w + badge_w);
                let title = truncate_str(&run.title, name_budget);
                let used_w = marker_w + title.width() + badge_w;
                let pad = effective_title_col.saturating_sub(used_w);

                if !group_marker.is_empty() {
                    t_spans.push(Span::styled(
                        group_marker.to_string(),
                        Style::default().fg(theme.gray).add_modifier(base_mod),
                    ));
                }
                if !title.is_empty() {
                    t_spans.push(Span::styled(
                        title,
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(base_mod),
                    ));
                }
                if !group_badge.is_empty() {
                    t_spans.push(Span::styled(
                        group_badge,
                        Style::default().fg(theme.gray).add_modifier(base_mod),
                    ));
                }
                if pad > 0 {
                    t_spans.push(Span::raw(" ".repeat(pad)));
                }

                let state = if wrap_selected_waiting {
                    let (first, _, _) = self.flow_question_lines(run, w, now_ms);
                    first
                } else {
                    truncate_str(&display_state_text, flex_budget - 1 - title_col)
                };
                (t_spans, state)
            };

        let mut spans: Vec<Span<'static>> = Vec::new();
        if child_indent > 0 {
            spans.push(Span::raw(" ".repeat(child_indent)));
        }
        spans.push(Span::styled(
            format!("{glyph} "),
            Style::default().fg(glyph_color).add_modifier(base_mod),
        ));
        spans.extend(title_spans);
        if !rail_spans.is_empty() {
            spans.push(Span::raw(" "));
            spans.extend(rail_spans);
        }
        if !state_str.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                state_str,
                Style::default()
                    .fg(display_state_color)
                    .add_modifier(base_mod),
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
        let show_rail_count = w >= 80;
        let rail_width = self.widest_rail_column(show_rail, show_rail_count);
        let prefix_w = run.status.glyph().width() + 1;
        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
        let flex_budget = self.flow_row_base_flex_budget(run, w, now_ms);
        let longest_title = self
            .runs
            .iter()
            .map(|r| r.title.width())
            .max()
            .unwrap_or(0)
            + if self.has_any_flow_group() { 6 } else { 0 };

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

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2367-2514 of 3849 (repository grok-build)
````
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

fn executed_count_label(executed: Option<&[(String, Outcome)]>) -> String {
    match executed {
        None => "steps unknown".to_string(),
        Some([]) => "no steps ran".to_string(),
        Some([_]) => "1 step ran".to_string(),
        Some(steps) => format!("{} steps ran", steps.len()),
    }
}

fn format_executed_path(executed: Option<&[(String, Outcome)]>) -> String {
    let Some(steps) = executed else {
        return String::new();
    };
    if steps.is_empty() {
        return String::new();
    }
    steps
        .iter()
        .map(|(node_id, outcome)| format!("{} {node_id}", NodeState::Done(*outcome).glyph()))
        .collect::<Vec<_>>()
        .join(" → ")
}

#[cfg(test)]
mod tests {
    use super::*;

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2515-2785 of 3849 (repository grok-build)
````
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
                executed: Some(vec![
                    ("plan".to_string(), Outcome::Ok),
                    ("build".to_string(), Outcome::Ok),
                    ("test".to_string(), Outcome::Ok),
                ]),
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
                executed: Some(
                    (0..6)
                        .map(|i| (format!("step-{i}"), Outcome::Ok))
                        .collect(),
                ),
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
                executed: Some(
                    (0..5)
                        .map(|i| (format!("step-{i}"), Outcome::Ok))
                        .collect(),
                ),
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
                    text.contains("5 steps ran"),
                    "huge-rail count must show executed steps at w={w}: {text:?}"
                );
                assert!(
                    !text.contains("5/262"),
                    "legacy done/total count must be replaced at w={w}: {text:?}"
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
                executed: None,
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 2986-3238 of 3849 (repository grok-build)
````
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
                executed: Some(vec![
                    ("plan".to_string(), Outcome::Ok),
                    ("build".to_string(), Outcome::Ok),
                    ("test".to_string(), Outcome::Ok),
                    ("lint".to_string(), Outcome::Failed),
                ]),
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
        assert!(full.contains("✓✓✓✗◐○ 4 steps ran"));
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
                    executed: Some(vec![
                        ("a".to_string(), Outcome::Ok),
                        ("b".to_string(), Outcome::Ok),
                        ("c".to_string(), Outcome::Ok),
                    ]),
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
                        executed: None,
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
        assert!(
            answered.contains("✓✓✓✓○○ 3 steps ran"),
            "answered row: {answered}"
        );
        assert!(!answered.contains("4/6"));

        let waiting = render(false);
        assert!(
            waiting.contains("✓✓✓◐○○ 3 steps ran"),
            "waiting row: {waiting}"
        );
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
                executed: None,
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
                executed: None,
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
                executed: None,
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
                executed: None,
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 3239-3498 of 3849 (repository grok-build)
````
    fn make_named_run(
        id: &str,
        flow_name: &str,
        title: &str,
        status: RunStatus,
        started_ms: i64,
        updated_ms: i64,
    ) -> RunSummary {
        let mut r = make_run(id, title, status, updated_ms);
        r.flow_name = flow_name.to_string();
        r.started_ms = started_ms;
        r
    }

    #[test]
    fn finished_group_folds_expands_navigates_and_keeps_pending_individual() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_named_run("r1", "deploy", "Deploy 1", RunStatus::Completed, 1_000, 1_500),
            make_named_run("r2", "deploy", "Deploy 2", RunStatus::Failed, 2_000, 2_500),
            make_named_run("r3", "deploy", "Deploy 3", RunStatus::Completed, 3_000, 3_500),
            make_named_run("r-pend", "deploy", "Deploy Gate", RunStatus::Waiting, 4_000, 4_500),
            make_named_run("r-empty-1", "", "Adhoc 1", RunStatus::Completed, 1_100, 1_200),
            make_named_run("r-empty-2", "", "Adhoc 2", RunStatus::Completed, 1_200, 1_300),
        ]);

        assert!(state.shortcuts().contains(&"Space group"));

        // Pending run is selected first and is not a group.
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-pend"));
        assert_eq!(
            state.handle_key(&key(KeyCode::Char(' '))),
            LedgerOutcome::Unchanged
        );

        // Render folded state: FINISHED count is in units of runs (3 grouped + 2 empty-name = 5).
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let folded_text = all_text(&buf);
        assert!(folded_text.contains("NEEDS YOU 1"), "{folded_text}");
        assert!(folded_text.contains("FINISHED 5"), "{folded_text}");
        assert!(folded_text.contains("▸"), "{folded_text}");
        assert!(folded_text.contains("(3)"), "{folded_text}");

        // Move down to collapsed group representative r3; older members r2/r1 are folded.
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Changed);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r3"));
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Changed);
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("r-empty-2")
        );
        // Empty flow_name runs are never grouped.
        assert_eq!(
            state.handle_key(&key(KeyCode::Char(' '))),
            LedgerOutcome::Unchanged
        );

        // Move back to r3 and expand with Space.
        state.handle_key(&key(KeyCode::Up));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r3"));
        assert_eq!(
            state.handle_key(&key(KeyCode::Char(' '))),
            LedgerOutcome::Changed
        );
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r3"));

        // Older members appear newest-first: r2 then r1.
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Changed);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenRun(PathBuf::from("/tmp/runs/r2"))
        );
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Changed);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));

        // Explicit collapse while child r1 is selected moves selection to representative r3.
        assert_eq!(
            state.handle_key(&key(KeyCode::Char(' '))),
            LedgerOutcome::Changed
        );
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r3"));
    }

    #[test]
    fn finished_group_tie_breaks_representative_and_order_by_run_id() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_named_run("2026-01-01-a", "ci", "CI A", RunStatus::Completed, 5_000, 5_500),
            make_named_run("2026-01-01-c", "ci", "CI C", RunStatus::Completed, 5_000, 5_100),
            make_named_run("2026-01-01-b", "ci", "CI B", RunStatus::Failed, 5_000, 5_900),
        ]);

        // Greatest run_id wins the started_ms tie.
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("2026-01-01-c")
        );

        // Expanding lists remaining members descending by run_id.
        assert_eq!(
            state.handle_key(&key(KeyCode::Char(' '))),
            LedgerOutcome::Changed
        );
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("2026-01-01-b")
        );
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(
            state.selected_run().map(|r| r.run_id.as_str()),
            Some("2026-01-01-a")
        );
    }

    #[test]
    fn finished_group_search_matches_older_id_and_refresh_preserves_selection() {
        let mut state = LedgerState::new();
        let r1 = make_named_run("run-001", "deploy", "Deploy", RunStatus::Completed, 1_000, 1_500);
        let r2 = make_named_run("run-002", "deploy", "Deploy", RunStatus::Completed, 2_000, 2_500);
        let r3 = make_named_run("run-003", "deploy", "Deploy", RunStatus::Completed, 3_000, 3_500);
        state.set_runs(vec![r1.clone(), r2.clone(), r3.clone()]);

        // Folded initially; representative is run-003.
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("run-003"));

        // Search matches older folded ID run-001 -> selectable immediately.
        state.handle_key(&key(KeyCode::Char('/')));
        for ch in "run-001".chars() {
            state.handle_key(&key(KeyCode::Char(ch)));
        }
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("run-001"));

        // Clear filter: run-001 stays selected (group expands so it stays visible).
        state.handle_key(&key(KeyCode::Esc));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("run-001"));

        // Collapse explicitly -> moves to rep run-003.
        state.handle_key(&key(KeyCode::Char(' ')));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("run-003"));

        // Refresh adds a newer finished run-004 of the same flow: selection stays on run-003.
        let r4 = make_named_run("run-004", "deploy", "Deploy", RunStatus::Completed, 4_000, 4_500);
        state.set_runs(vec![r1, r2, r3, r4]);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("run-003"));
    }

    #[test]
    fn stalled_running_and_answered_waiting_same_flow_name_stay_individual() {
        let mut state = LedgerState::new();
        let r1 = make_named_run("r1", "deploy", "Deploy 1", RunStatus::Completed, 1_000, 1_500);
        let r2 = make_named_run("r2", "deploy", "Deploy 2", RunStatus::Completed, 2_000, 2_500);
        let stalled =
            make_named_run("r-stalled", "deploy", "Deploy Stalled", RunStatus::Running, 3_000, 3_500);
        let answered =
            make_named_run("r-ans", "deploy", "Deploy Gate", RunStatus::Waiting, 4_000, 4_500);
        let next =
            make_named_run("r-next", "other", "Followup", RunStatus::Completed, 5_000, 5_500);
        let mut extras = HashMap::new();
        extras.insert(
            "r-stalled".to_string(),
            RowExtra {
                rail: vec![],
                executed: None,
                agents: String::new(),
                stalled: true,
                question: None,
                answers: None,
            },
        );
        extras.insert(
            "r-next".to_string(),
            RowExtra {
                rail: vec![],
                executed: None,
                agents: String::new(),
                stalled: false,
                question: None,
                answers: Some(GateAnswer {
                    run_id: "r-ans".to_string(),
                    choice: "Ship".to_string(),
                }),
            },
        );
        // Extras first so the initial selection sees r-ans as already answered.
        state.set_extras(extras);
        state.set_runs(vec![r1, r2, stalled, answered, next]);

        // All 5 are in FINISHED; r-next, r-ans, r-stalled are individual; only r2/r1 form a group.
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let text = all_text(&buf);
        assert!(text.contains("FINISHED 5"), "{text}");
        assert!(text.contains("(2)"), "{text}");

        // Walk down: r-next -> r-ans -> r-stalled -> r2 (group).
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-next"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Unchanged);

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-ans"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Unchanged);

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-stalled"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Unchanged);

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Changed);
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));
    }

    #[test]
    fn finished_group_works_under_recent_and_name_orders() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_named_run("r1", "deploy", "Alpha", RunStatus::Completed, 1_000, 1_500),
            make_named_run("r2", "deploy", "Alpha", RunStatus::Completed, 2_000, 2_500),
            make_named_run("r-other", "other", "Beta", RunStatus::Completed, 3_000, 3_500),
        ]);

        // Cycle to Recent: r-other (3500) then collapsed group rep r2 (2500).
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-other"));
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
        // Folded: cannot move past r2.
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Unchanged);
        // Expand under Recent reveals r1.
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Changed);
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));
        // Collapse back to r2.
        state.handle_key(&key(KeyCode::Char(' ')));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Cycle to Name: Alpha group (r2) comes before Beta (r-other).
        state.handle_key(&key(KeyCode::Char('s')));
        state.handle_key(&key(KeyCode::Home));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-other"));
        // Expand Alpha group under Name reveals r1 directly beneath r2.
        state.handle_key(&key(KeyCode::Up));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Changed);
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-other"));
    }

    #[test]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 3499-3848 of 3849 (repository grok-build)
````
    fn executed_count_and_path_with_retry_unknown_empty_and_unrun_rail() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-retry", "Retry build", RunStatus::Completed, 3_000),
            make_run("r-unknown", "Unknown steps", RunStatus::Completed, 2_000),
            make_run("r-empty", "Empty steps", RunStatus::Completed, 1_000),
        ]);
        let mut extras = HashMap::new();
        extras.insert(
            "r-retry".to_string(),
            RowExtra {
                rail: vec![
                    NodeState::Done(Outcome::Ok),
                    NodeState::Done(Outcome::Failed),
                    NodeState::NotRun,
                    NodeState::NotRun,
                ],
                executed: Some(vec![
                    ("cargo_check".to_string(), Outcome::Ok),
                    ("repair_compile".to_string(), Outcome::Failed),
                    ("cargo_check".to_string(), Outcome::Ok),
                ]),
                agents: String::new(),
                stalled: false,
                question: None,
                answers: None,
            },
        );
        extras.insert(
            "r-empty".to_string(),
            RowExtra {
                rail: vec![NodeState::NotRun, NodeState::NotRun],
                executed: Some(Vec::new()),
                agents: String::new(),
                stalled: false,
                question: None,
                answers: None,
            },
        );
        // r-unknown intentionally has no RowExtra entry.
        state.set_extras(extras);

        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 12));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let full = all_text(&buf);

        // Full-width retry row shows rail with unrun nodes, count, and ordered path with retry.
        let retry_line = (0..buf.area.height)
            .map(|y| row_text(&buf, y))
            .find(|line| line.contains("Retry build"))
            .unwrap_or_default();
        assert!(retry_line.contains("✓✗○○"), "unrun nodes missing from rail: {retry_line}");
        assert!(retry_line.contains("3 steps ran"), "count missing: {retry_line}");
        assert!(
            retry_line.contains("✓ cargo_check → ✗ repair_compile → ✓ cargo_check"),
            "executed path missing or wrong order: {retry_line}"
        );

        // Missing extras renders 'steps unknown', never '0 steps ran'.
        let unknown_line = (0..buf.area.height)
            .map(|y| row_text(&buf, y))
            .find(|line| line.contains("Unknown steps"))
            .unwrap_or_default();
        assert!(unknown_line.contains("steps unknown"), "{unknown_line}");
        assert!(!unknown_line.contains("0 steps"), "{unknown_line}");

        // Some(empty) renders 'no steps ran' while keeping unrun rail marks.
        let empty_line = (0..buf.area.height)
            .map(|y| row_text(&buf, y))
            .find(|line| line.contains("Empty steps"))
            .unwrap_or_default();
        assert!(empty_line.contains("○○"), "{empty_line}");
        assert!(empty_line.contains("no steps ran"), "{empty_line}");
        assert!(!full.contains("2/4"), "{full}");
    }

    #[test]
    fn narrow_width_wraps_full_path_only_for_selected_run() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r-sel", "Selected run", RunStatus::Completed, 2_000),
            make_run("r-unsel", "Unselected run", RunStatus::Completed, 1_000),
        ]);
        let long_path = vec![
            ("plan".to_string(), Outcome::Ok),
            ("cargo_check".to_string(), Outcome::Ok),
            ("repair_compile".to_string(), Outcome::Failed),
            ("cargo_check".to_string(), Outcome::Ok),
            ("publish_release".to_string(), Outcome::Ok),
        ];
        let mut extras = HashMap::new();
        for id in ["r-sel", "r-unsel"] {
            extras.insert(
                id.to_string(),
                RowExtra {
                    rail: vec![NodeState::Done(Outcome::Ok); 3],
                    executed: Some(long_path.clone()),
                    agents: String::new(),
                    stalled: false,
                    question: None,
                    answers: None,
                },
            );
        }
        state.set_extras(extras);

        // Narrow width (< 120): path dropped from rows, wrapped below selected run only.
        let mut buf = Buffer::empty(Rect::new(0, 0, 72, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let lines: Vec<String> = (0..buf.area.height).map(|y| row_text(&buf, y)).collect();

        let sel_idx = lines
            .iter()
            .position(|l| l.contains("Selected run"))
            .expect("selected row");
        let unsel_idx = lines
            .iter()
            .position(|l| l.contains("Unselected run"))
            .expect("unselected row");

        // Selected row itself does not show the inline path; lines between sel and unsel do.
        assert!(!lines[sel_idx].contains("repair_compile"), "{:?}", lines[sel_idx]);
        let wrapped_below_sel = lines[sel_idx + 1..unsel_idx].join(" ");
        assert!(wrapped_below_sel.contains("plan"), "{wrapped_below_sel}");
        assert!(wrapped_below_sel.contains("repair_compile"), "{wrapped_below_sel}");
        assert!(wrapped_below_sel.contains("publish_release"), "{wrapped_below_sel}");

        // Unselected row has neither inline path nor continuation lines below it.
        assert!(!lines[unsel_idx].contains("repair_compile"), "{:?}", lines[unsel_idx]);
        let below_unsel = lines[unsel_idx + 1..].join(" ");
        assert!(!below_unsel.contains("repair_compile"), "{below_unsel}");
    }

    #[test]
    fn y_copies_open_command_on_flows_and_record_id_on_sessions() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_named_run("r-old", "ship", "Ship", RunStatus::Completed, 1_000, 1_500),
            make_named_run("r-new", "ship", "Ship", RunStatus::Completed, 2_000, 2_500),
        ]);
        state.set_sessions(vec![make_session("rec-42", "Agent Turn", false, 2_000)]);

        // Collapsed group representative copies /open <rep_id>.
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("/open r-new".to_string())
        );

        // Expanded child copies /open <child_id>.
        state.handle_key(&key(KeyCode::Char(' ')));
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("/open r-old".to_string())
        );

        // Agent Turns tab still copies the selected record ID.
        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("rec-42".to_string())
        );
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
                executed: None,
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
                executed: None,
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
                executed: None,
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
                executed: None,
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
                executed: None,
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
                executed: None,
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
                executed: None,
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 311-352 of 1628 (repository grok-build)
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 695-747 of 1628 (repository grok-build)
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
        executed: Some(
            run.steps
                .iter()
                .map(|step| (step.node_id.clone(), step.outcome))
                .collect(),
        ),
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

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1439-1517 of 1628 (repository grok-build)
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

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 1-135 of 1936 (repository grok-build)
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
            "waiting" => Self::Waiting,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Unknown => "unknown",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Waiting => "◐",
            Self::Completed => "✓",
            Self::Failed => "✗",
            Self::TimedOut => "✗",
            Self::Unknown => "○",
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Running)
    }
}

impl NodeKind {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "acp" => Self::Acp,
            "action" => Self::Action,
            "compute" => Self::Compute,
            "checkpoint" => Self::Checkpoint,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Action => "action",
            Self::Compute => "compute",
            Self::Checkpoint => "checkpoint",
            Self::Unknown => "?",
        }
    }
}

impl Outcome {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "ok" => Self::Ok,
            "timed_out" => Self::TimedOut,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }
}

impl NodeState {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::NotRun => "○",
            Self::Running => "●",
            Self::Waiting => "◐",
            Self::Done(Outcome::Ok) => "✓",
            Self::Done(Outcome::Failed) => "✗",
            Self::Done(Outcome::TimedOut) => "✗",
            Self::Done(Outcome::Cancelled) => "⊘",
            Self::Done(Outcome::Unknown) => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 1347-1355 of 1936 (repository grok-build)
````
/// The flow file a run was started from (manifest.json, else projections/live.json).
pub fn flow_path(run_dir: &Path) -> Option<PathBuf> {
    [run_dir.join("manifest.json"), run_dir.join("projections").join("live.json")]
        .iter()
        .find_map(|file| read_json_file(file).and_then(|v| string_field(&v, "flowPath")))
        .map(PathBuf::from)
}

/// A waiting run's gate choices: the waiting checkpoint step's output `next` object (`{ "<label>": "<flow file>" }`),
````

### `docs/acpx-modal-opening-screen.md` lines 1-62 of 63 (repository grok-build)
````
# ACPX opening screen proposal

Draft for review. This proposal covers the Flows screen you see when opening `/acpx`. It does not redesign workflow cards in the main pager, the open-run stage view, or the inspector. Those have specific review and evaluation cases already.

## What the screen should answer

1. What work is running, and what is waiting on a decision?
2. What happened on the run I selected?
3. How do I get to the exact run or step when I need the full evidence?

Open directly into Flows. Make that screen useful before opening anything. Keep the full ledger on the same screen, with groups the operator can expand, rather than adding a separate landing page or another Enter before they can inspect a run. Keep Agent Turns as the other tab.

## First view

Use three sections in the default grouped order: **Waiting for input**, **Running**, and **Recent outcomes**. Omit empty sections. The labels describe what the workflow is doing without guessing what the operator cares about.

- **Waiting for input:** Give every recorded wait the same treatment. Group repeated waits by workflow, project, and source when known, and sort groups and their members by most recent update. Each gate remains individually selectable. A group never borrows a member's question or choices. Age is context; it never hides or downgrades a wait. Keep a distinct status for answers in flight or accepted while the engine still reports waiting. Packet 5 owns gate and answer state.
- **Running:** Keep each run individually selectable. Show the known current stage. A stalled run remains here with a stalled label.
- **Recent outcomes:** Use packet 2's newest-first finished grouping and preserve each historical run. Keep the latest outcome visible on the collapsed row. Show failures and cancellations in plain language. The dot rail, executed-attempt count, and named execution path retain the agreed width behavior in packet 2. They describe the selected or representative run, never the sum of all members.

The guidance to place a gate when a result is worth checking or a build depends on it is for workflow authors. The modal applies no priority tier or extra metadata requirement based on that guidance.

Illustration using the reported repeated `ship: slugify` and `mixed-talk` entries and the historical `beta8-merge` failure. The waiting counts are examples from the conversation. The right pane illustrates what to show when recorded evidence is available.

```text
 Flows  |  Agent Turns

 Waiting for input            │ beta8-merge · Failed
 ▸ ship: slugify    5 waiting  │ Failed at cargo_check
 ▸ mixed-talk      2 waiting  │ Compiler error: <saved diagnostic, if present>
                              │
 Recent outcomes              │ Executed path
 > beta8-merge     Failed     │ <recorded attempts in order>
 ▸ mixed-talk      8 finished │
                              │
 Space  Expand group          │ Tab  Focus preview
 /  Filter runs               │ Enter  Open run
```

The example has a failure selected. On a fresh open, select the first row in the current sort order. After inspecting a run or refreshing the list, keep the same selected run by ID while it remains available.

## What gets disclosed on selection and on open

- **Select a group:** Show its count and what it contains. For a finished group, the preview shows the newest run and labels it as such. For a waiting group, show that there are several separate gates and `Space` reveals them. Do not show one member's choices as group actions.
- **Select a run:** At wide widths, the right pane is a compact, read-only summary. Show its real status and one useful piece of evidence: the saved gate question and available choices, the active step, the failed attempt and a short diagnostic, or the result of a completed run. Use the recorded path and outcome rather than unrun nodes or raw JSON wrappers. If the summary has no diagnostic or question, say what is missing. The full output stays in the open run and inspector.
- **Narrow widths:** Put a brief status or question below the selected row, plus packet 2's selected-run path where applicable. Keep that inline preview focusable with the same keys as the side pane. Keep the workflow name and status readable, let the question wrap, and keep long output behind Enter.
- **Focus and copy:** In Flows, `Tab` and `Shift+Tab` move between the list and the selected preview, with a visible focus marker. Use `h`/`l` or Left/Right from the list to switch between Flows and Agent Turns. In the preview, reuse packet 3b's keys: `j`/`k` or arrows move the line cursor, `v` selects a range, `y` copies the line or range, and `Y` copies the preview. Copy original text without line breaks introduced by wrapping. `Esc` clears a range first and then returns focus to the same list row. The list keeps its existing copy behavior.
- **Open:** With the list focused, `Space` expands or folds a group; `Enter` on a waiting group expands it, and `Enter` on an individual run opens it. For a finished group, preserve packet 2's representative-run behavior. Keep `/` filter and existing sort options. Searching by run ID reveals a matching run through folded groups. `Esc` from an open run returns to the same row and expansion state. Previewing or opening a run never submits, starts, or cancels anything.

## Review against these cases

| Case | What should be observable |
| --- | --- |
| Five `ship: slugify` waits and two `mixed-talk` waits | The default view shows two group rows. Expand `ship: slugify`; each member is separately selectable and shows its own question. A five-day-old wait is still reachable. A known build dependency does not change where its gate appears. |
| `beta8-merge` failure | Select the run, Tab into its preview, and copy an exact saved diagnostic line when available. Esc returns focus to the same row; Enter opens the full evidence. Repeat the copy from the inline preview at narrow width. |
| Completed run with retries | Keep the rail, count, and named path; retries occur in recorded order. Narrow width keeps the selected run's full path reachable without widening the screen. |
| Waiting run updates after an answer | The UI distinguishes the answer status from the engine state. The row moves to outcomes only when the engine reports a terminal state. No input is sent by opening the row. |
| Empty list or no matches | Say which case applies. The other tab and clearing a filter stay reachable. |

## Boundary with the current packets

This proposal replaces packet 4's extra-entry-screen approach if accepted. Packet 2's finished grouping and rail/count/path contract stay intact. This proposal changes how repeated waits appear by default: multiple waits share a folded row, and each remains individually selectable after expansion. The compact preview is also new work. Packet 5 still governs permissions and engine state. The main pager, modal stage view, and inspector need their already identified walkthroughs, not a second blanket design pass. Review whether this arrangement solves the clutter and makes the next action clear. Code, slice boundaries, and dispatch are outside this draft.
````

=== TASK SPECIFICATION ===

Slice ID: acpx-opening-ledger-delta

Opening-screen delta after the accepted run2 packet 2 (1f7fe9b1). The
attached source already has finished grouping, rail/count/path and list copy.
Preserve those implementation and test contracts. Do not regenerate them.
Own Flows row grouping, selection and progress in crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs and its
unit tests, including the narrow inline render hook below. In crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
edit only the row_extra producer/cache and its tests. Leave modal preview
state, input and rendering to packet 4. Keep LedgerOutcome routing compatible.

The default Grouped order has Waiting for input, Running and Recent outcomes;
omit empty sections. Use actual RunStatus: stalled
Running remains Running with its stalled label, and answered Waiting remains
Waiting with the recorded answer distinction. Do not fabricate sent/accepted
status or change gate control. Keep running runs individual and group repeated
Waiting runs separately from finished groups. Group waits with the same
nonempty flow_name and source root (the run directory's parent); when an
absolute recorded flow path is available, also key on it so same-name runs from
different projects stay separate. Add an optional PathBuf to RowExtra for the
recorded path, supplied via bundle::flow_path inside cached row_extra; never
require that a recorded file exists. Missing paths only group with other
missing paths from the same source root. Waiting groups and their members sort
by updated_ms newest first with a stable tie; age and build association do not
change priority or visibility. Leave empty workflow names as individual runs.
Keep section counts in units of runs. Fold repeated waits in the default
Grouped order. Recent and Name continue to let operators browse waits by those
orders without the waiting-group fold; preserve existing sort and active
filter controls.

Extend the existing Space group toggle to waiting groups. Keep existing
shortcut/help behavior until packet 4 routes preview focus; no changes to
Ledger-mode key dispatch or help in crates/codegen/xai-grok-pager/src/views/acpx_modal.rs in this packet.
Enter on a waiting group expands it without opening a member; Enter on a run
or a finished group opens the same run as before. Expanded groups expose every
member. On a waiting group header the list y emits no copy outcome, because
there is no selected run ID. When search matches an older ID, reveal it even
if previously folded. Refresh keeps selection by run ID even if a newer
member arrives. On explicit collapse of a selected child, select the visible
group row or finished representative as appropriate. Leave the Agent Turns tab
(LedgerTab::Sessions; its label now reads Agent Turns) behavior alone.

Implement the shared inline interface using the existing continuation routing
in build_flow_rows, append_continuations and continuation_lines_for and the
existing wrapping and scroll paths in render_flows and render_flow_row. Some
replaces the selected run's existing question/path presentation, including
its first question fragment or inline row path when duplicated by the supplied
preview. None leaves packet 2's row and continuation rendering unchanged.
Something the modal supplies for the selected waiting group must also render
below its group row, without resolving a member. Give focused preview lines a
visible cursor/range style and include wrapping in height, scroll and overflow.
Keep the selected row reachable even if the preview is taller than the viewport.
Keep recorded rail, count, path and concrete-run y untouched. Update RowExtra
literals inside attached excerpts for the new optional path; name any remaining
literal as a coordinator follow-up. Do not add render I/O or widen cache limits.

Tests: keep packet 2's finished grouping, filter, recent/name sorts, retry,
unknown/empty, width and copy assertions. Add five ship waits and two mixed-talk
waits as two groups; source/project separation; one older wait still reachable;
Space/Enter expansion and no group y; individual run ID survives a refresh;
waiting with a recorded answer stays Waiting and stalled Running stays Running;
no closed or empty section placeholder; inline focused cursor visible with
wrapping and no duplicate question or path. Update existing assertions for the
changed section labels and classification only; leave gate evidence intact.

Shared ledger/preview contract (ledger owns the definitions, preview consumes):
- LedgerState::selected_waiting_group(&self) -> Option<(&str, usize)> returns
  the selected waiting group's display label and number of members, only on
  Flows. On that group selected_run() returns None; on a finished group it
  still returns the representative. The group itself has no gate or run ID.
- LedgerState::selected_run_is_finished_group(&self) -> bool is true only for
  a selected finished group representative, not an ordinary run or child.
- LedgerState::set_inline_preview(&mut self, lines: Option<Vec<InlinePreviewLine>>)
  sets the selected row's inline preview for the next render. In ledger.rs,
  pub(super) struct InlinePreviewLine has pub(super) fields text: String,
  cursor: bool, selected: bool. Each item is one logical line. None preserves
  the existing ledger continuation behavior; Some, including Some(empty),
  replaces default question/path continuations so they appear at most once.
  The ledger wraps for display and scrolls the focused cursor into view; the
  modal owns unwrapped copy text and never copies display indentation or wraps.
  Only the modal supplies lines, per render, in the narrow Flows view. No new
  I/O in the ledger, no new dependencies, and no new gate authority.

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
=== FILE: task-scripts/acpx-disclosure-returns/acpx-opening-ledger-delta.edits.md ===
```markdown
<complete Markdown edit list>
```
