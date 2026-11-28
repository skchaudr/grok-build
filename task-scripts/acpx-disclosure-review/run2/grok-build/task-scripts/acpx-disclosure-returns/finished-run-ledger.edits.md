## Edit 1: Populate executed step list in row_extra
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
    }
}
~~~

## Edit 2: Import HashSet in ledger.rs
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
use std::collections::HashMap;
use std::path::PathBuf;
~~~
NEW:
~~~rust
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
~~~

## Edit 3: Add executed step history to RowExtra
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
/// Per-run facts the parent loads (cached) so the ledger does no I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
    pub answers: Option<GateAnswer>,
}
~~~
NEW:
~~~rust
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

## Edit 4: Add flow group types, expanded-group state, and updated FlowListRow
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

## Edit 5: Add Space shortcut label on Flows tab
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
~~~

## Edit 6: Implement finished-run grouping, row construction, and continuation lines
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

## Edit 7: Preserve selected run ID across refreshes even when a newer group member arrives
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
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

## Edit 8: Bind Space to toggle finished-run groups and copy /open <run_id> on Flows
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
~~~
NEW:
~~~rust
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
~~~

## Edit 9: Render grouped rows and styled continuation lines in render_flows
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
~~~

## Edit 10: Render rail, executed step count, executed path, and group markers in render_flow_row and flow_question_lines
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
~~~

## Edit 11: Add executed_count_label and format_executed_path helpers and update probe test
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
    buf.set_line(x, y, &line, area.width.saturating_sub(x - area.x));
}

#[cfg(test)]
mod tests {
~~~
NEW:
~~~rust
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
~~~

## Edit 12: Update RowExtra literals and rail assertions in probe_title_state_age_render_at_live_width
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
~~~

## Edit 13: Add executed field to RowExtra literal in stalled_running_run_is_placed_under_finished_with_stalled_at
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
~~~

## Edit 14: Update y copy test and RowExtra literals in existing render tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
            LedgerOutcome::Copy("/open r1".to_string())
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
~~~

## Edit 15: Update RowExtra literals in answered_gate_counts_only_unanswered_and_moves_answered_to_finished and add finished-run grouping and executed-path unit tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
        state.set_runs(vec![r1, r2, stalled, answered, next]);

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
        state.set_extras(extras);

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
~~~

## Coordinator follow-ups
- `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` (outside the attached excerpts): add `executed: None` to any remaining `RowExtra { ... }` test literals so the crate compiles with the new `RowExtra::executed` field.
- `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` (outside the attached excerpts): add the `Space` help-table row for expanding/collapsing finished-run groups on the Flows tab, and add `executed: None` to any `RowExtra { ... }` literals outside the attached excerpts if present.
