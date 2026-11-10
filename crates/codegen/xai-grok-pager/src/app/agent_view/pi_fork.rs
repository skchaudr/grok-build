//! Pi `/fork` message picker keys: it sits in the prompt slot like `/jump`.

use super::AgentView;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::views::fork_picker::{ForkPickerInput, handle_fork_picker_key, move_cursor};
use crossterm::event::KeyEvent;

impl AgentView {
    pub(super) fn handle_fork_picker_key(&mut self, key: &KeyEvent) -> InputOutcome {
        let Some(state) = self.fork_state.as_mut() else {
            return InputOutcome::Unchanged;
        };
        match handle_fork_picker_key(state, key) {
            ForkPickerInput::Select(entry_id) => {
                self.fork_state = None;
                InputOutcome::Action(Action::PiFork(Some(entry_id)))
            }
            ForkPickerInput::Dismissed => {
                self.fork_state = None;
                InputOutcome::Changed
            }
            ForkPickerInput::MoveUp => {
                move_cursor(state, -1);
                InputOutcome::Changed
            }
            ForkPickerInput::MoveDown => {
                move_cursor(state, 1);
                InputOutcome::Changed
            }
            ForkPickerInput::Consumed => InputOutcome::Unchanged,
        }
    }
}
