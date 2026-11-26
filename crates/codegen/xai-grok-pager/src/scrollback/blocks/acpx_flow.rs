use std::path::Path;
use std::time::Duration;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use serde_json::Value;

use crate::acpx::bundle::{
    NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration, value_text,
};
use crate::appearance::AppearanceConfig;
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{AccentStyle, BlockContext, BlockOutput, DisplayMode};
use crate::theme::Theme;
use crate::util::format_duration;

#[derive(Debug, Clone, PartialEq)]
pub enum AcpxFlowStatus {
    Starting,
    Running,
    Waiting { node: String, detail: String },
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpxFlowRow {
    pub node_id: String,
    pub label: String,
    pub state: NodeState,
    pub duration_ms: Option<u64>,
    pub result: String,
}

#[derive(Debug, Clone)]
pub struct AcpxFlowBlock {
    pub flow_file: String,
    pub title: String,
    pub run_id: Option<String>,
    pub status: AcpxFlowStatus,
    pub rows: Vec<AcpxFlowRow>,
    pub elapsed: Duration,
    pub note: Option<String>,
}

impl AcpxFlowBlock {
    pub fn started(flow_file: impl Into<String>) -> Self {
        let flow_file = flow_file.into();
        let title = Path::new(&flow_file)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&flow_file)
            .to_string();

        Self {
            flow_file,
            title,
            run_id: None,
            status: AcpxFlowStatus::Starting,
            rows: Vec::new(),
            elapsed: Duration::ZERO,
            note: None,
        }
    }

    pub fn update_from_run(&mut self, run: &Run, now_ms: i64) {
        self.title = run.summary.title.clone();
        self.run_id = Some(run.summary.run_id.clone());
        self.elapsed = Duration::from_millis(run.duration_ms(now_ms));

        self.rows = run
            .flow
            .nodes
            .iter()
            .map(|node| {
                let state = run.node_state(&node.id);
                let last_step = run.steps.iter().rev().find(|s| s.node_id == node.id);

                let label = if node.kind == NodeKind::Acp {
                    last_step
                        .and_then(|s| s.agent_name.as_deref())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| node.kind.label())
                        .to_string()
                } else {
                    node.kind.label().to_string()
                };

                let duration_ms = last_step.map(Step::duration_ms);
                let result = last_step.map(step_result_text).unwrap_or_default();

                AcpxFlowRow {
                    node_id: node.id.clone(),
                    label,
                    state,
                    duration_ms,
                    result,
                }
            })
            .collect();

        if !matches!(self.status, AcpxFlowStatus::Cancelled) {
            match run.summary.status {
                RunStatus::Running => {
                    self.status = AcpxFlowStatus::Running;
                }
                RunStatus::Waiting => {
                    let node = run
                        .summary
                        .waiting_on
                        .clone()
                        .or_else(|| run.summary.current_node.clone())
                        .unwrap_or_default();
                    let detail = run.summary.status_detail.clone().unwrap_or_default();
                    self.status = AcpxFlowStatus::Waiting { node, detail };
                }
                RunStatus::Completed => {
                    self.status = AcpxFlowStatus::Completed;
                }
                RunStatus::Failed | RunStatus::TimedOut => {
                    let msg = run
                        .summary
                        .error
                        .clone()
                        .or_else(|| run.summary.status_detail.clone())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "failed".to_string());
                    self.status = AcpxFlowStatus::Failed(msg);
                }
                RunStatus::Unknown => {}
            }
        }
    }

    pub fn set_status(&mut self, status: AcpxFlowStatus) {
        self.status = status;
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.status,
            AcpxFlowStatus::Starting | AcpxFlowStatus::Running
        )
    }

    fn done_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r.state, NodeState::Done(_)))
            .count()
    }

    fn header_line(&self, theme: &Theme, is_selected: bool) -> Line<'static> {
        let bold = if is_selected {
            theme.primary().add_modifier(Modifier::BOLD)
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let muted = theme.muted();
        let text_style = if matches!(self.status, AcpxFlowStatus::Cancelled) {
            theme.dim()
        } else {
            muted
        };

        let elapsed = format_duration(self.elapsed);
        let status_text = match &self.status {
            AcpxFlowStatus::Starting => format!("{} starting…", self.title),
            AcpxFlowStatus::Running => {
                let done = self.done_count();
                let total = self.rows.len();
                format!("{} running · {done}/{total} · {elapsed}", self.title)
            }
            AcpxFlowStatus::Waiting { node, .. } => {
                if node.is_empty() {
                    format!("{} waiting · {elapsed}", self.title)
                } else {
                    format!("{} waiting at {node} · {elapsed}", self.title)
                }
            }
            AcpxFlowStatus::Completed => format!("{} done in {elapsed}", self.title),
            AcpxFlowStatus::Failed(_) => format!("{} failed after {elapsed}", self.title),
            AcpxFlowStatus::Cancelled => format!("{} ◌ cancelled after {elapsed}", self.title),
        };

        Line::from(vec![
            Span::styled("acpx ", bold),
            Span::styled(status_text, text_style),
        ])
    }

    fn lines(&self, selected: bool, collapsed: bool) -> Vec<Line<'static>> {
        let theme = Theme::current();
        let header = self.header_line(&theme, selected);
        if collapsed {
            return vec![header];
        }

        let mut out = vec![header];
        let muted = theme.muted();
        let dim = theme.dim();
        let error_style = Style::default().fg(theme.accent_error);
        let warn_style = Style::default().fg(theme.warning);

        let max_id_len = self
            .rows
            .iter()
            .map(|r| r.node_id.chars().count())
            .max()
            .unwrap_or(0);

        for row in &self.rows {
            let glyph_style = match row.state {
                NodeState::Running => Style::default().fg(theme.accent_running),
                NodeState::Waiting => warn_style,
                NodeState::Done(Outcome::Ok) => Style::default().fg(theme.accent_success),
                NodeState::Done(Outcome::Failed | Outcome::TimedOut) => error_style,
                NodeState::NotRun
                | NodeState::Done(Outcome::Cancelled | Outcome::Unknown) => muted,
            };

            let pad = max_id_len.saturating_sub(row.node_id.chars().count());
            let padded_id = format!("{}{}", row.node_id, " ".repeat(pad));
            let dur_str = row.duration_ms.map(fmt_duration).unwrap_or_default();

            let is_err = matches!(
                row.state,
                NodeState::Done(Outcome::Failed | Outcome::TimedOut)
            );
            let res_style = if is_err { error_style } else { muted };

            let mut spans = vec![
                Span::styled(format!("  {} ", row.state.glyph()), glyph_style),
                Span::styled(padded_id, muted),
                Span::styled(format!("  {}", row.label), muted),
            ];
            if !dur_str.is_empty() {
                spans.push(Span::styled(format!("  {dur_str}"), muted));
            }
            if !row.result.is_empty() {
                spans.push(Span::styled(format!("  {}", row.result), res_style));
            }
            out.push(Line::from(spans));
        }

        match &self.status {
            AcpxFlowStatus::Waiting { detail, .. } => {
                let text = if detail.is_empty() {
                    "  ◐ waiting".to_string()
                } else {
                    format!("  ◐ {}", sanitize_one_line(detail))
                };
                out.push(Line::from(Span::styled(text, warn_style)));
                out.push(Line::from(Span::styled(
                    "  acpx cannot resume a run; re-run the flow to continue",
                    dim,
                )));
            }
            AcpxFlowStatus::Failed(msg) => {
                let text = if msg.is_empty() {
                    "  ✗ failed".to_string()
                } else {
                    format!("  ✗ {}", sanitize_one_line(msg))
                };
                out.push(Line::from(Span::styled(text, error_style)));
            }
            _ => {}
        }

        if let Some(note) = &self.note
            && !note.is_empty()
        {
            out.push(Line::from(Span::styled(
                format!("  {}", sanitize_one_line(note)),
                dim,
            )));
        }

        if let Some(run_id) = &self.run_id
            && !run_id.is_empty()
        {
            out.push(Line::from(Span::styled(
                format!("  /acpx {run_id} opens this run"),
                dim,
            )));
        }

        out
    }
}

fn step_result_text(step: &Step) -> String {
    if let Some(err) = &step.error
        && !err.trim().is_empty()
    {
        return sanitize_one_line(err);
    }

    if let Value::Object(map) = &step.output
        && let Some(route) = map.get("route").and_then(Value::as_str)
        && !route.is_empty()
    {
        return format!("→ {}", sanitize_one_line(route));
    }

    if let Some(code) = step.action.as_ref().and_then(|a| a.exit_code) {
        return format!("exit {code}");
    }

    match &step.output {
        Value::Null => String::new(),
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string(&step.output).unwrap_or_default()
        }
        other => {
            let txt = value_text(other);
            txt.lines().next().unwrap_or("").trim().to_string()
        }
    }
}

fn sanitize_one_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

impl BlockContent for AcpxFlowBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let collapsed = matches!(ctx.mode, DisplayMode::Collapsed);
        let lines = self
            .lines(ctx.is_selected, collapsed)
            .into_iter()
            .map(Into::into)
            .collect();
        BlockOutput { lines }
    }

    fn accent(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match &self.status {
            AcpxFlowStatus::Starting | AcpxFlowStatus::Running if ctx.is_running => {
                Some(AccentStyle::static_color(theme.accent_running))
            }
            AcpxFlowStatus::Failed(_) => Some(AccentStyle::static_color(theme.accent_error)),
            _ => None,
        }
    }

    fn bullet(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match &self.status {
            AcpxFlowStatus::Starting | AcpxFlowStatus::Running => {
                if ctx.is_running {
                    Some(AccentStyle::animated_running(ctx, &theme))
                } else {
                    Some(AccentStyle::static_color(theme.accent_running))
                }
            }
            AcpxFlowStatus::Completed => Some(AccentStyle::static_color(theme.accent_success)),
            AcpxFlowStatus::Failed(_) => Some(AccentStyle::static_color(theme.accent_error)),
            AcpxFlowStatus::Cancelled => Some(AccentStyle::static_color(theme.gray_dim)),
            AcpxFlowStatus::Waiting { .. } => Some(AccentStyle::static_color(theme.warning)),
        }
    }

    fn has_vpad_for(&self, _appearance: &AppearanceConfig) -> bool {
        false
    }

    fn has_raw_mode(&self) -> bool {
        false
    }

    fn is_foldable(&self) -> bool {
        true
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Expanded
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn has_bullet(&self, _ctx: &BlockContext) -> bool {
        true
    }

    fn is_groupable(&self) -> bool {
        true
    }

    fn preamble(&self, _ctx: &BlockContext) -> Option<Text<'static>> {
        Some(Text::from(self.lines(false, false)))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::acpx::bundle::{ActionReceipt, FlowDef, FlowNode, RunSummary};

    fn test_ctx(mode: DisplayMode) -> BlockContext {
        BlockContext {
            mode,
            is_running: false,
            width: 120,
            raw: false,
            max_lines: None,
            appearance: AppearanceConfig::default(),
            is_selected: false,
            cwd: None,
        }
    }

    fn output_lines(block: &AcpxFlowBlock, mode: DisplayMode) -> Vec<String> {
        block
            .output(&test_ctx(mode))
            .lines
            .iter()
            .map(|l| {
                l.content
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn sample_run() -> Run {
        Run {
            summary: RunSummary {
                run_id: "run_42".into(),
                dir: PathBuf::from("/tmp/run_42"),
                flow_name: "deploy-flow".into(),
                title: "Deploy Service".into(),
                status: RunStatus::Running,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: Some("build".into()),
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "deploy-flow".into(),
                start_at: "plan".into(),
                nodes: vec![
                    FlowNode {
                        id: "plan".into(),
                        kind: NodeKind::Acp,
                        summary: None,
                        is_shell: false,
                        ..Default::default()
                    },
                    FlowNode {
                        id: "route".into(),
                        kind: NodeKind::Compute,
                        summary: None,
                        is_shell: false,
                        ..Default::default()
                    },
                    FlowNode {
                        id: "build".into(),
                        kind: NodeKind::Action,
                        summary: None,
                        is_shell: true,
                        ..Default::default()
                    },
                    FlowNode {
                        id: "rollback".into(),
                        kind: NodeKind::Action,
                        summary: None,
                        is_shell: true,
                        ..Default::default()
                    },
                ],
                edges: Vec::new(),
                ..Default::default()
            },
            steps: vec![
                Step {
                    attempt_id: "plan#1".into(),
                    node_id: "plan".into(),
                    kind: NodeKind::Acp,
                    outcome: Outcome::Ok,
                    started_ms: 1_000,
                    finished_ms: 2_500,
                    prompt_text: None,
                    raw_text: None,
                    output: json!("Ready to deploy\nsecond line"),
                    error: None,
                    agent_name: Some("claude".into()),
                    action: None,
                    conversation: None,
                    stdout: None,
                    stderr: None,
                    output_artifact: None,
                },
                Step {
                    attempt_id: "route#1".into(),
                    node_id: "route".into(),
                    kind: NodeKind::Compute,
                    outcome: Outcome::Ok,
                    started_ms: 2_500,
                    finished_ms: 2_600,
                    prompt_text: None,
                    raw_text: None,
                    output: json!({ "route": "fast_track", "extra": 1 }),
                    error: None,
                    agent_name: None,
                    action: None,
                    conversation: None,
                    stdout: None,
                    stderr: None,
                    output_artifact: None,
                },
            ],
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn started_state_extracts_filename_and_shows_starting() {
        let block = AcpxFlowBlock::started("flows/ci/release.flow.json");
        assert_eq!(block.title, "release.flow.json");
        assert!(block.is_running());
        assert!(block.run_id.is_none());

        let lines = output_lines(&block, DisplayMode::Expanded);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("acpx "), "got: {}", lines[0]);
        assert!(
            lines[0].contains("release.flow.json starting…"),
            "got: {}",
            lines[0]
        );
    }

    #[test]
    fn rows_in_flow_order_with_glyphs_and_agent_labels() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let run = sample_run();
        block.update_from_run(&run, 6_000);

        let lines = output_lines(&block, DisplayMode::Expanded);
        assert!(lines[0].contains("Deploy Service running · 2/4"), "got: {}", lines[0]);
        assert!(lines[1].contains("✓ plan") && lines[1].contains("claude"), "got: {}", lines[1]);
        assert!(lines[2].contains("✓ route") && lines[2].contains("compute"), "got: {}", lines[2]);
        assert!(lines[3].contains("● build") && lines[3].contains("action"), "got: {}", lines[3]);
        assert!(lines[4].contains("○ rollback") && lines[4].contains("action"), "got: {}", lines[4]);
    }

    #[test]
    fn waiting_status_renders_detail_and_no_resume_hint() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let mut run = sample_run();
        run.summary.status = RunStatus::Waiting;
        run.summary.waiting_on = Some("gate".into());
        run.summary.status_detail = Some("Needs human approval".into());
        block.update_from_run(&run, 10_000);

        assert!(!block.is_running());
        let lines = output_lines(&block, DisplayMode::Expanded);
        assert!(lines[0].contains("waiting at gate"), "got: {}", lines[0]);
        assert!(
            lines.iter().any(|l| l.contains("◐ Needs human approval")),
            "lines: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("acpx cannot resume a run; re-run the flow to continue")),
            "lines: {lines:?}"
        );
    }

    #[test]
    fn failed_status_renders_error_line_and_note() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let mut run = sample_run();
        run.summary.status = RunStatus::Failed;
        run.summary.error = Some("build container crashed".into());
        run.summary.finished_ms = Some(15_000);
        block.note = Some("stderr: exit status 137".into());
        block.update_from_run(&run, 15_000);

        assert!(!block.is_running());
        let lines = output_lines(&block, DisplayMode::Expanded);
        assert!(lines[0].contains("failed after"), "got: {}", lines[0]);
        assert!(
            lines.iter().any(|l| l.contains("✗ build container crashed")),
            "lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("stderr: exit status 137")),
            "lines: {lines:?}"
        );
    }

    #[test]
    fn completed_header_shows_done_in_elapsed() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let mut run = sample_run();
        run.summary.status = RunStatus::Completed;
        run.summary.finished_ms = Some(61_000);
        block.update_from_run(&run, 61_000);

        let lines = output_lines(&block, DisplayMode::Expanded);
        assert!(lines[0].contains("Deploy Service done in"), "got: {}", lines[0]);
    }

    #[test]
    fn cancelled_survives_update_from_run() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        block.set_status(AcpxFlowStatus::Cancelled);
        assert!(!block.is_running());

        let run = sample_run();
        block.update_from_run(&run, 8_000);
        assert_eq!(block.status, AcpxFlowStatus::Cancelled);

        let lines = output_lines(&block, DisplayMode::Expanded);
        assert!(lines[0].contains("◌ cancelled after"), "got: {}", lines[0]);
    }

    #[test]
    fn route_result_renders_arrow() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let run = sample_run();
        block.update_from_run(&run, 5_000);

        let route_row = block
            .rows
            .iter()
            .find(|r| r.node_id == "route")
            .expect("route row");
        assert_eq!(route_row.result, "→ fast_track");

        let lines = output_lines(&block, DisplayMode::Expanded);
        assert!(
            lines.iter().any(|l| l.contains("→ fast_track")),
            "lines: {lines:?}"
        );
    }

    #[test]
    fn exit_code_result_formats_exit_code() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let mut run = sample_run();
        run.steps.push(Step {
            attempt_id: "build#1".into(),
            node_id: "build".into(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 2_600,
            finished_ms: 4_600,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: Some(ActionReceipt {
                action_type: "shell".into(),
                command: Some("cargo".into()),
                args: vec!["build".into()],
                cwd: None,
                exit_code: Some(0),
                duration_ms: Some(2_000),
            }),
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        block.update_from_run(&run, 5_000);

        let build_row = block
            .rows
            .iter()
            .find(|r| r.node_id == "build")
            .expect("build row");
        assert_eq!(build_row.result, "exit 0");
    }

    #[test]
    fn untaken_branch_row_is_not_run_with_no_duration() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let run = sample_run();
        block.update_from_run(&run, 5_000);

        let rollback_row = block
            .rows
            .iter()
            .find(|r| r.node_id == "rollback")
            .expect("rollback row");
        assert_eq!(rollback_row.state, NodeState::NotRun);
        assert!(rollback_row.duration_ms.is_none());
        assert!(rollback_row.result.is_empty());

        let lines = output_lines(&block, DisplayMode::Expanded);
        let rollback_line = lines
            .iter()
            .find(|l| l.contains("rollback"))
            .expect("rollback line");
        assert!(rollback_line.contains("○ rollback"));
    }

    #[test]
    fn run_id_hint_line_present_once_known() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let before = output_lines(&block, DisplayMode::Expanded);
        assert!(!before.iter().any(|l| l.contains("opens this run")));

        let run = sample_run();
        block.update_from_run(&run, 5_000);
        let after = output_lines(&block, DisplayMode::Expanded);
        assert!(
            after.iter().any(|l| l.contains("/acpx run_42 opens this run")),
            "after: {after:?}"
        );
    }

    #[test]
    fn collapsed_mode_shows_single_header_line() {
        let mut block = AcpxFlowBlock::started("deploy.json");
        let run = sample_run();
        block.update_from_run(&run, 5_000);

        let collapsed = output_lines(&block, DisplayMode::Collapsed);
        assert_eq!(collapsed.len(), 1);
        assert!(collapsed[0].contains("acpx Deploy Service running"));
    }
}
