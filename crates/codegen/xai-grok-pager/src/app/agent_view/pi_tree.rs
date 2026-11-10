//! Pi `/tree` and `/tree-map` modals: keys, mouse and drawing. Ported from Dwsy's grok-pi pager
//! (719ba57f `app/modals.rs`); the screens themselves are `views::session_tree` and `views::tree_map`.

use super::AgentView;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::theme::Theme;
use crate::views::modal::ActiveModal;
use crate::views::modal_window::{self as mw, ModalSizing, ModalWindowConfig, ModalWindowState};
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

impl AgentView {
    pub(crate) fn handle_session_tree_mouse(
        state: &mut crate::views::session_tree::SessionTreeState,
        mouse: &crossterm::event::MouseEvent,
        skip_summary: bool,
    ) -> InputOutcome {
        use crate::views::session_tree::SessionTreeFocus;
        use crossterm::event::{MouseButton, MouseEventKind};
        use std::time::{Duration, Instant};

        // Simple double-click detection via thread-local last click.
        thread_local! {
            static LAST_CLICK: std::cell::RefCell<Option<(Instant, usize)>> = const { std::cell::RefCell::new(None) };
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if state.detail_expanded {
                    state.detail_scroll = state.detail_scroll.saturating_sub(1);
                } else {
                    state.move_selection(-1);
                }
                return InputOutcome::Changed;
            }
            MouseEventKind::ScrollDown => {
                if state.detail_expanded {
                    state.detail_scroll = state.detail_scroll.saturating_add(1);
                } else {
                    state.move_selection(1);
                }
                return InputOutcome::Changed;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = state.hit_test_list_row(mouse.column, mouse.row) {
                    let is_double = LAST_CLICK.with(|cell| {
                        let mut last = cell.borrow_mut();
                        let now = Instant::now();
                        let dbl = matches!(
                            *last,
                            Some((t, i)) if i == index && now.duration_since(t) < Duration::from_millis(400)
                        );
                        *last = Some((now, index));
                        dbl
                    });
                    state.selected = index;
                    state.detail_scroll = 0;
                    state.focus = SessionTreeFocus::List;
                    state.ensure_visible(state.list_viewport.max(1));
                    if is_double {
                        if let Some(entry_id) = state.selected_id() {
                            if Some(entry_id.as_str()) == state.leaf_id.as_deref() {
                                return InputOutcome::Changed;
                            }
                            if skip_summary {
                                return InputOutcome::Action(Action::NavigateSessionTree {
                                    entry_id,
                                    summarize: false,
                                    custom_instructions: None,
                                });
                            }
                            state.begin_summarize_prompt(entry_id);
                            return InputOutcome::Changed;
                        }
                    }
                    return InputOutcome::Changed;
                }
                InputOutcome::Unchanged
            }
            _ => InputOutcome::Unchanged,
        }
    }

    pub(crate) fn handle_session_tree_input(&mut self, ev: &crossterm::event::Event) -> InputOutcome {
        use crate::app::actions::{Action, SessionTreeFilter};
        use crate::views::session_tree::{SessionTreeEsc, SessionTreeFocus};
        use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

        let Event::Key(key) = ev else {
            return InputOutcome::Unchanged;
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return InputOutcome::Unchanged;
        }

        let Some(ActiveModal::SessionTree { state, .. }) = self.active_modal.as_mut() else {
            return InputOutcome::Unchanged;
        };

        // Skip-summary preference is stored on the tree state (set when the
        // modal opened) so we don't need AppView access here.
        let skip_summary_prompt = state.skip_summary_prompt;

        if matches!(state.focus, SessionTreeFocus::LabelEdit) {
            match key.code {
                KeyCode::Esc => {
                    let _ = state.clear_search_or_cancel_edit();
                    return InputOutcome::Changed;
                }
                KeyCode::Enter => {
                    let entry_id = state.selected_id();
                    let label = {
                        let draft = state.label_draft.trim().to_string();
                        if draft.is_empty() { None } else { Some(draft) }
                    };
                    state.focus = SessionTreeFocus::List;
                    state.label_draft.clear();
                    if let Some(entry_id) = entry_id {
                        return InputOutcome::Action(Action::LabelSessionTreeEntry {
                            entry_id,
                            label,
                        });
                    }
                    return InputOutcome::Changed;
                }
                KeyCode::Backspace => {
                    state.label_draft.pop();
                    return InputOutcome::Changed;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.label_draft.push(c);
                    return InputOutcome::Changed;
                }
                _ => return InputOutcome::Unchanged,
            }
        }

        if matches!(state.focus, SessionTreeFocus::SummarizePrompt) {
            match key.code {
                KeyCode::Esc => {
                    state.cancel_summarize();
                    return InputOutcome::Changed;
                }
                KeyCode::Up => {
                    state.summarize_move(-1);
                    return InputOutcome::Changed;
                }
                KeyCode::Down => {
                    state.summarize_move(1);
                    return InputOutcome::Changed;
                }
                KeyCode::Enter => {
                    use crate::views::session_tree::SummarizeConfirmAction;
                    match state.summarize_confirm() {
                        SummarizeConfirmAction::Navigate {
                            entry_id,
                            summarize,
                            custom_instructions,
                        } => {
                            return InputOutcome::Action(Action::NavigateSessionTree {
                                entry_id,
                                summarize,
                                custom_instructions,
                            });
                        }
                        SummarizeConfirmAction::EnterCustomEditor => {
                            return InputOutcome::Changed;
                        }
                    }
                }
                _ => return InputOutcome::Unchanged,
            }
        }

        if matches!(state.focus, SessionTreeFocus::SummarizeCustom) {
            match key.code {
                KeyCode::Esc => {
                    state.cancel_summarize();
                    return InputOutcome::Changed;
                }
                KeyCode::Enter => {
                    use crate::views::session_tree::SummarizeConfirmAction;
                    if let SummarizeConfirmAction::Navigate {
                        entry_id,
                        summarize,
                        custom_instructions,
                    } = state.summarize_custom_confirm()
                    {
                        return InputOutcome::Action(Action::NavigateSessionTree {
                            entry_id,
                            summarize,
                            custom_instructions,
                        });
                    }
                    return InputOutcome::Changed;
                }
                KeyCode::Backspace => {
                    state.summarize_custom_draft.pop();
                    return InputOutcome::Changed;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.summarize_custom_draft.push(c);
                    return InputOutcome::Changed;
                }
                _ => return InputOutcome::Unchanged,
            }
        }

        if state.detail_expanded {
            match key.code {
                KeyCode::Esc => {
                    state.detail_expanded = false;
                    state.focus = SessionTreeFocus::List;
                    return InputOutcome::Changed;
                }
                KeyCode::Up => {
                    state.detail_scroll = state.detail_scroll.saturating_sub(1);
                    return InputOutcome::Changed;
                }
                KeyCode::Down => {
                    state.detail_scroll = state.detail_scroll.saturating_add(1);
                    return InputOutcome::Changed;
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.detail_expanded = false;
                    state.focus = SessionTreeFocus::List;
                    return InputOutcome::Changed;
                }
                _ => {}
            }
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            KeyCode::Esc => match state.clear_search_or_cancel_edit() {
                SessionTreeEsc::Close => {
                    self.active_modal = None;
                    InputOutcome::Action(Action::SessionTreeClosed)
                }
                SessionTreeEsc::Consumed => InputOutcome::Changed,
            },
            KeyCode::Up => {
                state.move_selection(-1);
                InputOutcome::Changed
            }
            KeyCode::Down => {
                state.move_selection(1);
                InputOutcome::Changed
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                let _ = state.fold_or_navigate(crate::views::session_tree::FoldDirection::Up);
                InputOutcome::Changed
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                let _ = state.fold_or_navigate(crate::views::session_tree::FoldDirection::Down);
                InputOutcome::Changed
            }
            KeyCode::Left | KeyCode::PageUp => {
                state.page(-1, 10);
                InputOutcome::Changed
            }
            KeyCode::Right | KeyCode::PageDown => {
                state.page(1, 10);
                InputOutcome::Changed
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let dir = if matches!(key.code, KeyCode::BackTab) {
                    crate::views::session_tree::FoldDirection::Up
                } else {
                    crate::views::session_tree::FoldDirection::Down
                };
                let _ = state.fold_or_navigate(dir);
                InputOutcome::Changed
            }
            KeyCode::Enter => {
                if let Some(entry_id) = state.selected_id() {
                    if Some(entry_id.as_str()) == state.leaf_id.as_deref() {
                        self.show_toast("Already at this point");
                        InputOutcome::Changed
                    } else if skip_summary_prompt {
                        InputOutcome::Action(Action::NavigateSessionTree {
                            entry_id,
                            summarize: false,
                            custom_instructions: None,
                        })
                    } else {
                        state.begin_summarize_prompt(entry_id);
                        InputOutcome::Changed
                    }
                } else {
                    InputOutcome::Changed
                }
            }
            KeyCode::Char('c') if !ctrl => {
                if let Some(node) = state.selected_node() {
                    let text = if node.detail.is_empty() {
                        node.preview.clone()
                    } else {
                        node.detail.clone()
                    };
                    let _ = crate::clipboard::SystemClipboard::try_set(&text);
                    self.show_toast("Copied tree entry");
                }
                InputOutcome::Changed
            }
            KeyCode::Char('r') if !ctrl && !shift => {
                if let Some(entry_id) = state.selected_id() {
                    InputOutcome::Action(Action::RollbackFilesPreview { entry_id })
                } else {
                    InputOutcome::Changed
                }
            }
            KeyCode::Char('r') if shift && !ctrl => {
                if let Some(entry_id) = state.selected_id() {
                    InputOutcome::Action(Action::RollbackFilesExecute { entry_id })
                } else {
                    InputOutcome::Changed
                }
            }
            KeyCode::Char('l') if !ctrl => {
                state.begin_label_edit();
                InputOutcome::Changed
            }
            KeyCode::Char('t') if shift && !ctrl => {
                state.show_label_timestamps = !state.show_label_timestamps;
                InputOutcome::Changed
            }
            KeyCode::Char('o') if ctrl && shift => {
                state.cycle_filter_backward();
                InputOutcome::Changed
            }
            KeyCode::Char('o') if ctrl => {
                state.cycle_filter_forward();
                InputOutcome::Changed
            }
            KeyCode::Char('d') if ctrl => {
                state.set_filter(SessionTreeFilter::Default);
                InputOutcome::Changed
            }
            KeyCode::Char('t') if ctrl => {
                state.set_filter(SessionTreeFilter::NoTools);
                InputOutcome::Changed
            }
            KeyCode::Char('u') if ctrl => {
                state.set_filter(SessionTreeFilter::UserOnly);
                InputOutcome::Changed
            }
            KeyCode::Char('l') if ctrl => {
                state.set_filter(SessionTreeFilter::LabeledOnly);
                InputOutcome::Changed
            }
            KeyCode::Char('a') if ctrl => {
                state.set_filter(SessionTreeFilter::All);
                InputOutcome::Changed
            }
            KeyCode::Char('r') if ctrl => {
                state.detail_expanded = !state.detail_expanded;
                state.focus = if state.detail_expanded {
                    SessionTreeFocus::DetailExpanded
                } else {
                    SessionTreeFocus::List
                };
                state.detail_scroll = 0;
                InputOutcome::Changed
            }
            KeyCode::Char('/') if !ctrl => {
                state.focus = SessionTreeFocus::Search;
                InputOutcome::Changed
            }
            KeyCode::Backspace => {
                if !state.search_query.is_empty() {
                    state.search_query.pop();
                    state.folded.clear();
                    state.selected = 0;
                    state.scroll = 0;
                    state.clamp_selected();
                    InputOutcome::Changed
                } else {
                    InputOutcome::Unchanged
                }
            }
            KeyCode::Char(c) if !ctrl => {
                state.focus = SessionTreeFocus::Search;
                state.search_query.push(c);
                state.folded.clear();
                state.selected = 0;
                state.scroll = 0;
                state.clamp_selected();
                InputOutcome::Changed
            }
            _ => InputOutcome::Unchanged,
        }
    }

    pub(crate) fn handle_tree_map_input(&mut self, key: &KeyEvent) -> InputOutcome {
        use crossterm::event::{KeyCode, KeyEventKind};

        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return InputOutcome::Unchanged;
        }

        let Some(ActiveModal::TreeMap { state, .. }) = self.active_modal.as_mut() else {
            return InputOutcome::Unchanged;
        };

        match key.code {
            KeyCode::Esc => {
                self.active_modal = None;
                InputOutcome::Action(Action::SessionTreeClosed)
            }
            KeyCode::Up => {
                state.move_selection(-1);
                InputOutcome::Changed
            }
            KeyCode::Down => {
                state.move_selection(1);
                InputOutcome::Changed
            }
            KeyCode::PageUp => {
                state.move_selection(-10);
                InputOutcome::Changed
            }
            KeyCode::PageDown => {
                state.move_selection(10);
                InputOutcome::Changed
            }
            KeyCode::Enter => {
                if let Some(entry_id) = state.selected_entry_id() {
                    InputOutcome::Action(Action::NavigateSessionTree {
                        entry_id,
                        summarize: false,
                        custom_instructions: None,
                    })
                } else {
                    InputOutcome::Changed
                }
            }
            _ => InputOutcome::Unchanged,
        }
    }

    pub(crate) fn handle_tree_map_mouse(&mut self, mouse: &crossterm::event::MouseEvent) -> InputOutcome {
        use crate::views::modal_window as mw;
        use crate::views::modal_window::ModalWindowOutcome;
        use crossterm::event::{MouseButton, MouseEventKind};

        let Some(ActiveModal::TreeMap { state, window }) = self.active_modal.as_mut() else {
            return InputOutcome::Unchanged;
        };

        // Modal chrome (close button, etc.)
        let outcome = mw::handle_modal_mouse(window, mouse.kind, mouse.column, mouse.row);
        match outcome {
            ModalWindowOutcome::CloseRequested => {
                self.active_modal = None;
                return InputOutcome::Action(Action::SessionTreeClosed);
            }
            ModalWindowOutcome::Handled => return InputOutcome::Changed,
            ModalWindowOutcome::Unhandled => {}
            _ => return InputOutcome::Changed,
        }

        // Content area: row selection
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = state.hit_test_row(mouse.column, mouse.row) {
                    state.selected = index;
                    if let Some(entry_id) = state.selected_entry_id() {
                        return InputOutcome::Action(Action::NavigateSessionTree {
                            entry_id,
                            summarize: false,
                            custom_instructions: None,
                        });
                    }
                }
                InputOutcome::Changed
            }
            MouseEventKind::ScrollUp => {
                state.move_selection(-1);
                InputOutcome::Changed
            }
            MouseEventKind::ScrollDown => {
                state.move_selection(1);
                InputOutcome::Changed
            }
            _ => InputOutcome::Unchanged,
        }
    }
}

impl AgentView {
    /// `/tree` mouse: window chrome first (close button), then list click, scroll and double-click to go.
    pub(crate) fn handle_session_tree_modal_mouse(&mut self, mouse: &crossterm::event::MouseEvent) -> InputOutcome {
        let Some(ActiveModal::SessionTree { state, window }) = &mut self.active_modal else {
            return InputOutcome::Unchanged;
        };
        let skip_summary = state.skip_summary_prompt;
        match mw::handle_modal_mouse(window, mouse.kind, mouse.column, mouse.row) {
            mw::ModalWindowOutcome::CloseRequested => {
                self.active_modal = None;
                InputOutcome::Action(Action::SessionTreeClosed)
            }
            mw::ModalWindowOutcome::Unhandled => Self::handle_session_tree_mouse(state, mouse, skip_summary),
            _ => InputOutcome::Changed,
        }
    }
}

fn shortcuts(labels: &'static [&'static str]) -> Vec<mw::Shortcut<'static>> {
    labels.iter().map(|label| mw::Shortcut { label, clickable: false, id: 0 }).collect()
}

/// Draw `/tree` in a modal window.
pub(crate) fn draw_session_tree(
    buf: &mut Buffer,
    area: Rect,
    state: &mut crate::views::session_tree::SessionTreeState,
    window: &mut ModalWindowState,
    theme: &Theme,
    compact: bool,
) {
    let shortcuts = shortcuts(&["↑/↓ nav", "click select", "dblclick go", "Tab fold", "Esc close"]);
    let title = if state.loading {
        "Session tree · loading".to_string()
    } else {
        format!("Session tree · {}", state.filter.label())
    };
    let config = ModalWindowConfig {
        title: title.as_str(),
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing { width_pct: 0.72, max_width: 120, min_width: 56, v_margin: 2, h_pad: 1, v_pad: 0, footer_lines: 2 }
            .with_compact(compact),
        fold_info: None,
    };
    if let Some(content) = mw::render_modal_window(buf, area, window, &config, theme) {
        crate::views::session_tree::render_session_tree(buf, content.content, state, theme);
    }
}

/// Draw `/tree-map` in a modal window.
pub(crate) fn draw_tree_map(
    buf: &mut Buffer,
    area: Rect,
    state: &mut crate::views::tree_map::TreeMapState,
    window: &mut ModalWindowState,
    theme: &Theme,
    compact: bool,
) {
    let shortcuts = shortcuts(&["↑/↓ nav", "click go", "Esc close"]);
    let title = if state.loading {
        "Branch map · loading".to_string()
    } else {
        format!("Branch map · {} messages", state.nodes.len())
    };
    let config = ModalWindowConfig {
        title: title.as_str(),
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing { width_pct: 0.60, max_width: 90, min_width: 44, v_margin: 3, h_pad: 1, v_pad: 0, footer_lines: 1 }
            .with_compact(compact),
        fold_info: None,
    };
    if let Some(content) = mw::render_modal_window(buf, area, window, &config, theme) {
        crate::views::tree_map::render_tree_map(buf, content.content, state, theme);
    }
}

impl AgentView {
    /// `/notify` keys: type to search, e/→/Enter expand, ← collapse, y copy, Esc clears the search then closes.
    pub(crate) fn handle_notifications_input(&mut self, ev: &crossterm::event::Event) -> InputOutcome {
        use crate::views::picker::{PickerConfig, PickerOutcome, handle_picker_input};
        let Some(ActiveModal::Notifications { state, .. }) = self.active_modal.as_mut() else {
            return InputOutcome::Changed;
        };
        let count = state.filtered().len();
        let config = PickerConfig {
            title: None,
            show_search_hint: true,
            expandable: true,
            esc_clears_query: true,
            shortcuts: Some(crate::views::picker::picker_shortcuts()),
            pending_hint: None,
            non_selectable: &[],
            non_selectable_clickable: &[],
            shortcuts_area: None,
            tabs: None,
            active_tab: 0,
            filter_label: None,
            filter_key_hint: None,
            filter_active: false,
            header_note: None,
            action_keys: &[],
            disable_search: false,
            compact_bottom_bar: false,
            search_only_on_slash: false,
            // Browse-only list: vim normal mode would swallow type-to-search.
            vim_normal_first: false,
        };
        match handle_picker_input(ev, &mut state.picker, count, &config) {
            PickerOutcome::Closed => {
                self.active_modal = None;
                InputOutcome::Changed
            }
            PickerOutcome::Expand(i) | PickerOutcome::Selected(i) => {
                if !state.picker.expanded.insert(i) {
                    state.picker.expanded.remove(&i);
                }
                InputOutcome::Changed
            }
            PickerOutcome::Collapse(i) => {
                state.picker.expanded.remove(&i);
                InputOutcome::Changed
            }
            PickerOutcome::Copy(i) => {
                if let Some(text) = state.filtered().get(i).map(|n| n.message.clone()) {
                    let _ = crate::clipboard::SystemClipboard::try_set(&text);
                    self.show_toast("Copied notification");
                }
                InputOutcome::Changed
            }
            PickerOutcome::Unchanged => InputOutcome::Unchanged,
            _ => InputOutcome::Changed,
        }
    }
}

/// Draw `/notify`: first line per row with its kind on the right, the full message when expanded.
pub(crate) fn draw_notifications(
    buf: &mut Buffer,
    area: Rect,
    state: &mut crate::views::modal::NotificationListState,
    window: &mut ModalWindowState,
    theme: &Theme,
    compact: bool,
) {
    use crate::views::picker::{PickerEntry, PickerRow, render_picker_in_modal};
    let rows: Vec<(String, String, Vec<String>)> = state
        .filtered()
        .into_iter()
        .map(|n| {
            let body: Vec<String> = n.message.lines().map(str::to_owned).collect();
            let label = body.first().cloned().unwrap_or_default();
            (label, n.kind.clone().unwrap_or_else(|| "info".into()), body)
        })
        .collect();
    let bodies: Vec<Vec<&str>> = rows.iter().map(|(_, _, body)| body.iter().map(String::as_str).collect()).collect();
    let entries: Vec<PickerEntry> = rows
        .iter()
        .enumerate()
        .map(|(i, (label, kind, _))| {
            let expanded = state.picker.expanded.contains(&i);
            PickerEntry::Row(PickerRow {
                label,
                right_label: kind,
                selected: state.picker.hovered == Some(i) || (state.picker.hovered.is_none() && i == state.picker.selected),
                expanded,
                fields: &[],
                description_lines: if expanded { &bodies[i] } else { &[] },
                summary_lines: &[],
                dimmed: false,
                indent: 0,
                badge: "",
                badge_color: None,
                collapsible: true,
                underline_last_desc: false,
            })
        })
        .collect();
    let shortcuts = shortcuts(&["↑/↓ nav", "e/→/↵ expand", "← collapse", "y copy", "type search", "Esc close"]);
    let config = ModalWindowConfig {
        title: "Notifications",
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing { width_pct: 0.70, max_width: 120, min_width: 48, v_margin: 4, h_pad: 2, v_pad: 1, footer_lines: 2 }
            .with_compact(compact),
        fold_info: None,
    };
    if let Some(content) = mw::render_modal_window(buf, area, window, &config, theme) {
        render_picker_in_modal(buf, content.content, content.inner_x, content.inner_width, theme, &mut state.picker, &entries, &[], false);
    }
}
