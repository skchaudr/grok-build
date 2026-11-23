//! AttachCardBlock — the dpi welcome: what a durable agent is doing, shown once each time grok-pi attaches to it.
//!
//! dpi-grok attaches the pager to an agent that already runs on a dpi backend (often another machine), so the
//! local welcome screen (laptop folder, local tips) describes the wrong place. The dpi-rpc shim builds this card
//! from the backend (`pi/ui/attach_card`: a title and label/value rows) and the adapter sends it after the replayed
//! history, so it sits at the bottom of the scrollback right above the prompt. It is a block, not a modal: a
//! `--resume` session opens straight into it, and ctrl+r and quit keep their usual meaning.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::appearance::AppearanceConfig;
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{AccentStyle, BlockContext, BlockLine, BlockOutput, DisplayMode};
use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct AttachCardBlock {
    /// e.g. `dpi · polish-test on mini`.
    pub title: String,
    /// `(label, value)` rows in the shim's order: model, cwd, activity, last ask, tasks, keys.
    pub rows: Vec<(String, String)>,
}

impl AttachCardBlock {
    pub fn new(title: impl Into<String>, rows: Vec<(String, String)>) -> Self {
        Self {
            title: title.into(),
            rows,
        }
    }

    /// The `pi/ui/attach_card` payload: `{title, rows: [[label, value], ...]}`. Malformed rows are skipped.
    pub fn from_json(params: &serde_json::Value) -> Option<Self> {
        let title = params.get("title")?.as_str()?.trim();
        if title.is_empty() {
            return None;
        }
        let rows = params
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let row = row.as_array()?;
                let label = row.first()?.as_str()?;
                let value = row.get(1)?.as_str()?;
                Some((clean(label), clean(value)))
            })
            .collect();
        Some(Self::new(clean(title), rows))
    }

    pub fn searchable_text(&self) -> String {
        std::iter::once(self.title.clone())
            .chain(self.rows.iter().map(|(label, value)| format!("{label} {value}")))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Backend strings reach the terminal: one line, no control characters.
fn clean(text: &str) -> String {
    let first_line = text.trim().lines().next().unwrap_or_default();
    crate::views::session_title::sanitize_display_text(first_line.trim()).into_owned()
}

impl BlockContent for AttachCardBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        let width = ctx.width as usize;
        let title = Line::from(Span::styled(
            crate::util::truncate_to_width(&self.title, width).into_owned(),
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD),
        ));
        let mut lines = vec![
            BlockLine::styled(title).with_selection_range(Some(0)),
            BlockLine::separator(Line::from("")),
        ];
        // Labels share one column so the values line up; the column is only as wide as the longest label.
        let label_width = self.rows.iter().map(|(l, _)| l.width()).max().unwrap_or(0) + 2;
        for (label, value) in &self.rows {
            let pad = label_width.saturating_sub(label.width());
            let value_budget = width.saturating_sub(label_width);
            let value = crate::util::truncate_to_width(value, value_budget).into_owned();
            let line = Line::from(vec![
                Span::styled(format!("{label}{}", " ".repeat(pad)), theme.muted()),
                Span::styled(value, Style::default().fg(theme.text_primary)),
            ]);
            lines.push(BlockLine::styled(line).with_selection_range(Some(0)));
        }
        BlockOutput { lines }
    }

    fn accent(&self, _ctx: &BlockContext) -> Option<AccentStyle> {
        Some(AccentStyle::static_color(Theme::current().accent_assistant))
    }

    fn has_vpad_for(&self, _appearance: &AppearanceConfig) -> bool {
        true
    }

    fn has_raw_mode(&self) -> bool {
        false
    }

    fn is_foldable(&self) -> bool {
        false
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Expanded
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn is_groupable(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(width: u16) -> BlockContext {
        BlockContext {
            mode: DisplayMode::Expanded,
            is_running: false,
            width,
            raw: false,
            max_lines: None,
            appearance: AppearanceConfig::default(),
            is_selected: false,
            cwd: None,
        }
    }

    fn text(line: &BlockLine) -> String {
        line.content.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn card() -> AttachCardBlock {
        AttachCardBlock::from_json(&serde_json::json!({
            "title": "dpi · polish-test on mini",
            "rows": [
                ["model", "gpt-6.1-sol (cliproxy) · thinking medium"],
                ["cwd", "mini:~/repos/lab · main"],
                ["last ask", "fix the\nheader"],
                ["broken"],
                ["keys", "ctrl+r switches agents · quitting detaches; the agent keeps running"],
            ],
        }))
        .unwrap()
    }

    #[test]
    fn from_json_keeps_well_formed_rows_on_one_line() {
        let card = card();
        assert_eq!(card.title, "dpi · polish-test on mini");
        let labels: Vec<_> = card.rows.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["model", "cwd", "last ask", "keys"]);
        assert_eq!(card.rows[2].1, "fix the");
        assert!(AttachCardBlock::from_json(&serde_json::json!({ "rows": [] })).is_none());
        assert!(AttachCardBlock::from_json(&serde_json::json!({ "title": " " })).is_none());
    }

    #[test]
    fn output_is_a_bold_title_then_aligned_rows() {
        let output = card().output(&ctx(100));
        assert_eq!(output.lines.len(), 2 + 4);
        assert_eq!(text(&output.lines[0]), "dpi · polish-test on mini");
        assert!(output.lines[0].content.spans[0].style.add_modifier.contains(Modifier::BOLD));
        // "last ask" is the longest label, so every value starts in column 10.
        assert_eq!(text(&output.lines[2]), "model     gpt-6.1-sol (cliproxy) · thinking medium");
        assert_eq!(text(&output.lines[3]), "cwd       mini:~/repos/lab · main");
        assert!(text(&output.lines[5]).starts_with("keys      ctrl+r switches agents"));
    }

    #[test]
    fn narrow_widths_truncate_values_instead_of_overflowing() {
        let output = card().output(&ctx(30));
        for line in &output.lines {
            assert!(text(line).width() <= 30, "{:?}", text(line));
        }
    }

    #[test]
    fn block_contract_and_search() {
        let card = card();
        assert!(!card.is_foldable());
        assert!(card.is_selectable());
        assert!(!card.is_groupable());
        assert!(card.has_vpad(&ctx(80)));
        assert_eq!(card.accent(&ctx(80)).unwrap().color, Theme::current().accent_assistant);
        assert!(card.searchable_text().contains("cwd mini:~/repos/lab"));
    }
}
