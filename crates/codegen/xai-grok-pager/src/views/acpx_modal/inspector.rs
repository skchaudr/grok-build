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
            last_body_width: 80,
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
        if key.kind == KeyEventKind::Release {
            return InspectorOutcome::Unchanged;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => InspectorOutcome::Back,
            KeyCode::Tab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::BackTab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char(c @ '1'..='7') if !ctrl => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.tabs.len() && idx != self.active_tab {
                    self.active_tab = idx;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                if self.scroll(1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                if self.scroll(-1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('g') | KeyCode::Home if !ctrl => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y != 0
                {
                    tab.scroll_y = 0;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                let max_scroll = self.max_scroll_y();
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y != max_scroll
                {
                    tab.scroll_y = max_scroll;
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
            KeyCode::Char('y') if !ctrl => {
                let text = self
                    .tabs
                    .get(self.active_tab)
                    .map(|t| t.copy.clone().unwrap_or_else(|| t.text.clone()))
                    .unwrap_or_default();
                InspectorOutcome::Copy(text)
            }
            KeyCode::Char('r') if !ctrl => {
                if self.step.conversation.is_some() {
                    InspectorOutcome::Replay
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('[') if !ctrl => InspectorOutcome::Prev,
            KeyCode::Char(']') if !ctrl => InspectorOutcome::Next,
            _ => InspectorOutcome::Unchanged,
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        let max_scroll = self.max_scroll_y();
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let cur = tab.scroll_y.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            (cur + delta as usize).min(max_scroll)
        };
        if next != tab.scroll_y {
            tab.scroll_y = next;
            true
        } else {
            false
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, _now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Row 0: Header
        self.render_header(buf, Rect::new(area.x, area.y, area.width, 1), theme);

        if area.height < 2 {
            return;
        }

        // Row 1: Tabs
        self.render_tabs(buf, Rect::new(area.x, area.y + 1, area.width, 1), theme);

        if area.height < 3 {
            return;
        }

        // Rest: Body
        let body_area = Rect::new(area.x, area.y + 2, area.width, area.height - 2);
        self.last_body_height = body_area.height as usize;
        self.last_body_width = body_area.width as usize;
        self.render_body(buf, body_area, theme);
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut s = vec![
            "Tab/1-7: tab",
            "j/k: scroll",
            "w: wrap",
            "y: copy",
            "[/]: step",
        ];
        if self.step.conversation.is_some() {
            s.push("r: replay");
        }
        s.push("Esc: back");
        s
    }

    fn max_scroll_y(&self) -> usize {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return 0;
        };
        if tab.is_empty_artifact {
            return 0;
        }
        let (_, total_visual) = self.compute_visual_lines(tab, self.last_body_width);
        total_visual.saturating_sub(self.last_body_height.max(1))
    }

    fn compute_visual_lines(
        &self,
        tab: &TabEntry,
        area_width: usize,
    ) -> (Vec<VisualLine>, usize) {
        let src_lines: Vec<&str> = if tab.text.is_empty() {
            vec![""]
        } else {
            tab.text.lines().collect()
        };
        let total_src = src_lines.len().max(1);
        let digits = decimal_digits(total_src).max(2);
        let gutter_width = digits + 2; // "<num>  "
        let content_width = area_width.saturating_sub(gutter_width).max(1);

        let mut out = Vec::with_capacity(src_lines.len());
        for (idx, raw) in src_lines.into_iter().enumerate() {
            let line_no = idx + 1;
            if self.wrap {
                let segments = wrap_line_by_width(raw, content_width);
                for (seg_idx, seg) in segments.into_iter().enumerate() {
                    out.push(VisualLine {
                        line_no: if seg_idx == 0 { Some(line_no) } else { None },
                        text: seg,
                    });
                }
            } else {
                let sliced = slice_from_col(raw, tab.scroll_x);
                let cut = truncate_str(&sliced, content_width);
                out.push(VisualLine {
                    line_no: Some(line_no),
                    text: cut,
                });
            }
        }
        let len = out.len();
        (out, len)
    }

    fn render_header(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
        let width = row.width as usize;
        if width == 0 {
            return;
        }

        let right_label = format!("step {}/{}", self.position.0, self.position.1);
        let right_width = right_label.width();

        let glyph = NodeState::Done(self.step.outcome).glyph();
        let glyph_color = outcome_color(self.step.outcome, theme);
        let bold_primary = Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::BOLD);
        let sep_style = Style::default().fg(theme.gray_dim);
        let fact_style = Style::default().fg(theme.gray);

        let mut spans = vec![
            Span::styled(
                format!("{glyph} "),
                Style::default()
                    .fg(glyph_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(self.step.node_id.clone(), bold_primary),
        ];

        let mut facts = vec![self.step.kind.label().to_string()];
        if let Some(agent) = &self.step.agent_name
            && !agent.is_empty()
        {
            facts.push(agent.clone());
        }
        if !self.step.attempt_id.is_empty() {
            facts.push(self.step.attempt_id.clone());
        }
        facts.push(fmt_duration(self.step.duration_ms()));

        for fact in facts {
            spans.push(Span::styled(" · ", sep_style));
            spans.push(Span::styled(fact, fact_style));
        }

        let left_avail = if width > right_width + 1 {
            width - right_width - 1
        } else {
            width
        };

        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, left_avail as u16);

        if width >= right_width {
            let rx = row.x + (width - right_width) as u16;
            buf.set_stringn(
                rx,
                row.y,
                &right_label,
                right_width,
                Style::default().fg(theme.gray_dim),
            );
        }
    }

    fn render_tabs(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
        let width = row.width as usize;
        if width == 0 {
            return;
        }

        let mut spans = Vec::new();
        for (idx, tab) in self.tabs.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::styled("  ", Style::default().fg(theme.gray_dim)));
            }
            let num = idx + 1;
            let text = format!("{num} {}", tab.kind.label());
            let style = if idx == self.active_tab {
                Style::default()
                    .fg(theme.accent_assistant)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.gray)
            };
            spans.push(Span::styled(text, style));
        }

        let line = Line::from(spans);
        buf.set_line(row.x, row.y, &line, row.width);
    }

    fn render_body(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        let width = area.width as usize;
        let height = area.height as usize;
        if width == 0 || height == 0 {
            return;
        }

        let Some(mut tab_snapshot) = self.tabs.get(self.active_tab).cloned() else {
            return;
        };

        if tab_snapshot.is_empty_artifact {
            let msg = "(empty)";
            let msg_cut = truncate_str(msg, width);
            let msg_w = msg_cut.width();
            let x = area.x + ((width.saturating_sub(msg_w)) / 2) as u16;
            let y = area.y;
            buf.set_stringn(x, y, &msg_cut, width, Style::default().fg(theme.gray_dim));
            return;
        }

        let total_src = tab_snapshot.text.lines().count().max(1);
        let digits = decimal_digits(total_src).max(2);
        let gutter_width = digits + 2;

        // A block that already fits the body does not scroll sideways.
        if !self.wrap && gutter_width + widest_source_line(&tab_snapshot.text) <= width {
            tab_snapshot.scroll_x = 0;
        }

        let (visual_lines, total_visual) = self.compute_visual_lines(&tab_snapshot, width);
        let max_scroll = total_visual.saturating_sub(height);
        let scroll_y = tab_snapshot.scroll_y.min(max_scroll);
        if let Some(tab_mut) = self.tabs.get_mut(self.active_tab) {
            tab_mut.scroll_y = scroll_y;
        }

        let widest_text = if self.wrap {
            visual_lines
                .iter()
                .map(|v| v.text.width())
                .max()
                .unwrap_or(0)
        } else {
            widest_source_line(&tab_snapshot.text)
        };
        let (x_off, y_off) = centre_offset(gutter_width + widest_text, total_visual, width, height);

        let gutter_style = Style::default().fg(theme.gray_dim);
        let text_style = if tab_snapshot.kind == TabKind::Error {
            Style::default().fg(theme.accent_error)
        } else {
            Style::default().fg(theme.text_primary)
        };

        for row_idx in 0..height {
            let v_idx = scroll_y + row_idx;
            let Some(vline) = visual_lines.get(v_idx) else {
                break;
            };
            let y = area.y + (y_off + row_idx) as u16;

            let gutter_str = match vline.line_no {
                Some(n) => format!("{n:>digits$}  "),
                None => " ".repeat(gutter_width),
            };

            if width <= gutter_width {
                let cut_g = truncate_str(&gutter_str, width);
                buf.set_stringn(area.x, y, &cut_g, width, gutter_style);
                continue;
            }

            let line = Line::from(vec![
                Span::styled(gutter_str, gutter_style),
                Span::styled(vline.text.clone(), text_style),
            ]);
            buf.set_line(area.x + x_off as u16, y, &line, (width - x_off) as u16);
        }

        // Bottom-right overflow counter `12-40/312`
        if total_visual > height {
            let start = scroll_y + 1;
            let end = (scroll_y + height).min(total_visual);
            let overlay = format!("{start}-{end}/{total_visual}");
            let ov_width = overlay.width();
            if ov_width <= width {
                let ox = area.x + (width - ov_width) as u16;
                let oy = area.y + (height - 1) as u16;
                buf.set_stringn(
                    ox,
                    oy,
                    &overlay,
                    ov_width,
                    Style::default().fg(theme.gray_dim).bg(theme.bg_base),
                );
            }
        }
    }
}

struct VisualLine {
    line_no: Option<usize>,
    text: String,
}

/// Display width of the widest source line.
fn widest_source_line(text: &str) -> usize {
    text.lines().map(|l| l.width()).max().unwrap_or(0)
}

/// Offset (columns, rows) that centres a `block_w` x `block_h` block in a
/// `width` x `height` area. A block taller or wider than the area is drawn
/// from the top-left corner, as a scrolling block always has been.
fn centre_offset(block_w: usize, block_h: usize, width: usize, height: usize) -> (usize, usize) {
    if block_w > width || block_h > height {
        return (0, 0);
    }
    ((width - block_w) / 2, (height - block_h) / 2)
}

fn outcome_color(outcome: Outcome, theme: &Theme) -> Color {
    match outcome {
        Outcome::Ok => theme.accent_success,
        Outcome::Failed | Outcome::TimedOut => theme.accent_error,
        Outcome::Cancelled => theme.warning,
        Outcome::Unknown => theme.gray,
    }
}

fn read_artifact_tab(run_dir: &Path, artifact: &ArtifactRef) -> (String, bool) {
    if artifact.bytes == 0 {
        match bundle::read_artifact(run_dir, artifact, ARTIFACT_MAX_BYTES) {
            Ok(s) if s.is_empty() => (String::new(), true),
            Ok(s) => (s, false),
            Err(_) => (String::new(), true),
        }
    } else {
        match bundle::read_artifact(run_dir, artifact, ARTIFACT_MAX_BYTES) {
            Ok(s) if s.is_empty() => (String::new(), true),
            Ok(s) => (s, false),
            Err(e) => (format!("error reading {}: {e}", artifact.path), false),
        }
    }
}

fn build_meta_text(step: &Step, node: Option<&FlowNode>) -> String {
    let mut rows: Vec<(&str, String)> = Vec::new();

    rows.push(("node", step.node_id.clone()));
    rows.push(("attempt", step.attempt_id.clone()));
    rows.push(("kind", step.kind.label().to_string()));

    if let Some(n) = node
        && let Some(sum) = &n.summary
        && !sum.is_empty()
    {
        rows.push(("summary", sum.clone()));
    }

    let outcome_str = match step.outcome {
        Outcome::Ok => "✓ ok",
        Outcome::TimedOut => "✗ timed_out",
        Outcome::Failed => "✗ failed",
        Outcome::Cancelled => "⊘ cancelled",
        Outcome::Unknown => "? unknown",
    };
    rows.push(("outcome", outcome_str.to_string()));

    if step.started_ms > 0 {
        rows.push(("started", fmt_utc_hms_ms(step.started_ms)));
    }
    if step.finished_ms > 0 {
        rows.push(("finished", fmt_utc_hms_ms(step.finished_ms)));
    }
    rows.push(("duration", fmt_duration(step.duration_ms())));

    if let Some(agent) = &step.agent_name
        && !agent.is_empty()
    {
        rows.push(("agent", agent.clone()));
    }

    if let Some(act) = &step.action {
        if !act.action_type.is_empty() {
            rows.push(("action", act.action_type.clone()));
        }
        let mut cmd_parts = Vec::new();
        if let Some(cmd) = &act.command
            && !cmd.is_empty()
        {
            cmd_parts.push(cmd.clone());
        }
        for arg in &act.args {
            cmd_parts.push(arg.clone());
        }
        if !cmd_parts.is_empty() {
            rows.push(("command", cmd_parts.join(" ")));
        }
        if let Some(cwd) = &act.cwd
            && !cwd.is_empty()
        {
            rows.push(("cwd", cwd.clone()));
        }
        if let Some(code) = act.exit_code {
            rows.push(("exit_code", code.to_string()));
        }
    }

    if let Some(conv) = &step.conversation {
        rows.push(("session", conv.session_id.clone()));
        rows.push((
            "events",
            format!("{}..{}", conv.event_start_seq, conv.event_end_seq),
        ));
    }

    if let Some(art) = &step.stdout {
        rows.push(("stdout", format!("{} ({} B)", art.path, art.bytes)));
    }
    if let Some(art) = &step.stderr {
        rows.push(("stderr", format!("{} ({} B)", art.path, art.bytes)));
    }
    if let Some(art) = &step.output_artifact {
        rows.push(("output_artifact", format!("{} ({} B)", art.path, art.bytes)));
    }

    let key_width = rows.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    let mut lines = Vec::with_capacity(rows.len());
    for (k, v) in rows {
        lines.push(format!("{k:<key_width$}  {v}"));
    }
    lines.join("\n")
}

fn fmt_utc_hms_ms(ms: i64) -> String {
    match Utc.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%H:%M:%S%.3f").to_string(),
        None => ms.to_string(),
    }
}

fn decimal_digits(mut n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut d = 0;
    while n > 0 {
        n /= 10;
        d += 1;
    }
    d
}

fn wrap_line_by_width(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    if line.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for ch in line.chars() {
        let cw = ch.width().unwrap_or(0);
        if current_w + cw > max_width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_w = 0;
        }
        current.push(ch);
        current_w += cw;
    }

    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

fn slice_from_col(line: &str, skip_cols: usize) -> String {
    if skip_cols == 0 {
        return line.to_string();
    }
    let mut skipped = 0usize;
    let mut out = String::new();
    for ch in line.chars() {
        let cw = ch.width().unwrap_or(0);
        if skipped + cw <= skip_cols {
            skipped += cw;
            continue;
        }
        if skipped < skip_cols {
            // Wide character straddling skip boundary
            skipped += cw;
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{ActionReceipt, Conversation};
    use serde_json::json;
    use std::fs;

    fn sample_shell_step() -> Step {
        Step {
            attempt_id: "build#1".to_string(),
            node_id: "build".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Ok,
            started_ms: 1_700_000_000_000,
            finished_ms: 1_700_000_001_250,
            prompt_text: None,
            raw_text: Some("raw response log".to_string()),
            output: json!({"ok": true}),
            error: None,
            agent_name: None,
            action: Some(ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("cargo".to_string()),
                args: vec!["test".to_string()],
                cwd: Some("/workspace".to_string()),
                exit_code: Some(0),
                duration_ms: Some(1250),
            }),
            conversation: None,
            stdout: Some(ArtifactRef {
                path: "artifacts/stdout.txt".to_string(),
                media_type: "text/plain".to_string(),
                bytes: 12,
            }),
            stderr: Some(ArtifactRef {
                path: "artifacts/stderr.txt".to_string(),
                media_type: "text/plain".to_string(),
                bytes: 0,
            }),
            output_artifact: None,
        }
    }

    fn sample_acp_step() -> Step {
        Step {
            attempt_id: "plan#1".to_string(),
            node_id: "plan".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_700_000_000_000,
            finished_ms: 1_700_000_005_000,
            prompt_text: Some("Write a plan".to_string()),
            raw_text: Some("Step 1: do x\nStep 2: do y".to_string()),
            output: json!("Plan output"),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: Some(Conversation {
                session_id: "sess-1".to_string(),
                event_start_seq: 1,
                event_end_seq: 14,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        let mut s = String::new();
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.trim_end().to_string()
    }

    #[test]
    fn shell_action_tab_set_and_empty_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let art_dir = dir.path().join("artifacts");
        fs::create_dir_all(&art_dir).unwrap();
        fs::write(art_dir.join("stdout.txt"), "hello world\n").unwrap();
        fs::write(art_dir.join("stderr.txt"), "").unwrap();

        let mut state = InspectorState::new(dir.path(), sample_shell_step(), None, (2, 4));
        let kinds: Vec<TabKind> = state.tabs.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TabKind::Output,
                TabKind::Response,
                TabKind::Stdout,
                TabKind::Stderr,
                TabKind::Meta,
            ]
        );
        assert_eq!(state.active_tab, 0);

        // Select Stderr (4th tab -> index 3) and render: should show (empty)
        state.handle_key(&KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        assert_eq!(state.tabs[state.active_tab].kind, TabKind::Stderr);

        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10));
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 0);
        let body_first_row = row_text(&buf, 2);
        assert!(body_first_row.contains("(empty)"), "got: {body_first_row}");
    }

    #[test]
    fn acp_step_tab_set() {
        let dir = tempfile::tempdir().unwrap();
        let state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 3));
        let kinds: Vec<TabKind> = state.tabs.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TabKind::Output,
                TabKind::Prompt,
                TabKind::Response,
                TabKind::Meta,
            ]
        );
    }

    #[test]
    fn failed_step_opens_on_error_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.outcome = Outcome::Failed;
        step.error = Some("exit status 1".to_string());

        let state = InspectorState::new(dir.path(), step, None, (1, 1));
        assert_eq!(state.tabs[state.active_tab].kind, TabKind::Error);
    }

    #[test]
    fn digit_and_tab_cycle_selection() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 2));
        assert_eq!(state.active_tab, 0);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.active_tab, 1);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.active_tab, 0);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.active_tab, 2);

        // Out-of-range digit is Unchanged
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE)),
            InspectorOutcome::Unchanged
        );
        assert_eq!(state.active_tab, 2);
    }

    #[test]
    fn per_tab_scroll_memory_and_clamping() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!(
            (1..=30)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        step.prompt_text = Some(
            (1..=20)
                .map(|i| format!("prompt {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );

        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10)); // body height = 8
        let area = buf.area;
        state.render(&mut buf, area, &Theme::default(), 0);

        // Scroll Output tab down by 5
        assert!(state.scroll(5));
        assert_eq!(state.tabs[0].scroll_y, 5);

        // Switch to Prompt tab: scroll is 0
        state.handle_key(&KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(state.active_tab, 1);
        assert_eq!(state.tabs[1].scroll_y, 0);

        // Scroll Prompt tab to bottom via G and verify clamp
        state.handle_key(&KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
        assert_eq!(state.tabs[1].scroll_y, 20 - 8);
        assert!(!state.scroll(10)); // Clamped at end

        // Scroll back to top via g and verify clamp
        state.handle_key(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(state.tabs[1].scroll_y, 0);
        assert!(!state.scroll(-5)); // Clamped at top

        // Switch back to Output: still at 5
        state.handle_key(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(state.tabs[0].scroll_y, 5);
    }

    #[test]
    fn wrap_toggle_changes_visual_line_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!("a".repeat(120));

        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let tab = state.tabs[0].clone();

        let (_, wrapped_count) = state.compute_visual_lines(&tab, 40);
        assert!(wrapped_count > 1);

        state.handle_key(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert!(!state.wrap);
        let (_, unwrapped_count) = state.compute_visual_lines(&tab, 40);
        assert_eq!(unwrapped_count, 1);
    }

    #[test]
    fn wrapping_wide_cjk_chars_never_exceeds_width() {
        let line = "日本語テスト文字列宽度检查😊漢字";
        for max_w in 2..=15 {
            let wrapped = wrap_line_by_width(line, max_w);
            for seg in &wrapped {
                assert!(
                    seg.width() <= max_w,
                    "segment {seg:?} width {} > {max_w}",
                    seg.width()
                );
            }
            let rejoined: String = wrapped.into_iter().collect();
            assert_eq!(rejoined, line);
        }
    }

    #[test]
    fn copy_returns_active_tab_full_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 1));
        state.handle_key(&KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE)); // Prompt
        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out, InspectorOutcome::Copy("Write a plan".to_string()));
    }

    #[test]
    fn json_output_tab_reads_as_rows_and_copy_still_gives_raw_json() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!({
            "verdict": "approve",
            "findings": [],
            "notes": ["a", "b"],
            "next": {"approve": "land.flow.ts", "meta": {}}
        });
        let mut state = InspectorState::new(dir.path(), step.clone(), None, (1, 1));
        let tab = state
            .tabs
            .iter()
            .find(|t| t.kind == TabKind::Output)
            .expect("output tab");
        assert!(!tab.text.contains('{') && !tab.text.contains('"'), "{}", tab.text);
        assert!(tab.text.contains("verdict   approve"), "{}", tab.text);
        assert!(tab.text.contains("findings  (empty)"), "{}", tab.text);
        assert!(tab.text.contains("- a") && tab.text.contains("- b"), "{}", tab.text);
        assert!(tab.text.contains("  approve  land.flow.ts"), "{}", tab.text);
        assert!(tab.text.contains("meta     (empty)"), "{}", tab.text);

        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out, InspectorOutcome::Copy(value_text(&step.output)));
        let InspectorOutcome::Copy(raw) = out else { unreachable!() };
        assert!(raw.contains("\"verdict\": \"approve\""));
    }

    #[test]
    fn string_output_tab_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!("plain reply");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let tab = &state.tabs[state.active_tab];
        assert_eq!(tab.text, "plain reply");
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            InspectorOutcome::Copy("plain reply".to_string())
        );
    }

    #[test]
    fn replay_only_on_step_with_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let mut acp_state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 1));
        assert_eq!(
            acp_state.handle_key(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            InspectorOutcome::Replay
        );

        let mut shell_state = InspectorState::new(dir.path(), sample_shell_step(), None, (1, 1));
        assert_eq!(
            shell_state.handle_key(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            InspectorOutcome::Unchanged
        );
    }

    #[test]
    fn prev_next_and_back_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 2));
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE)),
            InspectorOutcome::Prev
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
            InspectorOutcome::Next
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Back
        );
    }

    #[test]
    fn missing_artifact_shows_error_text_in_tab() {
        let dir = tempfile::tempdir().unwrap();
        let state = InspectorState::new(dir.path(), sample_shell_step(), None, (1, 1));
        let stdout_tab = state
            .tabs
            .iter()
            .find(|t| t.kind == TabKind::Stdout)
            .unwrap();
        assert!(
            stdout_tab.text.contains("artifacts/stdout.txt"),
            "got: {}",
            stdout_tab.text
        );
    }

    #[test]
    fn render_100x20_has_header_and_tabs_and_0x0_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (3, 5));
        let theme = Theme::default();

        // 0x0 should not panic
        let mut empty_buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        let area = empty_buf.area;
        state.render(&mut empty_buf, area, &theme, 0);

        // 100x20 render
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 0);

        let r0 = row_text(&buf, 0);
        assert!(r0.contains("✓ plan"), "header row: {r0}");
        assert!(r0.contains("acp"), "header row: {r0}");
        assert!(r0.contains("claude"), "header row: {r0}");
        assert!(r0.contains("plan#1"), "header row: {r0}");
        assert!(r0.ends_with("step 3/5"), "header row: {r0}");

        let r1 = row_text(&buf, 1);
        assert!(r1.contains("1 Output"), "tab row: {r1}");
        assert!(r1.contains("2 Prompt"), "tab row: {r1}");
        assert!(r1.contains("3 Response"), "tab row: {r1}");
        assert!(r1.contains("4 Meta"), "tab row: {r1}");
    }

    /// Render the active tab of a state whose Output text is `text`.
    fn render_text(text: &str, wrap: bool, w: u16, h: u16, keys: &[char]) -> Buffer {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!(text);
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        state.wrap = wrap;
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        let area = buf.area;
        state.render(&mut buf, area, &theme, 0);
        for k in keys {
            state.handle_key(&KeyEvent::new(KeyCode::Char(*k), KeyModifiers::NONE));
        }
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        state.render(&mut buf, area, &theme, 0);
        buf
    }

    /// (row, column) of the first non-space cell at or below the body's first row.
    fn first_content_cell(buf: &Buffer) -> Option<(u16, u16)> {
        (2..buf.area.height).find_map(|y| {
            (0..buf.area.width)
                .find(|&x| buf[(x, y)].symbol() != " ")
                .map(|x| (y, x))
        })
    }

    #[test]
    fn short_content_is_centred_as_one_block() {
        // Body 40x18; block = 4-col gutter + 6-col widest line = 10 x 3 rows.
        let buf = render_text("ab\nabcdef\nabc", false, 40, 20, &[]);
        assert_eq!(
            first_content_cell(&buf),
            Some((9, 16)),
            "row, col of the line number"
        );
        assert_eq!(buf[(19, 9)].symbol(), "a");
        // Lines keep their own left alignment inside the block.
        assert_eq!(buf[(19, 10)].symbol(), "a");
        assert_eq!(buf[(19, 11)].symbol(), "a");
        assert_eq!(buf[(22, 10)].symbol(), "d");
        assert_eq!(buf[(22, 9)].symbol(), " ");
        // Header and tab rows stay put.
        assert!(row_text(&buf, 0).starts_with("✓ plan"));
        assert!(row_text(&buf, 1).starts_with("1 Output"));
        // Sideways scrolling does not move a block that fits.
        let moved = render_text("ab\nabcdef\nabc", false, 40, 20, &['l', 'l']);
        assert_eq!(buf, moved);
    }

    #[test]
    fn tall_content_starts_at_the_top_row() {
        let text = (1..=30)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let buf = render_text(&text, false, 40, 20, &[]);
        assert_eq!(first_content_cell(&buf), Some((2, 1)));
        assert!(
            row_text(&buf, 19).ends_with("1-18/30"),
            "{}",
            row_text(&buf, 19)
        );
    }

    #[test]
    fn wide_content_starts_at_the_left_column() {
        let buf = render_text(&"x".repeat(60), false, 40, 20, &[]);
        assert_eq!(first_content_cell(&buf), Some((2, 1)));
    }

    #[test]
    fn wrap_mode_centres_the_wrapped_lines() {
        // Body 50x18; content width 46 wraps 60 x's into 46 + 14, plus "b": 3 rows, 50 wide.
        let text = format!("{}\nb", "x".repeat(60));
        let buf = render_text(&text, true, 50, 20, &[]);
        assert_eq!(
            first_content_cell(&buf),
            Some((9, 1)),
            "3 wrapped rows centred vertically"
        );
        // A wrapped block narrower than the body is centred sideways as well.
        let buf = render_text("ab\nabcdef", true, 40, 20, &[]);
        assert_eq!(first_content_cell(&buf), Some((10, 16)), "2 rows in 18");
        assert_eq!(buf[(19, 10)].symbol(), "a");
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 3), (5, 3), (4, 4), (60, 2), (2, 30)] {
            render_text("ab\nabcdef", false, w, h, &[]);
            render_text("ab\nabcdef", true, w, h, &[]);
        }
        assert_eq!(centre_offset(10, 3, 0, 0), (0, 0));
        assert_eq!(centre_offset(10, 3, 40, 18), (15, 7));
        assert_eq!(centre_offset(10, 30, 40, 18), (0, 0));
        assert_eq!(centre_offset(50, 3, 40, 18), (0, 0));
    }
}
