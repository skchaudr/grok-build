//! Session page: everything known about this session, one row per fact.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{Ctx, Row, reset_in};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use super::Info;

pub(super) fn rows(c: &Ctx) -> Vec<Row> {
    info_lines(c.info, c.width, c.theme, c.now_ms / 1000).into_iter().map(Row::plain).collect()
}

/// `██████░░░░`, `pct` of `width` filled.
fn bar(pct: u8, width: usize) -> String {
    let filled = (pct.min(100) as usize * width + 50) / 100;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn info_lines(info: &Info, width: usize, theme: &Theme, now_secs: i64) -> Vec<Line<'static>> {
    let head = |s: &str| {
        Line::from(Span::styled(
            s.to_string(),
            Style::default()
                .fg(theme.accent_user)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let row = |k: &str, v: &str| {
        let v = truncate_str(v, width.saturating_sub(12));
        Line::from(vec![
            Span::styled(format!("  {k:<9} "), Style::default().fg(theme.gray)),
            Span::styled(v, Style::default().fg(theme.text_primary)),
        ])
    };
    let meter = |label: &str, pct: u8, tail: String| {
        // One bar width for every meter, leaving room for the longest tail (`100% resets 130h42m`).
        let bar_w = width.saturating_sub(12 + 21).clamp(4, 24);
        let color = match pct {
            90.. => theme.accent_error,
            70.. => theme.accent_plan,
            _ => theme.accent_success,
        };
        Line::from(vec![
            Span::styled(format!("  {label:<9} "), Style::default().fg(theme.gray)),
            Span::styled(bar(pct, bar_w), Style::default().fg(color)),
            Span::styled(format!(" {tail}"), Style::default().fg(theme.text_primary)),
        ])
    };
    let mut out = vec![head("Session")];
    if let Some(t) = info.title.as_deref().filter(|t| !t.is_empty()) {
        out.push(row("title", t));
    }
    out.push(row("id", info.session_id.as_deref().unwrap_or("—")));
    out.push(row("cwd", &info.cwd));
    out.push(Line::default());
    out.push(head("Model"));
    out.push(row("model", info.model.as_deref().unwrap_or("—")));
    out.push(row("effort", info.effort.as_deref().unwrap_or("—")));
    out.push(row("mode", &info.mode));
    out.push(Line::default());
    out.push(head("Context"));
    match info.context {
        Some((used, total)) if total > 0 => {
            let pct = (used.saturating_mul(100) / total).min(100) as u8;
            out.push(meter(
                "context",
                pct,
                format!("{pct}% {}/{}", tokens(used), tokens(total)),
            ));
        }
        _ => out.push(row("context", "not reported")),
    }
    for (label, key, file_pct) in [
        ("5h usage", "five_hour", info.five_hour_pct),
        ("7d usage", "seven_day", None),
    ] {
        // A window past its reset is stale: show nothing for it rather than `resets 0h00m` beside an old percent.
        let live = info
            .rate_limits
            .get(key)
            .filter(|w| w.resets_at.is_none_or(|r| r > now_secs));
        out.push(match (live, file_pct) {
            (Some(w), _) => {
                let resets = w
                    .resets_at
                    .map(|r| format!(" resets {}", reset_in(r, now_secs)))
                    .unwrap_or_default();
                meter(label, w.pct, format!("{}%{resets}", w.pct))
            }
            (None, Some(pct)) => meter(label, pct, format!("{pct}%")),
            (None, None) => row(label, "not reported"),
        });
    }
    out.push(Line::default());
    out.push(head("Runtime"));
    out.push(row("status", &info.status));
    for (label, items) in [("bg jobs", &info.jobs), ("subagents", &info.subagents)] {
        out.push(row(label, &items.len().to_string()));
        for (desc, meta) in items {
            let meta = truncate_str(meta, width / 3);
            let desc = truncate_str(desc, width.saturating_sub(meta.width() + 6));
            out.push(Line::from(vec![
                Span::styled(
                    format!("    · {desc}"),
                    Style::default().fg(theme.text_primary),
                ),
                Span::styled(format!("  {meta}"), Style::default().fg(theme.gray)),
            ]));
        }
    }
    out
}

