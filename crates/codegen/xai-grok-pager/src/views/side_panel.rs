//! F3 side panel, after dsh-tui's: a column beside the chat (a real layout split, not an overlay) with a small Traces
//! tail of this session and an Info page. Esc hands the keyboard back to the chat, ←/→ switch pages, `z` swaps the
//! panel and chat widths, `+`/`-` resize. The full-screen `/traces` modal is separate and unchanged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::{SessionRow, TraceIndex};

use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_follow::Follow;

pub const TOGGLE_KEY: KeyCode = KeyCode::F(3);
const DEFAULT_WIDTH: u16 = 46;
const MIN_WIDTH: u16 = 24;
const MIN_CHAT: u16 = 40;
const STEP: u16 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Traces,
    Info,
}

impl Page {
    const ALL: [Page; 2] = [Page::Traces, Page::Info];

    fn tab(self) -> &'static str {
        match self {
            Page::Traces => "∿ Traces",
            Page::Info => "◆ Info",
        }
    }
}

/// What the Info page shows, gathered from the agent view each frame.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub title: Option<String>,
    pub session_id: Option<String>,
    pub cwd: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: String,
    /// (used, window) tokens, when the harness has reported them.
    pub context: Option<(u64, u64)>,
    /// Claude's `_claude/rateLimit` windows by `rateLimitType`, as the ACP adapter reported them.
    pub rate_limits: BTreeMap<String, RateWindow>,
    /// The statusline-file 5h figure, shown only while `rate_limits` has no `five_hour`.
    pub five_hour_pct: Option<u8>,
    pub status: String,
    /// (description, meta) per running background command.
    pub jobs: Vec<(String, String)>,
    pub subagents: Vec<(String, String)>,
}

/// One Claude rate-limit window: percent used and when it resets (epoch seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateWindow {
    pub pct: u8,
    pub resets_at: Option<i64>,
}

/// Reads the adapter's `usage_update` `_meta["_claude/rateLimit"]` (`{rateLimitType, utilization 0..1, resetsAt}`).
/// Each event carries one window, so callers merge by the returned type.
pub fn parse_rate_limit(
    meta: &serde_json::Map<String, serde_json::Value>,
) -> Option<(String, RateWindow)> {
    let v = meta.get("_claude/rateLimit")?;
    let kind = v["rateLimitType"].as_str()?.to_string();
    let pct = (v["utilization"].as_f64()? * 100.0).clamp(0.0, 100.0).round() as u8;
    let resets_at = v["resetsAt"].as_f64().map(|t| t as i64);
    Some((kind, RateWindow { pct, resets_at }))
}

/// `2h32m`; hours are not capped, so a 7-day window reads `130h42m`.
fn reset_in(resets_at: i64, now_secs: i64) -> String {
    let mins = (resets_at - now_secs).max(0) / 60;
    format!("{}h{:02}m", mins / 60, mins % 60)
}

enum Traces {
    Idle,
    /// A worker polls the index once a second for this session's row; `None` is its heartbeat.
    Looking(mpsc::Receiver<Option<SessionRow>>),
    /// The index handle reads an opened call's command and result.
    Following(Box<Follow>, Option<TraceIndex>),
    Failed(String),
}

pub struct SidePanel {
    pub open: bool,
    pub focused: bool,
    pub zoomed: bool,
    pub page: Page,
    width: u16,
    traces: Traces,
    /// Claude's 5-hour limit, from the statusline snapshots `ocq` reads.
    pub five_hour_pct: Option<u8>,
}

impl Default for SidePanel {
    fn default() -> Self {
        Self {
            open: false,
            focused: false,
            zoomed: false,
            page: Page::default(),
            width: DEFAULT_WIDTH,
            traces: Traces::Idle,
            five_hour_pct: None,
        }
    }
}

impl SidePanel {
    /// F3: open with the keyboard, take the keyboard back from the chat, or close.
    pub fn toggle(&mut self) {
        if !self.open {
            self.open = true;
            self.focused = true;
        } else if !self.focused {
            self.focused = true;
        } else {
            self.open = false;
            self.focused = false;
        }
    }

    pub fn has_keys(&self) -> bool {
        self.open && self.focused
    }

    /// Keys while focused. False lets the key fall through to the chat (unhandled Ctrl chords such as Ctrl+C).
    pub fn handle_key(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            _ if ctrl => return false,
            KeyCode::Esc => self.focused = false,
            KeyCode::Left | KeyCode::Right => {
                let at = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0);
                let n = Page::ALL.len();
                let next = if key.code == KeyCode::Right {
                    at + 1
                } else {
                    at + n - 1
                };
                self.page = Page::ALL[next % n];
            }
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            // Zoomed, `width` is the chat's, so wider panel means a narrower chat.
            KeyCode::Char('+' | '=') => self.resize(!self.zoomed),
            KeyCode::Char('-' | '_') => self.resize(self.zoomed),
            KeyCode::Char('j' | 'k' | 'g' | 'G' | 'e' | 'h' | 'l')
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Tab
            | KeyCode::BackTab => {
                if self.page == Page::Traces
                    && let Traces::Following(f, _) = &mut self.traces
                {
                    f.key(key.code);
                }
            }
            _ => {}
        }
        true
    }

    fn resize(&mut self, grow: bool) {
        self.width = if grow {
            self.width.saturating_add(STEP)
        } else {
            self.width.saturating_sub(STEP).max(MIN_WIDTH)
        };
    }

    /// Splits `area` into the chat and the panel. Too narrow for both, a focused panel takes the whole width and an
    /// unfocused one hides.
    pub fn split(&self, area: Rect) -> (Rect, Option<Rect>) {
        if !self.open {
            return (area, None);
        }
        if area.width < MIN_WIDTH + MIN_CHAT {
            return if self.focused {
                (Rect { width: 0, ..area }, Some(area))
            } else {
                (area, None)
            };
        }
        let panel = if self.zoomed {
            area.width - self.width.clamp(MIN_CHAT, area.width - MIN_WIDTH)
        } else {
            self.width.clamp(MIN_WIDTH, area.width - MIN_CHAT)
        };
        let chat = Rect {
            width: area.width - panel,
            ..area
        };
        (
            chat,
            Some(Rect {
                x: area.x + chat.width,
                width: panel,
                ..area
            }),
        )
    }

    /// The 1 s traces tick: find this session in the index, then feed the tail. True when the panel should redraw.
    pub fn poll(&mut self, native: Option<&str>, now_ms: i64) -> bool {
        if !self.open {
            return false;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let five_hour = claude_five_hour_pct(&Path::new(&home).join(".cache/claude-status"));
        let usage_moved = std::mem::replace(&mut self.five_hour_pct, five_hour) != five_hour;
        usage_moved | self.poll_traces(native, now_ms)
    }

    fn poll_traces(&mut self, native: Option<&str>, now_ms: i64) -> bool {
        match &mut self.traces {
            Traces::Idle => match native.filter(|n| !n.is_empty()) {
                Some(native) => {
                    self.traces = Traces::Looking(look_up(
                        xai_grok_traces::default_db_path(),
                        native.to_string(),
                    ));
                    true
                }
                None => false,
            },
            Traces::Looking(rx) => match found_row(rx) {
                Ok(None) => false,
                Ok(Some(row)) => {
                    let db = xai_grok_traces::default_db_path();
                    let index = TraceIndex::open_with(&db, Some(Duration::from_millis(200))).ok();
                    self.traces = Traces::Following(Box::new(Follow::open(db, row)), index);
                    true
                }
                Err(()) => {
                    self.traces = Traces::Failed("traces index unavailable".into());
                    true
                }
            },
            Traces::Following(f, _) => f.poll(now_ms),
            Traces::Failed(_) => false,
        }
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        info: &Info,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width < 4 || area.height < 4 {
            return;
        }
        buf.set_style(area, Style::default().bg(theme.bg_base));
        let edge = Style::default().fg(if self.focused {
            theme.accent_user
        } else {
            theme.gray_dim
        });
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "│", edge);
        }
        let inner = Rect {
            x: area.x + 2,
            width: area.width - 3,
            ..area
        };
        let w = inner.width as usize;

        let mut tabs = vec![Span::styled("◀ ", Style::default().fg(theme.gray))];
        for page in Page::ALL {
            let style = if page == self.page {
                Style::default()
                    .fg(theme.text_primary)
                    .bg(theme.bg_highlight)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.gray)
            };
            tabs.push(Span::styled(format!(" {} ", page.tab()), style));
            tabs.push(Span::raw(" "));
        }
        tabs.push(Span::styled("▸", Style::default().fg(theme.gray)));
        buf.set_line(inner.x, inner.y, &Line::from(tabs), inner.width);
        buf.set_string(
            inner.x,
            inner.y + 1,
            "─".repeat(w),
            Style::default().fg(theme.gray_dim),
        );

        let footer = if self.focused {
            "Esc chat · ←/→ panels · z zoom · +/- width"
        } else {
            "F3 focus panel"
        };
        let footer_y = inner.y + inner.height - 1;
        buf.set_string(
            inner.x,
            footer_y,
            truncate_str(footer, w),
            Style::default().fg(theme.gray),
        );

        let body = Rect {
            y: inner.y + 2,
            height: inner.height - 3,
            ..inner
        };
        match self.page {
            Page::Info => {
                for (i, line) in info_lines(info, w, theme, now_ms / 1000)
                    .into_iter()
                    .take(body.height as usize)
                    .enumerate()
                {
                    buf.set_line(body.x, body.y + i as u16, &line, body.width);
                }
            }
            Page::Traces => {
                let note = match &mut self.traces {
                    Traces::Following(f, index) => {
                        f.render(buf, body, index.as_ref(), theme, now_ms);
                        return;
                    }
                    Traces::Idle if info.session_id.is_none() => "no session yet".to_string(),
                    Traces::Idle | Traces::Looking(_) => {
                        "waiting for this session in the traces index…".to_string()
                    }
                    Traces::Failed(e) => e.clone(),
                };
                buf.set_string(
                    body.x,
                    body.y,
                    truncate_str(&note, w),
                    Style::default().fg(theme.gray),
                );
            }
        }
    }
}

#[cfg(test)]
impl SidePanel {
    /// A Traces page already following `root`, fed `items` with no worker behind it.
    pub(crate) fn follow_for_test(
        &mut self,
        root: SessionRow,
        items: &[serde_json::Value],
        now_ms: i64,
    ) {
        let mut f = Follow::bare(root);
        f.apply(items, now_ms);
        self.traces = Traces::Following(Box::new(f), None);
    }
}

/// The newest snapshot under a day old that carries a 5-hour limit, as `ocq` picks it. Claude Code's statusline
/// writes one JSON file per session here.
fn claude_five_hour_pct(dir: &Path) -> Option<u8> {
    let now = SystemTime::now();
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .filter(|(m, _)| {
            now.duration_since(*m)
                .is_ok_and(|age| age < Duration::from_secs(86_400))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.iter().find_map(|(_, p)| {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
        let pct = v["rate_limits"]["five_hour"]["used_percentage"].as_f64()?;
        Some(pct.clamp(0.0, 100.0).round() as u8)
    })
}

/// The lookup worker's answer so far. The worker exits right after sending the row, so a hang-up only means
/// failure when no row came first.
fn found_row(rx: &mpsc::Receiver<Option<SessionRow>>) -> Result<Option<SessionRow>, ()> {
    let mut found = None;
    loop {
        match rx.try_recv() {
            Ok(row) => found = found.or(row),
            Err(mpsc::TryRecvError::Empty) => return Ok(found),
            Err(mpsc::TryRecvError::Disconnected) => return found.map(Some).ok_or(()),
        }
    }
}

/// Finds `native` in the index off the UI thread: opening the index can wait on a rebuild.
fn look_up(db: PathBuf, native: String) -> mpsc::Receiver<Option<SessionRow>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok(index) = TraceIndex::open_with(&db, Some(Duration::from_secs(2))) else {
            return;
        };
        loop {
            let row = index.session_by_harness_id(&native).ok().flatten();
            let found = row.is_some();
            // A closed panel drops the receiver, which ends this thread.
            if tx.send(row).is_err() || found {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    rx
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
        out.push(match (info.rate_limits.get(key), file_pct) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(open: bool, focused: bool, zoomed: bool) -> SidePanel {
        SidePanel {
            open,
            focused,
            zoomed,
            ..SidePanel::default()
        }
    }

    #[test]
    fn split_gives_the_panel_its_width_and_zoom_swaps_it_with_the_chat() {
        let area = Rect::new(0, 0, 120, 30);
        assert_eq!(panel(false, false, false).split(area), (area, None));
        let (chat, side) = panel(true, false, false).split(area);
        assert_eq!(
            (chat.width, side.unwrap().x, side.unwrap().width),
            (74, 74, 46)
        );
        let (chat, side) = panel(true, true, true).split(area);
        assert_eq!((chat.width, side.unwrap().width), (46, 74));
    }

    #[test]
    fn rate_limit_meta_parses_one_window_and_reset_time_formats_hours_uncapped() {
        let meta = |v: serde_json::Value| v.as_object().unwrap().clone();
        let m = meta(serde_json::json!({"_claude/rateLimit": {
            "status": "allowed", "rateLimitType": "seven_day", "utilization": 0.426, "resetsAt": 1_000_000
        }}));
        assert_eq!(
            parse_rate_limit(&m),
            Some((
                "seven_day".into(),
                RateWindow {
                    pct: 43,
                    resets_at: Some(1_000_000)
                }
            ))
        );
        assert_eq!(parse_rate_limit(&meta(serde_json::json!({"other": 1}))), None);
        assert_eq!(reset_in(1_000_000 + 2 * 3600 + 32 * 60, 1_000_000), "2h32m");
        assert_eq!(
            reset_in(1_000_000 + 130 * 3600 + 42 * 60, 1_000_000),
            "130h42m"
        );
        assert_eq!(reset_in(5, 1_000_000), "0h00m", "a past reset clamps to zero");
    }

    #[test]
    fn five_hour_reads_the_newest_snapshot_that_has_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(claude_five_hour_pct(dir.path()), None);
        std::fs::write(
            dir.path().join("a.json"),
            r#"{"rate_limits":{"five_hour":{"used_percentage":22.4}}}"#,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(
            dir.path().join("b.json"),
            r#"{"session_id":"no limits here"}"#,
        )
        .unwrap();
        assert_eq!(
            claude_five_hour_pct(dir.path()),
            Some(22),
            "a newer file without limits is skipped"
        );
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(
            dir.path().join("c.json"),
            r#"{"rate_limits":{"five_hour":{"used_percentage":61}}}"#,
        )
        .unwrap();
        assert_eq!(claude_five_hour_pct(dir.path()), Some(61));
    }

    #[test]
    fn a_row_sent_just_before_the_worker_exits_is_kept() {
        let (tx, rx) = mpsc::channel();
        tx.send(None).unwrap();
        tx.send(Some(SessionRow {
            id: "grok:abc".into(),
            ..Default::default()
        }))
        .unwrap();
        drop(tx);
        assert_eq!(
            found_row(&rx).unwrap().map(|r| r.id),
            Some("grok:abc".into())
        );
        let (tx, rx) = mpsc::channel::<Option<SessionRow>>();
        drop(tx);
        assert!(
            found_row(&rx).is_err(),
            "a hang-up with no row is a failure"
        );
        let (_tx, rx) = mpsc::channel::<Option<SessionRow>>();
        assert!(found_row(&rx).unwrap().is_none(), "still looking");
    }

    #[test]
    fn narrow_terminal_gives_a_focused_panel_the_whole_width_and_hides_an_unfocused_one() {
        let area = Rect::new(0, 0, 50, 30);
        assert_eq!(
            panel(true, true, false).split(area),
            (Rect { width: 0, ..area }, Some(area))
        );
        assert_eq!(panel(true, false, false).split(area), (area, None));
    }

    #[test]
    fn keys_switch_pages_zoom_resize_and_esc_returns_to_chat() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let mut p = SidePanel::default();
        p.toggle();
        assert!(p.has_keys());
        assert!(p.handle_key(&key(KeyCode::Right)));
        assert_eq!(p.page, Page::Info);
        p.handle_key(&key(KeyCode::Right));
        assert_eq!(p.page, Page::Traces, "pages wrap");
        p.handle_key(&key(KeyCode::Char('+')));
        assert_eq!(p.width, DEFAULT_WIDTH + STEP);
        p.handle_key(&key(KeyCode::Char('z')));
        p.handle_key(&key(KeyCode::Char('+')));
        assert_eq!(
            p.width, DEFAULT_WIDTH,
            "zoomed, a wider panel is a narrower chat"
        );
        assert!(
            !p.handle_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            "Ctrl+C reaches the chat"
        );
        p.handle_key(&key(KeyCode::Esc));
        assert!(p.open && !p.focused);
        p.toggle();
        assert!(p.focused, "F3 on an unfocused panel focuses it");
        p.toggle();
        assert!(!p.open, "F3 on a focused panel closes it");
    }
}
