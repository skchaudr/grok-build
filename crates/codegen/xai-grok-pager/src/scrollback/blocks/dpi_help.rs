//! DpiHelpBlock — the /dpi-help guide for a durable agent (`pi/ui/dpi_help`).
//!
//! The dpi-rpc shim sends the guide as data (an intro, titled sections of key/description rows, the playground) so
//! it can be drawn like a card: headings in the accent colour and key names in bold, rather than a notify line that
//! renders all in muted grey. Each section lines its descriptions up after its widest key; on a terminal too narrow
//! for that, a description goes under its key.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::appearance::AppearanceConfig;
use crate::render::wrapping::{RtOptions, word_wrap_lines};
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{AccentStyle, BlockContext, BlockLine, BlockOutput, DisplayMode};
use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct DpiHelpSection {
    pub title: String,
    /// `(key, description)`, e.g. `("F2", "settings …")` or `("/dpi-archive", "…")`.
    pub rows: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct DpiHelpBlock {
    pub intro: String,
    pub sections: Vec<DpiHelpSection>,
    pub playground_url: Option<String>,
}

/// Rows are indented this much under their heading.
const ROW_INDENT: usize = 2;
/// Below this many columns for a description, it goes on its own line under the key.
const MIN_DESCRIPTION_W: usize = 24;

impl DpiHelpBlock {
    /// The `pi/ui/dpi_help` payload: `{intro, sections: [{title, rows: [[key, description], ...]}], playgroundUrl?}`.
    /// Malformed rows and empty sections are skipped; a payload with nothing to show is `None`.
    pub fn from_json(params: &serde_json::Value) -> Option<Self> {
        let text = |value: Option<&serde_json::Value>| value.and_then(serde_json::Value::as_str).map(clean);
        let sections: Vec<DpiHelpSection> = params
            .get("sections")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|section| {
                let rows: Vec<(String, String)> = section
                    .get("rows")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|row| {
                        let row = row.as_array()?;
                        Some((text(row.first())?, text(row.get(1))?)).filter(|(key, _)| !key.is_empty())
                    })
                    .collect();
                let title = text(section.get("title")).unwrap_or_default();
                (!rows.is_empty()).then_some(DpiHelpSection { title, rows })
            })
            .collect();
        let intro = text(params.get("intro")).unwrap_or_default();
        if intro.is_empty() && sections.is_empty() {
            return None;
        }
        Some(Self {
            intro,
            sections,
            playground_url: text(params.get("playgroundUrl")).filter(|url| !url.is_empty()),
        })
    }

    pub fn searchable_text(&self) -> String {
        std::iter::once(self.intro.clone())
            .chain(self.sections.iter().flat_map(|section| {
                std::iter::once(section.title.clone())
                    .chain(section.rows.iter().map(|(key, description)| format!("{key} {description}")))
            }))
            .chain(self.playground_url.iter().map(|url| format!("Playground: {url}")))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Shim strings reach the terminal: one line, no control characters.
fn clean(text: &str) -> String {
    let first_line = text.trim().lines().next().unwrap_or_default();
    crate::views::session_title::sanitize_display_text(first_line.trim()).into_owned()
}

/// `line` wrapped to `width`, starting `first` columns in and continuing `rest` columns in.
fn wrap(line: Line<'static>, width: usize, first: usize, rest: usize) -> Vec<Line<'static>> {
    let options = RtOptions::new(width.max(1))
        .initial_indent(Line::from(" ".repeat(first)))
        .subsequent_indent(Line::from(" ".repeat(rest)))
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);
    word_wrap_lines([line], options)
}

impl BlockContent for DpiHelpBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        let width = usize::from(ctx.width).max(1);
        let primary = Style::default().fg(theme.text_primary);
        let key_style = primary.add_modifier(Modifier::BOLD);
        let mut lines: Vec<Line<'static>> = vec![];
        if !self.intro.is_empty() {
            lines.extend(wrap(Line::from(Span::styled(self.intro.clone(), primary)), width, 0, 0));
        }
        for section in &self.sections {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            if !section.title.is_empty() {
                lines.push(Line::from(Span::styled(
                    section.title.clone(),
                    Style::default()
                        .fg(theme.accent_assistant)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            let key_w = section.rows.iter().map(|(key, _)| key.width()).max().unwrap_or(0) + 2;
            let stacked = width < ROW_INDENT + key_w + MIN_DESCRIPTION_W;
            for (key, description) in &section.rows {
                if stacked {
                    let key = Line::from(Span::styled(key.clone(), key_style));
                    lines.extend(wrap(key, width, ROW_INDENT, ROW_INDENT));
                    let text = Line::from(Span::styled(description.clone(), primary));
                    lines.extend(wrap(text, width, ROW_INDENT * 2, ROW_INDENT * 2));
                } else {
                    let row = Line::from(vec![
                        Span::styled(key.clone(), key_style),
                        Span::raw(" ".repeat(key_w - key.width())),
                        Span::styled(description.clone(), primary),
                    ]);
                    lines.extend(wrap(row, width, ROW_INDENT, ROW_INDENT + key_w));
                }
            }
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        let playground = match &self.playground_url {
            Some(url) => Line::from(vec![
                Span::styled("Playground ", theme.muted()),
                Span::styled(url.clone(), primary.add_modifier(Modifier::UNDERLINED)),
            ]),
            None => Line::from(Span::styled("Playground: none for this backend", theme.muted())),
        };
        lines.extend(wrap(playground, width, 0, 0));
        BlockOutput {
            lines: lines
                .into_iter()
                .map(|line| BlockLine::styled(line).with_selection_range(Some(0)))
                .collect(),
        }
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

    fn help() -> DpiHelpBlock {
        DpiHelpBlock::from_json(&serde_json::json!({
            "intro": "Durable Pi: your agent lives on mini.",
            "sections": [
                {"title": "Keys", "rows": [["F2", "settings"], ["ctrl+r", "switch session or agent"], ["bad"]]},
                {"title": "Commands", "rows": [["/dpi-archive", "archive this session\nand detach"]]},
                {"title": "Empty", "rows": []},
            ],
            "playgroundUrl": "http://hub:4781",
        }))
        .unwrap()
    }

    #[test]
    fn from_json_keeps_well_formed_rows_and_drops_empty_sections() {
        let help = help();
        assert_eq!(help.sections.len(), 2);
        assert_eq!(help.sections[0].rows.len(), 2);
        assert_eq!(help.sections[1].rows[0].1, "archive this session");
        assert_eq!(help.playground_url.as_deref(), Some("http://hub:4781"));
        assert!(DpiHelpBlock::from_json(&serde_json::json!({ "sections": [] })).is_none());
        let bare = DpiHelpBlock::from_json(&serde_json::json!({ "intro": "hi", "playgroundUrl": null })).unwrap();
        assert_eq!(bare.playground_url, None);
        assert!(help.searchable_text().contains("/dpi-archive archive this session"));
    }

    #[test]
    fn headings_are_accent_bold_and_keys_line_up_in_bold() {
        let theme = Theme::current();
        let output = help().output(&ctx(100));
        let lines: Vec<String> = output.lines.iter().map(text).collect();
        assert_eq!(
            lines,
            [
                "Durable Pi: your agent lives on mini.",
                "",
                "Keys",
                "  F2      settings",
                "  ctrl+r  switch session or agent",
                "",
                "Commands",
                "  /dpi-archive  archive this session",
                "",
                "Playground http://hub:4781",
            ]
        );
        let heading = &output.lines[2].content.spans[0];
        assert_eq!(heading.style.fg, Some(theme.accent_assistant));
        assert!(heading.style.add_modifier.contains(Modifier::BOLD));
        let span = |row: usize, content: &str| {
            output.lines[row].content.spans.iter().find(|s| s.content.trim() == content).unwrap().style
        };
        assert!(span(3, "F2").add_modifier.contains(Modifier::BOLD));
        assert_eq!(span(3, "F2").fg, Some(theme.text_primary));
        assert_eq!(span(3, "settings").fg, Some(theme.text_primary));
        assert!(!span(3, "settings").add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn long_descriptions_hang_under_their_column_and_narrow_widths_stack() {
        let mut help = help();
        help.sections[0].rows[0].1 = "settings for model, thinking, read-only and more".into();
        let output = help.output(&ctx(40));
        let lines: Vec<String> = output.lines.iter().map(text).collect();
        assert_eq!(lines[3], "  F2      settings for model, thinking,");
        assert_eq!(lines[4], "          read-only and more");
        for width in [20, 30, 40] {
            for line in &help.output(&ctx(width)).lines {
                assert!(text(line).width() <= usize::from(width), "{width}: {:?}", text(line));
            }
        }
        // "/dpi-archive" leaves too little room at 30 columns: the description goes under it.
        let narrow: Vec<String> = help.output(&ctx(30)).lines.iter().map(text).collect();
        let at = narrow.iter().position(|line| line == "  /dpi-archive").unwrap();
        assert_eq!(narrow[at + 1], "    archive this session");
    }

    #[test]
    fn without_a_playground_the_footer_says_so() {
        let mut help = help();
        help.playground_url = None;
        let output = help.output(&ctx(80));
        assert_eq!(text(output.lines.last().unwrap()), "Playground: none for this backend");
    }
}
