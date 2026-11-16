//! Recent page: recent sessions from the traces index, newest first. Enter opens one in the `/traces` modal.

use std::sync::mpsc;
use std::time::Duration;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::{SessionFilter, SessionRow, TraceIndex};

use super::{Ctx, Row, note_rows};
use crate::render::line_utils::truncate_str;

const LIMIT: usize = 60;
const REFRESH_MS: i64 = 5_000;
/// Widest `source` column; a longer harness name is cut.
const MAX_SOURCE: usize = 6;

/// `Err` is an index that would not open or query.
type Answer = Result<Vec<SessionRow>, ()>;

#[derive(Default)]
pub(super) struct Recent {
    /// The last answer; kept across a failed refresh so the list does not blink out.
    sessions: Option<Vec<SessionRow>>,
    failed: bool,
    /// The worker's answer, while one is out.
    pending: Option<mpsc::Receiver<Answer>>,
    started_ms: Option<i64>,
}

impl Recent {
    /// The 1 s tick while this page is showing: collects the worker's answer and starts a new query at most every 5 s.
    /// True when the rows changed.
    pub(super) fn poll(&mut self, now_ms: i64) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(rows)) => {
                    self.sessions = Some(rows);
                    self.failed = false;
                }
                Ok(Err(())) | Err(mpsc::TryRecvError::Disconnected) => self.failed = true,
                Err(mpsc::TryRecvError::Empty) => return false,
            }
            self.pending = None;
            changed = true;
        }
        if self.started_ms.is_none_or(|t| now_ms - t >= REFRESH_MS) {
            self.started_ms = Some(now_ms);
            self.pending = Some(query());
        }
        changed
    }

    pub(super) fn rows(&self, c: &Ctx) -> Vec<Row> {
        match (&self.sessions, self.failed) {
            (Some(sessions), _) => rows_from(sessions, c),
            (None, true) => note_rows("traces index unavailable", c),
            (None, false) => note_rows("loading recent sessions…", c),
        }
    }
}

/// Opens the index and lists the newest root sessions off the UI thread: opening can wait on a rebuild.
fn query() -> mpsc::Receiver<Answer> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let filter = SessionFilter { roots_only: true, hide_trivial: true, limit: LIMIT, ..SessionFilter::default() };
        let answer = TraceIndex::open_with(&xai_grok_traces::default_db_path(), Some(Duration::from_secs(2)))
            .and_then(|index| index.list_sessions(&filter))
            .map_err(|_| ());
        // A closed panel dropped the receiver; nothing to tell.
        let _ = tx.send(answer);
    });
    rx
}

/// `now`, `3m`, `2h`, `4d`.
fn age(then_ms: i64, now_ms: i64) -> String {
    let mins = now_ms.saturating_sub(then_ms).max(0) / 60_000;
    match mins {
        0 => "now".into(),
        1..=59 => format!("{mins}m"),
        60..=1439 => format!("{}h", mins / 60),
        _ => format!("{}d", mins / 1440),
    }
}

/// The index's title is whatever the harness recorded: one line of it, or the dispatch label. The folder is the
/// group heading, so it is not a fallback here.
fn title(s: &SessionRow) -> String {
    let one_line = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = one_line(&s.title);
    if !t.is_empty() {
        return t;
    }
    s.label.as_deref().map(one_line).filter(|l| !l.is_empty()).unwrap_or_else(|| "(untitled)".into())
}

/// A path cut from the left so its last folders stay readable: `…/repos/grok-build`.
fn tail(path: &str, width: usize) -> String {
    let n = path.chars().count();
    if n <= width {
        return path.to_string();
    }
    let keep: String = path.chars().skip(n + 1 - width.max(1)).collect();
    format!("…{keep}")
}

/// Sessions under their workspace folder. Workspaces come in order of their newest session (the input is newest
/// first), with this session's own folder on top.
fn rows_from(sessions: &[SessionRow], c: &Ctx) -> Vec<Row> {
    if sessions.is_empty() {
        return note_rows("no recent sessions", c);
    }
    let place = |s: &SessionRow| match s.cwd.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => crate::git_info::collapse_home(std::path::Path::new(p)),
        None => "(no folder)".to_string(),
    };
    let mut groups: Vec<(String, Vec<&SessionRow>)> = Vec::new();
    for s in sessions {
        let p = place(s);
        match groups.iter_mut().find(|(g, _)| *g == p) {
            Some((_, list)) => list.push(s),
            None => groups.push((p, vec![s])),
        }
    }
    groups.sort_by_key(|(g, _)| *g != c.info.cwd);
    let source_w = sessions.iter().map(|s| s.source.width()).max().unwrap_or(0).min(MAX_SOURCE);
    let muted = Style::default().fg(c.theme.text_secondary);
    let head = Style::default().fg(c.theme.accent_user).add_modifier(Modifier::BOLD);
    let mut out = Vec::new();
    for (place, list) in groups {
        let count = list.len().to_string();
        let name = tail(&place, c.width.saturating_sub(count.width() + 1));
        let pad = c.width.saturating_sub(name.width() + count.width());
        out.push(Row::new(
            Line::from(vec![Span::styled(name, head), Span::raw(" ".repeat(pad)), Span::styled(count, muted)]),
            place,
        ));
        for s in list {
            let when = age(s.updated_at.or(s.mtime_ms).or(s.started_at).unwrap_or(0), c.now_ms);
            let source = truncate_str(&s.source, source_w);
            let lead = format!("  {when:>3} {source:<source_w$} ");
            let title = title(s);
            let line = Line::from(vec![
                Span::styled(lead.clone(), muted),
                Span::styled(
                    truncate_str(&title, c.width.saturating_sub(lead.width())),
                    Style::default().fg(c.theme.text_primary),
                ),
            ]);
            out.push(Row::new(line, format!("{title} · {} · {when} · {}", s.source, s.id)).opens(s.id.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Info, paint};
    use super::*;
    use crate::theme::Theme;

    const NOW: i64 = 1_000_000_000;

    fn session(id: &str, source: &str, title: &str, ago_min: i64) -> SessionRow {
        SessionRow {
            cwd: Some(if source == "codex" { "/work/other-repo" } else { "/work/grok-build" }.into()),
            id: id.into(),
            source: source.into(),
            title: title.into(),
            updated_at: Some(NOW - ago_min * 60_000),
            ..SessionRow::default()
        }
    }

    fn filled() -> Vec<SessionRow> {
        vec![
            session("claude:a1", "claude", "side panel: recent page and jobs page", 0),
            session("grok:b2", "grok", "why does the dock hide finished subagents", 3),
            session("codex:c3", "codex", "fix the\nflaky   traces test", 125),
            session("pi:d4", "pi", "", 5 * 1440),
        ]
    }

    fn at(sessions: &[SessionRow], w: usize) -> Vec<Row> {
        let info = Info::default();
        rows_from(sessions, &Ctx { info: &info, theme: &Theme::current(), width: w, now_ms: NOW })
    }

    #[test]
    fn empty_43() {
        insta::assert_snapshot!(paint(&at(&[], 43), 43));
    }

    #[test]
    fn empty_21() {
        insta::assert_snapshot!(paint(&at(&[], 21), 21));
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
    fn a_row_copies_the_whole_title_and_opens_its_id() {
        let rows = at(&filled(), 21);
        assert_eq!(rows[5].copy, "fix the flaky traces test · codex · 2h · codex:c3");
        assert_eq!(rows[5].open.as_deref(), Some("codex:c3"));
        assert_eq!(rows[3].copy, "(untitled) · pi · 5d · pi:d4");
    }

    #[test]
    fn sessions_sit_under_their_workspace_and_this_one_leads() {
        let info = Info { cwd: "/work/other-repo".into(), ..Info::default() };
        let rows = rows_from(&filled(), &Ctx { info: &info, theme: &Theme::current(), width: 43, now_ms: NOW });
        let heads: Vec<&str> = rows.iter().filter(|r| r.open.is_none()).map(|r| r.copy.as_str()).collect();
        assert_eq!(heads, ["/work/other-repo", "/work/grok-build"], "this session's folder first, then by newest");
        assert_eq!(at(&filled(), 43)[0].copy, "/work/grok-build");
        assert_eq!(tail("/a/long/path/to/grok-build", 12), "…/grok-build");
    }

    #[test]
    fn the_note_rows_open_nothing() {
        let info = Info::default();
        let c = Ctx { info: &info, theme: &Theme::current(), width: 30, now_ms: NOW };
        for recent in [Recent::default(), Recent { failed: true, ..Recent::default() }] {
            let rows = recent.rows(&c);
            assert_eq!(rows.len(), 1);
            assert!(rows[0].open.is_none());
        }
        assert_eq!(Recent::default().rows(&c)[0].copy, "loading recent sessions…");
        assert_eq!(Recent { failed: true, ..Recent::default() }.rows(&c)[0].copy, "traces index unavailable");
    }

    #[test]
    fn ages_round_down_to_the_largest_unit() {
        assert_eq!(age(NOW - 59_000, NOW), "now");
        assert_eq!(age(NOW - 3 * 60_000, NOW), "3m");
        assert_eq!(age(NOW - 125 * 60_000, NOW), "2h");
        assert_eq!(age(NOW - 4 * 86_400_000, NOW), "4d");
        assert_eq!(age(NOW + 5_000, NOW), "now", "a clock skew never reads negative");
    }

    #[test]
    fn a_finished_answer_replaces_the_rows_and_a_failure_keeps_them() {
        let (tx, rx) = mpsc::channel();
        let mut r = Recent { pending: Some(rx), started_ms: Some(0), ..Recent::default() };
        assert!(!r.poll(1_000), "no answer yet: nothing changed, no second query inside 5 s");
        tx.send(Ok(filled())).unwrap();
        assert!(r.poll(2_000));
        assert_eq!(r.sessions.as_ref().map(Vec::len), Some(4));
        let (tx, rx) = mpsc::channel::<Answer>();
        r.pending = Some(rx);
        drop(tx);
        assert!(r.poll(3_000), "a worker that died is a change");
        assert!(r.failed && r.sessions.is_some(), "the last good list stays");
    }
}
