//! Steps page: this session by turn, one line per step.

use ratatui::text::Line;

use super::{Ctx, Row};
use crate::views::traces_follow::Follow;

pub(super) fn rows(_follow: &Follow, _c: &Ctx) -> Vec<Row> {
    vec![Row::plain(Line::from("steps: not built yet"))]
}
