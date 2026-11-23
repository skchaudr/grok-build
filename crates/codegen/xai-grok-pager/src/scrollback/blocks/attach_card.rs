//! AttachCardBlock — the dpi welcome: what a durable agent is doing, shown once each time grok-pi attaches to it.
//!
//! dpi-grok attaches the pager to an agent that already runs on a dpi backend (often another machine), so the
//! local welcome screen (laptop folder, local tips) describes the wrong place. The dpi-rpc shim builds this card
//! from the backend (`pi/ui/attach_card`: a title and label/value rows) and the adapter sends it after the replayed
//! history, so it sits at the bottom of the scrollback right above the prompt. It is a block, not a modal: a
//! `--resume` session opens straight into it, and ctrl+r and quit keep their usual meaning.
//!
//! The title, the agent's browser playground and the rows share one column beside the Durable Pi mark, and a muted
//! line, wrapped to the width, ends the card with the keys a newcomer needs.

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
    /// `(label, value)` rows in the shim's order: model, cwd, activity, last ask, tasks.
    pub rows: Vec<(String, String)>,
    /// The agent in the browser playground, when the backend has one.
    pub playground_url: Option<String>,
}

/// The card's last line: what F2, ctrl+r and quitting do here. A no-break space keeps each dot with the hint before
/// it when the line wraps.
const HINTS: &str = "F2 settings\u{00A0}\u{00B7} ctrl+r switch agents\u{00A0}\u{00B7} quitting detaches, the agent keeps running\u{00A0}\u{00B7} /dpi-help";
/// Narrower than this, the mark gives its columns back to the card.
const LOGO_MIN_WIDTH: usize = 40;

impl AttachCardBlock {
    pub fn new(title: impl Into<String>, rows: Vec<(String, String)>) -> Self {
        Self {
            title: title.into(),
            rows,
            playground_url: None,
        }
    }

    /// The `pi/ui/attach_card` payload: `{title, rows: [[label, value], ...], playgroundUrl?}`. Malformed rows are
    /// skipped.
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
        let playground_url = params
            .get("playgroundUrl")
            .and_then(serde_json::Value::as_str)
            .map(clean)
            .filter(|url| !url.is_empty());
        Some(Self {
            playground_url,
            ..Self::new(clean(title), rows)
        })
    }

    pub fn searchable_text(&self) -> String {
        std::iter::once(self.title.clone())
            .chain(self.playground_url.clone())
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
        use crate::views::dpi_logo;
        let theme = Theme::current();
        let width = ctx.width as usize;
        let with_mark = width >= LOGO_MIN_WIDTH;
        // Everything sits in one column beside the mark: title, playground, then the rows.
        let budget = if with_mark { width - dpi_logo::INDENT } else { width };
        let fit = |text: &str| crate::util::truncate_to_width(text, budget).into_owned();
        let mut content = vec![Line::from(Span::styled(
            fit(&self.title),
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD),
        ))];
        if let Some(url) = &self.playground_url {
            content.push(Line::from(Span::styled(
                fit(url),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::UNDERLINED),
            )));
        }
        // Labels share one column so the values line up; the column is only as wide as the longest label.
        let label_width = self.rows.iter().map(|(l, _)| l.width()).max().unwrap_or(0) + 2;
        for (label, value) in &self.rows {
            let pad = label_width.saturating_sub(label.width());
            let value = crate::util::truncate_to_width(value, budget.saturating_sub(label_width)).into_owned();
            content.push(Line::from(vec![
                Span::styled(format!("{label}{}", " ".repeat(pad)), theme.muted()),
                Span::styled(value, Style::default().fg(theme.text_primary)),
            ]));
        }
        let content = if with_mark { dpi_logo::beside(&theme, content) } else { content };
        let mut lines: Vec<BlockLine> = content
            .into_iter()
            .map(|line| BlockLine::styled(line).with_selection_range(Some(0)))
            .collect();
        lines.push(BlockLine::separator(Line::from("")));
        // The hints wrap rather than lose their tail on a phone-width terminal.
        let hints = Line::from(Span::styled(HINTS, theme.muted()));
        let indent = Line::from(" ".repeat(width - budget));
        let options = crate::render::wrapping::RtOptions::new(width)
            .initial_indent(indent.clone())
            .subsequent_indent(indent)
            .word_separator(textwrap::WordSeparator::AsciiSpace)
            .word_splitter(textwrap::WordSplitter::NoHyphenation);
        lines.extend(
            crate::render::wrapping::word_wrap_lines([hints], options)
                .into_iter()
                .map(|line| BlockLine::styled(line).with_selection_range(Some(0))),
        );
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
                ["tasks", "none live"],
            ],
            "playgroundUrl": "http://mini:5173/s/polish-test",
        }))
        .unwrap()
    }

    #[test]
    fn from_json_keeps_well_formed_rows_on_one_line() {
        let card = card();
        assert_eq!(card.title, "dpi · polish-test on mini");
        let labels: Vec<_> = card.rows.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["model", "cwd", "last ask", "tasks"]);
        assert_eq!(card.rows[2].1, "fix the");
        assert_eq!(card.playground_url.as_deref(), Some("http://mini:5173/s/polish-test"));
        let bare = AttachCardBlock::from_json(&serde_json::json!({ "title": "t", "playgroundUrl": null })).unwrap();
        assert_eq!(bare.playground_url, None);
        assert!(AttachCardBlock::from_json(&serde_json::json!({ "rows": [] })).is_none());
        assert!(AttachCardBlock::from_json(&serde_json::json!({ "title": " " })).is_none());
    }

    #[test]
    fn output_is_one_column_beside_the_mark_then_the_hints() {
        let output = card().output(&ctx(120));
        // title, playground, 4 rows (the first two beside the mark), gap, hints on one line
        assert_eq!(output.lines.len(), 6 + 1 + 1);
        let pad = " ".repeat(crate::views::dpi_logo::INDENT);
        assert_eq!(text(&output.lines[0]), "████    ██████    dpi · polish-test on mini");
        assert_eq!(text(&output.lines[1]), "██  ██  ██  ██    http://mini:5173/s/polish-test");
        let title = output.lines[0].content.spans.last().unwrap();
        assert!(title.style.add_modifier.contains(Modifier::BOLD));
        // "last ask" is the longest label, so every value starts 10 columns into the card's column.
        assert_eq!(text(&output.lines[2]), "██  ██  ████  ██  model     gpt-6.1-sol (cliproxy) · thinking medium");
        assert_eq!(text(&output.lines[3]), "████    ██    ██  cwd       mini:~/repos/lab · main");
        assert_eq!(text(&output.lines[5]), format!("{pad}tasks     none live"));
        assert_eq!(
            text(&output.lines[7]).replace('\u{00A0}', " "),
            format!("{pad}F2 settings · ctrl+r switch agents · quitting detaches, the agent keeps running · /dpi-help")
        );
    }

    #[test]
    fn without_a_playground_the_rows_move_up_beside_the_mark() {
        let mut card = card();
        card.playground_url = None;
        let output = card.output(&ctx(120));
        assert_eq!(text(&output.lines[1]), "██  ██  ██  ██    model     gpt-6.1-sol (cliproxy) · thinking medium");
    }

    #[test]
    fn a_phone_width_wraps_the_hints_whole_and_never_overflows() {
        for width in [30, 44, 75] {
            let output = card().output(&ctx(width));
            for line in &output.lines {
                assert!(text(line).width() <= usize::from(width), "{:?}", text(line));
            }
            let hints: String = output
                .lines
                .iter()
                .skip_while(|line| !text(line).trim().is_empty())
                .skip(1)
                .map(|line| text(line).trim().to_owned())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(hints.replace('\u{00A0}', " ").ends_with("the agent keeps running · /dpi-help"), "{width}: {hints}");
            assert!(!hints.contains('\u{2026}'), "{width}: {hints}");
        }
        // Too narrow for the mark: the title gets the whole line.
        assert!(text(&card().output(&ctx(30)).lines[0]).starts_with("dpi · polish"));
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
