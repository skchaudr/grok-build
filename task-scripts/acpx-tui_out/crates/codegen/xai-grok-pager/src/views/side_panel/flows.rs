//! Flows page: the step rail of the featured acpx flow run, then recent runs. Enter opens one in the `/acpx` modal.

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{Ctx, Row, note_rows};
use crate::acpx::bundle::{self, NodeState, Outcome, Run, RunStatus, RunSummary, Step, fmt_duration, value_text};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

/// Prefix of `Row::open` for a row that opens a run in the /acpx modal: "acpx-run:<absolute run dir>".
pub(crate) const OPEN_PREFIX: &str = "acpx-run:";

const LIMIT: usize = 30;

#[derive(Default)]
pub(super) struct Flows {
    runs: Vec<RunSummary>,
    featured: Option<Run>,
}

impl Flows {
    /// The 1 s tick while this page is showing. True when the rows changed.
    pub(super) fn poll(&mut self, now_ms: i64) -> bool {
        match bundle::runs_base() {
            Some(base) => self.poll_at(&base, now_ms),
            None => {
                let changed = !self.runs.is_empty() || self.featured.is_some();
                self.runs.clear();
                self.featured = None;
                changed
            }
        }
    }

    fn poll_at(&mut self, base: &Path, _now_ms: i64) -> bool {
        let mut runs = bundle::list_runs(base);
        runs.truncate(LIMIT);

        let mut changed = self.runs != runs;
        self.runs = runs;

        let target = pick_featured(&self.runs);
        match target {
            None => {
                if self.featured.is_some() {
                    self.featured = None;
                    changed = true;
                }
            }
            Some(summary) => {
                let same_run = self.featured.as_ref().is_some_and(|r| r.summary.dir == summary.dir);
                if same_run {
                    if let Some(run) = &mut self.featured {
                        let was_running = run.summary.status == RunStatus::Running;
                        let summary_changed = run.summary != *summary;
                        if was_running || summary_changed {
                            if bundle::refresh(run) {
                                changed = true;
                            } else if summary_changed {
                                run.summary = summary.clone();
                                changed = true;
                            }
                        }
                    }
                } else {
                    match bundle::load_run(&summary.dir) {
                        Ok(run) => {
                            self.featured = Some(run);
                            changed = true;
                        }
                        Err(_) => {
                            if self.featured.is_some() {
                                self.featured = None;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        changed
    }

    pub(super) fn rows(&self, c: &Ctx) -> Vec<Row> {
        if self.runs.is_empty() {
            return note_rows("no acpx flow runs yet", c);
        }
        let mut out = Vec::new();
        let featured_dir = match (&self.featured, pick_featured(&self.runs)) {
            (Some(run), _) => {
                push_featured_run(&mut out, run, c);
                Some(run.summary.dir.as_path())
            }
            (None, Some(summary)) => {
                push_featured_summary(&mut out, summary, c);
                Some(summary.dir.as_path())
            }
            (None, None) => None,
        };

        let others: Vec<&RunSummary> =
            self.runs.iter().filter(|r| Some(r.dir.as_path()) != featured_dir).collect();

        if !others.is_empty() {
            out.push(Row::plain(Line::raw("")));
            let muted = Style::default().fg(c.theme.text_secondary);
            out.push(Row::plain(Line::from(Span::styled(truncate_str("recent", c.width), muted))));
            for r in others {
                out.push(recent_row(r, c));
            }
        }

        out
    }
}

fn pick_featured(runs: &[RunSummary]) -> Option<&RunSummary> {
    runs.iter()
        .find(|r| r.status == RunStatus::Running)
        .or_else(|| runs.iter().find(|r| r.status == RunStatus::Waiting))
        .or_else(|| runs.first())
}

fn open_target(dir: &Path) -> String {
    format!("{OPEN_PREFIX}{}", dir.display())
}

fn status_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.accent_success,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray_dim,
    }
}

fn node_state_color(state: NodeState, theme: &Theme) -> Color {
    match state {
        NodeState::Running => theme.accent_running,
        NodeState::Waiting => theme.warning,
        NodeState::Done(Outcome::Ok) => theme.accent_success,
        NodeState::Done(_) => theme.accent_error,
        NodeState::NotRun => theme.gray_dim,
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_featured_run(out: &mut Vec<Row>, run: &Run, c: &Ctx) {
    let s = &run.summary;
    let target = open_target(&s.dir);
    let muted = Style::default().fg(c.theme.text_secondary);

    out.push(title_row(s, c, &target));

    let (done, total) = run.progress();
    let dur = fmt_duration(run.duration_ms(c.now_ms));
    let age = fmt_age(c.now_ms.saturating_sub(s.updated_ms));
    let meta = format!("{} · {done}/{total} · {dur} · {age}", s.flow_name);
    out.push(
        Row::new(Line::from(Span::styled(truncate_str(&meta, c.width), muted)), meta).opens(target.clone()),
    );

    push_status_rows(out, s, c, &target);

    for node in &run.flow.nodes {
        let state = run.node_state(&node.id);
        let last_step = run.steps.iter().rev().find(|step| step.node_id == node.id);
        out.push(step_rail_row(&node.id, state, last_step, c, &target));
    }
}

fn push_featured_summary(out: &mut Vec<Row>, s: &RunSummary, c: &Ctx) {
    let target = open_target(&s.dir);
    let muted = Style::default().fg(c.theme.text_secondary);

    out.push(title_row(s, c, &target));

    let end = s.finished_ms.unwrap_or(if s.status.is_live() { c.now_ms } else { s.updated_ms });
    let dur_ms = u64::try_from(end.saturating_sub(s.started_ms).max(0)).unwrap_or(0);
    let dur = fmt_duration(dur_ms);
    let age = fmt_age(c.now_ms.saturating_sub(s.updated_ms));
    let meta = format!("{} · 0/0 · {dur} · {age}", s.flow_name);
    out.push(
        Row::new(Line::from(Span::styled(truncate_str(&meta, c.width), muted)), meta).opens(target.clone()),
    );

    push_status_rows(out, s, c, &target);
}

fn title_row(s: &RunSummary, c: &Ctx, target: &str) -> Row {
    let glyph = s.status.glyph();
    let title = one_line(&s.title);
    let full = format!("{glyph} {title}");
    let glyph_style = Style::default().fg(status_color(s.status, c.theme)).add_modifier(Modifier::BOLD);
    let title_style = Style::default().fg(c.theme.text_primary).add_modifier(Modifier::BOLD);

    let prefix = format!("{glyph} ");
    let prefix_w = prefix.width();
    let line = if c.width <= prefix_w {
        Line::from(Span::styled(truncate_str(&full, c.width), glyph_style))
    } else {
        let cut_title = truncate_str(&title, c.width.saturating_sub(prefix_w));
        Line::from(vec![Span::styled(prefix, glyph_style), Span::styled(cut_title, title_style)])
    };
    Row::new(line, full).opens(target.to_string())
}

fn push_status_rows(out: &mut Vec<Row>, s: &RunSummary, c: &Ctx, target: &str) {
    let muted = Style::default().fg(c.theme.text_secondary);
    match s.status {
        RunStatus::Waiting => {
            let node = s
                .waiting_on
                .as_deref()
                .or(s.current_node.as_deref())
                .filter(|n| !n.is_empty())
                .unwrap_or("checkpoint");
            let wait_text = format!("◐ waiting at {node}");
            let wait_style = Style::default().fg(c.theme.warning);
            out.push(
                Row::new(
                    Line::from(Span::styled(truncate_str(&wait_text, c.width), wait_style)),
                    wait_text,
                )
                .opens(target.to_string()),
            );
            if let Some(detail) = s.status_detail.as_deref().map(one_line).filter(|d| !d.is_empty()) {
                out.push(
                    Row::new(Line::from(Span::styled(truncate_str(&detail, c.width), muted)), detail)
                        .opens(target.to_string()),
                );
            }
        }
        RunStatus::Failed | RunStatus::TimedOut => {
            let err = s
                .error
                .as_deref()
                .or(s.status_detail.as_deref())
                .map(one_line)
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| s.status.label().to_string());
            let err_text = format!("✗ {err}");
            let err_style = Style::default().fg(c.theme.accent_error);
            out.push(
                Row::new(Line::from(Span::styled(truncate_str(&err_text, c.width), err_style)), err_text)
                    .opens(target.to_string()),
            );
        }
        _ => {}
    }
}

fn step_rail_row(
    node_id: &str,
    state: NodeState,
    last_step: Option<&Step>,
    c: &Ctx,
    target: &str,
) -> Row {
    let glyph = state.glyph();
    let glyph_style = Style::default().fg(node_state_color(state, c.theme));
    let name_style = Style::default().fg(c.theme.text_primary);
    let muted = Style::default().fg(c.theme.text_secondary);

    let dur = last_step.map(|st| fmt_duration(st.duration_ms())).unwrap_or_default();
    let prefix = format!("  {glyph} ");
    let prefix_w = prefix.width();
    let dur_w = dur.width();

    let line = if dur.is_empty() {
        if c.width <= prefix_w {
            Line::from(Span::styled(truncate_str(&format!("{prefix}{node_id}"), c.width), glyph_style))
        } else {
            let name = truncate_str(node_id, c.width.saturating_sub(prefix_w));
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{glyph} "), glyph_style),
                Span::styled(name, name_style),
            ])
        }
    } else if c.width <= prefix_w + dur_w + 1 {
        let combined = format!("{prefix}{node_id} {dur}");
        Line::from(Span::styled(truncate_str(&combined, c.width), name_style))
    } else {
        let avail_name = c.width.saturating_sub(prefix_w + dur_w + 1);
        let name = truncate_str(node_id, avail_name);
        let pad = c.width.saturating_sub(prefix_w + name.width() + dur_w);
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(name, name_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(dur, muted),
        ])
    };

    let result = node_result_line(state, last_step);
    let copy = format!("{node_id}: {result}");
    Row::new(line, copy).opens(target.to_string())
}

fn node_result_line(state: NodeState, last_step: Option<&Step>) -> String {
    if let Some(step) = last_step {
        if let Some(err) = step.error.as_deref().map(one_line).filter(|s| !s.is_empty()) {
            return err;
        }
        let out = one_line(&value_text(&step.output));
        if !out.is_empty() {
            return out;
        }
        if let Some(raw) = step.raw_text.as_deref().map(one_line).filter(|s| !s.is_empty()) {
            return raw;
        }
    }
    match state {
        NodeState::NotRun => "not run".into(),
        NodeState::Running => "running".into(),
        NodeState::Waiting => "waiting".into(),
        NodeState::Done(Outcome::Ok) => "ok".into(),
        NodeState::Done(Outcome::Failed) => "failed".into(),
        NodeState::Done(Outcome::TimedOut) => "timed out".into(),
        NodeState::Done(Outcome::Cancelled) => "cancelled".into(),
        NodeState::Done(Outcome::Unknown) => "done".into(),
    }
}

fn recent_row(s: &RunSummary, c: &Ctx) -> Row {
    let glyph = s.status.glyph();
    let glyph_style = Style::default().fg(status_color(s.status, c.theme));
    let title_style = Style::default().fg(c.theme.text_primary);
    let muted = Style::default().fg(c.theme.text_secondary);

    let title = one_line(&s.title);
    let age = fmt_age(c.now_ms.saturating_sub(s.updated_ms));
    let prefix = format!("  {glyph} ");
    let prefix_w = prefix.width();
    let age_w = age.width();

    let line = if c.width <= prefix_w + age_w + 1 {
        let combined = format!("{prefix}{title} {age}");
        Line::from(Span::styled(truncate_str(&combined, c.width), title_style))
    } else {
        let avail_title = c.width.saturating_sub(prefix_w + age_w + 1);
        let cut_title = truncate_str(&title, avail_title);
        let pad = c.width.saturating_sub(prefix_w + cut_title.width() + age_w);
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(cut_title, title_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(age, muted),
        ])
    };

    let target = open_target(&s.dir);
    Row::new(line, s.dir.display().to_string()).opens(target)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::super::{Info, paint};
    use super::*;

    const NOW: i64 = 1_775_655_600_000; // 2026-04-08T13:40:00Z

    struct BundleSpec<'a> {
        id: &'a str,
        flow_name: &'a str,
        title: &'a str,
        status: &'a str,
        started_iso: &'a str,
        updated_iso: &'a str,
        finished_iso: Option<&'a str>,
        current_node: Option<&'a str>,
        waiting_on: Option<&'a str>,
        status_detail: Option<&'a str>,
        error: Option<&'a str>,
        nodes: Vec<(&'a str, &'a str)>,
        edges: Vec<(&'a str, &'a str)>,
        steps: Vec<Value>,
    }

    impl<'a> BundleSpec<'a> {
        fn new(id: &'a str, status: &'a str, started_iso: &'a str) -> Self {
            Self {
                id,
                flow_name: "pr-triage",
                title: id,
                status,
                started_iso,
                updated_iso: started_iso,
                finished_iso: None,
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
                nodes: vec![("extract", "action"), ("triage", "acp"), ("apply", "action")],
                edges: vec![("extract", "triage"), ("triage", "apply")],
                steps: Vec::new(),
            }
        }
    }

    fn write_bundle(base: &Path, spec: &BundleSpec<'_>) -> PathBuf {
        let dir = base.join(spec.id);
        let proj = dir.join("projections");
        fs::create_dir_all(&proj).expect("create projections dir");

        let mut nodes_map = serde_json::Map::new();
        for (nid, nkind) in &spec.nodes {
            nodes_map.insert((*nid).to_string(), json!({ "nodeType": nkind }));
        }
        let edges_vec: Vec<Value> =
            spec.edges.iter().map(|(from, to)| json!({ "from": from, "to": to })).collect();
        let start_at = spec.nodes.first().map(|(n, _)| *n).unwrap_or("start");
        let flow_json = json!({
            "name": spec.flow_name,
            "startAt": start_at,
            "nodes": nodes_map,
            "edges": edges_vec,
        });
        fs::write(dir.join("flow.json"), flow_json.to_string()).expect("write flow.json");

        let live_json = json!({
            "runId": spec.id,
            "flowName": spec.flow_name,
            "runTitle": spec.title,
            "status": spec.status,
            "startedAt": spec.started_iso,
            "updatedAt": spec.updated_iso,
            "finishedAt": spec.finished_iso,
            "currentNode": spec.current_node,
            "waitingOn": spec.waiting_on,
            "statusDetail": spec.status_detail,
            "error": spec.error,
        });
        fs::write(proj.join("live.json"), live_json.to_string()).expect("write live.json");
        fs::write(proj.join("steps.json"), Value::Array(spec.steps.clone()).to_string())
            .expect("write steps.json");

        dir
    }

    fn render_rows(flows: &Flows, width: usize) -> Vec<Row> {
        let info = Info::default();
        let theme = Theme::default();
        let ctx = Ctx { info: &info, theme: &theme, width, now_ms: NOW };
        flows.rows(&ctx)
    }

    #[test]
    fn empty_base_renders_note_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut flows = Flows::default();
        assert!(!flows.poll_at(tmp.path(), NOW));
        let rows = render_rows(&flows, 36);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].copy, "no acpx flow runs yet");
        assert!(rows[0].open.is_none());
    }

    #[test]
    fn featured_prefers_running_over_waiting_over_newest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut completed = BundleSpec::new("run-newest", "completed", "2026-04-08T13:39:00Z");
        completed.title = "Newest Completed";
        write_bundle(tmp.path(), &completed);

        let mut waiting = BundleSpec::new("run-waiting", "waiting", "2026-04-08T13:35:00Z");
        waiting.title = "Older Waiting";
        waiting.waiting_on = Some("triage");
        let waiting_dir = write_bundle(tmp.path(), &waiting);

        let mut flows = Flows::default();
        assert!(flows.poll_at(tmp.path(), NOW));
        let rows = render_rows(&flows, 40);
        assert!(rows[0].copy.contains("Older Waiting"), "waiting beats newer completed");
        assert_eq!(rows[0].open.as_deref(), Some(open_target(&waiting_dir).as_str()));

        let mut running = BundleSpec::new("run-running", "running", "2026-04-08T13:30:00Z");
        running.title = "Oldest Running";
        running.current_node = Some("extract");
        let running_dir = write_bundle(tmp.path(), &running);

        assert!(flows.poll_at(tmp.path(), NOW));
        let rows = render_rows(&flows, 40);
        assert!(rows[0].copy.contains("Oldest Running"), "running beats waiting and completed");
        assert_eq!(rows[0].open.as_deref(), Some(open_target(&running_dir).as_str()));
    }

    #[test]
    fn step_rail_orders_nodes_and_renders_state_glyphs_and_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-1", "running", "2026-04-08T13:38:00Z");
        spec.title = "Triage PR #42";
        spec.current_node = Some("triage");
        spec.steps = vec![json!({
            "attemptId": "extract#1",
            "nodeId": "extract",
            "nodeType": "action",
            "outcome": "ok",
            "startedAt": "2026-04-08T13:38:00.000Z",
            "finishedAt": "2026-04-08T13:38:03.900Z",
            "output": "extracted\n  diff   summary",
        })];
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 42);
        let drawn = paint(&rows, 42);
        let lines: Vec<&str> = drawn.lines().collect();

        assert!(lines[0].contains("● Triage PR #42"));
        assert!(lines[1].contains("pr-triage · 1/3 ·"));
        assert!(lines[2].contains("✓ extract") && lines[2].ends_with("3.9s"));
        assert!(lines[3].contains("● triage"));
        assert!(lines[4].contains("○ apply"));

        assert_eq!(rows[2].copy, "extract: extracted diff summary");
        assert_eq!(rows[3].copy, "triage: running");
        assert_eq!(rows[4].copy, "apply: not run");
        for row in &rows[..5] {
            assert_eq!(row.open.as_deref(), Some(open_target(&dir).as_str()));
        }
    }

    #[test]
    fn waiting_run_includes_waiting_node_and_status_detail_rows() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-wait", "waiting", "2026-04-08T13:30:00Z");
        spec.title = "Deploy Staging";
        spec.waiting_on = Some("approve");
        spec.status_detail = Some("awaiting human approval on slack");
        spec.nodes = vec![("build", "action"), ("approve", "checkpoint")];
        spec.edges = vec![("build", "approve")];
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 44);
        assert_eq!(rows[2].copy, "◐ waiting at approve");
        assert_eq!(rows[3].copy, "awaiting human approval on slack");
        assert_eq!(rows[2].open.as_deref(), Some(open_target(&dir).as_str()));
        assert_eq!(rows[3].open.as_deref(), Some(open_target(&dir).as_str()));
    }

    #[test]
    fn failed_run_includes_error_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-fail", "failed", "2026-04-08T13:30:00Z");
        spec.title = "Nightly Bench";
        spec.error = Some("exit code 101:\ncompilation failed");
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 44);
        assert_eq!(rows[2].copy, "✗ exit code 101: compilation failed");
        assert_eq!(rows[2].open.as_deref(), Some(open_target(&dir).as_str()));
    }

    #[test]
    fn recent_list_excludes_featured_and_opens_each_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut r1 = BundleSpec::new("run-1", "running", "2026-04-08T13:35:00Z");
        r1.title = "Featured Run";
        let d1 = write_bundle(tmp.path(), &r1);

        let mut r2 = BundleSpec::new("run-2", "completed", "2026-04-08T13:38:00Z");
        r2.title = "Second Run";
        let d2 = write_bundle(tmp.path(), &r2);

        let mut r3 = BundleSpec::new("run-3", "failed", "2026-04-08T12:00:00Z");
        r3.title = "Third Run";
        let d3 = write_bundle(tmp.path(), &r3);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 40);

        let recent_idx = rows.iter().position(|r| r.copy == "recent").expect("recent header");
        assert_eq!(rows[recent_idx - 1].copy, "");
        let recent_rows = &rows[recent_idx + 1..];
        assert_eq!(recent_rows.len(), 2);

        assert_eq!(recent_rows[0].copy, d2.display().to_string());
        assert_eq!(recent_rows[0].open.as_deref(), Some(open_target(&d2).as_str()));
        assert_eq!(recent_rows[1].copy, d3.display().to_string());
        assert_eq!(recent_rows[1].open.as_deref(), Some(open_target(&d3).as_str()));
        assert!(recent_rows.iter().all(|r| r.open.as_deref() != Some(open_target(&d1).as_str())));
    }

    #[test]
    fn every_row_fits_within_28_columns() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut r1 = BundleSpec::new("run-long-featured", "waiting", "2026-04-08T13:35:00Z");
        r1.title = "A very long featured run title that exceeds twenty-eight columns easily";
        r1.waiting_on = Some("extremely_long_checkpoint_node_identifier");
        r1.status_detail = Some("waiting for a very verbose status detail explanation string");
        r1.nodes = vec![
            ("very_long_first_step_node_name", "action"),
            ("extremely_long_checkpoint_node_identifier", "checkpoint"),
        ];
        r1.edges = vec![("very_long_first_step_node_name", "extremely_long_checkpoint_node_identifier")];
        r1.steps = vec![json!({
            "attemptId": "very_long_first_step_node_name#1",
            "nodeId": "very_long_first_step_node_name",
            "nodeType": "action",
            "outcome": "ok",
            "startedAt": "2026-04-08T13:35:00.000Z",
            "finishedAt": "2026-04-08T13:37:05.000Z",
        })];
        write_bundle(tmp.path(), &r1);

        let mut r2 = BundleSpec::new("run-long-other", "completed", "2026-04-08T11:00:00Z");
        r2.title = "Another super long run title in the recent list section";
        write_bundle(tmp.path(), &r2);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 28);
        for row in &rows {
            let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                text.width() <= 28,
                "row width {} exceeds 28: {:?}",
                text.width(),
                text
            );
        }
        let painted = paint(&rows, 28);
        for line in painted.lines() {
            assert!(line.width() <= 28);
        }
    }

    #[test]
    fn poll_at_tracks_changes_and_refreshes_running_bundle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut spec = BundleSpec::new("run-live", "running", "2026-04-08T13:35:00Z");
        spec.current_node = Some("extract");
        let dir = write_bundle(tmp.path(), &spec);

        let mut flows = Flows::default();
        assert!(flows.poll_at(tmp.path(), NOW), "first load is a change");
        assert!(!flows.poll_at(tmp.path(), NOW), "unchanged files report false");

        // Update live.json to complete the run.
        spec.status = "completed";
        spec.updated_iso = "2026-04-08T13:36:00Z";
        spec.finished_iso = Some("2026-04-08T13:36:00Z");
        spec.current_node = None;
        let _ = dir;
        write_bundle(tmp.path(), &spec);

        assert!(flows.poll_at(tmp.path(), NOW), "updated live.json reports true");
        assert!(!flows.poll_at(tmp.path(), NOW), "subsequent poll on completed run is false");
    }

    #[test]
    fn open_targets_always_start_with_open_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r1 = BundleSpec::new("run-a", "running", "2026-04-08T13:35:00Z");
        write_bundle(tmp.path(), &r1);
        let r2 = BundleSpec::new("run-b", "completed", "2026-04-08T13:30:00Z");
        write_bundle(tmp.path(), &r2);

        let mut flows = Flows::default();
        flows.poll_at(tmp.path(), NOW);
        let rows = render_rows(&flows, 36);
        let openables: Vec<&str> = rows.iter().filter_map(|r| r.open.as_deref()).collect();
        assert!(!openables.is_empty());
        for target in openables {
            assert!(
                target.starts_with(OPEN_PREFIX),
                "expected {target:?} to start with {OPEN_PREFIX:?}"
            );
        }
    }
}
