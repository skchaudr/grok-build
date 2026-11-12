//! Slash-menu shaping for Prime Agent sessions (external agent label `prime` / `prime-agent`).
//!
//! Prime answers the core ACP surface plus its own `available_commands_update`; it does not answer
//! Grok's `_x.ai/*` service calls (`plugins/list`, `hooks/list`, `billing`, `suggestPrompt`, ...).
//! This profile hides the pager builtins whose action is one of those requests, hides the builtin
//! mode toggles that only target a session mode Prime may not publish, and forwards the commands
//! Prime intercepts itself (`/compact`) as prompt text. Adapter-advertised commands always win
//! over this table: `apply_acp_commands` releases every name the agent advertises.

use super::command::{CommandExecCtx, CommandProvenance, CommandResult, SlashCommand};

/// Builtins and common-agent stubs hidden from a Prime session menu. A typed hidden builtin gets a
/// notice ([`typed_builtin_override`]) instead of sending an `_x.ai/*` request Prime cannot answer.
/// Names Prime advertises are released to the adapter by `apply_acp_commands`, so this table only
/// decides what an agent with no catalog shows.
const HIDDEN: &[&str] = &[
    // Grok account and service features; Prime has no auth methods and no x.ai service surface.
    "login",
    "logout",
    "usage",
    "context",
    "plugins",
    "hooks",
    "skills",
    "marketplace",
    "workflows",
    "workflow",
    "mcps",
    "config-agents",
    "personas",
    "privacy",
    "feedback",
    "share",
    "delete",
    "remember",
    "imagine",
    "imagine-video",
    "loop",
    "recap",
    "session-info",
    "btw",
    // The builtin toggles target a session mode Prime publishes only when it advertises one;
    // `dispatch_cycle_mode` and `set_plan_mode` refuse at runtime when `available_modes` is empty.
    "plan",
    "plan-mode",
];

/// Builtins whose action is an `_x.ai/*` request Prime answers with `Method not found`; typing one
/// shows a notice instead of running it.
fn listed(name: &str) -> bool {
    HIDDEN.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Whether `name` from `source` is hidden from a Prime session's menu. Adapter-advertised commands
/// are never hidden.
pub fn hides(name: &str, source: super::registry::CommandSource) -> bool {
    source != super::registry::CommandSource::Acp && listed(name)
}

/// Commands Prime intercepts as prompt text. They stay in the menu without an adapter catalog.
pub fn forwards_command(name: &str) -> bool {
    name.eq_ignore_ascii_case("compact")
}

/// A typed hidden builtin would send a request Prime cannot answer, so it gets a notice instead.
pub fn typed_builtin_override(canonical: &str, token: &str) -> Option<CommandResult> {
    hides(canonical, super::registry::CommandSource::Builtin)
        .then(|| CommandResult::Message(format!("/{token} isn't available in Prime sessions")))
}

/// A pager stub for a command Prime answers from prompt text rather than ACP.
pub struct PrimeForwardedSlashCommand {
    name: String,
}

impl PrimeForwardedSlashCommand {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

impl SlashCommand for PrimeForwardedSlashCommand {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Compact the conversation context"
    }

    fn provenance(&self) -> CommandProvenance {
        CommandProvenance::Shell
    }

    fn takes_args(&self) -> bool {
        true
    }

    fn usage(&self) -> &str {
        &self.name
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let args = args.trim();
        if args.is_empty() {
            CommandResult::PassThrough(format!("/{}", self.name))
        } else {
            CommandResult::PassThrough(format!("/{} {args}", self.name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::registry::CommandSource;
    use crate::slash::command::CommandResult;

    #[test]
    fn hides_grok_service_and_mode_builtins_but_not_adapter_commands() {
        for name in [
            "usage",
            "context",
            "plugins",
            "hooks",
            "skills",
            "workflows",
            "marketplace",
            "plan",
        ] {
            assert!(hides(name, CommandSource::Builtin), "{name}");
            assert!(hides(name, CommandSource::ExternalCommonStub), "{name}");
            assert!(!hides(name, CommandSource::Acp), "{name}");
        }
        assert!(!hides("compact", CommandSource::ExternalCommonStub));
        assert!(!hides("model", CommandSource::Builtin));
    }

    #[test]
    fn typed_hidden_builtin_gets_a_notice() {
        assert!(matches!(
            typed_builtin_override("skills", "skills"),
            Some(CommandResult::Message(text)) if text == "/skills isn't available in Prime sessions"
        ));
        assert!(typed_builtin_override("model", "model").is_none());
        assert!(typed_builtin_override("compact", "compact").is_none());
    }

    struct Dummy(&'static str, &'static [&'static str]);

    impl crate::slash::command::SlashCommand for Dummy {
        fn name(&self) -> &str {
            self.0
        }
        fn aliases(&self) -> &[&str] {
            self.1
        }
        fn description(&self) -> &str {
            "dummy"
        }
        fn usage(&self) -> &str {
            self.0
        }
        fn run(
            &self,
            _ctx: &mut crate::slash::command::CommandExecCtx,
            _args: &str,
        ) -> CommandResult {
            CommandResult::Handled
        }
    }

    fn prime_registry() -> crate::slash::registry::CommandRegistry {
        use std::sync::Arc;
        let builtins: Vec<Arc<dyn crate::slash::command::SlashCommand>> = vec![
            Arc::new(Dummy("usage", &["cost"])),
            Arc::new(Dummy("plugins", &["plugin"])),
            Arc::new(Dummy("theme", &[])),
        ];
        let mut registry = crate::slash::registry::CommandRegistry::new(builtins);
        registry.set_external_agent_with_label(true, Some("prime-agent".into()));
        registry
    }

    fn names(registry: &crate::slash::registry::CommandRegistry) -> Vec<String> {
        registry
            .triggers()
            .iter()
            .map(|t| t.display.clone())
            .collect()
    }

    #[test]
    fn prime_profile_hides_grok_panels_and_forwards_compact() {
        let mut registry = prime_registry();
        let listed = names(&registry);
        for hidden in ["/usage", "/cost", "/plugins", "/plugin"] {
            assert!(!listed.contains(&hidden.to_string()), "{listed:?}");
        }
        assert!(listed.contains(&"/theme".to_string()));
        assert!(
            listed.contains(&"/compact".to_string()),
            "Prime's own command stays offered without a catalog: {listed:?}"
        );
        let models = crate::acp::model_state::ModelState::default();
        let bundle = crate::app::bundle::BundleState::default();
        let mut ctx = crate::slash::command::CommandExecCtx {
            models: &models,
            session_id: None,
            bundle_state: &bundle,
            screen_mode: crate::app::ScreenMode::Fullscreen,
            billing_surface_visible: false,
            usage_command_visible: false,
            pager_state: crate::settings::PagerLocalSnapshot::default(),
        };
        let compact = registry.get_for_dispatch("compact").expect("forwarded compact");
        assert!(matches!(
            compact.run(&mut ctx, ""),
            CommandResult::PassThrough(text) if text == "/compact"
        ));
        assert!(
            registry.get_for_dispatch("usage").is_some(),
            "a typed hidden builtin still resolves to the notice path"
        );
        assert!(matches!(
            typed_builtin_override("usage", "usage"),
            Some(CommandResult::Message(_))
        ));
    }

    #[test]
    fn advertised_names_replace_the_hidden_table_and_the_forward_stub() {
        use agent_client_protocol::AvailableCommand;
        let mut registry = prime_registry();
        registry.set_acp_commands(&[
            AvailableCommand::new("usage".to_string(), "Prime usage".to_string()),
            AvailableCommand::new("compact".to_string(), "Prime compact".to_string()),
        ]);
        let listed = names(&registry);
        assert!(listed.contains(&"/usage".to_string()), "{listed:?}");
        let usage = registry.get_for_dispatch("usage").expect("adapter usage");
        assert_eq!(usage.description(), "Prime usage");
        let compact = registry.get_for_dispatch("compact").expect("adapter compact");
        assert_eq!(compact.description(), "Prime compact");
    }

    #[test]
    fn compact_forwards_as_prompt_text() {
        assert!(forwards_command("compact"));
        assert!(!forwards_command("usage"));
        let cmd = PrimeForwardedSlashCommand::new("compact");
        let models = crate::acp::model_state::ModelState::default();
        let bundle = crate::app::bundle::BundleState::default();
        let mut ctx = crate::slash::command::CommandExecCtx {
            models: &models,
            session_id: None,
            bundle_state: &bundle,
            screen_mode: crate::app::ScreenMode::Fullscreen,
            billing_surface_visible: false,
            usage_command_visible: false,
            pager_state: crate::settings::PagerLocalSnapshot::default(),
        };
        assert!(matches!(
            cmd.run(&mut ctx, " now"),
            CommandResult::PassThrough(text) if text == "/compact now"
        ));
    }
}
