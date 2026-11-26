//! Pure view-model helpers for the `/acpx` single-run modal: pane zoom/layout,
//! branch-pruned graph views, vertical graph viewport slicing, waiting-gate
//! banners with preceding failure context, and per-step structural badges.

use std::collections::HashSet;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::acpx::bundle::{FlowEdge, Run};
use crate::acpx::step_digest::{
    active_branch_nodes, digest_step_files, extract_step_validations, extract_step_verdict,
    latest_run_failure, non_zero_exit_code, parse_scoped_node_id, step_attempt_budget,
    step_time_budget, step_trace_link,
};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;

/// Which pane of the `/acpx` single-run view is zoomed/maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalZoom {
    #[default]
    Split,
    Graph,
    Detail,
}

impl ModalZoom {
    pub fn toggle_graph(self) -> Self {
        match self {
            Self::Graph => Self::Split,
            Self::Split | Self::Detail => Self::Graph,
        }
    }

    pub fn toggle_detail(self) -> Self {
        match self {
            Self::Detail => Self::Split,
            Self::Split | Self::Graph => Self::Detail,
        }
    }

    pub fn header_badge(self) -> Option<&'static str> {
        match self {
            Self::Split => None,
            Self::Graph => Some("GRAPH ZOOM (m/Esc)"),
            Self::Detail => Some("DETAIL ZOOM (z/Esc)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBodyLayout {
    pub graph_rows: usize,
    pub show_separator: bool,
    pub stages_rect: Option<Rect>,
    pub detail_rect: Option<Rect>,
    pub vertical_sep_x: Option<u16>,
}

/// Compute the layout below the header and banner at row `y_after_banner` within `area`.
pub fn compute_run_body_layout(
    area: Rect,
    y_after_banner: u16,
    actual_graph_lines: usize,
    show_graph: bool,
    zoom: ModalZoom,
) -> RunBodyLayout {
    let remaining_h = area
        .y
        .saturating_add(area.height)
        .saturating_sub(y_after_banner);

    if remaining_h == 0 || area.width == 0 {
        return RunBodyLayout {
            graph_rows: 0,
            show_separator: false,
            stages_rect: None,
            detail_rect: None,
            vertical_sep_x: None,
        };
    }

    match zoom {
        ModalZoom::Graph => RunBodyLayout {
            graph_rows: remaining_h as usize,
            show_separator: false,
            stages_rect: None,
            detail_rect: None,
            vertical_sep_x: None,
        },
        ModalZoom::Detail => RunBodyLayout {
            graph_rows: 0,
            show_separator: false,
            stages_rect: None,
            detail_rect: Some(Rect::new(area.x, y_after_banner, area.width, remaining_h)),
            vertical_sep_x: None,
        },
        ModalZoom::Split => {
            let max_graph_budget = if show_graph && area.height >= 16 {
                ((area.height as usize) * 2 / 5)
                    .max(1)
                    .min((remaining_h as usize).saturating_sub(3))
            } else {
                0
            };

            let graph_rows = actual_graph_lines.min(max_graph_budget);
            let show_separator = (remaining_h as usize) > graph_rows;
            let body_y = y_after_banner
                .saturating_add(graph_rows as u16)
                .saturating_add(u16::from(show_separator));
            let body_h = area.y.saturating_add(area.height).saturating_sub(body_y);

            if body_h == 0 {
                return RunBodyLayout {
                    graph_rows,
                    show_separator,
                    stages_rect: None,
                    detail_rect: None,
                    vertical_sep_x: None,
                };
            }

            if area.width >= 110 {
                let left_w = ((area.width as u32 * 2 / 5) as u16).clamp(44, 72);
                let sep_x = area.x.saturating_add(left_w);
                let detail_x = sep_x.saturating_add(1);
                let detail_w = area.x.saturating_add(area.width).saturating_sub(detail_x);

                RunBodyLayout {
                    graph_rows,
                    show_separator,
                    stages_rect: Some(Rect::new(area.x, body_y, left_w, body_h)),
                    detail_rect: Some(Rect::new(detail_x, body_y, detail_w, body_h)),
                    vertical_sep_x: Some(sep_x),
                }
            } else {
                RunBodyLayout {
                    graph_rows,
                    show_separator,
                    stages_rect: Some(Rect::new(area.x, body_y, area.width, body_h)),
                    detail_rect: None,
                    vertical_sep_x: None,
                }
            }
        }
    }
}

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

/// Slice a rendered graph (`lines`) to fit `budget_rows` at `scroll_y`, returning `(visible_lines, clamped_scroll_y)`.
pub fn slice_graph_viewport(
    lines: &[Line<'static>],
    budget_rows: usize,
    scroll_y: usize,
    zoom: ModalZoom,
    width: usize,
    theme: &Theme,
) -> (Vec<Line<'static>>, usize) {
    if budget_rows == 0 || lines.is_empty() {
        return (Vec::new(), 0);
    }

    if lines.len() <= budget_rows {
        return (lines.to_vec(), 0);
    }

    let content_rows = budget_rows.saturating_sub(1);
    let hint_style = Style::default().fg(theme.gray_dim);

    match zoom {
        ModalZoom::Graph => {
            let max_scroll = lines.len().saturating_sub(content_rows);
            let clamped = scroll_y.min(max_scroll);
            let mut visible: Vec<Line<'static>> = lines
                .iter()
                .skip(clamped)
                .take(content_rows)
                .cloned()
                .collect();

            let start_row = clamped.saturating_add(1).min(lines.len());
            let end_row = clamped.saturating_add(content_rows).min(lines.len());
            let hint = if content_rows == 0 {
                format!(
                    "↕ {} graph lines (j/k scroll, m/Esc back)",
                    lines.len()
                )
            } else {
                format!(
                    "↕ rows {start_row}-{end_row}/{} (j/k scroll, m/Esc back)",
                    lines.len()
                )
            };
            visible.push(Line::from(Span::styled(
                truncate_str(&hint, width),
                hint_style,
            )));
            (visible, clamped)
        }
        ModalZoom::Split | ModalZoom::Detail => {
            let mut visible: Vec<Line<'static>> =
                lines.iter().take(content_rows).cloned().collect();
            let hidden = lines.len().saturating_sub(content_rows);
            let hint = format!("… press m to maximize graph ({hidden} more)");
            visible.push(Line::from(Span::styled(
                truncate_str(&hint, width),
                hint_style,
            )));
            (visible, 0)
        }
    }
}

/// Build the multi-line banner for a waiting gate in `/acpx`, automatically querying
/// `crate::acpx::step_digest::latest_run_failure(run)` for the preceding failure summary.
pub fn build_waiting_gate_banner(
    run: &Run,
    node: &str,
    question: Option<&str>,
    choice_count: usize,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let mut waiting_msg = format!("◐ WAITING FOR YOUR ANSWER at {node}");
    if let Some(q) = question.map(str::trim)
        && !q.is_empty()
        && q != node
    {
        waiting_msg.push_str(": ");
        waiting_msg.push_str(q);
    }
    if choice_count > 0 {
        let noun = if choice_count == 1 {
            "choice"
        } else {
            "choices"
        };
        waiting_msg.push_str(&format!(" ({choice_count} {noun})"));
    }

    let warn_style = Style::default()
        .fg(theme.warning)
        .add_modifier(Modifier::BOLD);
    let mut out: Vec<Line<'static>> = wrap_text(&waiting_msg, width)
        .into_iter()
        .map(|segment| Line::from(Span::styled(segment, warn_style)))
        .collect();

    if let Some((failed_node, exit_code, summary)) = latest_run_failure(run) {
        let compact_summary = compact_inline_summary(&summary);
        let failure_msg = match (exit_code, compact_summary.is_empty()) {
            (Some(code), false) => {
                format!("✗ Preceding failure in {failed_node} (exit {code}): {compact_summary}")
            }
            (Some(code), true) => {
                format!("✗ Preceding failure in {failed_node} (exit {code})")
            }
            (None, false) => {
                format!("✗ Preceding failure in {failed_node}: {compact_summary}")
            }
            (None, true) => {
                format!("✗ Preceding failure in {failed_node}")
            }
        };

        let err_style = Style::default()
            .fg(theme.accent_error)
            .add_modifier(Modifier::BOLD);
        for segment in wrap_text(&failure_msg, width) {
            out.push(Line::from(Span::styled(segment, err_style)));
        }
    }

    out
}

/// Build a compact structural + trace-link annotation string for `run.steps[step_index]` at `now_ms`.
pub fn format_step_structure_annotation(run: &Run, step_index: usize, now_ms: i64) -> String {
    let Some(step) = run.steps.get(step_index) else {
        return String::new();
    };

    let mut parts: Vec<String> = Vec::new();

    // (f) Named include exit badge
    let flow_node = run.flow.nodes.iter().find(|n| n.id == step.node_id);
    if flow_node.and_then(|n| n.include_transition.as_deref()) == Some("exit") {
        let exit_name = flow_node
            .and_then(|n| n.local_node_id.as_deref())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                parse_scoped_node_id(&step.node_id)
                    .2
                    .filter(|s| !s.is_empty())
            });
        if let Some(exit_name) = exit_name {
            parts.push(format!("exit:{exit_name}"));
        }
    }

    // (a) Loop attempt count badge
    if let Some(badge) = step_attempt_budget(run, step).badge_label()
        && !badge.is_empty()
    {
        parts.push(badge);
    }

    // (d) Time budget / timeout badge
    let time_budget = step_time_budget(run, step, now_ms);
    if time_budget.timeout_ms.is_some() || time_budget.timed_out {
        let badge = time_budget.badge_label();
        if !badge.is_empty() {
            parts.push(badge);
        }
    }

    // Non-zero exit code badge
    if let Some(code) = non_zero_exit_code(step) {
        parts.push(format!("exit {code}"));
    }

    // (b) Validator rejections badge
    if let Some(badge) = extract_step_validations(run, step, &[]).badge_label()
        && !badge.is_empty()
    {
        parts.push(badge);
    }

    // (c) Reviewer / router verdict summary
    if let Some(verdict) = extract_step_verdict(run, step_index) {
        let summary = verdict.summary_line();
        if !summary.is_empty() {
            parts.push(summary);
        }
    }

    // File progress / scope summary
    let file_summary = digest_step_files(run, step, &[]).summary_line();
    if !file_summary.is_empty() {
        parts.push(file_summary);
    }

    // Trace link badge
    if let Some(badge) = step_trace_link(run, step).badge_label()
        && !badge.is_empty()
    {
        parts.push(badge);
    }

    parts.join(" · ")
}

fn compact_inline_summary(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return Vec::new();
    }
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for word in text.split_whitespace() {
        let word_w = UnicodeWidthStr::width(word);
        if current.is_empty() {
            if word_w <= max_width {
                current.push_str(word);
                current_w = word_w;
            } else {
                push_broken_word(&mut lines, &mut current, &mut current_w, word, max_width);
            }
        } else if current_w.saturating_add(1).saturating_add(word_w) <= max_width {
            current.push(' ');
            current.push_str(word);
            current_w = current_w.saturating_add(1).saturating_add(word_w);
        } else {
            lines.push(std::mem::take(&mut current));
            current_w = 0;
            if word_w <= max_width {
                current.push_str(word);
                current_w = word_w;
            } else {
                push_broken_word(&mut lines, &mut current, &mut current_w, word, max_width);
            }
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }

    lines
}

fn push_broken_word(
    lines: &mut Vec<String>,
    current: &mut String,
    current_w: &mut usize,
    word: &str,
    max_width: usize,
) {
    for ch in word.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if *current_w > 0 && current_w.saturating_add(cw) > max_width {
            lines.push(std::mem::take(current));
            *current_w = 0;
        }
        current.push(ch);
        *current_w = current_w.saturating_add(cw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::{Value, json};

    use crate::acpx::bundle::{
        ActionReceipt, Conversation, FlowDef, FlowMount, FlowNode, NodeKind, Outcome, RunStatus,
        RunSummary, SessionRef, Step,
    };

    fn make_node(id: &str, kind: NodeKind) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            kind,
            summary: None,
            is_shell: false,
            timeout_ms: None,
            mount_path: Vec::new(),
            local_node_id: None,
            include_transition: None,
        }
    }

    fn make_step(attempt_id: &str, node_id: &str, kind: NodeKind, outcome: Outcome) -> Step {
        Step {
            attempt_id: attempt_id.to_string(),
            node_id: node_id.to_string(),
            kind,
            outcome,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn make_run(nodes: Vec<FlowNode>, edges: Vec<FlowEdge>, steps: Vec<Step>) -> Run {
        let start_at = nodes.first().map(|n| n.id.clone()).unwrap_or_default();
        Run {
            summary: RunSummary {
                run_id: "run-1".to_string(),
                dir: PathBuf::from("/tmp/run-1"),
                flow_name: "test-flow".to_string(),
                title: "Test Flow".to_string(),
                status: RunStatus::Running,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "test-flow".to_string(),
                start_at,
                nodes,
                edges,
                max_steps: Some(100),
                mounts: Vec::<FlowMount>::new(),
            },
            steps,
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn modal_zoom_toggles_cleanly_between_modes() {
        assert_eq!(ModalZoom::default(), ModalZoom::Split);
        assert_eq!(ModalZoom::Split.header_badge(), None);

        let graph = ModalZoom::Split.toggle_graph();
        assert_eq!(graph, ModalZoom::Graph);
        assert!(graph.header_badge().is_some());
        assert_eq!(graph.toggle_graph(), ModalZoom::Split);

        let detail = ModalZoom::Split.toggle_detail();
        assert_eq!(detail, ModalZoom::Detail);
        assert!(detail.header_badge().is_some());
        assert_eq!(detail.toggle_detail(), ModalZoom::Split);

        assert_eq!(ModalZoom::Graph.toggle_detail(), ModalZoom::Detail);
        assert_eq!(ModalZoom::Detail.toggle_graph(), ModalZoom::Graph);
    }

    #[test]
    fn compute_run_body_layout_handles_graph_detail_and_split_modes() {
        let area = Rect::new(0, 0, 120, 40);
        let y_after_banner = 2;

        // Graph zoom gets 100% of remaining height.
        let graph_layout =
            compute_run_body_layout(area, y_after_banner, 10, true, ModalZoom::Graph);
        assert_eq!(graph_layout.graph_rows, 38);
        assert!(!graph_layout.show_separator);
        assert_eq!(graph_layout.stages_rect, None);
        assert_eq!(graph_layout.detail_rect, None);
        assert_eq!(graph_layout.vertical_sep_x, None);

        // Detail zoom gets 100% of width and remaining height.
        let detail_layout =
            compute_run_body_layout(area, y_after_banner, 10, true, ModalZoom::Detail);
        assert_eq!(detail_layout.graph_rows, 0);
        assert!(!detail_layout.show_separator);
        assert_eq!(detail_layout.stages_rect, None);
        assert_eq!(detail_layout.detail_rect, Some(Rect::new(0, 2, 120, 38)));
        assert_eq!(detail_layout.vertical_sep_x, None);

        // Split at width 120 gets 2/5 + 3/5 split below graph + separator.
        let split_layout =
            compute_run_body_layout(area, y_after_banner, 10, true, ModalZoom::Split);
        assert_eq!(split_layout.graph_rows, 10);
        assert!(split_layout.show_separator);
        assert_eq!(split_layout.vertical_sep_x, Some(48));
        assert_eq!(split_layout.stages_rect, Some(Rect::new(0, 13, 48, 27)));
        assert_eq!(split_layout.detail_rect, Some(Rect::new(49, 13, 71, 27)));

        // Narrow split omits detail pane.
        let narrow = compute_run_body_layout(Rect::new(0, 0, 90, 30), 2, 6, true, ModalZoom::Split);
        assert_eq!(narrow.graph_rows, 6);
        assert_eq!(narrow.stages_rect, Some(Rect::new(0, 9, 90, 21)));
        assert_eq!(narrow.detail_rect, None);
        assert_eq!(narrow.vertical_sep_x, None);

        // Zero remaining height returns empty layout.
        let empty = compute_run_body_layout(area, 40, 10, true, ModalZoom::Split);
        assert_eq!(empty.graph_rows, 0);
        assert!(!empty.show_separator);
        assert_eq!(empty.stages_rect, None);
        assert_eq!(empty.detail_rect, None);
    }

    #[test]
    fn prune_run_for_graph_removes_untaken_switch_branch() {
        let nodes = vec![
            make_node("route_host", NodeKind::Compute),
            make_node("mini", NodeKind::Action),
            make_node("air", NodeKind::Action),
            make_node("finish", NodeKind::Acp),
        ];
        let edges = vec![
            FlowEdge::Switch {
                from: "route_host".to_string(),
                on: "host".to_string(),
                cases: vec![
                    ("mini".to_string(), "mini".to_string()),
                    ("air".to_string(), "air".to_string()),
                ],
            },
            FlowEdge::Simple {
                from: "mini".to_string(),
                to: "finish".to_string(),
            },
            FlowEdge::Simple {
                from: "air".to_string(),
                to: "finish".to_string(),
            },
        ];
        let steps = vec![
            make_step("a1", "route_host", NodeKind::Compute, Outcome::Ok),
            make_step("a2", "mini", NodeKind::Action, Outcome::Ok),
        ];

        let run = make_run(nodes, edges, steps);
        let pruned = prune_run_for_graph(&run);

        let kept_ids: Vec<&str> = pruned.flow.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(kept_ids.contains(&"route_host"));
        assert!(kept_ids.contains(&"mini"));
        assert!(kept_ids.contains(&"finish"));
        assert!(!kept_ids.contains(&"air"));

        let switch_cases: Vec<(&str, &str)> = pruned
            .flow
            .edges
            .iter()
            .filter_map(|e| match e {
                FlowEdge::Switch { from, cases, .. } if from == "route_host" => Some(
                    cases
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(switch_cases, vec![("mini", "mini")]);

        assert!(
            !pruned
                .flow
                .edges
                .iter()
                .any(|e| matches!(e, FlowEdge::Simple { from, .. } if from == "air"))
        );
    }

    #[test]
    fn slice_graph_viewport_scrolls_in_graph_zoom_and_hints_in_split() {
        let theme = Theme::default();
        let lines: Vec<Line<'static>> = (0..12)
            .map(|i| Line::from(format!("graph-row-{i}")))
            .collect();

        // Graph zoom scrolls vertically and clamps at the end.
        let (vis_top, scroll_top) =
            slice_graph_viewport(&lines, 5, 0, ModalZoom::Graph, 80, &theme);
        assert_eq!(scroll_top, 0);
        assert_eq!(vis_top.len(), 5);
        assert_eq!(line_text(&vis_top[0]), "graph-row-0");
        assert_eq!(line_text(&vis_top[3]), "graph-row-3");
        assert!(line_text(&vis_top[4]).contains("1-4/12"));

        let (vis_mid, scroll_mid) =
            slice_graph_viewport(&lines, 5, 4, ModalZoom::Graph, 80, &theme);
        assert_eq!(scroll_mid, 4);
        assert_eq!(line_text(&vis_mid[0]), "graph-row-4");
        assert_eq!(line_text(&vis_mid[3]), "graph-row-7");

        let (vis_end, scroll_end) =
            slice_graph_viewport(&lines, 5, 99, ModalZoom::Graph, 80, &theme);
        assert_eq!(scroll_end, 8);
        assert_eq!(line_text(&vis_end[0]), "graph-row-8");
        assert_eq!(line_text(&vis_end[3]), "graph-row-11");

        // Split mode truncates and appends the maximize hint.
        let (vis_split, scroll_split) =
            slice_graph_viewport(&lines, 5, 3, ModalZoom::Split, 80, &theme);
        assert_eq!(scroll_split, 0);
        assert_eq!(vis_split.len(), 5);
        assert_eq!(line_text(&vis_split[0]), "graph-row-0");
        assert!(line_text(&vis_split[4]).contains("press m to maximize graph"));

        // Fits without truncation.
        let (vis_all, scroll_all) =
            slice_graph_viewport(&lines[..3], 5, 2, ModalZoom::Graph, 80, &theme);
        assert_eq!(scroll_all, 0);
        assert_eq!(vis_all.len(), 3);
    }

    #[test]
    fn build_waiting_gate_banner_renders_warning_and_preceding_failure() {
        let theme = Theme::default();
        let nodes = vec![
            make_node("cargo_check", NodeKind::Action),
            make_node("human_gate", NodeKind::Checkpoint),
        ];
        let edges = vec![FlowEdge::Simple {
            from: "cargo_check".to_string(),
            to: "human_gate".to_string(),
        }];

        let mut failed_step = make_step("a1", "cargo_check", NodeKind::Action, Outcome::Failed);
        failed_step.error = Some("error[E0425]: cannot find value `x` in this scope".to_string());
        failed_step.action = Some(ActionReceipt {
            action_type: "shell".to_string(),
            command: Some("cargo".to_string()),
            args: vec!["check".to_string()],
            cwd: None,
            exit_code: Some(101),
            duration_ms: Some(1_500),
        });

        let mut run = make_run(nodes, edges, vec![failed_step]);
        run.summary.status = RunStatus::Waiting;
        run.summary.waiting_on = Some("human_gate".to_string());

        let banner = build_waiting_gate_banner(
            &run,
            "human_gate",
            Some("How should we proceed?"),
            2,
            120,
            &theme,
        );
        let texts: Vec<String> = banner.iter().map(line_text).collect();
        let joined = texts.join("\n");

        assert!(
            joined.contains("WAITING FOR YOUR ANSWER"),
            "missing waiting header: {joined}"
        );
        assert!(
            joined.contains("✗ Preceding failure in cargo_check (exit 101):"),
            "missing preceding failure line: {joined}"
        );
        assert!(
            joined.contains("cannot find value `x`"),
            "missing failure detail: {joined}"
        );
    }

    #[test]
    fn format_step_structure_annotation_combines_all_badges_and_trace_link() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("acpx-modal-view-test-{unique}"));
        fs::create_dir_all(&temp_dir).unwrap();
        let events_path = temp_dir.join("events.ndjson");

        let ndjson = [
            json!({
                "seq": 1,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tc1",
                            "title": "submit_workflow_output",
                            "kind": "other",
                            "status": "in_progress",
                            "rawInput": {"verdict": "bad"},
                            "rawOutput": null,
                            "truncated": false
                        }
                    }
                }
            })
            .to_string(),
            json!({
                "seq": 2,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": "tc1",
                            "title": "submit_workflow_output",
                            "kind": "other",
                            "status": "failed",
                            "rawInput": {"verdict": "bad"},
                            "rawOutput": {"error": "schema mismatch"},
                            "truncated": false
                        }
                    }
                }
            })
            .to_string(),
            json!({
                "seq": 3,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tc2",
                            "title": "edit",
                            "kind": "edit",
                            "status": "completed",
                            "rawInput": {"path": "src/lib.rs"},
                            "rawOutput": {"ok": true},
                            "truncated": false
                        }
                    }
                }
            })
            .to_string(),
            json!({
                "seq": 4,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tc3",
                            "title": "submit_workflow_output",
                            "kind": "other",
                            "status": "completed",
                            "rawInput": {"verdict": "approve", "reason": "all checks green"},
                            "rawOutput": {"ok": true},
                            "truncated": false
                        }
                    }
                }
            })
            .to_string(),
        ]
        .join("\n");
        fs::write(&events_path, ndjson).unwrap();

        let mut review_node = make_node("review", NodeKind::Acp);
        review_node.timeout_ms = Some(60_000);
        let done_node = make_node("done", NodeKind::Compute);
        let fix_node = make_node("fix", NodeKind::Acp);

        let edges = vec![FlowEdge::Switch {
            from: "review".to_string(),
            on: "verdict".to_string(),
            cases: vec![
                ("approve".to_string(), "done".to_string()),
                ("reject".to_string(), "fix".to_string()),
            ],
        }];

        let step1 = make_step("a1", "review", NodeKind::Acp, Outcome::Ok);
        let mut step2 = make_step("a2", "review", NodeKind::Acp, Outcome::Ok);
        step2.started_ms = 10_000;
        step2.finished_ms = 22_000;
        step2.prompt_text = Some("Update `src/lib.rs` and submit your verdict.".to_string());
        step2.output = json!({
            "verdict": "approve",
            "reason": "all checks green"
        });
        step2.conversation = Some(Conversation {
            session_id: "main".to_string(),
            event_start_seq: 1,
            event_end_seq: 5,
        });

        let mut run = make_run(
            vec![review_node, done_node, fix_node],
            edges,
            vec![step1, step2],
        );
        run.sessions.push(SessionRef {
            id: "main".to_string(),
            handle: "main".to_string(),
            events_path: events_path.clone(),
            agent_name: Some("reviewer".to_string()),
        });

        let annotation = format_step_structure_annotation(&run, 1, 25_000);

        // Clean up temp files before assertions so cleanup always runs.
        let _ = fs::remove_dir_all(&temp_dir);

        assert!(
            annotation.contains("2/2") || annotation.contains("attempt"),
            "missing attempt badge in: {annotation}"
        );
        assert!(
            annotation.contains("12s") || annotation.contains("1m"),
            "missing time budget badge in: {annotation}"
        );
        assert!(
            annotation.contains("validation rejected"),
            "missing validator rejection badge in: {annotation}"
        );
        assert!(
            annotation.contains("approve"),
            "missing reviewer verdict in: {annotation}"
        );
        assert!(
            annotation.contains("1/1") || annotation.contains("src/lib.rs"),
            "missing file digest in: {annotation}"
        );
        assert!(
            annotation.contains("trace: events 1..5"),
            "missing trace link badge in: {annotation}"
        );
        assert_eq!(format_step_structure_annotation(&run, 99, 25_000), "");
    }

    #[test]
    fn format_step_structure_annotation_includes_exit_name_and_exit_code() {
        let mut exit_node = make_node("child/__piw_exit_passed", NodeKind::Compute);
        exit_node.mount_path = vec!["child".to_string()];
        exit_node.local_node_id = Some("passed".to_string());
        exit_node.include_transition = Some("exit".to_string());

        let mut step = make_step(
            "a1",
            "child/__piw_exit_passed",
            NodeKind::Compute,
            Outcome::Failed,
        );
        step.action = Some(ActionReceipt {
            action_type: "shell".to_string(),
            command: Some("sh".to_string()),
            args: Vec::new(),
            cwd: None,
            exit_code: Some(2),
            duration_ms: Some(100),
        });

        let run = make_run(vec![exit_node], Vec::new(), vec![step]);
        let annotation = format_step_structure_annotation(&run, 0, 5_000);

        assert!(annotation.contains("exit:passed"), "got: {annotation}");
        assert!(annotation.contains("exit 2"), "got: {annotation}");
    }
}
