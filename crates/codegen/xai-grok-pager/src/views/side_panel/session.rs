//! Session page: everything known about this session, one row per fact, under bold headings.
//! `y` copies a row's full `key: value`; the drawn line is cut with `…` to the panel width.

use chrono::TimeZone;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use xai_grok_traces::SessionRow;

use super::{Ctx, Info, Row, SidePanel, Traces, reset_in};
use crate::render::line_utils::truncate_str;
use crate::views::traces_modal::fmt_age;

/// Columns of a row's `  key ` prefix (two indent, twelve for the longest key `cache write`, one gap).
const KEY: usize = 12;
const PREFIX: usize = KEY + 3;
const MISSING: &str = "—";

impl SidePanel {
    /// The trace-index row of this session, once the panel has found it. Lives here so the Session page can read
    /// the panel's private traces state.
    pub(crate) fn session_row(&self) -> Option<&SessionRow> {
        match &self.traces {
            Traces::Following(f, _) => Some(&f.root),
            _ => None,
        }
    }
}

pub(super) fn rows(c: &Ctx) -> Vec<Row> {
    rows_in(c, &chrono::Local)
}

fn rows_in<Tz: TimeZone>(c: &Ctx, tz: &Tz) -> Vec<Row>
where
    Tz::Offset: std::fmt::Display,
{
    let (info, now_secs) = (c.info, c.now_ms / 1000);
    let mut b = Builder { c, out: Vec::new() };
    let or_dash = |s: &str| if s.is_empty() { MISSING.to_string() } else { s.to_string() };

    b.head("Session");
    if let Some(t) = info.title.as_deref().filter(|t| !t.is_empty()) {
        b.fact("title", t);
    }
    b.fact("id", info.session_id.as_deref().unwrap_or(MISSING));
    b.fact("started", &started(info.started_ms, c.now_ms, tz));
    b.fact("cwd", &or_dash(&info.cwd));
    b.fact("branch", &branch(info.branch.as_deref()));

    b.gap();
    b.head("Model");
    b.fact("model", info.model.as_deref().unwrap_or(MISSING));
    b.fact("effort", info.effort.as_deref().unwrap_or(MISSING));
    b.fact("mode", &or_dash(&info.mode));
    b.fact("status", &or_dash(&info.status));

    b.gap();
    b.head("Context");
    match info.context {
        Some((used, total)) if total > 0 => {
            let pct = (used.saturating_mul(100) / total).min(100) as u8;
            b.meter("context", pct, format!("{pct}% {}/{}", tokens(used), tokens(total)));
        }
        _ => b.fact("context", "not reported"),
    }
    b.fact("turns", &count(info.turns));
    b.fact("tool calls", &count(info.tool_calls));
    b.fact("compacted", &count(info.compactions));

    b.gap();
    b.head("Tokens");
    let usage = [info.tokens_in, info.tokens_out, info.cache_read, info.cache_write];
    if usage.iter().all(Option::is_none) {
        b.fact("tokens", "not reported");
    } else {
        let t = |n: Option<u64>| n.map_or(MISSING.to_string(), tokens);
        b.fact("input", &t(info.tokens_in));
        b.fact("output", &t(info.tokens_out));
        b.fact("cache read", &t(info.cache_read));
        b.fact("cache write", &t(info.cache_write));
        b.fact("cache hit", &cache_hit(info).map_or(MISSING.to_string(), |p| format!("{p}%")));
    }

    b.gap();
    b.head("Rate limits");
    for (label, key, file_pct) in [
        ("5h usage", "five_hour", info.five_hour_pct),
        ("7d usage", "seven_day", None),
    ] {
        // A window past its reset is stale: show nothing for it rather than `resets 0h00m` beside an old percent.
        let live = info
            .rate_limits
            .get(key)
            .filter(|w| w.resets_at.is_none_or(|r| r > now_secs));
        match (live, file_pct) {
            (Some(w), _) => {
                let resets = w
                    .resets_at
                    .map(|r| format!(" resets {}", reset_in(r, now_secs)))
                    .unwrap_or_default();
                b.meter(label, w.pct, format!("{}%{resets}", w.pct));
            }
            (None, Some(pct)) => b.meter(label, pct, format!("{pct}%")),
            (None, None) => b.fact(label, "not reported"),
        }
    }

    b.gap();
    b.head("Runtime");
    b.fact("subagents", &info.subagents.len().to_string());
    b.fact("bg jobs", &info.jobs.len().to_string());
    b.out
}

struct Builder<'a, 'b> {
    c: &'a Ctx<'b>,
    out: Vec<Row>,
}

impl Builder<'_, '_> {
    fn head(&mut self, text: &str) {
        let style = Style::default()
            .fg(self.c.theme.accent_user)
            .add_modifier(Modifier::BOLD);
        let line = Line::from(Span::styled(truncate_str(text, self.c.width), style));
        self.out.push(Row::new(line, text));
    }

    fn gap(&mut self) {
        self.out.push(Row::new(Line::default(), ""));
    }

    fn key(&self, k: &str) -> Span<'static> {
        Span::styled(format!("  {k:<w$} ", w = KEY), Style::default().fg(self.c.theme.gray))
    }

    /// `  key value`; `y` copies `key: value` in full.
    fn fact(&mut self, k: &str, v: &str) {
        let shown = truncate_str(v, self.c.width.saturating_sub(PREFIX));
        let value = Span::styled(shown, Style::default().fg(self.c.theme.text_primary));
        let key = self.key(k);
        self.out.push(Row::new(Line::from(vec![key, value]), format!("{k}: {v}")));
    }

    /// `  key ████░░░░ tail`; the bar is dropped when the panel is too narrow to leave room for the tail.
    fn meter(&mut self, k: &str, pct: u8, tail: String) {
        let t = self.c.theme;
        let color = match pct {
            90.. => t.accent_error,
            70.. => t.accent_plan,
            _ => t.accent_success,
        };
        // One bar width for every meter, leaving room for the longest tail (`100% resets 130h42m`).
        let bar_w = self.c.width.saturating_sub(PREFIX + 21).min(24);
        let mut spans = vec![self.key(k)];
        let mut room = self.c.width.saturating_sub(PREFIX);
        if bar_w >= 4 {
            spans.push(Span::styled(bar(pct, bar_w), Style::default().fg(color)));
            room = room.saturating_sub(bar_w + 1);
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(truncate_str(&tail, room), Style::default().fg(t.text_primary)));
        self.out.push(Row::new(Line::from(spans), format!("{k}: {tail}")));
    }
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

/// A harness counter; zero is what a partial snapshot carries before the full one arrives, so it reads as unknown.
fn count(n: Option<u64>) -> String {
    n.filter(|n| *n > 0).map_or(MISSING.to_string(), |n| n.to_string())
}

fn branch(b: Option<&str>) -> String {
    match b {
        None => MISSING.to_string(),
        Some("") => "detached HEAD".to_string(),
        Some(b) => b.to_string(),
    }
}

/// `2h ago · 2026-11-16 14:03`, in `tz`.
fn started<Tz: TimeZone>(started_ms: Option<i64>, now_ms: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Some(at) = started_ms.and_then(|ms| tz.timestamp_millis_opt(ms).single()) else {
        return MISSING.to_string();
    };
    let ago = match fmt_age(now_ms - started_ms.unwrap_or(now_ms)).as_str() {
        "now" => "just now".to_string(),
        age => format!("{age} ago"),
    };
    format!("{ago} · {}", at.format("%Y-%m-%d %H:%M"))
}

/// Share of prompt tokens served from cache, as `/session-info` counts it: read over input + read + write.
fn cache_hit(info: &Info) -> Option<u8> {
    let (input, read, write) = (info.tokens_in?, info.cache_read?, info.cache_write.unwrap_or(0));
    let prompt = input + read + write;
    (prompt > 0).then(|| (read * 100 / prompt) as u8)
}

#[cfg(test)]
mod tests {
    use super::super::paint;
    use super::*;
    use crate::theme::Theme;
    use super::super::{Job, JobState};

    fn job(kind: &str, description: &str) -> Job {
        Job { kind: kind.into(), description: description.into(), state: JobState::Running, model: None, elapsed: "1m".into() }
    }

    const NOW: i64 = 1_800_000_000_000;

    fn filled() -> Info {
        let mut rate_limits = std::collections::BTreeMap::new();
        rate_limits.insert(
            "five_hour".to_string(),
            super::super::RateWindow { pct: 42, resets_at: Some(NOW / 1000 + 2 * 3600 + 600) },
        );
        rate_limits.insert(
            "seven_day".to_string(),
            super::super::RateWindow { pct: 91, resets_at: Some(NOW / 1000 + 130 * 3600) },
        );
        Info {
            title: Some("Fix the side panel session page".into()),
            session_id: Some("019a8c3e-7b1d-7c40-9a52-0d3f6e2b41aa".into()),
            cwd: "~/repos/grok-build".into(),
            model: Some("grok-build".into()),
            effort: Some("high".into()),
            mode: "agent · auto".into(),
            context: Some((84_000, 200_000)),
            rate_limits,
            status: "running".into(),
            branch: Some("main".into()),
            started_ms: Some(NOW - 2 * 3600 * 1000 - 5 * 60 * 1000),
            turns: Some(12),
            tool_calls: Some(87),
            compactions: Some(1),
            tokens_in: Some(120_000),
            tokens_out: Some(30_000),
            cache_read: Some(800_000),
            cache_write: Some(80_000),
            jobs: vec![job("", "sleep 60")],
            subagents: vec![job("Explore", "explore"), job("Reviewer", "review")],
            ..Info::default()
        }
    }

    fn snap(info: &Info, width: usize) -> String {
        let ctx = Ctx { info, theme: &Theme::current(), width, now_ms: NOW };
        paint(&rows_in(&ctx, &chrono::Utc), width as u16)
    }

    #[test]
    fn started_reads_date_time_and_age() {
        let two_h = NOW - 2 * 3600 * 1000 - 5 * 60 * 1000;
        assert_eq!(started(Some(two_h), NOW, &chrono::Utc), "2h ago · 2027-01-15 05:55");
        assert_eq!(started(Some(NOW - 2000), NOW, &chrono::Utc), "just now · 2027-01-15 07:59");
        assert_eq!(started(None, NOW, &chrono::Utc), "—");
    }

    #[test]
    fn cache_hit_is_read_share_of_prompt_tokens() {
        let i = Info { tokens_in: Some(100), cache_read: Some(800), cache_write: Some(100), ..Info::default() };
        assert_eq!(cache_hit(&i), Some(80));
        assert_eq!(cache_hit(&Info::default()), None);
        let zero = Info { tokens_in: Some(0), cache_read: Some(0), ..Info::default() };
        assert_eq!(cache_hit(&zero), None);
    }

    #[test]
    fn branch_and_count_show_unknowns() {
        assert_eq!(branch(None), "—");
        assert_eq!(branch(Some("")), "detached HEAD");
        assert_eq!(count(Some(0)), "—");
        assert_eq!(count(Some(3)), "3");
    }

    #[test]
    fn copy_is_the_full_key_value_even_when_the_line_is_cut() {
        let info = filled();
        let ctx = Ctx { info: &info, theme: &Theme::current(), width: 21, now_ms: NOW };
        let rows = rows_in(&ctx, &chrono::Utc);
        let copies: Vec<&str> = rows.iter().map(|r| r.copy.as_str()).collect();
        assert!(copies.contains(&"Session"));
        assert!(copies.contains(&"title: Fix the side panel session page"));
        assert!(copies.contains(&"id: 019a8c3e-7b1d-7c40-9a52-0d3f6e2b41aa"));
        assert!(copies.contains(&"context: 42% 84k/200k"));
        assert!(copies.contains(&"5h usage: 42% resets 2h10m"));
        assert!(copies.contains(&"cache hit: 80%"));
        assert!(copies.contains(&"subagents: 2"));
    }

    #[test]
    fn filled_inner_width_43() {
        insta::assert_snapshot!(snap(&filled(), 43));
    }

    #[test]
    fn filled_inner_width_21() {
        insta::assert_snapshot!(snap(&filled(), 21));
    }

    #[test]
    fn empty_inner_width_43() {
        insta::assert_snapshot!(snap(&Info::default(), 43));
    }

    #[test]
    fn empty_inner_width_21() {
        insta::assert_snapshot!(snap(&Info::default(), 21));
    }
}
