//! `S`: search the cass index on another machine from the traces screen.
//!
//! * `S` opens a one-line prompt over a results screen. Enter asks `cass` (host from `~/.grok/traces/cass`) on a
//!   worker thread: `cass::search` over ssh, then `cass::join` against the local index. The 1 s poll collects the
//!   answer, like the remote host fetches do. Every search carries a generation; an answer from an older one is
//!   dropped, and a new search replaces a running one.
//! * One row per hit in cass order. A hit that joined a local session opens its trajectory with the selection on
//!   the hit's line (or the nearest shown event before it); any other hit says it is not on this machine.
//! * The ledger or board underneath is never touched, so Esc restores it exactly. Esc in a trajectory opened from
//!   a hit returns to the results.
//! * The lexical / semantic / hybrid mode is left at cass's default: no key for it.

use std::collections::HashSet;
use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;
use xai_grok_traces::cass::{self, Joined, Query};
use xai_grok_traces::{TraceIndex, remote};

use super::board::split_at_width;
use super::{Mode, TracesModalState, Typing, WHEEL, edit_line, fmt_age, keep_visible, move_sel, now_ms};
use crate::app::app_view::InputOutcome;
use crate::input::key::key;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::shortcuts_bar::{HintItem, ShortcutsBar};
use crate::views::traces_outline::OutlineKey;

/// Columns of the origin machine, the agent and the age in a hit row.
const ORIGIN_W: usize = 10;
const AGENT_W: usize = 8;
const AGE_W: usize = 4;
const PAGE: isize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// Prompt open, nothing asked yet.
    Idle,
    Loading,
    Ready,
    Failed(String),
}

/// The results screen. While it exists, `Mode::List` draws it in place of the board or ledger.
pub(super) struct View {
    /// The prompt's text.
    pub text: String,
    /// What the shown results answer.
    pub query: String,
    /// Label of the ssh target asked.
    pub host: String,
    pub phase: Phase,
    pub rows: Vec<Joined>,
    pub total: Option<i64>,
    pub sel: usize,
    scroll: usize,
}

impl View {
    fn new() -> Self {
        Self { text: String::new(), query: String::new(), host: String::new(), phase: Phase::Idle, rows: Vec::new(), total: None, sel: 0, scroll: 0 }
    }

    /// `<n> hits, <m> on this machine`.
    pub fn summary(&self) -> String {
        let local = self.rows.iter().filter(|j| j.id.is_some()).count();
        format!("{} hits, {local} on this machine", self.rows.len())
    }
}

/// A search for the worker thread.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Job {
    pub seq: u64,
    pub target: String,
    pub query: Query,
}

/// A worker's answer: the joined hits and cass's total, or the message to show.
pub(super) struct Msg {
    pub seq: u64,
    pub result: Result<(Vec<Joined>, Option<i64>), String>,
}

/// The machine a hit came from. cass calls its own machine `local`; that is the cass host, not this one.
fn origin(j: &Joined, host: &str) -> String {
    let s = j.hit.source_id.as_deref().or(j.hit.origin_host.as_deref()).map(str::trim).filter(|s| !s.is_empty());
    match s {
        Some("local") => host.to_string(),
        Some(s) => s.to_string(),
        None => "-".to_string(),
    }
}

/// A snippet on one line: every run of whitespace, newlines included, is one space.
fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl TracesModalState {
    /// `S`: the results screen with the prompt open, holding the last query to edit.
    pub(super) fn cass_prompt(&mut self) {
        let view = self.cass.get_or_insert_with(View::new);
        view.text = view.query.clone();
        self.typing = Typing::Cass;
    }

    /// Enter in the prompt, with the host already resolved. A new search replaces a running one (its answer is
    /// dropped by generation). `None` when there is nothing to run: an empty prompt, or no host (shown as the error).
    pub(super) fn cass_begin(&mut self, host: Result<String, String>) -> Option<Job> {
        self.typing = Typing::No;
        let text = self.cass.as_ref()?.text.trim().to_string();
        if text.is_empty() {
            // Nothing to ask: an untouched screen goes back to what it covered.
            if self.cass.as_ref().is_some_and(|v| v.phase == Phase::Idle) {
                self.cass = None;
            }
            return None;
        }
        self.cass_gen += 1;
        let seq = self.cass_gen;
        let view = self.cass.as_mut()?;
        view.query = text.clone();
        view.rows.clear();
        view.total = None;
        view.sel = 0;
        view.scroll = 0;
        match host {
            Err(msg) => {
                view.phase = Phase::Failed(msg);
                None
            }
            Ok(target) => {
                view.host = remote::label(&target).to_string();
                view.phase = Phase::Loading;
                Some(Job { seq, target, query: Query { text, limit: cass::DEFAULT_LIMIT, mode: None } })
            }
        }
    }

    fn cass_submit(&mut self) -> InputOutcome {
        let file = std::fs::read_to_string(cass::cass_path()).ok();
        let host = cass::resolve_host(None, file.as_deref());
        if let Some(job) = self.cass_begin(host) {
            self.cass_spawn(job);
        }
        InputOutcome::Changed
    }

    /// The search and the join run off the UI thread; the answer comes back through `cass_rx`.
    fn cass_spawn(&self, job: Job) {
        let tx = self.cass_tx.clone();
        let db = self.index.as_ref().map(|i| i.db_path().to_path_buf());
        std::thread::spawn(move || {
            let Job { seq, target, query } = job;
            let result = cass::search(&target, &query).map_err(|e| e.to_string()).map(|reply| {
                let total = reply.total_matches;
                let rows = match db.and_then(|p| TraceIndex::open(&p).ok()) {
                    Some(index) => cass::join(&index, reply.hits),
                    None => reply.hits.into_iter().map(Joined::bare).collect(),
                };
                (rows, total)
            });
            let _ = tx.send(Msg { seq, result });
        });
    }

    /// Installs a worker's answer unless a newer search (or leaving the screen) made it stale. True to redraw.
    pub(super) fn apply_cass(&mut self, msg: Msg) -> bool {
        if msg.seq != self.cass_gen {
            return false;
        }
        let Some(view) = self.cass.as_mut() else { return false };
        match msg.result {
            Ok((rows, total)) => {
                view.rows = rows;
                view.total = total;
                view.sel = 0;
                view.scroll = 0;
                view.phase = Phase::Ready;
            }
            Err(e) => {
                view.rows.clear();
                view.phase = Phase::Failed(e);
            }
        }
        true
    }

    /// Collect finished searches. True to redraw.
    pub(super) fn poll_cass(&mut self) -> bool {
        let mut redraw = false;
        while let Ok(msg) = self.cass_rx.try_recv() {
            redraw |= self.apply_cass(msg);
        }
        redraw
    }

    /// Esc on the results: the board or ledger underneath is exactly as it was. A search still running is cancelled.
    fn cass_leave(&mut self) {
        self.cass = None;
        self.cass_gen += 1;
        self.typing = Typing::No;
    }

    /// Enter on the selected hit.
    fn cass_enter(&mut self) -> InputOutcome {
        let Some(view) = self.cass.as_ref() else { return InputOutcome::Unchanged };
        let Some(hit) = view.rows.get(view.sel) else { return InputOutcome::Unchanged };
        match hit.id.clone() {
            Some(id) => {
                let line = hit.line;
                self.cass_open(&id, line);
            }
            None => {
                let note = format!("not on this machine: {}", origin(hit, &view.host));
                self.set_status(note);
            }
        }
        InputOutcome::Changed
    }

    /// Opens local session `id` as a ledger row does, then puts the selection on `line`.
    pub(super) fn cass_open(&mut self, id: &str, line: Option<i64>) {
        let Some(row) = self.index.as_ref().and_then(|i| i.session(id).ok().flatten()) else {
            self.set_status("that session is no longer in the local index");
            return;
        };
        self.trail.clear();
        self.live_return = false;
        self.open_session(row);
        if let Some(line) = line {
            self.focus_line(line);
        }
    }

    /// Selects the event at `line`, or the nearest shown one before it (the line may be a tool result folded into
    /// its call, or noise). Nothing before it: the top stays selected.
    pub(super) fn focus_line(&mut self, line: i64) {
        let shown: HashSet<&OutlineKey> = self.outline.nodes.iter().map(|n| &n.key).collect();
        let key = self.events.iter().rev().filter(|e| e.line <= line).map(|e| self.canon(OutlineKey::Ev(e.line))).find(|k| shown.contains(k));
        let Some(key) = key else { return };
        let Some(node) = self.outline.nodes.iter().position(|n| n.key == key) else { return };
        self.reveal(&key);
        self.reshow(node);
    }
}

/// Keys while the prompt has the keyboard.
pub(super) fn prompt_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    match key.code {
        KeyCode::Esc => {
            state.typing = Typing::No;
            // Cancelling leaves the results as they were; a screen that never searched goes away.
            if state.cass.as_ref().is_some_and(|v| v.phase == Phase::Idle) {
                state.cass = None;
            }
            InputOutcome::Changed
        }
        KeyCode::Enter => state.cass_submit(),
        _ => {
            let Some(view) = state.cass.as_mut() else { return InputOutcome::Unchanged };
            if edit_line(&mut view.text, key) { InputOutcome::Changed } else { InputOutcome::Unchanged }
        }
    }
}

/// Keys on the results (prompt closed).
pub(super) fn handle_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    // Plain-letter keys ignore Ctrl/Alt chords, as the rest of the screen does.
    if matches!(key.code, KeyCode::Char(_)) && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return InputOutcome::Unchanged;
    }
    let Some(view) = state.cass.as_mut() else { return InputOutcome::Unchanged };
    let len = view.rows.len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => state.cass_leave(),
        KeyCode::Char('S') => state.cass_prompt(),
        KeyCode::Enter => return state.cass_enter(),
        KeyCode::Down | KeyCode::Char('j') => move_sel(&mut view.sel, len, 1),
        KeyCode::Up | KeyCode::Char('k') => move_sel(&mut view.sel, len, -1),
        KeyCode::PageDown => move_sel(&mut view.sel, len, PAGE),
        KeyCode::PageUp => move_sel(&mut view.sel, len, -PAGE),
        KeyCode::Home | KeyCode::Char('g') => view.sel = 0,
        KeyCode::End | KeyCode::Char('G') => move_sel(&mut view.sel, len, isize::MAX / 2),
        _ => return InputOutcome::Unchanged,
    }
    InputOutcome::Changed
}

pub(super) fn handle_mouse(state: &mut TracesModalState, mouse: &MouseEvent) -> InputOutcome {
    let delta = match mouse.kind {
        MouseEventKind::ScrollDown => WHEEL,
        MouseEventKind::ScrollUp => -WHEEL,
        _ => return InputOutcome::Unchanged,
    };
    if let Some(view) = state.cass.as_mut() {
        let len = view.rows.len();
        move_sel(&mut view.sel, len, delta);
    }
    InputOutcome::Changed
}

/// Plain `S` (a Ctrl or Alt chord is not it).
pub(super) fn is_open_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('S') && !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

fn pad(s: &str, w: usize) -> String {
    let (t, _) = split_at_width(s, w);
    let gap = w.saturating_sub(t.width());
    format!("{t}{}", " ".repeat(gap))
}

/// One hit as a row of spans, `w` columns wide. A hit with no local session is all `dim`.
fn hit_line(j: &Joined, host: &str, now: i64, w: usize, theme: &Theme, bg: ratatui::style::Color) -> Line<'static> {
    let age = j.hit.created_at.map_or("-".to_string(), |t| fmt_age(now - t));
    let meta = format!(
        "{} {} {} ",
        pad(&origin(j, host), ORIGIN_W),
        pad(j.hit.agent.as_deref().unwrap_or("-"), AGENT_W),
        pad(&age, AGE_W)
    );
    let room = w.saturating_sub(meta.width());
    let snippet = flat(j.hit.snippet.as_deref().unwrap_or(""));
    let joined = j.id.is_some();
    let (meta_style, snippet_style) = if joined { (theme.muted().bg(bg), theme.muted().bg(bg)) } else { (theme.dim().bg(bg), theme.dim().bg(bg)) };
    let mut spans = vec![Span::styled(meta, meta_style)];
    let mut used = 0;
    if let Some(title) = j.session_title.as_deref().map(flat).filter(|t| !t.is_empty()) {
        let (t, _) = split_at_width(&title, room / 2);
        used += t.width();
        spans.push(Span::styled(t, Style::default().fg(theme.text_primary).bg(bg)));
        if room > used + 3 {
            spans.push(Span::styled(" — ", snippet_style));
            used += 3;
        }
    }
    if room > used {
        let (s, _) = split_at_width(&snippet, room - used);
        spans.push(Span::styled(s, snippet_style));
    }
    Line::from(spans)
}

pub(super) fn render(buf: &mut Buffer, area: Rect, state: &mut TracesModalState) {
    let theme = Theme::current();
    let bg = theme.bg_base;
    let base = Style::default().bg(bg);
    // The board or ledger under the results is already painted: wipe it.
    ratatui::widgets::Clear.render(area, buf);
    buf.set_style(area, base);
    if area.width < 30 || area.height < 6 {
        return;
    }
    let typing = state.typing == Typing::Cass;
    let status = state.status.clone();
    let Some(view) = state.cass.as_mut() else { return };
    let x = area.x + 2;
    let w = area.width.saturating_sub(4);

    let mut head = vec![
        Span::styled("traces ", Style::default().fg(theme.text_primary).bg(bg).add_modifier(Modifier::BOLD)),
        Span::styled(" Search ", Style::default().fg(theme.accent_user).bg(bg).add_modifier(Modifier::BOLD)),
    ];
    if !view.host.is_empty() {
        head.push(Span::styled(format!(" {}", view.host), theme.dim().bg(bg)));
    }
    buf.set_line(x, area.y, &Line::from(head), w);

    // The row under the header: the prompt while it is open, else the question and its tally.
    let (text, style) = if typing {
        (format!("cass: {}▏", view.text), Style::default().fg(theme.text_primary).bg(bg))
    } else {
        let tally = match &view.phase {
            Phase::Ready => format!(" · {}", view.summary()),
            Phase::Loading => " · searching…".to_string(),
            _ => String::new(),
        };
        (format!("\"{}\"{tally}   Esc back", view.query), Style::default().fg(theme.fuzzy_accent).bg(bg))
    };
    if typing || !view.query.is_empty() {
        buf.set_line(x, area.y + 1, &Line::from(Span::styled(truncate_str(&text, w as usize), style)), w);
    }

    let footer_y = area.y + area.height - 1;
    let status_h = u16::from(status.is_some());
    let list = Rect::new(x, area.y + 2, w, area.height.saturating_sub(3 + status_h));
    if let Some(msg) = status {
        buf.set_line(x, footer_y - 1, &Line::from(Span::styled(msg, theme.dim().bg(bg))), w);
    }
    let hints = if typing {
        vec![HintItem::new(key!(Enter), "search"), HintItem::new(key!(Esc), "cancel")]
    } else {
        vec![HintItem::new(key!('S'), "new search")]
    };
    let footer = Rect::new(area.x, footer_y, area.width, 1);
    ShortcutsBar::new(&hints).render(Rect { x: footer.x + 2, width: footer.width.saturating_sub(2), ..footer }, buf);

    // Notes in the list area, dim like the board's empty states; a failure in the error red.
    let note = |buf: &mut Buffer, text: String, style: Style| {
        buf.set_line(list.x, list.y, &Line::from(Span::styled(truncate_str(&text, w as usize), style.bg(bg))), w);
    };
    match &view.phase {
        Phase::Idle => return note(buf, "Type a query, Enter asks the cass host in ~/.grok/traces/cass.".into(), theme.dim()),
        Phase::Loading => return note(buf, format!("Searching {} for \"{}\"… up to 15 s", view.host, view.query), theme.dim()),
        Phase::Failed(e) => return note(buf, e.clone(), Style::default().fg(theme.accent_error)),
        Phase::Ready if view.rows.is_empty() => return note(buf, "No hits.".into(), theme.dim()),
        Phase::Ready => {}
    }
    let h = list.height as usize;
    view.sel = view.sel.min(view.rows.len() - 1);
    keep_visible(view.sel, &mut view.scroll, h);
    let now = now_ms();
    for (i, j) in view.rows.iter().enumerate().skip(view.scroll).take(h) {
        let y = list.y + (i - view.scroll) as u16;
        let selected = i == view.sel;
        let row_bg = if selected { theme.bg_highlight } else { bg };
        buf.set_style(Rect::new(x, y, w, 1), Style::default().bg(row_bg));
        let bar = if selected { crate::glyphs::selection_bar() } else { " " };
        buf.set_string(x, y, bar, Style::default().fg(theme.accent_user).bg(row_bg).add_modifier(Modifier::BOLD));
        let line = hit_line(j, &view.host, now, (w as usize).saturating_sub(1), &theme, row_bg);
        buf.set_line(x + 1, y, &line, w.saturating_sub(1));
    }
}

/// Whether the results screen is what `Mode::List` draws.
pub(super) fn shown(state: &TracesModalState) -> bool {
    state.mode == Mode::List && state.cass.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;
    use xai_grok_traces::cass::Hit;
    use xai_grok_traces::{EventRow, SessionRow};

    fn press(state: &mut TracesModalState, code: KeyCode) -> InputOutcome {
        super::super::handle_traces_key(state, &KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(state: &mut TracesModalState, text: &str) {
        for c in text.chars() {
            press(state, KeyCode::Char(c));
        }
    }

    fn board_state() -> TracesModalState {
        let mut state = TracesModalState::open_with_hosts(None, vec![]);
        state.board = Some(super::super::board::Board::new());
        state.mode = Mode::List;
        state.sessions = vec![SessionRow { id: "x".into(), source: "claude".into(), title: "local session".into(), ..Default::default() }];
        state.rebuild_ledger();
        state
    }

    fn hit(origin: &str, agent: &str, snippet: &str, age_ms: i64) -> Hit {
        Hit {
            agent: Some(agent.into()),
            source_id: Some(origin.into()),
            snippet: Some(snippet.into()),
            created_at: Some(now_ms() - age_ms),
            ..Default::default()
        }
    }

    fn joined(h: Hit, id: Option<&str>, line: Option<i64>, title: Option<&str>) -> Joined {
        Joined { hit: h, id: id.map(Into::into), line, source: id.map(|_| "claude".into()), session_title: title.map(Into::into) }
    }

    /// Opens the prompt, types `text`, and runs begin with a fixed host (no file read, no ssh).
    fn ask(state: &mut TracesModalState, text: &str) -> Option<Job> {
        press(state, KeyCode::Char('S'));
        if let Some(v) = state.cass.as_mut() {
            v.text.clear();
        }
        type_text(state, text);
        state.cass_begin(Ok("sab@sabibp".into()))
    }

    fn answer(seq: u64, rows: Vec<Joined>) -> Msg {
        let total = Some(rows.len() as i64 + 10);
        Msg { seq, result: Ok((rows, total)) }
    }

    fn draw(state: &mut TracesModalState) -> Vec<String> {
        let area = Rect::new(0, 0, 110, 14);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, state);
        (0..area.height).map(|y| (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn prompt_opens_types_and_cancels_back_to_the_board() {
        let mut state = board_state();
        let before = (state.query.clone(), state.selected, state.mode);
        press(&mut state, KeyCode::Char('S'));
        assert_eq!(state.typing, Typing::Cass);
        assert!(state.cass.is_some() && shown(&state));
        type_text(&mut state, "auth bug");
        assert_eq!(state.cass.as_ref().unwrap().text, "auth bug");
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.cass.as_ref().unwrap().text, "auth bu");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.typing, Typing::No);
        assert!(state.cass.is_none(), "cancelling a screen that never searched removes it");
        assert!(!super::super::wants_esc(&state));
        assert_eq!((state.query.clone(), state.selected, state.mode), before);
    }

    #[test]
    fn submit_runs_one_job_with_the_text_and_shows_loading() {
        let mut state = board_state();
        let job = ask(&mut state, "  auth bug ").expect("a job");
        assert_eq!(job.seq, 1);
        assert_eq!(job.target, "sab@sabibp");
        assert_eq!(job.query.text, "auth bug");
        assert_eq!(job.query.mode, None, "cass's own default mode");
        assert_eq!(state.typing, Typing::No);
        let view = state.cass.as_ref().unwrap();
        assert_eq!((view.phase.clone(), view.host.as_str()), (Phase::Loading, "sabibp"));
        let screen = draw(&mut state);
        assert!(screen.iter().any(|l| l.contains("Searching sabibp for \"auth bug\"")), "{screen:#?}");
    }

    #[test]
    fn empty_prompt_runs_nothing() {
        let mut state = board_state();
        assert!(ask(&mut state, "   ").is_none());
        assert!(state.cass.is_none(), "an untouched screen goes back to the board");
        assert_eq!(state.cass_gen, 0);
    }

    #[test]
    fn a_late_answer_from_an_older_search_is_dropped() {
        let mut state = board_state();
        let first = ask(&mut state, "one").unwrap();
        let second = ask(&mut state, "two").unwrap();
        assert_eq!((first.seq, second.seq), (1, 2));
        let stale = answer(first.seq, vec![joined(hit("sabibp", "claude", "old", 1000), None, None, None)]);
        assert!(!state.apply_cass(stale));
        assert_eq!(state.cass.as_ref().unwrap().phase, Phase::Loading, "still waiting for the second");
        assert!(state.cass.as_ref().unwrap().rows.is_empty());
        assert!(state.apply_cass(answer(second.seq, vec![joined(hit("sabibp", "claude", "new", 1000), None, None, None)])));
        assert_eq!(state.cass.as_ref().unwrap().rows.len(), 1);
        assert_eq!(state.cass.as_ref().unwrap().phase, Phase::Ready);
    }

    #[test]
    fn answers_arrive_through_the_poll_and_leaving_cancels_them() {
        let mut state = board_state();
        let job = ask(&mut state, "q").unwrap();
        state.cass_tx.send(answer(job.seq, vec![])).unwrap();
        assert!(state.poll_cass());
        assert_eq!(state.cass.as_ref().unwrap().phase, Phase::Ready);
        let job = ask(&mut state, "q2").unwrap();
        press(&mut state, KeyCode::Esc);
        assert!(state.cass.is_none());
        state.cass_tx.send(answer(job.seq, vec![])).unwrap();
        assert!(!state.poll_cass(), "the cancelled search's answer is dropped");
        assert!(state.cass.is_none());
    }

    #[test]
    fn hits_become_rows_in_cass_order_with_a_tally() {
        let mut state = board_state();
        let job = ask(&mut state, "needle").unwrap();
        let rows = vec![
            joined(hit("sab-air", "claude", "first\nsnippet   with\twhitespace", 3 * 3_600_000), Some("s1"), Some(7), Some("Fix the auth bug")),
            joined(hit("sabibp", "codex", "second snippet", 2 * 86_400_000), None, None, None),
            joined(hit("local", "pi", "third", 60_000), None, None, None),
        ];
        state.apply_cass(answer(job.seq, rows));
        assert_eq!(state.cass.as_ref().unwrap().summary(), "3 hits, 1 on this machine");
        let screen = draw(&mut state);
        let at = |needle: &str| screen.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} in {screen:#?}"));
        assert!(screen[at("\"needle\"")].contains("3 hits, 1 on this machine"));
        let (a, b, c) = (at("sab-air"), at("second snippet"), at("third"));
        assert!(a < b && b < c, "cass order");
        let first = &screen[a];
        assert!(first.contains("claude") && first.contains("3h") && first.contains("Fix the auth bug"), "{first}");
        assert!(first.contains("first snippet with whitespace"), "newlines and runs of blanks become one space: {first}");
        assert!(screen[b].contains("sabibp") && screen[b].contains("codex") && screen[b].contains("2d"), "{}", screen[b]);
        assert!(screen[c].contains("sabibp") && screen[c].contains("1m"), "cass's own `local` is the cass host: {}", screen[c]);
    }

    #[test]
    fn an_unjoined_hit_is_drawn_muted_and_a_joined_one_is_not() {
        let mut state = board_state();
        let job = ask(&mut state, "q").unwrap();
        state.apply_cass(answer(
            job.seq,
            vec![
                joined(hit("sab-air", "claude", "joined snippet", 1000), Some("s1"), None, Some("T")),
                joined(hit("sab-air", "claude", "orphan snippet", 1000), None, None, None),
            ],
        ));
        let area = Rect::new(0, 0, 110, 14);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &mut state);
        let theme = Theme::current();
        let fg_at = |needle: &str| {
            let y = (0..area.height).find(|&y| (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().contains(needle)).expect(needle);
            let line: String = (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
            let x = line[..line.find(needle).unwrap()].chars().count() as u16;
            buf[(x, y)].fg
        };
        assert_eq!(fg_at("orphan snippet"), theme.dim().fg.unwrap_or(ratatui::style::Color::Reset));
        assert_ne!(fg_at("joined snippet"), fg_at("orphan snippet"));
    }

    #[test]
    fn enter_on_an_unjoined_hit_says_where_it_is() {
        let mut state = board_state();
        let job = ask(&mut state, "q").unwrap();
        state.apply_cass(answer(job.seq, vec![joined(hit("sab-air", "claude", "x", 1000), None, None, None)]));
        let before = state.mode;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.status.as_deref(), Some("not on this machine: sab-air"));
        assert_eq!(state.mode, before, "nothing opens");
        assert!(state.cass.is_some());
    }

    #[test]
    fn enter_on_a_hit_missing_from_the_index_says_so() {
        let mut state = board_state();
        let job = ask(&mut state, "q").unwrap();
        state.apply_cass(answer(job.seq, vec![joined(hit("sabibp", "claude", "x", 1000), Some("gone"), Some(3), Some("T"))]));
        press(&mut state, KeyCode::Enter);
        assert!(state.status.as_deref().is_some_and(|s| s.contains("no longer in the local index")), "{:?}", state.status);
        assert_eq!(state.mode, Mode::List);
    }

    fn user(line: i64) -> EventRow {
        EventRow {
            line,
            event_type: "x".into(),
            kind: "message".into(),
            role: Some("user".into()),
            title: String::new(),
            preview: format!("prompt {line}"),
            timestamp: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: None,
            step: None,
            depth: 0,
            model: None,
            is_error: false,
            noise: false,
            detail: None,
            intent: None,
        }
    }

    fn with_role(kind: &str, role: &str, title: &str, line: i64) -> EventRow {
        EventRow { kind: kind.into(), role: Some(role.into()), title: title.into(), ..user(line) }
    }

    fn trajectory(rows: Vec<EventRow>) -> TracesModalState {
        let mut state = TracesModalState::open_with_hosts(None, vec![]);
        state.session = Some(SessionRow { id: "s1".into(), source: "claude".into(), ..Default::default() });
        state.mode = Mode::Trajectory;
        state.install_trajectory(rows, None, vec![]);
        state
    }

    fn selected_line(state: &TracesModalState) -> Option<i64> {
        match state.oshown.get(state.sel_pos()).and_then(|&n| state.key_at(n)) {
            Some(OutlineKey::Ev(l) | OutlineKey::Turn(l) | OutlineKey::Run(l)) => Some(*l),
            _ => None,
        }
    }

    #[test]
    fn focus_line_selects_the_event_or_the_nearest_shown_one_before_it() {
        let rows = vec![
            user(1),
            with_role("message", "assistant", "", 2),
            with_role("tool", "tool", "Bash", 3),
            with_role("tool", "tool_result", "", 4),
            with_role("message", "assistant", "", 5),
        ];
        let mut exact = trajectory(rows.clone());
        exact.focus_line(5);
        assert_eq!(selected_line(&exact), Some(5));

        let mut folded = trajectory(rows.clone());
        folded.focus_line(4);
        assert_eq!(folded.sel, Some(OutlineKey::Ev(3)), "the result is folded into its call: the call is selected");
        assert!(folded.oshown.iter().any(|&n| folded.key_at(n) == Some(&OutlineKey::Ev(3))), "and it is shown");

        let mut gap = trajectory(rows.clone());
        gap.focus_line(99);
        assert_eq!(selected_line(&gap), Some(5), "past the end: the last event");

        let mut before = trajectory(rows);
        let top = before.sel.clone();
        before.focus_line(0);
        assert_eq!(before.sel, top, "nothing at or before the line: the selection stays at the top");
    }

    #[test]
    fn a_joined_hit_opens_its_session_at_the_hit_line_and_esc_returns_to_the_results() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pi");
        std::fs::create_dir_all(&dir).unwrap();
        let msg = |ts: &str, text: &str| {
            format!("{{\"type\":\"message\",\"timestamp\":\"{ts}\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n")
        };
        let body: String = (1..=4).map(|i| msg(&format!("2026-10-01T00:00:0{i}Z"), &format!("prompt number {i}"))).collect();
        std::fs::write(dir.join("a.jsonl"), body).unwrap();
        let roots = [xai_grok_traces::Root { id: "pi".into(), source: "pi".into(), path: dir, format: xai_grok_traces::Format::Jsonl, file_name: None }];
        let db = tmp.path().join("idx/t.sqlite");
        TraceIndex::open(&db).unwrap().scan(&roots).unwrap();
        let mut state = TracesModalState::open_with_hosts(Some(TraceIndex::open(&db).unwrap()), vec![]);
        state.show_all = true;
        state.refresh();
        state.board = Some(super::super::board::Board::new());
        let id = state.sessions[0].id.clone();
        let before = (state.query.clone(), state.selected);

        let job = ask(&mut state, "prompt").unwrap();
        state.apply_cass(answer(job.seq, vec![joined(hit("sab-mini", "pi", "prompt number 3", 1000), Some(&id), Some(3), Some("a"))]));
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::Trajectory);
        assert_eq!(state.session.as_ref().map(|s| s.id.clone()), Some(id.clone()));
        assert_eq!(selected_line(&state), Some(3), "the selection sits on the hit's line");
        assert!(state.cass.is_some(), "the results wait behind the trajectory");

        press(&mut state, KeyCode::Esc);
        assert!(shown(&state), "Esc in the trajectory returns to the results");
        assert_eq!(state.cass.as_ref().unwrap().rows.len(), 1);
        press(&mut state, KeyCode::Esc);
        assert!(state.cass.is_none() && state.mode == Mode::List);
        assert_eq!((state.query.clone(), state.selected), before, "the list is as it was");

        // No line: the trajectory opens at the top.
        let job = ask(&mut state, "prompt").unwrap();
        state.apply_cass(answer(job.seq, vec![joined(hit("sab-mini", "pi", "x", 1000), Some(&id), None, Some("a"))]));
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::Trajectory);
        assert_eq!(state.sel_pos(), 0, "no line: the top");
    }

    #[test]
    fn esc_restores_the_ledger_filter_selection_and_scroll() {
        let mut state = board_state();
        state.sessions = (0..5).map(|i| SessionRow { id: format!("s{i}"), source: "claude".into(), title: format!("t{i}"), ..Default::default() }).collect();
        state.rebuild_ledger();
        state.query = "kept".into();
        state.deep = true;
        state.selected = 3;
        state.scroll = 2;
        let job = ask(&mut state, "needle").unwrap();
        state.apply_cass(answer(job.seq, vec![joined(hit("sab-air", "claude", "x", 1), None, None, None)]));
        press(&mut state, KeyCode::Char('j'));
        press(&mut state, KeyCode::Esc);
        assert!(state.cass.is_none());
        assert_eq!((state.query.as_str(), state.deep, state.selected, state.scroll), ("kept", true, 3, 2));
        assert_eq!(state.sessions.len(), 5);
        assert!(state.board.is_some());
    }

    #[test]
    fn errors_show_in_the_screen_and_never_open_anything() {
        let mut state = board_state();
        press(&mut state, KeyCode::Char('S'));
        type_text(&mut state, "needle");
        assert!(state.cass_begin(Err("no cass host: put an ssh target in ~/.grok/traces/cass".into())).is_none());
        let screen = draw(&mut state);
        assert!(screen.iter().any(|l| l.contains("no cass host: put an ssh target")), "{screen:#?}");
        assert_eq!(state.cass.as_ref().unwrap().phase, Phase::Failed("no cass host: put an ssh target in ~/.grok/traces/cass".into()));

        press(&mut state, KeyCode::Char('S'));
        let job = state.cass_begin(Ok("sab@sabibp".into())).expect("the last query is back in the prompt");
        state.apply_cass(Msg { seq: job.seq, result: Err("sabibp unreachable (timed out after 15s)".into()) });
        let screen = draw(&mut state);
        assert!(screen.iter().any(|l| l.contains("sabibp unreachable (timed out after 15s)")), "{screen:#?}");
        assert!(state.cass.as_ref().unwrap().rows.is_empty());
        // The failure leaves a way out.
        press(&mut state, KeyCode::Esc);
        assert!(state.cass.is_none());
    }

    #[test]
    fn s_on_the_results_edits_the_last_query_and_ctrl_s_does_nothing() {
        let mut state = board_state();
        let job = ask(&mut state, "first").unwrap();
        state.apply_cass(answer(job.seq, vec![]));
        let ctrl = KeyEvent::new_with_kind(KeyCode::Char('S'), KeyModifiers::CONTROL, KeyEventKind::Press);
        assert!(!is_open_key(&ctrl));
        press(&mut state, KeyCode::Char('S'));
        assert_eq!(state.typing, Typing::Cass);
        assert_eq!(state.cass.as_ref().unwrap().text, "first");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.typing, Typing::No);
        assert!(state.cass.is_some(), "cancelling keeps the results");
        assert!(super::super::wants_esc(&state), "Esc must reach the results, not quit the process");
    }
}
