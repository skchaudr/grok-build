//! Agents page: background commands and subagents with state, model and elapsed time.

use ratatui::text::Line;

use super::{Ctx, Row};

pub(super) fn rows(c: &Ctx) -> Vec<Row> {
    c.info
        .jobs
        .iter()
        .chain(c.info.subagents.iter())
        .map(|(desc, meta)| Row::plain(Line::from(format!("{desc}  {meta}"))))
        .collect()
}
