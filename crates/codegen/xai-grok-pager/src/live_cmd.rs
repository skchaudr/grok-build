//! `grok live`: a standalone full-screen view of the running agents.
//!
//! No agent, auth or model boots here: a plain ratatui loop over `TracesModalState` in `Mode::Live`.
//! The 1s tick is `poll_live()` called from this loop, not the app's event loop, and every tick redraws.

use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use xai_grok_traces::{TraceIndex, default_db_path, remote};

use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::theme::Theme;
use crate::views::traces_modal::{
    Mode, TracesModalState, handle_traces_key, handle_traces_mouse, render_live, render_traces_modal,
};

const TICK: Duration = Duration::from_secs(1);
const NOT_HERE: &str = "not available in grok live";

/// Restores the terminal on drop, so every exit path (including unwinding) leaves it usable.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        )?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen, cursor::Show);
    let _ = out.write_all(xai_crash_handler::terminal::MOUSE_PASTE_RESET);
    let _ = out.flush();
    xai_tty_utils::restore_native_stderr();
}

/// Applies one terminal event. True when the screen needs a redraw.
fn step(state: &mut TracesModalState, ev: &Event) -> bool {
    match ev {
        Event::Key(key) if key.kind == KeyEventKind::Release => false,
        Event::Key(key) => {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                state.request_live_exit();
                return true;
            }
            match handle_traces_key(state, key) {
                InputOutcome::Action(Action::TracesNotice { text }) => state.status = Some(text),
                // No agent here: resume, copy, rescan and the rest have nothing to act on.
                InputOutcome::Action(_) | InputOutcome::ActionThenForward(_) => {
                    state.status = Some(NOT_HERE.into());
                }
                _ => {}
            }
            true
        }
        Event::Mouse(m) => !matches!(handle_traces_mouse(state, m), InputOutcome::Unchanged),
        Event::Resize(..) => true,
        _ => false,
    }
}

fn draw(terminal: &mut Terminal<CrosstermBackend<Stdout>>, state: &mut TracesModalState) -> Result<()> {
    let theme = Theme::current();
    terminal.draw(|frame| {
        let area = frame.area();
        if state.mode == Mode::Live {
            render_live(frame.buffer_mut(), area, state, &theme);
        } else {
            render_traces_modal(frame.buffer_mut(), area, state, true);
        }
    })?;
    Ok(())
}

pub fn run() -> Result<()> {
    // Opened before the terminal changes, so an index error is not lost; the empty state says `no index`.
    let index = TraceIndex::open(&default_db_path()).ok();
    let mut state = TracesModalState::open_live(index, remote::hosts(), true);
    crate::theme::cache::set(crate::theme::cache::resolve_initial_theme_no_osc11());
    xai_tty_utils::redirect_native_stderr();

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev(info);
    }));
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut next_tick = Instant::now() + TICK;
    draw(&mut terminal, &mut state)?;
    while !state.live_wants_exit() {
        let wait = next_tick.saturating_duration_since(Instant::now());
        let mut redraw = false;
        if event::poll(wait)? {
            redraw |= step(&mut state, &event::read()?);
        }
        if Instant::now() >= next_tick {
            next_tick = Instant::now() + TICK;
            redraw |= state.poll_live();
        }
        if redraw && !state.live_wants_exit() {
            draw(&mut terminal, &mut state)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: mods,
            kind,
            state: KeyEventState::NONE,
        })
    }

    fn state() -> TracesModalState {
        TracesModalState::open_live(None, vec![], true)
    }

    #[test]
    fn release_is_ignored() {
        let mut s = state();
        let ev = key(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Release);
        assert!(!step(&mut s, &ev));
        assert!(!s.live_wants_exit());
    }

    #[test]
    fn q_exits() {
        let mut s = state();
        let ev = key(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Press);
        assert!(step(&mut s, &ev));
        assert!(s.live_wants_exit());
    }

    #[test]
    fn ctrl_c_exits() {
        let mut s = state();
        let ev = key(KeyCode::Char('c'), KeyModifiers::CONTROL, KeyEventKind::Press);
        assert!(step(&mut s, &ev));
        assert!(s.live_wants_exit());
    }

    #[test]
    fn resize_redraws() {
        let mut s = state();
        assert!(step(&mut s, &Event::Resize(80, 24)));
        assert!(!s.live_wants_exit());
    }
}
