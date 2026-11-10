//! `/notify`: the Pi extension notifications this session received.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct NotifyCommand;

impl SlashCommand for NotifyCommand {
    fn name(&self) -> &str {
        "notify"
    }

    fn description(&self) -> &str {
        "Show notifications from Pi extensions in this session"
    }

    fn session_scoped(&self) -> bool {
        true
    }

    fn usage(&self) -> &str {
        "/notify"
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::ShowNotifications)
    }
}
