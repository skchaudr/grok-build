//! F3 side panel, after dsh-tui's: a column beside the chat (a real layout split, not an overlay) with four pages:
//! Session, Steps, Agents, Recent. Every page is a list of one-line [`Row`]s; ↑/↓ (j/k) move a row cursor and `y`
//! copies the highlighted row. Esc hands the keyboard back to the chat, ←/→ switch pages, `z` swaps the panel and chat
//! widths, `+`/`-` resize. The full-screen `/traces` modal is separate and unchanged.
//!
//! Shared parts live here (page enum, cursor, copy, scrolling, drawing). Each page owns its module and exposes
//! `rows(&Ctx) -> Vec<Row>`; nothing else in a page module may touch the cursor or the key handling.

mod jobs;
mod recent;
mod session;
mod steps;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
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
    Session,
    #[default]
    Steps,
    Agents,
    Recent,
}

impl Page {
    const ALL: [Page; 4] = [Page::Session, Page::Steps, Page::Agents, Page::Recent];

    fn tab(self) -> &'static str {
        match self {
            Page::Session => "Session",
            Page::Steps => "Steps",
            Page::Agents => "Agents",
            Page::Recent => "Recent",
        }
    }

    fn idx(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// A live log: the cursor sits on the newest row until the user moves off it.
    fn tails(self) -> bool {
        self == Page::Steps
    }
}

/// One line of a page. `copy` is what `y` puts on the clipboard (full text, not the truncated line); `open` is the
/// trace-index session id Enter opens, when the row stands for one.
#[derive(Clone, Debug)]
pub struct Row {
    pub line: Line<'static>,
    pub copy: String,
    pub open: Option<String>,
}

impl Row {
    pub fn new(line: Line<'static>, copy: impl Into<String>) -> Self {
        Self { line, copy: copy.into(), open: None }
    }

    /// A row that copies its own text.
    pub fn plain(line: Line<'static>) -> Self {
        let copy = line.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        Self::new(line, copy.trim_end())
    }

    pub fn opens(mut self, session_id: impl Into<String>) -> Self {
        self.open = Some(session_id.into());
        self
    }
}

/// What a page needs to build its rows.
pub struct Ctx<'a> {
    pub info: &'a Info,
    pub theme: &'a Theme,
    /// Drawable columns of a row; rows longer than this are cut by the drawing, so truncate with a `…` yourself.
    pub width: usize,
    pub now_ms: i64,
}

/// What a key asked of the agent view; taken by its glue after `handle_key`.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Copy(String),
    /// Open this trace-index session id.
    Open(String),
}

/// Cursor and scroll of one page.
#[derive(Clone, Copy, Debug, Default)]
struct View {
    cursor: usize,
    top: usize,
    /// The user moved off the tail of a [`Page::tails`] page.
    free: bool,
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
    // Epoch seconds only: a millisecond value would read as a reset ~50,000 years out.
    let resets_at = v["resetsAt"]
        .as_f64()
        .filter(|t| t.is_finite() && (0.0..1e11).contains(t))
        .map(|t| t as i64);
    Some((kind, RateWindow { pct, resets_at }))
}

/// `2h32m`; hours are not capped, so a 7-day window reads `130h42m`.
fn reset_in(resets_at: i64, now_secs: i64) -> String {
    let mins = resets_at.saturating_sub(now_secs).max(0) / 60;
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
    recent: recent::Recent,
    /// Claude's 5-hour limit, from the statusline snapshots `ocq` reads.
    pub five_hour_pct: Option<u8>,
    view: [View; 4],
    /// Rows on each page as of the last frame.
    len: [usize; 4],
    /// Body height as of the last frame, for PgUp/PgDn.
    page_h: usize,
    /// (copy text, open id) of the highlighted row as of the last frame.
    selected: Option<(String, Option<String>)>,
    event: Option<Event>,
    /// A line for the footer (`copied`), cleared by the next key.
    note: Option<String>,
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
            recent: recent::Recent::default(),
            five_hour_pct: None,
            view: [View::default(); 4],
            len: [0; 4],
            page_h: 10,
            selected: None,
            event: None,
            note: None,
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
        self.note = None;
        match key.code {
            _ if ctrl => return false,
            KeyCode::Esc => self.focused = false,
            KeyCode::Left | KeyCode::Right => {
                let n = Page::ALL.len();
                let at = self.page.idx();
                let next = if key.code == KeyCode::Right { at + 1 } else { at + n - 1 };
                self.page = Page::ALL[next % n];
            }
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            // Zoomed, `width` is the chat's, so wider panel means a narrower chat.
            KeyCode::Char('+' | '=') => self.resize(!self.zoomed),
            KeyCode::Char('-' | '_') => self.resize(self.zoomed),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageUp => self.step(-(self.page_h.max(1) as isize)),
            KeyCode::PageDown => self.step(self.page_h.max(1) as isize),
            KeyCode::Home | KeyCode::Char('g') => self.jump(false),
            KeyCode::End | KeyCode::Char('G') => self.jump(true),
            KeyCode::Char('y') => {
                self.event = self.selected.as_ref().map(|(copy, _)| Event::Copy(copy.clone()));
            }
            KeyCode::Enter => {
                self.event = self.selected.as_ref().and_then(|(_, open)| open.clone()).map(Event::Open);
            }
            _ => {}
        }
        true
    }

    /// The copy or open a key asked for, once.
    pub fn take_event(&mut self) -> Option<Event> {
        self.event.take()
    }

    pub fn set_note(&mut self, note: String) {
        self.note = Some(note);
    }

    fn step(&mut self, delta: isize) {
        let i = self.page.idx();
        let last = self.len[i].saturating_sub(1);
        let v = &mut self.view[i];
        v.cursor = (v.cursor as isize + delta).clamp(0, last as isize) as usize;
        v.free = !(self.page.tails() && v.cursor >= last);
    }

    /// `g` / `G`: first or last row. The last row of a tailing page pins it again.
    fn jump(&mut self, end: bool) {
        let i = self.page.idx();
        let last = self.len[i].saturating_sub(1);
        let v = &mut self.view[i];
        v.cursor = if end { last } else { 0 };
        v.free = !(self.page.tails() && end);
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
        let recent = self.page == Page::Recent && self.recent.poll(now_ms);
        usage_moved | self.poll_traces(native, now_ms) | recent
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

        let footer = match (&self.note, self.focused) {
            (Some(note), _) => note.clone(),
            (None, true) => "Esc chat · ←/→ page · j/k move · y copy · z zoom".to_string(),
            (None, false) => "F3 focus panel".to_string(),
        };
        let footer_y = inner.y + inner.height - 1;
        buf.set_string(
            inner.x,
            footer_y,
            truncate_str(&footer, w),
            Style::default().fg(theme.gray),
        );

        let body = Rect {
            y: inner.y + 2,
            height: inner.height - 3,
            ..inner
        };
        let ctx = Ctx { info, theme, width: w, now_ms };
        let rows = match self.page {
            Page::Session => session::rows(&ctx),
            Page::Steps => match &self.traces {
                Traces::Following(f, _) => steps::rows(f, &ctx),
                Traces::Idle if info.session_id.is_none() => note_rows("no session yet", &ctx),
                Traces::Idle | Traces::Looking(_) => {
                    note_rows("waiting for this session in the traces index…", &ctx)
                }
                Traces::Failed(e) => note_rows(e, &ctx),
            },
            Page::Agents => jobs::rows(&ctx),
            Page::Recent => self.recent.rows(&ctx),
        };
        self.draw_rows(buf, body, rows, theme);
    }

    /// Clamps the page's cursor to its rows, scrolls it into view, and draws.
    fn draw_rows(&mut self, buf: &mut Buffer, body: Rect, rows: Vec<Row>, theme: &Theme) {
        let i = self.page.idx();
        let n = rows.len();
        let h = body.height as usize;
        self.len[i] = n;
        self.page_h = h;
        let tails = self.page.tails();
        let v = &mut self.view[i];
        if tails && !v.free {
            v.cursor = n.saturating_sub(1);
        }
        v.cursor = v.cursor.min(n.saturating_sub(1));
        if v.cursor < v.top {
            v.top = v.cursor;
        } else if v.cursor >= v.top + h {
            v.top = v.cursor + 1 - h;
        }
        v.top = v.top.min(n.saturating_sub(h));
        let (cursor, top) = (v.cursor, v.top);
        self.selected = rows.get(cursor).map(|r| (r.copy.clone(), r.open.clone()));
        for (k, row) in rows.iter().skip(top).take(h).enumerate() {
            let y = body.y + k as u16;
            buf.set_line(body.x, y, &row.line, body.width);
            if self.focused && top + k == cursor {
                buf.set_style(
                    Rect { y, height: 1, ..body },
                    Style::default().bg(theme.bg_visual).add_modifier(Modifier::BOLD),
                );
            }
        }
    }
}

/// A page that has nothing to list yet says why on its first row.
fn note_rows(note: &str, ctx: &Ctx) -> Vec<Row> {
    vec![Row::plain(Line::from(Span::styled(
        truncate_str(note, ctx.width),
        Style::default().fg(ctx.theme.gray),
    )))]
}

/// Page rows as text, one drawn line each, for the pages' snapshot tests.
#[cfg(test)]
pub(crate) fn paint(rows: &[Row], width: u16) -> String {
    let area = Rect::new(0, 0, width, rows.len().max(1) as u16);
    let mut buf = Buffer::empty(area);
    for (y, row) in rows.iter().enumerate() {
        buf.set_line(0, y as u16, &row.line, width);
    }
    (0..area.height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
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
        assert_eq!(
            reset_in(i64::MIN, i64::MAX),
            "0h00m",
            "saturating subtraction, no overflow"
        );
    }

    #[test]
    fn rate_limit_rejects_milliseconds_and_non_finite_resets() {
        let with = |t: serde_json::Value| {
            let m = serde_json::json!({"_claude/rateLimit": {
                "rateLimitType": "five_hour", "utilization": 0.5, "resetsAt": t
            }});
            parse_rate_limit(m.as_object().unwrap()).unwrap().1.resets_at
        };
        assert_eq!(with(serde_json::json!(1.8e12)), None, "ms value dropped");
        assert_eq!(with(serde_json::json!(-1e30)), None, "negative dropped");
        assert_eq!(with(serde_json::json!(1_000_000)), Some(1_000_000));
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
        assert_eq!(p.page, Page::Steps);
        assert!(p.handle_key(&key(KeyCode::Right)));
        assert_eq!(p.page, Page::Agents);
        for _ in 0..3 {
            p.handle_key(&key(KeyCode::Right));
        }
        assert_eq!(p.page, Page::Agents, "four pages wrap");
        p.handle_key(&key(KeyCode::Left));
        assert_eq!(p.page, Page::Steps);
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

    fn lines(rows: &[&str]) -> Vec<Row> {
        rows.iter().map(|t| Row::plain(Line::from(t.to_string()))).collect()
    }

    fn draw(p: &mut SidePanel, rows: &[&str], h: u16) -> Vec<String> {
        let area = Rect::new(0, 0, 30, h);
        let mut buf = Buffer::empty(area);
        p.draw_rows(&mut buf, area, lines(rows), &Theme::current());
        (0..h)
            .map(|y| (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect()
    }

    #[test]
    fn cursor_moves_scrolls_and_y_copies_the_highlighted_row() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let rows = ["a", "b", "c", "d", "e"];
        let mut p = SidePanel { open: true, focused: true, page: Page::Agents, ..SidePanel::default() };
        assert_eq!(draw(&mut p, &rows, 3), ["a", "b", "c"]);
        p.handle_key(&key(KeyCode::Char('y')));
        assert_eq!(p.take_event(), Some(Event::Copy("a".into())));
        assert_eq!(p.take_event(), None, "an event is taken once");
        for _ in 0..4 {
            p.handle_key(&key(KeyCode::Char('j')));
        }
        assert_eq!(draw(&mut p, &rows, 3), ["c", "d", "e"], "the cursor row stays on screen");
        p.handle_key(&key(KeyCode::Char('y')));
        assert_eq!(p.take_event(), Some(Event::Copy("e".into())));
        p.handle_key(&key(KeyCode::Char('g')));
        assert_eq!(draw(&mut p, &rows, 3), ["a", "b", "c"]);
        p.handle_key(&key(KeyCode::Enter));
        assert_eq!(p.take_event(), None, "a row with no session to open does nothing on Enter");
    }

    #[test]
    fn enter_opens_a_row_that_stands_for_a_session() {
        let mut p = SidePanel { open: true, focused: true, page: Page::Recent, ..SidePanel::default() };
        let area = Rect::new(0, 0, 30, 3);
        let mut buf = Buffer::empty(area);
        let row = Row::plain(Line::from("session")).opens("grok:abc");
        p.draw_rows(&mut buf, area, vec![row], &Theme::current());
        p.handle_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(p.take_event(), Some(Event::Open("grok:abc".into())));
    }

    #[test]
    fn the_steps_page_tails_until_the_cursor_moves_off_the_newest_row() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let mut p = SidePanel { open: true, focused: true, page: Page::Steps, ..SidePanel::default() };
        assert_eq!(draw(&mut p, &["a", "b", "c", "d"], 2), ["c", "d"]);
        assert_eq!(draw(&mut p, &["a", "b", "c", "d", "e"], 2), ["d", "e"], "pinned, a new row scrolls in");
        p.handle_key(&key(KeyCode::Char('k')));
        assert_eq!(draw(&mut p, &["a", "b", "c", "d", "e", "f"], 2), ["d", "e"], "moved off the tail, the view holds");
        p.handle_key(&key(KeyCode::Char('G')));
        assert_eq!(draw(&mut p, &["a", "b", "c", "d", "e", "f", "g"], 2), ["f", "g"], "G pins the tail again");
    }
}
