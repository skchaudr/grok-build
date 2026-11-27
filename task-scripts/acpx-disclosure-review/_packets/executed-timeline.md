Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For patch tasks, return a complete Git
unified diff against the attached excerpts with exact context and focused
tests. For design tasks, return only the requested Markdown artifact and
use inline code or indented examples inside it. Do not return placeholder code.
Treat the attached excerpts as evidence; lines outside the excerpts are not
available to you. Rust edition 2024, toolchain 1.94.0. No new dependencies,
unsafe, or new production unwrap/expect. Preserve theme styles and keymaps
unless the task explicitly changes them. Do not change gate authority,
workflow state, ledger routing, or release behavior.

=== REPOSITORY CONTEXT (8 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 1-305 of 2758 (repository grok-build)
````
//! Screen for one ACPX flow run: header, status banner, flow graph, and a
//! split below with the foldable stage tree on the left and the reserved
//! detail panel `Rect` on the right.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::acpx::bundle::{
    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, TraceEvent,
    fmt_duration, value_text,
};
use crate::acpx::graph::{Direction, mermaid_source};
use crate::render::line_utils::truncate_str;
use crate::scrollback::blocks::markdown_content::MarkdownContent;
use crate::theme::Theme;
use crate::views::acpx_modal::detail::{DetailTarget, value_inline_summary};
use crate::views::acpx_modal::modal_view::{
    ModalZoom, build_waiting_gate_banner, compute_run_body_layout, format_step_structure_annotation,
    prune_run_for_graph, slice_graph_viewport,
};
use crate::views::traces_fold::{FoldNode, FoldState, FoldTree};
use crate::views::traces_modal::fmt_age;

/// Outcome of handling a key in the run screen.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    Unchanged,
    Changed,
    Back,
    OpenStep(usize),
    Replay(usize),
    Copy(String),
    ScrollDetail(isize),
    Gate { choice: String, flow_file: String },
}

#[derive(Debug, Clone)]
enum NodePayload {
    StepStage {
        step_idx: usize,
        stage_num: usize,
        state: NodeState,
    },
    LiveStage {
        node_id: String,
        stage_num: usize,
        state: NodeState,
    },
    SkippedHeader,
    SkippedNode {
        node_id: String,
        kind: NodeKind,
    },
    Event {
        owner: StageOwner,
        rel_ms: i64,
        kind: String,
        detail: String,
    },
    Result {
        step_idx: usize,
    },
    GateChoice {
        owner: StageOwner,
        choice_idx: usize,
        label: String,
        flow_file: String,
    },
    GateEmpty {
        owner: StageOwner,
    },
}

#[derive(Debug, Clone)]
enum StageOwner {
    Step(usize),
    Live(String),
}

#[derive(Debug, Clone)]
struct GraphCache {
    width: u16,
    lr_src: String,
    td_src: String,
    lr_lines: Vec<Line<'static>>,
    td_lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone)]
struct GateConfirm {
    choice: String,
    flow_file: String,
}

/// Interactive state for the single-run screen.
pub struct RunViewState {
    run: Run,
    node_settings: HashMap<String, AgentSettings>,
    step_settings: HashMap<String, AgentSettings>,
    tree: FoldTree<String>,
    payloads: Vec<NodePayload>,
    fold: FoldState<String>,
    /// Index into `visible` (the output of `tree.visible(&fold)`).
    selected: usize,
    scroll: usize,
    last_pane_height: usize,
    show_graph: bool,
    show_noise: bool,
    detail_rect: Option<Rect>,
    gate_confirm: Option<GateConfirm>,
    graph_cache: Option<GraphCache>,
    answered: Option<String>,
    zoom: ModalZoom,
    graph_scroll: usize,
}

impl RunViewState {
    pub fn new(run: Run) -> Self {
        let mut st = Self {
            run,
            node_settings: HashMap::new(),
            step_settings: HashMap::new(),
            tree: FoldTree { nodes: Vec::new() },
            payloads: Vec::new(),
            fold: FoldState::new(),
            selected: 0,
            scroll: 0,
            last_pane_height: 20,
            show_graph: true,
            show_noise: false,
            detail_rect: None,
            gate_confirm: None,
            graph_cache: None,
            answered: None,
            zoom: ModalZoom::Split,
            graph_scroll: 0,
        };
        st.rebuild_tree();
        st.select_initial_stage();
        st
    }

    pub fn set_settings(
        &mut self,
        nodes: HashMap<String, AgentSettings>,
        steps: HashMap<String, AgentSettings>,
    ) {
        self.node_settings = nodes;
        self.step_settings = steps;
    }

    pub fn set_answered(&mut self, choice: Option<String>) {
        // A confirm prompt opened before the answer arrived must not stay live.
        if choice.is_some() {
            self.gate_confirm = None;
        }
        let changed = self.answered != choice;
        self.answered = choice;
        if changed {
            // Stage glyphs and the graph are drawn from the answer.
            self.sync();
        }
    }

    pub fn answered(&self) -> Option<&str> {
        self.answered.as_deref()
    }

    pub fn detail_target(&self) -> DetailTarget {
        let vis = self.visible();
        let Some(&node_idx) = vis.get(self.selected) else {
            return DetailTarget::Run;
        };
        let Some(payload) = self.payloads.get(node_idx) else {
            return DetailTarget::Run;
        };
        match payload {
            NodePayload::StepStage { step_idx, .. } | NodePayload::Result { step_idx } => {
                DetailTarget::Step(*step_idx)
            }
            NodePayload::LiveStage { node_id, .. } | NodePayload::SkippedNode { node_id, .. } => {
                DetailTarget::Node(node_id.clone())
            }
            NodePayload::Event { owner, .. }
            | NodePayload::GateChoice { owner, .. }
            | NodePayload::GateEmpty { owner } => match owner {
                StageOwner::Step(idx) => DetailTarget::Step(*idx),
                StageOwner::Live(id) => DetailTarget::Node(id.clone()),
            },
            NodePayload::SkippedHeader => DetailTarget::Run,
        }
    }

    pub fn detail_area(&self) -> Option<Rect> {
        self.detail_rect
    }

    pub fn captures_keys(&self) -> bool {
        self.gate_confirm.is_some()
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn run_mut(&mut self) -> &mut Run {
        &mut self.run
    }

    pub fn sync(&mut self) {
        let vis = self.visible();
        let prev_key = vis
            .get(self.selected)
            .and_then(|&i| self.tree.nodes.get(i))
            .map(|n| n.key.clone());

        // Find the nearest earlier root key as fallback.
        let prev_root_keys: Vec<String> = vis
            .iter()
            .take(self.selected + 1)
            .filter_map(|&i| {
                let n = self.tree.nodes.get(i)?;
                (n.depth == 0).then(|| n.key.clone())
            })
            .collect();

        self.rebuild_tree();
        self.fold.retain(&self.tree);
        self.graph_cache = None;

        let new_vis = self.visible();
        if new_vis.is_empty() {
            self.selected = 0;
            return;
        }

        if let Some(ref key) = prev_key
            && let Some(pos) = new_vis
                .iter()
                .position(|&i| self.tree.nodes.get(i).is_some_and(|n| &n.key == key))
        {
            self.selected = pos;
            return;
        }

        for rkey in prev_root_keys.iter().rev() {
            if let Some(pos) = new_vis
                .iter()
                .position(|&i| self.tree.nodes.get(i).is_some_and(|n| &n.key == rkey))
            {
                self.selected = pos;
                return;
            }
        }

        self.selected = self.selected.min(new_vis.len().saturating_sub(1));
    }

    pub fn selected_step(&self) -> Option<usize> {
        match self.detail_target() {
            DetailTarget::Step(idx) => Some(idx),
            DetailTarget::Run | DetailTarget::Node(_) => None,
        }
    }

    pub fn select_step(&mut self, index: usize) {
        let Some(step) = self.run.steps.get(index) else {
            return;
        };
        let target_key = step.attempt_id.clone();
        let vis = self.visible();
        if let Some(pos) = vis
            .iter()
            .position(|&i| self.tree.nodes.get(i).is_some_and(|n| n.key == target_key))
        {
            self.selected = pos;
        }
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut out = vec!["Enter step", "r replay"];
        if self.run.summary.status == RunStatus::Waiting
            && self.answered.is_none()
            && !bundle::gate_choices(&self.run).is_empty()
        {
            out.push("1-9 gate");
        }
        out.extend([
            "Space fold",
            "J/K detail",
            "v graph",
            "m/z zoom",
            "M/Z detail",
            "n noise",
            "y copy",
            "Esc back",
            "? keys",
        ]);
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 400-539 of 2758 (repository grok-build)
````
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.scroll(1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.scroll(-1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.selected != 0 {
                    self.selected = 0;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                let vis = self.visible();
                let last = vis.len().saturating_sub(1);
                if self.selected != last {
                    self.selected = last;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                if let Some(parent_idx) = self.fold.collapse(&self.tree, node_idx) {
                    let new_vis = self.visible();
                    if let Some(pos) = new_vis.iter().position(|&i| i == parent_idx) {
                        self.selected = pos;
                    }
                } else {
                    self.clamp_selected();
                }
                RunOutcome::Changed
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                self.fold.expand(&self.tree, node_idx);
                RunOutcome::Changed
            }
            KeyCode::Char(' ') => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                self.fold.toggle(&self.tree, node_idx);
                self.clamp_selected();
                RunOutcome::Changed
            }
            KeyCode::Char('E') => {
                let vis = self.visible();
                let cur_node = vis.get(self.selected).copied();
                let cur_root = cur_node.map(|i| self.root_of(i));
                self.fold.toggle_all(&self.tree);
                let new_vis = self.visible();
                if let Some(n) = cur_node
                    && let Some(pos) = new_vis.iter().position(|&i| i == n)
                {
                    self.selected = pos;
                } else if let Some(r) = cur_root
                    && let Some(pos) = new_vis.iter().position(|&i| i == r)
                {
                    self.selected = pos;
                } else {
                    self.clamp_selected();
                }
                RunOutcome::Changed
            }
            KeyCode::Char('n') => {
                self.show_noise = !self.show_noise;
                self.sync();
                RunOutcome::Changed
            }
            KeyCode::Char('v') => {
                self.show_graph = !self.show_graph;
                RunOutcome::Changed
            }
            KeyCode::Char('!') => {
                if self.jump_next_failed() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('c') => {
                if self.jump_current_or_waiting() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('[') => {
                if self.jump_stage(-1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(']') => {
                if self.jump_stage(1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('J') => RunOutcome::ScrollDetail(3),
            KeyCode::Char('K') => RunOutcome::ScrollDetail(-3),
            KeyCode::Char('D') => RunOutcome::ScrollDetail(15),
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 700-1045 of 2758 (repository grok-build)
````
            if self.gate_confirm.is_some() {
                self.render_gate_confirm(buf, stages, theme);
            }
        } else if self.gate_confirm.is_some() {
            if let Some(detail) = layout.detail_rect {
                self.render_gate_confirm(buf, detail, theme);
            }
        }
    }

    // ---- Internal tree construction ----

    fn rebuild_tree(&mut self) {
        let mut nodes = Vec::new();
        let mut payloads = Vec::new();

        let is_waiting = self.run.summary.status == RunStatus::Waiting;
        let is_running = self.run.summary.status == RunStatus::Running;
        let waiting_node = self.run.summary.waiting_on.as_deref();
        let current_node = self.run.summary.current_node.as_deref();

        // Find if the waiting node already has a step in `run.steps` (pick the last one).
        let waiting_step_idx: Option<usize> = if is_waiting {
            waiting_node.and_then(|wn| self.run.steps.iter().rposition(|s| s.node_id == wn))
        } else {
            None
        };

        // Determine if we need a live/<node> root:
        // - Running node that has no step record, or is currently active without a finished step for this run
        //   Spec: "a root for a node that is Running but has no step record (key = `"live/<node id>"`).
        //   The WAITING stage is the last root whose node id equals `run.summary.waiting_on`: acpx records
        //   the checkpoint step BEFORE the run goes waiting, so it is normally a step root; use a `live/…`
        //   root for it only when no such step exists"
        let live_root_node: Option<(String, NodeState)> = if is_waiting {
            if waiting_step_idx.is_none() {
                waiting_node.map(|wn| (wn.to_string(), self.run.node_state(wn)))
            } else {
                None
            }
        } else if is_running {
            current_node.and_then(|cn| {
                let has_step = self.run.steps.iter().any(|s| s.node_id == cn);
                if !has_step {
                    Some((cn.to_string(), self.run.node_state(cn)))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let mut stage_num = 0usize;

        // a. One root per step in `run.steps` order
        for (step_idx, step) in self.run.steps.iter().enumerate() {
            stage_num += 1;
            let is_waiting_stage = waiting_step_idx == Some(step_idx);
            let state = if is_waiting_stage && self.answered.is_none() {
                NodeState::Waiting
            } else if is_waiting_stage {
                NodeState::Done(Outcome::Ok)
            } else {
                NodeState::Done(step.outcome)
            };

            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: step.attempt_id.clone(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::StepStage {
                step_idx,
                stage_num,
                state,
            });

            // Trace event children
            let evs = self.run.step_events(&step.attempt_id);
            let start_ms = if step.started_ms > 0 {
                step.started_ms
            } else {
                evs.first().map_or(0, |e| e.at_ms)
            };
            for ev in evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("{}/{}", step.attempt_id, ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Step(step_idx),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            // Result row
            nodes.push(FoldNode {
                key: format!("{}/result", step.attempt_id),
                parent: Some(root_idx),
                depth: 1,
            });
            payloads.push(NodePayload::Result { step_idx });

            // Gate children if this step is the waiting stage
            if is_waiting_stage {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Step(step_idx),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // b. Live root (Running with no step record, or Waiting with no step record)
        if let Some((live_node_id, state)) = live_root_node {
            stage_num += 1;
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: format!("live/{live_node_id}"),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::LiveStage {
                node_id: live_node_id.clone(),
                stage_num,
                state: if state == NodeState::Waiting && self.answered.is_some() {
                    NodeState::Done(Outcome::Ok)
                } else {
                    state
                },
            });

            let step_attempts: HashSet<&str> = self
                .run
                .steps
                .iter()
                .map(|s| s.attempt_id.as_str())
                .collect();
            let live_evs: Vec<&TraceEvent> = self
                .run
                .events
                .iter()
                .filter(|ev| {
                    ev.node_id.as_deref() == Some(live_node_id.as_str())
                        && ev
                            .attempt_id
                            .as_deref()
                            .is_none_or(|a| !step_attempts.contains(a))
                })
                .collect();
            let start_ms = live_evs
                .iter()
                .find(|e| e.kind == "node_started")
                .or_else(|| live_evs.first())
                .map_or(self.run.summary.started_ms, |e| e.at_ms);

            for ev in live_evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("live/{live_node_id}/{}", ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Live(live_node_id.clone()),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            if state == NodeState::Waiting {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Live(live_node_id),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // c. Skipped / NotRun flow nodes
        let skipped: Vec<(String, NodeKind)> = self
            .run
            .flow
            .nodes
            .iter()
            .filter(|n| self.run.node_state(&n.id) == NodeState::NotRun)
            .map(|n| (n.id.clone(), n.kind))
            .collect();
        if !skipped.is_empty() {
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: "skipped".to_string(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::SkippedHeader);
            for (nid, kind) in skipped {
                nodes.push(FoldNode {
                    key: format!("skip/{nid}"),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::SkippedNode { node_id: nid, kind });
            }
        }

        self.tree = FoldTree { nodes };
        self.payloads = payloads;
    }

    fn push_gate_children(
        run: &Run,
        root_idx: usize,
        owner: StageOwner,
        nodes: &mut Vec<FoldNode<String>>,
        payloads: &mut Vec<NodePayload>,
    ) {
        let choices = bundle::gate_choices(run);
        if choices.is_empty() {
            nodes.push(FoldNode {
                key: "gate/empty".to_string(),
                parent: Some(root_idx),
                depth: 1,
            });
            payloads.push(NodePayload::GateEmpty { owner });
        } else {
            for (idx, (label, flow_file)) in choices.into_iter().enumerate() {
                let choice_idx = idx + 1;
                nodes.push(FoldNode {
                    key: format!("gate/{choice_idx}"),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::GateChoice {
                    owner: owner.clone(),
                    choice_idx,
                    label,
                    flow_file,
                });
            }
        }
    }

    fn visible(&self) -> Vec<usize> {
        self.tree.visible(&self.fold)
    }

    fn clamp_selected(&mut self) {
        let vis = self.visible();
        if vis.is_empty() {
            self.selected = 0;
        } else if self.selected >= vis.len() {
            self.selected = vis.len() - 1;
        }
    }

    fn root_of(&self, mut idx: usize) -> usize {
        while let Some(node) = self.tree.nodes.get(idx) {
            if let Some(p) = node.parent {
                idx = p;
            } else {
                break;
            }
        }
        idx
    }

    fn select_initial_stage(&mut self) {
        if self.jump_current_or_waiting() {
            return;
        }
        // Fallback: last stage that ran (a + b roots)
        let vis = self.visible();
        if let Some(pos) = vis.iter().rposition(|&i| {
            matches!(
                self.payloads.get(i),
                Some(NodePayload::StepStage { .. } | NodePayload::LiveStage { .. })
            )
        }) {
            self.selected = pos;
        } else {
            self.selected = 0;
        }
    }

    fn jump_current_or_waiting(&mut self) -> bool {
        let vis = self.visible();
        // Waiting stage first
        if self.run.summary.status == RunStatus::Waiting {
            if let Some(pos) = vis.iter().rposition(|&i| {
                matches!(
                    self.payloads.get(i),
                    Some(
                        NodePayload::StepStage {
                            state: NodeState::Waiting,
                            ..
                        } | NodePayload::LiveStage {
                            state: NodeState::Waiting,
                            ..
                        }
                    )
                )
            }) {
                self.selected = pos;
                return true;
            }
        }
        // Running stage
        if let Some(pos) = vis.iter().rposition(|&i| {
            matches!(
                self.payloads.get(i),
                Some(
                    NodePayload::StepStage {
                        state: NodeState::Running,
                        ..
                    } | NodePayload::LiveStage {
                        state: NodeState::Running,
                        ..
                    }
                )
            )
        }) {
            self.selected = pos;
            return true;
        }
        // Otherwise match current_node or waiting_on against the last matching stage root
        let target_node = self
            .run
            .summary
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 2025-2314 of 2758 (repository grok-build)
````
#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        ActionReceipt, Conversation, FlowDef, FlowEdge, FlowNode, RunSummary,
    };
    use serde_json::json;
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn buf_text(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    fn make_step(
        attempt_id: &str,
        node_id: &str,
        kind: NodeKind,
        outcome: Outcome,
        output: Value,
    ) -> Step {
        Step {
            attempt_id: attempt_id.to_string(),
            node_id: node_id.to_string(),
            kind,
            outcome,
            started_ms: 1_000,
            finished_ms: 2_500,
            prompt_text: None,
            raw_text: None,
            output,
            error: (outcome == Outcome::Failed).then(|| "boom: command failed".to_string()),
            agent_name: (kind == NodeKind::Acp).then(|| "grok-build".to_string()),
            action: (kind == NodeKind::Action).then(|| ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("cargo test".to_string()),
                args: vec![],
                cwd: None,
                exit_code: Some(0),
                duration_ms: Some(1500),
            }),
            conversation: (kind == NodeKind::Acp).then(|| Conversation {
                session_id: "sess-1".to_string(),
                event_start_seq: 1,
                event_end_seq: 5,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn sample_waiting_run(gate_output: Value) -> Run {
        let flow = FlowDef {
            name: "review-flow".to_string(),
            start_at: "plan".to_string(),
            nodes: vec![
                FlowNode {
                    id: "plan".to_string(),
                    kind: NodeKind::Acp,
                    summary: None,
                    is_shell: false,
                    ..Default::default()
                },
                FlowNode {
                    id: "test".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                    ..Default::default()
                },
                FlowNode {
                    id: "gate".to_string(),
                    kind: NodeKind::Checkpoint,
                    summary: None,
                    is_shell: false,
                    ..Default::default()
                },
                FlowNode {
                    id: "deploy".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                    ..Default::default()
                },
            ],
            edges: vec![
                FlowEdge::Simple {
                    from: "plan".to_string(),
                    to: "test".to_string(),
                },
                FlowEdge::Simple {
                    from: "test".to_string(),
                    to: "gate".to_string(),
                },
                FlowEdge::Simple {
                    from: "gate".to_string(),
                    to: "deploy".to_string(),
                },
            ],
            ..Default::default()
        };

        let steps = vec![
            make_step(
                "plan#1",
                "plan",
                NodeKind::Acp,
                Outcome::Ok,
                json!({"route": "ok"}),
            ),
            make_step(
                "test#1",
                "test",
                NodeKind::Action,
                Outcome::Failed,
                Value::Null,
            ),
            make_step(
                "gate#1",
                "gate",
                NodeKind::Checkpoint,
                Outcome::Ok,
                gate_output,
            ),
        ];

        let events = vec![
            TraceEvent {
                seq: 1,
                at_ms: 1_000,
                scope: "node".to_string(),
                kind: "node_started".to_string(),
                node_id: Some("plan".to_string()),
                attempt_id: Some("plan#1".to_string()),
                detail: "starting plan".to_string(),
            },
            TraceEvent {
                seq: 2,
                at_ms: 1_500,
                scope: "node".to_string(),
                kind: "node_heartbeat".to_string(),
                node_id: Some("plan".to_string()),
                attempt_id: Some("plan#1".to_string()),
                detail: "alive".to_string(),
            },
            TraceEvent {
                seq: 3,
                at_ms: 2_500,
                scope: "node".to_string(),
                kind: "node_completed".to_string(),
                node_id: Some("plan".to_string()),
                attempt_id: Some("plan#1".to_string()),
                detail: "ok".to_string(),
            },
        ];

        Run {
            summary: RunSummary {
                run_id: "run-001".to_string(),
                dir: PathBuf::from("/tmp/run-001"),
                flow_name: "review-flow".to_string(),
                title: "PR #42 Review".to_string(),
                status: RunStatus::Waiting,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: Some("gate".to_string()),
                waiting_on: Some("gate".to_string()),
                status_detail: Some("awaiting human approval".to_string()),
                error: None,
            },
            flow,
            steps,
            events,
            input: json!({}),
            sessions: vec![],
        }
    }

    #[test]
    fn initial_selection_starts_on_waiting_stage() {
        let run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
        let st = RunViewState::new(run);
        assert_eq!(st.selected_step(), Some(2));
        assert_eq!(st.detail_target(), DetailTarget::Step(2));
    }

    #[test]
    fn stages_start_expanded_and_hide_heartbeats_until_noise_toggled() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let keys_before: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_before.contains(&"plan#1/1".to_string()));
        assert!(!keys_before.contains(&"plan#1/2".to_string()));
        assert!(keys_before.contains(&"plan#1/3".to_string()));

        assert_eq!(st.handle_key(&key(KeyCode::Char('n'))), RunOutcome::Changed);
        let keys_after: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_after.contains(&"plan#1/2".to_string()));
    }

    #[test]
    fn collapse_on_child_jumps_to_parent() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0);
        assert_eq!(st.selected, 0);
        // Move down onto the first child of plan#1
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.selected, 1);
        assert_eq!(st.detail_target(), DetailTarget::Step(0));
        // Left on a leaf child jumps back to parent root (index 0)
        st.handle_key(&key(KeyCode::Left));
        assert_eq!(st.selected, 0);
        // Left again collapses the root
        st.handle_key(&key(KeyCode::Char('h')));
        assert!(st.fold.is_collapsed(&"plan#1".to_string()));
    }

    #[test]
    fn shift_e_toggles_all_folds() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let expanded_len = st.visible().len();
        st.handle_key(&key(KeyCode::Char('E')));
        let collapsed_len = st.visible().len();
        assert!(collapsed_len < expanded_len);
        // Only root headers remain visible (3 steps + 1 skipped root = 4)
        assert_eq!(collapsed_len, 4);
        st.handle_key(&key(KeyCode::Char('E')));
        assert_eq!(st.visible().len(), expanded_len);
    }

    #[test]
    fn bang_jumps_to_failed_step_and_wraps() {
        let mut run = sample_waiting_run(json!({}));
        run.steps[0].outcome = Outcome::Failed;
        run.steps[1].outcome = Outcome::Failed;
        let mut st = RunViewState::new(run);
        // Starts on step 2 (gate#1)
        assert_eq!(st.selected_step(), Some(2));
        // '!' wraps to step 0
        assert_eq!(st.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(st.selected_step(), Some(0));
        // '!' advances to step 1
        assert_eq!(st.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(st.selected_step(), Some(1));
        // '!' wraps back to step 0
        assert_eq!(st.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(st.selected_step(), Some(0));
    }

    #[test]
    fn c_and_brackets_navigate_stage_headers() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.handle_key(&key(KeyCode::Home));
        assert_eq!(st.selected_step(), Some(0));

        st.handle_key(&key(KeyCode::Char(']')));
        assert_eq!(st.selected_step(), Some(1));

        st.handle_key(&key(KeyCode::Char('[')));
        assert_eq!(st.selected_step(), Some(0));

        st.handle_key(&key(KeyCode::Char('c')));
        assert_eq!(st.selected_step(), Some(2));
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 2365-2558 of 2758 (repository grok-build)
````
        assert_eq!(st.handle_key(&key(KeyCode::Esc)), RunOutcome::Back);
    }

    #[test]
    fn detail_target_for_step_child_live_root_and_skipped_node() {
        let mut run = sample_waiting_run(json!({}));
        run.summary.status = RunStatus::Running;
        run.summary.waiting_on = None;
        run.summary.current_node = Some("deploy".to_string());
        run.events.push(TraceEvent {
            seq: 99,
            at_ms: 4_000,
            scope: "node".to_string(),
            kind: "node_started".to_string(),
            node_id: Some("deploy".to_string()),
            attempt_id: Some("deploy#1".to_string()),
            detail: "deploying".to_string(),
        });
        // Add an extra unreached node so skipped section also exists
        run.flow.nodes.push(FlowNode {
            id: "cleanup".to_string(),
            kind: NodeKind::Action,
            summary: None,
            is_shell: true,
            ..Default::default()
        });

        let mut st = RunViewState::new(run);
        // Initial selection on running live/deploy stage
        assert_eq!(st.detail_target(), DetailTarget::Node("deploy".to_string()));
        // Move to child event of live/deploy
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.detail_target(), DetailTarget::Node("deploy".to_string()));
        // Move to skipped header
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.detail_target(), DetailTarget::Run);
        // Move to skip/cleanup row
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(
            st.detail_target(),
            DetailTarget::Node("cleanup".to_string())
        );
    }

    #[test]
    fn gate_choices_confirmation_flow() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        let vis_keys: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(vis_keys.contains(&"gate/1".to_string()));
        assert!(vis_keys.contains(&"gate/2".to_string()));
        assert!(!st.captures_keys());

        // Press '2' to open confirmation for "reject"
        assert_eq!(st.handle_key(&key(KeyCode::Char('2'))), RunOutcome::Changed);
        assert!(st.captures_keys());

        // Esc closes confirmation box
        assert_eq!(st.handle_key(&key(KeyCode::Esc)), RunOutcome::Changed);
        assert!(!st.captures_keys());

        // Press '2' then Enter confirms the gate
        assert_eq!(st.handle_key(&key(KeyCode::Char('2'))), RunOutcome::Changed);
        assert!(st.captures_keys());
        assert_eq!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::Gate {
                choice: "reject".to_string(),
                flow_file: "fix.flow.ts".to_string(),
            }
        );
        assert!(!st.captures_keys());
    }

    #[test]
    fn waiting_run_without_next_shows_empty_gate_row() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 35));
        let area_ = buf.area;
        st.render(&mut buf, area_, &theme, 10_000);
        let text = buf_text(&buf);
        assert!(text.contains("no next step declared"));
    }

    #[test]
    fn sync_keeps_selection_when_step_is_appended() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(1);
        assert_eq!(st.selected_step(), Some(1));

        st.run_mut().steps.push(make_step(
            "deploy#1",
            "deploy",
            NodeKind::Action,
            Outcome::Ok,
            Value::Null,
        ));
        st.sync();
        assert_eq!(st.selected_step(), Some(1));
    }

    #[test]
    fn banner_shows_the_gate_question_and_falls_back_to_the_node_name() {
        let theme = Theme::default();
        let banner = |detail: Option<&str>| {
            let mut run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
            run.summary.status_detail = detail.map(str::to_string);
            let st = RunViewState::new(run);
            st.build_banner(120, &theme, 10_000)
                .iter()
                .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
                .collect::<String>()
        };
        assert!(banner(Some("land it, or send it back?"))
            .contains("◐ WAITING FOR YOUR ANSWER at gate: land it, or send it back?"));
        // acpx reports the node id when the checkpoint has no question: no repeat.
        let repeated = banner(Some("gate"));
        assert!(repeated.contains("◐ WAITING FOR YOUR ANSWER at gate"));
        assert!(!repeated.contains("gate: gate"));
        let none = banner(None);
        assert!(none.contains("◐ WAITING FOR YOUR ANSWER at gate"));
        assert!(!none.contains("waiting"));
    }

    #[test]
    fn render_wide_160x40_shows_banner_stages_separator_and_clean_detail_area() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 160, 40);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);

        let text = buf_text(&buf);
        assert!(text.contains("PR #42 Review"));
        assert!(text.contains("◐ WAITING FOR YOUR ANSWER at gate: awaiting human approval"));
        assert!(text.contains("✓ plan"));
        assert!(text.contains("✗ test"));
        assert!(text.contains("◐ gate"));
        assert!(text.contains("│"));
        // Skipped section lists NotRun node 'deploy'
        assert!(text.contains("── not run ──"));
        assert!(text.contains("○ deploy"));

        let detail = st.detail_area();
        assert!(detail.is_some());
        let drect = detail.unwrap_or_default();
        assert!(drect.x > 46);
        for y in drect.y..drect.y + drect.height {
            for x in drect.x..drect.x + drect.width {
                assert_eq!(buf[(x, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn step_tree_result_line_is_plain_text_not_raw_json() {
        let mut run = sample_waiting_run(json!({}));
        run.steps[0].output = json!({"verdict": "approve", "findings": []});
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);

        let text = buf_text(&buf);
        let line = text
            .lines()
            .find(|l| l.contains("verdict approve"))
            .unwrap_or_else(|| panic!("no result line in:\n{text}"));
        assert!(line.contains("verdict approve, findings (empty)"), "{line}");
        assert!(!line.contains('{') && !line.contains('"'), "{line}");
    }

    #[test]
    fn step_result_summary_keeps_route_exit_and_error_forms() {
        let theme = Theme::default();
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs` lines 153-225 of 1012 (repository grok-build)
````
/// Return a cloned `Run` whose `flow` has untaken branches pruned by calling
/// `crate::acpx::step_digest::active_branch_nodes`.
pub fn prune_run_for_graph(run: &Run) -> Run {
    let mut pruned = run.clone();
    if pruned.flow.nodes.is_empty() {
        return pruned;
    }

    let node_ids: Vec<String> = pruned.flow.nodes.iter().map(|n| n.id.clone()).collect();
    let mut flat_edges: Vec<(String, String)> = Vec::new();
    for edge in &pruned.flow.edges {
        match edge {
            FlowEdge::Simple { from, to } => {
                flat_edges.push((from.clone(), to.clone()));
            }
            FlowEdge::Switch { from, cases, .. } => {
                for (_, to) in cases {
                    flat_edges.push((from.clone(), to.clone()));
                }
            }
        }
    }

    let mut visited: HashSet<String> = pruned.steps.iter().map(|s| s.node_id.clone()).collect();
    if let Some(current) = pruned.summary.current_node.as_ref() {
        visited.insert(current.clone());
    }
    if let Some(waiting) = pruned.summary.waiting_on.as_ref() {
        visited.insert(waiting.clone());
    }

    let keep = active_branch_nodes(&pruned.flow.start_at, &node_ids, &flat_edges, &visited);
    if keep.is_empty() {
        return pruned;
    }

    pruned.flow.nodes.retain(|node| keep.contains(&node.id));
    pruned.flow.edges = pruned
        .flow
        .edges
        .into_iter()
        .filter_map(|edge| match edge {
            FlowEdge::Simple { from, to } => {
                if keep.contains(&from) && keep.contains(&to) {
                    Some(FlowEdge::Simple { from, to })
                } else {
                    None
                }
            }
            FlowEdge::Switch { from, on, cases } => {
                if !keep.contains(&from) {
                    return None;
                }
                let kept_cases: Vec<(String, String)> = cases
                    .into_iter()
                    .filter(|(_, to)| keep.contains(to))
                    .collect();
                if kept_cases.is_empty() {
                    None
                } else {
                    Some(FlowEdge::Switch {
                        from,
                        on,
                        cases: kept_cases,
                    })
                }
            }
        })
        .collect();

    pruned
}

````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 1-350 of 1936 (repository grok-build)
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
pub struct ArtifactRef {
    pub path: String,
    pub media_type: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub session_id: String,
    pub event_start_seq: u64,
    pub event_end_seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
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
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    pub summary: Option<String>,
    pub is_shell: bool,
    /// Snapshot `timeoutMs`. `None` when absent, null, or not a positive integer.
    pub timeout_ms: Option<u64>,
    /// Snapshot `mountPath`. Empty for root-workflow nodes.
    pub mount_path: Vec<String>,
    /// Snapshot `localNodeId`. For `includeTransition == "exit"`, the exit name.
    pub local_node_id: Option<String>,
    /// Snapshot `includeTransition`: `Some("entry" | "exit")` or `None`.
    pub include_transition: Option<String>,
}

/// One included workflow from `flow.json` `mounts` (snapshot `composition.mounts`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowMount {
    pub mount_path: Vec<String>,
    pub workflow_name: String,
    /// Child `maxSteps` when the included workflow declared one. `None` means no separate child cap.
    pub max_steps: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlowEdge {
    Simple {
        from: String,
        to: String,
    },
    Switch {
        from: String,
        on: String,
        cases: Vec<(String, String)>,
    },
}

impl Default for FlowEdge {
    fn default() -> Self {
        Self::Simple {
            from: String::new(),
            to: String::new(),
        }
    }
}

/// flow.json. `nodes` is ordered breadth-first from `start_at` following edges (Simple first, then Switch cases in
/// order); nodes not reachable are appended sorted by id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowDef {
    pub name: String,
    pub start_at: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    /// Verified root `maxSteps` (configured or engine default 100). `None` when unverified.
    pub max_steps: Option<usize>,
    /// Snapshot `composition.mounts`.
    pub mounts: Vec<FlowMount>,
}

/// One line of trace.ndjson.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    pub seq: u64,
    pub at_ms: i64,
    pub scope: String,
    pub kind: String,
    pub node_id: Option<String>,
    pub attempt_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
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

### `crates/codegen/xai-grok-pager/src/views/traces_fold.rs` lines 1-162 of 491 (repository grok-build)
````
//! Pure fold model for `/traces` tree views.
//!
//! A `FoldTree` is a preorder list of nodes; a `FoldState` records which
//! foldable nodes are collapsed, keyed by a stable `K` so state survives
//! live requeries. Leaves are never stored as collapsed.

use std::collections::HashSet;
use std::hash::Hash;
use std::ops::Range;

/// One node in preorder. `parent` is the index of its parent; `depth` is 0 for roots.
#[derive(Clone, Debug)]
pub(crate) struct FoldNode<K> {
    pub key: K,
    pub parent: Option<usize>,
    pub depth: usize,
}

/// Preorder node list. Invariant: `parent < child index` and `depth == parent.depth + 1`.
#[derive(Clone, Debug)]
pub(crate) struct FoldTree<K> {
    pub nodes: Vec<FoldNode<K>>,
}

/// Collapsed-node set keyed by stable identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoldState<K: Eq + Hash + Clone> {
    collapsed: HashSet<K>,
}

impl<K: Eq + Hash + Clone> FoldTree<K> {
    /// True when node `i` has at least one child (the node directly after it).
    pub fn has_children(&self, i: usize) -> bool {
        self.nodes.get(i + 1).is_some_and(|n| n.parent == Some(i))
    }

    /// Index range of node `i` and all its descendants.
    pub fn subtree(&self, i: usize) -> Range<usize> {
        let depth = self.nodes.get(i).map_or(0, |n| n.depth);
        let end = self
            .nodes
            .iter()
            .enumerate()
            .skip(i + 1)
            .find(|(_, n)| n.depth <= depth)
            .map_or(self.nodes.len(), |(j, _)| j);
        i..end
    }

    /// Indices of nodes to render, in order. Descendants of collapsed nodes are skipped.
    pub fn visible(&self, st: &FoldState<K>) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut i = 0;
        while let Some(node) = self.nodes.get(i) {
            out.push(i);
            i = if self.has_children(i) && st.is_collapsed(&node.key) {
                self.subtree(i).end
            } else {
                i + 1
            };
        }
        out
    }
}

impl<K: Eq + Hash + Clone> Default for FoldState<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash + Clone> FoldState<K> {
    /// Empty state: everything expanded.
    pub fn new() -> Self {
        Self {
            collapsed: HashSet::new(),
        }
    }

    /// State with exactly the given keys collapsed.
    pub fn with_collapsed<I: IntoIterator<Item = K>>(keys: I) -> Self {
        Self {
            collapsed: keys.into_iter().collect(),
        }
    }

    /// True when `key` is stored as collapsed.
    pub fn is_collapsed(&self, key: &K) -> bool {
        self.collapsed.contains(key)
    }

    /// Flip node `i`'s fold. No-op on leaves.
    pub fn toggle(&mut self, tree: &FoldTree<K>, i: usize) {
        if !tree.has_children(i) {
            return;
        }
        let Some(node) = tree.nodes.get(i) else {
            return;
        };
        if !self.collapsed.remove(&node.key) {
            self.collapsed.insert(node.key.clone());
        }
    }

    /// Collapse node `i`. If `i` is a leaf or already collapsed, returns its parent index
    /// so the caller can move selection there; otherwise returns `None`.
    pub fn collapse(&mut self, tree: &FoldTree<K>, i: usize) -> Option<usize> {
        let node = tree.nodes.get(i)?;
        if !tree.has_children(i) || self.is_collapsed(&node.key) {
            return node.parent;
        }
        self.collapsed.insert(node.key.clone());
        None
    }

    /// Expand node `i` only. No-op on leaves.
    pub fn expand(&mut self, tree: &FoldTree<K>, i: usize) {
        if let Some(node) = tree.nodes.get(i) {
            self.collapsed.remove(&node.key);
        }
    }

    /// Expand node `i` and every descendant. Nodes outside the subtree are untouched.
    pub fn expand_subtree(&mut self, tree: &FoldTree<K>, i: usize) {
        for j in tree.subtree(i) {
            self.expand(tree, j);
        }
    }

    /// Collapse node `i` and every descendant that has children. Nodes outside the subtree are untouched.
    pub fn collapse_subtree(&mut self, tree: &FoldTree<K>, i: usize) {
        for j in tree.subtree(i) {
            if tree.has_children(j) {
                self.collapse(tree, j);
            }
        }
    }

    /// Smart toggle over the whole tree: if any foldable node is collapsed, expand all;
    /// otherwise collapse all foldable nodes.
    pub fn toggle_all(&mut self, tree: &FoldTree<K>) {
        let any_collapsed = tree
            .nodes
            .iter()
            .enumerate()
            .any(|(j, n)| tree.has_children(j) && self.is_collapsed(&n.key));
        if any_collapsed {
            self.collapsed.clear();
        } else {
            for j in 0..tree.nodes.len() {
                self.collapse(tree, j);
            }
        }
    }

    /// Drop stored keys that no longer appear in `tree`.
    pub fn retain(&mut self, tree: &FoldTree<K>) {
        let live: HashSet<&K> = tree.nodes.iter().map(|n| &n.key).collect();
        self.collapsed.retain(|k| live.contains(k));
    }
}

````

=== TASK SPECIFICATION ===

Slice ID: executed-timeline

Ownership: only crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs, including its tests. Work with the existing
RunViewState, FoldTree and FoldState; don't add another generic fold model.

Goal: on entry the stage pane reads as the executed timeline. Finished
attempt roots start collapsed with their result and trace children one key
away. The active stage and the genuine pending gate are outside that default;
the waiting choices stay discoverable and the initial selection still lands
on the waiting stage. Preserve the existing core key actions and any user
choice to expand an attempt across refresh and noise toggles. Apply a default
once per newly observed finished attempt, not on every rebuild. If the current
live attempt has a recorded step, distinguish it from earlier finished loop
attempts by position and the step timestamps. Do not derive the run's terminal
status from an attempt's completion or change existing answer handling.

Keep one executed root per step in run.steps order, including repeated node
IDs. Preserve stable root keys and selected detail targets. Use
prune_run_for_graph from the existing modal_view module as the source of
eligible flow nodes for the skipped section. Within that already-pruned set
count or list only NotRun nodes. Preserve the current treatment of live
nodes, incomplete flow information and noise unless required by the change.

Tests: update the existing expanded-default assertion, then prove that
Space or Right reveals evidence, the noise toggle keeps the expansion,
initial selection and the pending choice remain accessible, and distinct
attempts of one node stay in order, e.g. build -> repair -> build. Cover
switch pruning so an untaken branch does not appear under Skipped, and cover
preservation of manual expansion after sync. Update other affected tests
that relied on implicit expansion by expanding explicitly before navigation.
Do not rewrite the tree, input dispatcher or rendering code wholesale.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-returns/executed-timeline.patch ===
```diff
<complete git unified diff>
```
