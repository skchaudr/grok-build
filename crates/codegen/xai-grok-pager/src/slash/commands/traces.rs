//! `/traces`: browse local Claude, Codex, Pi, OMP, DeepSeek and Grok session traces.
//!
//! Ledger → trajectory → raw-line inspector over a local index (traces-ledger-v1).

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct TracesCommand;

impl SlashCommand for TracesCommand {
    slash_meta! {
        name: "traces",
        description: "Browse local agent session traces (Claude, Codex, Pi, OMP, DeepSeek, Grok) [traces-ledger-v1]",
        usage: "/traces",
        session_scoped: true,
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        if args.trim().is_empty() {
            CommandResult::Action(Action::OpenTraces)
        } else {
            CommandResult::Error("/traces takes no arguments. Open it, then press / to search.".to_string())
        }
    }
}
