use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::acpx::bundle::{
    self, AgentSettings, FlowEdge, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
};
use crate::acpx::step_digest::{
    self, StepVerdict, digest_step_files, effective_outcome, extract_step_validations,
    extract_step_verdict, non_zero_exit_code, step_attempt_budget, step_failure_reason,
    step_time_budget,
};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

/// What the right-hand panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum DetailTarget {
    Run,          // overview of the whole run
    Step(usize),  // index into run.steps: one finished attempt
    Node(String), // a node id with no step record: running, or not run (rarely: waiting without a step)
}

#[derive(Debug, Clone, PartialEq)]
struct CacheKey {
    run_dir: PathBuf,
    target: DetailTarget,
    width: u16,
    updated_ms: i64,
    steps_len: usize,
    events_len: usize,
    events_file_len: Option<u64>,
    answered: Option<String>,
}

#[derive(Debug, Clone)]
struct CopyUnit {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Debug, Default)]
struct Rows {
    lines: Vec<Line<'static>>,
    units: Vec<CopyUnit>,
}

impl Rows {
    fn new() -> Self {
        Self::default()
    }

    fn push_unit(&mut self, rows: Vec<Line<'static>>, text: String) {
        if rows.is_empty() {
            return;
        }
        let start = self.lines.len();
        self.lines.extend(rows);
        let end = self.lines.len();
        self.units.push(CopyUnit { start, end, text });
    }

    fn append_rows(&mut self, other: Rows) {
        let offset = self.lines.len();
        self.lines.extend(other.lines);
        for mut unit in other.units {
            unit.start += offset;
            unit.end += offset;
            self.units.push(unit);
        }
    }

    fn unit_at(&self, row: usize) -> Option<&CopyUnit> {
        let idx = self.units.partition_point(|u| u.end <= row);
        self.units
            .get(idx)
            .filter(|u| u.start <= row && row < u.end)
    }

    fn copy_range(&self, start_row: usize, end_row: usize) -> String {
        if self.lines.is_empty() || start_row > end_row {
            return String::new();
        }
        let last = end_row.min(self.lines.len() - 1);
        let mut out = Vec::new();
        let mut row = start_row.min(last);
        while row <= last {
            if let Some(unit) = self.unit_at(row) {
                out.push(unit.text.clone());
                row = unit.end;
            } else {
                out.push(line_plain_text(&self.lines[row]));
                row += 1;
            }
        }
        out.join("\n")
    }

    fn copy_all(&self) -> String {
        if self.lines.is_empty() {
            String::new()
        } else {
            self.copy_range(0, self.lines.len() - 1)
        }
    }
}

impl std::ops::Deref for Rows {
    type Target = Vec<Line<'static>>;

    fn deref(&self) -> &Self::Target {
        &self.lines
    }
}

impl std::ops::DerefMut for Rows {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.lines
    }
}

impl IntoIterator for Rows {
    type Item = Line<'static>;
    type IntoIter = std::vec::IntoIter<Line<'static>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.into_iter()
    }
}

impl<'a> IntoIterator for &'a Rows {
    type Item = &'a Line<'static>;
    type IntoIter = std::slice::Iter<'a, Line<'static>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter()
    }
}

fn line_plain_text(line: &Line<'_>) -> String {
    let mut text = String::new();
    for span in &line.spans {
        text.push_str(span.content.as_ref());
    }
    text.trim_end().to_string()
}

fn source_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect()
    }
}

pub struct DetailPanel {
    cache_key: Option<CacheKey>,
    /// The picked choice of an answered gate; `Some("")` when it was answered
    /// but the choice is unknown. `None` while the gate is open.
    answered: Option<String>,
    last_target: Option<(PathBuf, DetailTarget)>,
    lines: Rows,
    scroll: usize,
    tail_follow: bool,
    last_height: usize,
    focused: bool,
    cursor: usize,
    visual_anchor: Option<usize>,
}

impl Default for DetailPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailPanel {
    pub fn new() -> Self {
        Self {
            cache_key: None,
            answered: None,
            last_target: None,
            lines: Rows::new(),
            scroll: 0,
            tail_follow: false,
            last_height: 0,
            focused: false,
            cursor: 0,
            visual_anchor: None,
        }
    }

    /// Tell the panel whether the run's gate has been answered, and with what.
    pub fn set_answered(&mut self, answered: Option<&str>) {
        if self.answered.as_deref() != answered {
            self.answered = answered.map(str::to_string);
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_visual(&self) -> bool {
        self.focused && self.visual_anchor.is_some()
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if !focused {
            self.visual_anchor = None;
            return;
        }
        self.tail_follow = false;
        if self.lines.is_empty() {
            self.cursor = 0;
            return;
        }
        let max_row = self.lines.len() - 1;
        self.cursor = self.cursor.min(max_row);
        if self.last_height > 0 {
            let top = self.scroll.min(max_row);
            let bottom = (top + self.last_height - 1).min(max_row);
            if self.cursor < top || self.cursor > bottom {
                self.cursor = top;
            }
        }
    }

    pub fn page_step(&self) -> isize {
        if self.last_height > 1 {
            (self.last_height - 1) as isize
        } else if self.last_height == 1 {
            1
        } else {
            15
        }
    }

    pub fn move_cursor(&mut self, delta: isize) -> bool {
        if self.lines.is_empty() || delta == 0 {
            return false;
        }
        let max_row = self.lines.len() - 1;
        let cur = self.cursor.min(max_row);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_row)
        };
        self.tail_follow = false;
        let moved = next != self.cursor;
        self.cursor = next;
        self.ensure_cursor_visible(self.last_height);
        moved
    }

    pub fn toggle_visual(&mut self) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.visual_anchor = Some(self.cursor.min(self.lines.len() - 1));
        }
        true
    }

    pub fn handle_esc(&mut self) -> bool {
        if !self.focused {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.focused = false;
        }
        true
    }

    pub fn copy_selection(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        Some(self.lines.copy_range(start, end))
    }

    pub fn copy_all(&self) -> Option<String> {
        if self.lines.is_empty() {
            None
        } else {
            Some(self.lines.copy_all())
        }
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        if !self.focused || self.lines.is_empty() {
            return None;
        }
        let max_row = self.lines.len() - 1;
        let cur = self.cursor.min(max_row);
        match self.visual_anchor {
            Some(anchor) => {
                let a = anchor.min(max_row);
                Some((a.min(cur), a.max(cur)))
            }
            None => Some((cur, cur)),
        }
    }

    fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 || self.lines.is_empty() {
            return;
        }
        let max_row = self.lines.len() - 1;
        self.cursor = self.cursor.min(max_row);
        let max_scroll = self.max_scroll(height);
        self.scroll = self.scroll.min(max_scroll);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        let max_scroll = self.max_scroll(self.last_height);
        let cur = self.scroll.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_scroll)
        };
        if delta < 0 && next < cur {
            self.tail_follow = false;
        }
        if next >= max_scroll && delta > 0 {
            self.tail_follow = true;
        }
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn max_scroll(&self, height: usize) -> usize {
        if height == 0 {
            self.lines.len().saturating_sub(1)
        } else {
            self.lines.len().saturating_sub(height)
        }
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        run: &Run,
        target: &DetailTarget,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width < 10 || area.height == 0 {
            return;
        }
        self.last_height = area.height as usize;

        let target_pair = (run.summary.dir.clone(), target.clone());
        let target_changed = self.last_target.as_ref() != Some(&target_pair);
        if target_changed {
            self.last_target = Some(target_pair);
            let is_running_node = matches!(
                target,
                DetailTarget::Node(id) if run.node_state(id) == NodeState::Running
            );
            self.tail_follow = is_running_node;
            self.scroll = 0;
            self.cursor = 0;
            self.visual_anchor = None;
        }

        let events_file_len = conversation_file_len(run, target);
        let key = CacheKey {
            run_dir: run.summary.dir.clone(),
            target: target.clone(),
            width: area.width,
            updated_ms: run.summary.updated_ms,
            steps_len: run.steps.len(),
            events_len: run.events.len(),
            events_file_len,
            answered: self.answered.clone(),
        };

        if self.cache_key.as_ref() != Some(&key) {
            self.lines = build_lines(
                run,
                target,
                self.answered.as_deref(),
                (area.width as usize).saturating_sub(2),
                theme,
                now_ms,
            );
            self.cache_key = Some(key);
        }

        let max_scroll = self.max_scroll(area.height as usize);
        if self.lines.is_empty() {
            self.cursor = 0;
            self.visual_anchor = None;
        } else {
            let last_row = self.lines.len() - 1;
            self.cursor = self.cursor.min(last_row);
            if let Some(anchor) = self.visual_anchor.as_mut() {
                *anchor = (*anchor).min(last_row);
            }
        }
        if self.focused {
            self.ensure_cursor_visible(area.height as usize);
        } else if self.tail_follow {
            self.scroll = max_scroll;
        } else if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let height = area.height as usize;
        let total = self.lines.len();
        let needs_scrollbar = total > height;
        let text_width = area.width;
        let highlight = self.selected_range();

        for row in 0..height {
            let line_idx = self.scroll + row;
            let y = area.y + row as u16;
            if let Some(line) = self.lines.get(line_idx) {
                buf.set_line(area.x, y, line, text_width);
            }
            if highlight.is_some_and(|(s, e)| line_idx >= s && line_idx <= e) {
                let hl_style = Style::default().bg(theme.bg_highlight);
                for x in area.x..area.x + area.width {
                    buf[(x, y)].set_style(hl_style);
                }
            }
        }

        if needs_scrollbar && area.width >= 1 {
            let sb_x = area.x + area.width - 1;
            let thumb_h = ((height * height) / total).max(1).min(height);
            let max_top = height.saturating_sub(thumb_h);
            let thumb_top = if max_scroll == 0 {
                0
            } else {
                (self.scroll * max_top) / max_scroll
            };
            let sb_style = Style::default().fg(theme.gray_dim);
            for r in 0..thumb_h {
                let y = area.y + (thumb_top + r) as u16;
                if y < area.y + area.height {
                    buf.set_stringn(sb_x, y, "█", 1, sb_style);
                }
            }
        }
    }
}

fn conversation_file_len(run: &Run, target: &DetailTarget) -> Option<u64> {
    match target {
        DetailTarget::Step(i) => {
            let step = run.steps.get(*i)?;
            let conv = step.conversation.as_ref()?;
            let path = run.session_events_path(&conv.session_id)?;
            std::fs::metadata(path).ok().map(|m| m.len())
        }
        DetailTarget::Node(id) => {
            if run.node_state(id) != NodeState::Running {
                return None;
            }
            let settings = bundle::node_settings(&run.summary.dir);
            let handle = settings
                .get(id)
                .and_then(|s| s.handle.as_deref())
                .unwrap_or("main");
            let session = run.sessions.iter().find(|s| s.handle == handle)?;
            std::fs::metadata(&session.events_path).ok().map(|m| m.len())
        }
        DetailTarget::Run => None,
    }
}

fn build_lines(
    run: &Run,
    target: &DetailTarget,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Rows {
    if width < 10 {
        return Rows::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines_at(run, *i, step, answered, width, theme, now_ms)
            } else {
                let mut rows = Rows::new();
                rows.push(dim_line("step not found", theme));
                rows
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}

fn build_run_lines(
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Rows {
    let mut out = Rows::new();
    let status = run.summary.status;
    // A waiting run whose gate is answered reads as done, like the run header.
    let shown = if status == RunStatus::Waiting && answered.is_some() {
        RunStatus::Completed
    } else {
        status
    };
    let status_color = run_status_color(shown, theme);
    let glyph = shown.glyph();
    let title = if run.summary.title.is_empty() {
        &run.summary.run_id
    } else {
        &run.summary.title
    };
    let head = truncate_str(&format!("{glyph} {title}"), width);
    out.push(Line::from( vec![
        Span::styled(
            format!("{glyph} "),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            head.strip_prefix(&format!("{glyph} ")).unwrap_or(&head).to_string(),
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Facts row
    let (done, total) = run.progress();
    let dur = fmt_duration(run.duration_ms(now_ms));
    let mut facts = Vec::new();
    if !run.summary.flow_name.is_empty() {
        facts.push(run.summary.flow_name.clone());
    }
    facts.push(format!("{done}/{total} nodes"));
    facts.push(dur);
    if !status.is_live() {
        let ref_ms = run.summary.finished_ms.unwrap_or(run.summary.updated_ms);
        let ago = now_ms.saturating_sub(ref_ms).max(0);
        facts.push(fmt_age(ago));
    }
    let stalled = bundle::is_stalled(&run.summary, now_ms);
    let facts_text = facts.join(" · ");
    let mut fact_spans = vec![Span::styled(
        truncate_str(&facts_text, width),
        Style::default().fg(theme.gray),
    )];
    if stalled {
        let used = UnicodeWidthStr::width(facts_text.as_str());
        if used + 10 <= width {
            fact_spans.push(Span::styled(" · ", Style::default().fg(theme.gray_dim)));
            fact_spans.push(Span::styled(
                "stalled",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    out.push(Line::from(fact_spans));

    if status == RunStatus::Waiting {
        if let Some(detail) = &run.summary.status_detail {
            if !detail.is_empty() {
                let color = if answered.is_some() { theme.gray } else { theme.warning };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    if matches!(status, RunStatus::Failed | RunStatus::TimedOut) {
        if let Some(err) = &run.summary.error {
            if !err.is_empty() {
                push_wrapped(
                    &mut out,
                    err,
                    width,
                    Style::default().fg(theme.accent_error),
                );
            }
        }
    }

    // Stages section
    if !run.flow.nodes.is_empty() {
        push_section_rule(&mut out, "Stages", width, theme);
        let node_settings = bundle::node_settings(&run.summary.dir);
        let step_settings = bundle::step_settings(&run.summary.dir);
        let max_id_w = run
            .flow
            .nodes
            .iter()
            .map(|n| UnicodeWidthStr::width(n.id.as_str()))
            .max()
            .unwrap_or(4);

        for node in &run.flow.nodes {
            let state = run.node_state_answered(&node.id, answered.is_some());
            let s_glyph = state.glyph();
            let s_color = node_state_color(state, theme);
            let last_step = run.steps.iter().rev().find(|s| s.node_id == node.id);

            let agent_or_kind = last_step
                .and_then(|s| step_settings.get(&s.attempt_id))
                .map(AgentSettings::label)
                .filter(|l| !l.is_empty())
                .or_else(|| {
                    node_settings
                        .get(&node.id)
                        .map(AgentSettings::label)
                        .filter(|l| !l.is_empty())
                })
                .unwrap_or_else(|| node.kind.label().to_string());

            let dur_str = last_step
                .map(|s| fmt_duration(s.duration_ms()))
                .unwrap_or_default();

            let one_line = last_step.map(step_one_line_summary).unwrap_or_default();

            let id_w = UnicodeWidthStr::width(node.id.as_str());
            let pad = " ".repeat(max_id_w.saturating_sub(id_w) + 2);

            let mut spans = Vec::new();
            spans.push(Span::styled(
                format!("{s_glyph} "),
                Style::default().fg(s_color),
            ));
            spans.push(Span::styled(
                format!("{}{pad}", node.id),
                Style::default().fg(theme.text_primary),
            ));
            spans.push(Span::styled(
                agent_or_kind.clone(),
                Style::default().fg(theme.gray),
            ));
            if !dur_str.is_empty() {
                spans.push(Span::styled(
                    format!("  {dur_str}"),
                    Style::default().fg(theme.gray_dim),
                ));
            }
            let prefix_w: usize = spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if !one_line.is_empty() && prefix_w + 3 < width {
                let rem = width - prefix_w - 2;
                let cut = truncate_str(&one_line, rem);
                let res_color = if last_step.and_then(|s| s.error.as_ref()).is_some() {
                    theme.accent_error
                } else {
                    theme.text_secondary
                };
                spans.push(Span::styled("  ", Style::default()));
                spans.push(Span::styled(cut, Style::default().fg(res_color)));
            }
            out.push(truncate_line(Line::from(spans), width));
        }
    }

    // Last result
    if let Some(last_step) = run.steps.last() {
        let lines = format_value_lines(&last_step.output, width, theme);
        if !lines.is_empty() {
            push_section_rule(&mut out, "Last result", width, theme);
            out.append_rows(lines);
        }
    }

    // Input
    let input_lines = format_value_lines(&run.input, width, theme);
    if !input_lines.is_empty() {
        push_section_rule(&mut out, "Input", width, theme);
        out.append_rows(input_lines);
    }

    out
}

fn step_one_line_summary(step: &Step) -> String {
    if let Some(err) = &step.error {
        if !err.is_empty() {
            return first_non_empty_line(err);
        }
    }
    if let Some(route) = step
        .output
        .as_object()
        .and_then(|o| o.get("route"))
        .and_then(Value::as_str)
    {
        return format!("→ {route}");
    }
    if let Some(act) = &step.action {
        if let Some(code) = act.exit_code {
            return format!("exit {code}");
        }
    }
    match &step.output {
        Value::Null => String::new(),
        Value::String(s) => first_non_empty_line(s),
        other => value_inline_summary(other),
    }
}

/// What an empty list or object shows as a field value (drawn in the same colour as the keys beside it).
const EMPTY: &str = "(empty)";

fn is_empty_container(val: &Value) -> bool {
    match val {
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

pub(super) fn value_inline_summary(val: &Value) -> String {
    match val {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => first_non_empty_line(s),
        Value::Array(arr) => arr
            .iter()
            .map(value_inline_summary)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let vs = if is_empty_container(v) {
                    EMPTY.to_string()
                } else {
                    value_inline_summary(v)
                };
                if vs.is_empty() {
                    k.clone()
                } else {
                    format!("{k} {vs}")
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The key/value rows as plain text, for views that hold text rather than styled lines (the step inspector).
/// Same layout as the rows: one `key  value` line per field, nested values indented, lists as `- `, `(empty)` for
/// empty lists and objects, no braces or quotes. Lines are never wrapped here; the viewer wraps.
pub(super) fn value_plain_text(val: &Value) -> String {
    if is_empty_container(val) {
        return EMPTY.to_string();
    }
    let theme = Theme::current();
    format_value_lines(val, usize::MAX / 4, &theme)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Rows {
    let mut out = Rows::new();
    if matches!(val, Value::Null) {
        return out;
    }
    if let Value::String(s) = val {
        if s.trim().is_empty() {
            return out;
        }
    }
    append_value_lines(&mut out, val, 0, width, theme);
    out
}

fn append_value_lines(
    out: &mut Rows,
    val: &Value,
    indent: usize,
    width: usize,
    theme: &Theme,
) {
    let val_style = Style::default().fg(theme.text_primary);
    let key_style = Style::default().fg(theme.gray);
    let pad_indent = " ".repeat(indent);
    let body_w = width.saturating_sub(indent).max(1);

    match val {
        Value::Null => {
            if indent > 0 {
                out.push(Line::from(vec![
                    Span::raw(pad_indent),
                    Span::styled("null", val_style),
                ]));
            }
        }
        Value::Bool(b) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(b.to_string(), val_style),
            ]));
        }
        Value::Number(n) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(n.to_string(), val_style),
            ]));
        }
        Value::String(s) => {
            for src_line in source_lines(s) {
                let rows: Vec<Line<'static>> = wrap(src_line, body_w)
                    .into_iter()
                    .map(|line| {
                        if indent == 0 {
                            Line::from(Span::styled(line, val_style))
                        } else {
                            Line::from(vec![
                                Span::raw(pad_indent.clone()),
                                Span::styled(line, val_style),
                            ])
                        }
                    })
                    .collect();
                out.push_unit(rows, src_line.to_string());
            }
        }
        Value::Array(arr) => {
            for item in arr {
                let bullet_prefix = format!("{pad_indent}- ");
                let item_w = width.saturating_sub(indent + 2).max(1);
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        let mut sub = Rows::new();
                        append_value_lines(&mut sub, item, 0, item_w, theme);
                        if sub.is_empty() {
                            sub.push(Line::from(Span::styled(
                                EMPTY,
                                Style::default().fg(theme.gray),
                            )));
                        }
                        let offset = out.len();
                        let Rows { lines, units } = sub;
                        for (idx, sub_line) in lines.into_iter().enumerate() {
                            let mut spans = vec![Span::styled(
                                if idx == 0 {
                                    bullet_prefix.clone()
                                } else {
                                    format!("{pad_indent}  ")
                                },
                                key_style,
                            )];
                            spans.extend(sub_line.spans);
                            out.push(Line::from(spans));
                        }
                        for mut unit in units {
                            unit.start += offset;
                            unit.end += offset;
                            out.units.push(unit);
                        }
                    }
                    _ => {
                        let s = match item {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let wrapped = wrap(&s, item_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            let pfx = if idx == 0 {
                                bullet_prefix.clone()
                            } else {
                                format!("{pad_indent}  ")
                            };
                            out.push(Line::from(vec![
                                Span::styled(pfx, key_style),
                                Span::styled(line, val_style),
                            ]));
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            let max_key_w = map
                .keys()
                .map(|k| UnicodeWidthStr::width(k.as_str()))
                .max()
                .unwrap_or(0);
            for (k, v) in map {
                let kw = UnicodeWidthStr::width(k.as_str());
                let key_pad = " ".repeat(max_key_w.saturating_sub(kw) + 2);
                match v {
                    _ if is_empty_container(v) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(format!("{k}{key_pad}"), key_style),
                            Span::styled(EMPTY, Style::default().fg(theme.gray)),
                        ]));
                    }
                    Value::Object(_) | Value::Array(_) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(k.clone(), key_style),
                        ]));
                        append_value_lines(out, v, indent + 2, width, theme);
                    }
                    _ => {
                        let s = match v {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let prefix_w = indent + max_key_w + 2;
                        let val_w = width.saturating_sub(prefix_w).max(1);
                        let wrapped = wrap(&s, val_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            if idx == 0 {
                                out.push(Line::from(vec![
                                    Span::raw(pad_indent.clone()),
                                    Span::styled(format!("{k}{key_pad}"), key_style),
                                    Span::styled(line, val_style),
                                ]));
                            } else {
                                out.push(Line::from(vec![
                                    Span::raw(" ".repeat(prefix_w)),
                                    Span::styled(line, val_style),
                                ]));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn first_non_empty_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn build_step_lines(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Rows {
    build_step_lines_at(
        run,
        step_idx,
        step,
        answered,
        width,
        theme,
        run.summary.updated_ms,
    )
}

fn build_step_lines_at(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Rows {
    let mut out = Rows::new();

    let is_last_of_node = run
        .steps
        .iter()
        .rposition(|s| s.node_id == step.node_id)
        == Some(step_idx);
    let is_gate = run.summary.status == RunStatus::Waiting
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && is_last_of_node;

    let outcome = effective_outcome(step);
    let (glyph, glyph_color) = if is_gate && answered.is_none() {
        (NodeState::Waiting.glyph(), theme.warning)
    } else if is_gate {
        // The gate has been answered: done, like the graph and the run header.
        let state = NodeState::Done(Outcome::Ok);
        (state.glyph(), node_state_color(state, theme))
    } else {
        let state = NodeState::Done(outcome);
        (state.glyph(), node_state_color(state, theme))
    };

    let time_budget = step_time_budget(run, step, now_ms);
    let timing = time_budget.badge_label();
    out.push(title_row_right(
        glyph,
        glyph_color,
        &step.node_id,
        &timing,
        width,
        theme,
    ));

    // Facts row
    let settings_map = bundle::step_settings(&run.summary.dir);
    let agent_lbl = settings_map
        .get(&step.attempt_id)
        .map(AgentSettings::label)
        .filter(|s| !s.is_empty())
        .or_else(|| step.agent_name.clone());

    let attempt_budget = step_attempt_budget(run, step);
    let recorded_exit_code = step
        .action
        .as_ref()
        .and_then(|act| act.exit_code)
        .or_else(|| step.output.get("exitCode").and_then(Value::as_i64));

    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        if !lbl.is_empty() {
            facts.push(lbl);
        }
    }
    facts.push(step.kind.label().to_string());
    if let Some(act) = &step.action {
        if !act.action_type.is_empty() && act.action_type != step.kind.label() {
            facts.push(act.action_type.clone());
        }
    }
    facts.push(format!("attempt {}", attempt_budget.attempt));
    facts.push(step.attempt_id.clone());
    if let Some(code) = recorded_exit_code {
        facts.push(format!("exit {code}"));
    }
    let facts_str = truncate_str(&facts.join(" · "), width);
    out.push(Line::from(Span::styled(
        facts_str,
        Style::default().fg(theme.gray),
    )));

    // Bounded failure tail comes before command/cwd for failed attempts.
    if let Some(failure_tail) = failure_tail_for_step(run, step) {
        push_section_rule(&mut out, "Failure", width, theme);
        push_wrapped(
            &mut out,
            &failure_tail,
            width,
            Style::default().fg(theme.accent_error),
        );
    } else if let Some(err) = &step.error {
        if !err.is_empty() {
            push_wrapped(
                &mut out,
                err,
                width,
                Style::default().fg(theme.accent_error),
            );
        }
    }

    if is_gate {
        if let Some(detail) = &run.summary.status_detail {
            if !detail.is_empty() {
                let color = if answered.is_some() {
                    theme.gray
                } else {
                    theme.warning
                };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    // Compact attempt digest (verdict, validation, file scope, short note, affordances)
    let verdict = extract_step_verdict(run, step_idx);
    let (validation, file_digest) = if step.kind == NodeKind::Acp {
        (
            extract_step_validations(run, step, &[]),
            digest_step_files(run, step, &[]),
        )
    } else {
        (Default::default(), Default::default())
    };
    let validation_badge = validation.badge_label();
    let file_summary = file_digest.summary_line();
    let note = if step.kind == NodeKind::Acp && !is_gate {
        extract_step_note(step, verdict.as_ref())
    } else {
        None
    };

    let has_brief_digest = verdict.is_some()
        || validation_badge.is_some()
        || !file_summary.is_empty()
        || note.is_some();

    if has_brief_digest {
        push_section_rule(&mut out, "Digest", width, theme);
        if let Some(v) = &verdict {
            push_wrapped(
                &mut out,
                &v.summary_line(),
                width,
                Style::default().fg(theme.text_primary),
            );
        }
        if let Some(badge) = &validation_badge {
            let color = if validation.resolved {
                theme.gray
            } else {
                theme.warning
            };
            push_wrapped(&mut out, badge, width, Style::default().fg(color));
        }
        if !file_summary.is_empty() {
            let color = if file_digest.out_of_scope_files.is_empty() {
                theme.text_secondary
            } else {
                theme.warning
            };
            push_wrapped(&mut out, &file_summary, width, Style::default().fg(color));
        }
        if let Some((note_text, _)) = &note {
            push_wrapped(
                &mut out,
                note_text,
                width,
                Style::default().fg(theme.text_secondary),
            );
        }
    }

    let affordance = if step.conversation.is_some() {
        "Enter inspect · r replay"
    } else {
        "Enter inspect"
    };
    push_wrapped(&mut out, affordance, width, Style::default().fg(theme.gray_dim));

    // Command (for actions, after any failure tail)
    if let Some(action) = &step.action {
        let mut cmd_parts = Vec::new();
        if let Some(cmd) = &action.command {
            if !cmd.is_empty() {
                cmd_parts.push(cmd.clone());
            }
        }
        for arg in &action.args {
            cmd_parts.push(arg.clone());
        }
        let has_cmd = !cmd_parts.is_empty();
        let has_cwd = action.cwd.as_deref().is_some_and(|c| !c.is_empty());
        if has_cmd || has_cwd {
            push_section_rule(&mut out, "Command", width, theme);
            if has_cmd {
                let cmd_line = cmd_parts.join(" ");
                push_wrapped(
                    &mut out,
                    &cmd_line,
                    width,
                    Style::default().fg(theme.command),
                );
            }
            if let Some(cwd) = &action.cwd {
                if !cwd.is_empty() {
                    push_wrapped(&mut out, cwd, width, Style::default().fg(theme.path));
                }
            }
        }
    }

    // When a finished agent step already has brief digest evidence, keep its
    // first view free of raw prompt dumps, redundant wrapper fields, and
    // repeated transcript; full evidence stays reachable via Enter / r.
    let suppress_agent_dumps = step.kind == NodeKind::Acp && has_brief_digest;

    // Prompt
    if !suppress_agent_dumps {
        if let Some(prompt) = &step.prompt_text {
            if !prompt.trim().is_empty() {
                push_section_rule(&mut out, "Prompt", width, theme);
                push_wrapped_capped(
                    &mut out,
                    prompt,
                    width,
                    14,
                    Style::default().fg(theme.text_secondary),
                    theme,
                );
            }
        }
    }

    // Conversation
    if !suppress_agent_dumps {
        if let Some(conv) = &step.conversation {
            push_section_rule(&mut out, "Conversation", width, theme);
            match run.session_events_path(&conv.session_id) {
                Some(path) => {
                    append_conversation(
                        &mut out,
                        path,
                        Some((conv.event_start_seq, conv.event_end_seq)),
                        None,
                        width,
                        theme,
                    );
                }
                None => {
                    out.push(dim_line("session events file not found", theme));
                }
            }
        }
    }

    // Result
    let show_result = !suppress_agent_dumps
        || has_extra_result_fields(run, step, note.as_ref().is_some_and(|(_, full)| *full));
    if show_result {
        let result_lines = format_value_lines(&step.output, width, theme);
        if !result_lines.is_empty() {
            push_section_rule(&mut out, "Result", width, theme);
            out.append_rows(result_lines);
        }
    }

    // stdout / stderr for actions
    if let Some(art) = &step.stdout {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stdout", &run.summary.dir, art, width, theme);
        }
    }
    if let Some(art) = &step.stderr {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stderr", &run.summary.dir, art, width, theme);
        }
    }

    // Events
    let evs: Vec<_> = run
        .step_events(&step.attempt_id)
        .into_iter()
        .filter(|e| e.kind != "node_heartbeat")
        .collect();
    if !evs.is_empty() {
        push_section_rule(&mut out, "Events", width, theme);
        let base_ms = if step.started_ms > 0 {
            step.started_ms
        } else {
            evs.first().map(|e| e.at_ms).unwrap_or(0)
        };
        for ev in evs {
            out.push(format_trace_event(ev, base_ms, width, theme));
        }
    }

    out
}

fn failure_tail_for_step(run: &Run, step: &Step) -> Option<String> {
    if effective_outcome(step) == Outcome::Ok {
        return None;
    }

    const TAIL_ARTIFACT_MAX_BYTES: usize = 64 * 1024;

    let error = step
        .error
        .as_deref()
        .or_else(|| step.output.get("error").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty());

    let inline_stderr = step
        .output
        .get("stderr")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());

    let artifact_stderr = if error.is_none() && inline_stderr.is_none() {
        step.stderr
            .as_ref()
            .filter(|a| a.bytes > 0)
            .and_then(|art| {
                bundle::read_artifact(&run.summary.dir, art, TAIL_ARTIFACT_MAX_BYTES).ok()
            })
            .filter(|s| !s.trim().is_empty())
    } else {
        None
    };

    let output_tail = step
        .output
        .get("outputTail")
        .or_else(|| step.output.get("output_tail"))
        .or_else(|| step.output.get("tail"))
        .or_else(|| step.output.get("message"))
        .and_then(Value::as_str)
        .or(step.raw_text.as_deref())
        .filter(|s| !s.trim().is_empty());

    let inline_stdout = step
        .output
        .get("stdout")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());

    let artifact_stdout = if error.is_none()
        && inline_stderr.is_none()
        && artifact_stderr.is_none()
        && output_tail.is_none()
        && inline_stdout.is_none()
    {
        step.stdout
            .as_ref()
            .filter(|a| a.bytes > 0)
            .and_then(|art| {
                bundle::read_artifact(&run.summary.dir, art, TAIL_ARTIFACT_MAX_BYTES).ok()
            })
            .filter(|s| !s.trim().is_empty())
    } else {
        None
    };

    if let Some(trimmed) = step_digest::trim_failure_tail(
        error,
        inline_stderr.or(artifact_stderr.as_deref()),
        output_tail,
        inline_stdout.or(artifact_stdout.as_deref()),
        non_zero_exit_code(step),
        step_digest::DEFAULT_FAILURE_MAX_LINES,
        step_digest::DEFAULT_FAILURE_MAX_CHARS,
    ) {
        return Some(trimmed);
    }

    step_failure_reason(step)
}

fn extract_step_note(step: &Step, verdict: Option<&StepVerdict>) -> Option<(String, bool)> {
    const NOTE_KEYS: &[&str] = &[
        "summary",
        "note",
        "notes",
        "message",
        "answer",
        "reason",
        "rationale",
        "explanation",
        "detail",
    ];

    let raw = match &step.output {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.starts_with('{') || trimmed.starts_with('[') {
                return None;
            }
            trimmed
        }
        Value::Object(map) => NOTE_KEYS.iter().find_map(|key| {
            map.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })?,
        _ => return None,
    };

    if let Some(v) = verdict {
        if let Some(reason) = v.reason.as_deref() {
            let reason_prefix = reason.trim_end_matches('…');
            if !reason_prefix.is_empty()
                && (raw == reason || raw.starts_with(reason_prefix) || reason.starts_with(raw))
            {
                return None;
            }
        }
    }

    Some(truncate_note_text(raw))
}

fn truncate_note_text(text: &str) -> (String, bool) {
    const MAX_LINES: usize = 3;
    const MAX_CHARS: usize = 240;

    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return (String::new(), true);
    }

    let mut truncated = lines.len() > MAX_LINES;
    let joined = lines
        .iter()
        .take(MAX_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");

    let char_count = joined.chars().count();
    let mut out = if char_count > MAX_CHARS {
        truncated = true;
        joined.chars().take(MAX_CHARS.saturating_sub(1)).collect()
    } else {
        joined
    };

    if truncated {
        out = out.trim_end().to_string();
        out.push('…');
    }

    (out, !truncated)
}

fn has_extra_result_fields(run: &Run, step: &Step, note_covered_entire_string: bool) -> bool {
    match &step.output {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty() && !note_covered_entire_string,
        Value::Array(arr) => !arr.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
        Value::Object(map) => {
            if map.is_empty() {
                return false;
            }
            let switch_on_key = run.flow.edges.iter().find_map(|edge| match edge {
                FlowEdge::Switch { from, on, .. } if from == &step.node_id => on.split('.').next(),
                _ => None,
            });
            map.keys()
                .any(|key| !is_digest_covered_key(key, switch_on_key))
        }
    }
}

fn is_digest_covered_key(key: &str, switch_on_key: Option<&str>) -> bool {
    if switch_on_key == Some(key) {
        return true;
    }
    matches!(
        key,
        "verdict"
            | "decision"
            | "route"
            | "reason"
            | "rationale"
            | "summary"
            | "explanation"
            | "detail"
            | "notes"
            | "note"
            | "message"
            | "answer"
            | "ok"
            | "status"
    )
}

fn append_artifact_section(
    out: &mut Rows,
    title: &str,
    run_dir: &Path,
    artifact: &bundle::ArtifactRef,
    width: usize,
    theme: &Theme,
) {
    const MAX_BYTES: usize = 64 * 1024;
    match bundle::read_artifact(run_dir, artifact, MAX_BYTES) {
        Ok(text) => {
            if text.is_empty() {
                return;
            }
            push_section_rule(out, title, width, theme);
            let raw_lines: Vec<&str> = text.lines().collect();
            let start = raw_lines.len().saturating_sub(30);
            for raw in &raw_lines[start..] {
                push_wrapped(out, raw, width, Style::default().fg(theme.text_primary));
            }
            if artifact.bytes > MAX_BYTES as u64 {
                out.push(dim_line("… output cut at 64 KiB", theme));
            }
        }
        Err(_) => {
            push_section_rule(out, title, width, theme);
            out.push(dim_line("artifact unreadable", theme));
        }
    }
}

fn build_node_lines(
    run: &Run,
    node_id: &str,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Rows {
    let mut out = Rows::new();
    let state = run.node_state(node_id);
    let shown = run.node_state_answered(node_id, answered.is_some());
    let glyph = shown.glyph();
    let color = node_state_color(shown, theme);

    out.push(title_row_right(glyph, color, node_id, "", width, theme));

    let settings = bundle::node_settings(&run.summary.dir);
    let agent_lbl = settings
        .get(node_id)
        .map(AgentSettings::label)
        .filter(|s| !s.is_empty());
    let flow_node = run.flow.nodes.iter().find(|n| n.id == node_id);
    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        facts.push(lbl);
    }
    if let Some(n) = flow_node {
        facts.push(n.kind.label().to_string());
    }
    if !facts.is_empty() {
        out.push(Line::from(Span::styled(
            truncate_str(&facts.join(" · "), width),
            Style::default().fg(theme.gray),
        )));
    }

    match state {
        NodeState::Running => {
            push_section_rule(&mut out, "Conversation (live)", width, theme);
            let handle = settings
                .get(node_id)
                .and_then(|s| s.handle.as_deref())
                .unwrap_or("main");
            if let Some(sess) = run.sessions.iter().find(|s| s.handle == handle) {
                let min_exclusive = run
                    .steps
                    .iter()
                    .filter_map(|s| s.conversation.as_ref())
                    .filter(|c| c.session_id == sess.id)
                    .map(|c| c.event_end_seq)
                    .max();
                append_conversation(
                    &mut out,
                    &sess.events_path,
                    None,
                    min_exclusive,
                    width,
                    theme,
                );
            } else {
                out.push(dim_line("no session bound yet", theme));
            }

            let finished_attempts: std::collections::HashSet<&str> =
                run.steps.iter().map(|s| s.attempt_id.as_str()).collect();
            let evs: Vec<_> = run
                .events
                .iter()
                .filter(|e| {
                    e.node_id.as_deref() == Some(node_id)
                        && e.attempt_id
                            .as_deref()
                            .map(|a| !finished_attempts.contains(a))
                            .unwrap_or(true)
                        && e.kind != "node_heartbeat"
                })
                .collect();
            if !evs.is_empty() {
                push_section_rule(&mut out, "Events", width, theme);
                let base_ms = evs.first().map(|e| e.at_ms).unwrap_or(run.summary.started_ms);
                for ev in evs {
                    out.push(format_trace_event(ev, base_ms, width, theme));
                }
            }
        }
        NodeState::Waiting => {
            if let Some(detail) = &run.summary.status_detail {
                if !detail.is_empty() {
                    let color = if answered.is_some() { theme.gray } else { theme.warning };
                    push_wrapped(&mut out, detail, width, Style::default().fg(color));
                }
            }
            append_gate_section(&mut out, run, answered, width, theme);
        }
        NodeState::NotRun => {
            let msg = if matches!(
                run.summary.status,
                RunStatus::Running | RunStatus::Waiting
            ) {
                "not run yet"
            } else {
                "not run"
            };
            out.push(dim_line(msg, theme));
        }
        NodeState::Done(_) => {
            out.push(dim_line("completed", theme));
        }
    }

    out
}

fn append_gate_section(
    out: &mut Rows,
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) {
    let choices = bundle::gate_choices(run);
    if choices.is_empty() && answered.is_some() {
        return;
    }
    push_section_rule(out, "Gate", width, theme);
    if choices.is_empty() {
        out.push(dim_line(
            "no next step declared: re-run the flow to continue",
            theme,
        ));
        return;
    }
    for (idx, (label, flow_file)) in choices.iter().enumerate() {
        let spans = match answered {
            // Answered: nothing to pick any more, so no key numbers; the picked one is ticked.
            Some(picked) => {
                let chosen = picked.eq_ignore_ascii_case(label);
                let (mark, mark_color, label_style) = if chosen {
                    (
                        "✓ ",
                        theme.accent_success,
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", theme.gray_dim, Style::default().fg(theme.gray))
                };
                vec![
                    Span::styled(mark, Style::default().fg(mark_color)),
                    Span::styled(format!("{label} "), label_style),
                    Span::styled("→ ", Style::default().fg(theme.gray_dim)),
                    Span::styled(flow_file.clone(), Style::default().fg(theme.gray)),
                ]
            }
            None => {
                let n = idx + 1;
                vec![
                    Span::styled(
                        format!("[{n}] "),
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{label} "),
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("→ ", Style::default().fg(theme.gray_dim)),
                    Span::styled(flow_file.clone(), Style::default().fg(theme.path)),
                ]
            }
        };
        out.push(truncate_line(Line::from(spans), width));
    }
}

#[derive(Debug, Clone)]
enum ConvItem {
    Prompt(String),
    Thought(String),
    ToolCall {
        id: String,
        title: String,
        status: String,
    },
    Message(String),
}

fn append_conversation(
    out: &mut Rows,
    path: &Path,
    seq_range: Option<(u64, u64)>,
    min_exclusive_seq: Option<u64>,
    width: usize,
    theme: &Theme,
) {
    let raw = match read_last_512k(path) {
        Ok(s) => s,
        Err(_) => {
            out.push(dim_line("session events file unavailable", theme));
            return;
        }
    };

    let items = parse_conversation_items(&raw, seq_range, min_exclusive_seq);
    if items.is_empty() {
        out.push(dim_line("no conversation events", theme));
        return;
    }

    let mut prev_kind: Option<u8> = None;
    for item in items {
        let kind_tag = match &item {
            ConvItem::Prompt(_) => 1,
            ConvItem::Thought(_) => 2,
            ConvItem::ToolCall { .. } => 3,
            ConvItem::Message(_) => 4,
        };
        if let Some(prev) = prev_kind {
            if prev != kind_tag {
                out.push(Line::default());
            }
        }
        prev_kind = Some(kind_tag);

        match item {
            ConvItem::Prompt(text) => {
                let wrapped = wrap(&text, width);
                let style = Style::default().fg(theme.gray_dim);
                for line in wrapped.into_iter().take(3) {
                    out.push(Line::from(Span::styled(line, style)));
                }
            }
            ConvItem::Thought(text) => {
                let body_w = width.saturating_sub(2).max(1);
                let wrapped = wrap(&text, body_w);
                let thought_style = Style::default()
                    .fg(theme.gray)
                    .add_modifier(Modifier::ITALIC);
                for (idx, line) in wrapped.into_iter().enumerate() {
                    let prefix = if idx == 0 { "∴ " } else { "  " };
                    out.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
                        Span::styled(line, thought_style),
                    ]));
                }
            }
            ConvItem::ToolCall { title, status, .. } => {
                let status_color = match status.as_str() {
                    "completed" => theme.accent_success,
                    "failed" => theme.accent_error,
                    _ => theme.accent_running,
                };
                let status_w = UnicodeWidthStr::width(status.as_str());
                let max_title_w = width.saturating_sub(status_w + 4).max(1);
                let clean_title = first_non_empty_line(&title);
                let cut_title = truncate_str(
                    if clean_title.is_empty() {
                        "tool"
                    } else {
                        &clean_title
                    },
                    max_title_w,
                );
                out.push(Line::from(vec![
                    Span::styled("▸ ", Style::default().fg(theme.accent_tool)),
                    Span::styled(cut_title, Style::default().fg(theme.text_secondary)),
                    Span::styled("  ", Style::default()),
                    Span::styled(status, Style::default().fg(status_color)),
                ]));
            }
            ConvItem::Message(text) => {
                push_wrapped(
                    out,
                    &text,
                    width,
                    Style::default().fg(theme.text_primary),
                );
            }
        }
    }
}

fn read_last_512k(path: &Path) -> std::io::Result<String> {
    const LIMIT: u64 = 512 * 1024;
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let cut = len > LIMIT;
    if cut {
        f.seek(SeekFrom::End(-(LIMIT as i64)))?;
    }
    let mut bytes = Vec::with_capacity(len.min(LIMIT) as usize);
    f.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    if cut {
        if let Some(pos) = text.find('\n') {
            return Ok(text[pos + 1..].to_string());
        }
        return Ok(String::new());
    }
    Ok(text.into_owned())
}

fn parse_conversation_items(
    raw: &str,
    seq_range: Option<(u64, u64)>,
    min_exclusive_seq: Option<u64>,
) -> Vec<ConvItem> {
    let mut items: Vec<ConvItem> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let seq = val.get("seq").and_then(Value::as_u64).unwrap_or(0);
        if let Some((start, end)) = seq_range {
            if seq < start || seq > end {
                continue;
            }
        }
        if let Some(min_seq) = min_exclusive_seq {
            if seq <= min_seq {
                continue;
            }
        }
        let Some(msg) = val.get("message") else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "_auth/status_update" {
            continue;
        }
        let direction = val.get("direction").and_then(Value::as_str).unwrap_or("");

        if direction == "outbound" && method == "session/prompt" {
            if let Some(prompt_arr) = msg
                .get("params")
                .and_then(|p| p.get("prompt"))
                .and_then(Value::as_array)
            {
                let mut parts = Vec::new();
                for part in prompt_arr {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        if !t.is_empty() {
                            parts.push(t);
                        }
                    }
                }
                if !parts.is_empty() {
                    items.push(ConvItem::Prompt(parts.join("\n")));
                }
            }
            continue;
        }

        if direction == "inbound" && method == "session/update" {
            let Some(update) = msg.get("params").and_then(|p| p.get("update")) else {
                continue;
            };
            let kind = update
                .get("sessionUpdate")
                .and_then(Value::as_str)
                .unwrap_or("");
            match kind {
                "agent_thought_chunk" => {
                    if let Some(text) = update
                        .get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(Value::as_str)
                    {
                        if let Some(ConvItem::Thought(prev)) = items.last_mut() {
                            prev.push_str(text);
                        } else {
                            items.push(ConvItem::Thought(text.to_string()));
                        }
                    }
                }
                "agent_message_chunk" => {
                    if let Some(text) = update
                        .get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(Value::as_str)
                    {
                        if let Some(ConvItem::Message(prev)) = items.last_mut() {
                            prev.push_str(text);
                        } else {
                            items.push(ConvItem::Message(text.to_string()));
                        }
                    }
                }
                "tool_call" => {
                    let id = update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let title = update
                        .get("title")
                        .and_then(Value::as_str)
                        .or_else(|| update.get("kind").and_then(Value::as_str))
                        .unwrap_or("tool")
                        .to_string();
                    let status = update
                        .get("status")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("running")
                        .to_string();
                    items.push(ConvItem::ToolCall { id, title, status });
                }
                "tool_call_update" => {
                    let target_id = update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let new_title = update
                        .get("title")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    let new_status = update
                        .get("status")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());

                    if let Some(ConvItem::ToolCall { title, status, .. }) = items
                        .iter_mut()
                        .rev()
                        .find(|it| matches!(it, ConvItem::ToolCall { id, .. } if id == target_id))
                    {
                        if let Some(t) = new_title {
                            *title = t.to_string();
                        }
                        if let Some(s) = new_status {
                            *status = s.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    items
}

fn format_trace_event(
    ev: &bundle::TraceEvent,
    base_ms: i64,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let delta_ms = ev.at_ms.saturating_sub(base_ms).max(0) as u64;
    let prefix = format!("+{delta_ms}ms  ");
    let kind_str = format!("{}", ev.kind);
    let mut spans = vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(kind_str, Style::default().fg(theme.gray)),
    ];
    if !ev.detail.is_empty() {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            ev.detail.clone(),
            Style::default().fg(theme.text_secondary),
        ));
    }
    truncate_line(Line::from(spans), width)
}

fn push_section_rule(
    out: &mut Rows,
    name: &str,
    width: usize,
    theme: &Theme,
) {
    out.push(Line::default());
    let prefix = "── ";
    let mid = format!("{name} ");
    let used = UnicodeWidthStr::width(prefix) + UnicodeWidthStr::width(mid.as_str());
    let rem = width.saturating_sub(used);
    let trail = "─".repeat(rem);
    let rule = Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(
            mid,
            Style::default()
                .fg(theme.gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(trail, Style::default().fg(theme.gray_dim)),
    ]);
    out.push_unit(vec![rule], name.to_string());
}

fn title_row_right(
    glyph: &str,
    glyph_color: Color,
    title: &str,
    right: &str,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let right_w = UnicodeWidthStr::width(right);
    let left_max = if right_w > 0 {
        width.saturating_sub(right_w + 2)
    } else {
        width
    };
    let glyph_part = format!("{glyph} ");
    let glyph_w = UnicodeWidthStr::width(glyph_part.as_str());
    let title_cut = truncate_str(title, left_max.saturating_sub(glyph_w));
    let left_used = glyph_w + UnicodeWidthStr::width(title_cut.as_str());

    let mut spans = vec![
        Span::styled(
            glyph_part,
            Style::default().fg(glyph_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title_cut,
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if right_w > 0 && left_used + 1 + right_w <= width {
        let pad = " ".repeat(width - left_used - right_w);
        spans.push(Span::raw(pad));
        spans.push(Span::styled(
            right.to_string(),
            Style::default().fg(theme.gray),
        ));
    }
    Line::from(spans)
}

fn push_wrapped(
    out: &mut Rows,
    text: &str,
    width: usize,
    style: Style,
) {
    for src_line in source_lines(text) {
        let rows: Vec<Line<'static>> = wrap(src_line, width)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, style)))
            .collect();
        out.push_unit(rows, src_line.to_string());
    }
}

fn push_wrapped_capped(
    out: &mut Rows,
    text: &str,
    width: usize,
    max_rows: usize,
    style: Style,
    theme: &Theme,
) {
    let mut emitted = 0usize;
    let mut total = 0usize;
    for src_line in source_lines(text) {
        let wrapped = wrap(src_line, width);
        let line_rows = wrapped.len();
        total += line_rows;
        if emitted < max_rows {
            let take = (max_rows - emitted).min(line_rows);
            let rows: Vec<Line<'static>> = wrapped
                .into_iter()
                .take(take)
                .map(|line| Line::from(Span::styled(line, style)))
                .collect();
            emitted += rows.len();
            out.push_unit(rows, src_line.to_string());
        }
    }
    if total > max_rows {
        let more = total - max_rows;
        out.push(dim_line(&format!("… +{more} more rows"), theme));
    }
}

fn dim_line(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(theme.gray_dim),
    ))
}

fn truncate_line(line: Line<'static>, width: usize) -> Line<'static> {
    let mut remaining = width;
    let mut spans = Vec::new();
    for span in line.spans {
        if remaining == 0 {
            break;
        }
        let w = UnicodeWidthStr::width(span.content.as_ref());
        if w <= remaining {
            remaining -= w;
            spans.push(span);
        } else {
            let cut = truncate_str(span.content.as_ref(), remaining);
            spans.push(Span::styled(cut, span.style));
            break;
        }
    }
    Line::from(spans)
}

fn run_status_color(status: RunStatus, theme: &Theme) -> Color {
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

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        let mut clean = String::with_capacity(raw_line.len());
        for ch in raw_line.chars() {
            if ch == '\t' {
                clean.push_str("    ");
            } else if !ch.is_control() {
                clean.push(ch);
            }
        }
        if clean.is_empty() {
            out.push(String::new());
            continue;
        }

        let mut cur = String::new();
        let mut cur_w = 0usize;

        let mut chars = clean.chars().peekable();
        while let Some(&ch) = chars.peek() {
            if ch == ' ' {
                let mut sp_count = 0usize;
                while chars.peek() == Some(&' ') {
                    chars.next();
                    sp_count += 1;
                }
                if cur.is_empty() {
                    for _ in 0..sp_count {
                        if cur_w + 1 > width {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        cur.push(' ');
                        cur_w += 1;
                    }
                } else if chars.peek().is_some() {
                    if cur_w + sp_count <= width {
                        for _ in 0..sp_count {
                            cur.push(' ');
                        }
                        cur_w += sp_count;
                    } else {
                        out.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                }
            } else {
                let mut word = String::new();
                let mut word_w = 0usize;
                while let Some(&c) = chars.peek() {
                    if c == ' ' {
                        break;
                    }
                    chars.next();
                    word.push(c);
                    word_w += UnicodeWidthChar::width(c).unwrap_or(0);
                }

                if word_w <= width {
                    if cur_w + word_w <= width {
                        cur.push_str(&word);
                        cur_w += word_w;
                    } else {
                        let trimmed = cur.trim_end_matches(' ').to_string();
                        out.push(trimmed);
                        cur = word;
                        cur_w = word_w;
                    }
                } else {
                    if !cur.is_empty() {
                        let trimmed = cur.trim_end_matches(' ').to_string();
                        if !trimmed.is_empty() {
                            out.push(trimmed);
                        }
                        cur.clear();
                        cur_w = 0;
                    }
                    for c in word.chars() {
                        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                        if cur_w + cw > width && !cur.is_empty() {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        cur.push(c);
                        cur_w += cw;
                    }
                }
            }
        }

        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        ActionReceipt, ArtifactRef, Conversation, FlowDef, FlowEdge, FlowNode, RunSummary,
        SessionRef, TraceEvent,
    };
    use serde_json::json;

    fn make_test_run(dir: &Path) -> Run {
        Run {
            summary: RunSummary {
                run_id: "run-1".to_string(),
                dir: dir.to_path_buf(),
                flow_name: "feel".to_string(),
                title: "feel: /tmp/test".to_string(),
                status: RunStatus::Running,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: Some("work".to_string()),
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "feel".to_string(),
                start_at: "list".to_string(),
                nodes: vec![
                    FlowNode {
                        id: "list".to_string(),
                        kind: NodeKind::Action,
                        summary: None,
                        is_shell: true,
                        ..Default::default()
                    },
                    FlowNode {
                        id: "work".to_string(),
                        kind: NodeKind::Acp,
                        summary: None,
                        is_shell: false,
                        ..Default::default()
                    },
                    FlowNode {
                        id: "review".to_string(),
                        kind: NodeKind::Checkpoint,
                        summary: Some("human review".to_string()),
                        is_shell: false,
                        ..Default::default()
                    },
                ],
                edges: vec![
                    FlowEdge::Simple {
                        from: "list".to_string(),
                        to: "work".to_string(),
                    },
                    FlowEdge::Simple {
                        from: "work".to_string(),
                        to: "review".to_string(),
                    },
                ],
                ..Default::default()
            },
            steps: Vec::new(),
            events: Vec::new(),
            input: json!({"target": "/tmp/test"}),
            sessions: Vec::new(),
        }
    }

    fn buffer_lines(buf: &Buffer) -> Vec<String> {
        let area = buf.area;
        let mut rows = Vec::new();
        for y in area.y..area.y + area.height {
            let mut s = String::new();
            for x in area.x..area.x + area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            rows.push(s.trim_end().to_string());
        }
        rows
    }

    #[test]
    fn test_wrap_breaks_on_spaces_and_keeps_blank_lines() {
        let wrapped = wrap("hello world\n\nfoo bar baz", 10);
        assert_eq!(wrapped, vec!["hello", "world", "", "foo bar", "baz"]);
    }

    #[test]
    fn test_wrap_hard_breaks_long_words_and_tabs() {
        let wrapped = wrap("abcdefghijkl\t\tx", 5);
        assert_eq!(wrapped[0], "abcde");
        assert_eq!(wrapped[1], "fghij");
        assert!(wrapped.iter().any(|l| l.contains("kl")));
    }

    #[test]
    fn test_wrap_wide_characters() {
        // Each CJK char has display width 2
        let wrapped = wrap("你好世界测试", 6);
        assert_eq!(wrapped, vec!["你好世", "界测试"]);
    }

    #[test]
    fn test_step_sections_order_prompt_result_events() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_500,
            prompt_text: Some("Inspect the repository".to_string()),
            raw_text: None,
            output: json!({"answer": "all good"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.events.push(TraceEvent {
            seq: 1,
            at_ms: 1_100,
            scope: "node".to_string(),
            kind: "node_started".to_string(),
            node_id: Some("work".to_string()),
            attempt_id: Some("work#1".to_string()),
            detail: "Running acp node".to_string(),
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 40),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let text = buffer_lines(&buf).join("\n");
        let d_idx = text.find("── Digest").expect("Digest section");
        let e_idx = text.find("── Events").expect("Events section");
        assert!(d_idx < e_idx, "{text}");
        assert!(text.contains("all good"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");
        assert!(!text.contains("── Prompt"), "{text}");
    }

    #[test]
    fn test_action_step_shows_command_exit_code_and_stdout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let art_dir = tmp.path().join("artifacts");
        std::fs::create_dir_all(&art_dir).expect("mkdir");
        let stdout_rel = "artifacts/out.txt";
        std::fs::write(tmp.path().join(stdout_rel), "alpha\nbeta\n").expect("write");

        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_050,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: Some(ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("ls".to_string()),
                args: vec!["-la".to_string(), "/tmp/test".to_string()],
                cwd: Some("/private/tmp/test".to_string()),
                exit_code: Some(0),
                duration_ms: Some(12),
            }),
            conversation: None,
            stdout: Some(ArtifactRef {
                path: stdout_rel.to_string(),
                media_type: "text/plain".to_string(),
                bytes: 11,
            }),
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 30),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let text = buffer_lines(&buf).join("\n");
        assert!(text.contains("exit 0"));
        assert!(text.contains("── Command"));
        assert!(text.contains("ls -la /tmp/test"));
        assert!(text.contains("── stdout"));
        assert!(text.contains("alpha"));
        assert!(text.contains("beta"));
    }

    #[test]
    fn test_conversation_thought_tool_call_and_message() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let lines = [
            json!({"seq": 1, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_thought_chunk", "content": {"text": "Thinking part 1 "}}}}}),
            json!({"seq": 2, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_thought_chunk", "content": {"text": "and part 2."}}}}}),
            json!({"seq": 3, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "tool_call", "toolCallId": "tc1", "title": "Bash", "kind": "execute"}}}}),
            json!({"seq": 4, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "tool_call_update", "toolCallId": "tc1", "title": "ls -la", "status": "completed"}}}}),
            json!({"seq": 5, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "Hello "}}}}}),
            json!({"seq": 6, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "world!"}}}}}),
        ];
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&events_path, body).expect("write events");

        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path,
            agent_name: Some("claude".to_string()),
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 1,
                event_end_seq: 6,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 30),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let rendered = buffer_lines(&buf);
        let full = rendered.join("\n");
        assert!(full.contains("∴ Thinking part 1 and part 2."));
        let tool_rows: Vec<_> = rendered.iter().filter(|l| l.contains('▸')).collect();
        assert_eq!(tool_rows.len(), 1);
        assert!(tool_rows[0].contains("ls -la"));
        assert!(tool_rows[0].contains("completed"));
        assert!(full.contains("Hello world!"));
    }

    #[test]
    fn test_conversation_filters_seq_range_and_skips_auth_status_update() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let lines = [
            json!({"seq": 1, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "OUTSIDE_BEFORE"}}}}}),
            json!({"seq": 2, "direction": "inbound", "message": {"method": "_auth/status_update", "params": {"secret": "SUPER_SECRET_TOKEN_123", "update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "SUPER_SECRET_TOKEN_123"}}}}}),
            json!({"seq": 3, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "INSIDE_RANGE"}}}}}),
            json!({"seq": 4, "direction": "inbound", "message": {"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "OUTSIDE_AFTER"}}}}}),
        ];
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&events_path, body).expect("write");

        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path,
            agent_name: None,
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 2,
                event_end_seq: 3,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 25));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 25),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("INSIDE_RANGE"));
        assert!(!full.contains("OUTSIDE_BEFORE"));
        assert!(!full.contains("OUTSIDE_AFTER"));
        assert!(!full.contains("SUPER_SECRET_TOKEN_123"));
    }

    #[test]
    fn test_missing_events_file_renders_dim_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path: tmp.path().join("does-not-exist.ndjson"),
            agent_name: None,
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 1,
                event_end_seq: 5,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 20),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("── Conversation"));
        assert!(full.contains("unavailable"));
    }

    #[test]
    fn test_run_target_lists_stages_and_waiting_gate_choices() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.summary.status = RunStatus::Waiting;
        run.summary.waiting_on = Some("review".to_string());
        run.summary.status_detail = Some("awaiting approval".to_string());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_100,
            prompt_text: None,
            raw_text: None,
            output: json!({"ok": true}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.steps.push(Step {
            attempt_id: "review#1".to_string(),
            node_id: "review".to_string(),
            kind: NodeKind::Checkpoint,
            outcome: Outcome::Ok,
            started_ms: 1_200,
            finished_ms: 1_210,
            prompt_text: None,
            raw_text: None,
            output: json!({
                "checkpoint": "review",
                "next": {
                    "ship": "deploy.flow.ts",
                    "abort": "rollback.flow.ts"
                }
            }),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 35));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 90, 35),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("awaiting approval"));
        assert!(full.contains("── Gate"));
        assert!(full.contains("[1] abort → rollback.flow.ts"));
        assert!(full.contains("[2] ship → deploy.flow.ts"));
        assert!(full.contains("── Stages"));
        assert!(full.contains("✓ list"));
        assert!(full.contains("◐ review"));
    }

    /// A run waiting at checkpoint `review`, whose last step offers two choices.
    fn waiting_gate_run(dir: &Path) -> Run {
        let mut run = make_test_run(dir);
        run.summary.status = RunStatus::Waiting;
        run.summary.waiting_on = Some("review".to_string());
        run.summary.status_detail = Some("land it, or send it back?".to_string());
        for (id, node, kind, output) in [
            ("list#1", "list", NodeKind::Action, json!({"ok": true})),
            (
                "review#1",
                "review",
                NodeKind::Checkpoint,
                json!({"next": {"ship": "deploy.flow.ts", "abort": "rollback.flow.ts"}}),
            ),
        ] {
            run.steps.push(Step {
                attempt_id: id.to_string(),
                node_id: node.to_string(),
                kind,
                outcome: Outcome::Ok,
                started_ms: 1_000,
                finished_ms: 1_100,
                prompt_text: None,
                raw_text: None,
                output,
                error: None,
                agent_name: None,
                action: None,
                conversation: None,
                stdout: None,
                stderr: None,
                output_artifact: None,
            });
        }
        run
    }

    fn panel_text(
        run: &Run,
        target: &DetailTarget,
        answered: Option<&str>,
    ) -> String {
        let mut panel = DetailPanel::new();
        panel.set_answered(answered);
        let area = Rect::new(0, 0, 90, 35);
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, run, target, &Theme::default(), 5_000);
        buffer_lines(&buf).join("\n")
    }

    #[test]
    fn answered_gate_is_drawn_done_without_pickable_keys() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());

        // Open gate, for contrast: live glyph and key numbers.
        let open_step = panel_text(&run, &DetailTarget::Step(1), None);
        assert!(open_step.contains("◐ review"));
        assert!(open_step.contains("[1] abort → rollback.flow.ts"));

        for target in [DetailTarget::Step(1), DetailTarget::Run] {
            let text = panel_text(&run, &target, Some("ship"));
            assert!(text.contains("── Gate"), "{target:?}\n{text}");
            assert!(text.contains("✓ ship → deploy.flow.ts"), "{target:?}\n{text}");
            assert!(text.contains("  abort → rollback.flow.ts"), "{target:?}\n{text}");
            assert!(!text.contains("[1]") && !text.contains("[2]"), "{target:?}\n{text}");
            assert!(!text.contains('◐'), "{target:?}\n{text}");
            assert!(text.contains("✓ review"), "{target:?}\n{text}");
        }
    }

    #[test]
    fn answered_gate_with_unknown_choice_ticks_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());
        let text = panel_text(&run, &DetailTarget::Step(1), Some(""));
        assert!(text.contains("✓ review"));
        assert!(text.contains("  ship → deploy.flow.ts"));
        assert!(!text.contains("✓ ship") && !text.contains("✓ abort"));
        assert!(!text.contains("[1]"));
    }

    #[test]
    fn answer_arriving_redraws_a_cached_panel() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());
        let mut panel = DetailPanel::new();
        let area = Rect::new(0, 0, 90, 35);
        let theme = Theme::default();
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Step(1), &theme, 5_000);
        assert!(buffer_lines(&buf).join("\n").contains("[1] abort"));

        panel.set_answered(Some("abort"));
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Step(1), &theme, 5_000);
        let text = buffer_lines(&buf).join("\n");
        assert!(text.contains("✓ abort → rollback.flow.ts"));
        assert!(!text.contains("[1]"));
    }

    #[test]
    fn empty_list_and_object_values_read_as_empty_in_dim_style() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!({"verdict": "approve", "findings": [], "meta": {}, "items": [[], {}]}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        let theme = Theme::default();

        // Key/value rows: key, then (empty) in the same colour as the key.
        let lines = build_step_lines(&run, 0, &run.steps[0], None, 60, &theme);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert!(text.iter().any(|r| r.starts_with("findings") && r.ends_with("(empty)")), "{text:?}");
        assert!(text.iter().any(|r| r.starts_with("meta") && r.ends_with("(empty)")), "{text:?}");
        assert_eq!(text.iter().filter(|r| r.contains("- (empty)")).count(), 2, "{text:?}");
        let empties: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|sp| sp.content.as_ref() == "(empty)")
            .collect();
        assert!(!empties.is_empty());
        // The same colour as the keys next to it, not fainter.
        assert!(empties.iter().all(|sp| sp.style.fg == Some(theme.gray)));
        let key = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|sp| sp.content.trim() == "findings")
            .expect("findings key");
        assert_eq!(key.style.fg, empties[0].style.fg);

        // Stages one-line summary.
        let run_lines = build_run_lines(&run, None, 90, &theme, 5_000);
        let run_text: String = run_lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(run_text.contains("verdict approve, findings (empty), meta (empty)"), "{run_text}");
    }

    #[test]
    fn test_node_target_not_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.summary.status = RunStatus::Completed;
        run.summary.current_node = None;

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 15));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 15),
            &run,
            &DetailTarget::Node("review".to_string()),
            &Theme::default(),
            5_000,
        );
        let full = buffer_lines(&buf).join("\n");
        assert!(full.contains("○ review"));
        assert!(full.contains("not run"));
    }

    #[test]
    fn test_scroll_clamps_and_returns_false_at_limits() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        let long_prompt = (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some(long_prompt),
            raw_text: None,
            output: json!({"a": 1, "b": 2, "c": 3, "d": 4, "e": 5}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 8));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 8),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );

        assert!(!panel.scroll(-1));
        assert!(panel.scroll(2));
        assert!(panel.scroll(1_000));
        assert!(!panel.scroll(1));
        assert!(panel.scroll(-1));
    }

    #[test]
    fn test_target_change_resets_scroll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm".to_string()),
            raw_text: None,
            output: json!({"res": "1\n2\n3\n4\n5\n6\n7\n8"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll(5));
        assert!(panel.scroll > 0);

        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, 0);
    }

    #[test]
    fn wrapped_failure_line_copies_exact_source_on_second_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        let long_path =
            "/workspace/crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs";
        assert!(long_path.len() >= 70);
        let err_rel = "artifacts/stderr.txt";
        let err_path = tmp.path().join(err_rel);
        std::fs::create_dir_all(err_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&err_path, format!("{long_path}\nsecond failure line\n")).expect("write");

        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Failed,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!(long_path),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: Some(ArtifactRef {
                path: err_rel.to_string(),
                media_type: "text/plain".to_string(),
                bytes: (long_path.len() + 21) as u64,
            }),
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let area = Rect::new(0, 0, 32, 30);
        let mut buf = Buffer::empty(area);
        panel.render(
            &mut buf,
            area,
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        panel.set_focused(true);

        // Find the wrapped rows produced for the 70-character path in Result and stderr.
        let matching_rows: Vec<usize> = panel
            .lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                let text = line_plain_text(line);
                if text.starts_with("/workspace/crates/codegen/") {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();
        assert!(!matching_rows.is_empty(), "expected wrapped path rows");
        for first_row in matching_rows {
            // Second wrapped row belongs to the same copy unit as the first row.
            panel.cursor = first_row + 1;
            assert_eq!(panel.copy_selection().as_deref(), Some(long_path));
        }

        // Visual range over the wrapped path unit and the next unit copies each unit once.
        let stderr_path_row = panel
            .lines
            .iter()
            .enumerate()
            .rposition(|(_, line)| line_plain_text(line).starts_with("/workspace/crates/codegen/"))
            .expect("stderr path row");
        // The Failure section repeats the line earlier; take the stderr occurrence.
        let second_line_row = panel
            .lines
            .iter()
            .rposition(|line| line_plain_text(line) == "second failure line")
            .expect("second failure line row");
        assert!(second_line_row > stderr_path_row + 1);
        panel.cursor = stderr_path_row + 1;
        assert!(panel.toggle_visual());
        panel.cursor = second_line_row;
        assert_eq!(
            panel.copy_selection().as_deref(),
            Some(format!("{long_path}\nsecond failure line").as_str())
        );

        // Copying all units omits section-rule dashes.
        let all = panel.copy_all().expect("copy all");
        assert!(all.contains(long_path));
        assert!(all.contains("Result"));
        assert!(all.contains("stderr"));
        assert!(!all.contains('─'), "copied text must not include rule dashes:\n{all}");
    }

    #[test]
    fn focus_highlight_visual_range_and_target_change_reset_cursor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!("first\nsecond\nthird"),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let theme = Theme::default();
        let area = Rect::new(0, 0, 50, 12);
        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(area);

        // Unfocused: no row receives bg_highlight.
        panel.render(&mut buf, area, &run, &DetailTarget::Step(0), &theme, 5_000);
        for y in 0..area.height {
            assert_ne!(buf[(0, y)].bg, theme.bg_highlight);
        }

        // Focused with visual range across rows 1..=2 highlights rows 1 and 2 only.
        panel.set_focused(true);
        assert!(panel.move_cursor(1));
        assert!(panel.toggle_visual());
        assert!(panel.move_cursor(1));
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Step(0), &theme, 5_000);
        assert_ne!(buf[(0, 0)].bg, theme.bg_highlight);
        assert_eq!(buf[(0, 1)].bg, theme.bg_highlight);
        assert_eq!(buf[(area.width - 1, 1)].bg, theme.bg_highlight);
        assert_eq!(buf[(0, 2)].bg, theme.bg_highlight);
        assert_ne!(buf[(0, 3)].bg, theme.bg_highlight);

        // Switching target resets cursor to 0 and ends visual mode.
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Run, &theme, 5_000);
        assert_eq!(panel.cursor, 0);
        assert!(!panel.is_visual());
        assert_eq!(buf[(0, 0)].bg, theme.bg_highlight);
        assert_ne!(buf[(0, 1)].bg, theme.bg_highlight);
    }

    #[test]
    fn test_failed_shell_orders_failure_tail_before_command_and_cwd_without_success() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let art_dir = tmp.path().join("artifacts");
        std::fs::create_dir_all(&art_dir).expect("mkdir");
        let stderr_rel = "artifacts/err.txt";
        let stderr_body = (0..10)
            .map(|i| format!("err line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(tmp.path().join(stderr_rel), &stderr_body).expect("write stderr");

        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            // Raw outcome is Ok, but nonzero exit code must render as failed.
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_250,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: Some(ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("cargo".to_string()),
                args: vec!["test".to_string(), "--lib".to_string()],
                cwd: Some("/private/tmp/repo".to_string()),
                exit_code: Some(1),
                duration_ms: Some(250),
            }),
            conversation: None,
            stdout: None,
            stderr: Some(ArtifactRef {
                path: stderr_rel.to_string(),
                media_type: "text/plain".to_string(),
                bytes: stderr_body.len() as u64,
            }),
            output_artifact: None,
        });

        let text = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(text.contains("✗ list"), "{text}");
        assert!(!text.contains("✓ list"), "{text}");
        assert!(text.contains("attempt 1"), "{text}");
        assert!(text.contains("exit 1"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");

        let fail_idx = text.find("── Failure").expect("Failure section");
        let tail_idx = text.find("err line 9").expect("bounded tail");
        let cmd_idx = text.find("── Command").expect("Command section");
        let cmd_line_idx = text.find("cargo test --lib").expect("command line");
        let cwd_idx = text.find("/private/tmp/repo").expect("cwd");
        assert!(fail_idx < tail_idx, "{text}");
        assert!(tail_idx < cmd_idx, "{text}");
        assert!(cmd_idx < cmd_line_idx && cmd_line_idx < cwd_idx, "{text}");
        assert!(!text[fail_idx..cmd_idx].contains("err line 0"), "{text}");
    }

    #[test]
    fn test_repeated_attempt_shows_own_number_and_timing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        if let Some(node) = run.flow.nodes.iter_mut().find(|n| n.id == "work") {
            node.timeout_ms = Some(10_000);
        }
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Failed,
            started_ms: 1_000,
            finished_ms: 3_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: Some("first try failed".to_string()),
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.steps.push(Step {
            attempt_id: "work#2".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 4_000,
            finished_ms: 7_000,
            prompt_text: None,
            raw_text: None,
            output: json!({"summary": "fixed on retry"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let first = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(first.contains("attempt 1"), "{first}");
        assert!(
            first.contains(&format!("{}/{}", fmt_duration(2_000), fmt_duration(10_000))),
            "{first}"
        );

        let second = panel_text(&run, &DetailTarget::Step(1), None);
        assert!(second.contains("attempt 2"), "{second}");
        assert!(
            second.contains(&format!("{}/{}", fmt_duration(3_000), fmt_duration(10_000))),
            "{second}"
        );
    }

    #[test]
    fn test_agent_with_brief_evidence_omits_prompt_wrappers_and_transcript_in_first_view() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let lines = [
            json!({
                "seq": 1,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tc1",
                            "title": "Edit",
                            "kind": "edit",
                            "status": "completed",
                            "rawInput": {
                                "path": "crates/app/src/lib.rs",
                                "file_path": "crates/app/src/lib.rs"
                            }
                        }
                    }
                }
            }),
            json!({
                "seq": 2,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"text": "TRANSCRIPT_BODY_SHOULD_NOT_DUMP"}
                        }
                    }
                }
            }),
        ];
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&events_path, body).expect("write events");

        let mut run = make_test_run(tmp.path());
        run.flow.edges = vec![
            FlowEdge::Simple {
                from: "list".to_string(),
                to: "work".to_string(),
            },
            FlowEdge::Switch {
                from: "work".to_string(),
                on: "verdict".to_string(),
                cases: vec![("approve".to_string(), "review".to_string())],
            },
        ];
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path,
            agent_name: Some("claude".to_string()),
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("Update crates/app/src/lib.rs\nLONG_PROMPT_MARKER".to_string()),
            raw_text: Some("{\"verdict\":\"approve\"}".to_string()),
            output: json!({"verdict": "approve", "reason": "All checks passed"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 1,
                event_end_seq: 2,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let text = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(text.contains("── Digest"), "{text}");
        assert!(
            text.contains("verdict: approve -> review (All checks passed)"),
            "{text}"
        );
        assert!(text.contains("files "), "{text}");
        assert!(text.contains("crates/app/src/lib.rs"), "{text}");
        assert!(text.contains("Enter inspect · r replay"), "{text}");
        assert!(!text.contains("LONG_PROMPT_MARKER"), "{text}");
        assert!(!text.contains("TRANSCRIPT_BODY_SHOULD_NOT_DUMP"), "{text}");
        assert!(!text.contains("\"verdict\":"), "{text}");
        assert!(!text.contains("── Prompt"), "{text}");
        assert!(!text.contains("── Conversation"), "{text}");
    }

    #[test]
    fn test_agent_without_evidence_does_not_invent_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let text = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(text.contains("attempt 1"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");
        assert!(!text.contains("── Digest"), "{text}");
        assert!(!text.contains("verdict:"), "{text}");
        assert!(!text.contains("validation"), "{text}");
        assert!(!text.contains("files "), "{text}");
        assert!(!text.contains("exit "), "{text}");
        let header = text.lines().next().unwrap_or("");
        assert!(!header.contains('/'), "{header}");
    }

    #[test]
    fn test_pending_gate_keeps_question_and_choices_conspicuous() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());

        let text = panel_text(&run, &DetailTarget::Step(1), None);
        assert!(text.contains("◐ review"), "{text}");
        let question_idx = text
            .find("land it, or send it back?")
            .expect("gate question");
        let gate_idx = text.find("── Gate").expect("Gate section");
        let choice_idx = text
            .find("[1] abort → rollback.flow.ts")
            .expect("first choice");
        assert!(question_idx < gate_idx && gate_idx < choice_idx, "{text}");
        assert!(text.contains("[2] ship → deploy.flow.ts"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");
    }

    #[test]
    fn test_running_node_tail_follows_and_stops_on_scroll_up() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let mut lines = Vec::new();
        for seq in 1..=15 {
            lines.push(
                serde_json::to_string(&json!({
                    "seq": seq,
                    "direction": "inbound",
                    "message": {
                        "method": "session/update",
                        "params": {
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {"text": format!("msg line {seq}\n")}
                            }
                        }
                    }
                }))
                .expect("json"),
            );
        }
        std::fs::write(&events_path, lines.join("\n")).expect("write");

        let mut run = make_test_run(tmp.path());
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path: events_path.clone(),
            agent_name: Some("claude".to_string()),
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        let target = DetailTarget::Node("work".to_string());
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 6),
            &run,
            &target,
            &Theme::default(),
            5_000,
        );
        let initial_scroll = panel.scroll;
        assert!(initial_scroll > 0);
        assert!(buffer_lines(&buf).join("\n").contains("msg line 15"));

        // Append more events; tail_follow keeps us at the bottom
        for seq in 16..=20 {
            lines.push(
                serde_json::to_string(&json!({
                    "seq": seq,
                    "direction": "inbound",
                    "message": {
                        "method": "session/update",
                        "params": {
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {"text": format!("msg line {seq}\n")}
                            }
                        }
                    }
                }))
                .expect("json"),
            );
        }
        std::fs::write(&events_path, lines.join("\n")).expect("write");
        buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 6),
            &run,
            &target,
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll > initial_scroll);
        assert!(buffer_lines(&buf).join("\n").contains("msg line 20"));

        // Scroll up disables tail-following
        assert!(panel.scroll(-3));
        let fixed_scroll = panel.scroll;
        lines.push(
            serde_json::to_string(&json!({
                "seq": 21,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"text": "msg line 21\n"}
                        }
                    }
                }
            }))
            .expect("json"),
        );
        std::fs::write(&events_path, lines.join("\n")).expect("write");
        buf = Buffer::empty(Rect::new(0, 0, 60, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 60, 6),
            &run,
            &target,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, fixed_scroll);
    }

    #[test]
    fn test_last_result_object_and_array_renders_clean_without_json_punctuation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_100,
            prompt_text: None,
            raw_text: None,
            output: json!({
                "summary": "all good",
                "files": ["a.rs", "b.rs"]
            }),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 35));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 90, 35),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        let rows = buffer_lines(&buf);
        let stage_row = rows
            .iter()
            .find(|r| r.contains("✓ list"))
            .expect("list stage row");
        assert!(stage_row.contains("summary all good"));
        assert!(!stage_row.contains('{'));
        assert!(!stage_row.contains('"'));

        let lr_idx = rows
            .iter()
            .position(|r| r.starts_with("── Last result"))
            .expect("Last result rule");
        let inp_idx = rows
            .iter()
            .position(|r| r.starts_with("── Input"))
            .expect("Input rule");
        let section = &rows[lr_idx + 1..inp_idx];
        assert!(section.iter().any(|r| r.contains("summary") && r.contains("all good")));
        assert!(section.iter().any(|r| r.contains("a.rs")));
        assert!(section.iter().any(|r| r.contains("b.rs")));
        for line in section {
            assert!(!line.contains('{'));
            assert!(!line.contains('}'));
            assert!(!line.contains('['));
            assert!(!line.contains("\"summary\":"));
        }
    }

    #[test]
    fn test_string_output_wraps_and_empty_or_null_hides_section() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.input = json!({});
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!("plain wrapped text"),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let lines = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let joined = text.join("\n");
        assert!(joined.contains("── Last result"));
        assert!(joined.contains("plain wrapped text"));
        assert!(!joined.contains("── Input"));

        run.steps[0].output = Value::Null;
        let lines_null = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let joined_null: String = lines_null
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined_null.contains("── Last result"));

        run.steps[0].output = json!({});
        let lines_empty = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let joined_empty: String = lines_empty
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined_empty.contains("── Last result"));
    }

    #[test]
    fn test_array_output_of_40_items_renders_all_without_cap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        let items: Vec<String> = (0..40).map(|i| format!("item-{i}")).collect();
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!(items),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let lines = build_run_lines(&run, None, 60, &Theme::default(), 5_000);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        for i in 0..40 {
            assert!(text.iter().any(|r| r.contains(&format!("item-{i}"))));
        }
        assert!(!text.iter().any(|r| r.contains("more rows")));
    }

    #[test]
    fn test_small_and_zero_rects_do_not_panic_and_stay_inside_area() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = make_test_run(tmp.path());
        let mut panel = DetailPanel::new();

        let mut buf0 = Buffer::empty(Rect::new(0, 0, 0, 0));
        panel.render(
            &mut buf0,
            Rect::new(0, 0, 0, 0),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );

        let mut buf_narrow = Buffer::empty(Rect::new(0, 0, 8, 3));
        panel.render(
            &mut buf_narrow,
            Rect::new(0, 0, 8, 3),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        for cell in &buf_narrow.content {
            assert_eq!(cell.symbol(), " ");
        }

        // Sub-rect bounds check
        let outer = Rect::new(0, 0, 60, 20);
        let sub = Rect::new(5, 3, 40, 10);
        let mut big = Buffer::empty(outer);
        panel.render(
            &mut big,
            sub,
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        for y in 0..outer.height {
            for x in 0..outer.width {
                let inside =
                    x >= sub.x && x < sub.x + sub.width && y >= sub.y && y < sub.y + sub.height;
                if !inside {
                    assert_eq!(
                        big[(x, y)].symbol(),
                        " ",
                        "cell ({x},{y}) modified outside area"
                    );
                }
            }
        }
    }
}
