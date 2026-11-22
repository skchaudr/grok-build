```rust
//! Capped live tail of a subagent's transcript for inline display inside `SubagentBlock`.

use std::collections::VecDeque;

use agent_client_protocol as acp;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

const MAX_ENTRIES: usize = 400;
const MAX_TEXT_CHARS: usize = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolState {
    Pending,
    Running,
    Done,
    Failed,
}

impl ToolState {
    fn from_acp(status: acp::ToolCallStatus) -> Self {
        match status {
            acp::ToolCallStatus::Pending => Self::Pending,
            acp::ToolCallStatus::InProgress => Self::Running,
            acp::ToolCallStatus::Completed => Self::Done,
            acp::ToolCallStatus::Failed => Self::Failed,
            _ => Self::Running,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewEntry {
    Thought(String),
    Message(String),
    User(String),
    Tool {
        id: String,
        title: String,
        status: ToolState,
    },
    Plan {
        done: usize,
        total: usize,
    },
}

/// A capped live tail of a subagent's transcript, fed with the same session updates the child view receives.
#[derive(Debug, Clone, Default)]
pub struct SubagentPreview {
    entries: VecDeque<PreviewEntry>,
    revision: u64,
}

impl SubagentPreview {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one child session update into the preview. Handles AgentThoughtChunk, AgentMessageChunk,
    /// UserMessageChunk, ToolCall, ToolCallUpdate, Plan; ignores the rest. Never panics on any input.
    pub fn ingest(&mut self, update: &acp::SessionUpdate) {
        let changed = match update {
            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                if let Some(text) = extract_text(&chunk.content) {
                    self.append_thought(&text)
                } else {
                    false
                }
            }
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                if let Some(text) = extract_text(&chunk.content) {
                    self.append_message(&text)
                } else {
                    false
                }
            }
            acp::SessionUpdate::UserMessageChunk(chunk) => {
                if let Some(text) = extract_text(&chunk.content) {
                    self.append_user(&text)
                } else {
                    false
                }
            }
            acp::SessionUpdate::ToolCall(tc) => {
                let id = tc.tool_call_id.0.to_string();
                let title = sanitize_single_line(&tc.title);
                let status = ToolState::from_acp(tc.status);
                self.upsert_tool(id, Some(title), Some(status), true)
            }
            acp::SessionUpdate::ToolCallUpdate(tcu) => {
                let id = tcu.tool_call_id.0.to_string();
                let title = tcu
                    .fields
                    .title
                    .as_deref()
                    .map(sanitize_single_line)
                    .filter(|s| !s.is_empty());
                let status = tcu.fields.status.map(ToolState::from_acp);
                self.upsert_tool(id, title, status, false)
            }
            acp::SessionUpdate::Plan(plan) => {
                let total = plan.entries.len();
                let done = plan
                    .entries
                    .iter()
                    .filter(|e| matches!(e.status, acp::PlanEntryStatus::Completed))
                    .count();
                self.upsert_plan(done, total)
            }
            _ => false,
        };

        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// The last `max_rows` display rows at `width` columns (wrapped, never wider than width), oldest first.
    pub fn render(&self, width: u16, max_rows: usize, theme: &Theme) -> Vec<Line<'static>> {
        let width = width as usize;
        if width == 0 || max_rows == 0 || self.entries.is_empty() {
            return Vec::new();
        }

        let mut collected_rev: Vec<Line<'static>> = Vec::with_capacity(max_rows);
        for entry in self.entries.iter().rev() {
            let entry_lines = render_entry(entry, width, theme);
            for line in entry_lines.into_iter().rev() {
                collected_rev.push(line);
                if collected_rev.len() >= max_rows {
                    break;
                }
            }
            if collected_rev.len() >= max_rows {
                break;
            }
        }

        collected_rev.reverse();
        collected_rev
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A cheap counter that changes whenever ingest changed what render would draw (for cache invalidation).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn push_entry(&mut self, entry: PreviewEntry) {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    fn append_thought(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if let Some(PreviewEntry::Thought(existing)) = self.entries.back_mut() {
            append_capped(existing, text);
        } else {
            let mut s = String::new();
            append_capped(&mut s, text);
            self.push_entry(PreviewEntry::Thought(s));
        }
        true
    }

    fn append_message(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if let Some(PreviewEntry::Message(existing)) = self.entries.back_mut() {
            append_capped(existing, text);
        } else {
            let mut s = String::new();
            append_capped(&mut s, text);
            self.push_entry(PreviewEntry::Message(s));
        }
        true
    }

    fn append_user(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if let Some(PreviewEntry::User(existing)) = self.entries.back_mut() {
            append_capped(existing, text);
        } else {
            let mut s = String::new();
            append_capped(&mut s, text);
            self.push_entry(PreviewEntry::User(s));
        }
        true
    }

    fn upsert_tool(
        &mut self,
        id: String,
        title: Option<String>,
        status: Option<ToolState>,
        always_insert_if_missing: bool,
    ) -> bool {
        if let Some(PreviewEntry::Tool {
            title: existing_title,
            status: existing_status,
            ..
        }) = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e, PreviewEntry::Tool { id: existing_id, .. } if *existing_id == id))
        {
            let mut changed = false;
            if let Some(new_title) = title.filter(|t| !t.is_empty())
                && *existing_title != new_title
            {
                *existing_title = new_title;
                changed = true;
            }
            if let Some(new_status) = status
                && *existing_status != new_status
            {
                *existing_status = new_status;
                changed = true;
            }
            return changed;
        }

        if !always_insert_if_missing && title.is_none() && status.is_none() {
            return false;
        }

        let resolved_title = title.filter(|t| !t.is_empty()).unwrap_or_else(|| id.clone());
        let resolved_status = status.unwrap_or(ToolState::Running);
        self.push_entry(PreviewEntry::Tool {
            id,
            title: resolved_title,
            status: resolved_status,
        });
        true
    }

    fn upsert_plan(&mut self, done: usize, total: usize) -> bool {
        if let Some(PreviewEntry::Plan {
            done: existing_done,
            total: existing_total,
        }) = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e, PreviewEntry::Plan { .. }))
        {
            if *existing_done == done && *existing_total == total {
                return false;
            }
            *existing_done = done;
            *existing_total = total;
            return true;
        }
        self.push_entry(PreviewEntry::Plan { done, total });
        true
    }
}

fn append_capped(target: &mut String, addition: &str) {
    target.push_str(addition);
    if target.len() > MAX_TEXT_CHARS {
        let excess = target.len() - MAX_TEXT_CHARS;
        let cut = target
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= excess)
            .unwrap_or(target.len());
        target.drain(..cut);
    }
}

fn extract_text(content: &acp::ContentBlock) -> Option<String> {
    match content {
        acp::ContentBlock::Text(text_content) => {
            if text_content.text.is_empty() {
                None
            } else {
                Some(text_content.text.clone())
            }
        }
        _ => None,
    }
}

fn sanitize_single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_entry(entry: &PreviewEntry, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    match entry {
        PreviewEntry::Thought(text) => {
            let style = Style::default()
                .fg(theme.gray)
                .add_modifier(Modifier::ITALIC);
            render_prefixed_text(text, "∴ ", "  ", width, style, style)
        }
        PreviewEntry::Message(text) => {
            let style = Style::default().fg(theme.text_primary);
            render_prefixed_text(text, "", "  ", width, style, style)
        }
        PreviewEntry::User(text) => {
            let style = Style::default().fg(theme.gray_dim);
            render_prefixed_text(text, "› ", "  ", width, style, style)
        }
        PreviewEntry::Tool { title, status, .. } => {
            vec![render_tool_row(title, *status, width, theme)]
        }
        PreviewEntry::Plan { done, total } => {
            let raw = format!("☐ {done}/{total} steps");
            let clipped = truncate_to_width(&raw, width);
            let style = Style::default().fg(theme.gray);
            vec![Line::from(Span::styled(clipped, style))]
        }
    }
}

fn render_tool_row(
    title: &str,
    status: ToolState,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let status_style = match status {
        ToolState::Running => Style::default().fg(theme.accent_running),
        ToolState::Done => Style::default().fg(theme.accent_success),
        ToolState::Failed => Style::default().fg(theme.accent_error),
        ToolState::Pending => Style::default().fg(theme.gray_dim),
    };
    let title_style = Style::default().fg(theme.gray_bright);
    let glyph = "▸ ";
    let glyph_w = glyph.width();
    let status_word = status.word();
    let suffix = format!("  {status_word}");
    let suffix_w = suffix.width();

    if width <= glyph_w {
        return Line::from(Span::styled(truncate_to_width(glyph, width), status_style));
    }

    let clean_title = if title.trim().is_empty() {
        "tool"
    } else {
        title.trim()
    };

    if glyph_w + suffix_w < width {
        let avail_title = width - glyph_w - suffix_w;
        let fitted_title = truncate_to_width(clean_title, avail_title);
        Line::from(vec![
            Span::styled(glyph.to_string(), status_style),
            Span::styled(fitted_title, title_style),
            Span::styled(suffix, status_style),
        ])
    } else {
        // Very narrow width: ensure glyph and status word are prioritized and never exceed width.
        let rest_avail = width - glyph_w;
        let compact = format!("{clean_title} {status_word}");
        let fitted = truncate_to_width(&compact, rest_avail);
        Line::from(vec![
            Span::styled(glyph.to_string(), status_style),
            Span::styled(fitted, status_style),
        ])
    }
}

fn normalize_paragraphs(text: &str) -> Vec<String> {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<String> = Vec::new();
    let mut prev_blank = false;

    for raw_line in trimmed.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            if !out.is_empty() && !prev_blank {
                out.push(String::new());
                prev_blank = true;
            }
        } else {
            out.push(line.to_string());
            prev_blank = false;
        }
    }

    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }

    out
}

fn render_prefixed_text(
    text: &str,
    first_prefix: &str,
    cont_prefix: &str,
    width: usize,
    prefix_style: Style,
    body_style: Style,
) -> Vec<Line<'static>> {
    let paragraphs = normalize_paragraphs(text);
    if paragraphs.is_empty() || width == 0 {
        return Vec::new();
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut is_first_row = true;

    for para in paragraphs {
        if para.is_empty() {
            let pfx = if is_first_row { first_prefix } else { "" };
            let clipped = truncate_to_width(pfx.trim_end(), width);
            lines.push(Line::from(Span::styled(clipped, prefix_style)));
            is_first_row = false;
            continue;
        }

        let wrapped = wrap_paragraph(&para, first_prefix, cont_prefix, is_first_row, width);
        for (pfx, body) in wrapped {
            if pfx.is_empty() {
                lines.push(Line::from(Span::styled(body, body_style)));
            } else if body.is_empty() {
                lines.push(Line::from(Span::styled(pfx, prefix_style)));
            } else if prefix_style == body_style {
                lines.push(Line::from(Span::styled(format!("{pfx}{body}"), body_style)));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(pfx, prefix_style),
                    Span::styled(body, body_style),
                ]));
            }
            is_first_row = false;
        }
    }

    lines
}

/// Wraps a single non-empty line into `(prefix, body)` pairs where `prefix.width() + body.width() <= width`.
fn wrap_paragraph(
    para: &str,
    first_prefix: &str,
    cont_prefix: &str,
    starts_entry: bool,
    width: usize,
) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();

    let effective_prefix = |is_first_of_entry: bool| -> String {
        let candidate = if is_first_of_entry {
            first_prefix
        } else {
            cont_prefix
        };
        if candidate.width() < width {
            candidate.to_string()
        } else {
            String::new()
        }
    };

    let mut current_is_first = starts_entry;
    let mut current_prefix = effective_prefix(current_is_first);
    let mut avail = width.saturating_sub(current_prefix.width()).max(1);
    let mut current_body = String::new();
    let mut current_w = 0usize;

    let words: Vec<&str> = para.split_whitespace().collect();
    if words.is_empty() {
        return rows;
    }

    for word in words {
        let word_w = word.width();

        if current_body.is_empty() {
            if word_w <= avail {
                current_body.push_str(word);
                current_w = word_w;
            } else {
                // Hard-cut word across rows
                let mut rest = word;
                while !rest.is_empty() {
                    let (chunk, remainder) = split_at_display_width(rest, avail);
                    if chunk.is_empty() {
                        break;
                    }
                    if remainder.is_empty() {
                        current_body.push_str(chunk);
                        current_w = chunk.width();
                    } else {
                        rows.push((current_prefix, chunk.to_string()));
                        current_is_first = false;
                        current_prefix = effective_prefix(current_is_first);
                        avail = width.saturating_sub(current_prefix.width()).max(1);
                    }
                    rest = remainder;
                }
            }
        } else if current_w + 1 + word_w <= avail {
            current_body.push(' ');
            current_body.push_str(word);
            current_w += 1 + word_w;
        } else {
            // Flush current line and start a continuation line
            rows.push((current_prefix, std::mem::take(&mut current_body)));
            current_is_first = false;
            current_prefix = effective_prefix(current_is_first);
            avail = width.saturating_sub(current_prefix.width()).max(1);
            current_w = 0;

            if word_w <= avail {
                current_body.push_str(word);
                current_w = word_w;
            } else {
                let mut rest = word;
                while !rest.is_empty() {
                    let (chunk, remainder) = split_at_display_width(rest, avail);
                    if chunk.is_empty() {
                        break;
                    }
                    if remainder.is_empty() {
                        current_body.push_str(chunk);
                        current_w = chunk.width();
                    } else {
                        rows.push((current_prefix, chunk.to_string()));
                        current_prefix = effective_prefix(false);
                        avail = width.saturating_sub(current_prefix.width()).max(1);
                    }
                    rest = remainder;
                }
            }
        }
    }

    if !current_body.is_empty() {
        rows.push((current_prefix, current_body));
    }

    rows
}

fn split_at_display_width(s: &str, max_width: usize) -> (&str, &str) {
    if max_width == 0 || s.is_empty() {
        return ("", s);
    }
    let mut used = 0usize;
    let mut split_idx = 0usize;
    for (idx, ch) in s.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > max_width {
            break;
        }
        used += cw;
        split_idx = idx + ch.len_utf8();
    }
    // If the very first character is wider than `max_width` (e.g., CJK char at width 1),
    // skip or drop it so we never exceed `max_width` and never loop infinitely.
    if split_idx == 0 {
        if let Some((_, ch)) = s.char_indices().next() {
            let next = ch.len_utf8();
            return ("", &s[next..]);
        }
    }
    (&s[..split_idx], &s[split_idx..])
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 || s.is_empty() {
        return String::new();
    }
    if s.width() <= max_width {
        return s.to_string();
    }
    if max_width == 1 {
        let (head, _) = split_at_display_width(s, 1);
        return head.to_string();
    }
    let (head, _) = split_at_display_width(s, max_width - 1);
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use unicode_width::UnicodeWidthStr;

    fn parse_update(val: Value) -> acp::SessionUpdate {
        serde_json::from_value(val).expect("valid acp::SessionUpdate JSON in test")
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn line_width(line: &Line<'_>) -> usize {
        line_text(line).width()
    }

    #[test]
    fn thought_chunks_coalesce() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "Thinking about " }
        })));
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "the solution." }
        })));

        let theme = Theme::default();
        let lines = preview.render(80, 10, &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "∴ Thinking about the solution.");
        assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(lines[0].spans[0].style.fg, Some(theme.gray));
    }

    #[test]
    fn message_after_thought_starts_new_entry() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "First thought" }
        })));
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "Then message" }
        })));
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "Second thought" }
        })));

        let theme = Theme::default();
        let lines = preview.render(80, 10, &theme);
        assert_eq!(lines.len(), 3);
        assert_eq!(line_text(&lines[0]), "∴ First thought");
        assert_eq!(line_text(&lines[1]), "Then message");
        assert_eq!(line_text(&lines[2]), "∴ Second thought");
    }

    #[test]
    fn tool_call_then_update_changes_status_and_title_in_place() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "read x",
            "kind": "read",
            "status": "in_progress"
        })));

        let theme = Theme::default();
        let lines_running = preview.render(80, 10, &theme);
        assert_eq!(lines_running.len(), 1);
        assert_eq!(line_text(&lines_running[0]), "▸ read x  running");

        preview.ingest(&parse_update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "t1",
            "status": "completed"
        })));

        let lines_done = preview.render(80, 10, &theme);
        assert_eq!(lines_done.len(), 1);
        assert_eq!(line_text(&lines_done[0]), "▸ read x  done");
    }

    #[test]
    fn wrapping_at_width_20_never_exceeds_20_columns_including_cjk() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {
                "type": "text",
                "text": "English words mixed with 日本語のテキストかな漢字 and more words to wrap cleanly"
            }
        })));
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "user_message_chunk",
            "content": {
                "type": "text",
                "text": "質問があります どうぞよろしくおねがいします"
            }
        })));

        let theme = Theme::default();
        let lines = preview.render(20, 50, &theme);
        assert!(lines.len() >= 4);
        for line in &lines {
            let w = line_width(line);
            assert!(
                w <= 20,
                "line {:?} had display width {} > 20",
                line_text(line),
                w
            );
        }
        // Continuation lines should be indented by 2 spaces
        assert!(line_text(&lines[1]).starts_with("  "));
    }

    #[test]
    fn hard_cut_of_long_word() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {
                "type": "text",
                "text": "abcdefghijklmnopqrstuvwxyz0123456789"
            }
        })));

        let theme = Theme::default();
        let lines = preview.render(12, 20, &theme);
        assert!(lines.len() >= 3);
        for line in &lines {
            assert!(line_width(line) <= 12);
        }
        assert_eq!(line_text(&lines[0]), "abcdefghijkl");
        assert_eq!(line_text(&lines[1]), "  mnopqrstuv");
    }

    #[test]
    fn max_rows_keeps_newest_rows() {
        let mut preview = SubagentPreview::new();
        for i in 1..=6 {
            preview.ingest(&parse_update(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": format!("t{i}"),
                "title": format!("step {i}"),
                "kind": "other",
                "status": "completed"
            })));
        }

        let theme = Theme::default();
        let lines = preview.render(80, 3, &theme);
        assert_eq!(lines.len(), 3);
        assert_eq!(line_text(&lines[0]), "▸ step 4  done");
        assert_eq!(line_text(&lines[1]), "▸ step 5  done");
        assert_eq!(line_text(&lines[2]), "▸ step 6  done");
    }

    #[test]
    fn width_zero_or_max_rows_zero_returns_empty() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello world" }
        })));

        let theme = Theme::default();
        assert!(preview.render(0, 10, &theme).is_empty());
        assert!(preview.render(80, 0, &theme).is_empty());
    }

    #[test]
    fn cap_at_400_entries_drops_oldest() {
        let mut preview = SubagentPreview::new();
        for i in 0..450 {
            preview.ingest(&parse_update(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": format!("id-{i}"),
                "title": format!("tool-{i}"),
                "kind": "other",
                "status": "pending"
            })));
        }

        assert_eq!(preview.entries.len(), 400);
        let theme = Theme::default();
        let lines = preview.render(80, 500, &theme);
        assert_eq!(lines.len(), 400);
        assert_eq!(line_text(&lines[0]), "▸ tool-50  pending");
        assert_eq!(line_text(&lines[399]), "▸ tool-449  pending");
    }

    #[test]
    fn revision_increments_only_on_effective_change() {
        let mut preview = SubagentPreview::new();
        assert_eq!(preview.revision(), 0);
        assert!(preview.is_empty());

        // Empty text chunk does not change entries
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "" }
        })));
        assert_eq!(preview.revision(), 0);

        // Non-empty chunk increments revision
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hi" }
        })));
        assert_eq!(preview.revision(), 1);
        assert!(!preview.is_empty());

        // Tool call increments revision
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "bash",
            "kind": "execute",
            "status": "in_progress"
        })));
        assert_eq!(preview.revision(), 2);

        // Identical tool call update does not increment revision
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "t1",
            "status": "in_progress"
        })));
        assert_eq!(preview.revision(), 2);

        // Status transition increments revision
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "t1",
            "status": "failed"
        })));
        assert_eq!(preview.revision(), 3);
    }

    #[test]
    fn unknown_or_unhandled_update_kind_is_ignored() {
        let mut preview = SubagentPreview::new();
        let rev_before = preview.revision();

        preview.ingest(&parse_update(json!({
            "sessionUpdate": "current_mode_update",
            "currentModeId": "code"
        })));

        assert_eq!(preview.revision(), rev_before);
        assert!(preview.is_empty());
    }

    #[test]
    fn plan_updates_single_plan_entry_and_collapses_blank_lines() {
        let mut preview = SubagentPreview::new();
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "line 1\n\n\n\nline 2   \n\n" }
        })));
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "plan",
            "entries": [
                { "content": "step a", "priority": "high", "status": "completed" },
                { "content": "step b", "priority": "medium", "status": "in_progress" },
                { "content": "step c", "priority": "low", "status": "pending" }
            ]
        })));
        preview.ingest(&parse_update(json!({
            "sessionUpdate": "plan",
            "entries": [
                { "content": "step a", "priority": "high", "status": "completed" },
                { "content": "step b", "priority": "medium", "status": "completed" },
                { "content": "step c", "priority": "low", "status": "pending" }
            ]
        })));

        let theme = Theme::default();
        let lines = preview.render(80, 10, &theme);
        assert_eq!(lines.len(), 4);
        assert_eq!(line_text(&lines[0]), "line 1");
        assert_eq!(line_text(&lines[1]), "");
        assert_eq!(line_text(&lines[2]), "  line 2");
        assert_eq!(line_text(&lines[3]), "☐ 2/3 steps");
    }
}
```