=== FILE: task-scripts/acpx-disclosure-returns/acpx-opening-ledger-delta.edits.md ===
```markdown
## Edit 1: Populate recorded flow path in cached row_extra
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
        flow_path: bundle::flow_path(&summary.dir),
    }
}
~~~

## Edit 2: Add row_extra flow_path test without requiring recorded file to exist
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
        let state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.extras.contains_key("run-001"));
        assert_eq!(
            state.ledger.answered_choice("waiting-run"),
            Some("ship")
        );
    }
}
~~~
NEW:
~~~rust
        let state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.extras.contains_key("run-001"));
        assert_eq!(
            state.ledger.answered_choice("waiting-run"),
            Some("ship")
        );
    }

    #[test]
    fn row_extra_reads_recorded_flow_path_without_requiring_file_to_exist() {
        let fx = write_fixture("completed");
        let missing_path = "/nonexistent/project-alpha/flows/ship.yaml";
        fs::write(
            fx.run_dir.join("manifest.json"),
            format!(r#"{{"flowPath":"{missing_path}"}}"#),
        )
        .expect("write manifest.json");

        let runs = bundle::list_runs(&fx.runs_base);
        let summary = runs.first().expect("one run");
        let extra = row_extra(summary);
        assert_eq!(extra.flow_path, Some(PathBuf::from(missing_path)));
    }
}
~~~

## Edit 3: Add flow_path to RowExtra and define InlinePreviewLine
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// One logical line supplied by the modal for narrow inline preview rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlinePreviewLine {
    pub(super) text: String,
    pub(super) cursor: bool,
    pub(super) selected: bool,
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
    pub flow_path: Option<PathBuf>,
}
~~~

## Edit 4: Update SectionKind labels, add waiting-group types, and update LedgerState fields
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    WaitingForInput,
    Running,
    RecentOutcomes,
}

impl SectionKind {
    fn label(self) -> &'static str {
        match self {
            Self::WaitingForInput => "Waiting for input",
            Self::Running => "Running",
            Self::RecentOutcomes => "Recent outcomes",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::WaitingForInput => theme.warning,
            Self::Running => theme.accent_running,
            Self::RecentOutcomes => theme.gray_dim,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WaitingGroupKey {
    flow_name: String,
    source_root: PathBuf,
    recorded_flow_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlowSelection {
    Run(String),
    WaitingGroup(WaitingGroupKey),
}

#[derive(Debug, Clone)]
enum FlowGroupKind {
    Single,
    FinishedGroup(String),
    WaitingGroup(WaitingGroupKey),
}

#[derive(Debug, Clone)]
struct FlowGroupEntry {
    /// Representative run index in `runs` (newest member for a group).
    rep_idx: usize,
    /// Member indices in `runs` ordered newest-first (`members[0] == rep_idx`).
    members: Vec<usize>,
    kind: FlowGroupKind,
}

#[derive(Debug, Clone)]
enum GroupRef {
    Finished { key: String, rep_run_id: String },
    Waiting { key: WaitingGroupKey },
}

#[derive(Debug, Clone)]
enum VisibleFlowItem {
    Run {
        run_idx: usize,
        /// When this row represents a group of 2+ finished runs: `(member_count, expanded)`.
        group_rep: Option<(usize, bool)>,
        /// True when this row is a member inside an expanded finished or waiting group.
        is_child: bool,
        /// When this row belongs to a group (rep or child).
        group_ref: Option<GroupRef>,
    },
    WaitingGroup {
        key: WaitingGroupKey,
        count: usize,
        expanded: bool,
        latest_updated_ms: i64,
    },
}

impl VisibleFlowItem {
    fn selection(&self, runs: &[RunSummary]) -> Option<FlowSelection> {
        match self {
            Self::Run { run_idx, .. } => runs
                .get(*run_idx)
                .map(|r| FlowSelection::Run(r.run_id.clone())),
            Self::WaitingGroup { key, .. } => Some(FlowSelection::WaitingGroup(key.clone())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContinuationStyle {
    Warning,
    Muted,
    Preview { cursor: bool, selected: bool },
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
    WaitingGroup {
        key: WaitingGroupKey,
        count: usize,
        expanded: bool,
        latest_updated_ms: i64,
    },
    Continuation {
        indent: usize,
        prefix: &'static str,
        text: String,
        style: ContinuationStyle,
    },
}

impl FlowListRow {
    fn is_selectable(&self) -> bool {
        matches!(self, Self::Run { .. } | Self::WaitingGroup { .. })
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
    flow_selection: Option<FlowSelection>,
    flow_scroll: usize,
    flow_expanded_groups: HashSet<String>,
    flow_expanded_waiting_groups: HashSet<WaitingGroupKey>,
    inline_preview: Option<Vec<InlinePreviewLine>>,

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
            flow_selection: None,
            flow_scroll: 0,
            flow_expanded_groups: HashSet::new(),
            flow_expanded_waiting_groups: HashSet::new(),
            inline_preview: None,

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
~~~

## Edit 5: Update selected_run and add shared preview/group contract methods
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    pub fn selected_run(&self) -> Option<&RunSummary> {
        if self.tab != LedgerTab::Flows {
            return None;
        }
        let FlowSelection::Run(ref id) = self.flow_selection.as_ref()? else {
            return None;
        };
        self.visible_flow_items().into_iter().find_map(|item| match item {
            VisibleFlowItem::Run { run_idx, .. } => {
                self.runs.get(run_idx).filter(|r| r.run_id == *id)
            }
            VisibleFlowItem::WaitingGroup { .. } => None,
        })
    }

    pub fn selected_waiting_group(&self) -> Option<(&str, usize)> {
        if self.tab != LedgerTab::Flows {
            return None;
        }
        let FlowSelection::WaitingGroup(ref key) = self.flow_selection.as_ref()? else {
            return None;
        };
        let entry = self.build_flow_entries().into_iter().find(|entry| {
            matches!(&entry.kind, FlowGroupKind::WaitingGroup(k) if k == key)
        })?;
        Some((key.flow_name.as_str(), entry.members.len()))
    }

    pub fn selected_run_is_finished_group(&self) -> bool {
        if self.tab != LedgerTab::Flows {
            return false;
        }
        let Some(FlowSelection::Run(ref id)) = self.flow_selection else {
            return false;
        };
        self.visible_flow_items().into_iter().any(|item| match item {
            VisibleFlowItem::Run {
                run_idx,
                group_rep: Some(_),
                ..
            } => self.runs.get(run_idx).map(|r| &r.run_id) == Some(id),
            _ => false,
        })
    }

    pub(super) fn set_inline_preview(&mut self, lines: Option<Vec<InlinePreviewLine>>) {
        self.inline_preview = lines;
    }
~~~

## Edit 6: Keep stalled Running in Running section and add waiting-group key/sort helpers
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn run_section(&self, run: &RunSummary) -> SectionKind {
        if run.status == RunStatus::Waiting && !self.is_answered(run) {
            SectionKind::WaitingForInput
        } else if run.status == RunStatus::Running {
            SectionKind::Running
        } else {
            SectionKind::RecentOutcomes
        }
    }

    fn is_groupable_finished(run: &RunSummary) -> bool {
        !run.flow_name.is_empty()
            && matches!(
                run.status,
                RunStatus::Completed | RunStatus::Failed | RunStatus::TimedOut
            )
    }

    fn is_groupable_waiting(&self, run: &RunSummary) -> bool {
        !run.flow_name.is_empty() && run.status == RunStatus::Waiting && !self.is_answered(run)
    }

    fn waiting_group_key(&self, run: &RunSummary) -> Option<WaitingGroupKey> {
        if !self.is_groupable_waiting(run) {
            return None;
        }
        let source_root = run
            .dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let recorded_flow_path = self
            .extras
            .get(&run.run_id)
            .and_then(|e| e.flow_path.as_ref())
            .filter(|p| p.is_absolute())
            .cloned();
        Some(WaitingGroupKey {
            flow_name: run.flow_name.clone(),
            source_root,
            recorded_flow_path,
        })
    }

    fn cmp_waiting_newest_first(a: &RunSummary, b: &RunSummary) -> std::cmp::Ordering {
        b.updated_ms
            .cmp(&a.updated_ms)
            .then_with(|| b.started_ms.cmp(&a.started_ms))
            .then_with(|| a.run_id.cmp(&b.run_id))
    }

    fn cmp_group_newest_first(a: &RunSummary, b: &RunSummary) -> std::cmp::Ordering {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    }
~~~

## Edit 7: Build waiting groups in Grouped order and route inline preview continuations
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn build_flow_entries(&self) -> Vec<FlowGroupEntry> {
        let mut waiting_grouped: HashMap<WaitingGroupKey, Vec<usize>> = HashMap::new();
        let mut finished_grouped: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut entries: Vec<FlowGroupEntry> = Vec::new();

        for (i, run) in self.runs.iter().enumerate() {
            if !self.matches_flow_filter(run) {
                continue;
            }
            if self.flow_order == FlowOrder::Grouped
                && let Some(key) = self.waiting_group_key(run)
            {
                waiting_grouped.entry(key).or_default().push(i);
            } else if Self::is_groupable_finished(run) {
                finished_grouped
                    .entry(run.flow_name.as_str())
                    .or_default()
                    .push(i);
            } else {
                entries.push(FlowGroupEntry {
                    rep_idx: i,
                    members: vec![i],
                    kind: FlowGroupKind::Single,
                });
            }
        }

        for (key, mut members) in waiting_grouped {
            if members.len() >= 2 {
                members
                    .sort_by(|&a, &b| Self::cmp_waiting_newest_first(&self.runs[a], &self.runs[b]));
                let rep_idx = members[0];
                entries.push(FlowGroupEntry {
                    rep_idx,
                    members,
                    kind: FlowGroupKind::WaitingGroup(key),
                });
            } else if let Some(&idx) = members.first() {
                entries.push(FlowGroupEntry {
                    rep_idx: idx,
                    members: vec![idx],
                    kind: FlowGroupKind::Single,
                });
            }
        }

        for (flow_name, mut members) in finished_grouped {
            if members.len() >= 2 {
                members.sort_by(|&a, &b| Self::cmp_group_newest_first(&self.runs[a], &self.runs[b]));
                let rep_idx = members[0];
                entries.push(FlowGroupEntry {
                    rep_idx,
                    members,
                    kind: FlowGroupKind::FinishedGroup(flow_name.to_string()),
                });
            } else if let Some(&idx) = members.first() {
                entries.push(FlowGroupEntry {
                    rep_idx: idx,
                    members: vec![idx],
                    kind: FlowGroupKind::Single,
                });
            }
        }

        match self.flow_order {
            FlowOrder::Grouped => {
                entries.sort_by(|a, b| {
                    let ra = &self.runs[a.rep_idx];
                    let rb = &self.runs[b.rep_idx];
                    let sa = match self.run_section(ra) {
                        SectionKind::WaitingForInput => 0,
                        SectionKind::Running => 1,
                        SectionKind::RecentOutcomes => 2,
                    };
                    let sb = match self.run_section(rb) {
                        SectionKind::WaitingForInput => 0,
                        SectionKind::Running => 1,
                        SectionKind::RecentOutcomes => 2,
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
        match &entry.kind {
            FlowGroupKind::WaitingGroup(key) => {
                let expanded = self.flow_expanded_waiting_groups.contains(key);
                let latest_updated_ms = self
                    .runs
                    .get(entry.rep_idx)
                    .map(|r| r.updated_ms)
                    .unwrap_or(0);
                let mut items = vec![VisibleFlowItem::WaitingGroup {
                    key: key.clone(),
                    count: entry.members.len(),
                    expanded,
                    latest_updated_ms,
                }];
                if expanded {
                    for &member_idx in &entry.members {
                        items.push(VisibleFlowItem::Run {
                            run_idx: member_idx,
                            group_rep: None,
                            is_child: true,
                            group_ref: Some(GroupRef::Waiting { key: key.clone() }),
                        });
                    }
                }
                items
            }
            FlowGroupKind::FinishedGroup(key) => {
                let expanded = self.flow_expanded_groups.contains(key);
                let rep_id = self
                    .runs
                    .get(entry.rep_idx)
                    .map(|r| r.run_id.clone())
                    .unwrap_or_default();
                let mut items = vec![VisibleFlowItem::Run {
                    run_idx: entry.rep_idx,
                    group_rep: Some((entry.members.len(), expanded)),
                    is_child: false,
                    group_ref: Some(GroupRef::Finished {
                        key: key.clone(),
                        rep_run_id: rep_id.clone(),
                    }),
                }];
                if expanded {
                    for &child_idx in &entry.members[1..] {
                        items.push(VisibleFlowItem::Run {
                            run_idx: child_idx,
                            group_rep: None,
                            is_child: true,
                            group_ref: Some(GroupRef::Finished {
                                key: key.clone(),
                                rep_run_id: rep_id.clone(),
                            }),
                        });
                    }
                }
                items
            }
            FlowGroupKind::Single => vec![VisibleFlowItem::Run {
                run_idx: entry.rep_idx,
                group_rep: None,
                is_child: false,
                group_ref: None,
            }],
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
            .any(|entry| !matches!(entry.kind, FlowGroupKind::Single))
    }

    fn toggle_selected_flow_group(&mut self) -> bool {
        if self.tab != LedgerTab::Flows {
            return false;
        }
        let Some(sel) = self.flow_selection.as_ref() else {
            return false;
        };
        let items = self.visible_flow_items();
        let Some(item) = items
            .into_iter()
            .find(|it| it.selection(&self.runs).as_ref() == Some(sel))
        else {
            return false;
        };
        match item {
            VisibleFlowItem::WaitingGroup { key, .. } => {
                if self.flow_expanded_waiting_groups.contains(&key) {
                    self.flow_expanded_waiting_groups.remove(&key);
                } else {
                    self.flow_expanded_waiting_groups.insert(key);
                }
                true
            }
            VisibleFlowItem::Run {
                is_child,
                group_ref: Some(GroupRef::Finished { key, rep_run_id }),
                ..
            } => {
                if self.flow_expanded_groups.contains(&key) {
                    self.flow_expanded_groups.remove(&key);
                    if is_child {
                        self.flow_selection = Some(FlowSelection::Run(rep_run_id));
                    }
                } else {
                    self.flow_expanded_groups.insert(key);
                }
                true
            }
            VisibleFlowItem::Run {
                group_ref: Some(GroupRef::Waiting { key }),
                ..
            } => {
                if self.flow_expanded_waiting_groups.contains(&key) {
                    self.flow_expanded_waiting_groups.remove(&key);
                    self.flow_selection = Some(FlowSelection::WaitingGroup(key));
                } else {
                    self.flow_expanded_waiting_groups.insert(key);
                }
                true
            }
            VisibleFlowItem::Run {
                group_ref: None, ..
            } => false,
        }
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
                    self.push_visible_flow_item(&mut out, item, now_ms);
                }
            }
            return out;
        }

        let mut waiting = Vec::new();
        let mut running = Vec::new();
        let mut finished = Vec::new();
        for entry in entries {
            if let Some(r) = self.runs.get(entry.rep_idx) {
                match self.run_section(r) {
                    SectionKind::WaitingForInput => waiting.push(entry),
                    SectionKind::Running => running.push(entry),
                    SectionKind::RecentOutcomes => finished.push(entry),
                }
            }
        }

        let sections: Vec<(SectionKind, usize, Vec<VisibleFlowItem>)> = [
            (SectionKind::WaitingForInput, waiting),
            (SectionKind::Running, running),
            (SectionKind::RecentOutcomes, finished),
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
            .map(|item| self.continuation_lines_for_item(item, now_ms).len())
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
                self.push_visible_flow_item(&mut out, item, now_ms);
            }
        }
        out
    }

    fn push_visible_flow_item(
        &self,
        out: &mut Vec<FlowListRow>,
        item: VisibleFlowItem,
        now_ms: i64,
    ) {
        match item {
            VisibleFlowItem::Run {
                run_idx,
                group_rep,
                is_child,
                ..
            } => {
                out.push(FlowListRow::Run {
                    idx: run_idx,
                    group_rep,
                    is_child,
                });
                self.append_continuations(out, run_idx, is_child, now_ms);
            }
            VisibleFlowItem::WaitingGroup {
                key,
                count,
                expanded,
                latest_updated_ms,
            } => {
                let continuations = self.continuation_lines_for_waiting_group(&key);
                out.push(FlowListRow::WaitingGroup {
                    key,
                    count,
                    expanded,
                    latest_updated_ms,
                });
                for (indent, prefix, text, style) in continuations {
                    out.push(FlowListRow::Continuation {
                        indent,
                        prefix,
                        text,
                        style,
                    });
                }
            }
        }
    }

    fn append_continuations(
        &self,
        out: &mut Vec<FlowListRow>,
        idx: usize,
        is_child: bool,
        now_ms: i64,
    ) {
        for (indent, prefix, text, style) in self.continuation_lines_for(idx, is_child, now_ms) {
            out.push(FlowListRow::Continuation {
                indent,
                prefix,
                text,
                style,
            });
        }
    }

    fn continuation_lines_for_item(
        &self,
        item: &VisibleFlowItem,
        now_ms: i64,
    ) -> Vec<(usize, &'static str, String, ContinuationStyle)> {
        match item {
            VisibleFlowItem::Run {
                run_idx, is_child, ..
            } => self.continuation_lines_for(*run_idx, *is_child, now_ms),
            VisibleFlowItem::WaitingGroup { key, .. } => {
                self.continuation_lines_for_waiting_group(key)
            }
        }
    }

    fn preview_continuation_lines(
        &self,
        indent: usize,
        lines: &[InlinePreviewLine],
    ) -> Vec<(usize, &'static str, String, ContinuationStyle)> {
        if self.last_render_width == 0 {
            return Vec::new();
        }
        let marker_w = 2;
        let wrap_w = self
            .last_render_width
            .saturating_sub(indent + marker_w)
            .max(1);
        let mut out = Vec::new();
        for line in lines {
            let style = ContinuationStyle::Preview {
                cursor: line.cursor,
                selected: line.selected,
            };
            let wrapped = if line.text.trim().is_empty() {
                vec![String::new()]
            } else {
                wrap_words(&line.text, wrap_w)
            };
            for (i, seg) in wrapped.into_iter().enumerate() {
                let prefix = if i == 0 && line.cursor {
                    "▸ "
                } else if line.selected {
                    "▎ "
                } else {
                    "  "
                };
                out.push((indent, prefix, seg, style));
            }
        }
        out
    }

    fn continuation_lines_for_waiting_group(
        &self,
        key: &WaitingGroupKey,
    ) -> Vec<(usize, &'static str, String, ContinuationStyle)> {
        let is_selected = matches!(
            self.flow_selection,
            Some(FlowSelection::WaitingGroup(ref sel_key)) if sel_key == key
        );
        if !is_selected {
            return Vec::new();
        }
        let Some(ref preview_lines) = self.inline_preview else {
            return Vec::new();
        };
        let indent = RunStatus::Waiting.glyph().width() + 1;
        self.preview_continuation_lines(indent, preview_lines)
    }

    fn continuation_lines_for(
        &self,
        idx: usize,
        is_child: bool,
        now_ms: i64,
    ) -> Vec<(usize, &'static str, String, ContinuationStyle)> {
        let Some(run) = self.runs.get(idx) else {
            return Vec::new();
        };
        let is_selected = matches!(
            self.flow_selection,
            Some(FlowSelection::Run(ref id)) if id == &run.run_id
        );
        if !is_selected {
            return Vec::new();
        }
        if let Some(ref preview_lines) = self.inline_preview {
            let indent = (if is_child { 2 } else { 0 }) + run.status.glyph().width() + 1;
            return self.preview_continuation_lines(indent, preview_lines);
        }
        let mut out = Vec::new();
        if run.status == RunStatus::Waiting && !self.is_answered(run) {
            let (_, indent, lines) =
                self.flow_question_lines(run, self.last_render_width, now_ms);
            out.extend(
                lines
                    .into_iter()
                    .skip(1)
                    .map(|t| (indent, "", t, ContinuationStyle::Warning)),
            );
        }
        if !self.can_show_inline_path(run, self.last_render_width, now_ms) {
            let extra = self.extras.get(&run.run_id);
            let path = format_executed_path(extra.and_then(|e| e.executed.as_deref()));
            if !path.is_empty() && self.last_render_width > 0 {
                let indent = (if is_child { 2 } else { 0 }) + run.status.glyph().width() + 1;
                let wrap_w = self.last_render_width.saturating_sub(indent).max(1);
                for line in wrap_words(&path, wrap_w) {
                    out.push((indent, "", line, ContinuationStyle::Muted));
                }
            }
        }
        out
    }
~~~

## Edit 8: Update reconcile_flow_selection for waiting groups and run-ID preservation
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn reconcile_flow_selection(&mut self) {
        let entries = self.build_flow_entries();
        if entries.is_empty() {
            self.flow_selection = None;
            self.flow_scroll = 0;
            return;
        }
        match self.flow_selection.clone() {
            Some(FlowSelection::Run(cur_id)) => {
                for entry in &entries {
                    if entry
                        .members
                        .iter()
                        .any(|&idx| self.runs.get(idx).map(|r| &r.run_id) == Some(&cur_id))
                    {
                        match &entry.kind {
                            FlowGroupKind::FinishedGroup(key)
                                if self.runs.get(entry.rep_idx).map(|r| &r.run_id)
                                    != Some(&cur_id) =>
                            {
                                self.flow_expanded_groups.insert(key.clone());
                            }
                            FlowGroupKind::WaitingGroup(key) => {
                                self.flow_expanded_waiting_groups.insert(key.clone());
                            }
                            _ => {}
                        }
                        return;
                    }
                }
            }
            Some(FlowSelection::WaitingGroup(ref cur_key)) => {
                if entries.iter().any(|e| {
                    matches!(&e.kind, FlowGroupKind::WaitingGroup(k) if k == cur_key)
                }) {
                    return;
                }
                if let Some(entry) = entries.iter().find(|e| {
                    self.runs
                        .get(e.rep_idx)
                        .is_some_and(|r| self.waiting_group_key(r).as_ref() == Some(cur_key))
                }) && let Some(r) = self.runs.get(entry.rep_idx)
                {
                    self.flow_selection = Some(FlowSelection::Run(r.run_id.clone()));
                    return;
                }
            }
            None => {}
        }
        if let Some(first) = entries.first() {
            self.flow_selection = match &first.kind {
                FlowGroupKind::WaitingGroup(key) => Some(FlowSelection::WaitingGroup(key.clone())),
                FlowGroupKind::FinishedGroup(_) | FlowGroupKind::Single => self
                    .runs
                    .get(first.rep_idx)
                    .map(|r| FlowSelection::Run(r.run_id.clone())),
            };
        }
    }
~~~

## Edit 9: Update move_selection, jump_start, and jump_end for FlowSelection
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn move_selection(&mut self, delta: isize) -> bool {
        match self.tab {
            LedgerTab::Flows => {
                let items = self.visible_flow_items();
                if items.is_empty() {
                    return false;
                }
                let cur_pos = self
                    .flow_selection
                    .as_ref()
                    .and_then(|sel| {
                        items
                            .iter()
                            .position(|it| it.selection(&self.runs).as_ref() == Some(sel))
                    })
                    .unwrap_or(0);
                let max_pos = items.len().saturating_sub(1);
                let new_pos = (cur_pos as isize + delta).clamp(0, max_pos as isize) as usize;
                let new_sel = items[new_pos].selection(&self.runs);
                if new_sel != self.flow_selection {
                    self.flow_selection = new_sel;
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
                let items = self.visible_flow_items();
                let first_sel = items.first().and_then(|it| it.selection(&self.runs));
                if first_sel.is_some() && first_sel != self.flow_selection {
                    self.flow_selection = first_sel;
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
                let items = self.visible_flow_items();
                let last_sel = items.last().and_then(|it| it.selection(&self.runs));
                if last_sel.is_some() && last_sel != self.flow_selection {
                    self.flow_selection = last_sel;
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
~~~

## Edit 10: Expand waiting group on Enter without opening a member
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
            KeyCode::Enter => match self.tab {
                LedgerTab::Flows => {
                    if let Some(FlowSelection::WaitingGroup(key)) = self.flow_selection.clone() {
                        if self.flow_expanded_waiting_groups.insert(key) {
                            LedgerOutcome::Changed
                        } else {
                            LedgerOutcome::Unchanged
                        }
                    } else if let Some(run) = self.selected_run() {
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
~~~

## Edit 11: Update compute_overflow_counter for FlowSelection and waiting groups
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                let items = self.visible_flow_items();
                let total = items.len();
                if total == 0 {
                    return None;
                }
                let pos = self
                    .flow_selection
                    .as_ref()
                    .and_then(|sel| {
                        items
                            .iter()
                            .position(|it| it.selection(&self.runs).as_ref() == Some(sel))
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
~~~

## Edit 12: Render waiting group rows, inline preview cursor/range styles, and cursor scroll
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

        // Find visual row index of the selected run or waiting group
        let selected_visual_idx = self.flow_selection.as_ref().and_then(|sel| {
            rows.iter().position(|row| match (row, sel) {
                (FlowListRow::Run { idx, .. }, FlowSelection::Run(sel_id)) => {
                    self.runs.get(*idx).map(|r| &r.run_id) == Some(sel_id)
                }
                (
                    FlowListRow::WaitingGroup { key, .. },
                    FlowSelection::WaitingGroup(sel_key),
                ) => key == sel_key,
                _ => false,
            })
        });

        // Adjust scroll to keep selection (and any focused inline preview cursor) visible.
        if let Some(sel_v) = selected_visual_idx {
            let mut sel_end = sel_v + 1;
            let mut cursor_range: Option<(usize, usize)> = None;
            while let Some(FlowListRow::Continuation { style, .. }) = rows.get(sel_end) {
                if matches!(style, ContinuationStyle::Preview { cursor: true, .. }) {
                    cursor_range = match cursor_range {
                        None => Some((sel_end, sel_end + 1)),
                        Some((start, _)) => Some((start, sel_end + 1)),
                    };
                }
                sel_end += 1;
            }
            let sel_h = sel_end - sel_v;
            if let Some((cur_start, cur_end)) = cursor_range {
                if sel_h <= list_h {
                    if sel_v < self.flow_scroll {
                        self.flow_scroll = sel_v;
                        if sel_v > 0
                            && matches!(rows.get(sel_v - 1), Some(FlowListRow::Header { .. }))
                            && list_h > sel_h
                        {
                            self.flow_scroll = sel_v - 1;
                        }
                    } else if sel_end > self.flow_scroll + list_h {
                        self.flow_scroll = sel_end.saturating_sub(list_h).min(sel_v);
                    }
                } else if cur_start < self.flow_scroll {
                    self.flow_scroll = cur_start;
                } else if cur_end > self.flow_scroll + list_h {
                    self.flow_scroll = cur_end.saturating_sub(list_h).min(cur_start);
                }
            } else if sel_v < self.flow_scroll {
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
                        let is_selected = matches!(
                            self.flow_selection,
                            Some(FlowSelection::Run(ref id)) if id == &run.run_id
                        );
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
                FlowListRow::WaitingGroup {
                    key,
                    count,
                    expanded,
                    latest_updated_ms,
                } => {
                    let is_selected = matches!(
                        self.flow_selection,
                        Some(FlowSelection::WaitingGroup(ref sel_key)) if sel_key == key
                    );
                    self.render_waiting_group_row(
                        buf,
                        row_rect,
                        key,
                        *count,
                        *expanded,
                        *latest_updated_ms,
                        is_selected,
                        theme,
                        now_ms,
                    );
                }
                FlowListRow::Continuation {
                    indent,
                    prefix,
                    text,
                    style,
                } => {
                    buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
                    match style {
                        ContinuationStyle::Warning => {
                            let content = format!("{}{}", " ".repeat(*indent), text);
                            let line = Line::from(Span::styled(
                                truncate_str(&content, row_rect.width as usize),
                                Style::default()
                                    .fg(theme.warning)
                                    .add_modifier(Modifier::BOLD),
                            ));
                            buf.set_line(row_rect.x, row_rect.y, &line, row_rect.width);
                        }
                        ContinuationStyle::Muted => {
                            let content = format!("{}{}", " ".repeat(*indent), text);
                            let line = Line::from(Span::styled(
                                truncate_str(&content, row_rect.width as usize),
                                Style::default()
                                    .fg(theme.gray)
                                    .add_modifier(Modifier::BOLD),
                            ));
                            buf.set_line(row_rect.x, row_rect.y, &line, row_rect.width);
                        }
                        ContinuationStyle::Preview { cursor, selected } => {
                            let w = row_rect.width as usize;
                            let pad_str = " ".repeat((*indent).min(w));
                            let rem_after_pad = w.saturating_sub(pad_str.width());
                            let prefix_cut = truncate_str(prefix, rem_after_pad);
                            let rem_after_prefix =
                                rem_after_pad.saturating_sub(prefix_cut.width());
                            let text_cut = truncate_str(text, rem_after_prefix);

                            let prefix_style = if *cursor && *selected {
                                Style::default()
                                    .fg(theme.command)
                                    .add_modifier(Modifier::BOLD)
                            } else if *cursor {
                                Style::default()
                                    .fg(theme.accent_assistant)
                                    .add_modifier(Modifier::BOLD)
                            } else if *selected {
                                Style::default()
                                    .fg(theme.command)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.gray_dim)
                            };

                            let text_style = if *cursor && *selected {
                                Style::default()
                                    .fg(theme.text_primary)
                                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                            } else if *cursor {
                                Style::default()
                                    .fg(theme.text_primary)
                                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                            } else if *selected {
                                Style::default()
                                    .fg(theme.text_primary)
                                    .add_modifier(Modifier::REVERSED)
                            } else {
                                Style::default().fg(theme.gray)
                            };

                            let line = Line::from(vec![
                                Span::raw(pad_str),
                                Span::styled(prefix_cut, prefix_style),
                                Span::styled(text_cut, text_style),
                            ]);
                            buf.set_line(row_rect.x, row_rect.y, &line, row_rect.width);
                        }
                    }
                }
            }
        }
    }
~~~

## Edit 13: Suppress duplicated row question/path under inline preview and add render_waiting_group_row
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

        // When the modal supplies an inline preview for this selected row, avoid duplicating
        // the gate question or inline executed path on the row itself.
        let preview_replaces_row_detail = selected && self.inline_preview.is_some();
        let (raw_state_text, state_color) =
            if preview_replaces_row_detail && run.status == RunStatus::Waiting && !answered {
                let wait_target = run.waiting_on.as_deref().unwrap_or("checkpoint");
                (format!("waiting: {wait_target}"), theme.warning)
            } else {
                self.flow_state_text(run, extra, stalled, theme)
            };

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

        let wrap_selected_waiting = !preview_replaces_row_detail
            && selected
            && run.status == RunStatus::Waiting
            && !answered
            && !raw_state_text.is_empty();
        let executed_path = format_executed_path(extra.and_then(|e| e.executed.as_deref()));
        let inline_path_available = !preview_replaces_row_detail
            && show_inline_path
            && !executed_path.is_empty()
            && flex_budget > 17;
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

    #[allow(clippy::too_many_arguments)]
    fn render_waiting_group_row(
        &self,
        buf: &mut Buffer,
        row: Rect,
        key: &WaitingGroupKey,
        count: usize,
        expanded: bool,
        latest_updated_ms: i64,
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

        let glyph = RunStatus::Waiting.glyph();
        let marker = if expanded { "▾ " } else { "▸ " };
        let count_str = format!("{count} waiting");
        let age_str = if w >= 64 {
            fmt_age(now_ms.saturating_sub(latest_updated_ms))
        } else {
            String::new()
        };

        let prefix_w = glyph.width() + 1;
        let age_col_w = if !age_str.is_empty() {
            1 + age_str.width()
        } else {
            0
        };
        let avail = w.saturating_sub(prefix_w + age_col_w);

        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            format!("{glyph} "),
            Style::default().fg(theme.warning).add_modifier(base_mod),
        )];

        if avail > count_str.width() + marker.width() + 1 {
            let max_title = avail - 1 - count_str.width();
            let longest_title = self
                .runs
                .iter()
                .map(|r| r.title.width().max(r.flow_name.width()))
                .max()
                .unwrap_or(0)
                + 2;
            let title_col = longest_title.clamp(16.min(max_title), max_title);
            let name_budget = title_col.saturating_sub(marker.width());
            let label = truncate_str(&key.flow_name, name_budget);
            let used_w = marker.width() + label.width();
            let pad = title_col.saturating_sub(used_w);

            spans.push(Span::styled(
                marker.to_string(),
                Style::default().fg(theme.gray).add_modifier(base_mod),
            ));
            if !label.is_empty() {
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(theme.text_primary)
                        .add_modifier(base_mod),
                ));
            }
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                count_str,
                Style::default().fg(theme.warning).add_modifier(base_mod),
            ));
        } else {
            let combined = format!("{marker}{} {count_str}", key.flow_name);
            let cut = truncate_str(&combined, avail);
            if !cut.is_empty() {
                spans.push(Span::styled(
                    cut,
                    Style::default()
                        .fg(theme.text_primary)
                        .add_modifier(base_mod),
                ));
            }
        }

        if !age_str.is_empty() {
            let left_w: usize = spans.iter().map(|s| s.content.width()).sum();
            let right_w = 1 + age_str.width();
            if left_w + right_w <= w {
                let gap = w - left_w - right_w;
                if gap > 0 {
                    spans.push(Span::raw(" ".repeat(gap)));
                }
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    age_str,
                    Style::default().fg(theme.gray).add_modifier(base_mod),
                ));
            }
        }

        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, row.width);
    }
~~~

## Edit 14: Update RowExtra literals in probe_title_state_age_render_at_live_width
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
            },
        );
        state.set_extras(extras);
~~~

## Edit 15: Update stalled_running test for Running section and omitted empty sections
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                flow_path: None,
            },
        );
        state.set_extras(extras);

        // Stalled run is not considered live, but remains under Running with its stalled label.
        assert!(!state.any_live());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        let area_ = buf.area;
        state.render(&mut buf, area_, &theme, 10_000);
        let text = all_text(&buf);
        assert!(!text.contains("Waiting for input"), "{text}");
        assert!(text.contains("Running 1"), "{text}");
        assert!(text.contains("Recent outcomes 1"), "{text}");
        assert!(text.contains("⊘ Stalled Run"));
        assert!(text.contains("stalled at deploy_step"));
    }
~~~

## Edit 16: Update section labels and RowExtra literals in render/answered tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                flow_path: None,
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
        assert!(full.contains("Waiting for input 1"));
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
                    flow_path: None,
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
                        flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
            },
        );
        state.set_extras(extras);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 14));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let full = all_text(&buf);
        assert!(full.contains("1 needs you"));
        assert!(full.contains("Waiting for input 1"));
        assert!(full.contains("Recent outcomes 2"));
        let fin_pos = full.find("Recent outcomes 2").unwrap_or(0);
        let old_pos = full.find("Old Gate").unwrap_or(0);
        assert!(old_pos > fin_pos);
    }
~~~

## Edit 17: Update section labels in finished_group_folds_expands_navigates_and_keeps_pending_individual
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
        // Render folded state: FINISHED count is in units of runs (3 grouped + 2 empty-name = 5).
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let folded_text = all_text(&buf);
        assert!(folded_text.contains("NEEDS YOU 1"), "{folded_text}");
        assert!(folded_text.contains("FINISHED 5"), "{folded_text}");
        assert!(folded_text.contains("▸"), "{folded_text}");
        assert!(folded_text.contains("(3)"), "{folded_text}");
~~~
NEW:
~~~rust
        // Render folded state: Recent outcomes count is in units of runs (3 grouped + 2 empty-name = 5).
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let folded_text = all_text(&buf);
        assert!(folded_text.contains("Waiting for input 1"), "{folded_text}");
        assert!(folded_text.contains("Recent outcomes 5"), "{folded_text}");
        assert!(folded_text.contains("▸"), "{folded_text}");
        assert!(folded_text.contains("(3)"), "{folded_text}");
~~~

## Edit 18: Update stalled/answered individual test for Running section and RowExtra flow_path
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                flow_path: None,
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
                flow_path: None,
            },
        );
        // Extras first so the initial selection sees r-ans as already answered.
        state.set_extras(extras);
        state.set_runs(vec![r1, r2, stalled, answered, next]);

        // r-stalled stays under Running; r-next, r-ans, and the r2/r1 group are under Recent outcomes.
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let text = all_text(&buf);
        assert!(!text.contains("Waiting for input"), "{text}");
        assert!(text.contains("Running 1"), "{text}");
        assert!(text.contains("Recent outcomes 4"), "{text}");
        assert!(text.contains("(2)"), "{text}");

        // Walk down: r-stalled (Running) -> r-next -> r-ans -> r2 (group).
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-stalled"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Unchanged);

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-next"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Unchanged);

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r-ans"));
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Unchanged);

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
        assert!(state.selected_run_is_finished_group());
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Changed);
        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));
        assert!(!state.selected_run_is_finished_group());
    }
~~~

## Edit 19: Update RowExtra literals in executed_count and narrow_width tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                flow_path: None,
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
                flow_path: None,
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
                    flow_path: None,
                },
            );
        }
~~~

## Edit 20: Update remaining attached tests and add waiting-group and inline-preview tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                flow_path: None,
            },
        );
        state.set_extras(extras_unanswered.clone());

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 12));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 10_000);
        let full = all_text(&buf);
        assert!(full.contains("1 needs you"));
        assert!(full.contains("Waiting for input 1"));

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
                flow_path: None,
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
        assert!(full3.contains("Waiting for input 1"));
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
                flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
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
                flow_path: None,
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

    #[test]
    fn waiting_groups_fold_ship_and_mixed_talk_expand_navigate_and_survive_refresh() {
        let now_ms = 10_000_000_000i64;
        let five_days_ms = 5 * 24 * 60 * 60 * 1_000i64;
        let mut runs = vec![
            make_named_run(
                "ship-old",
                "ship: slugify",
                "ship old",
                RunStatus::Waiting,
                now_ms - five_days_ms - 5_000,
                now_ms - five_days_ms,
            ),
            make_named_run("ship-2", "ship: slugify", "ship 2", RunStatus::Waiting, now_ms - 40_000, now_ms - 40_000),
            make_named_run("ship-3", "ship: slugify", "ship 3", RunStatus::Waiting, now_ms - 30_000, now_ms - 30_000),
            make_named_run("ship-4", "ship: slugify", "ship 4", RunStatus::Waiting, now_ms - 20_000, now_ms - 20_000),
            make_named_run("ship-5", "ship: slugify", "ship 5", RunStatus::Waiting, now_ms - 10_000, now_ms - 10_000),
            make_named_run("mix-1", "mixed-talk", "mix 1", RunStatus::Waiting, now_ms - 25_000, now_ms - 25_000),
            make_named_run("mix-2", "mixed-talk", "mix 2", RunStatus::Waiting, now_ms - 15_000, now_ms - 15_000),
        ];

        let mut state = LedgerState::new();
        state.set_runs(runs.clone());

        // Top selection is the newest waiting group; no run is selected, no copy on group header.
        assert_eq!(state.selected_waiting_group(), Some(("ship: slugify", 5)));
        assert!(state.selected_run().is_none());
        assert!(!state.selected_run_is_finished_group());
        assert_eq!(state.handle_key(&key(KeyCode::Char('y'))), LedgerOutcome::Unchanged);

        // Render collapsed state: two groups, section count in units of runs, empty sections omitted.
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), now_ms);
        let text = all_text(&buf);
        assert!(text.contains("7 needs you"), "{text}");
        assert!(text.contains("Waiting for input 7"), "{text}");
        assert!(!text.contains("Running"), "{text}");
        assert!(!text.contains("Recent outcomes"), "{text}");
        assert!(text.contains("ship: slugify"), "{text}");
        assert!(text.contains("5 waiting"), "{text}");
        assert!(text.contains("mixed-talk"), "{text}");
        assert!(text.contains("2 waiting"), "{text}");

        // Enter on waiting group expands it without opening a member; second Enter is a no-op.
        assert_eq!(state.handle_key(&key(KeyCode::Enter)), LedgerOutcome::Changed);
        assert_eq!(state.selected_waiting_group(), Some(("ship: slugify", 5)));
        assert_eq!(state.handle_key(&key(KeyCode::Enter)), LedgerOutcome::Unchanged);

        // Walk down through all 5 members newest-first, reaching the 5-day-old wait.
        let expected_ids = ["ship-5", "ship-4", "ship-3", "ship-2", "ship-old"];
        for expected in expected_ids {
            assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Changed);
            assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some(expected));
        }
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenRun(PathBuf::from("/tmp/runs/ship-old"))
        );

        // Individual run ID survives a refresh when a newer member arrives.
        runs.push(make_named_run(
            "ship-6",
            "ship: slugify",
            "ship 6",
            RunStatus::Waiting,
            now_ms - 5_000,
            now_ms - 5_000,
        ));
        state.set_runs(runs);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("ship-old"));

        // Explicit Space collapse on a child returns selection to the waiting group row.
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), LedgerOutcome::Changed);
        assert_eq!(state.selected_waiting_group(), Some(("ship: slugify", 6)));
        assert!(state.selected_run().is_none());

        // Next row is the second collapsed waiting group.
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Changed);
        assert_eq!(state.selected_waiting_group(), Some(("mixed-talk", 2)));

        // Recent order unfolds waits so they can be browsed individually.
        assert_eq!(state.handle_key(&key(KeyCode::Char('s'))), LedgerOutcome::Changed);
        assert!(state.selected_waiting_group().is_none());
        assert!(state.selected_run().is_some());
    }

    #[test]
    fn waiting_groups_separate_by_source_root_and_recorded_flow_path() {
        let mut r_proj_a1 = make_named_run("a1", "ship", "Ship A1", RunStatus::Waiting, 1_000, 5_000);
        let mut r_proj_a2 = make_named_run("a2", "ship", "Ship A2", RunStatus::Waiting, 2_000, 6_000);
        let mut r_proj_b = make_named_run("b1", "ship", "Ship B1", RunStatus::Waiting, 3_000, 4_000);
        let mut r_missing_a1 = make_named_run("m1", "ship", "Ship M1", RunStatus::Waiting, 1_000, 3_000);
        let mut r_missing_a2 = make_named_run("m2", "ship", "Ship M2", RunStatus::Waiting, 1_500, 2_500);
        let mut r_missing_other_root =
            make_named_run("m3", "ship", "Ship M3", RunStatus::Waiting, 1_000, 1_000);

        r_proj_a1.dir = PathBuf::from("/tmp/root-a/a1");
        r_proj_a2.dir = PathBuf::from("/tmp/root-a/a2");
        r_proj_b.dir = PathBuf::from("/tmp/root-a/b1");
        r_missing_a1.dir = PathBuf::from("/tmp/root-a/m1");
        r_missing_a2.dir = PathBuf::from("/tmp/root-a/m2");
        r_missing_other_root.dir = PathBuf::from("/tmp/root-b/m3");

        let mut extras = HashMap::new();
        for (id, path) in [
            ("a1", Some("/proj-a/ship.yaml")),
            ("a2", Some("/proj-a/ship.yaml")),
            ("b1", Some("/proj-b/ship.yaml")),
            ("m1", None),
            ("m2", None),
            ("m3", None),
        ] {
            extras.insert(
                id.to_string(),
                RowExtra {
                    flow_path: path.map(PathBuf::from),
                    ..RowExtra::default()
                },
            );
        }

        let mut state = LedgerState::new();
        state.set_extras(extras);
        state.set_runs(vec![
            r_proj_a1,
            r_proj_a2,
            r_proj_b,
            r_missing_a1,
            r_missing_a2,
            r_missing_other_root,
        ]);

        // Order by newest updated_ms:
        // 1. WaitingGroup (proj-a, 2 members, updated 6000)
        // 2. Single run b1 (proj-b, updated 4000)
        // 3. WaitingGroup (missing path in root-a, 2 members, updated 3000)
        // 4. Single run m3 (missing path in root-b, updated 1000)
        assert_eq!(state.selected_waiting_group(), Some(("ship", 2)));
        assert!(state.selected_run().is_none());

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("b1"));
        assert!(state.selected_waiting_group().is_none());

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_waiting_group(), Some(("ship", 2)));
        assert!(state.selected_run().is_none());

        state.handle_key(&key(KeyCode::Down));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("m3"));
        assert!(state.selected_waiting_group().is_none());
    }

    #[test]
    fn five_unanswered_and_two_answered_native_waits_group_as_five_with_two_under_recent_outcomes() {
        let mut runs = Vec::new();
        let mut extras = HashMap::new();
        for i in 1..=5 {
            let id = format!("w-{i}");
            runs.push(make_named_run(
                &id,
                "ship: slugify",
                &format!("Ship {i}"),
                RunStatus::Waiting,
                i * 1_000,
                i * 1_000,
            ));
            extras.insert(
                id,
                RowExtra {
                    rail: vec![NodeState::Waiting],
                    question: Some(format!("Ship {i}?")),
                    ..RowExtra::default()
                },
            );
        }
        for (i, choice) in [(6, "Accept"), (7, "Retry")] {
            let wait_id = format!("w-{i}");
            let cont_id = format!("cont-{i}");
            runs.push(make_named_run(
                &wait_id,
                "ship: slugify",
                &format!("Answered {i}"),
                RunStatus::Waiting,
                i * 1_000,
                i * 1_000,
            ));
            runs.push(make_named_run(
                &cont_id,
                "ship-next",
                &format!("Continuation {i}"),
                RunStatus::Completed,
                i * 1_000 + 500,
                i * 1_000 + 500,
            ));
            extras.insert(
                cont_id,
                RowExtra {
                    answers: Some(GateAnswer {
                        run_id: wait_id,
                        choice: choice.to_string(),
                    }),
                    ..RowExtra::default()
                },
            );
        }

        let mut state = LedgerState::new();
        state.set_extras(extras);
        state.set_runs(runs);

        assert_eq!(state.selected_waiting_group(), Some(("ship: slugify", 5)));

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 16));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 20_000);
        let text = all_text(&buf);
        assert!(text.contains("5 needs you"), "{text}");
        assert!(text.contains("Waiting for input 5"), "{text}");
        assert!(text.contains("5 waiting"), "{text}");
        assert!(text.contains("Recent outcomes 4"), "{text}");
        let recent_pos = text.find("Recent outcomes 4").expect("recent outcomes header");
        let ans6_pos = text.find("answered: Accept").expect("first answered row");
        let ans7_pos = text.find("answered: Retry").expect("second answered row");
        assert!(ans6_pos > recent_pos, "{text}");
        assert!(ans7_pos > recent_pos, "{text}");
    }

    #[test]
    fn inline_preview_replaces_question_and_path_renders_cursor_wrapping_and_scrolls() {
        let mut state = LedgerState::new();
        let wait = make_named_run("r-wait", "ship", "Ship Gate", RunStatus::Waiting, 1_000, 2_000);
        let mut extras = HashMap::new();
        extras.insert(
            "r-wait".to_string(),
            RowExtra {
                rail: vec![NodeState::Done(Outcome::Ok), NodeState::Waiting],
                executed: Some(vec![
                    ("plan".to_string(), Outcome::Ok),
                    ("cargo_check".to_string(), Outcome::Ok),
                ]),
                question: Some("Ship to prod now?".to_string()),
                ..RowExtra::default()
            },
        );
        state.set_extras(extras);
        state.set_runs(vec![wait]);

        // Supply inline preview with question, path, focused wrapped cursor line, and selected line.
        let long_cursor = "Diagnostic line alpha beta gamma delta epsilon zeta eta theta iota kappa";
        state.set_inline_preview(Some(vec![
            InlinePreviewLine {
                text: "Ship to prod now?".to_string(),
                cursor: false,
                selected: false,
            },
            InlinePreviewLine {
                text: long_cursor.to_string(),
                cursor: true,
                selected: false,
            },
            InlinePreviewLine {
                text: "✓ plan → ✓ cargo_check".to_string(),
                cursor: false,
                selected: true,
            },
        ]));

        let mut buf = Buffer::empty(Rect::new(0, 0, 56, 14));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 10_000);
        let text = all_text(&buf);
        // Question and executed path appear only once (not duplicated on the run row).
        assert_eq!(text.matches("Ship to prod now?").count(), 1, "{text}");
        assert_eq!(text.matches("cargo_check").count(), 1, "{text}");
        // Focused cursor marker and wrapped words are visible.
        assert!(text.contains("▸ Diagnostic"), "{text}");
        assert!(text.contains("kappa"), "{text}");
        assert!(text.contains("▎ ✓ plan"), "{text}");

        // Tall preview exceeding viewport scrolls focused cursor into view; clearing cursor
        // keeps the selected row reachable.
        let mut tall: Vec<InlinePreviewLine> = (0..12)
            .map(|i| InlinePreviewLine {
                text: format!("preview-line-{i:02}"),
                cursor: i == 11,
                selected: false,
            })
            .collect();
        state.set_inline_preview(Some(tall.clone()));
        let mut small_buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        let small_area = small_buf.area;
        state.render(&mut small_buf, small_area, &Theme::default(), 10_000);
        let focused_text = all_text(&small_buf);
        assert!(focused_text.contains("▸ preview-line-11"), "{focused_text}");

        // Unfocus preview cursor -> selected row returns into view even though preview is tall.
        tall[11].cursor = false;
        state.set_inline_preview(Some(tall));
        let mut unfocused_buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        state.render(&mut unfocused_buf, small_area, &Theme::default(), 10_000);
        let unfocused_text = all_text(&unfocused_buf);
        assert!(unfocused_text.contains("Ship Gate"), "{unfocused_text}");

        // Inline preview supplied for a selected waiting group renders below the group row
        // without resolving a member.
        let mut g_state = LedgerState::new();
        g_state.set_runs(vec![
            make_named_run("g1", "ship", "Ship 1", RunStatus::Waiting, 1_000, 1_000),
            make_named_run("g2", "ship", "Ship 2", RunStatus::Waiting, 2_000, 2_000),
        ]);
        g_state.set_inline_preview(Some(vec![InlinePreviewLine {
            text: "2 separate gates; Space reveals them".to_string(),
            cursor: true,
            selected: false,
        }]));
        let mut g_buf = Buffer::empty(Rect::new(0, 0, 72, 10));
        let g_area = g_buf.area;
        g_state.render(&mut g_buf, g_area, &Theme::default(), 10_000);
        let g_text = all_text(&g_buf);
        assert!(g_state.selected_run().is_none());
        assert_eq!(g_state.selected_waiting_group(), Some(("ship", 2)));
        assert!(g_text.contains("2 waiting"), "{g_text}");
        assert!(g_text.contains("▸ 2 separate gates; Space reveals them"), "{g_text}");
    }
}
~~~

## Coordinator follow-ups
- `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` (unattached test lines 2786-2985): add `flow_path: None` (or `..RowExtra::default()`) to any `RowExtra { ... }` struct literals in that range, and update any assertions on old section headers (`"NEEDS YOU"`, `"RUNNING"`, `"FINISHED"`) if present.
- Any other files in the repository outside the attached excerpts that construct `RowExtra { ... }` without `..RowExtra::default()`: add `flow_path: None`.
```