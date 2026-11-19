//! Screen for one flow run: the flow graph on top, a foldable outline of its steps below.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{
    NodeKind, NodeState, Outcome, Run, RunStatus, Step, TraceEvent, fmt_duration, value_text,
};
use crate::acpx::graph::{Direction, mermaid_source};
use crate::render::line_utils::truncate_str;
use crate::scrollback::blocks::markdown_content::MarkdownContent;
use crate::theme::Theme;
use crate::views::traces_fold::{FoldNode, FoldState, FoldTree};
use crate::views::traces_modal::fmt_age;

/// Outcome of handling a key in the run view.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    Unchanged,
    Changed,
    Back,
    OpenStep(usize),
    Replay(usize),
    Copy(String),
}

#[derive(Debug, Clone)]
enum OutlineRow {
    Step {
        step_idx: usize,
    },
    LiveNode {
        node_id: String,
        kind: NodeKind,
        state: NodeState,
    },
    Event {
        step_idx: usize,
        delta_ms: i64,
        kind: String,
        detail: String,
    },
}

#[derive(Debug, Clone)]
struct GraphCache {
    width: usize,
    lr_src: String,
    td_src: String,
    lr_lines: Vec<Line<'static>>,
    td_lines: Vec<Line<'static>>,
}

/// Interactive state for the single-run screen.
pub struct RunViewState {
    run: Run,
    tree: FoldTree<String>,
    fold: FoldState<String>,
    expanded_roots: HashSet<String>,
    rows: Vec<OutlineRow>,
    visible: Vec<usize>,
    selected_vis: usize,
    scroll: usize,
    last_list_height: usize,
    show_graph: bool,
    show_noise: bool,
    graph_cache: Option<GraphCache>,
}

impl RunViewState {
    /// Create a run view. Selection starts on the waiting/current step, else the last step.
    pub fn new(run: Run) -> Self {
        let mut state = Self {
            run,
            tree: FoldTree { nodes: Vec::new() },
            fold: FoldState::new(),
            expanded_roots: HashSet::new(),
            rows: Vec::new(),
            visible: Vec::new(),
            selected_vis: 0,
            scroll: 0,
            last_list_height: 10,
            show_graph: true,
            show_noise: false,
            graph_cache: None,
        };
        state.rebuild_outline(None);
        state.select_initial();
        state
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn run_mut(&mut self) -> &mut Run {
        &mut self.run
    }

    /// Call after the run changed: rebuild rows, keep selection (by attempt id) and folds.
    pub fn sync(&mut self) {
        let prev_key = self
            .current_node_index()
            .and_then(|idx| self.tree.nodes.get(idx))
            .map(|n| n.key.clone());
        let prev_root_pos = self.current_root_ordinal();
        self.rebuild_outline(prev_key.as_deref());
        if self
            .current_node_index()
            .and_then(|idx| self.tree.nodes.get(idx))
            .map(|n| n.key.as_str())
            != prev_key.as_deref()
        {
            // Fallback to nearest earlier root.
            self.select_Fallback_root(prev_root_pos);
        }
    }

    /// Return the selected step index in `run.steps`, if the selection is on (or inside) a recorded step.
    pub fn selected_step(&self) -> Option<usize> {
        let node_idx = self.current_node_index()?;
        match self.rows.get(node_idx)? {
            OutlineRow::Step { step_idx } | OutlineRow::Event { step_idx, .. } => Some(*step_idx),
            OutlineRow::LiveNode { .. } => None,
        }
    }

    /// Move selection to the root row for `run.steps[index]`.
    pub fn select_step(&mut self, index: usize) {
        for (vis_idx, &node_idx) in self.visible.iter().enumerate() {
            if let Some(OutlineRow::Step { step_idx }) = self.rows.get(node_idx)
                && *step_idx == index
            {
                self.selected_vis = vis_idx;
                return;
            }
        }
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut out = vec![
            "Enter step",
            "Space/h/l fold",
            "E all",
            "v graph",
            "n noise",
            "!/c/[ ] jump",
            "y copy",
        ];
        if self
            .selected_step()
            .and_then(|i| self.run.steps.get(i))
            .is_some_and(|s| s.conversation.is_some())
        {
            out.insert(1, "r replay");
        }
        out.push("Esc back");
        out
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        if self.visible.is_empty() || delta == 0 {
            return false;
        }
        let max_idx = self.visible.len().saturating_sub(1);
        let next = if delta > 0 {
            self.selected_vis.saturating_add(delta as usize).min(max_idx)
        } else {
            self.selected_vis.saturating_sub(delta.unsigned_abs())
        };
        if next != self.selected_vis {
            self.selected_vis = next;
            true
        } else {
            false
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> RunOutcome {
        if key.kind == KeyEventKind::Release {
            return RunOutcome::Unchanged;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => RunOutcome::Back,
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                if self.scroll(1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                if self.scroll(-1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('g') | KeyCode::Home if !ctrl => {
                if !self.visible.is_empty() && self.selected_vis != 0 {
                    self.selected_vis = 0;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                let last = self.visible.len().saturating_sub(1);
                if !self.visible.is_empty() && self.selected_vis != last {
                    self.selected_vis = last;
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_list_height / 2).max(1) as isize;
                if self.scroll(half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_list_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_list_height / 2).max(1) as isize;
                if self.scroll(half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_list_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Left | KeyCode::Char('h') if !ctrl => {
                if self.collapse_selected() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Right | KeyCode::Char('l') if !ctrl => {
                if self.expand_selected() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(' ') if !ctrl => {
                if self.toggle_selected() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('E') if !ctrl => {
                if self.toggle_all_folds() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('n') if !ctrl => {
                self.show_noise = !self.show_noise;
                let keep_key = self
                    .current_node_index()
                    .and_then(|idx| self.tree.nodes.get(idx))
                    .map(|n| n.key.clone());
                let prev_parent_key = self
                    .current_node_index()
                    .and_then(|idx| self.tree.nodes.get(idx))
                    .and_then(|n| n.parent)
                    .and_then(|p| self.tree.nodes.get(p))
                    .map(|n| n.key.clone());
                self.rebuild_outline(keep_key.as_deref().or(prev_parent_key.as_deref()));
                if keep_key.is_some()
                    && self
                        .current_node_index()
                        .and_then(|idx| self.tree.nodes.get(idx))
                        .map(|n| n.key.as_str())
                        != keep_key.as_deref()
                    && let Some(pk) = prev_parent_key.as_deref()
                {
                    self.select_by_key(pk);
                }
                RunOutcome::Changed
            }
            KeyCode::Char('v') if !ctrl => {
                self.show_graph = !self.show_graph;
                RunOutcome::Changed
            }
            KeyCode::Char('!') if !ctrl => {
                if self.jump_next_failed() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('c') if !ctrl => {
                if self.jump_current_or_waiting() {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('[') if !ctrl => {
                if self.jump_step_root(-1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(']') if !ctrl => {
                if self.jump_step_root(1) {
                    RunOutcome::Changed
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Enter => {
                if let Some(step_idx) = self.selected_step() {
                    RunOutcome::OpenStep(step_idx)
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('r') if !ctrl => {
                if let Some(step_idx) = self.selected_step()
                    && self
                        .run
                        .steps
                        .get(step_idx)
                        .is_some_and(|s| s.conversation.is_some())
                {
                    RunOutcome::Replay(step_idx)
                } else {
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char('y') if !ctrl => {
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
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = area.width as usize;
        let mut y = area.y;
        let bottom = area.y.saturating_add(area.height);

        // 1. Header row
        self.render_header(buf, area.x, y, width, theme, now_ms);
        y = y.saturating_add(1);
        if y >= bottom {
            return;
        }

        // 2. Optional banner row
        if let Some(banner) = self.build_banner_line(width, theme) {
            buf.set_line(area.x, y, &banner, area.width);
            y = y.saturating_add(1);
            if y >= bottom {
                return;
            }
        }

        // 3. Graph (when enabled and area.height >= 12)
        if self.show_graph && area.height >= 12 {
            let max_graph_rows = (area.height as usize) / 2;
            let remaining_before_sep = (bottom - y) as usize;
            // Leave at least 2 rows for separator + 1 outline row when possible.
            let budget = max_graph_rows.min(remaining_before_sep.saturating_sub(2));
            if budget > 0 {
                let (lines, cut) = self.prepare_graph_lines(width, max_graph_rows, budget);
                for line in lines {
                    if y >= bottom {
                        break;
                    }
                    buf.set_line(area.x, y, &line, area.width);
                    y = y.saturating_add(1);
                }
                if cut && y < bottom {
                    let cut_msg = truncate_str("… graph cut (v hides it)", width);
                    let cut_line = Line::from(Span::styled(
                        cut_msg,
                        Style::default().fg(theme.gray_dim),
                    ));
                    buf.set_line(area.x, y, &cut_line, area.width);
                    y = y.saturating_add(1);
                }
            }
        }

        if y >= bottom {
            return;
        }

        // 4. Separator row (with optional right-aligned position counter when outline overflows)
        let list_height = (bottom - y).saturating_sub(1) as usize;
        let overflow = self.visible.len() > list_height && list_height > 0;
        self.render_separator(buf, area.x, y, width, theme, overflow);
        y = y.saturating_add(1);
        if y >= bottom {
            return;
        }

        // 5. Outline
        let outline_height = (bottom - y) as usize;
        self.last_list_height = outline_height.max(1);
        self.render_outline(buf, area.x, y, width, outline_height, theme);
    }

    // ---- Internal helpers ----

    fn rebuild_outline(&mut self, keep_selected_key: Option<&str>) {
        let mut nodes: Vec<FoldNode<String>> = Vec::new();
        let mut rows: Vec<OutlineRow> = Vec::new();

        for (step_idx, step) in self.run.steps.iter().enumerate() {
            let root_idx = nodes.len();
            let root_key = step.attempt_id.clone();
            nodes.push(FoldNode {
                key: root_key,
                parent: None,
                depth: 0,
            });
            rows.push(OutlineRow::Step { step_idx });

            for ev in self.run.step_events(&step.attempt_id) {
                if !self.show_noise && ev.kind == "node_heartbeat" {
                    continue;
                }
                let ev_key = format!("{}/{}", step.attempt_id, ev.seq);
                let delta_ms = if step.started_ms > 0 && ev.at_ms >= step.started_ms {
                    ev.at_ms - step.started_ms
                } else {
                    0
                };
                nodes.push(FoldNode {
                    key: ev_key,
                    parent: Some(root_idx),
                    depth: 1,
                });
                rows.push(OutlineRow::Event {
                    step_idx,
                    delta_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }
        }

        // Add synthetic root for a live (Running / Waiting) node that has no step record yet
        // (or is currently active while not represented by an active step).
        for live_id in self.synthetic_live_nodes() {
            let kind = self
                .run
                .flow
                .nodes
                .iter()
                .find(|n| n.id == live_id)
                .map_or(NodeKind::Unknown, |n| n.kind);
            let state = self.run.node_state(&live_id);
            nodes.push(FoldNode {
                key: format!("live/{live_id}"),
                parent: None,
                depth: 0,
            });
            rows.push(OutlineRow::LiveNode {
                node_id: live_id,
                kind,
                state,
            });
        }

        self.tree = FoldTree { nodes };
        self.rows = rows;
        self.fold.retain(&self.tree);

        // Steps start collapsed unless in `expanded_roots`.
        for i in 0..self.tree.nodes.len() {
            if self.tree.nodes[i].depth == 0 && self.tree.has_children(i) {
                let key = &self.tree.nodes[i].key;
                if self.expanded_roots.contains(key) {
                    self.fold.expand(&self.tree, i);
                } else {
                    self.fold.collapse(&self.tree, i);
                }
            }
        }

        self.visible = self.tree.visible(&self.fold);
        if let Some(k) = keep_selected_key {
            self.select_by_key(k);
        } else if self.selected_vis >= self.visible.len() {
            self.selected_vis = self.visible.len().saturating_sub(1);
        }
    }

    fn synthetic_live_nodes(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut candidates: Vec<&str> = Vec::new();
        if let Some(w) = self.run.summary.waiting_on.as_deref()
            && !w.is_empty()
        {
            candidates.push(w);
        }
        if self.run.summary.status == RunStatus::Running
            && let Some(c) = self.run.summary.current_node.as_deref()
            && !c.is_empty()
            && !candidates.contains(&c)
        {
            candidates.push(c);
        }
        for id in candidates {
            let has_step = self.run.steps.iter().any(|s| s.node_id == id);
            if !has_step {
                out.push(id.to_string());
            }
        }
        out
    }

    fn select_initial(&mut self) {
        if self.visible.is_empty() {
            self.selected_vis = 0;
            return;
        }
        let target_node = self
            .run
            .summary
            .waiting_on
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.run
                    .summary
                    .current_node
                    .as_deref()
                    .filter(|s| !s.is_empty())
            });

        if let Some(target) = target_node {
            // Find the last root matching `target` (either a Step or a LiveNode).
            if let Some(vis_pos) = self.visible.iter().rposition(|&node_idx| {
                match self.rows.get(node_idx) {
                    Some(OutlineRow::Step { step_idx }) => self
                        .run
                        .steps
                        .get(*step_idx)
                        .is_some_and(|s| s.node_id == target),
                    Some(OutlineRow::LiveNode { node_id, .. }) => node_id == target,
                    _ => false,
                }
            }) {
                self.selected_vis = vis_pos;
                return;
            }
        }

        // Else select the last step root (or last visible root).
        if let Some(vis_pos) = self
            .visible
            .iter()
            .rposition(|&node_idx| self.tree.nodes.get(node_idx).is_some_and(|n| n.depth == 0))
        {
            self.selected_vis = vis_pos;
        } else {
            self.selected_vis = self.visible.len().saturating_sub(1);
        }
    }

    fn current_node_index(&self) -> Option<usize> {
        self.visible.get(self.selected_vis).copied()
    }

    fn current_root_ordinal(&self) -> usize {
        let Some(cur_node) = self.current_node_index() else {
            return 0;
        };
        let root_node = self
            .tree
            .nodes
            .get(cur_node)
            .and_then(|n| n.parent)
            .unwrap_or(cur_node);
        self.tree
            .nodes
            .iter()
            .take(root_node + 1)
            .filter(|n| n.depth == 0)
            .count()
            .saturating_sub(1)
    }

    #[allow(non_snake_case)]
    fn select_Fallback_root(&mut self, root_ordinal: usize) {
        let roots: Vec<usize> = self
            .visible
            .iter()
            .enumerate()
            .filter_map(|(vis_i, &node_i)| {
                self.tree
                    .nodes
                    .get(node_i)
                    .filter(|n| n.depth == 0)
                    .map(|_| vis_i)
            })
            .collect();
        if roots.is_empty() {
            self.selected_vis = 0;
        } else {
            let idx = root_ordinal.min(roots.len() - 1);
            self.selected_vis = roots[idx];
        }
    }

    fn select_by_key(&mut self, key: &str) -> bool {
        if let Some(pos) = self.visible.iter().position(|&node_idx| {
            self.tree
                .nodes
                .get(node_idx)
                .is_some_and(|n| n.key == key)
        }) {
            self.selected_vis = pos;
            true
        } else {
            false
        }
    }

    fn sync_expanded_roots_from_fold(&mut self) {
        self.expanded_roots.clear();
        for (i, node) in self.tree.nodes.iter().enumerate() {
            if node.depth == 0 && self.tree.has_children(i) && !self.fold.is_collapsed(&node.key) {
                self.expanded_roots.insert(node.key.clone());
            }
        }
    }

    fn collapse_selected(&mut self) -> bool {
        let Some(node_idx) = self.current_node_index() else {
            return false;
        };
        if let Some(parent_idx) = self.fold.collapse(&self.tree, node_idx) {
            if let Some(parent_key) = self.tree.nodes.get(parent_idx).map(|n| n.key.clone()) {
                return self.select_by_key(&parent_key);
            }
            false
        } else {
            let key = self
                .tree
                .nodes
                .get(node_idx)
                .map(|n| n.key.clone())
                .unwrap_or_default();
            self.expanded_roots.remove(&key);
            self.visible = self.tree.visible(&self.fold);
            self.select_by_key(&key);
            true
        }
    }

    fn expand_selected(&mut self) -> bool {
        let Some(node_idx) = self.current_node_index() else {
            return false;
        };
        if !self.tree.has_children(node_idx) {
            return false;
        }
        let Some(key) = self.tree.nodes.get(node_idx).map(|n| n.key.clone()) else {
            return false;
        };
        if !self.fold.is_collapsed(&key) {
            return false;
        }
        self.fold.expand(&self.tree, node_idx);
        self.expanded_roots.insert(key.clone());
        self.visible = self.tree.visible(&self.fold);
        self.select_by_key(&key);
        true
    }

    fn toggle_selected(&mut self) -> bool {
        let Some(node_idx) = self.current_node_index() else {
            return false;
        };
        if !self.tree.has_children(node_idx) {
            return false;
        }
        let Some(key) = self.tree.nodes.get(node_idx).map(|n| n.key.clone()) else {
            return false;
        };
        self.fold.toggle(&self.tree, node_idx);
        self.sync_expanded_roots_from_fold();
        self.visible = self.tree.visible(&self.fold);
        self.select_by_key(&key);
        true
    }

    fn toggle_all_folds(&mut self) -> bool {
        let any_foldable = (0..self.tree.nodes.len()).any(|i| self.tree.has_children(i));
        if !any_foldable {
            return false;
        }
        let cur_node = self.current_node_index();
        let cur_key = cur_node
            .and_then(|i| self.tree.nodes.get(i))
            .map(|n| n.key.clone());
        let parent_key = cur_node
            .and_then(|i| self.tree.nodes.get(i))
            .and_then(|n| n.parent)
            .and_then(|p| self.tree.nodes.get(p))
            .map(|n| n.key.clone());

        self.fold.toggle_all(&self.tree);
        self.sync_expanded_roots_from_fold();
        self.visible = self.tree.visible(&self.fold);

        if let Some(k) = cur_key.as_deref()
            && self.select_by_key(k)
        {
            return true;
        }
        if let Some(pk) = parent_key.as_deref() {
            self.select_by_key(pk);
        }
        true
    }

    fn jump_next_failed(&mut self) -> bool {
        if self.visible.is_empty() {
            return false;
        }
        let len = self.visible.len();
        for offset in 1..=len {
            let vis_idx = (self.selected_vis + offset) % len;
            let node_idx = self.visible[vis_idx];
            if let Some(OutlineRow::Step { step_idx }) = self.rows.get(node_idx)
                && let Some(step) = self.run.steps.get(*step_idx)
                && matches!(step.outcome, Outcome::Failed | Outcome::TimedOut)
            {
                if vis_idx != self.selected_vis {
                    self.selected_vis = vis_idx;
                    return true;
                }
                return false;
            }
        }
        false
    }

    fn jump_current_or_waiting(&mut self) -> bool {
        let before = self.selected_vis;
        let target = self
            .run
            .summary
            .waiting_on
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.run
                    .summary
                    .current_node
                    .as_deref()
                    .filter(|s| !s.is_empty())
            });
        let Some(target) = target else {
            return false;
        };
        if let Some(vis_pos) = self.visible.iter().rposition(|&node_idx| {
            match self.rows.get(node_idx) {
                Some(OutlineRow::Step { step_idx }) => self
                    .run
                    .steps
                    .get(*step_idx)
                    .is_some_and(|s| s.node_id == target),
                Some(OutlineRow::LiveNode { node_id, .. }) => node_id == target,
                _ => false,
            }
        }) {
            self.selected_vis = vis_pos;
            return self.selected_vis != before;
        }
        false
    }

    fn jump_step_root(&mut self, dir: isize) -> bool {
        if self.visible.is_empty() {
            return false;
        }
        if dir < 0 {
            for vis_i in (0..self.selected_vis).rev() {
                let node_idx = self.visible[vis_i];
                if self.tree.nodes.get(node_idx).is_some_and(|n| n.depth == 0) {
                    self.selected_vis = vis_i;
                    return true;
                }
            }
        } else {
            for vis_i in (self.selected_vis + 1)..self.visible.len() {
                let node_idx = self.visible[vis_i];
                if self.tree.nodes.get(node_idx).is_some_and(|n| n.depth == 0) {
                    self.selected_vis = vis_i;
                    return true;
                }
            }
        }
        false
    }

    // ---- Rendering helpers ----

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
            NodeState::Running => theme.accent_running,
            NodeState::Waiting => theme.warning,
            NodeState::Done(Outcome::Ok) => theme.accent_success,
            NodeState::Done(Outcome::Failed | Outcome::TimedOut) => theme.accent_error,
            NodeState::Done(Outcome::Cancelled | Outcome::Unknown) | NodeState::NotRun => {
                theme.gray
            }
        }
    }

    fn outcome_state(outcome: Outcome) -> NodeState {
        NodeState::Done(outcome)
    }

    fn render_header(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        width: usize,
        theme: &Theme,
        now_ms: i64,
    ) {
        let sum = &self.run.summary;
        let glyph = sum.status.glyph();
        let status_color = Self::status_color(sum.status, theme);
        let (done, total) = self.run.progress();
        let dur = fmt_duration(self.run.duration_ms(now_ms));

        let mut facts = vec![
            sum.flow_name.clone(),
            format!("{done}/{total} nodes"),
            dur,
        ];
        if !sum.status.is_live() {
            let ref_ms = sum.finished_ms.unwrap_or(sum.updated_ms);
            if ref_ms > 0 && now_ms >= ref_ms {
                facts.push(format!("{} ago", fmt_age(now_ms - ref_ms)));
            }
        }
        let facts_str = format!(" · {}", facts.join(" · "));

        let prefix = format!("{glyph} ");
        let prefix_w = prefix.width();
        let title_str = sum.title.as_str();
        let title_w = title_str.width();

        let mut spans = Vec::new();
        spans.push(Span::styled(
            prefix,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ));

        if prefix_w + title_w <= width {
            spans.push(Span::styled(
                title_str.to_string(),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            ));
            let rem = width.saturating_sub(prefix_w + title_w);
            if rem > 0 {
                spans.push(Span::styled(
                    truncate_str(&facts_str, rem),
                    Style::default().fg(theme.gray_dim),
                ));
            }
        } else {
            let rem = width.saturating_sub(prefix_w);
            spans.push(Span::styled(
                truncate_str(title_str, rem),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        buf.set_line(x, y, &Line::from(spans), width as u16);
    }

    fn build_banner_line(&self, width: usize, theme: &Theme) -> Option<Line<'static>> {
        let sum = &self.run.summary;
        match sum.status {
            RunStatus::Waiting => {
                let node = sum
                    .waiting_on
                    .as_deref()
                    .or(sum.current_node.as_deref())
                    .unwrap_or("?");
                let detail = sum.status_detail.as_deref().unwrap_or("waiting");
                let primary = format!("◐ WAITING at {node}: {detail}");
                let note = "  acpx cannot resume a run; re-run the flow to continue";
                let primary_w = primary.width();
                if primary_w >= width {
                    Some(Line::from(Span::styled(
                        truncate_str(&primary, width),
                        Style::default().fg(theme.warning),
                    )))
                } else {
                    let rem = width - primary_w;
                    Some(Line::from(vec![
                        Span::styled(primary, Style::default().fg(theme.warning)),
                        Span::styled(
                            truncate_str(note, rem),
                            Style::default().fg(theme.gray_dim),
                        ),
                    ]))
                }
            }
            RunStatus::Failed | RunStatus::TimedOut => {
                let err = sum
                    .error
                    .as_deref()
                    .or(sum.status_detail.as_deref())
                    .or_else(|| {
                        self.run
                            .steps
                            .iter()
                            .rev()
                            .find_map(|s| s.error.as_deref())
                    })
                    .unwrap_or(sum.status.label());
                let text = truncate_str(&format!("✗ {err}"), width);
                Some(Line::from(Span::styled(
                    text,
                    Style::default().fg(theme.accent_error),
                )))
            }
            RunStatus::Running => {
                let node = sum.current_node.as_deref().unwrap_or("running");
                let text = match sum.status_detail.as_deref().filter(|s| !s.is_empty()) {
                    Some(detail) => format!("● {node} · {detail}"),
                    None => format!("● {node}"),
                };
                Some(Line::from(Span::styled(
                    truncate_str(&text, width),
                    Style::default().fg(theme.accent_running),
                )))
            }
            _ => None,
        }
    }

    fn prepare_graph_lines(
        &mut self,
        width: usize,
        half_height: usize,
        budget: usize,
    ) -> (Vec<Line<'static>>, bool) {
        let lr_src = mermaid_source(&self.run, Direction::LeftRight);
        let td_src = mermaid_source(&self.run, Direction::TopDown);

        let cache_hit = self.graph_cache.as_ref().is_some_and(|c| {
            c.width == width && c.lr_src == lr_src && c.td_src == td_src
        });

        if !cache_hit {
            let lr_md = format!("```mermaid\n{lr_src}\n```");
            let lr_out = MarkdownContent::new_with_table_width(lr_md, Some(width)).output(width);
            let lr_lines: Vec<Line<'static>> =
                lr_out.lines.into_iter().map(|b| b.content).collect();

            let td_md = format!("```mermaid\n{td_src}\n```");
            let td_out = MarkdownContent::new_with_table_width(td_md, Some(width)).output(width);
            let td_lines: Vec<Line<'static>> =
                td_out.lines.into_iter().map(|b| b.content).collect();

            self.graph_cache = Some(GraphCache {
                width,
                lr_src,
                td_src,
                lr_lines,
                td_lines,
            });
        }

        let Some(cache) = self.graph_cache.as_ref() else {
            return (Vec::new(), false);
        };

        let lr_too_tall = cache.lr_lines.len() > half_height;
        let lr_too_wide = cache.lr_lines.iter().any(|l| l.width() > width);
        let chosen = if lr_too_tall || lr_too_wide {
            &cache.td_lines
        } else {
            &cache.lr_lines
        };

        let limit = half_height.min(budget);
        if chosen.len() > limit {
            let take = limit.saturating_sub(1);
            (chosen.iter().take(take).cloned().collect(), true)
        } else {
            (chosen.clone(), false)
        }
    }

    fn render_separator(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        width: usize,
        theme: &Theme,
        show_counter: bool,
    ) {
        let style = Style::default().fg(theme.gray_dim);
        if show_counter && !self.visible.is_empty() {
            let counter = format!(" {}/{}", self.selected_vis + 1, self.visible.len());
            let cw = counter.width();
            if cw < width {
                let bars = "─".repeat(width - cw);
                let line = Line::from(vec![
                    Span::styled(bars, style),
                    Span::styled(counter, style),
                ]);
                buf.set_line(x, y, &line, width as u16);
                return;
            }
        }
        let line = Line::from(Span::styled("─".repeat(width), style));
        buf.set_line(x, y, &line, width as u16);
    }

    fn render_outline(
        &mut self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        width: usize,
        height: usize,
        theme: &Theme,
    ) {
        if height == 0 || width == 0 {
            return;
        }
        if self.visible.is_empty() {
            let msg = "No steps recorded yet";
            let truncated = truncate_str(msg, width);
            let pad = width.saturating_sub(truncated.width()) / 2;
            let row_y = y + (height as u16) / 2;
            let line = Line::from(Span::styled(
                format!("{}{}", " ".repeat(pad), truncated),
                Style::default().fg(theme.gray_dim),
            ));
            buf.set_line(x, row_y, &line, width as u16);
            return;
        }

        if self.selected_vis >= self.visible.len() {
            self.selected_vis = self.visible.len().saturating_sub(1);
        }
        if self.selected_vis < self.scroll {
            self.scroll = self.selected_vis;
        } else if self.selected_vis >= self.scroll + height {
            self.scroll = self.selected_vis + 1 - height;
        }
        let max_scroll = self.visible.len().saturating_sub(height);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let overflow = self.visible.len() > height;
        let counter_str = if overflow {
            Some(format!(" {}/{}", self.selected_vis + 1, self.visible.len()))
        } else {
            None
        };

        for row_offset in 0..height {
            let vis_idx = self.scroll + row_offset;
            let Some(&node_idx) = self.visible.get(vis_idx) else {
                break;
            };
            let selected = vis_idx == self.selected_vis;
            let row_y = y + row_offset as u16;

            // Reserve space on the first visible outline row for the right-aligned counter if needed
            let right_counter = if row_offset == 0 {
                counter_str.as_deref()
            } else {
                None
            };
            let avail_w = match right_counter {
                Some(c) if width > c.width() + 4 => width - c.width(),
                _ => width,
            };

            let line = self.format_outline_row(node_idx, avail_w, selected, theme);
            buf.set_line(x, row_y, &line, avail_w as u16);

            if let Some(c) = right_counter
                && width > c.width() + 4
            {
                let cx = x + (width - c.width()) as u16;
                buf.set_stringn(
                    cx,
                    row_y,
                    c,
                    c.width(),
                    Style::default().fg(theme.gray_dim),
                );
            }

            if selected {
                buf.set_style(
                    Rect::new(x, row_y, width as u16, 1),
                    Style::default()
                        .bg(theme.bg_highlight)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
    }

    fn format_outline_row(
        &self,
        node_idx: usize,
        width: usize,
        selected: bool,
        theme: &Theme,
    ) -> Line<'static> {
        let Some(row) = self.rows.get(node_idx) else {
            return Line::default();
        };
        let bold = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };

        match row {
            OutlineRow::Step { step_idx } => {
                let Some(step) = self.run.steps.get(*step_idx) else {
                    return Line::default();
                };
                let fold_glyph = if self.tree.has_children(node_idx) {
                    if self.fold.is_collapsed(&step.attempt_id) {
                        "▸"
                    } else {
                        "▾"
                    }
                } else {
                    " "
                };

                // If this is the latest attempt of a node that is currently Running or Waiting, show that state
                let is_last_for_node = self
                    .run
                    .steps
                    .iter()
                    .rposition(|s| s.node_id == step.node_id)
                    == Some(*step_idx);
                let live_state = self.run.node_state(&step.node_id);
                let state = if is_last_for_node
                    && matches!(live_state, NodeState::Running | NodeState::Waiting)
                {
                    live_state
                } else {
                    Self::outcome_state(step.outcome)
                };

                let state_glyph = state.glyph();
                let state_color = Self::node_state_color(state, theme);
                let kind_or_agent = step
                    .agent_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| step.kind.label());
                let dur = fmt_duration(step.duration_ms());
                let result = Self::step_one_line_result(step);

                self.compose_step_line(
                    fold_glyph,
                    state_glyph,
                    state_color,
                    &step.node_id,
                    kind_or_agent,
                    &dur,
                    &result,
                    matches!(step.outcome, Outcome::Failed | Outcome::TimedOut),
                    width,
                    bold,
                    theme,
                )
            }
            OutlineRow::LiveNode {
                node_id,
                kind,
                state,
            } => {
                let state_glyph = state.glyph();
                let state_color = Self::node_state_color(*state, theme);
                let detail = self
                    .run
                    .summary
                    .status_detail
                    .as_deref()
                    .unwrap_or(match state {
                        NodeState::Waiting => "waiting",
                        NodeState::Running => "running",
                        _ => "",
                    });
                self.compose_step_line(
                    " ",
                    state_glyph,
                    state_color,
                    node_id,
                    kind.label(),
                    "…",
                    detail,
                    false,
                    width,
                    bold,
                    theme,
                )
            }
            OutlineRow::Event {
                delta_ms,
                kind,
                detail,
                ..
            } => {
                let prefix = format!("    +{delta_ms}ms  {kind}");
                let prefix_w = prefix.width();
                let dim_style = Style::default().fg(theme.gray).add_modifier(bold);
                let dimmer_style = Style::default().fg(theme.gray_dim).add_modifier(bold);

                if prefix_w >= width {
                    Line::from(Span::styled(truncate_str(&prefix, width), dim_style))
                } else if detail.is_empty() {
                    Line::from(Span::styled(prefix, dim_style))
                } else {
                    let rem = width - prefix_w;
                    let tail = truncate_str(&format!("  {detail}"), rem);
                    Line::from(vec![
                        Span::styled(prefix, dim_style),
                        Span::styled(tail, dimmer_style),
                    ])
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_step_line(
        &self,
        fold_glyph: &str,
        state_glyph: &str,
        state_color: Color,
        node_id: &str,
        kind_or_agent: &str,
        dur: &str,
        result: &str,
        is_err: bool,
        width: usize,
        bold: Modifier,
        theme: &Theme,
    ) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut used = 0usize;

        let push_part = |spans: &mut Vec<Span<'static>>,
                         used: &mut usize,
                         text: &str,
                         style: Style| {
            if *used >= width || text.is_empty() {
                return;
            }
            let rem = width - *used;
            let w = text.width();
            if w <= rem {
                spans.push(Span::styled(text.to_string(), style));
                *used += w;
            } else {
                let cut = truncate_str(text, rem);
                *used += cut.width();
                spans.push(Span::styled(cut, style));
            }
        };

        push_part(
            &mut spans,
            &mut used,
            &format!("{fold_glyph} "),
            Style::default().fg(theme.gray_dim).add_modifier(bold),
        );
        push_part(
            &mut spans,
            &mut used,
            &format!("{state_glyph} "),
            Style::default().fg(state_color).add_modifier(bold),
        );
        push_part(
            &mut spans,
            &mut used,
            node_id,
            Style::default().fg(theme.text_primary).add_modifier(bold),
        );
        push_part(
            &mut spans,
            &mut used,
            &format!("  {kind_or_agent}"),
            Style::default().fg(theme.gray).add_modifier(bold),
        );
        push_part(
            &mut spans,
            &mut used,
            &format!("  {dur}"),
            Style::default().fg(theme.gray_dim).add_modifier(bold),
        );
        if !result.is_empty() {
            let res_color = if is_err {
                theme.accent_error
            } else {
                theme.gray
            };
            push_part(
                &mut spans,
                &mut used,
                &format!("  {result}"),
                Style::default().fg(res_color).add_modifier(bold),
            );
        }

        Line::from(spans)
    }

    fn step_one_line_result(step: &Step) -> String {
        if matches!(step.outcome, Outcome::Failed | Outcome::TimedOut)
            && let Some(err) = step.error.as_deref()
            && !err.is_empty()
        {
            return Self::first_line(err);
        }
        if let Some(route) = step
            .output
            .as_object()
            .and_then(|o| o.get("route"))
            .and_then(|v| v.as_str())
            && !route.is_empty()
        {
            return format!("→ {route}");
        }
        if let Some(act) = step.action.as_ref()
            && (act.action_type == "shell" || act.command.is_some() || act.exit_code.is_some())
            && let Some(code) = act.exit_code
        {
            return format!("exit {code}");
        }
        if let Some(err) = step.error.as_deref()
            && !err.is_empty()
        {
            return Self::first_line(err);
        }
        // An object on one row reads better compact than as the `{` its pretty form starts with.
        match &step.output {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => step.output.to_string(),
            other => Self::first_line(&value_text(other)),
        }
    }

    fn first_line(s: &str) -> String {
        s.lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::acpx::bundle::{
        ActionReceipt, Conversation, FlowDef, FlowEdge, FlowNode, RunSummary,
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut lines = Vec::with_capacity(area.height as usize);
        for y in area.y..area.y + area.height {
            let mut row = String::new();
            for x in area.x..area.x + area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            lines.push(row.trim_end().to_string());
        }
        lines.join("\n")
    }

    fn sample_step(
        attempt_id: &str,
        node_id: &str,
        kind: NodeKind,
        outcome: Outcome,
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
            output: json!({"summary": "done"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn sample_run() -> Run {
        let mut s1 = sample_step("prep#1", "prep", NodeKind::Action, Outcome::Ok);
        s1.action = Some(ActionReceipt {
            action_type: "shell".to_string(),
            command: Some("cargo test".to_string()),
            args: vec![],
            cwd: None,
            exit_code: Some(0),
            duration_ms: Some(1500),
        });

        let mut s2 = sample_step("plan#1", "plan", NodeKind::Acp, Outcome::Failed);
        s2.agent_name = Some("claude".to_string());
        s2.error = Some("rate limit exceeded".to_string());
        s2.conversation = Some(Conversation {
            session_id: "sess-1".to_string(),
            event_start_seq: 1,
            event_end_seq: 5,
        });

        let mut s3 = sample_step("gate#1", "gate", NodeKind::Checkpoint, Outcome::Ok);
        s3.output = json!({"route": "ship"});

        let events = vec![
            TraceEvent {
                seq: 1,
                at_ms: 1_010,
                scope: "node".to_string(),
                kind: "node_started".to_string(),
                node_id: Some("prep".to_string()),
                attempt_id: Some("prep#1".to_string()),
                detail: "starting prep".to_string(),
            },
            TraceEvent {
                seq: 2,
                at_ms: 1_200,
                scope: "node".to_string(),
                kind: "node_heartbeat".to_string(),
                node_id: Some("prep".to_string()),
                attempt_id: Some("prep#1".to_string()),
                detail: "alive".to_string(),
            },
            TraceEvent {
                seq: 3,
                at_ms: 1_500,
                scope: "action".to_string(),
                kind: "action_completed".to_string(),
                node_id: Some("prep".to_string()),
                attempt_id: Some("prep#1".to_string()),
                detail: "exit 0".to_string(),
            },
        ];

        Run {
            summary: RunSummary {
                run_id: "run-42".to_string(),
                dir: PathBuf::from("/tmp/run-42"),
                flow_name: "pr-triage".to_string(),
                title: "Triage PR #42".to_string(),
                status: RunStatus::Waiting,
                started_ms: 1_000,
                updated_ms: 4_000,
                finished_ms: None,
                current_node: Some("gate".to_string()),
                waiting_on: Some("gate".to_string()),
                status_detail: Some("awaiting human approval".to_string()),
                error: None,
            },
            flow: FlowDef {
                name: "pr-triage".to_string(),
                start_at: "prep".to_string(),
                nodes: vec![
                    FlowNode {
                        id: "prep".to_string(),
                        kind: NodeKind::Action,
                        summary: None,
                        is_shell: true,
                    },
                    FlowNode {
                        id: "plan".to_string(),
                        kind: NodeKind::Acp,
                        summary: None,
                        is_shell: false,
                    },
                    FlowNode {
                        id: "gate".to_string(),
                        kind: NodeKind::Checkpoint,
                        summary: None,
                        is_shell: false,
                    },
                ],
                edges: vec![
                    FlowEdge::Simple {
                        from: "prep".to_string(),
                        to: "plan".to_string(),
                    },
                    FlowEdge::Simple {
                        from: "plan".to_string(),
                        to: "gate".to_string(),
                    },
                ],
            },
            steps: vec![s1, s2, s3],
            events,
            input: json!({}),
            sessions: vec![],
        }
    }

    #[test]
    fn initial_selection_starts_on_waiting_step() {
        let view = RunViewState::new(sample_run());
        assert_eq!(view.selected_step(), Some(2));
    }

    #[test]
    fn expand_shows_events_and_hides_heartbeats_until_n() {
        let mut view = RunViewState::new(sample_run());
        view.select_step(0);
        // Initially collapsed: 3 step roots visible
        assert_eq!(view.visible.len(), 3);

        // Expand step 0 ("prep#1"): 2 non-heartbeat events become visible
        assert_eq!(view.handle_key(&key(KeyCode::Right)), RunOutcome::Changed);
        assert_eq!(view.visible.len(), 5);

        // Toggle noise on with `n`: heartbeat event now included
        assert_eq!(view.handle_key(&key(KeyCode::Char('n'))), RunOutcome::Changed);
        assert_eq!(view.visible.len(), 6);
    }

    #[test]
    fn collapse_on_child_jumps_to_parent() {
        let mut view = RunViewState::new(sample_run());
        view.select_step(0);
        view.handle_key(&key(KeyCode::Right));
        // Move down onto the first event child
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(view.selected_vis, 1);

        // First Left on a child jumps selection back to parent root (keep expanded)
        assert_eq!(view.handle_key(&key(KeyCode::Left)), RunOutcome::Changed);
        assert_eq!(view.selected_vis, 0);
        assert_eq!(view.visible.len(), 5);

        // Second Left on the expanded root collapses it
        assert_eq!(view.handle_key(&key(KeyCode::Left)), RunOutcome::Changed);
        assert_eq!(view.visible.len(), 3);
    }

    #[test]
    fn shift_e_toggles_all_folds() {
        let mut view = RunViewState::new(sample_run());
        assert_eq!(view.visible.len(), 3);
        assert_eq!(view.handle_key(&key(KeyCode::Char('E'))), RunOutcome::Changed);
        assert_eq!(view.visible.len(), 5);
        assert_eq!(view.handle_key(&key(KeyCode::Char('E'))), RunOutcome::Changed);
        assert_eq!(view.visible.len(), 3);
    }

    #[test]
    fn bang_jumps_to_next_failed_step_and_wraps() {
        let mut run = sample_run();
        let mut s4 = sample_step("deploy#1", "deploy", NodeKind::Action, Outcome::Failed);
        s4.error = Some("deploy failed".to_string());
        run.steps.push(s4);

        let mut view = RunViewState::new(run);
        // Starts on step 2 ("gate#1")
        assert_eq!(view.selected_step(), Some(2));
        // Next failed is step 3
        assert_eq!(view.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(view.selected_step(), Some(3));
        // Wraps around to step 1
        assert_eq!(view.handle_key(&key(KeyCode::Char('!'))), RunOutcome::Changed);
        assert_eq!(view.selected_step(), Some(1));
    }

    #[test]
    fn c_jumps_to_current_or_waiting_step() {
        let mut view = RunViewState::new(sample_run());
        view.select_step(0);
        assert_eq!(view.selected_step(), Some(0));
        assert_eq!(view.handle_key(&key(KeyCode::Char('c'))), RunOutcome::Changed);
        assert_eq!(view.selected_step(), Some(2));
    }

    #[test]
    fn brackets_jump_between_step_roots() {
        let mut view = RunViewState::new(sample_run());
        view.select_step(0);
        view.handle_key(&key(KeyCode::Right)); // expand step 0 (indices 1, 2 are events; index 3 is step 1)
        assert_eq!(view.handle_key(&key(KeyCode::Char(']'))), RunOutcome::Changed);
        assert_eq!(view.selected_step(), Some(1));
        assert_eq!(view.selected_vis, 3);

        assert_eq!(view.handle_key(&key(KeyCode::Char('['))), RunOutcome::Changed);
        assert_eq!(view.selected_step(), Some(0));
        assert_eq!(view.selected_vis, 0);
    }

    #[test]
    fn enter_and_replay_outcomes() {
        let mut view = RunViewState::new(sample_run());
        // Step 0 is an Action without conversation -> `r` is Unchanged, Enter opens step 0
        view.select_step(0);
        assert_eq!(
            view.handle_key(&key(KeyCode::Enter)),
            RunOutcome::OpenStep(0)
        );
        assert_eq!(
            view.handle_key(&key(KeyCode::Char('r'))),
            RunOutcome::Unchanged
        );

        // Expand step 0, move to child event, Enter still opens parent step 0
        view.handle_key(&key(KeyCode::Right));
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(
            view.handle_key(&key(KeyCode::Enter)),
            RunOutcome::OpenStep(0)
        );

        // Step 1 is an ACP step with conversation -> `r` returns Replay(1)
        view.select_step(1);
        assert_eq!(
            view.handle_key(&key(KeyCode::Char('r'))),
            RunOutcome::Replay(1)
        );
    }

    #[test]
    fn y_copies_selected_step_output_and_esc_returns_back() {
        let mut view = RunViewState::new(sample_run());
        view.select_step(2);
        let outcome = view.handle_key(&key(KeyCode::Char('y')));
        assert_eq!(outcome, RunOutcome::Copy(value_text(&json!({"route": "ship"}))));
        assert_eq!(view.handle_key(&key(KeyCode::Esc)), RunOutcome::Back);
    }

    #[test]
    fn sync_keeps_selection_and_folds_when_step_appended() {
        let mut view = RunViewState::new(sample_run());
        view.select_step(0);
        view.handle_key(&key(KeyCode::Right)); // expand prep#1
        view.select_step(1);
        assert_eq!(view.selected_step(), Some(1));

        // Append a new step and sync
        view.run_mut().steps.push(sample_step(
            "final#1",
            "final",
            NodeKind::Compute,
            Outcome::Ok,
        ));
        view.sync();

        assert_eq!(view.selected_step(), Some(1));
        assert!(!view.fold.is_collapsed(&"prep#1".to_string()));
    }

    #[test]
    fn render_120x30_shows_title_waiting_banner_and_step_rows() {
        let mut view = RunViewState::new(sample_run());
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        view.render(&mut buf, area, &theme, 10_000);

        let text = buffer_text(&buf);
        assert!(text.contains("Triage PR #42"), "missing title in:\n{text}");
        assert!(
            text.contains("WAITING at gate: awaiting human approval"),
            "missing waiting banner in:\n{text}"
        );
        assert!(
            text.contains("acpx cannot resume a run"),
            "missing resume note in:\n{text}"
        );
        assert!(
            text.contains("✓ prep") && text.contains("exit 0"),
            "missing prep step row in:\n{text}"
        );
        assert!(
            text.contains("✗ plan") && text.contains("rate limit exceeded"),
            "missing failed plan step row in:\n{text}"
        );
        assert!(
            text.contains("→ ship"),
            "missing route result on gate step in:\n{text}"
        );
    }

    #[test]
    fn render_with_v_off_hides_graph_and_tiny_rects_do_not_panic() {
        let mut view = RunViewState::new(sample_run());
        let theme = Theme::default();

        // Render with graph on vs off
        let area = Rect::new(0, 0, 120, 30);
        let mut buf_on = Buffer::empty(area);
        view.render(&mut buf_on, area, &theme, 10_000);
        let text_on = buffer_text(&buf_on);

        view.handle_key(&key(KeyCode::Char('v')));
        let mut buf_off = Buffer::empty(area);
        view.render(&mut buf_off, area, &theme, 10_000);
        let text_off = buffer_text(&buf_off);

        // With `v` off, row 2 is immediately the separator line
        let lines_off: Vec<&str> = text_off.lines().collect();
        assert!(
            lines_off.get(2).is_some_and(|l| l.starts_with("───")),
            "expected separator on row 2 when graph is off, got:\n{text_off}"
        );
        assert_ne!(text_on, text_off);

        // 40x8 (< 12 rows auto-hides graph) and 0x0 must not panic
        let small = Rect::new(0, 0, 40, 8);
        let mut buf_small = Buffer::empty(small);
        view.render(&mut buf_small, small, &theme, 10_000);

        let zero = Rect::new(0, 0, 0, 0);
        let mut buf_zero = Buffer::empty(zero);
        view.render(&mut buf_zero, zero, &theme, 10_000);
    }
}
