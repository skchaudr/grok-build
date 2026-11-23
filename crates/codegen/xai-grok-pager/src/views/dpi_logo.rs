//! The Durable Pi mark: a D and a pi on an 8×4 pixel grid, drawn in full blocks as 16 columns × 4 rows.
//!
//! It speaks grok-pi's welcome language: Pi's own block π (`PI_LOGO` in grok-pi's brand) at the same two columns per
//! pixel, resting in the welcome logo's gray, with the D lit in the bright text colour its shine sweeps to. Shared by
//! the F2 panel's Durable Pi header and the attach card, which set their first lines beside it with [`beside`].

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Columns the mark takes: two per pixel.
pub const WIDTH: u16 = 16;
/// Terminal rows the mark takes: one per pixel row.
pub const ROWS: usize = 4;
/// Columns in front of every line set beside the mark: the mark and a two-column gap.
pub const INDENT: usize = WIDTH as usize + GAP.len();
const GAP: &str = "  ";
/// Pixel rows, top to bottom. Columns 0-3 are the D, 4-7 Pi's π.
const GRID: [&[u8; 8]; ROWS] = [b"XX..XXX.", b"X.X.X.X.", b"X.X.XX.X", b"XX..X..X"];
const D_COLUMNS: usize = 4;

/// The four terminal rows of the mark.
pub fn lines(theme: &Theme) -> [Line<'static>; ROWS] {
    GRID.map(|row| {
        let glyphs = |columns: std::ops::Range<usize>| {
            columns
                .map(|c| if row[c] == b'X' { "\u{2588}\u{2588}" } else { "  " })
                .collect::<String>()
        };
        Line::from(vec![
            Span::styled(glyphs(0..D_COLUMNS), Style::default().fg(theme.text_primary)),
            Span::styled(glyphs(D_COLUMNS..8), Style::default().fg(theme.gray)),
        ])
    })
}

/// `content` set beside the mark: the first four lines follow a mark row, later lines the same indent, so every line
/// starts in one column. Fewer than four lines still get the whole mark. Callers fit `content` to the width less
/// [`INDENT`].
pub fn beside(theme: &Theme, content: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let rows = content.len().max(ROWS);
    let mut mark = lines(theme).into_iter();
    let mut content = content.into_iter();
    (0..rows)
        .map(|_| {
            let mut spans = match mark.next() {
                Some(row) => row.spans,
                None => vec![Span::raw(" ".repeat(usize::from(WIDTH)))],
            };
            spans.push(Span::raw(GAP));
            spans.extend(content.next().map(|line| line.spans).unwrap_or_default());
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_mark_is_sixteen_columns_by_four_rows_of_full_blocks_in_the_welcome_colours() {
        let theme = Theme::current();
        let rows = lines(&theme);
        let texts: Vec<String> = rows.iter().map(text).collect();
        assert_eq!(
            texts,
            ["████    ██████  ", "██  ██  ██  ██  ", "██  ██  ████  ██", "████    ██    ██"]
        );
        for line in &rows {
            assert_eq!(unicode_width::UnicodeWidthStr::width(text(line).as_str()), usize::from(WIDTH));
            assert_eq!(line.spans[0].style.fg, Some(theme.text_primary));
            assert_eq!(line.spans[1].style.fg, Some(theme.gray));
        }
    }

    #[test]
    fn beside_puts_every_line_in_one_column_and_always_shows_the_whole_mark() {
        let theme = Theme::current();
        let content: Vec<Line<'static>> = (1..=6).map(|i| Line::from(format!("line {i}"))).collect();
        let out: Vec<String> = beside(&theme, content).iter().map(text).collect();
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], "████    ██████    line 1");
        assert_eq!(out[3], "████    ██    ██  line 4");
        assert_eq!(out[4], format!("{}line 5", " ".repeat(INDENT)));
        let short: Vec<String> = beside(&theme, vec![Line::from("only")]).iter().map(text).collect();
        assert_eq!(short.len(), ROWS);
        assert_eq!(short[3], "████    ██    ██  ");
    }
}
