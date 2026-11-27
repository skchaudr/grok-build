Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, ledger routing, or release behavior.

=== REPOSITORY CONTEXT (22 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1-243 of 2698 (repository grok-build)
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
    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
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
                build_step_lines(run, *i, step, answered, width, theme)
            } else {
                vec![dim_line("step not found", theme)]
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 436-533 of 2698 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 680-1102 of 2698 (repository grok-build)
````
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
    let mut out = Vec::new();

    let is_last_of_node = run
        .steps
        .iter()
        .rposition(|s| s.node_id == step.node_id)
        == Some(step_idx);
    let is_gate = run.summary.status == RunStatus::Waiting
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && is_last_of_node;

    let (glyph, glyph_color) = if is_gate && answered.is_none() {
        (NodeState::Waiting.glyph(), theme.warning)
    } else if is_gate {
        // The gate has been answered: done, like the graph and the run header.
        let state = NodeState::Done(Outcome::Ok);
        (state.glyph(), node_state_color(state, theme))
    } else {
        let state = NodeState::Done(step.outcome);
        (state.glyph(), node_state_color(state, theme))
    };

    let dur = fmt_duration(step.duration_ms());
    out.push(title_row_right(
        glyph,
        glyph_color,
        &step.node_id,
        &dur,
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

    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        if !lbl.is_empty() {
            facts.push(lbl);
        }
    }
    facts.push(step.kind.label().to_string());
    facts.push(step.attempt_id.clone());
    if let Some(act) = &step.action {
        if let Some(code) = act.exit_code {
            facts.push(format!("exit {code}"));
        }
    }
    let facts_str = truncate_str(&facts.join(" · "), width);
    out.push(Line::from(Span::styled(
        facts_str,
        Style::default().fg(theme.gray),
    )));

    if let Some(err) = &step.error {
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
                let color = if answered.is_some() { theme.gray } else { theme.warning };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    // Prompt
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

    // Command (for actions)
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

    // Conversation
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

    // Result
    let result_lines = format_value_lines(&step.output, width, theme);
    if !result_lines.is_empty() {
        push_section_rule(&mut out, "Result", width, theme);
        out.extend(result_lines);
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1370-1649 of 2698 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1650-2035 of 2698 (repository grok-build)
````

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
        let p_idx = text.find("── Prompt").expect("Prompt section");
        let r_idx = text.find("── Result").expect("Result section");
        let e_idx = text.find("── Events").expect("Events section");
        assert!(p_idx < r_idx && r_idx < e_idx);
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 2104-2377 of 2698 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 2548-2639 of 2698 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 1-219 of 1366 (repository grok-build)
````
use std::path::Path;

use chrono::{TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::acpx::bundle::{
    self, ArtifactRef, FlowNode, NodeKind, NodeState, Outcome, Step, fmt_duration, value_text,
};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;

const ARTIFACT_MAX_BYTES: usize = 256 * 1024;
const HORIZ_SCROLL_STEP: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum InspectorOutcome {
    Unchanged,
    Changed,
    Back,
    Prev,
    Next,
    Replay,
    Copy(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabKind {
    Output,
    Prompt,
    Response,
    Stdout,
    Stderr,
    Error,
    Meta,
}

impl TabKind {
    fn label(self) -> &'static str {
        match self {
            Self::Output => "Output",
            Self::Prompt => "Prompt",
            Self::Response => "Response",
            Self::Stdout => "Stdout",
            Self::Stderr => "Stderr",
            Self::Error => "Error",
            Self::Meta => "Meta",
        }
    }
}

#[derive(Debug, Clone)]
struct TabEntry {
    kind: TabKind,
    text: String,
    is_empty_artifact: bool,
    scroll_y: usize,
    scroll_x: usize,
    /// What `y` copies when it differs from what the tab shows (a JSON value shown as rows copies its raw JSON).
    copy: Option<String>,
}

pub struct InspectorState {
    step: Step,
    node: Option<FlowNode>,
    position: (usize, usize),
    tabs: Vec<TabEntry>,
    active_tab: usize,
    wrap: bool,
    last_body_height: usize,
    last_body_width: usize,
}

impl InspectorState {
    /// `position` is (1-based index of this step, total steps) for the header.
    pub fn new(
        run_dir: &Path,
        step: Step,
        node: Option<FlowNode>,
        position: (usize, usize),
    ) -> Self {
        let mut tabs = Vec::new();

        // 1. Output
        let output_val_text = value_text(&step.output);
        if !output_val_text.is_empty() {
            // A JSON value reads as rows, not braces and quotes; `y` still copies the raw JSON.
            let (text, copy) = if step.output.is_string() {
                (output_val_text, None)
            } else {
                (
                    crate::views::acpx_modal::detail::value_plain_text(&step.output),
                    Some(output_val_text),
                )
            };
            tabs.push(TabEntry {
                kind: TabKind::Output,
                text,
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy,
            });
        } else if let Some(art) = &step.output_artifact {
            let (text, is_empty) = read_artifact_tab(run_dir, art);
            tabs.push(TabEntry {
                kind: TabKind::Output,
                text,
                is_empty_artifact: is_empty,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 2. Prompt
        if let Some(prompt) = &step.prompt_text
            && !prompt.is_empty()
        {
            tabs.push(TabEntry {
                kind: TabKind::Prompt,
                text: prompt.clone(),
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 3. Response
        if let Some(raw) = &step.raw_text
            && !raw.is_empty()
        {
            tabs.push(TabEntry {
                kind: TabKind::Response,
                text: raw.clone(),
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 4. Stdout
        if let Some(art) = &step.stdout {
            let (text, is_empty) = read_artifact_tab(run_dir, art);
            tabs.push(TabEntry {
                kind: TabKind::Stdout,
                text,
                is_empty_artifact: is_empty,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 5. Stderr
        if let Some(art) = &step.stderr {
            let (text, is_empty) = read_artifact_tab(run_dir, art);
            tabs.push(TabEntry {
                kind: TabKind::Stderr,
                text,
                is_empty_artifact: is_empty,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 6. Error
        if let Some(err) = &step.error
            && !err.is_empty()
        {
            tabs.push(TabEntry {
                kind: TabKind::Error,
                text: err.clone(),
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 7. Meta
        let meta_text = build_meta_text(&step, node.as_ref());
        tabs.push(TabEntry {
            kind: TabKind::Meta,
            text: meta_text,
            is_empty_artifact: false,
            scroll_y: 0,
            scroll_x: 0,
            copy: None,
        });

        let is_failed = matches!(step.outcome, Outcome::Failed | Outcome::TimedOut);
        let preferred = if is_failed {
            TabKind::Error
        } else {
            TabKind::Output
        };

        let active_tab = tabs
            .iter()
            .position(|t| t.kind == preferred)
            .or_else(|| tabs.iter().position(|t| t.kind == TabKind::Output))
            .unwrap_or(0);

        Self {
            step,
            node,
            position,
            tabs,
            active_tab,
            wrap: true,
            last_body_height: 20,
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 320-358 of 1366 (repository grok-build)
````
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Left | KeyCode::Char('h') if !ctrl => {
                if self.wrap {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_x > 0
                {
                    tab.scroll_x = tab.scroll_x.saturating_sub(HORIZ_SCROLL_STEP);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Right | KeyCode::Char('l') if !ctrl => {
                if self.wrap {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.scroll_x = tab.scroll_x.saturating_add(HORIZ_SCROLL_STEP);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('w') if !ctrl => {
                self.wrap = !self.wrap;
                let max_scroll = self.max_scroll_y();
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y > max_scroll
                {
                    tab.scroll_y = max_scroll;
                }
                InspectorOutcome::Changed
            }
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 160-215 of 2758 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs` lines 552-611 of 2758 (repository grok-build)
````
                    RunOutcome::Unchanged
                }
            }
            KeyCode::Char(ch @ '1'..='9') => {
                if self.run.summary.status != RunStatus::Waiting || self.answered.is_some() {
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
                    if self.answered.is_some() {
                        return RunOutcome::Unchanged;
                    }
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 60-130 of 1571 (repository grok-build)
````
    Close,
    Copy(String),
    Replay(ReplayRequest),
    /// A gate was answered: start this flow with this input.
    RunFlow { flow_file: PathBuf, input_json: String },
}

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
    notice: Option<String>,
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 186-290 of 1571 (repository grok-build)
````
    }

    pub fn wants_esc(&self) -> bool {
        self.help_open || self.mode != Mode::Ledger || !self.ledger.esc_closes()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// One line shown at the bottom until the next key.
    pub fn set_notice(&mut self, text: String) {
        self.notice = Some(text);
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

    pub fn handle_mouse(&mut self, mouse: &MouseEvent) -> AcpxOutcome {
        match modal_window::handle_modal_mouse(
            &mut self.window,
            mouse.kind,
            mouse.column,
            mouse.row,
        ) {
            ModalWindowOutcome::CloseRequested => AcpxOutcome::Close,
            ModalWindowOutcome::Handled
            | ModalWindowOutcome::TabChanged(_)
            | ModalWindowOutcome::ShortcutActivated(_)
            | ModalWindowOutcome::CollapseGroup
            | ModalWindowOutcome::ExpandGroup
            | ModalWindowOutcome::CollapseDetails
            | ModalWindowOutcome::ExpandDetails
            | ModalWindowOutcome::JumpToParent(_) => AcpxOutcome::Changed,
            ModalWindowOutcome::Unhandled => {
                if self.help_open {
                    return AcpxOutcome::Unchanged;
                }
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => -3,
                    MouseEventKind::ScrollDown => 3,
                    _ => return AcpxOutcome::Unchanged,
                };
                let over_detail = self.detail_rect.is_some_and(|r| {
                    mouse.column >= r.x && mouse.column < r.x + r.width && mouse.row >= r.y && mouse.row < r.y + r.height
                });
                if over_detail && self.mode != Mode::Step {
                    return if self.detail.scroll(delta) { AcpxOutcome::Changed } else { AcpxOutcome::Unchanged };
                }
                let moved = match self.mode {
                    Mode::Ledger => self.ledger.scroll(delta),
                    Mode::Run => self.run_view.as_mut().is_some_and(|rv| rv.scroll(delta)),
                    Mode::Step => self
                        .inspector
                        .as_mut()
                        .is_some_and(|(_, ins)| ins.scroll(delta)),
                };
                if moved {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 484-541 of 1571 (repository grok-build)
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
                self.notice = Some(format!("{choice}: started {}", flow_file.display()));
                self.mode = Mode::Ledger;
                self.run_view = None;
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
        }
    }

    fn handle_step_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some((step_idx, ins)) = self.inspector.as_mut() else {
            self.mode = Mode::Run;
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 775-900 of 1571 (repository grok-build)
````
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

    if let Some(notice) = state.notice.as_deref() {
        render_notice_row(buf, content, notice, &theme);
    }
}

fn render_notice_row(buf: &mut Buffer, content: Rect, notice: &str, theme: &Theme) {
    if content.height == 0 || content.width == 0 {
        return;
    }
    let y = content.y + content.height - 1;
    let row_rect = Rect::new(content.x, y, content.width, 1);
    let warn_style = Style::default()
        .fg(theme.warning)
        .bg(theme.bg_base)
        .add_modifier(Modifier::BOLD);
    buf.set_style(row_rect, warn_style);
    let text = truncate_str(&format!("! {notice}"), content.width as usize);
    let line = Line::from(Span::styled(text, warn_style));
    buf.set_line(content.x, y, &line, content.width);
}

const LEDGER_KEYS: &[(&str, &str)] = &[
    ("Tab / h / l", "Switch between Flows and Sessions"),
    ("j / k / ↑ / ↓", "Move selection"),
    ("g / G", "Jump to first / last row"),
    ("J / K", "Scroll the preview of the selected run"),
    ("Enter", "Open selected flow run or replay session"),
    ("/", "Filter rows by text"),
    ("a", "Only running and waiting runs"),
    ("s", "Cycle order: grouped, recent, name"),
    ("y", "Copy selected id or path"),
    ("R", "Rescan flows and sessions on disk"),
    ("?", "Toggle this key reference"),
    ("Esc", "Clear filter or close modal"),
];

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

const STEP_KEYS: &[(&str, &str)] = &[
    ("Tab / 1-5", "Switch inspector tab (Output / Prompt / Stdout / Stderr / Events)"),
    ("[ / ]", "Previous / next step in run"),
    ("j / k / ↑ / ↓", "Scroll active pane"),
    ("PgUp / PgDn", "Page up / down"),
    ("r", "Replay ACP conversation for this step"),
    ("y", "Copy current tab content"),
    ("?", "Toggle this key reference"),
    ("Esc / Backspace", "Back to run view"),
];

fn render_help_overlay(buf: &mut Buffer, area: Rect, mode: Mode, theme: &Theme) {
    let base_style = Style::default().fg(theme.text_primary).bg(theme.bg_base);
    buf.set_style(area, base_style);

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 973-1194 of 1571 (repository grok-build)
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
        fs::create_dir_all(&proj_dir).expect("create projections dir");
        fs::create_dir_all(&sessions_base).expect("create sessions dir");

        let events_path = run_dir.join("sess-1.events.ndjson");
        fs::write(&events_path, "{}\n").expect("write events");

        let live_json = format!(
            r#"{{
                "runId": "run-001",
                "flowName": "deploy-flow",
                "runTitle": "Deploy Production",
                "status": "{status}",
                "startedAt": "2025-03-01T10:00:00Z",
                "updatedAt": "2025-03-01T10:01:00Z",
                "currentNode": "verify"
            }}"#
        );
        fs::write(proj_dir.join("live.json"), live_json).expect("write live.json");

        let flow_json = r#"{
            "name": "deploy-flow",
            "startAt": "plan",
            "nodes": {
                "plan": { "nodeType": "acp", "summary": "Draft deployment plan" },
                "verify": { "nodeType": "action", "summary": "Run verification suite", "actionExecution": "shell" }
            },
            "edges": [
                { "from": "plan", "to": "verify" }
            ]
        }"#;
        fs::write(run_dir.join("flow.json"), flow_json).expect("write flow.json");

        let steps_json = r#"[
            {
                "attemptId": "plan#1",
                "nodeId": "plan",
                "nodeType": "acp",
                "outcome": "ok",
                "startedAt": "2025-03-01T10:00:00Z",
                "finishedAt": "2025-03-01T10:00:30Z",
                "promptText": "Create plan",
                "output": "Plan ready",
                "agent": { "agentName": "claude" },
                "trace": {
                    "conversation": {
                        "sessionId": "sess-1",
                        "eventStartSeq": 1,
                        "eventEndSeq": 12
                    }
                }
            },
            {
                "attemptId": "verify#1",
                "nodeId": "verify",
                "nodeType": "action",
                "outcome": "ok",
                "startedAt": "2025-03-01T10:00:31Z",
                "finishedAt": "2025-03-01T10:00:55Z",
                "output": { "passed": true },
                "trace": {
                    "action": {
                        "actionType": "shell",
                        "command": "cargo",
                        "args": ["test"],
                        "exitCode": 0,
                        "durationMs": 2400
                    }
                }
            }
        ]"#;
        fs::write(proj_dir.join("steps.json"), steps_json).expect("write steps.json");

        let trace_ndjson = r#"{"seq":1,"at":"2025-03-01T10:00:00Z","scope":"node","type":"node_started","nodeId":"plan","attemptId":"plan#1"}
{"seq":2,"at":"2025-03-01T10:00:31Z","scope":"node","type":"node_started","nodeId":"verify","attemptId":"verify#1"}
"#;
        fs::write(run_dir.join("trace.ndjson"), trace_ndjson).expect("write trace.ndjson");

        let manifest_json = format!(
            r#"{{
                "runId": "run-001",
                "flowName": "deploy-flow",
                "startedAt": "2025-03-01T10:00:00Z",
                "sessions": [
                    {{
                        "id": "sess-1",
                        "handle": "main",
                        "eventsPath": "{}",
                        "agentName": "claude"
                    }}
                ]
            }}"#,
            events_path.display()
        );
        fs::write(run_dir.join("manifest.json"), manifest_json).expect("write manifest.json");

        let session_stream_path = sessions_base.join("rec-100.stream.ndjson");
        fs::write(&session_stream_path, "{}\n").expect("write session stream");
        let session_json = r#"{
            "acpx_record_id": "rec-100",
            "acp_session_id": "acp-sess-100",
            "name": "triage",
            "title": "Bug Triage Session",
            "agent_command": "claude-code",
            "cwd": "/tmp/project",
            "created_at": "2025-03-01T09:00:00Z",
            "last_used_at": "2025-03-01T09:30:00Z",
            "closed": false,
            "messages": [
                { "role": "user" },
                { "role": "assistant" }
            ],
            "cumulative_token_usage": { "total_tokens": 1500 }
        }"#;
        fs::write(sessions_base.join("rec-100.json"), session_json).expect("write session record");

        TestFixture {
            _tmp: tmp,
            runs_base,
            sessions_base,
            run_dir,
            events_path,
            session_stream_path,
        }
    }

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
    fn open_at_lists_runs_and_sessions_and_navigates_modes() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert_eq!(state.mode(), Mode::Ledger);
        assert_eq!(state.runs_snapshot.len(), 1);

        // Enter opens the run -> Mode::Run
        let out = state.handle_key(&key(KeyCode::Enter));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Run);

        // Enter again opens the selected step -> Mode::Step
        let out = state.handle_key(&key(KeyCode::Enter));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Step);

        // Esc returns to Mode::Run
        let out = state.handle_key(&key(KeyCode::Esc));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Run);

        // Esc again returns to Mode::Ledger
        let out = state.handle_key(&key(KeyCode::Esc));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Ledger);

        // Esc in Ledger with empty filter returns Close
        let out = state.handle_key(&key(KeyCode::Esc));
        assert_eq!(out, AcpxOutcome::Close);
    }

    #[test]
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
    fn replay_on_acp_step_returns_expected_request() {
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

### `crates/codegen/xai-grok-pager/src/acpx/step_digest.rs` lines 1-85 of 1171 (repository grok-build)
````
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::acpx::bundle::{
    FlowEdge, FlowNode, NodeKind, Outcome, Run, RunStatus, Step, fmt_duration,
};

pub use crate::acpx::step_evidence::{
    MergedToolCall, StepFileDigest, StepTraceLink, ValidationDigest, digest_step_files,
    extract_step_validations_with_outcome, read_attempt_tool_calls, step_trace_link,
};

pub const DEFAULT_FAILURE_MAX_LINES: usize = 6;
pub const DEFAULT_FAILURE_MAX_CHARS: usize = 480;

const VERDICT_REASON_MAX_CHARS: usize = 160;
const VERDICT_REASON_KEYS: &[&str] = &[
    "reason",
    "rationale",
    "summary",
    "explanation",
    "detail",
    "notes",
];

/// (a) Loop attempt count for one step execution (counted from `run.steps`; the engine has no per-node attempt limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeAttemptBudget {
    pub attempt: usize,
    pub observed_attempts: usize,
}

impl NodeAttemptBudget {
    pub fn badge_label(&self) -> Option<String> {
        if self.attempt > 1 || self.observed_attempts > 1 {
            Some(format!("attempt {}", self.attempt))
        } else {
            None
        }
    }
}

/// (c) Reviewer or router verdict extracted from a step's output and outgoing switch edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepVerdict {
    pub route_key: String,
    pub target_node: Option<String>,
    pub reason: Option<String>,
}

impl StepVerdict {
    pub fn summary_line(&self) -> String {
        let mut line = match &self.target_node {
            Some(target) => format!("verdict: {} -> {target}", self.route_key),
            None => format!("verdict: {}", self.route_key),
        };
        if let Some(reason) = &self.reason {
            line.push_str(&format!(" ({reason})"));
        }
        line
    }
}

/// (d) Time budget (`elapsed_ms` vs `timeout_ms`) and timeout state for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepTimeBudget {
    pub elapsed_ms: u64,
    pub timeout_ms: Option<u64>,
    pub timed_out: bool,
    pub near_expiry: bool,
}

impl StepTimeBudget {
    pub fn badge_label(&self) -> String {
        let elapsed = fmt_duration(self.elapsed_ms);
        match (self.timed_out, self.timeout_ms, self.near_expiry) {
            (true, Some(t), _) => format!("TIMED OUT {elapsed}/{}", fmt_duration(t)),
            (true, None, _) => format!("TIMED OUT {elapsed}"),
            (false, Some(t), true) => format!(
                "⚠ {elapsed}/{} ({}%)",
                fmt_duration(t),
                (self.elapsed_ms.saturating_mul(100) / t.max(1)).min(100)
            ),
            (false, Some(t), false) => format!("{elapsed}/{}", fmt_duration(t)),
````

### `crates/codegen/xai-grok-pager/src/acpx/step_digest.rs` lines 153-259 of 1171 (repository grok-build)
````
        line
    }
}

pub fn non_zero_exit_code(step: &Step) -> Option<i64> {
    step.action
        .as_ref()
        .and_then(|action| action.exit_code)
        .filter(|&code| code != 0)
        .or_else(|| {
            step.output
                .get("exitCode")
                .and_then(Value::as_i64)
                .filter(|&code| code != 0)
        })
}

pub fn effective_outcome(step: &Step) -> Outcome {
    if step.outcome != Outcome::Ok {
        return step.outcome;
    }
    if non_zero_exit_code(step).is_some()
        || step.output.get("ok").and_then(Value::as_bool) == Some(false)
    {
        Outcome::Failed
    } else {
        Outcome::Ok
    }
}

pub fn trim_failure_tail(
    error: Option<&str>,
    stderr: Option<&str>,
    output_tail: Option<&str>,
    stdout: Option<&str>,
    exit_code: Option<i64>,
    max_lines: usize,
    max_chars: usize,
) -> Option<String> {
    if max_lines == 0 || max_chars == 0 {
        return None;
    }

    let source = [error, stderr, output_tail, stdout]
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty());

    if let Some(text) = source {
        let non_empty_lines: Vec<&str> = text
            .lines()
            .map(|line| line.trim_end_matches('\r').trim_end())
            .filter(|line| !line.trim().is_empty())
            .collect();

        if !non_empty_lines.is_empty() {
            let start = non_empty_lines.len().saturating_sub(max_lines);
            let joined = non_empty_lines.get(start..).unwrap_or_default().join("\n");
            return Some(truncate_chars(&joined, max_chars));
        }
    }

    exit_code
        .filter(|&code| code != 0)
        .map(|code| truncate_chars(&format!("exit code {code}"), max_chars))
}

pub fn step_failure_reason(step: &Step) -> Option<String> {
    let outcome = effective_outcome(step);
    if outcome == Outcome::Ok {
        return None;
    }

    let error = step
        .error
        .as_deref()
        .or_else(|| step.output.get("error").and_then(Value::as_str));
    let stderr = step.output.get("stderr").and_then(Value::as_str);
    let output_tail = step
        .output
        .get("outputTail")
        .or_else(|| step.output.get("output_tail"))
        .or_else(|| step.output.get("tail"))
        .or_else(|| step.output.get("message"))
        .and_then(Value::as_str)
        .or(step.raw_text.as_deref());
    let stdout = step.output.get("stdout").and_then(Value::as_str);
    let exit_code = non_zero_exit_code(step);

    if let Some(trimmed) = trim_failure_tail(
        error,
        stderr,
        output_tail,
        stdout,
        exit_code,
        DEFAULT_FAILURE_MAX_LINES,
        DEFAULT_FAILURE_MAX_CHARS,
    ) {
        return Some(trimmed);
    }

    match outcome {
        Outcome::TimedOut => Some(format!("timed out after {}", fmt_duration(step.duration_ms()))),
        Outcome::Cancelled => Some("cancelled".to_string()),
        _ => None,
    }
}
````

### `crates/codegen/xai-grok-pager/src/acpx/step_digest.rs` lines 353-460 of 1171 (repository grok-build)
````
pub fn extract_step_validations(
    run: &Run,
    step: &Step,
    observed_tools: &[&str],
) -> ValidationDigest {
    extract_step_validations_with_outcome(run, step, observed_tools, effective_outcome(step))
}

pub fn step_attempt_budget(run: &Run, step: &Step) -> NodeAttemptBudget {
    let mut observed_attempts = 0usize;
    let mut attempt = None;

    for candidate in &run.steps {
        if candidate.node_id == step.node_id {
            observed_attempts += 1;
            if attempt.is_none() && candidate.attempt_id == step.attempt_id {
                attempt = Some(observed_attempts);
            }
        }
    }

    NodeAttemptBudget {
        attempt: attempt.unwrap_or(observed_attempts + 1),
        observed_attempts,
    }
}

pub fn extract_step_verdict(run: &Run, step_index: usize) -> Option<StepVerdict> {
    let step = run.steps.get(step_index)?;
    let reason = extract_verdict_reason(step);
    let next_node_id = run.steps.get(step_index + 1).map(|s| s.node_id.as_str());

    let switch_edge = run.flow.edges.iter().find_map(|edge| match edge {
        FlowEdge::Switch { from, on, cases } if from == &step.node_id => Some((on, cases)),
        _ => None,
    });

    if let Some((on, cases)) = switch_edge {
        if let Some(route_key) = resolve_output_key(&step.output, on) {
            let target_node = match_switch_target(cases, &route_key, next_node_id);
            return Some(StepVerdict {
                route_key,
                target_node,
                reason,
            });
        }

        if let Some(next_id) = next_node_id
            && let Some((case_key, target)) = cases.iter().find(|(_, target)| target == next_id)
        {
            return Some(StepVerdict {
                route_key: case_key.clone(),
                target_node: Some(target.clone()),
                reason,
            });
        }
    }

    let explicit_key = ["verdict", "decision"].iter().find_map(|key| {
        step.output
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })?;

    let target_node = switch_edge
        .and_then(|(_, cases)| match_switch_target(cases, &explicit_key, next_node_id));

    Some(StepVerdict {
        route_key: explicit_key,
        target_node,
        reason,
    })
}

pub fn step_time_budget(run: &Run, step: &Step, now_ms: i64) -> StepTimeBudget {
    let is_live_step = run.summary.status == RunStatus::Running
        && run.summary.current_node.as_deref() == Some(step.node_id.as_str())
        && step.finished_ms <= step.started_ms
        && now_ms > step.started_ms;

    let elapsed_ms = if is_live_step {
        (now_ms - step.started_ms) as u64
    } else {
        step.duration_ms()
    };

    let timeout_ms = run
        .flow
        .nodes
        .iter()
        .find(|node| node.id == step.node_id)
        .and_then(|node| node.timeout_ms);

    let timed_out = effective_outcome(step) == Outcome::TimedOut;
    let near_expiry = !timed_out
        && timeout_ms.is_some_and(|t| t > 0 && elapsed_ms.saturating_mul(100) / t >= 75);

    StepTimeBudget {
        elapsed_ms,
        timeout_ms,
        timed_out,
        near_expiry,
    }
}

````

### `crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs` lines 32-337 of 1504 (repository grok-build)
````
pub struct MergedToolCall {
    pub tool_call_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub raw_input: Value,
    pub raw_output: Option<Value>,
    pub content: Option<Value>,
    pub truncated: bool,
}

/// File-level progress and scope check for one step attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepFileDigest {
    pub expected_files: Vec<String>,
    pub touched_files: Vec<String>,
    pub out_of_scope_files: Vec<String>,
    pub current_file: Option<String>,
}

impl StepFileDigest {
    pub fn summary_line(&self) -> String {
        if self.expected_files.is_empty() && self.touched_files.is_empty() {
            return String::new();
        }
        let mut line = if !self.expected_files.is_empty() {
            format!(
                "files {}/{}",
                self.touched_files
                    .len()
                    .saturating_sub(self.out_of_scope_files.len()),
                self.expected_files.len()
            )
        } else {
            format!("files {}", self.touched_files.len())
        };
        if !self.out_of_scope_files.is_empty() {
            line.push_str(&format!(" (!{} out-of-scope)", self.out_of_scope_files.len()));
        }
        if let Some(curr) = &self.current_file {
            line.push_str(&format!(" · {curr}"));
        }
        line
    }
}

/// (b) Validator rejections and correction attempts for one step attempt, extracted from the step's transcript slice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationDigest {
    pub rejection_count: usize,
    pub correction_attempts: usize,
    pub resolved: bool,
    pub last_error: Option<String>,
    pub last_rejected_output: Option<String>,
}

impl ValidationDigest {
    /// Returns:
    /// - `None` when `self.rejection_count == 0`,
    /// - `Some(format!("1 validation rejected -> corrected ({err})"))` (or `"N validations rejected -> corrected ({err})"`)
    ///   when `self.resolved` is `true` (omitting `" ({err})"` when `self.last_error` is `None`),
    /// - `Some(format!("1 validation rejected ({err})"))` (or `"N validations rejected ({err})"`)
    ///   when `self.resolved` is `false` (omitting `" ({err})"` when `self.last_error` is `None`).
    pub fn badge_label(&self) -> Option<String> {
        if self.rejection_count == 0 {
            return None;
        }
        let noun = if self.rejection_count == 1 {
            "validation"
        } else {
            "validations"
        };
        let mut label = if self.resolved {
            format!("{} {noun} rejected -> corrected", self.rejection_count)
        } else {
            format!("{} {noun} rejected", self.rejection_count)
        };
        if let Some(err) = self.last_error.as_deref().filter(|e| !e.is_empty()) {
            label.push_str(&format!(" ({err})"));
        }
        Some(label)
    }
}

/// Direct link from a step card to its session transcript slice, stdout/stderr artifacts, and live trace filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTraceLink {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub session_events_path: Option<PathBuf>,
    pub event_range: Option<(u64, u64)>,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    pub output_artifact_path: Option<PathBuf>,
}

impl StepTraceLink {
    /// True when at least one transcript range or artifact path is available to open.
    pub fn is_openable(&self) -> bool {
        self.event_range.is_some()
            || self.session_events_path.is_some()
            || self.stdout_path.is_some()
            || self.stderr_path.is_some()
            || self.output_artifact_path.is_some()
    }

    /// Compact badge showing what can be opened, e.g. `"trace: events 12..48"` or `"trace: stderr"`, or `None` when `!is_openable()`.
    pub fn badge_label(&self) -> Option<String> {
        if !self.is_openable() {
            return None;
        }
        if let Some((start, end)) = self.event_range {
            Some(format!("trace: events {start}..{end}"))
        } else if self.stderr_path.is_some() {
            Some("trace: stderr".to_string())
        } else if self.stdout_path.is_some() {
            Some("trace: stdout".to_string())
        } else if self.output_artifact_path.is_some() {
            Some("trace: output".to_string())
        } else {
            Some("trace: session".to_string())
        }
    }
}

pub fn step_trace_link(run: &Run, step: &Step) -> StepTraceLink {
    let (session_events_path, event_range) = if let Some(conv) = &step.conversation {
        (
            run.session_events_path(&conv.session_id)
                .map(Path::to_path_buf),
            Some((conv.event_start_seq, conv.event_end_seq)),
        )
    } else if step.kind == NodeKind::Acp {
        (
            run.sessions.first().map(|s| s.events_path.clone()),
            None,
        )
    } else {
        (None, None)
    };

    let resolve_artifact = |art: &Option<crate::acpx::bundle::ArtifactRef>| -> Option<PathBuf> {
        art.as_ref()
            .filter(|a| !a.path.is_empty())
            .map(|a| run.summary.dir.join(&a.path))
    };

    StepTraceLink {
        run_id: run.summary.run_id.clone(),
        node_id: step.node_id.clone(),
        attempt_id: step.attempt_id.clone(),
        session_events_path,
        event_range,
        stdout_path: resolve_artifact(&step.stdout),
        stderr_path: resolve_artifact(&step.stderr),
        output_artifact_path: resolve_artifact(&step.output_artifact),
    }
}

pub fn read_attempt_tool_calls(run: &Run, step: &Step) -> Vec<MergedToolCall> {
    let link = step_trace_link(run, step);
    let Some(events_path) = link.session_events_path else {
        return Vec::new();
    };
    if link.event_range.is_none() && run.steps.len() > 1 {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(&events_path) else {
        return Vec::new();
    };

    let mut calls: Vec<MergedToolCall> = Vec::new();
    let mut index_by_id: HashMap<String, usize> = HashMap::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some((start, end)) = link.event_range {
            let Some(seq) = val.get("seq").and_then(Value::as_u64) else {
                continue;
            };
            if seq < start || seq > end {
                continue;
            }
        }
        if val.get("direction").and_then(Value::as_str) != Some("inbound") {
            continue;
        }
        if val.pointer("/message/method").and_then(Value::as_str) != Some("session/update") {
            continue;
        }
        let Some(update) = val.pointer("/message/params/update") else {
            continue;
        };
        let session_update = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        if session_update != "tool_call" && session_update != "tool_call_update" {
            continue;
        }

        let tool_call_id = update
            .get("toolCallId")
            .or_else(|| update.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let entry_idx = if !tool_call_id.is_empty() {
            if let Some(&idx) = index_by_id.get(tool_call_id) {
                idx
            } else {
                let idx = calls.len();
                calls.push(MergedToolCall {
                    tool_call_id: tool_call_id.to_string(),
                    title: String::new(),
                    kind: String::new(),
                    status: "pending".to_string(),
                    raw_input: Value::Null,
                    raw_output: None,
                    content: None,
                    truncated: false,
                });
                index_by_id.insert(tool_call_id.to_string(), idx);
                idx
            }
        } else {
            let idx = calls.len();
            calls.push(MergedToolCall {
                tool_call_id: String::new(),
                title: String::new(),
                kind: String::new(),
                status: "pending".to_string(),
                raw_input: Value::Null,
                raw_output: None,
                content: None,
                truncated: false,
            });
            idx
        };

        let Some(entry) = calls.get_mut(entry_idx) else {
            continue;
        };

        if let Some(title) = update
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                update
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            })
        {
            entry.title = title.to_string();
        }
        if let Some(kind) = update
            .get("kind")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            entry.kind = kind.to_string();
        }
        if let Some(status) = update
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            entry.status = status.to_string();
        }
        if let Some(raw_input) = update
            .get("rawInput")
            .or_else(|| update.get("input"))
            .or_else(|| update.get("args"))
        {
            if is_non_empty_json(raw_input) || entry.raw_input.is_null() {
                entry.raw_input = raw_input.clone();
            }
        }
        if let Some(raw_output) = update
            .get("rawOutput")
            .or_else(|| update.get("output"))
            .or_else(|| update.get("result"))
            .filter(|v| !v.is_null())
        {
            entry.raw_output = Some(raw_output.clone());
        }
        if let Some(content) = update.get("content").filter(|v| !v.is_null()) {
            entry.content = Some(content.clone());
        }
        entry.truncated |= update
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }

    calls
}
````

### `crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs` lines 339-505 of 1504 (repository grok-build)
````
pub fn digest_step_files(run: &Run, step: &Step, observed_tools: &[&str]) -> StepFileDigest {
    let expected_files = extract_expected_files(step.prompt_text.as_deref().unwrap_or(""));
    let mut touched_files: Vec<String> = Vec::new();

    for call in read_attempt_tool_calls(run, step) {
        if is_read_only_tool(&call) {
            continue;
        }
        if let Some(path) = extract_edit_or_write_path(&call) {
            let normalized = normalize_repo_path(&path);
            if !normalized.is_empty() {
                push_unique(&mut touched_files, normalized);
            }
        }
        if let Some(command) = extract_execute_command(&call) {
            for path in extract_git_command_paths(command) {
                let normalized = normalize_repo_path(&path);
                if !normalized.is_empty() {
                    push_unique(&mut touched_files, normalized);
                }
            }
        }
    }

    for tool_str in observed_tools {
        for path in extract_observed_tool_paths(tool_str) {
            let normalized = normalize_repo_path(&path);
            if !normalized.is_empty() {
                push_unique(&mut touched_files, normalized);
            }
        }
    }

    let out_of_scope_files = if expected_files.is_empty() {
        Vec::new()
    } else {
        touched_files
            .iter()
            .filter(|path| !expected_files.contains(path))
            .cloned()
            .collect()
    };
    let current_file = touched_files.last().cloned();

    StepFileDigest {
        expected_files,
        touched_files,
        out_of_scope_files,
        current_file,
    }
}

pub fn extract_step_validations_with_outcome(
    run: &Run,
    step: &Step,
    observed_tools: &[&str],
    effective_outcome: Outcome,
) -> ValidationDigest {
    let calls = read_attempt_tool_calls(run, step);
    let mut submit_calls: Vec<(&MergedToolCall, String)> = Vec::new();

    for call in &calls {
        let output_text = extract_tool_output_text(call);
        if !is_workflow_submit_call(call, &output_text) {
            continue;
        }
        if output_text.contains(PERMISSION_DENIED_MARKER)
            || output_text.contains(CHANNEL_UNAVAILABLE_MARKER)
        {
            continue;
        }
        submit_calls.push((call, output_text));
    }

    let total_submit_calls = submit_calls.len();
    let mut rejection_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut last_rejected_output: Option<String> = None;
    let mut first_rejection_idx: Option<usize> = None;
    let mut has_accepted_submit_after_last_rejection = false;

    for (idx, (call, output_text)) in submit_calls.iter().enumerate() {
        let inner_input = call.raw_input.get("tool_input").unwrap_or(&call.raw_input);
        let output_preview = inner_input
            .get("output")
            .and_then(|v| serde_json::to_string(v).ok())
            .filter(|s| !s.is_empty())
            .map(|s| truncate_chars(&s, 120));

        let is_rejected = output_text.contains(REJECTION_MARKER)
            || call.status == "failed"
            || idx + 1 < total_submit_calls;

        if is_rejected {
            rejection_count += 1;
            if first_rejection_idx.is_none() {
                first_rejection_idx = Some(idx);
            }
            has_accepted_submit_after_last_rejection = false;
            if let Some(reason) = clean_rejection_reason(output_text) {
                last_error = Some(reason);
            }
            if let Some(preview) = output_preview {
                last_rejected_output = Some(preview);
            }
        } else {
            let is_accepted = call.status == "completed"
                || output_text.contains("Submission accepted")
                || output_text.contains("Workflow step output accepted")
                || (idx + 1 == total_submit_calls
                    && effective_outcome == Outcome::Ok
                    && step.finished_ms > step.started_ms);
            if is_accepted && rejection_count > 0 {
                has_accepted_submit_after_last_rejection = true;
            }
        }
    }

    for tool_str in observed_tools {
        if tool_str.contains(REJECTION_MARKER) {
            rejection_count += 1;
            has_accepted_submit_after_last_rejection = false;
            if let Some(reason) = clean_rejection_reason(tool_str) {
                last_error = Some(reason);
            }
        }
    }

    if rejection_count == 0
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && step.kind != NodeKind::Checkpoint
    {
        if let Some(detail) = run.summary.status_detail.as_deref() {
            if detail.contains(REJECTION_MARKER) {
                rejection_count += 1;
                if let Some(reason) = clean_rejection_reason(detail) {
                    last_error = Some(reason);
                }
            }
        }
    }

    let total_submit_calls_after_first_rejection = first_rejection_idx
        .map(|first| total_submit_calls.saturating_sub(first + 1))
        .unwrap_or(0);

    let resolved = rejection_count > 0
        && (has_accepted_submit_after_last_rejection
            || (effective_outcome == Outcome::Ok
                && step.finished_ms > step.started_ms
                && run.summary.current_node.as_deref() != Some(step.node_id.as_str())
                && run.summary.waiting_on.as_deref() != Some(step.node_id.as_str())));

    let correction_attempts = if resolved {
        rejection_count
    } else {
        total_submit_calls_after_first_rejection
    };

    ValidationDigest {
        rejection_count,
        correction_attempts,
        resolved,
        last_error,
        last_rejected_output,
    }
}
````

=== TASK SPECIFICATION ===

Slice ID: attempt-detail

Ownership: the selected-attempt view in crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs, including tests,
and in crates/codegen/xai-grok-pager/src/views/acpx_modal.rs only the Run-mode focus routing: the AcpxModalState field and
initializer for it, handle_key, handle_run_key, wants_esc, the Mode::Run render
call, RUN_KEYS, and tests in its test module. Do not edit run.rs (attached
read-only; another packet owns it), the ledger, the inspector, or the Flows
preview call. Preserve the existing run and node views, inspector, ledger rows,
run copy behavior, gates, and entry routing. Keep DetailTarget's public
variants unchanged so the timeline slice can proceed without touching its
match arms. Preserve the existing scroll and narrow-width behavior.

Goal: show a compact attempt digest first, with the full evidence available
through current inspector, timeline expansion, and conversation replay.
For a failed shell attempt put the relevant bounded failure tail before the
command and cwd. Reuse step_digest, including effective outcome for a failed
shell attempt. Show attempt number, elapsed/timeout when available, and a
recorded exit code when available. Use the step position already provided by
the caller and the existing render time passed into build_lines; no invented
timeout, exit code, verdict or scope count. Preserve
any step/receipt settings that provide the actual context for the attempt.

For a finished agent step show a verdict or validator evidence when present,
a file-scope summary only when grounded in existing evidence
(digest_step_files with tools from read_attempt_tool_calls, called only inside
the cached build_lines path), and a short human-readable note if available. No
raw wrapper JSON or repeated transcript in the first view. Include accurate,
conditional directions for the current Enter inspector and r replay. Existing
details for a genuinely waiting gate must remain reachable and its
question/choices must stay conspicuous; leave its current answer routing and
labels untouched.

The digest goes first; the existing conversation, artifact and event sections
may follow it. Do not delete or rename any function: callers outside the
excerpts use them (append_conversation, parse_conversation_items,
read_last_512k and append_value_lines live outside the excerpts;
value_plain_text is used by the inspector and value_inline_summary by run.rs).
Keep meaningful evidence for other kinds of attempts available. Preserve
narrow-width wrapping and cache behavior. Don't add extra reads on every render.

Detail focus and linewise copy. The detail pane scrolls today but can't take
keyboard focus, so nothing in it can be selected or copied. In Mode::Run, Tab
and Shift+Tab move focus between the stage tree and the detail pane (two
targets, both directions), except while the run view captures keys. Focus
always starts on the stage tree and returns there when the run closes. While
the detail pane has focus, keys go to it, not the stage tree: j/k and Up/Down
move a line cursor and keep it visible; v starts or ends a visual line range
from the cursor; y copies the cursor line or the range; Y copies the whole
pane; Esc first ends visual mode, then returns focus to the stage tree, and
only Esc on the stage tree leaves the run as today (wants_esc stays true in
Mode::Run). Copy returns AcpxOutcome::Copy with plain text: no styles, no
indentation or breaks added by wrapping; a wrapped source line copies whole.
Draw the cursor line and range with existing theme styles and only while the
pane has focus; mark the focused pane the way the existing views mark focus.
Changing the selected stage resets the cursor to the top. With the tree
focused every current key, including J/K/D/U scrolling and y copying the
stage output, behaves as today. Add the new keys to RUN_KEYS under a
"detail pane" label.

Tests: Tab into the detail pane of a failed shell attempt, move to its error
tail line, y copies exactly that line; v plus movement plus y copies the range
joined with newlines; Y copies the pane; Esc ends visual mode, then returns
focus to the tree with the same stage selected, then leaves the run; Shift+Tab
cycles the other way; with the tree focused, y still copies the stage output
and v still toggles noise. Failed shell orders tail before command and cwd
without saying success on a nonzero exit; repeated attempt shows its own number and timing; agent
with brief evidence does not dump its prompt/output wrappers/transcript in the
first view, and agent without evidence doesn't invent it; a pending gate still
presents the question and choices; full evidence affordances remain accurate.
Adapt tests that explicitly asserted the old detail order; keep unrelated
formatter and navigation expectations intact.

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
=== FILE: task-scripts/acpx-disclosure-returns/attempt-detail.edits.md ===
```markdown
<complete Markdown edit list>
```
