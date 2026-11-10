//! `/tree-map` — branch map: view main path forks with user messages, click to switch.
//!
//! Opens a native modal showing only user messages on the session tree,
//! with fork-point markers. Selecting a message navigates via Pi `ctx.navigateTree`.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct TreeMapCommand;

impl SlashCommand for TreeMapCommand {
    fn name(&self) -> &str {
        "tree-map"
    }

    fn description(&self) -> &str {
        "Branch map: view main path forks with user messages, click to switch"
    }

    fn usage(&self) -> &str {
        "/tree-map"
    }

    fn session_scoped(&self) -> bool {
        true
    }

    fn takes_args(&self) -> bool {
        false
    }

    fn run(&self, ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        if ctx.session_id.is_none() {
            return CommandResult::Error("No active session".into());
        }
        CommandResult::Action(Action::ShowTreeMap)
    }
}
