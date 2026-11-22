//! `/acpx`: browse acpx flow runs and sessions (`~/.acpx`), and start a flow.
//!
//! Ledger → run (graph + step outline) → step inspector; `r` replays a step's conversation as a transcript.
//! `/acpx run <file> [json]` starts a flow and follows it in a transcript card; `/acpx cancel` stops it.

use crate::acpx::launch::parse_run_args;
use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};

pub struct AcpxCommand;

impl SlashCommand for AcpxCommand {
    slash_meta! {
        name: "acpx",
        description: "Browse acpx flow runs and sessions, or run a flow (F4)",
        usage: "/acpx [run id] | /acpx run <flow file> [input json] | /acpx cancel",
        session_scoped: true,
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let args = args.trim();
        let (word, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
        match word {
            "" => CommandResult::Action(Action::OpenAcpx { run: None }),
            "cancel" if rest.trim().is_empty() => CommandResult::Action(Action::AcpxCancel),
            "run" => match parse_run_args(rest) {
                Ok((flow_file, input_json)) => CommandResult::Action(Action::AcpxRun { flow_file, input_json }),
                Err(message) => CommandResult::Error(message),
            },
            _ => CommandResult::Action(Action::OpenAcpx { run: Some(args.to_string()) }),
        }
    }
}
