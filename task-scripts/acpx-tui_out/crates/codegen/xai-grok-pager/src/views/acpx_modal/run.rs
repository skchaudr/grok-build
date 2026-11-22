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
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{
    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, TraceEvent,
    fmt_duration, value_text,
};
use crate::acpx::graph::{Direction, mermaid_source};
use crate::render::line_utils::truncate_str;
use crate::scrollback::blocks::markdown_content::MarkdownContent;
use crate::theme::Theme;
use crate::views::acpx_modal::detail::DetailTarget;
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
        if self.run.summary.status == RunStatus::Waiting && !bundle::gate_choices(&self.run).is_empty()
        {
            out.push("1-9 gate");
        }
        out.extend([
            "Space fold",
            "J/K detail",
            "v graph",
            "n noise",
            "y copy",
            "Esc back",
            "? keys",
        ]);
        out
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        let vis = self.visible();
        if vis.is_empty() || delta == 0 {
            return false;
        }
        let max_idx = vis.len().saturating_sub(1);
        let next = if delta > 0 {
            self.selected.saturating_add(delta as usize).min(max_idx)
        } else {
            self.selected.saturating_sub(delta.unsigned_abs())
        };
        if next != self.selected {
            self.selected = next;
            true
        } else {
            false
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> RunOutcome {
        if key.kind == KeyEventKind::Release {
            return RunOutcome::Unchanged;
        }

        if let Some(confirm) = self.gate_confirm.clone() {
            return match key.code {
                KeyCode::Enter => {
                    self.gate_confirm = None;
                    RunOutcome::Gate {
                        choice: confirm.choice,
                        flow_file: confirm.flow_file,
                    }
                }
                KeyCode::Esc => {
                    self.gate_confirm = None;
                    RunOutcome::Changed
                }
                _ => RunOutcome::Unchanged,
            };
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => RunOutcome::Back,
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_pane_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
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
            KeyCode::Char('U') => RunOutcome::ScrollDetail(-15),
            KeyCode::F(5) => {
                let vis = self.visible();
                if let Some(pos) = vis.iter().position(|&i| {
                    matches!(
                        self.payloads.get(i),
                        Some(NodePayload::GateChoice { .. } | NodePayload::GateEmpty { .. })
                    )
                }) {
                    self.selected = pos;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(ch @ '1'..='9') => {
                if self.run.summary.status != RunStatus::Waiting {
                    return RunOutcome::Unchanged;
                }
                let idx = (ch as usize) - ('1' as usize);
                let choices = bundle::gate_choices(&self.run);
                if let Some((label, flow_file)) = choices.get(idx) {
                    self.gate_confirm = Some(GateConfirm {
                        choice: label.clone(),
                        flow_file: flow_file.clone(),
                    });
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Enter => {
                let vis = self.visible();
                let Some(&node_idx) = vis.get(self.selected) else {
                    return RunOutcome::Unchanged;
                };
                if let Some(NodePayload::GateChoice {
                    label, flow_file, ..
                }) = self.payloads.get(node_idx)
                {
                    self.gate_confirm = Some(GateConfirm {
                        choice: label.clone(),
                        flow_file: flow_file.clone(),
                    });
                    return RunOutcome::Changed;
                }
                if let Some(step_idx) = self.selected_step() {
                    RunOutcome::OpenStep(step_idx)
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('r') => {
                if let Some(step_idx) = self.selected_step()
                    && let Some(step) = self.run.steps.get(step_idx)
                    && step.conversation.is_some()
                {
                    RunOutcome::Replay(step_idx)
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('y') => {
                if let Some(step_idx) = self.selected_step()
                    && let Some(step) = self.run.steps.get(step_idx)
                {
                    RunOutcome::Copy(value_text(&step.output))
                } else {
                    RunOutcome::Unchanged
                }
            }
            _ => RunOutcome::Unchanged,
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        self.detail_rect = None;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut y = area.y;
        let max_y = area.y.saturating_add(area.height);

        // 1. Header row
        if y < max_y {
            self.render_header(buf, Rect::new(area.x, y, area.width, 1), theme, now_ms);
            y = y.saturating_add(1);
        }

        // 2. Banner row (when needed)
        if y < max_y
            && let Some(banner_line) = self.build_banner(area.width as usize, theme, now_ms)
        {
            buf.set_line(area.x, y, &banner_line, area.width);
            y = y.saturating_add(1);
        }

        // 3. Flow graph (when toggled on and area height >= 16)
        if self.show_graph && area.height >= 16 && y < max_y {
            let max_graph_rows = (area.height as usize / 3).max(1);
            // Leave at least 2 rows for separator + stages pane
            let avail_for_graph = (max_y.saturating_sub(y) as usize).saturating_sub(2);
            let budget = max_graph_rows.min(avail_for_graph);
            if budget > 0 {
                let graph_lines = self.prepare_graph_lines(area.width, budget, theme);
                for gline in graph_lines {
                    if y >= max_y {
                        break;
                    }
                    buf.set_line(area.x, y, &gline, area.width);
                    y = y.saturating_add(1);
                }
            }
        }

        // 4. Separator row
        if y < max_y {
            let rule: String = "─".repeat(area.width as usize);
            buf.set_stringn(
                area.x,
                y,
                &rule,
                area.width as usize,
                Style::default().fg(theme.gray_dim),
            );
            y = y.saturating_add(1);
        }

        if y >= max_y {
            return;
        }

        // 5. Split pane below
        let body_h = max_y.saturating_sub(y);
        let stages_rect = if area.width >= 110 {
            let left_w = ((area.width as u32 * 2 / 5) as u16).clamp(46, 76);
            let sep_x = area.x.saturating_add(left_w);
            let detail_x = sep_x.saturating_add(1);
            let detail_w = area.x.saturating_add(area.width).saturating_sub(detail_x);
            let sep_style = Style::default().fg(theme.gray_dim);
            for row_y in y..max_y {
                buf.set_stringn(sep_x, row_y, "│", 1, sep_style);
            }
            self.detail_rect = Some(Rect::new(detail_x, y, detail_w, body_h));
            Rect::new(area.x, y, left_w, body_h)
        } else {
            self.detail_rect = None;
            Rect::new(area.x, y, area.width, body_h)
        };

        self.render_stages_pane(buf, stages_rect, theme, now_ms);

        if self.gate_confirm.is_some() {
            self.render_gate_confirm(buf, stages_rect, theme);
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
            let state = if is_waiting_stage {
                NodeState::Waiting
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
                if !self.show_noise && ev.kind == "node_heartbeat" {
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
                state,
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
                if !self.show_noise && ev.kind == "node_heartbeat" {
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
            .waiting_on
            .as_deref()
            .or(self.run.summary.current_node.as_deref());
        if let Some(tn) = target_node
            && let Some(pos) = vis.iter().rposition(|&i| match self.payloads.get(i) {
                Some(NodePayload::StepStage { step_idx, .. }) => self
                    .run
                    .steps
                    .get(*step_idx)
                    .is_some_and(|s| s.node_id == tn),
                Some(NodePayload::LiveStage { node_id, .. }) => node_id == tn,
                _ => false,
            })
        {
            self.selected = pos;
            return true;
        }
        // Else last stage that ran
        if let Some(pos) = vis.iter().rposition(|&i| {
            matches!(
                self.payloads.get(i),
                Some(NodePayload::StepStage { .. } | NodePayload::LiveStage { .. })
            )
        }) {
            self.selected = pos;
            return true;
        }
        false
    }

    fn jump_next_failed(&mut self) -> bool {
        let vis = self.visible();
        let failed_positions: Vec<usize> = vis
            .iter()
            .enumerate()
            .filter_map(|(pos, &i)| match self.payloads.get(i) {
                Some(NodePayload::StepStage { step_idx, .. }) => {
                    let s = self.run.steps.get(*step_idx)?;
                    matches!(
                        s.outcome,
                        Outcome::Failed | Outcome::TimedOut | Outcome::Cancelled
                    )
                    .then_some(pos)
                }
                _ => None,
            })
            .collect();
        if failed_positions.is_empty() {
            return false;
        }
        let next = failed_positions
            .iter()
            .copied()
            .find(|&p| p > self.selected)
            .unwrap_or(failed_positions[0]);
        self.selected = next;
        true
    }

    fn jump_stage(&mut self, dir: isize) -> bool {
        let vis = self.visible();
        let stage_positions: Vec<usize> = vis
            .iter()
            .enumerate()
            .filter_map(|(pos, &i)| {
                matches!(
                    self.payloads.get(i),
                    Some(
                        NodePayload::StepStage { .. }
                            | NodePayload::LiveStage { .. }
                            | NodePayload::SkippedHeader
                    )
                )
                .then_some(pos)
            })
            .collect();
        if stage_positions.is_empty() {
            return false;
        }
        let target = if dir > 0 {
            stage_positions.iter().copied().find(|&p| p > self.selected)
        } else {
            stage_positions
                .iter()
                .rev()
                .copied()
                .find(|&p| p < self.selected)
        };
        if let Some(pos) = target
            && pos != self.selected
        {
            self.selected = pos;
            true
        } else {
            false
        }
    }

    // ---- Rendering helpers ----

    fn render_header(&self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        let status = self.run.summary.status;
        let status_color = status_color(status, theme);
        let bold = Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(theme.gray);
        let sep = Style::default().fg(theme.gray_dim);

        let mut spans = vec![
            Span::styled(
                format!("{} ", status.glyph()),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(self.run.summary.title.clone(), bold),
        ];

        let mut facts: Vec<String> = Vec::new();
        if !self.run.summary.flow_name.is_empty()
            && !self
                .run
                .summary
                .title
                .starts_with(&self.run.summary.flow_name)
        {
            facts.push(self.run.summary.flow_name.clone());
        }
        let (done, total) = self.run.progress();
        facts.push(format!("{done}/{total} nodes"));
        facts.push(fmt_duration(self.run.duration_ms(now_ms)));
        if !status.is_live() {
            let ref_ms = self
                .run
                .summary
                .finished_ms
                .unwrap_or(self.run.summary.updated_ms);
            if ref_ms > 0 && now_ms >= ref_ms {
                facts.push(fmt_age(now_ms - ref_ms));
            }
        }

        for fact in facts {
            spans.push(Span::styled(" · ", sep));
            spans.push(Span::styled(fact, dim));
        }

        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    }

    fn build_banner(&self, width: usize, theme: &Theme, now_ms: i64) -> Option<Line<'static>> {
        let sum = &self.run.summary;
        let (text, color) = if bundle::is_stalled(sum, now_ms) {
            let node = sum.current_node.as_deref().unwrap_or("?");
            (
                format!("⊘ stalled at {node}: the acpx process is gone"),
                theme.gray,
            )
        } else {
            match sum.status {
                RunStatus::Waiting => {
                    let node = sum
                        .waiting_on
                        .as_deref()
                        .or(sum.current_node.as_deref())
                        .unwrap_or("?");
                    let detail = sum.status_detail.as_deref().unwrap_or("waiting");
                    (format!("◐ NEEDS YOU at {node}: {detail}"), theme.warning)
                }
                RunStatus::Failed | RunStatus::TimedOut => {
                    let err = sum
                        .error
                        .as_deref()
                        .or(sum.status_detail.as_deref())
                        .unwrap_or(sum.status.label());
                    (format!("✗ {err}"), theme.accent_error)
                }
                RunStatus::Running => {
                    let node = sum.current_node.as_deref().unwrap_or("?");
                    let msg = match sum.status_detail.as_deref() {
                        Some(d) if !d.is_empty() => format!("● {node} · {d}"),
                        _ => format!("● {node}"),
                    };
                    (msg, theme.accent_running)
                }
                RunStatus::Completed | RunStatus::Unknown => return None,
            }
        };

        let cut = truncate_str(&text, width);
        Some(Line::from(Span::styled(
            cut,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )))
    }

    fn prepare_graph_lines(
        &mut self,
        width: u16,
        max_rows: usize,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let w = width as usize;
        let lr_src = mermaid_source(&self.run, Direction::LeftRight);
        let td_src = mermaid_source(&self.run, Direction::TopDown);

        let cache_valid = self.graph_cache.as_ref().is_some_and(|c| {
            c.width == width && c.lr_src == lr_src && c.td_src == td_src
        });

        if !cache_valid {
            let lr_lines = render_mermaid_lines(&lr_src, w);
            let td_lines = render_mermaid_lines(&td_src, w);
            self.graph_cache = Some(GraphCache {
                width,
                lr_src,
                td_src,
                lr_lines,
                td_lines,
            });
        }

        let Some(cache) = self.graph_cache.as_ref() else {
            return Vec::new();
        };

        let lr_bad = is_source_listing(&cache.lr_lines)
            || cache.lr_lines.len() > max_rows
            || any_line_wider(&cache.lr_lines, w);

        if !lr_bad {
            return cache.lr_lines.clone();
        }

        if is_source_listing(&cache.td_lines) || any_line_wider(&cache.td_lines, w) {
            return vec![Line::from(Span::styled(
                truncate_str("graph does not fit at this width (v hides this line)", w),
                Style::default().fg(theme.gray_dim),
            ))];
        }

        if cache.td_lines.len() <= max_rows {
            return cache.td_lines.clone();
        }

        let mut out: Vec<Line<'static>> = cache
            .td_lines
            .iter()
            .take(max_rows.saturating_sub(1))
            .cloned()
            .collect();
        out.push(Line::from(Span::styled(
            truncate_str("… graph cut (v hides it)", w),
            Style::default().fg(theme.gray_dim),
        )));
        out
    }

    fn render_stages_pane(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let h = area.height as usize;
        let w = area.width as usize;
        self.last_pane_height = h;

        let vis = self.visible();
        if vis.is_empty() {
            let msg = truncate_str("no stages recorded", w);
            let msg_w = UnicodeWidthStr::width(msg.as_str()) as u16;
            let x = area.x + area.width.saturating_sub(msg_w) / 2;
            let y = area.y + area.height / 2;
            buf.set_stringn(x, y, &msg, w, Style::default().fg(theme.gray_dim));
            return;
        }

        self.clamp_selected();

        // Count root stage transitions to see if blank separator rows fit when pane is taller than content.
        let num_roots = vis
            .iter()
            .filter(|&&i| self.tree.nodes.get(i).is_some_and(|n| n.depth == 0))
            .count();
        let blank_gaps = num_roots.saturating_sub(1);
        let use_blank_separators = vis.len() + blank_gaps <= h;

        if use_blank_separators {
            self.scroll = 0;
        } else {
            if self.selected < self.scroll {
                self.scroll = self.selected;
            } else if self.selected >= self.scroll + h {
                self.scroll = self.selected + 1 - h;
            }
            let max_scroll = vis.len().saturating_sub(h);
            if self.scroll > max_scroll {
                self.scroll = max_scroll;
            }
        }

        let mut row_y = area.y;
        let max_y = area.y.saturating_add(area.height);

        for (vis_pos, &node_idx) in vis.iter().enumerate().skip(self.scroll) {
            if row_y >= max_y {
                break;
            }
            let is_root = self
                .tree
                .nodes
                .get(node_idx)
                .is_some_and(|n| n.depth == 0);
            if use_blank_separators && is_root && vis_pos > 0 {
                row_y = row_y.saturating_add(1);
                if row_y >= max_y {
                    break;
                }
            }

            let is_selected = vis_pos == self.selected;
            let is_last_child = !is_root && self.is_last_visible_child(&vis, vis_pos, node_idx);

            let row_rect = Rect::new(area.x, row_y, area.width, 1);
            if is_selected {
                buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
            }

            let line = self.render_tree_row(
                node_idx,
                is_selected,
                is_last_child,
                w,
                theme,
                now_ms,
            );
            buf.set_line(area.x, row_y, &line, area.width);
            row_y = row_y.saturating_add(1);
        }

        // Right-aligned position counter when rows overflow
        if vis.len() > h {
            let counter = format!(" {}/{} ", self.selected + 1, vis.len());
            let cw = UnicodeWidthStr::width(counter.as_str()) as u16;
            if cw <= area.width {
                let cx = area.x + area.width - cw;
                buf.set_stringn(
                    cx,
                    area.y,
                    &counter,
                    cw as usize,
                    Style::default().fg(theme.gray_dim),
                );
            }
        }
    }

    fn is_last_visible_child(&self, vis: &[usize], vis_pos: usize, node_idx: usize) -> bool {
        let Some(parent) = self.tree.nodes.get(node_idx).and_then(|n| n.parent) else {
            return false;
        };
        match vis.get(vis_pos + 1) {
            Some(&next_idx) => self
                .tree
                .nodes
                .get(next_idx)
                .is_none_or(|n| n.parent != Some(parent)),
            None => true,
        }
    }

    fn render_tree_row(
        &self,
        node_idx: usize,
        selected: bool,
        is_last_child: bool,
        width: usize,
        theme: &Theme,
        now_ms: i64,
    ) -> Line<'static> {
        let Some(node) = self.tree.nodes.get(node_idx) else {
            return Line::default();
        };
        let Some(payload) = self.payloads.get(node_idx) else {
            return Line::default();
        };

        let bg = selected.then_some(theme.bg_highlight);
        let style_with = |fg: Color, bold: bool| {
            let mut s = Style::default().fg(fg);
            if let Some(b) = bg {
                s = s.bg(b);
            }
            if bold || selected {
                s = s.add_modifier(Modifier::BOLD);
            }
            s
        };

        match payload {
            NodePayload::StepStage {
                step_idx,
                stage_num,
                state,
            } => {
                let Some(step) = self.run.steps.get(*step_idx) else {
                    return Line::default();
                };
                let arrow = self.fold_arrow(node_idx, &node.key);
                let label = self.stage_agent_or_kind_label(Some(step), &step.node_id, step.kind);
                let dur = fmt_duration(step.duration_ms());
                self.format_stage_header_line(
                    arrow,
                    *stage_num,
                    *state,
                    &step.node_id,
                    &label,
                    &dur,
                    selected,
                    width,
                    theme,
                )
            }
            NodePayload::LiveStage {
                node_id,
                stage_num,
                state,
            } => {
                let arrow = self.fold_arrow(node_idx, &node.key);
                let kind = self
                    .run
                    .flow
                    .nodes
                    .iter()
                    .find(|n| &n.id == node_id)
                    .map_or(NodeKind::Unknown, |n| n.kind);
                let label = self.stage_agent_or_kind_label(None, node_id, kind);
                let dur_ms = self.live_stage_duration_ms(node_id, now_ms);
                let dur = fmt_duration(dur_ms);
                self.format_stage_header_line(
                    arrow,
                    *stage_num,
                    *state,
                    node_id,
                    &label,
                    &dur,
                    selected,
                    width,
                    theme,
                )
            }
            NodePayload::SkippedHeader => {
                let arrow = self.fold_arrow(node_idx, &node.key);
                let text = truncate_str(&format!("{arrow} ── not run ──"), width);
                Line::from(Span::styled(text, style_with(theme.gray_dim, false)))
            }
            NodePayload::SkippedNode { node_id, kind } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let rest = format!("○ {node_id}  {}", kind.label());
                let avail = width.saturating_sub(4);
                let cut = truncate_str(&rest, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.gray_dim, false)),
                ])
            }
            NodePayload::Event {
                rel_ms,
                kind,
                detail,
                ..
            } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let body = if detail.is_empty() {
                    format!("+{}  {}", fmt_duration(*rel_ms as u64), kind)
                } else {
                    format!("+{}  {}  {}", fmt_duration(*rel_ms as u64), kind, detail)
                };
                let avail = width.saturating_sub(4);
                let cut = truncate_str(&body, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.gray, false)),
                ])
            }
            NodePayload::Result { step_idx } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let avail = width.saturating_sub(4);
                let (text, fg) = self
                    .run
                    .steps
                    .get(*step_idx)
                    .map_or_else(|| (String::new(), theme.gray), |s| step_result_summary(s, theme));
                let cut = truncate_str(&text, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(fg, false)),
                ])
            }
            NodePayload::GateChoice {
                choice_idx,
                label,
                flow_file,
                ..
            } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let avail = width.saturating_sub(4);
                let body = format!("[{choice_idx}] {label} → {flow_file}");
                let cut = truncate_str(&body, avail);
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.warning, true)),
                ])
            }
            NodePayload::GateEmpty { .. } => {
                let guide = if is_last_child { "  ╰ " } else { "  │ " };
                let avail = width.saturating_sub(4);
                let cut = truncate_str(
                    "no next step declared: re-run the flow to continue",
                    avail,
                );
                Line::from(vec![
                    Span::styled(guide, style_with(theme.gray_dim, false)),
                    Span::styled(cut, style_with(theme.gray_dim, false)),
                ])
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn format_stage_header_line(
        &self,
        arrow: &'static str,
        stage_num: usize,
        state: NodeState,
        node_id: &str,
        agent_label: &str,
        dur: &str,
        selected: bool,
        width: usize,
        theme: &Theme,
    ) -> Line<'static> {
        let bg = selected.then_some(theme.bg_highlight);
        let mk_style = |fg: Color, bold: bool| {
            let mut s = Style::default().fg(fg);
            if let Some(b) = bg {
                s = s.bg(b);
            }
            if bold || selected {
                s = s.add_modifier(Modifier::BOLD);
            }
            s
        };

        let glyph = state.glyph();
        let glyph_col = node_state_color(state, theme);

        let dur_part = format!(" {dur}");
        let dur_w = UnicodeWidthStr::width(dur_part.as_str());
        let left_budget = width.saturating_sub(dur_w);

        let prefix = format!("{arrow} {stage_num} ");
        let glyph_part = format!("{glyph} ");
        let id_part = node_id.to_string();
        let label_part = if agent_label.is_empty() {
            String::new()
        } else {
            format!("  {agent_label}")
        };

        let p_w = UnicodeWidthStr::width(prefix.as_str());
        let g_w = UnicodeWidthStr::width(glyph_part.as_str());
        let id_budget = left_budget.saturating_sub(p_w + g_w);
        let id_cut = truncate_str(&id_part, id_budget);
        let id_w = UnicodeWidthStr::width(id_cut.as_str());

        let lbl_budget = left_budget.saturating_sub(p_w + g_w + id_w);
        let lbl_cut = truncate_str(&label_part, lbl_budget);
        let lbl_w = UnicodeWidthStr::width(lbl_cut.as_str());

        let used_left = p_w + g_w + id_w + lbl_w;
        let pad = width.saturating_sub(used_left + dur_w);

        let mut spans = vec![
            Span::styled(prefix, mk_style(theme.text_primary, true)),
            Span::styled(glyph_part, mk_style(glyph_col, true)),
            Span::styled(id_cut, mk_style(theme.text_primary, true)),
        ];
        if !lbl_cut.is_empty() {
            spans.push(Span::styled(lbl_cut, mk_style(theme.gray, false)));
        }
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), mk_style(theme.gray, false)));
        }
        if dur_w <= width {
            spans.push(Span::styled(dur_part, mk_style(theme.gray, false)));
        }
        Line::from(spans)
    }

    fn fold_arrow(&self, node_idx: usize, key: &String) -> &'static str {
        if !self.tree.has_children(node_idx) {
            " "
        } else if self.fold.is_collapsed(key) {
            "▸"
        } else {
            "▾"
        }
    }

    fn stage_agent_or_kind_label(
        &self,
        step: Option<&Step>,
        node_id: &str,
        kind: NodeKind,
    ) -> String {
        if let Some(s) = step
            && let Some(st) = self.step_settings.get(&s.attempt_id)
        {
            let lbl = st.label();
            if !lbl.is_empty() {
                return lbl;
            }
        }
        if let Some(st) = self.node_settings.get(node_id) {
            let lbl = st.label();
            if !lbl.is_empty() {
                return lbl;
            }
        }
        if let Some(s) = step
            && let Some(ref agent_name) = s.agent_name
            && !agent_name.is_empty()
        {
            return agent_name.clone();
        }
        let is_shell_node = self
            .run
            .flow
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .is_some_and(|n| n.is_shell);
        let is_shell_step = step
            .and_then(|s| s.action.as_ref())
            .is_some_and(|a| a.action_type == "shell" || a.command.is_some());
        if kind == NodeKind::Action && (is_shell_node || is_shell_step) {
            return "shell".to_string();
        }
        kind.label().to_string()
    }

    fn live_stage_duration_ms(&self, node_id: &str, now_ms: i64) -> u64 {
        let step_attempts: HashSet<&str> = self
            .run
            .steps
            .iter()
            .map(|s| s.attempt_id.as_str())
            .collect();
        let started = self
            .run
            .events
            .iter()
            .rev()
            .find(|ev| {
                ev.node_id.as_deref() == Some(node_id)
                    && ev.kind == "node_started"
                    && ev
                        .attempt_id
                        .as_deref()
                        .is_none_or(|a| !step_attempts.contains(a))
            })
            .map_or(self.run.summary.started_ms, |ev| ev.at_ms);
        if started > 0 && now_ms > started {
            (now_ms - started) as u64
        } else {
            0
        }
    }

    fn render_gate_confirm(&self, buf: &mut Buffer, pane: Rect, theme: &Theme) {
        let Some(ref confirm) = self.gate_confirm else {
            return;
        };
        if pane.width < 12 || pane.height < 5 {
            return;
        }
        let line1 = format!("Start {}?", confirm.flow_file);
        let line2 = format!("choice \"{}\" · runs with approve-all", confirm.choice);
        let line3 = "Enter start · Esc cancel";

        let content_w = [
            UnicodeWidthStr::width(line1.as_str()),
            UnicodeWidthStr::width(line2.as_str()),
            UnicodeWidthStr::width(line3),
        ]
        .into_iter()
        .max()
        .unwrap_or(24);
        let box_w = ((content_w + 4) as u16).min(pane.width);
        let box_h = 5u16.min(pane.height);
        let bx = pane.x + (pane.width.saturating_sub(box_w)) / 2;
        let by = pane.y + (pane.height.saturating_sub(box_h)) / 2;
        let box_rect = Rect::new(bx, by, box_w, box_h);

        let bg_style = Style::default().bg(theme.bg_dark).fg(theme.text_primary);
        for ry in box_rect.y..box_rect.y + box_rect.height {
            for rx in box_rect.x..box_rect.x + box_rect.width {
                buf.set_stringn(rx, ry, " ", 1, bg_style);
            }
        }

        let border_style = Style::default().fg(theme.warning).bg(theme.bg_dark);
        let inner_w = box_w.saturating_sub(2) as usize;
        let top = format!("┌{}┐", "─".repeat(inner_w));
        let bot = format!("└{}┘", "─".repeat(inner_w));
        buf.set_stringn(bx, by, &top, box_w as usize, border_style);
        buf.set_stringn(
            bx,
            by + box_h - 1,
            &bot,
            box_w as usize,
            border_style,
        );
        for ry in (by + 1)..(by + box_h - 1) {
            buf.set_stringn(bx, ry, "│", 1, border_style);
            buf.set_stringn(bx + box_w - 1, ry, "│", 1, border_style);
        }

        let text_w = inner_w.saturating_sub(2);
        let tx = bx.saturating_add(2);
        if box_h >= 3 {
            buf.set_stringn(
                tx,
                by + 1,
                &truncate_str(&line1, text_w),
                text_w,
                Style::default()
                    .fg(theme.text_primary)
                    .bg(theme.bg_dark)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if box_h >= 4 {
            buf.set_stringn(
                tx,
                by + 2,
                &truncate_str(&line2, text_w),
                text_w,
                Style::default().fg(theme.gray).bg(theme.bg_dark),
            );
        }
        if box_h >= 5 {
            buf.set_stringn(
                tx,
                by + 3,
                &truncate_str(line3, text_w),
                text_w,
                Style::default().fg(theme.warning).bg(theme.bg_dark),
            );
        }
    }
}

fn status_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.accent_success,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray,
    }
}

fn node_state_color(state: NodeState, theme: &Theme) -> Color {
    match state {
        NodeState::NotRun => theme.gray_dim,
        NodeState::Running => theme.accent_running,
        NodeState::Waiting => theme.warning,
        NodeState::Done(Outcome::Ok) => theme.accent_success,
        NodeState::Done(Outcome::Failed | Outcome::TimedOut) => theme.accent_error,
        NodeState::Done(Outcome::Cancelled | Outcome::Unknown) => theme.gray,
    }
}

fn step_result_summary(step: &Step, theme: &Theme) -> (String, Color) {
    if matches!(
        step.outcome,
        Outcome::Failed | Outcome::TimedOut | Outcome::Cancelled
    ) {
        let msg = step
            .error
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(match step.outcome {
                Outcome::TimedOut => "timed out",
                Outcome::Cancelled => "cancelled",
                _ => "failed",
            });
        return (first_line(msg), theme.accent_error);
    }
    if let Some( ref err) = step.error
        && !err.is_empty()
    {
        return (first_line(err), theme.accent_error);
    }
    if let Some(route) = step
        .output
        .as_object()
        .and_then(|o| o.get("route"))
        .and_then(Value::as_str)
    {
        return (format!("→ {route}"), theme.text_secondary);
    }
    if let Some(ref action) = step.action
        && (action.action_type == "shell" || action.command.is_some())
        && let Some(code) = action.exit_code
    {
        return (format!("exit {code}"), theme.gray);
    }
    let text = match &step.output {
        Value::Null => step
            .raw_text
            .as_deref()
            .map(first_line)
            .unwrap_or_else(|| "ok".to_string()),
        Value::String(s) => first_line(s),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    (text, theme.gray)
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn render_mermaid_lines(src: &str, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let md = format!("```mermaid\n{src}\n```");
    let out = MarkdownContent::new_with_table_width(md, Some(width)).output(width);
    out.lines.into_iter().map(|bl| bl.content).collect()
}

fn line_plain_text(line: &Line<'_>) -> String {
    let mut s = String::new();
    for span in &line.spans {
        s.push_str(span.content.as_ref());
    }
    s
}

fn is_source_listing(lines: &[Line<'static>]) -> bool {
    lines.iter().any(|l| {
        let t = line_plain_text(l);
        t.contains("mermaid: flowchart") || t.contains("too wide to display")
    })
}

fn any_line_wider(lines: &[Line<'static>], width: usize) -> bool {
    lines
        .iter()
        .any(|l| UnicodeWidthStr::width(line_plain_text(l).as_str()) > width)
}

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
                },
                FlowNode {
                    id: "test".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                },
                FlowNode {
                    id: "gate".to_string(),
                    kind: NodeKind::Checkpoint,
                    summary: None,
                    is_shell: false,
                },
                FlowNode {
                    id: "deploy".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
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

    #[test]
    fn enter_and_r_outcomes_for_acp_and_non_acp_steps() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0); // ACP step with conversation
        assert_eq!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::OpenStep(0)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('r'))),
            RunOutcome::Replay(0)
        );

        st.select_step(1); // Action step without conversation
        assert_eq!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::OpenStep(1)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('r'))),
            RunOutcome::Unchanged
        );
    }

    #[test]
    fn y_esc_and_detail_scroll_outcomes() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0);
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('y'))),
            RunOutcome::Copy("{\n  \"route\": \"ok\"\n}".to_string())
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('J'))),
            RunOutcome::ScrollDetail(3)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('K'))),
            RunOutcome::ScrollDetail(-3)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('D'))),
            RunOutcome::ScrollDetail(15)
        );
        assert_eq!(
            st.handle_key(&key(KeyCode::Char('U'))),
            RunOutcome::ScrollDetail(-15)
        );
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
        st.render(&mut buf, buf.area, &theme, 10_000);
        let text = buf_text(&buf);
        assert!(text.contains("no next step declared: re-run the flow to continue"));
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
        assert!(text.contains("◐ NEEDS YOU at gate: awaiting human approval"));
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
    fn render_narrow_100x30_has_no_detail_area() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
        assert_eq!(st.detail_area(), None);
    }

    #[test]
    fn render_with_v_off_hides_graph_rows() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 140, 35);

        let mut buf_on = Buffer::empty(area);
        st.render(&mut buf_on, area, &theme, 10_000);
        let d_on = st.detail_area().unwrap_or_default();

        st.handle_key(&key(KeyCode::Char('v')));
        let mut buf_off = Buffer::empty(area);
        st.render(&mut buf_off, area, &theme, 10_000);
        let d_off = st.detail_area().unwrap_or_default();

        // With graph off, the separator + split panes start right after header + banner (row 3)
        assert_eq!(d_off.y, 3);
        assert!(d_on.y > d_off.y);
    }

    #[test]
    fn render_tiny_40x8_and_0x0_does_not_panic() {
        let run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
        let mut st = RunViewState::new(run);
        let theme = Theme::default();

        let mut buf_zero = Buffer::empty(Rect::new(0, 0, 0, 0));
        st.render(&mut buf_zero, Rect::new(0, 0, 0, 0), &theme, 10_000);

        let small = Rect::new(0, 0, 40, 8);
        let mut buf_small = Buffer::empty(small);
        st.render(&mut buf_small, small, &theme, 10_000);

        // Also open the confirmation box and render at small & zero sizes
        st.handle_key(&key(KeyCode::Char('1')));
        st.render(&mut buf_small, small, &theme, 10_000);
        st.render(&mut buf_zero, Rect::new(0, 0, 0, 0), &theme, 10_000);
    }
}
