//! `/acpx`: browse acpx flow runs and sessions (`~/.acpx`).
//!
//! Ledger → run (graph + step outline) → step inspector; `r` replays a step's conversation as a transcript.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct AcpxCommand;

impl SlashCommand for AcpxCommand {
    slash_meta! {
        name: "acpx",
        description: "Browse acpx flow runs and sessions",
        usage: "/acpx [run id]",
        session_scoped: true,
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let run = Some(args.trim()).filter(|a| !a.is_empty()).map(str::to_string);
        CommandResult::Action(Action::OpenAcpx { run })
    }
}
