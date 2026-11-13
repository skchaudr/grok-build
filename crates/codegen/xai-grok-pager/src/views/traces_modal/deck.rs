//! Follow one agent without leaving the others.
//!
//! The stream is the in-process `Follower` (the same rows as `grok-traces follow`): the agent's text, calls
//! and its subagents, together. Under it, every other running session stays visible. Tab, or a click on that
//! strip, follows the next one on this machine. `v` opens grok's session view. A short screen keeps the stream
//! and folds the strip to one line.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::follow::short_model;
use xai_grok_traces::{SessionRow, Status, source_label};

use super::board::{self, Board};
use super::{TracesModalState, clean_name, fmt_age, now_ms};
use crate::app::app_view::InputOutcome;
use crate::input::key::key;
use crate::render::line_utils::{truncate_line, truncate_str};
use crate::theme::Theme;
use crate::views::shortcuts_bar::{HintItem, ShortcutsBar};

/// The stream keeps at least this many rows. Below that the strip gives its lines back.
const MIN_STREAM: u16 = 8;
/// Agents drawn under "Also live" before `+N more`.
const STRIP_CAP: usize = 5;

/// `(stream rows, strip rows)` inside an area that also has a header line and a footer line.
pub(super) fn bands(height: u16, others: usize) -> (u16, u16) {
    let body = height.saturating_sub(2);
    if others == 0 || body <= MIN_STREAM {
        return (body, 0);
    }
    let full = 1 + others.min(STRIP_CAP) as u16;
    if body >= MIN_STREAM + full {
        (body - full, full)
    } else if body >= MIN_STREAM + 1 {
        (body - 1, 1)
    } else {
        (body, 0)
    }
}

/// Running sessions, each parent then its running subagents, in list order.
pub(super) fn roster(
    roots: &[SessionRow],
    kids: &HashMap<String, Vec<SessionRow>>,
) -> Vec<SessionRow> {
    let mut out = Vec::new();
    for r in roots {
        if r.status != Status::Active {
            continue;
        }
        out.push(r.clone());
        for c in kids.get(&r.id).into_iter().flatten() {
            if c.status == Status::Active {
                out.push(c.clone());
            }
        }
    }
    out
}

/// Index into `agents` of the next running session on this machine. `None` when there is nowhere to go.
pub(super) fn next_local(agents: &[SessionRow], current: &str, back: bool) -> Option<usize> {
    let local: Vec<usize> = agents
        .iter()
        .enumerate()
        .filter(|(_, a)| a.host.is_none() && a.status == Status::Active)
        .map(|(i, _)| i)
        .collect();
    let n = local.len();
    if n == 0 || (n == 1 && agents[local[0]].id == current) {
        return None;
    }
    let at = match local.iter().position(|&i| agents[i].id == current) {
        Some(p) if back => (p + n - 1) % n,
        Some(p) => (p + 1) % n,
        None if back => n - 1,
        None => 0,
    };
    Some(local[at])
}

pub(super) fn handle_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.leave_follow();
            InputOutcome::Changed
        }
        KeyCode::Enter => {
            if state.follow_inspect() {
                InputOutcome::Changed
            } else {
                InputOutcome::Unchanged
            }
        }
        KeyCode::Char('o') => {
            state.follow_trajectory();
            InputOutcome::Changed
        }
        KeyCode::Char('v') => open_grok(state),
        KeyCode::Char('f') => changed(state.follow_into()),
        KeyCode::Char('p') => changed(state.follow_parent()),
        KeyCode::Tab => jump(state, false),
        KeyCode::BackTab => jump(state, true),
        code => {
            if state.follow_view.as_mut().is_some_and(|f| f.key(code)) {
                InputOutcome::Changed
            } else {
                InputOutcome::Unchanged
            }
        }
    }
}

fn changed(did: bool) -> InputOutcome {
    if did { InputOutcome::Changed } else { InputOutcome::Unchanged }
}

pub(super) fn handle_mouse(state: &mut TracesModalState, mouse: &MouseEvent) -> InputOutcome {
    match mouse.kind {
        MouseEventKind::ScrollDown => state.wheel(super::WHEEL),
        MouseEventKind::ScrollUp => state.wheel(-(super::WHEEL)),
        MouseEventKind::Down(MouseButton::Left) => click_strip(state, mouse.row),
        _ => InputOutcome::Unchanged,
    }
}

fn jump(state: &mut TracesModalState, back: bool) -> InputOutcome {
    let agents = roster(&state.shown_roots, &state.shown_children);
    let current = state
        .follow_view
        .as_ref()
        .map(|f| f.root.id.clone())
        .unwrap_or_default();
    let Some(i) = next_local(&agents, &current, back) else {
        return InputOutcome::Unchanged;
    };
    let row = agents[i].clone();
    if state.open_follow_row(row) {
        InputOutcome::Changed
    } else {
        InputOutcome::Unchanged
    }
}

fn click_strip(state: &mut TracesModalState, y: u16) -> InputOutcome {
    let Some(id) = state.board.as_ref().and_then(|b| {
        b.strip_hits
            .iter()
            .find(|(row, _)| *row == y)
            .map(|(_, id)| id.clone())
    }) else {
        return InputOutcome::Unchanged;
    };
    let agents = roster(&state.shown_roots, &state.shown_children);
    let Some(row) = agents.into_iter().find(|r| r.id == id) else {
        return InputOutcome::Unchanged;
    };
    if row.host.is_some() {
        return open_grok_row(state, row);
    }
    if state.open_follow_row(row) {
        InputOutcome::Changed
    } else {
        InputOutcome::Unchanged
    }
}

fn open_grok(state: &mut TracesModalState) -> InputOutcome {
    let Some(row) = state.follow_view.as_ref().map(|f| f.root.clone()) else {
        return InputOutcome::Unchanged;
    };
    open_grok_row(state, row)
}

fn open_grok_row(state: &mut TracesModalState, row: SessionRow) -> InputOutcome {
    match board::grok_action(row) {
        Some(action) => InputOutcome::Action(action),
        None => {
            state.set_status("o opens its events");
            InputOutcome::Changed
        }
    }
}

pub(super) fn render(buf: &mut Buffer, area: Rect, state: &mut TracesModalState) {
    let theme = Theme::current();
    let base = Style::default().bg(theme.bg_base);
    ratatui::widgets::Clear.render(area, buf);
    buf.set_style(area, base);
    if let Some(b) = state.board.as_mut() {
        b.strip_hits.clear();
    }
    if area.width < 30 || area.height < 6 {
        return;
    }
    let now = now_ms();
    let agents = roster(&state.shown_roots, &state.shown_children);
    let (title, live, source, cwd, id, model) = state
        .follow_view
        .as_ref()
        .map(|f| {
            (
                clean_name(&f.root),
                f.is_live(),
                f.root.source.clone(),
                f.root.cwd.clone(),
                f.root.id.clone(),
                f.root.model.clone(),
            )
        })
        .unwrap_or_default();
    let others: Vec<SessionRow> = agents.into_iter().filter(|r| r.id != id).collect();
    let (stream_h, strip_h) = bands(area.height, others.len());

    render_header(
        buf,
        Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), 1),
        &title,
        live,
        &source,
        model.as_deref(),
        cwd.as_deref(),
        others.len(),
        &theme,
    );
    let footer_y = area.y + area.height - 1;
    render_footer(buf, Rect::new(area.x, footer_y, area.width, 1), &theme);

    // One row of air under the header, as grok's view leaves under its title bar, when the stream can spare it.
    let air = u16::from(stream_h > MIN_STREAM);
    let stream = Rect::new(
        area.x + 2,
        area.y + 1 + air,
        area.width.saturating_sub(4),
        stream_h - air,
    );
    if let Some(f) = state.follow_view.as_mut() {
        f.render(buf, stream, state.index.as_ref(), &theme, now);
    }
    if strip_h > 0 {
        let strip = Rect::new(
            area.x + 2,
            stream.y + stream.height,
            area.width.saturating_sub(4),
            strip_h,
        );
        render_strip(buf, strip, state.board.as_mut(), &others, now, &theme);
    }
}

fn render_header(
    buf: &mut Buffer,
    area: Rect,
    title: &str,
    live: bool,
    source: &str,
    model: Option<&str>,
    cwd: Option<&str>,
    others: usize,
    theme: &Theme,
) {
    let bg = theme.bg_base;
    let (word, color) = if live {
        ("● live", theme.accent_success)
    } else {
        ("■ ended", theme.gray)
    };
    let mut meta = vec![source_label(source).to_string()];
    meta.extend(model.map(short_model));
    meta.push(board::project(cwd));
    if others > 0 {
        meta.push(format!("{others} other live"));
    }
    let meta = format!("  {}", meta.join(" · "));
    let rw = word.width() + meta.width();
    let left_w = (area.width as usize).saturating_sub(rw + 2);
    let line = Line::from(vec![
        Span::styled("traces  ", theme.dim().bg(bg)),
        Span::styled(
            truncate_str(title, left_w.saturating_sub(8)),
            Style::default()
                .fg(theme.text_primary)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    buf.set_line(area.x, area.y, &line, left_w as u16);
    if rw + 2 < area.width as usize {
        let right = Line::from(vec![
            Span::styled(word, Style::default().fg(color).bg(bg)),
            Span::styled(meta, theme.muted().bg(bg)),
        ]);
        buf.set_line(area.x + area.width - rw as u16, area.y, &right, rw as u16);
    }
}

fn render_footer(buf: &mut Buffer, area: Rect, theme: &Theme) {
    let _ = theme;
    let hints = vec![
        HintItem::new(key!(Tab), "next live"),
        HintItem::new(key!('v'), "grok view"),
        HintItem::new(key!('f'), "subagent"),
        HintItem::new(key!('p'), "parent"),
        HintItem::new(key!('l'), "open call"),
        HintItem::new(key!('e'), "error"),
        HintItem::new(key!('G'), "tail"),
        HintItem::new(key!(Esc), "back"),
    ];
    let inner = Rect {
        x: area.x + 2,
        width: area.width.saturating_sub(2),
        ..area
    };
    ShortcutsBar::new(&hints).render(inner, buf);
}

fn render_strip(
    buf: &mut Buffer,
    area: Rect,
    board: Option<&mut Board>,
    others: &[SessionRow],
    now: i64,
    theme: &Theme,
) {
    let bg = theme.bg_base;
    if area.height == 1 {
        let names = others
            .iter()
            .map(clean_name)
            .collect::<Vec<_>>()
            .join(" · ");
        let line = truncate_str(
            &format!("also live {}   {names}", others.len()),
            area.width as usize,
        );
        buf.set_line(
            area.x,
            area.y,
            &Line::from(Span::styled(line, theme.dim().bg(bg))),
            area.width,
        );
        return;
    }
    let slots = (area.height as usize).saturating_sub(1);
    let show = if others.len() <= slots {
        others.len()
    } else {
        slots.saturating_sub(1)
    };
    head(buf, area.x, area.y, area.width, others.len(), theme);
    for (i, row) in others.iter().take(show).enumerate() {
        agent_line(
            buf,
            area.x,
            area.y + 1 + i as u16,
            area.width,
            row,
            now,
            theme,
        );
    }
    if let Some(b) = board {
        for (i, row) in others.iter().take(show).enumerate() {
            b.strip_hits.push((area.y + 1 + i as u16, row.id.clone()));
        }
    }
    if others.len() > show {
        let y = area.y + 1 + show as u16;
        if y < area.y + area.height {
            let hidden = others.len() - show;
            buf.set_string(area.x, y, format!("  {hidden} more"), theme.dim().bg(bg));
        }
    }
}

fn head(buf: &mut Buffer, x: u16, y: u16, w: u16, n: usize, theme: &Theme) {
    let bg = theme.bg_base;
    let lead = format!("{} Also live", crate::glyphs::disclosure_open());
    let count = format!(" {n} ");
    let used = lead.width() + count.width();
    let rule = "─".repeat((w as usize).saturating_sub(used));
    buf.set_line(
        x,
        y,
        &Line::from(vec![
            Span::styled(lead, theme.muted().bg(bg).add_modifier(Modifier::BOLD)),
            Span::styled(count, theme.dim().bg(bg)),
            Span::styled(rule, Style::default().fg(theme.selection_border).bg(bg)),
        ]),
        w,
    );
}

fn agent_line(buf: &mut Buffer, x: u16, y: u16, w: u16, row: &SessionRow, now: i64, theme: &Theme) {
    let bg = theme.bg_base;
    buf.set_style(Rect::new(x, y, w, 1), Style::default().bg(bg));
    let frames = crate::glyphs::dot_spinner_frames();
    let glyph = frames
        .get((now / 1000) as usize % frames.len().max(1))
        .copied()
        .unwrap_or("●");
    let indent = if row.parent_id.is_some() {
        "    "
    } else {
        "  "
    };
    let mut meta = Vec::new();
    if let Some(h) = &row.host {
        meta.push(h.clone());
    }
    if row.parent_id.is_none() {
        meta.push(source_label(&row.source).to_string());
        meta.push(board::project(row.cwd.as_deref()));
    } else {
        meta.push("subagent".into());
    }
    let age = row
        .last_event_at
        .or(row.updated_at)
        .map_or("now".into(), |t| fmt_age(now - t));
    meta.push(age);
    let right = meta.join(" · ");
    let doing = row.now.as_deref().unwrap_or("").trim();
    let doing = if w >= 90 && !doing.is_empty() {
        format!("  {doing}")
    } else {
        String::new()
    };
    let room = (w as usize).saturating_sub(right.width() + 2);
    let left = truncate_line(
        Line::from(vec![
            Span::styled(
                format!("{indent}{glyph} "),
                Style::default().fg(theme.accent_success).bg(bg),
            ),
            Span::styled(
                clean_name(row),
                Style::default().fg(theme.text_primary).bg(bg),
            ),
            Span::styled(doing, theme.muted().bg(bg)),
        ]),
        room,
    );
    buf.set_line(x, y, &left, w);
    let rw = right.width() as u16;
    if rw + 2 < w {
        buf.set_span(x + w - rw, y, &Span::styled(right, theme.dim().bg(bg)), rw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, active: bool) -> SessionRow {
        SessionRow {
            id: id.into(),
            source: "claude".into(),
            title: id.into(),
            status: if active {
                Status::Active
            } else {
                Status::Completed
            },
            ..Default::default()
        }
    }

    #[test]
    fn roster_is_running_parents_then_their_running_subagents() {
        let roots = vec![row("a", false), row("b", true)];
        let mut done = row("b1", false);
        done.parent_id = Some("b".into());
        let mut live = row("b2", true);
        live.parent_id = Some("b".into());
        let kids = HashMap::from([("b".into(), vec![done, live])]);
        let ids: Vec<_> = roster(&roots, &kids).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["b", "b2"]);
    }

    #[test]
    fn tab_moves_across_local_live_agents_and_skips_other_machines() {
        let mut remote = row("air", true);
        remote.host = Some("sab-air".into());
        let agents = vec![row("a", true), remote, row("c", true)];
        assert_eq!(next_local(&agents, "a", false), Some(2));
        assert_eq!(next_local(&agents, "c", false), Some(0));
        assert_eq!(next_local(&agents, "c", true), Some(0));
        assert_eq!(
            next_local(&agents, "gone", false),
            Some(0),
            "a followed session that has ended jumps to a live one"
        );
        assert_eq!(next_local(&agents[..1], "a", false), None);
    }

    #[test]
    fn a_short_screen_keeps_the_stream_and_collapses_the_strip() {
        assert_eq!(bands(40, 3), (34, 4));
        assert_eq!(bands(20, 6), (12, 6));
        assert_eq!(bands(15, 6), (12, 1), "one line names the others");
        assert_eq!(bands(10, 4), (8, 0));
        assert_eq!(bands(40, 0), (38, 0));
    }
}
