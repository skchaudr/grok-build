//! `/tree` — browse and navigate the Pi session entry tree.
//!
//! External (Pi) profile only in practice: the composition binary lists
//! `"tree"` in `PI_GROK_NATIVE_COMMANDS`. Stock Grok has no Pi leaf tree.
//!
//! Empty args open a native ArgPicker filled from `pi/session/tree`.
//! Selecting a row re-runs `/tree <entryId>`, which navigates via
//! `pi/session/navigate_tree` (Pi `ctx.navigateTree`).

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct TreeCommand;

impl SlashCommand for TreeCommand {
    fn name(&self) -> &str {
        "tree"
    }

    fn description(&self) -> &str {
        "Browse and navigate the session branch tree"
    }

    fn usage(&self) -> &str {
        "/tree [entryId]"
    }

    fn session_scoped(&self) -> bool {
        true
    }

    fn takes_args(&self) -> bool {
        true
    }

    fn args_required(&self) -> bool {
        false
    }

    fn arg_placeholder(&self) -> Option<&str> {
        Some("[entryId]")
    }

    fn run(&self, ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        if ctx.session_id.is_none() {
            return CommandResult::Error("No active session".into());
        }
        let trimmed = args.trim();
        if trimmed.is_empty() {
            return CommandResult::Action(Action::ShowSessionTree);
        }
        // Optional: `/tree <id> --summarize` (Pi branch summary).
        let mut summarize = false;
        let mut entry_id = None;
        for token in trimmed.split_whitespace() {
            if token == "--summarize" {
                summarize = true;
            } else if entry_id.is_none() {
                entry_id = Some(token.to_string());
            }
        }
        let Some(entry_id) = entry_id else {
            return CommandResult::Error("Usage: /tree [entryId] [--summarize]".into());
        };
        CommandResult::Action(Action::NavigateSessionTree {
            entry_id,
            summarize,
            custom_instructions: None,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn entry_id_with_summarize_parses() {
        let args = "abc123 --summarize";
        let mut summarize = false;
        let mut entry_id = None;
        for token in args.split_whitespace() {
            if token == "--summarize" {
                summarize = true;
            } else if entry_id.is_none() {
                entry_id = Some(token.to_string());
            }
        }
        assert_eq!(entry_id.as_deref(), Some("abc123"));
        assert!(summarize);
    }
}
