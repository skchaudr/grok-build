//! Pi `/fork` list overlay: key/mouse handling (jump-style prompt shell).

use super::AgentView;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::views::fork_picker::{
    ForkPickerInput, fork_picker_activate, fork_picker_row_at, set_fork_picker_cursor,
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

impl AgentView {
    pub(crate) fn dismiss_fork_picker(&mut self) {
        self.fork_state = None;
    }

    pub(crate) fn fork_slot_taken(&self) -> bool {
        self.jump_state.is_some()
            || self.rewind_state.is_some()
            || self.inline_edit.is_some()
            || self.btw_state.is_some()
            || !self.no_input_overlay_pending()
    }

    pub(super) fn dismiss_fork_picker_if_suppressed(&mut self) -> bool {
        if self.fork_state.is_some() && self.fork_slot_taken() {
            self.dismiss_fork_picker();
            return true;
        }
        false
    }

    /// A picked or dismissed row closes the picker; a pick forks through `pi/session/fork` (`pi_session.rs`).
    fn fork_picker_input_to_outcome(&mut self, input: ForkPickerInput) -> InputOutcome {
        match input {
            ForkPickerInput::Select(entry_id) => {
                self.fork_state = None;
                InputOutcome::Action(Action::PiFork(Some(entry_id)))
            }
            ForkPickerInput::Dismissed => {
                self.fork_state = None;
                InputOutcome::Changed
            }
            ForkPickerInput::MoveUp | ForkPickerInput::MoveDown | ForkPickerInput::Consumed => {
                InputOutcome::Changed
            }
        }
    }

    pub(super) fn handle_fork_picker_mouse(&mut self, mouse: &MouseEvent) -> InputOutcome {
        let Some(state) = self.fork_state.as_mut() else {
            return InputOutcome::Unchanged;
        };
        let area = self.pane_areas.prompt;
        let Some(index) = fork_picker_row_at(state, area, mouse.column, mouse.row) else {
            return InputOutcome::Unchanged;
        };

        match mouse.kind {
            MouseEventKind::Moved => {
                if set_fork_picker_cursor(state, index) {
                    InputOutcome::Changed
                } else {
                    InputOutcome::Unchanged
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                set_fork_picker_cursor(state, index);
                let input = fork_picker_activate(state);
                self.fork_picker_input_to_outcome(input)
            }
            _ => InputOutcome::Unchanged,
        }
    }
}
