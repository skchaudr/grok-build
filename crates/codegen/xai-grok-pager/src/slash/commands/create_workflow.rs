use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

/// Upstream-aligned `/create-workflow` — author a project Rhai workflow.
pub struct CreateWorkflowCommand;

impl SlashCommand for CreateWorkflowCommand {
    fn name(&self) -> &str {
        "create-workflow"
    }

    fn description(&self) -> &str {
        "Author a new multi-agent workflow"
    }

    fn usage(&self) -> &str {
        "/create-workflow [goal]"
    }

    fn takes_args(&self) -> bool {
        true
    }

    fn args_required(&self) -> bool {
        false
    }

    fn arg_placeholder(&self) -> Option<&str> {
        Some("[goal]")
    }

    fn visible(&self, ctx: &crate::slash::command::AppCtx) -> bool {
        ctx.workflows_available
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let goal = args.trim();
        let goal_line = if goal.is_empty() {
            "Ask me what multi-agent workflow to author if the user did not specify a goal."
                .to_string()
        } else {
            format!("User goal: {goal}")
        };
        CommandResult::PassThrough(format!(
            "Author a new multi-agent Rhai workflow for this project.\n\n\
{goal_line}\n\n\
Requirements:\n\
- Save to `<repo>/.grok-pi/workflows/<name>.rhai` (or `$GROK_PROJECT_DIR/workflows/`).\n\
- User-global alternative: `$GROK_HOME/workflows/<name>.rhai` (default `~/.grok-pi/workflows/`).\n\
- Script MUST start with pure-literal `let meta = #{{ name: \"...\", description: \"...\", phases: [...] }};`\n\
- Host APIs: `agent(prompt)`, `parallel([...])`, `complete(value)`, `pause(kind, msg)`.\n\
- Prefer small agent budgets; name matches filename without `.rhai`.\n\
- After writing, launch with the `workflow` tool or `/workflow <name> [args]`.\n\
- Do not invent non-existent host APIs.\n"
        ))
    }
}
