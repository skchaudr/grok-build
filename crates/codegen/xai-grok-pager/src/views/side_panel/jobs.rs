//! Agents page: background commands and subagents, one row each with state, model and elapsed time.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{Ctx, Row};
use crate::render::line_utils::truncate_str;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// A background command or a subagent, as the Agents page lists it.
#[derive(Clone, Debug)]
pub struct Job {
    /// Subagent type (`Explore`, ...); empty for a background command.
    pub kind: String,
    pub description: String,
    pub state: JobState,
    pub model: Option<String>,
    /// `2m14s`, formatted by the glue.
    pub elapsed: String,
}

impl JobState {
    fn word(self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    fn mark(self) -> &'static str {
        match self {
            JobState::Running => "●",
            JobState::Done => "✓",
            JobState::Failed => "✗",
            JobState::Cancelled => "■",
        }
    }
}

impl Job {
    fn label(&self) -> String {
        if self.kind.is_empty() {
            self.description.clone()
        } else {
            format!("{}: {}", self.kind, self.description)
        }
    }

    /// The `y` text: everything, untruncated.
    fn copy(&self) -> String {
        let model = self.model.as_deref().unwrap_or_default();
        let parts = [self.label(), self.state.word().into(), model.into(), self.elapsed.clone()];
        parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · ")
    }

    /// Level 0 is `running grok-4.5 2m14s`, 1 drops the model, 2 is the elapsed time alone (the mark still says the state).
    fn meta(&self, level: usize) -> String {
        let model = self.model.as_deref().unwrap_or_default();
        let parts: Vec<&str> = match level {
            0 => vec![self.state.word(), model, &self.elapsed],
            1 => vec![self.state.word(), &self.elapsed],
            _ => vec![&self.elapsed],
        };
        parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }

    /// `meta` is right-aligned in a `col`-wide column, so every row ends at the row width and the metas start together.
    fn row(&self, c: &Ctx, level: usize, col: usize) -> Row {
        let color = match self.state {
            JobState::Running => c.theme.accent_user,
            JobState::Done => c.theme.accent_success,
            JobState::Failed => c.theme.accent_error,
            JobState::Cancelled => c.theme.text_secondary,
        };
        let meta = format!("{:>col$}", self.meta(level));
        // mark + space, the label, two spaces, then the meta column.
        let label = truncate_str(&self.label(), c.width.saturating_sub(col + 4));
        let pad = c.width.saturating_sub(2 + label.width() + col).max(2);
        let line = Line::from(vec![
            Span::styled(format!("{} ", self.state.mark()), Style::default().fg(color)),
            Span::styled(label, Style::default().fg(c.theme.text_primary)),
            Span::raw(" ".repeat(pad)),
            Span::styled(meta, Style::default().fg(c.theme.text_secondary)),
        ]);
        Row::new(line, self.copy())
    }
}

/// The first meta level whose widest meta takes at most three fifths of the row; every row uses it so the column lines up.
fn meta_level(items: &[&Job], width: usize) -> (usize, usize) {
    let col = |level| items.iter().map(|j| j.meta(level).width()).max().unwrap_or(0);
    let level = (0..2).find(|l| col(*l) * 5 <= width * 3).unwrap_or(2);
    (level, col(level))
}

fn section(title: &str, items: &[Job], c: &Ctx, (level, col): (usize, usize), out: &mut Vec<Row>) {
    let head = Style::default().fg(c.theme.accent_user).add_modifier(Modifier::BOLD);
    out.push(Row::plain(Line::from(Span::styled(format!("{title} ({})", items.len()), head))));
    if items.is_empty() {
        out.push(Row::new(
            Line::from(Span::styled("  none", Style::default().fg(c.theme.text_secondary))),
            format!("{title}: none"),
        ));
    }
    out.extend(items.iter().map(|j| j.row(c, level, col)));
}

pub(super) fn rows(c: &Ctx) -> Vec<Row> {
    let mut out = Vec::new();
    let all: Vec<&Job> = c.info.jobs.iter().chain(&c.info.subagents).collect();
    let fit = meta_level(&all, c.width);
    section("Background jobs", &c.info.jobs, c, fit, &mut out);
    section("Subagents", &c.info.subagents, c, fit, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Info, paint};
    use super::*;
    use crate::theme::Theme;

    fn job(kind: &str, desc: &str, state: JobState, model: Option<&str>, elapsed: &str) -> Job {
        Job {
            kind: kind.into(),
            description: desc.into(),
            state,
            model: model.map(Into::into),
            elapsed: elapsed.into(),
        }
    }

    fn filled() -> Info {
        Info {
            jobs: vec![
                job("", "cargo test --release -p xai-grok-pager", JobState::Running, None, "2m14s"),
                job("", "npm run build", JobState::Failed, None, "41s"),
            ],
            subagents: vec![
                job("Explore", "find where the dock computes its rows", JobState::Running, Some("grok-4.5"), "1m05s"),
                job("Reviewer", "review the side panel diff", JobState::Done, Some("claude-sonnet-5.5"), "6m30s"),
                job("Explore", "scan docs", JobState::Cancelled, None, "9s"),
            ],
            ..Info::default()
        }
    }

    fn at(info: &Info, w: usize) -> Vec<Row> {
        rows(&Ctx { info, theme: &Theme::current(), width: w, now_ms: 0 })
    }

    #[test]
    fn empty_43() {
        insta::assert_snapshot!(paint(&at(&Info::default(), 43), 43));
    }

    #[test]
    fn empty_21() {
        insta::assert_snapshot!(paint(&at(&Info::default(), 21), 21));
    }

    #[test]
    fn filled_43() {
        insta::assert_snapshot!(paint(&at(&filled(), 43), 43));
    }

    #[test]
    fn filled_21() {
        insta::assert_snapshot!(paint(&at(&filled(), 21), 21));
    }

    #[test]
    fn copy_is_the_whole_untruncated_row() {
        let info = filled();
        let rows = at(&info, 21);
        assert_eq!(rows[1].copy, "cargo test --release -p xai-grok-pager · running · 2m14s");
        assert_eq!(rows[4].copy, "Explore: find where the dock computes its rows · running · grok-4.5 · 1m05s");
        assert_eq!(rows.iter().filter(|r| r.open.is_some()).count(), 0);
    }

    #[test]
    fn meta_sheds_the_model_then_the_word_to_fit() {
        let j = job("Explore", "x", JobState::Running, Some("grok-4.5"), "1m05s");
        assert_eq!(j.meta(0), "running grok-4.5 1m05s");
        assert_eq!(j.meta(1), "running 1m05s");
        assert_eq!(j.meta(2), "1m05s");
        assert_eq!(meta_level(&[&j], 43), (0, 22));
        assert_eq!(meta_level(&[&j], 30), (1, 13));
        assert_eq!(meta_level(&[&j], 21), (2, 5));
    }

    #[test]
    fn every_row_ends_at_the_row_width_and_the_metas_start_together() {
        let info = filled();
        let rows = at(&info, 43);
        let w = |r: &Row| r.line.spans.iter().map(|s| s.content.width()).sum::<usize>();
        let meta_at = |r: &Row| w(r) - r.line.spans.last().unwrap().content.width();
        let listed: Vec<&Row> = rows.iter().filter(|r| r.line.spans.len() == 4).collect();
        assert_eq!(listed.len(), 5);
        assert!(listed.iter().all(|r| w(r) == 43));
        assert!(listed.iter().all(|r| meta_at(r) == meta_at(listed[0])));
    }
}
