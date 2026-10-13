//! Common slash commands offered for every external ACP agent (`grok pi`, `grok claude`, DSH).
//!
//! When the agent does not advertise a name, an unsupported stub stays in the menu and
//! returns a visible notice instead of falling back to Grok builtins.

use super::command::{CommandExecCtx, CommandProvenance, CommandResult, SlashCommand};

/// Names always listed for external-agent sessions. `/model` is handled by the pager's
/// model picker (ACP `model` config); it is not stubbed here.
pub const COMMON_EXTERNAL_AGENT_COMMANDS: &[&str] =
    &["usage", "context", "compact", "model", "login"];

/// Commands that get an unsupported stub when the agent does not advertise them.
pub const EXTERNAL_COMMON_STUB_COMMANDS: &[&str] = &["usage", "context", "compact", "login"];

/// Short agent name from an `--agent-cmd` line, for notices when the session reports no agent name.
/// Skips a leading `env` and its `VAR=value` pairs, takes the program's basename, and drops an `-acp` / `-code-acp` suffix.
pub fn agent_label_from_cmd(cmd: &str) -> Option<String> {
    if crate::acp::claude_subagents::command_is_claude_acp(cmd) {
        return Some("claude".to_string());
    }
    let mut words = cmd.split_whitespace().peekable();
    if words.peek() == Some(&"env") {
        words.next();
    }
    let program = words.find(|w| !w.contains('=') && !w.starts_with('-'))?;
    let base = program.rsplit('/').next().unwrap_or(program);
    let label = base
        .strip_suffix("-code-acp")
        .or_else(|| base.strip_suffix("-acp"))
        .unwrap_or(base);
    (!label.is_empty()).then(|| label.to_string())
}

fn description_for(name: &str) -> &'static str {
    match name {
        "usage" => "View token and cost usage",
        "context" => "View context window usage",
        "compact" => "Compact the conversation context",
        "login" => "Log in or re-authenticate",
        _ => "Agent command",
    }
}

/// Placeholder slash command shown when an external agent has not advertised the name.
pub struct UnsupportedExternalSlashCommand {
    name: String,
    agent_label: String,
}

impl UnsupportedExternalSlashCommand {
    pub fn new(name: &str, agent_label: &str) -> Self {
        Self {
            name: name.to_string(),
            agent_label: agent_label.to_string(),
        }
    }
}

impl SlashCommand for UnsupportedExternalSlashCommand {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        description_for(&self.name)
    }

    fn provenance(&self) -> CommandProvenance {
        CommandProvenance::Shell
    }

    fn usage(&self) -> &str {
        &self.name
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Message(format!(
            "/{} is not supported by {}",
            self.name, self.agent_label
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::model_state::ModelState;
    use crate::app::bundle::BundleState;
    use crate::slash::command::CommandExecCtx;

    #[test]
    fn agent_label_from_cmd_names_the_program() {
        assert_eq!(
            agent_label_from_cmd(
                "env CLAUDE_CODE_EXECUTABLE=/usr/bin/claude /home/u/.local/bin/claude-acp"
            ),
            Some("claude".to_string())
        );
        assert_eq!(agent_label_from_cmd("pi-acp"), Some("pi".to_string()));
        assert_eq!(
            agent_label_from_cmd("claude-code-acp"),
            Some("claude".to_string())
        );
        assert_eq!(
            agent_label_from_cmd("env DSH_ACP_MODEL=x dsh --profile acp-enhanced"),
            Some("dsh".to_string())
        );
        assert_eq!(
            agent_label_from_cmd("cursor-agent acp"),
            Some("cursor-agent".to_string())
        );
        assert_eq!(
            agent_label_from_cmd(
                "env CLAUDE_CODE_EXECUTABLE=/usr/bin/claude node /opt/claude-agent-acp/dist/index.js"
            ),
            Some("claude".to_string())
        );
        assert_eq!(agent_label_from_cmd("  "), None);
    }

    #[test]
    fn unsupported_stub_message_names_the_agent() {
        let cmd = UnsupportedExternalSlashCommand::new("usage", "pi");
        let models = ModelState::default();
        let bundle = BundleState::default();
        let mut ctx = CommandExecCtx {
            models: &models,
            session_id: None,
            bundle_state: &bundle,
            screen_mode: crate::app::ScreenMode::Fullscreen,
            billing_surface_visible: false,
            usage_command_visible: false,
            pager_state: crate::settings::PagerLocalSnapshot::default(),
        };
        let result = cmd.run(&mut ctx, "");
        assert!(matches!(
            result,
            CommandResult::Message(msg) if msg == "/usage is not supported by pi"
        ));
    }
}
