//! F3 side panel, after dsh-tui's: a column beside the chat (a real layout split, not an overlay) with a small Traces
//! tail of this session and an Info page. Esc hands the keyboard back to the chat, ←/→ switch pages, `z` swaps the
//! panel and chat widths, `+`/`-` resize. The full-screen `/traces` modal is separate and unchanged.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

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
    pub five_hour_pct: Option<u8>,
    pub status: String,
    /// (description, meta) per running background command.
    pub jobs: Vec<(String, String)>,
    pub subagents: Vec<(String, String)>,
}

enum Traces {
    Idle,
    /// A worker polls the index once a second for this session's row; `None` is its heartbeat.
    Looking(mpsc::Receiver<Option<SessionRow>>),
    Following(Box<Follow>),
    Failed(String),
}

pub struct SidePanel {
    pub open: bool,
    pub focused: bool,
    pub zoomed: bool,
    pub page: Page,
    width: u16,
    traces: Traces,
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
            KeyCode::Char('j' | 'k' | 'g' | 'G' | 'e')
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Tab
            | KeyCode::BackTab => {
                if self.page == Page::Traces
                    && let Traces::Following(f) = &mut self.traces
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
            Traces::Looking(rx) => {
                let mut found = None;
                loop {
                    match rx.try_recv() {
                        Ok(Some(row)) => found = Some(row),
                        Ok(None) => continue,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            self.traces = Traces::Failed("traces index unavailable".into());
                            return true;
                        }
                    }
                }
                let Some(row) = found else { return false };
                self.traces = Traces::Following(Box::new(Follow::open(
                    xai_grok_traces::default_db_path(),
                    row,
                )));
                true
            }
            Traces::Following(f) => f.poll(now_ms),
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
                for (i, line) in info_lines(info, w, theme)
                    .into_iter()
                    .take(body.height as usize)
                    .enumerate()
                {
                    buf.set_line(body.x, body.y + i as u16, &line, body.width);
                }
            }
            Page::Traces => {
                let note = match &mut self.traces {
                    Traces::Following(f) => {
                        f.render(buf, body, None, theme, now_ms);
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
        self.traces = Traces::Following(Box::new(f));
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

fn info_lines(info: &Info, width: usize, theme: &Theme) -> Vec<Line<'static>> {
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
        // One bar width for every meter, leaving room for the longest tail (`100% 999k/999k`).
        let bar_w = width.saturating_sub(12 + 15).clamp(4, 24);
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
    if let Some(pct) = info.five_hour_pct {
        out.push(meter("5h usage", pct, format!("{pct}%")));
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
