//! Extension shortcut manager modal.
//!
//! A centered overlay showing all Pi extension-registered shortcuts with
//! enable/disable toggling, remap via key capture, and conflict diagnostics.
//! Opened via `/pi-shortcut-manager`. Blocks input until closed with Esc.
//! Independent of remote-tui (extension component host).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::app::extension_shortcuts::{
    ExtensionShortcut, ExtensionShortcutRegistry, event_to_key_id,
};
use crate::theme::Theme;
use crate::views::modal_window::{
    ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowState, Shortcut,
    render_modal_window,
};

// ============================================================================
// State
// ============================================================================

/// Interaction mode within the shortcut manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Normal list navigation.
    Browse,
    /// Waiting for user to press a new key for remapping.
    RemapCapture { index: usize },
}

/// View state for the shortcut manager modal.
#[derive(Debug)]
pub struct ShortcutManagerModal {
    /// Modal chrome state (sizing, tabs).
    pub modal_state: ModalWindowState,
    /// Current interaction mode.
    mode: Mode,
    /// Snapshot of shortcuts at open time (for rendering).
    shortcuts: Vec<ExtensionShortcut>,
    /// Conflict map: key → other extension name.
    conflicts: std::collections::HashMap<String, String>,
    /// Toast message (transient feedback).
    toast: Option<String>,
    /// Whether any changes were made (to notify on close).
    dirty: bool,
    /// Scroll offset into the shortcut list.
    scroll_offset: usize,
    /// Currently selected row index.
    selected: usize,
    /// Last known visible row count (updated during render).
    visible_rows: usize,
}

impl ShortcutManagerModal {
    /// Create a new shortcut manager modal from the current registry state.
    pub fn new(registry: &ExtensionShortcutRegistry) -> Self {
        let shortcuts: Vec<ExtensionShortcut> = registry.all().into_iter().cloned().collect();
        let conflicts: std::collections::HashMap<String, String> = registry
            .conflicts()
            .into_iter()
            .map(|(key, conflict)| {
                let other = match conflict {
                    crate::app::extension_shortcuts::ShortcutConflict::Duplicate {
                        other_extension,
                    } => other_extension,
                };
                (key, other)
            })
            .collect();

        Self {
            modal_state: ModalWindowState::new(),
            mode: Mode::Browse,
            shortcuts,
            conflicts,
            toast: None,
            dirty: false,
            scroll_offset: 0,
            selected: 0,
            visible_rows: 10,
        }
    }

    /// Handle a key event. Returns `true` if the modal should close.
    pub fn handle_key(&mut self, key: &KeyEvent, registry: &mut ExtensionShortcutRegistry) -> bool {
        // Clear toast on any keypress
        self.toast = None;

        match self.mode {
            Mode::RemapCapture { index } => self.handle_remap_key(key, index, registry),
            Mode::Browse => self.handle_browse_key(key, registry),
        }
    }

    fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        if self.selected >= n {
            self.selected -= n;
        } else {
            self.selected = 0;
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let len = self.shortcuts.len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected + n).min(len - 1);
        if self.selected >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = self.selected.saturating_sub(self.visible_rows - 1);
        }
    }

    fn handle_browse_key(
        &mut self,
        key: &KeyEvent,
        registry: &mut ExtensionShortcutRegistry,
    ) -> bool {
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Char('q') if !key.modifiers.contains(KeyModifiers::CONTROL) => return true,

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_up(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_down(1);
            }
            KeyCode::Home => {
                self.scroll_offset = 0;
                self.selected = 0;
            }
            KeyCode::End => {
                let len = self.shortcuts.len();
                if len > 0 {
                    self.selected = len - 1;
                    if self.selected >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = self.selected.saturating_sub(self.visible_rows - 1);
                    }
                }
            }
            KeyCode::PageUp => {
                self.scroll_up(self.visible_rows);
            }
            KeyCode::PageDown => {
                self.scroll_down(self.visible_rows);
            }

            // Toggle enable/disable
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(shortcut) = self.shortcuts.get(self.selected) {
                    let key_id = shortcut.key.clone();
                    let new_state = !shortcut.enabled;
                    registry.set_enabled(&key_id, new_state);
                    if let Some(s) = self.shortcuts.get_mut(self.selected) {
                        s.enabled = new_state;
                    }
                    self.dirty = true;
                    self.toast = Some(format!(
                        "{} {}",
                        format_key_display(&key_id),
                        if new_state { "enabled" } else { "disabled" }
                    ));
                }
            }

            // Enter remap mode
            KeyCode::Char('r') | KeyCode::Char('R') => {
                let index = self.selected;
                if index < self.shortcuts.len() {
                    self.mode = Mode::RemapCapture { index };
                }
            }

            // Reset (remove remap, re-enable)
            KeyCode::Char('x') | KeyCode::Char('X') => {
                if let Some(shortcut) = self.shortcuts.get(self.selected) {
                    let key_id = shortcut.key.clone();
                    registry.set_remap(&key_id, None);
                    registry.set_enabled(&key_id, true);
                    if let Some(s) = self.shortcuts.get_mut(self.selected) {
                        s.remapped_to = None;
                        s.enabled = true;
                    }
                    self.dirty = true;
                    self.toast = Some(format!("{} reset to default", format_key_display(&key_id)));
                }
            }

            _ => {}
        }
        false
    }

    fn handle_remap_key(
        &mut self,
        key: &KeyEvent,
        index: usize,
        registry: &mut ExtensionShortcutRegistry,
    ) -> bool {
        // Esc cancels remap mode
        if key.code == KeyCode::Esc {
            self.mode = Mode::Browse;
            return false;
        }

        // Convert the pressed key to a Pi KeyId
        if let Some(new_key_id) = event_to_key_id(key) {
            if let Some(shortcut) = self.shortcuts.get(index) {
                let old_key_id = shortcut.key.clone();

                // Check for conflicts with other shortcuts
                let conflict = self.shortcuts.iter().find(|s| {
                    let effective = s.remapped_to.as_deref().unwrap_or(&s.key);
                    effective == new_key_id && s.key != old_key_id
                });

                if let Some(conflicting) = conflict {
                    self.toast = Some(format!(
                        "⚠ {} already used by {}",
                        format_key_display(&new_key_id),
                        conflicting.extension
                    ));
                    self.mode = Mode::Browse;
                    return false;
                }

                registry.set_remap(&old_key_id, Some(new_key_id.clone()));
                if let Some(s) = self.shortcuts.get_mut(index) {
                    s.remapped_to = Some(new_key_id.clone());
                }
                self.dirty = true;
                self.toast = Some(format!(
                    "{} → {}",
                    format_key_display(&old_key_id),
                    format_key_display(&new_key_id)
                ));
            }
        } else {
            self.toast = Some("Cannot capture this key".to_string());
        }

        self.mode = Mode::Browse;
        false
    }

    /// Whether changes were made during this session.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    // ── Rendering ──────────────────────────────────────────────────────────

    /// Render the modal into the given area.
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let shortcuts = [
            Shortcut {
                label: "↑↓ navigate",
                clickable: false,
                id: 0,
            },
            Shortcut {
                label: "Enter toggle",
                clickable: true,
                id: 1,
            },
            Shortcut {
                label: "R remap",
                clickable: true,
                id: 2,
            },
            Shortcut {
                label: "X reset",
                clickable: true,
                id: 3,
            },
            Shortcut {
                label: "Esc close",
                clickable: true,
                id: 4,
            },
        ];
        let config = ModalWindowConfig {
            title: "Extension Shortcuts",
            tabs: None,
            fold_info: None,
            sizing: ModalSizing {
                width_pct: 0.70,
                max_width: 72,
                min_width: 48,
                ..Default::default()
            },
            shortcuts: &shortcuts,
        };

        let Some(content_area) =
            render_modal_window(buf, area, &mut self.modal_state.clone(), &config, theme)
        else {
            return;
        };

        match self.mode {
            Mode::RemapCapture { .. } => {
                self.render_remap_prompt(content_area, buf, theme);
            }
            Mode::Browse => {
                self.render_list(content_area, buf, theme);
            }
        }
    }

    fn render_list(&self, area: ModalContentArea, buf: &mut Buffer, theme: &Theme) {
        let inner = area.content;
        if inner.height < 2 || inner.width < 20 {
            return;
        }

        if self.shortcuts.is_empty() {
            let msg = "No extension shortcuts registered";
            let x = inner.x + (inner.width.saturating_sub(msg.width() as u16)) / 2;
            let y = inner.y + inner.height / 2;
            buf.set_string(x, y, msg, Style::default().fg(theme.gray));
            return;
        }

        // Column widths
        let key_col: u16 = 14;
        let status_col: u16 = 4;
        let desc_col = inner
            .width
            .saturating_sub(key_col + status_col + 20)
            .max(16);

        // Header
        let header_y = inner.y;
        let header_style = Style::default().fg(theme.gray).add_modifier(Modifier::BOLD);
        buf.set_string(inner.x, header_y, "KEY", header_style);
        buf.set_string(inner.x + key_col, header_y, "STATUS", header_style);
        buf.set_string(
            inner.x + key_col + status_col,
            header_y,
            "DESCRIPTION",
            header_style,
        );

        // Separator
        let sep_y = header_y + 1;
        let sep = "─".repeat(inner.width as usize);
        buf.set_string(inner.x, sep_y, &sep, Style::default().fg(theme.gray));

        // Items
        let list_start_y = sep_y + 1;
        let visible_rows = (inner.height.saturating_sub(2)) as usize;
        let scroll_offset = self.scroll_offset;
        let selected = self.selected;

        for (i, shortcut) in self
            .shortcuts
            .iter()
            .skip(scroll_offset)
            .take(visible_rows)
            .enumerate()
        {
            let y = list_start_y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let is_selected = scroll_offset + i == selected;
            let row_style = if is_selected {
                Style::default()
                    .bg(theme.bg_highlight)
                    .fg(theme.text_primary)
            } else {
                Style::default().fg(theme.text_primary)
            };

            // Selection indicator
            if is_selected {
                buf.set_string(inner.x, y, "▸ ", Style::default().fg(theme.accent_user));
            }

            let x_offset = inner.x + 2;

            // Key column
            let effective_key = shortcut.remapped_to.as_deref().unwrap_or(&shortcut.key);
            let key_display = format_key_display(effective_key);
            let key_style = if shortcut.enabled {
                row_style
            } else {
                Style::default().fg(theme.gray)
            };
            buf.set_string(
                x_offset,
                y,
                &truncate_str(&key_display, key_col as usize - 1),
                key_style,
            );

            // Status column
            let status_x = x_offset + key_col;
            if !shortcut.enabled {
                buf.set_string(status_x, y, "○ off", Style::default().fg(theme.gray));
            } else if self.conflicts.contains_key(&shortcut.key) {
                buf.set_string(status_x, y, "⚠", Style::default().fg(theme.warning));
            } else {
                buf.set_string(status_x, y, "●", Style::default().fg(theme.accent_success));
            }

            // Description column
            let desc_x = status_x + status_col;
            let desc = &shortcut.description;
            let desc_style = if shortcut.enabled {
                row_style
            } else {
                Style::default().fg(theme.gray)
            };
            buf.set_string(
                desc_x,
                y,
                &truncate_str(desc, desc_col as usize),
                desc_style,
            );

            // Extension name (right-aligned, muted)
            let ext_name = &shortcut.extension;
            let ext_display = truncate_str(ext_name, 18);
            let ext_x = inner.x + inner.width - ext_display.width() as u16;
            if ext_x > desc_x + desc_col {
                buf.set_string(ext_x, y, &ext_display, Style::default().fg(theme.gray));
            }
        }

        // Toast
        if let Some(toast) = &self.toast {
            let toast_y = inner.y + inner.height - 1;
            buf.set_string(
                inner.x,
                toast_y,
                &truncate_str(toast, inner.width as usize),
                Style::default().fg(theme.accent_user),
            );
        }
    }

    fn render_remap_prompt(&self, area: ModalContentArea, buf: &mut Buffer, theme: &Theme) {
        let inner = area.content;
        let y = inner.y + inner.height / 2;
        let msg = "Press new key for remap (Esc to cancel)...";
        let x = inner.x + (inner.width.saturating_sub(msg.width() as u16)) / 2;
        buf.set_string(
            x,
            y,
            msg,
            Style::default()
                .fg(theme.accent_user)
                .add_modifier(Modifier::BOLD),
        );
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Format a Pi KeyId for display: "alt+t" → "Alt+T"
fn format_key_display(key: &str) -> String {
    key.split('+')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Truncate a string to fit within max_width characters.
fn truncate_str(s: &str, max_width: usize) -> String {
    if s.chars().count() <= max_width {
        return s.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let truncated: String = s.chars().take(max_width - 1).collect();
    format!("{}…", truncated)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_key_display_basic() {
        assert_eq!(format_key_display("alt+t"), "Alt+T");
        assert_eq!(format_key_display("ctrl+shift+x"), "Ctrl+Shift+X");
        assert_eq!(format_key_display("f5"), "F5");
        assert_eq!(format_key_display("enter"), "Enter");
    }

    #[test]
    fn truncate_str_fits() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello w…");
        assert_eq!(truncate_str("hi", 2), "hi");
    }
}
