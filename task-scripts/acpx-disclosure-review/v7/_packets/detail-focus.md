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

=== REPOSITORY CONTEXT (26 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1-260 of 3315 (repository grok-build)
````
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

pub struct DetailPanel {
    cache_key: Option<CacheKey>,
    /// The picked choice of an answered gate; `Some("")` when it was answered
    /// but the choice is unknown. `None` while the gate is open.
    answered: Option<String>,
    last_target: Option<(PathBuf, DetailTarget)>,
    lines: Vec<Line<'static>>,
    scroll: usize,
    tail_follow: bool,
    last_height: usize,
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
            lines: Vec::new(),
            scroll: 0,
            tail_follow: false,
            last_height: 0,
        }
    }

    /// Tell the panel whether the run's gate has been answered, and with what.
    pub fn set_answered(&mut self, answered: Option<&str>) {
        if self.answered.as_deref() != answered {
            self.answered = answered.map(str::to_string);
        }
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
        if self.tail_follow {
            self.scroll = max_scroll;
        } else if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let height = area.height as usize;
        let total = self.lines.len();
        let needs_scrollbar = total > height;
        let text_width = area.width;

        for row in 0..height {
            let line_idx = self.scroll + row;
            if let Some(line) = self.lines.get(line_idx) {
                buf.set_line(area.x, area.y + row as u16, line, text_width);
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
) -> Vec<Line<'static>> {
    if width < 10 {
        return Vec::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines_at(run, *i, step, answered, width, theme, now_ms)
            } else {
                vec![dim_line("step not found", theme)]
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
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let status = run.summary.status;
    // A waiting run whose gate is answered reads as done, like the run header.
    let shown = if status == RunStatus::Waiting && answered.is_some() {
        RunStatus::Completed
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 415-445 of 3315 (repository grok-build)
````
            }
            out.push(truncate_line(Line::from(spans), width));
        }
    }

    // Last result
    if let Some(last_step) = run.steps.last() {
        let lines = format_value_lines(&last_step.output, width, theme);
        if !lines.is_empty() {
            push_section_rule(&mut out, "Last result", width, theme);
            out.extend(lines);
        }
    }

    // Input
    let input_lines = format_value_lines(&run.input, width, theme);
    if !input_lines.is_empty() {
        push_section_rule(&mut out, "Input", width, theme);
        out.extend(
            input_lines,
        );
    }

    out
}

fn step_one_line_summary(step: &Step) -> String {
    if let Some(err) = &step.error {
        if !err.is_empty() {
            return first_non_empty_line(err);
        }
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 525-740 of 3315 (repository grok-build)
````
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
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
    out: &mut Vec<Line<'static>>,
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
            for line in wrap(s, body_w) {
                if indent == 0 {
                    out.push(Line::from(Span::styled(line, val_style)));
                } else {
                    out.push(Line::from(vec![
                        Span::raw(pad_indent.clone()),
                        Span::styled(line, val_style),
                    ]));
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                let bullet_prefix = format!("{pad_indent}- ");
                let item_w = width.saturating_sub(indent + 2).max(1);
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        let mut sub = Vec::new();
                        append_value_lines(&mut sub, item, 0, item_w, theme);
                        if sub.is_empty() {
                            sub.push(Line::from(Span::styled(
                                EMPTY,
                                Style::default().fg(theme.gray),
                            )));
                        }
                        for (idx, sub_line) in sub.into_iter().enumerate() {
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
) -> Vec<Line<'static>> {
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
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    let is_last_of_node = run
        .steps
        .iter()
        .rposition(|s| s.node_id == step.node_id)
        == Some(step_idx);
    let is_gate = run.summary.status == RunStatus::Waiting
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && is_last_of_node;
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 975-995 of 3315 (repository grok-build)
````
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
            out.extend(result_lines);
        }
    }

    // stdout / stderr for actions
    if let Some(art) = &step.stdout {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stdout", &run.summary.dir, art, width, theme);
        }
    }
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1220-1275 of 3315 (repository grok-build)
````
            | "answer"
            | "ok"
            | "status"
    )
}

fn append_artifact_section(
    out: &mut Vec<Line<'static>>,
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
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let state = run.node_state(node_id);
    let shown = run.node_state_answered(node_id, answered.is_some());
    let glyph = shown.glyph();
    let color = node_state_color(shown, theme);

    out.push(title_row_right(glyph, color, node_id, "", width, theme));

    let settings = bundle::node_settings(&run.summary.dir);
    let agent_lbl = settings
        .get(node_id)
        .map(AgentSettings::label)
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1365-1380 of 3315 (repository grok-build)
````

    out
}

fn append_gate_section(
    out: &mut Vec<Line<'static>>,
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1436-1500 of 3315 (repository grok-build)
````
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
    out: &mut Vec<Line<'static>>,
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1725-1835 of 3315 (repository grok-build)
````
    }
    truncate_line(Line::from(spans), width)
}

fn push_section_rule(
    out: &mut Vec<Line<'static>>,
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
    out.push(Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(
            mid,
            Style::default()
                .fg(theme.gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(trail, Style::default().fg(theme.gray_dim)),
    ]));
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
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    style: Style,
) {
    for line in wrap(text, width) {
        out.push(Line::from(Span::styled(line, style)));
    }
}

fn push_wrapped_capped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    max_rows: usize,
    style: Style,
    theme: &Theme,
) {
    let wrapped = wrap(text, width);
    let total = wrapped.len();
    for line in wrapped.into_iter().take(max_rows) {
        out.push(Line::from(Span::styled(line, style)));
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1975-2058 of 3315 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 2477-2540 of 3315 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 2622-2713 of 3315 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1-40 of 1628 (repository grok-build)
````
pub(crate) mod detail;
pub(crate) mod inspector;
pub(crate) mod ledger;
pub(crate) mod modal_view;
pub(crate) mod run;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{self, Run, RunSummary};
use crate::acpx::sessions::{self, SessionSummary};
use crate::piw::gate::{self as piw_gate, GateAnswer as PiwGateAnswer, GateAnswerer};
use crate::piw::mirror;
use crate::piw::runtime;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{
    self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowOutcome, ModalWindowState,
    Shortcut,
};

use self::detail::{DetailPanel, DetailTarget};
use self::inspector::{InspectorOutcome, InspectorState};
use self::ledger::{GateAnswer, LedgerOutcome, LedgerState, RowExtra};
use self::run::{RunOutcome, RunViewState};

const MAX_TITLE_WIDTH: usize = 72;
/// Below this content width the runs list has no room for a preview beside it.
const PREVIEW_MIN_WIDTH: u16 = 120;
/// Rails and agents are loaded for this many of the newest runs.
const EXTRAS_LIMIT: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 78-160 of 1628 (repository grok-build)
````
pub struct AcpxModalState {
    pub window: ModalWindowState,
    runs_base: Option<PathBuf>,
    sessions_base: Option<PathBuf>,
    runs_snapshot: Vec<RunSummary>,
    ledger: LedgerState,
    run_view: Option<RunViewState>,
    inspector: Option<(usize, InspectorState)>,
    mode: Mode,
    help_open: bool,
    notice: Option<ModalNotice>,
    /// The right-hand panel: the selected run beside the list, the selected stage beside a run.
    detail: DetailPanel,
    /// The run selected in the list, loaded for the preview.
    preview: Option<Run>,
    /// Rail and agents per run id, with the `updated_ms` they were read at.
    extras: HashMap<String, (i64, RowExtra)>,
    /// Gate answers per run id for runs beyond `EXTRAS_LIMIT`.
    answers: HashMap<String, Option<GateAnswer>>,
    /// Answers being sent to pi-workflows gates, polled on the one-second tick.
    answerers: Vec<GateAnswerer>,
    /// The mirrored run whose folder the run view is open on, so closing the
    /// view can release it.
    open_mirror_run: Option<String>,
    /// Where the detail panel was last drawn, for the mouse wheel.
    detail_rect: Option<Rect>,
}

impl AcpxModalState {
    pub fn open() -> Self {
        Self::open_at(bundle::runs_base(), sessions::sessions_base())
    }

    pub fn open_at(runs_base: Option<PathBuf>, sessions_base: Option<PathBuf>) -> Self {
        let mut state = Self {
            window: ModalWindowState::new(),
            runs_base,
            sessions_base,
            runs_snapshot: Vec::new(),
            ledger: LedgerState::new(),
            run_view: None,
            inspector: None,
            mode: Mode::Ledger,
            help_open: false,
            notice: None,
            detail: DetailPanel::new(),
            preview: None,
            extras: HashMap::new(),
            answers: HashMap::new(),
            answerers: Vec::new(),
            open_mirror_run: None,
            detail_rect: None,
        };
        state.rescan_runs();
        state.rescan_sessions();
        state
    }

    pub fn open_run(&mut self, dir: &Path) -> bool {
        self.release_open_mirror_run();
        match bundle::load_run(dir) {
            Ok(run) => {
                let run_id = run.summary.run_id.clone();
                self.watch_open_mirror_run(dir, &run_id);
                let mut view = RunViewState::new(run);
                view.set_answered(self.ledger.answered_choice(&run_id).map(str::to_string));
                view.set_settings(bundle::node_settings(dir), bundle::step_settings(dir));
                self.run_view = Some(view);
                self.inspector = None;
                self.mode = Mode::Run;
                self.help_open = false;
                self.notice = None;
                true
            }
            Err(err) => {
                self.set_notice(format!("cannot open run: {err}"));
                false
            }
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 199-262 of 1628 (repository grok-build)
````
    pub fn wants_esc(&self) -> bool {
        self.help_open || self.mode != Mode::Ledger || !self.ledger.esc_closes()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|notice| notice.text.as_str())
    }

    /// One line shown at the bottom until the next key.
    pub fn set_notice(&mut self, text: String) {
        self.notice = Some(ModalNotice::warning(text));
    }

    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
        let confirmed = matches!(delivery, crate::clipboard::CopyDelivery::Clipboard { result, .. }
            if result.delivery.is_confirmed());
        self.notice = Some(ModalNotice {
            text: delivery.toast_message().into_owned(),
            warning: !confirmed,
        });
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let had_notice = self.notice.take().is_some();

        if self.help_open {
            self.help_open = false;
            return AcpxOutcome::Changed;
        }

        let captures = match self.mode {
            Mode::Ledger => self.ledger.captures_text(),
            Mode::Run => self.run_view.as_ref().is_some_and(|rv| rv.captures_keys()),
            Mode::Step => false,
        };
        if !captures && matches!(key.code, KeyCode::Char('?')) {
            self.help_open = true;
            return AcpxOutcome::Changed;
        }
        // Beside the list, J / K scroll the preview (the list itself owns j / k).
        if !captures && self.mode == Mode::Ledger {
            let delta = match key.code {
                KeyCode::Char('J') => 3,
                KeyCode::Char('K') => -3,
                _ => 0,
            };
            if delta != 0 {
                return if self.detail.scroll(delta) { AcpxOutcome::Changed } else { AcpxOutcome::Unchanged };
            }
        }

        let outcome = match self.mode {
            Mode::Ledger => self.handle_ledger_key(key),
            Mode::Run => self.handle_run_key(key),
            Mode::Step => self.handle_step_key(key),
        };

        if outcome == AcpxOutcome::Unchanged && had_notice {
            AcpxOutcome::Changed
        } else {
            outcome
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 377-396 of 1628 (repository grok-build)
````
    fn handle_ledger_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        match self.ledger.handle_key(key) {
            LedgerOutcome::Unchanged => AcpxOutcome::Unchanged,
            LedgerOutcome::Changed => AcpxOutcome::Changed,
            LedgerOutcome::OpenRun(dir) => {
                self.open_run(&dir);
                AcpxOutcome::Changed
            }
            LedgerOutcome::OpenSession(s) => AcpxOutcome::Replay(session_replay(s)),
            LedgerOutcome::Copy(text) => AcpxOutcome::Copy(text),
            LedgerOutcome::Rescan => {
                self.rescan_runs();
                self.rescan_sessions();
                AcpxOutcome::Changed
            }
            LedgerOutcome::Close => AcpxOutcome::Close,
        }
    }

    /// A mirrored run was opened: keep mirroring it in full.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 504-558 of 1628 (repository grok-build)
````
    fn handle_run_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some(rv) = self.run_view.as_mut() else {
            self.mode = Mode::Ledger;
            return AcpxOutcome::Changed;
        };
        match rv.handle_key(key) {
            RunOutcome::Unchanged => AcpxOutcome::Unchanged,
            RunOutcome::Changed => AcpxOutcome::Changed,
            RunOutcome::Back => {
                self.mode = Mode::Ledger;
                self.run_view = None;
                self.inspector = None;
                self.release_open_mirror_run();
                self.rescan_runs();
                AcpxOutcome::Changed
            }
            RunOutcome::OpenStep(idx) => {
                if self.open_step_at(idx) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Replay(idx) => self.replay_step(idx),
            RunOutcome::Copy(text) => AcpxOutcome::Copy(text),
            RunOutcome::ScrollDetail(delta) => {
                if self.detail.scroll(delta) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Gate { choice, flow_file } => {
                // A pi-workflows gate answers in place instead of starting a
                // flow; the target is checked before any relative-path join.
                if let Some((run_id, choice_key)) = piw_gate::parse_answer_target(&flow_file) {
                    return self.answer_mirror_gate(&run_id, &choice, &choice_key);
                }
                let run = rv.run();
                // A relative flow file is beside the flow that declared it.
                let declared = PathBuf::from(&flow_file);
                let flow_file = match bundle::flow_path(&run.summary.dir).as_deref().and_then(Path::parent) {
                    Some(dir) if declared.is_relative() => dir.join(declared),
                    _ => declared,
                };
                let input_json = bundle::gate_input(run, &choice).to_string();
                // Back to the list, where the new run appears.
                self.set_notice(format!("{choice}: started {}", flow_file.display()));
                self.mode = Mode::Ledger;
                self.run_view = None;
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 651-694 of 1628 (repository grok-build)
````
    fn title_text(&self) -> String {
        // The pop-up lists pi-workflows runs too, not only acpx flows, so it is titled "runs".
        let raw = match self.mode {
            Mode::Ledger => "runs".to_string(),
            Mode::Run => match self.run_view.as_ref() {
                Some(rv) => format!("runs › {}", rv.run().summary.title),
                None => "runs".to_string(),
            },
            Mode::Step => match (self.run_view.as_ref(), self.inspector.as_ref()) {
                (Some(rv), Some((idx, _))) => {
                    let run_title = &rv.run().summary.title;
                    let node_id = rv
                        .run()
                        .steps
                        .get(*idx)
                        .map_or("?", |s| s.node_id.as_str());
                    format!("runs › {run_title} › {node_id}")
                }
                _ => "runs".to_string(),
            },
        };
        truncate_str(&raw, MAX_TITLE_WIDTH)
    }

    fn active_shortcut_labels(&self) -> Vec<&'static str> {
        let mut labels = match self.mode {
            Mode::Ledger => self.ledger.shortcuts(),
            Mode::Run => self
                .run_view
                .as_ref()
                .map_or_else(Vec::new, |rv| rv.shortcuts()),
            Mode::Step => self
                .inspector
                .as_ref()
                .map_or_else(Vec::new, |(_, ins)| ins.shortcuts()),
        };
        if !labels.contains(&"? keys") {
            labels.push("? keys");
        }
        labels
    }
}

/// What the list shows for one run beyond its summary. Reads the run folder.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 757-864 of 1628 (repository grok-build)
````
pub fn render_acpx_modal(
    buf: &mut Buffer,
    full_area: Rect,
    state: &mut AcpxModalState,
    compact: bool,
) {
    if full_area.width == 0 || full_area.height == 0 {
        return;
    }
    let theme = Theme::current();
    let sizing = ModalSizing {
        width_pct: 1.0,
        max_width: u16::MAX,
        min_width: 44,
        v_margin: 0,
        h_pad: 2,
        v_pad: 0,
        footer_lines: 2,
    }
    .with_compact(compact);

    let title = state.title_text();
    let labels = state.active_shortcut_labels();
    let shortcuts: Vec<Shortcut<'_>> = labels
        .iter()
        .map(|label| Shortcut {
            label,
            clickable: false,
            id: 0,
        })
        .collect();

    let config = ModalWindowConfig {
        title: &title,
        tabs: None,
        shortcuts: &shortcuts,
        sizing,
        fold_info: None,
    };

    // Nothing of the screen behind may show around the frame.
    let blank = Style::default().fg(theme.text_primary).bg(theme.bg_base);
    for y in full_area.y..full_area.y + full_area.height {
        buf.set_stringn(full_area.x, y, " ".repeat(full_area.width as usize), full_area.width as usize, blank);
    }
    let Some(ModalContentArea { content, .. }) =
        modal_window::render_modal_window(buf, full_area, &mut state.window, &config, &theme)
    else {
        return;
    };

    if content.height < 4 || content.width < 20 {
        return;
    }

    let now = now_ms();

    state.detail_rect = None;
    if state.help_open {
        render_help_overlay(buf, content, state.mode, &theme);
    } else {
        match state.mode {
            Mode::Ledger => {
                state.sync_preview();
                match (state.preview.as_ref(), content.width >= PREVIEW_MIN_WIDTH) {
                    (Some(run), true) => {
                        // List on the left, the selected run on the right, a rule between.
                        let list_w = (content.width * 9 / 20).clamp(56, 110);
                        let list = Rect::new(content.x, content.y, list_w, content.height);
                        let right = Rect::new(content.x + list_w + 2, content.y, content.width - list_w - 2, content.height);
                        state.ledger.render(buf, list, &theme, now);
                        let rule = Style::default().fg(theme.gray_dim);
                        for y in content.y..content.y + content.height {
                            buf.set_stringn(content.x + list_w, y, "│", 1, rule);
                        }
                        state
                            .detail
                            .set_answered(state.ledger.answered_choice(&run.summary.run_id));
                        state.detail.render(buf, right, run, &DetailTarget::Run, &theme, now);
                        state.detail_rect = Some(right);
                    }
                    _ => state.ledger.render(buf, content, &theme, now),
                }
            }
            Mode::Run => {
                if let Some(rv) = state.run_view.as_mut() {
                    rv.render(buf, content, &theme, now);
                    if let Some(area) = rv.detail_area() {
                        let area = area.intersection(content);
                        state.detail.set_answered(rv.answered());
                        state.detail.render(buf, area, rv.run(), &rv.detail_target(), &theme, now);
                        state.detail_rect = Some(area);
                    }
                }
            }
            Mode::Step => {
                if let Some((_, ins)) = state.inspector.as_mut() {
                    ins.render(buf, content, &theme, now);
                }
            }
        }
    }

    if let Some(notice) = state.notice.as_ref() {
        render_notice_row(buf, content, notice, &theme);
    }
}

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 902-917 of 1628 (repository grok-build)
````
const RUN_KEYS: &[(&str, &str)] = &[
    ("j / k / ↑ / ↓", "Move through stages and their substeps"),
    ("[ / ]", "Previous / next stage"),
    ("J / K, D / U", "Scroll the detail panel by a few rows / by a page"),
    ("Enter", "Inspect the selected stage; on a gate row, answer it"),
    ("1-9", "Answer the gate with that choice (asks first)"),
    ("F5", "Jump to the gate"),
    ("Space, h / l, E", "Fold or unfold a stage; E folds all"),
    ("c / !", "Jump to the current stage / the next failed one"),
    ("v / n", "Show or hide the graph / heartbeat noise"),
    ("r", "Replay the stage's conversation as a transcript"),
    ("y", "Copy the stage's output"),
    ("?", "Toggle this key reference"),
    ("Esc", "Back to the list"),
];

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1005-1030 of 1628 (repository grok-build)
````
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use tempfile::TempDir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    struct TestFixture {
        _tmp: TempDir,
        runs_base: PathBuf,
        sessions_base: PathBuf,
        run_dir: PathBuf,
        events_path: PathBuf,
        session_stream_path: PathBuf,
    }

    fn write_fixture(status: &str) -> TestFixture {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_base = tmp.path().join("runs");
        let sessions_base = tmp.path().join("sessions");
        let run_dir = runs_base.join("run-001");
        let proj_dir = run_dir.join("projections");
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1156-1168 of 1628 (repository grok-build)
````
    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1227-1250 of 1628 (repository grok-build)
````
    fn wants_esc_reflects_mode_and_help_state() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.wants_esc());

        // Help overlay makes wants_esc true even in Ledger
        state.handle_key(&key(KeyCode::Char('?')));
        assert!(state.wants_esc());
        state.handle_key(&key(KeyCode::Char('a')));
        assert!(!state.wants_esc());

        // Mode::Run wants_esc
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);
        assert!(state.wants_esc());

        // Mode::Step wants_esc
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Step);
        assert!(state.wants_esc());
    }

    #[test]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1297-1326 of 1628 (repository grok-build)
````
    fn step_next_bracket_advances_and_esc_preserves_run_selection() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);

        // Start at step 0 and open it
        if let Some(rv) = state.run_view.as_mut() {
            rv.select_step(0);
        }
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Step);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(0));

        // Press `]` to move to step 1
        let out = state.handle_key(&key(KeyCode::Char(']')));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(1));

        // Esc back to Run view lands on step 1
        state.handle_key(&key(KeyCode::Esc));
        assert_eq!(state.mode(), Mode::Run);
        assert_eq!(
            state.run_view.as_ref().and_then(|rv| rv.selected_step()),
            Some(1)
        );
    }

    #[test]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 25-45 of 2758 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 170-215 of 2758 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 520-545 of 2758 (repository grok-build)
````
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
````

=== TASK SPECIFICATION ===

Slice ID: detail-focus

Packet 3b. Ownership: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs (DetailPanel and its row building)
and, in crates/codegen/xai-grok-pager/src/views/acpx_modal.rs, only Run-mode routing: handle_key, handle_run_key,
wants_esc, open_run, active_shortcut_labels, the Mode::Run render branch,
RUN_KEYS, and their tests. Do not edit run.rs (attached read-only), the
ledger, the inspector, the Mode::Ledger preview branch or LEDGER_KEYS.

Goal, from the operator's friction log: in an open run he can Tab into the
detail pane of a failed stage, select the compiler-error line, copy it
exactly, and come back to the same stage.

Key contract, shared with later inspector and Agent Turns packets:
- Tab and Shift+Tab (KeyCode::Tab, KeyCode::BackTab) move focus between the
  stage tree and the detail pane, both directions. Only when the detail pane
  is drawn (RunViewState::detail_area() is Some) and the run view is not
  capturing keys (captures_keys()); otherwise Tab keeps doing nothing.
- With the detail pane focused: j/k and Up/Down move a line cursor by one row
  and keep it visible (scroll follows); PageUp/PageDown move by a page; v starts
  or ends a visual line range anchored at the cursor; y copies the cursor's
  copy unit, or every unit the range touches; Y copies the whole pane. Copies
  return AcpxOutcome::Copy(text), so the existing report_copy notice applies.
- Esc with the pane focused: first ends visual mode, then returns focus to the
  tree. Esc with the tree focused keeps leaving the run, as now.
- With the tree focused every current key keeps its meaning: v graph toggle,
  n noise toggle, y stage-output copy, J/K/D/U scrolling the detail pane, Space, Enter, r.
- With the pane focused, keys the pane does not use (r, [, ], F5, 1-9, c, !,
  n, Enter, ?) go to the run view exactly as before.
- Focus is shown by the cursor row: bg_highlight across the pane width, and
  the range rows in the same style while visual mode is on. Nothing is
  highlighted while the tree has focus. The footer shows the pane's keys
  while it has focus (e.g. "j/k line", "v range", "y copy", "Y all",
  "Esc tree"), and RUN_KEYS gains rows for Tab and the pane keys.
- Changing the selected stage (a new DetailTarget) moves the cursor to the
  first row and ends visual mode. Leaving the run (RunOutcome::Back) and
  open_run reset focus to the tree, so the Ledger preview never shows a
  cursor.

Copy units. The pane stores pre-wrapped rows, and wrap() drops the spaces it
breaks at, so copy text cannot be rebuilt from the rows. Record it where rows
are made: replace the row vector with a small module-private type (for
example `Rows`) that derefs to Vec<Line<'static>> so existing `out.push`,
`out.extend` and `.len()` keep working, and that also records, for rows
produced by wrapping one source line, the exact source line they copy as.
- Every row is in exactly one copy unit. A row pushed directly copies as its
  own visible text with trailing spaces trimmed. Rows from push_wrapped and
  push_wrapped_capped copy as the source line (each line of the text, split
  on newlines) whose wrapping produced them, with no indentation, prefix span
  or break added; a capped unit still copies its whole source line. The
  string-value loop in append_value_lines (the `wrap(s, body_w)` loop) does
  the same, copying the value's line without its indent span. Other direct
  wrap() loops may stay row-by-row.
- A range copies each touched unit once, in order, joined with newlines. Y
  copies every unit. Copies are plain text: no styles, no section-rule
  dashes or scrollbar characters added.
- Change the signatures of the helpers that take `&mut Vec<Line<'static>>`
  and of the builders that create the row vector, including
  format_value_lines, to the new type. Where rows built separately are added
  to the main rows (the `out.extend(...)` of value lines), keep their units,
  e.g. with an append method that offsets them; a plain Vec<Line> extended in
  copies row by row. No function is deleted or renamed; build_lines may
  return the new type.
- Cache: units are rebuilt with the rows under the existing CacheKey; no new
  reads, and nothing per frame beyond drawing.

Tests, in the existing styles: Tab then Shift+Tab move focus and back; Tab
does nothing without a detail area; with the pane focused j/k move the cursor
and v, j, y copy a two-unit range; y on the second row of a wrapped failure
line returns the whole source line exactly, e.g. a 70-character path that wraps
at width 30 copies as the original line with no added space or newline; Y
copies all units; Esc ends visual, then returns focus to the tree, then
leaves the run, landing on the same run in the list; with the tree focused y
still returns the stage output and v still toggles the graph; a new target resets
the cursor; the copied text contains no section-rule characters. Update the
wants_esc test only if its expectations change.

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
=== FILE: task-scripts/acpx-disclosure-returns/detail-focus.edits.md ===
```markdown
<complete Markdown edit list>
```
