//! The Durable Pi mark: a D and a pi on an 8×4 pixel grid, drawn with half blocks as 16 columns × 2 rows.
//!
//! Shared by the F2 panel's Durable Pi header and the attach card. Colours come from the theme: the D in the
//! primary text colour, the pi in the assistant accent.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Columns the mark takes: two per pixel.
pub const WIDTH: u16 = 16;
/// Pixel rows, top to bottom. Columns 0-3 are the D, 4-7 the pi.
const GRID: [&[u8; 8]; 4] = [b"XX..XXX.", b"X.X.X.X.", b"X.X.XX.X", b"XX..X..X"];
const D_COLUMNS: usize = 4;

/// The two terminal rows of the mark. Each row folds two pixel rows into one half-block glyph per pixel.
pub fn lines(theme: &Theme) -> [Line<'static>; 2] {
    let row = |top: &[u8; 8], bottom: &[u8; 8]| {
        let glyphs = |columns: std::ops::Range<usize>| {
            columns
                .map(|c| match (top[c] == b'X', bottom[c] == b'X') {
                    (true, true) => "\u{2588}\u{2588}",
                    (true, false) => "\u{2580}\u{2580}",
                    (false, true) => "\u{2584}\u{2584}",
                    (false, false) => "  ",
                })
                .collect::<String>()
        };
        Line::from(vec![
            Span::styled(glyphs(0..D_COLUMNS), Style::default().fg(theme.text_primary)),
            Span::styled(glyphs(D_COLUMNS..8), Style::default().fg(theme.accent_assistant)),
        ])
    };
    [row(GRID[0], GRID[1]), row(GRID[2], GRID[3])]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_mark_is_sixteen_columns_of_half_blocks_in_theme_colours() {
        let theme = Theme::current();
        let [top, bottom] = lines(&theme);
        assert_eq!(text(&top), "██▀▀▄▄  ██▀▀██  ");
        assert_eq!(text(&bottom), "██▄▄▀▀  ██▀▀  ██");
        for line in [&top, &bottom] {
            assert_eq!(unicode_width::UnicodeWidthStr::width(text(line).as_str()), usize::from(WIDTH));
            assert_eq!(line.spans[0].style.fg, Some(theme.text_primary));
            assert_eq!(line.spans[1].style.fg, Some(theme.accent_assistant));
        }
    }
}
