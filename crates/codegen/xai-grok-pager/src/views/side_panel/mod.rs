//! F3 side panel, after dsh-tui's: a column beside the chat (a real layout split, not an overlay) with four pages:
//! Session, Traces, Agents, Recent. Traces is the `/traces` follow view ([`Follow`]) drawn in the column, with its own
//! cursor and folding. The other pages are lists of one-line [`Row`]s; ↑/↓ (j/k) move a row cursor and `y` copies the
//! highlighted row. Esc hands the keyboard back to the chat, `z` swaps the panel and chat widths, `+`/`-` resize.
//! Tab/Shift+Tab switch pages everywhere; on the other pages ←/→ and h/l do too, but on Traces they fold and unfold.
//! The full-screen `/traces` modal is separate and unchanged.
//!
//! Shared parts live here (page enum, cursor, copy, scrolling, drawing). Each row page owns its module and exposes
//! `rows(&Ctx) -> Vec<Row>`; nothing else in a page module may touch the cursor or the key handling.

mod jobs;
mod recent;
mod session;

pub use jobs::{Job, JobState};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
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
    Traces,
    Agents,
    Recent,
}

impl Page {
    const ALL: [Page; 4] = [Page::Session, Page::Traces, Page::Agents, Page::Recent];

    fn tab(self) -> &'static str {
        match self {
            Page::Session => "Session",
            Page::Traces => "Traces",
            Page::Agents => "Agents",
            Page::Recent => "Recent",
        }
    }

    fn idx(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
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
    // session page
    /// Git branch of the cwd; `Some("")` is a detached HEAD.
    pub branch: Option<String>,
    /// Session start, epoch ms, from the trace index row.
    pub started_ms: Option<i64>,
    /// Conversation turns / tool calls / compactions as the harness counted them (zero = not reported yet).
    pub turns: Option<u64>,
    pub tool_calls: Option<u64>,
    pub compactions: Option<u64>,
    /// Session token totals; none of the live pager state carries them, so these stay `None` until fed.
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    // end session page
    /// Background commands and subagents, running first; the Agents page lists both.
    pub jobs: Vec<Job>,
    pub subagents: Vec<Job>,
    /// The chat's status row, when it shows an activity: the Traces page draws it as its last row.
    pub turn: Option<TurnRow>,
}

/// What the chat's turn-status row draws from ([`crate::views::turn_status::TurnStatusArgs`]), so the panel's row
/// shows the same spinner, label and per-activity timer.
#[derive(Clone, Debug)]
pub struct TurnRow {
    pub state: crate::app::agent::AgentState,
    pub activity: Option<crate::acp::tracker::TurnActivity>,
    pub activity_started_at: Option<std::time::Instant>,
    pub tick: u64,
    pub is_bash_turn: bool,
    pub is_pending_user_input: bool,
    pub goal_verifying: bool,
}

impl TurnRow {
    /// One row at `area`: spinner, label and per-activity timer. Not the turn timer, token count, `[stop]`, `[↓]`,
    /// the interrupt hint or the queued suffix, and no mouse area is reported.
    fn draw(&self, buf: &mut Buffer, area: Rect) {
        use crate::views::turn_status::{render_turn_status, TurnStatusArgs, Watchers};
        render_turn_status(
            buf,
            area,
            TurnStatusArgs {
                state: &self.state,
                activity: &self.activity,
                turn_elapsed: None,
                activity_started_at: self.activity_started_at,
                tick: self.tick,
                drain_blocked: false,
                buttons: None,
                has_running_execute: false,
                message_interruptible_tool: false,
                total_tokens: None,
                session_starting_since: None,
                is_bash_turn: self.is_bash_turn,
                is_pending_user_input: self.is_pending_user_input,
                goal_verifying: self.goal_verifying,
                watchers: Watchers::default(),
                parked: false,
                flat_background: false,
                held_queue: 0,
                held_queue_top_sendable: false,
                phase_timer_min_width: 0,
            },
        );
    }
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
        let traces = self.page == Page::Traces;
        self.note = None;
        match key.code {
            _ if ctrl => return false,
            KeyCode::Esc => self.focused = false,
            // Tab/Shift+Tab switch pages everywhere. Traces keeps h/l/Left/Right for folding, as the follow view does.
            KeyCode::Tab => self.turn_page(true),
            KeyCode::BackTab => self.turn_page(false),
            KeyCode::Right | KeyCode::Char('l') if !traces => self.turn_page(true),
            KeyCode::Left | KeyCode::Char('h') if !traces => self.turn_page(false),
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            // Zoomed, `width` is the chat's, so wider panel means a narrower chat.
            KeyCode::Char('+' | '=') => self.resize(!self.zoomed),
            KeyCode::Char('-' | '_') => self.resize(self.zoomed),
            // The follow view owns its cursor, so its keys never touch the row cursor below.
            _ if traces => {
                if let Traces::Following(f, _) = &mut self.traces {
                    if key.code == KeyCode::Char('y') {
                        self.event = copy_text(f).map(Event::Copy);
                    } else {
                        f.key(key.code);
                    }
                }
            }
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

    fn turn_page(&mut self, forward: bool) {
        let n = Page::ALL.len();
        let at = self.page.idx();
        self.page = Page::ALL[(if forward { at + 1 } else { at + n - 1 }) % n];
    }

    fn step(&mut self, delta: isize) {
        let i = self.page.idx();
        let last = self.len[i].saturating_sub(1);
        let v = &mut self.view[i];
        v.cursor = (v.cursor as isize + delta).clamp(0, last as isize) as usize;
    }

    /// `g` / `G`: first or last row.
    fn jump(&mut self, end: bool) {
        let i = self.page.idx();
        let last = self.len[i].saturating_sub(1);
        self.view[i].cursor = if end { last } else { 0 };
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

        let tabs = tab_bar(self.page, w, theme);
        buf.set_line(inner.x, inner.y, &tabs, inner.width);
        buf.set_string(
            inner.x,
            inner.y + 1,
            "─".repeat(w),
            Style::default().fg(theme.gray_dim),
        );

        let footer = match (&self.note, self.focused) {
            (Some(note), _) => note.clone(),
            (None, true) if self.page == Page::Traces => "Esc chat · Tab page · h/l fold · y copy".to_string(),
            (None, true) => "Esc chat · h/l page · j/k row · y copy".to_string(),
            (None, false) => "F3 focus panel".to_string(),
        };
        let footer_y = inner.y + inner.height - 1;
        buf.set_string(
            inner.x,
            footer_y,
            truncate_str(&footer, w),
            Style::default().fg(theme.text_secondary),
        );

        let body = Rect {
            y: inner.y + 2,
            height: inner.height - 3,
            ..inner
        };
        let ctx = Ctx { info, theme, width: w, now_ms };
        let rows = match self.page {
            Page::Session => session::rows(&ctx),
            Page::Traces => {
                // The chat's status row is the lowest row, above the footer; at tiny heights it keeps the row and the page gives it up.
                let mut body = body;
                if let Some(turn) = &info.turn {
                    body.height -= 1;
                    turn.draw(buf, Rect { y: body.y + body.height, height: 1, ..body });
                }
                let note = match &mut self.traces {
                    Traces::Following(f, index) => {
                        if body.height > 0 {
                            f.render(buf, body, index.as_ref(), theme, now_ms);
                        }
                        return;
                    }
                    Traces::Idle if info.session_id.is_none() => "no session yet".to_string(),
                    Traces::Idle | Traces::Looking(_) => "waiting for this session in the traces index…".to_string(),
                    Traces::Failed(e) => e.clone(),
                };
                if body.height > 0 {
                    buf.set_string(body.x, body.y, truncate_str(&note, w), Style::default().fg(theme.text_secondary));
                }
                return;
            }
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
        let v = &mut self.view[i];
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
                // Muted text is unreadable on the selection band; lift it the way grok's own selection does.
                for x in body.x..body.x + body.width {
                    let cell = &mut buf[(x, y)];
                    let faint = match (cell.fg, theme.bg_visual) {
                        (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) => contrast((r, g, b), (br, bg, bb)) < 4.5,
                        (fg, _) => fg == theme.gray || fg == theme.gray_dim,
                    };
                    if faint {
                        cell.set_fg(theme.text_primary);
                    }
                }
            }
        }
    }
}

/// The page tabs in `w` columns: arrows and padded names when they fit, then bare names two apart, then only the
/// current page spelled out with the others as their first letters.
fn tab_bar(page: Page, w: usize, theme: &Theme) -> Line<'static> {
    // Inverse, so the current page reads in every theme whatever its highlight colour is.
    let on = Style::default().fg(theme.bg_base).bg(theme.text_primary).add_modifier(Modifier::BOLD);
    let off = Style::default().fg(theme.text_secondary);
    let names: usize = Page::ALL.iter().map(|p| p.tab().len()).sum();
    let (wide, short) = (w >= names + 15, w < names + 6);
    let mut spans = Vec::new();
    if wide {
        spans.push(Span::styled("◀ ", off));
    }
    for (i, p) in Page::ALL.into_iter().enumerate() {
        let sel = p == page;
        if i > 0 {
            spans.push(Span::raw(if short { " " } else if wide { " " } else { "  " }));
        }
        let name = if short && !sel { &p.tab()[..1] } else { p.tab() };
        let label = if wide { format!(" {name} ") } else { name.to_string() };
        spans.push(Span::styled(label, if sel { on } else { off }));
    }
    if wide {
        spans.push(Span::styled(" ▸", off));
    }
    Line::from(spans)
}

fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let lum = |(r, g, b): (u8, u8, u8)| {
        let f = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    };
    let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
    (hi + 0.05) / (lo + 0.05)
}

/// What `y` copies on the Traces page: the highlighted call as `Tool target`, or the highlighted prompt.
fn copy_text(f: &Follow) -> Option<String> {
    match (f.selected_call(), f.selected_prompt()) {
        (Some((_, c)), _) => Some(format!("{} {}", c.tool, c.what).trim_end().to_string()),
        (None, Some((text, _))) => Some(text.to_string()),
        _ => None,
    }
}

/// A page that has nothing to list yet says why on its first row.
fn note_rows(note: &str, ctx: &Ctx) -> Vec<Row> {
    vec![Row::plain(Line::from(Span::styled(
        truncate_str(note, ctx.width),
        Style::default().fg(ctx.theme.text_secondary),
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
        assert_eq!(p.page, Page::Traces);
        p.handle_key(&key(KeyCode::Tab));
        assert_eq!(p.page, Page::Agents);
        for _ in 0..3 {
            p.handle_key(&key(KeyCode::Right));
        }
        assert_eq!(p.page, Page::Traces, "off Traces, Right turns a page and four pages wrap");
        let pages: Vec<Page> = [KeyCode::Tab, KeyCode::Char('l'), KeyCode::Char('h'), KeyCode::BackTab]
            .into_iter()
            .map(|k| {
                p.handle_key(&key(k));
                p.page
            })
            .collect();
        assert_eq!(pages, [Page::Agents, Page::Recent, Page::Agents, Page::Traces], "l/Tab forward, h/Shift+Tab back");
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

    fn follow_items() -> Vec<serde_json::Value> {
        let item = |kind: &str, extra: serde_json::Value| {
            let mut v = serde_json::json!({"v": 1, "kind": kind, "agent": "root", "depth": 0, "line": 0, "ts": 1_000});
            v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap());
            v
        };
        vec![
            item("user", serde_json::json!({"text": "one\ntwo\nthree\nfour\nfive"})),
            item("call", serde_json::json!({"line": 3, "call_id": "t1", "tool": "Bash", "what": "cargo test"})),
            item("result", serde_json::json!({"line": 4, "call_id": "t1", "ok": true, "error": "", "ms": 400})),
            item("call", serde_json::json!({"line": 5, "call_id": "t2", "tool": "Read", "what": "src/main.rs"})),
            item("result", serde_json::json!({"line": 6, "call_id": "t2", "ok": true, "error": "", "ms": 100})),
        ]
    }

    #[test]
    fn on_traces_tab_turns_the_page_while_l_folds_and_leaves_the_page() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let mut p = SidePanel { open: true, focused: true, ..SidePanel::default() };
        p.follow_for_test(SessionRow { id: "root".into(), ..Default::default() }, &follow_items(), 2_000);
        for k in [KeyCode::Char('l'), KeyCode::Right, KeyCode::Char('h'), KeyCode::Left] {
            p.handle_key(&key(k));
            assert_eq!(p.page, Page::Traces, "{k:?} folds on Traces, it does not turn the page");
        }
        p.handle_key(&key(KeyCode::Tab));
        assert_eq!(p.page, Page::Agents);
        p.handle_key(&key(KeyCode::BackTab));
        assert_eq!(p.page, Page::Traces);
    }

    #[test]
    fn the_traces_page_draws_grouped_calls_and_folds_a_long_prompt() {
        let mut p = SidePanel { open: true, focused: true, ..SidePanel::default() };
        p.follow_for_test(SessionRow { id: "root".into(), ..Default::default() }, &follow_items(), 2_000);
        let area = Rect::new(0, 0, 46, 14);
        let mut buf = Buffer::empty(area);
        p.render(&mut buf, area, &Info::default(), &Theme::current(), 2_000);
        let text = (0..14)
            .map(|y| (0..46).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>();
        let at = |needle: &str| text.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle}: {text:#?}"));
        assert!(text[at("+2 lines")].contains("…"), "a 5-line prompt folds to 3 lines: {text:#?}");
        assert_eq!(at("Read"), at("Bash") + 1, "calls sit on consecutive rows: {text:#?}");
    }

    #[test]
    fn the_tab_bar_fits_every_panel_width_and_keeps_the_current_page_named() {
        let text = |w| tab_bar(Page::Traces, w, &Theme::current()).spans.iter().map(|s| s.content.to_string()).collect::<String>();
        for w in [21, 29, 30, 31, 37, 38, 39, 43] {
            assert!(text(w).chars().count() <= w, "{w}: {:?}", text(w));
            assert!(text(w).contains("Traces"));
        }
        assert_eq!(text(21), "S Traces A R");
        assert!(text(43).starts_with("◀ ") && text(43).ends_with('▸'));
        assert!(text(31).contains("Session  Traces  Agents  Recent"));
    }

    // ---- the chat's status row as the Traces page's last row ----

    use crate::acp::tracker::{TurnActivity, WaitingReason};
    use crate::app::agent::AgentState;
    use crate::views::turn_status::format_turn_timer;

    const PHASE_SECS: u64 = 125;

    fn running(activity: Option<TurnActivity>) -> TurnRow {
        TurnRow {
            state: AgentState::TurnRunning,
            activity,
            activity_started_at: Some(std::time::Instant::now() - Duration::from_secs(PHASE_SECS)),
            tick: 0,
            is_bash_turn: false,
            is_pending_user_input: false,
            goal_verifying: false,
        }
    }

    fn model_wait() -> TurnRow {
        running(Some(TurnActivity::Waiting(WaitingReason::Model)))
    }

    fn following() -> SidePanel {
        let mut p = SidePanel { open: true, focused: true, ..SidePanel::default() };
        p.follow_for_test(SessionRow { id: "root".into(), ..Default::default() }, &follow_items(), 2_000);
        p
    }

    /// The panel drawn `inner` columns wide and `h` rows tall, one string per row.
    fn traces_lines(p: &mut SidePanel, turn: Option<TurnRow>, inner: u16, h: u16) -> Vec<String> {
        let area = Rect::new(0, 0, inner + 3, h);
        let mut buf = Buffer::empty(area);
        let info = Info { turn, ..Info::default() };
        p.render(&mut buf, area, &info, &Theme::current(), 2_000);
        (0..h)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect()
    }

    /// The row above the footer, without the panel's left edge.
    fn status_row(lines: &[String]) -> String {
        lines[lines.len() - 2].trim_start_matches('│').trim_start().to_string()
    }

    fn spinner() -> String {
        crate::glyphs::braille_spinner_frames()[0].to_string()
    }

    fn timer() -> String {
        format_turn_timer(Duration::from_secs(PHASE_SECS))
    }

    #[test]
    fn no_turn_no_status_row_and_the_follow_view_keeps_the_height() {
        for inner in [43, 21] {
            let lines = traces_lines(&mut following(), None, inner, 14);
            assert_eq!(status_row(&lines), "", "inner {inner}: {lines:#?}");
            assert!(lines.iter().all(|l| !l.contains("Waiting")), "inner {inner}: {lines:#?}");
        }
        // The same follow view with a turn gives up exactly one row, the one above the footer.
        let without = traces_lines(&mut following(), None, 43, 14);
        let with = traces_lines(&mut following(), Some(model_wait()), 43, 14);
        assert_eq!(without[..12], with[..12], "rows above the status row do not move");
        assert_eq!(without[13], with[13], "the footer does not move");
    }

    #[test]
    fn model_wait_shows_spinner_label_and_timer() {
        let lines = traces_lines(&mut following(), Some(model_wait()), 43, 14);
        assert_eq!(status_row(&lines), format!("{} Waiting for response… {}", spinner(), timer()), "{lines:#?}");
    }

    #[test]
    fn narrow_row_cuts_the_label_with_an_ellipsis_and_never_the_timer() {
        let lines = traces_lines(&mut following(), Some(model_wait()), 21, 14);
        let row = status_row(&lines);
        assert!(row.starts_with(&format!("{} Waiting", spinner())), "{row:?}");
        assert!(row.contains('…') && !row.contains("response…"), "the label is cut: {row:?}");
        assert!(row.ends_with(&timer()), "the timer is whole: {row:?}");
        assert!(lines[12].chars().count() <= 24, "inside the panel: {:?}", lines[12]);
    }

    #[test]
    fn running_tool_shows_its_description_as_the_chat_does() {
        let tool = |description: Option<&str>| {
            running(Some(TurnActivity::ToolRunning {
                title: "Bash: rex capture".into(),
                description: description.map(str::to_string),
            }))
        };
        let lines = traces_lines(&mut following(), Some(tool(Some("Capture this pane"))), 43, 14);
        assert_eq!(status_row(&lines), format!("{} Capture this pane… {}", spinner(), timer()), "{lines:#?}");
        let lines = traces_lines(&mut following(), Some(tool(None)), 43, 14);
        let row = status_row(&lines);
        assert!(row.starts_with(&format!("{} Run ", spinner())) && row.ends_with(&timer()), "{row:?}");
        let long = "Capture this pane while a call is running and read the screen back";
        let lines = traces_lines(&mut following(), Some(tool(Some(long))), 21, 14);
        let row = status_row(&lines);
        assert!(row.contains('…') && row.ends_with(&timer()), "{row:?}");
    }

    #[test]
    fn replying_thinking_and_cancelling_use_the_chat_labels() {
        for (activity, state, label) in [
            (Some(TurnActivity::Thinking), AgentState::TurnRunning, "Thinking…"),
            (Some(TurnActivity::Responding), AgentState::TurnRunning, "Responding…"),
            (Some(TurnActivity::AutoCompacting), AgentState::TurnRunning, "Compacting…"),
            (None, AgentState::TurnCancelling, "Cancelling…"),
        ] {
            let mut turn = running(activity);
            turn.state = state;
            let lines = traces_lines(&mut following(), Some(turn), 43, 14);
            assert_eq!(status_row(&lines), format!("{} {label} {}", spinner(), timer()), "{lines:#?}");
        }
    }

    #[test]
    fn asking_the_user_hides_the_timer_as_the_chat_does() {
        let turn = running(Some(TurnActivity::ToolRunning { title: "Ask: which file".into(), description: None }));
        let lines = traces_lines(&mut following(), Some(turn), 43, 14);
        let row = status_row(&lines);
        assert!(row.contains("Waiting on answers") && !row.contains(&timer()), "{row:?}");
    }

    #[test]
    fn a_paused_follow_view_keeps_its_line_above_the_status_row_and_stays_paused() {
        let mut p = following();
        traces_lines(&mut p, Some(model_wait()), 43, 8);
        p.handle_key(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        for _ in 0..2 {
            let lines = traces_lines(&mut p, Some(model_wait()), 43, 8);
            assert!(lines[5].contains("paused"), "the follow view's own line is just above: {lines:#?}");
            assert_eq!(status_row(&lines), format!("{} Waiting for response… {}", spinner(), timer()), "{lines:#?}");
        }
        let Traces::Following(f, _) = &p.traces else { panic!("following") };
        assert!(f.paused(), "drawing the status row does not resume following");
    }

    #[test]
    fn the_status_row_is_drawn_unfocused_and_zoomed_too() {
        let focused = traces_lines(&mut following(), Some(model_wait()), 43, 14);
        let mut p = following();
        p.focused = false;
        let unfocused = traces_lines(&mut p, Some(model_wait()), 43, 14);
        assert_eq!(status_row(&focused), status_row(&unfocused));
        assert_eq!(unfocused[13].trim_start_matches('│').trim_start(), "F3 focus panel");
        // Zoomed, the panel is wider (inner 70 here); the same row at that width.
        let mut z = following();
        z.zoomed = true;
        let lines = traces_lines(&mut z, Some(model_wait()), 70, 14);
        assert_eq!(status_row(&lines), format!("{} Waiting for response… {}", spinner(), timer()));
    }

    #[test]
    fn the_note_and_failed_states_keep_the_status_row() {
        for (traces, note_start) in [
            (Traces::Idle, "wait"),
            (Traces::Failed("traces index unavailable".into()), "trac"),
        ] {
            for inner in [43, 21] {
                let mut p = SidePanel { open: true, focused: true, ..SidePanel::default() };
                p.traces = match &traces {
                    Traces::Failed(e) => Traces::Failed(e.clone()),
                    _ => Traces::Idle,
                };
                let info = Info { session_id: Some("s".into()), turn: Some(model_wait()), ..Info::default() };
                let area = Rect::new(0, 0, inner + 3, 10);
                let mut buf = Buffer::empty(area);
                p.render(&mut buf, area, &info, &Theme::current(), 2_000);
                let lines: Vec<String> = (0..10)
                    .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
                    .collect();
                assert!(lines[2].contains(note_start), "{lines:#?}");
                assert!(status_row(&lines).starts_with(&spinner()) && status_row(&lines).ends_with(&timer()), "{lines:#?}");
            }
        }
    }

    #[test]
    fn at_tiny_heights_the_status_row_wins_over_the_follow_view() {
        // 4 rows: tab bar, rule, one body row, footer. The one body row goes to the status row.
        let lines = traces_lines(&mut following(), Some(model_wait()), 43, 4);
        assert!(status_row(&lines).starts_with(&spinner()), "{lines:#?}");
        assert!(!lines.iter().any(|l| l.contains("cargo test") || l.contains("one")), "{lines:#?}");
    }

    #[test]
    fn other_pages_do_not_draw_the_status_row() {
        let mut p = following();
        p.page = Page::Recent;
        let lines = traces_lines(&mut p, Some(model_wait()), 43, 14);
        assert!(lines.iter().all(|l| !l.contains("Waiting for response")), "{lines:#?}");
    }
}
