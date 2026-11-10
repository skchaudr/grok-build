//! Pi Remote TUI: an extension's `ctx.ui.custom()` screen (Pi's `/login` provider picker, selectors, demos).
//! grok-pi's remote-tui host renders it off-screen and streams frames as the `remote_tui` widget (or
//! `pi/ui/remote_tui` ops); while it is open, this session's keys go back to Pi instead of the prompt.
//! Ported from Dwsy's grok-pi pager (719ba57f `app_view.rs`, `render.rs`, `agent_status.rs`), held per session.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

use crate::theme::Theme;

/// Widget key carrying the component's frame lines.
pub const WIDGET_KEY: &str = "remote_tui";
/// Widget key carrying `{op: open|close, id}`: which component owns the keyboard.
pub const SESSION_WIDGET_KEY: &str = "__pi_grok_remote_tui_session__";
/// Widget key carrying the overlay placement JSON.
pub const LAYOUT_WIDGET_KEY: &str = "__pi_grok_remote_tui_layout__";

#[derive(Debug, Default)]
pub struct PiRemoteTui {
    /// Component that owns the keyboard; `None` hands it back to the prompt.
    pub id: Option<String>,
    /// Hosts that announce open/close on [`SESSION_WIDGET_KEY`]; their frames alone never arm input.
    session_managed: bool,
    /// `pi/ui/remote_tui` overlay stack; the top one is shown and gets keys.
    overlays: Vec<Overlay>,
    pub layout: Option<Layout>,
    /// The frame on screen.
    pub lines: Vec<String>,
}

#[derive(Debug)]
struct Overlay {
    id: String,
    title: Option<String>,
    lines: Vec<String>,
}

impl PiRemoteTui {
    pub fn is_open(&self) -> bool {
        self.id.is_some()
    }

    /// Floating over the transcript (`ctx.ui.custom(..., { overlay: true })`) instead of above the prompt.
    pub fn is_overlay(&self) -> bool {
        !self.lines.is_empty() && self.layout.as_ref().is_some_and(|layout| layout.overlay)
    }

    /// `__pi_grok_remote_tui_session__`: an old close cannot unlock the prompt under a newer component.
    pub fn set_session(&mut self, raw: &str) {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(raw) else {
            return;
        };
        let Some(id) = message.get("id").and_then(serde_json::Value::as_str) else {
            return;
        };
        match message.get("op").and_then(serde_json::Value::as_str) {
            Some("open") => self.id = Some(id.to_owned()),
            Some("close") if self.id.as_deref() == Some(id) => self.id = None,
            _ => {}
        }
        self.session_managed = true;
    }

    /// `remote_tui` widget frame (`None` clears it).
    pub fn set_frame(&mut self, lines: Option<Vec<String>>) {
        if self.session_managed {
            if self.id.is_none() && lines.is_some() {
                return;
            }
        } else if lines.is_some() {
            self.id.get_or_insert_with(|| "widget".to_owned());
        } else if self.id.as_deref() == Some("widget") {
            self.id = None;
        }
        self.lines = lines
            .filter(|lines| lines.iter().any(|line| !line.trim().is_empty()))
            .unwrap_or_default();
    }

    /// `pi/ui/remote_tui` open / frame / overlay_push / overlay_pop / close.
    pub fn apply(&mut self, op: &str, id: Option<String>, lines: Option<Vec<String>>, title: Option<String>) -> bool {
        match op {
            "open" | "overlay_push" => {
                let id = id.unwrap_or_else(|| match op {
                    "open" => "default".to_owned(),
                    _ => format!("overlay-{}", self.overlays.len()),
                });
                self.overlays.push(Overlay { id, title, lines: lines.unwrap_or_default() });
            }
            "frame" => match self.overlays.last_mut() {
                Some(top) => {
                    if let Some(lines) = lines {
                        top.lines = lines;
                    }
                    if title.is_some() {
                        top.title = title;
                    }
                    if let Some(id) = id {
                        top.id = id;
                    }
                }
                None => self.overlays.push(Overlay {
                    id: id.unwrap_or_else(|| "default".to_owned()),
                    title,
                    lines: lines.unwrap_or_default(),
                }),
            },
            "overlay_pop" => {
                self.overlays.pop();
            }
            "close" => match id {
                Some(id) => self.overlays.retain(|overlay| overlay.id != id),
                None => {
                    self.overlays.pop();
                }
            },
            _ => return false,
        }
        match self.overlays.last() {
            Some(top) => {
                self.id = Some(top.id.clone());
                self.lines = top
                    .title
                    .iter()
                    .filter(|title| !title.trim().is_empty())
                    .map(|title| format!("\x1b[1m{title}\x1b[0m"))
                    .chain(top.lines.iter().cloned())
                    .collect();
            }
            None => {
                self.id = None;
                self.lines.clear();
            }
        }
        true
    }

    /// Pi reload (`resetExtensionUI`) or the Ctrl+Shift+Esc escape hatch: drop the component.
    pub fn clear(&mut self) {
        self.id = None;
        self.overlays.clear();
        self.layout = None;
        self.lines.clear();
    }
}

/// Pi `TUI.showOverlay()` anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

/// Absolute cells or a percentage of the available terminal dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Cells(i32),
    Percent(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub overlay: bool,
    /// Width the component rendered its latest frame at.
    pub width: u16,
    pub max_height: Option<Position>,
    pub anchor: Anchor,
    pub row: Option<Position>,
    pub col: Option<Position>,
    pub offset_x: i32,
    pub offset_y: i32,
}

impl Layout {
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let width = value
            .get("width")
            .and_then(serde_json::Value::as_u64)
            .filter(|width| *width > 0)
            .map(|width| width.min(u64::from(u16::MAX)) as u16)?;
        let anchor = match value.get("anchor").and_then(serde_json::Value::as_str) {
            Some("top-left") => Anchor::TopLeft,
            Some("top-right") => Anchor::TopRight,
            Some("bottom-left") => Anchor::BottomLeft,
            Some("bottom-right") => Anchor::BottomRight,
            Some("top-center") => Anchor::TopCenter,
            Some("bottom-center") => Anchor::BottomCenter,
            Some("left-center") => Anchor::LeftCenter,
            Some("right-center") => Anchor::RightCenter,
            Some("center") | None => Anchor::Center,
            Some(_) => return None,
        };
        let clamp = |n: i64| n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let position = |key: &str| {
            let value = value.get(key)?;
            if let Some(cells) = value.as_i64() {
                return Some(Position::Cells(clamp(cells)));
            }
            let percent = value.as_str()?.strip_suffix('%')?.parse::<u16>().ok()?;
            Some(Position::Percent(percent.min(100)))
        };
        let offset = |key: &str| value.get(key).and_then(serde_json::Value::as_i64).map_or(0, clamp);
        Some(Self {
            overlay: value.get("overlay").and_then(serde_json::Value::as_bool).unwrap_or(false),
            width,
            max_height: position("maxHeight"),
            anchor,
            row: position("row"),
            col: position("col"),
            offset_x: offset("offsetX"),
            offset_y: offset("offsetY"),
        })
    }
}

fn axis(position: Option<Position>, available: u16, span: u16, fallback: u16) -> i32 {
    match position {
        Some(Position::Cells(value)) => value,
        Some(Position::Percent(percent)) => (u32::from(available.saturating_sub(span)) * u32::from(percent) / 100) as i32,
        None => i32::from(fallback),
    }
}

/// Where an overlay frame of `line_count` rows lands inside `area`.
pub fn overlay_rect(area: Rect, layout: &Layout, line_count: usize) -> Option<Rect> {
    if !layout.overlay || area.width == 0 || area.height == 0 || line_count == 0 {
        return None;
    }
    let width = layout.width.clamp(1, area.width);
    let max_height = layout
        .max_height
        .map_or(i32::from(area.height), |position| axis(Some(position), area.height, 0, area.height))
        .clamp(1, i32::from(area.height)) as u16;
    let height = line_count.min(usize::from(max_height)) as u16;
    let (free_x, free_y) = (area.width.saturating_sub(width), area.height.saturating_sub(height));
    let default_x = match layout.anchor {
        Anchor::TopLeft | Anchor::LeftCenter | Anchor::BottomLeft => 0,
        Anchor::TopRight | Anchor::RightCenter | Anchor::BottomRight => free_x,
        Anchor::TopCenter | Anchor::Center | Anchor::BottomCenter => free_x / 2,
    };
    let default_y = match layout.anchor {
        Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => 0,
        Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => free_y,
        Anchor::LeftCenter | Anchor::Center | Anchor::RightCenter => free_y / 2,
    };
    let x = axis(layout.col, area.width, width, default_x)
        .saturating_add(layout.offset_x)
        .clamp(0, i32::from(free_x)) as u16;
    let y = axis(layout.row, area.height, height, default_y)
        .saturating_add(layout.offset_y)
        .clamp(0, i32::from(free_y)) as u16;
    Some(Rect::new(area.x + x, area.y + y, width, height))
}

/// A key as the byte sequence Pi-TUI reads: legacy sequences for plain presses, Kitty CSI-u otherwise
/// so modifiers and repeat/release survive.
pub fn key_sequence(key: &KeyEvent) -> Option<String> {
    // Terminals deliver Shift+letter as uppercase text; Pi components bind the literal letter.
    if key.kind == KeyEventKind::Press
        && key.modifiers == KeyModifiers::SHIFT
        && let KeyCode::Char(c) = key.code
        && c.is_ascii_alphabetic()
    {
        return Some(c.to_ascii_uppercase().to_string());
    }
    if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
        if key.kind == KeyEventKind::Press && key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('c') => return Some("\u{3}".to_owned()),
                KeyCode::Char('d') => return Some("\u{4}".to_owned()),
                _ => {}
            }
        }
        return kitty_sequence(key);
    }
    Some(
        match key.code {
            KeyCode::Up => "\x1b[A",
            KeyCode::Down => "\x1b[B",
            KeyCode::Right => "\x1b[C",
            KeyCode::Left => "\x1b[D",
            KeyCode::Enter => "\r",
            KeyCode::Esc => "\x1b",
            KeyCode::Backspace => "\x7f",
            KeyCode::Tab => "\t",
            KeyCode::BackTab => "\x1b[Z",
            KeyCode::Home => "\x1b[H",
            KeyCode::End => "\x1b[F",
            KeyCode::PageUp => "\x1b[5~",
            KeyCode::PageDown => "\x1b[6~",
            KeyCode::Char(c) => return Some(c.to_string()),
            _ => return None,
        }
        .to_owned(),
    )
}

fn kitty_sequence(key: &KeyEvent) -> Option<String> {
    let event = match key.kind {
        KeyEventKind::Repeat => ":2",
        KeyEventKind::Release => ":3",
        KeyEventKind::Press => "",
    };
    let modifier = 1
        + u8::from(key.modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(key.modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(key.modifiers.contains(KeyModifiers::CONTROL))
        + 8 * u8::from(key.modifiers.contains(KeyModifiers::SUPER));
    let functional = match key.code {
        KeyCode::Up => Some(("1", 'A')),
        KeyCode::Down => Some(("1", 'B')),
        KeyCode::Right => Some(("1", 'C')),
        KeyCode::Left => Some(("1", 'D')),
        KeyCode::Home => Some(("1", 'H')),
        KeyCode::End => Some(("1", 'F')),
        KeyCode::PageUp => Some(("5", '~')),
        KeyCode::PageDown => Some(("6", '~')),
        KeyCode::Insert => Some(("2", '~')),
        KeyCode::Delete => Some(("3", '~')),
        _ => None,
    };
    if let Some((code, terminator)) = functional {
        return Some(format!("\x1b[{code};{modifier}{event}{terminator}"));
    }
    let codepoint = match key.code {
        KeyCode::Char(c) => c as u32,
        KeyCode::Enter => 13,
        KeyCode::Esc => 27,
        KeyCode::Backspace => 127,
        KeyCode::Tab | KeyCode::BackTab => 9,
        _ => return None,
    };
    Some(format!("\x1b[{codepoint};{modifier}{event}u"))
}

/// Drop APC strings (`ESC _ … BEL` or `ESC _ … ESC \\`), as a terminal does. Pi-TUI marks its cursor with them
/// (`_pi:c`, `_pi:fc`); ansi-to-tui would print them as text.
pub fn strip_apc(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("\x1b_") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("\x1b_") {
        out.push_str(&rest[..start]);
        let body = &rest[start + 2..];
        let end = [body.find('\x07').map(|i| i + 1), body.find("\x1b\\").map(|i| i + 2)]
            .into_iter()
            .flatten()
            .min();
        rest = end.map_or("", |end| &body[end..]);
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// One frame line in pager colours. Reverse video (Pi's input cursor) and the `→` selected row take the
/// theme highlight; Pi's 24-bit semantic colours and truecolor backgrounds (half-block art) are kept.
pub fn ansi_line(ansi: &str, theme: &Theme) -> Line<'static> {
    use ansi_to_tui::IntoText;
    // ansi-to-tui does not reliably clear a reverse background at `ESC[27m`; rewrite it to a plain dark
    // background with its own reset so the trailing padding does not inherit the cursor highlight.
    let normalized = strip_apc(ansi).replace("\x1b[7m", "\x1b[48;5;0m").replace("\x1b[27m", "\x1b[49m");
    let mut line = normalized
        .as_bytes()
        .into_text()
        .ok()
        .and_then(|text| text.lines.into_iter().next())
        .unwrap_or_else(|| Line::raw(ansi.to_owned()));
    let selected_row = line.spans.iter().any(|span| span.content.trim_start().starts_with('→'));
    for span in &mut line.spans {
        let reversed = span.style.add_modifier.contains(Modifier::REVERSED);
        span.style = span.style.remove_modifier(Modifier::REVERSED);
        let dark_fill = matches!(span.style.bg, Some(Color::Black | Color::DarkGray | Color::Indexed(0 | 8)));
        if reversed || dark_fill {
            span.style.fg = Some(theme.text_primary);
            span.style.bg = Some(theme.bg_highlight);
            continue;
        }
        span.style.fg = Some(match span.style.fg {
            Some(Color::Green) => theme.accent_success,
            Some(Color::Yellow) => theme.warning,
            Some(Color::Red) => theme.accent_error,
            Some(Color::Blue) => theme.accent_system,
            Some(Color::Magenta) => theme.accent_assistant,
            Some(Color::Cyan) => theme.running,
            None | Some(Color::Reset | Color::White) => theme.text_primary,
            Some(Color::Black | Color::DarkGray) => theme.gray,
            Some(Color::Gray) => theme.text_secondary,
            Some(color) => color,
        });
        span.style.bg = match span.style.bg {
            Some(Color::Rgb(..)) => span.style.bg,
            _ => Some(if selected_row { theme.bg_highlight } else { theme.bg_base }),
        };
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_marker_owns_the_keyboard_and_a_stale_close_does_not_release_it() {
        let mut tui = PiRemoteTui::default();
        tui.set_session(r#"{"op":"open","id":"a"}"#);
        tui.set_frame(Some(vec!["→ xAI".into()]));
        assert_eq!(tui.id.as_deref(), Some("a"));
        tui.set_session(r#"{"op":"open","id":"b"}"#);
        tui.set_session(r#"{"op":"close","id":"a"}"#);
        assert_eq!(tui.id.as_deref(), Some("b"));
        tui.set_session(r#"{"op":"close","id":"b"}"#);
        tui.set_frame(None);
        assert!(!tui.is_open() && tui.lines.is_empty());
        // Session-managed host: a frame with no open component does not grab the keyboard.
        tui.set_frame(Some(vec!["late".into()]));
        assert!(!tui.is_open() && tui.lines.is_empty());
    }

    #[test]
    fn overlay_stack_shows_the_top_and_closing_the_last_releases_input() {
        let mut tui = PiRemoteTui::default();
        tui.apply("open", Some("root".into()), Some(vec!["one".into()]), Some("Login".into()));
        tui.apply("overlay_push", Some("child".into()), Some(vec!["two".into()]), None);
        assert_eq!((tui.id.as_deref(), tui.lines.as_slice()), (Some("child"), &["two".to_owned()][..]));
        tui.apply("overlay_pop", None, None, None);
        assert_eq!(tui.lines, vec!["\x1b[1mLogin\x1b[0m".to_owned(), "one".to_owned()]);
        tui.apply("close", Some("root".into()), None, None);
        assert!(!tui.is_open() && tui.lines.is_empty());
    }

    #[test]
    fn keys_encode_as_pi_tui_reads_them() {
        let key = |code, modifiers| KeyEvent::new(code, modifiers);
        assert_eq!(key_sequence(&key(KeyCode::Down, KeyModifiers::NONE)).as_deref(), Some("\x1b[B"));
        assert_eq!(key_sequence(&key(KeyCode::Enter, KeyModifiers::NONE)).as_deref(), Some("\r"));
        assert_eq!(key_sequence(&key(KeyCode::Char('j'), KeyModifiers::SHIFT)).as_deref(), Some("J"));
        assert_eq!(key_sequence(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)).as_deref(), Some("\u{3}"));
        assert_eq!(key_sequence(&key(KeyCode::Up, KeyModifiers::ALT)).as_deref(), Some("\x1b[1;3A"));
    }

    #[test]
    fn overlay_rect_anchors_and_clamps() {
        let layout = Layout::from_json(&serde_json::json!({
            "overlay": true, "width": 40, "anchor": "bottom-right", "offsetX": 5
        }))
        .unwrap();
        assert_eq!(overlay_rect(Rect::new(0, 0, 80, 24), &layout, 4), Some(Rect::new(40, 20, 40, 4)));
        let centered = Layout::from_json(&serde_json::json!({ "overlay": true, "width": 20 })).unwrap();
        assert_eq!(overlay_rect(Rect::new(0, 0, 80, 24), &centered, 4), Some(Rect::new(30, 10, 20, 4)));
        assert!(Layout::from_json(&serde_json::json!({ "width": 0 })).is_none());
    }

    #[test]
    fn pi_cursor_markers_are_not_printed() {
        assert_eq!(strip_apc("> kim\x1b_pi:fc\x07\x1b[7m \x1b[27m\x1b_pi:/fc\x07 pad"), "> kim\x1b[7m \x1b[27m pad");
        assert_eq!(strip_apc("a\x1b_pi:c\x1b\\b"), "ab");
        let text: String = ansi_line("> kim\x1b_pi:fc\x07 \x1b_pi:/fc\x07", &Theme::current())
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, "> kim ");
    }

    #[test]
    fn ansi_line_highlights_the_selected_row_and_keeps_rgb() {
        let theme = Theme::current();
        let line = ansi_line("\x1b[36m→ selected option\x1b[0m", &theme);
        assert!(line.spans.iter().all(|span| span.style.bg == Some(theme.bg_highlight)));
        let line = ansi_line("\x1b[38;2;59;130;246mprovider\x1b[0m", &theme);
        assert!(line.spans.iter().any(|span| span.style.fg == Some(Color::Rgb(59, 130, 246))));
    }
}
