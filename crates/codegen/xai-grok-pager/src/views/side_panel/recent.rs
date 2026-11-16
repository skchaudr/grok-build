//! Recent page: recent sessions from the traces index, newest first.

use ratatui::text::Line;

use super::{Ctx, Row};

#[derive(Default)]
pub(super) struct Recent;

impl Recent {
    /// The 1 s tick while this page is showing. True when it should redraw.
    pub(super) fn poll(&mut self, _now_ms: i64) -> bool {
        false
    }

    pub(super) fn rows(&self, _c: &Ctx) -> Vec<Row> {
        vec![Row::plain(Line::from("recent: not built yet"))]
    }
}
