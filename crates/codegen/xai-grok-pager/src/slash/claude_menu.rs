//! Slash-menu shaping for `grok claude` sessions (external agent label `claude*`).
//!
//! All Claude-specific data lives in this file; upstream command files stay untouched. Hooks:
//! `CommandRegistry` (hide + free names for the adapter) and `SlashController` (order + skill group).

use super::command::CommandResult;
use super::registry::{CommandRegistry, CommandSource, CommandTrigger};

/// Most-used commands, in menu order (30-day usage). Matches canonical names and aliases.
/// Names the session does not offer (`/status`, `/remote-control`, ...) are simply absent from the menu.
pub const TOP: &[&str] = &[
    "compact",
    "usage",
    "model",
    "copy",
    "exit",
    "resume",
    "new",
    "plugin",
    "rename",
    "effort",
    "settings",
    "export",
    "remote-control",
    "hooks",
    "fork",
    "mode",
    "status",
    "advisor",
    "login",
    "statusline",
    "reload-plugins",
    "artifacts",
    "rewind",
];

/// Claude Code's own commands as the adapter advertises them. Every other adapter command is a
/// skill or plugin command and sorts into the skill group below the real commands.
const ADAPTER_COMMANDS: &[&str] = &[
    "advisor",
    "agents",
    "auto-mode-setup",
    "autocompact",
    "color",
    "compact",
    "config",
    "context",
    "design-consent",
    "design-revoke",
    "effort",
    "extra-usage",
    "fast",
    "focus",
    "goal",
    "heapdump",
    "import",
    "init",
    "insights",
    "list-agents",
    "login",
    "mcp",
    "mode",
    "model",
    "output-style",
    "recap",
    "reload-plugins",
    "reload-skills",
    "rename",
    "security-review",
    "skill-doctor",
    "team-onboarding",
    "ultrareview",
    "usage",
    "usage-credits",
];

/// Builtins hidden from a Claude session's menu: those that send a grok-only `x.ai/*` request the stock
/// claude-agent-acp lacks. Typed in full they show a notice instead (see [`typed_builtin_override`]).
/// Hiding also frees their names, so the adapter's command of the same name shows instead:
/// `config-agents` (alias `agents`), `loop` (tool-gated off for Claude), `recap` (Grok-shell gate).
const HIDDEN: &[&str] = &[
    "login",
    "logout",
    "privacy",
    "feedback",
    "marketplace",
    "personas",
    "config-agents",
    // Not an `x.ai` request, but the name must stay free for the adapter's own `/loop`.
    "loop",
    "recap",
    // Builtins whose action is an `x.ai/*` request the adapter lacks; evidence in `.claude-menu-undecided.md`.
    "btw",
    "memory",
    "flush",
    "dream",
    "session-info",
    "share",
    "delete",
    "skills",
    "mcps",
    "workflows",
    "workflow",
    "imagine",
    "imagine-video",
    "auto",
];

/// Adapter plumbing that only server-launched sessions use.
const HIDDEN_ADAPTER: &[&str] = &["__remote-workflow", "workflow-launch-exec"];

fn listed(list: &[&str], name: &str) -> bool {
    list.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Whether `name` from `source` is hidden from a Claude session's menu.
pub fn hides(name: &str, source: CommandSource) -> bool {
    match source {
        CommandSource::Acp => listed(HIDDEN_ADAPTER, name),
        _ => listed(HIDDEN, name),
    }
}

/// What a Claude session does with a typed pager builtin: `None` runs it, `Some` replaces the run.
/// A hidden builtin would send its `x.ai/*` request, so it gets a notice instead. `recap` (reached via the
/// `summarize` alias once the adapter advertises it) goes to the adapter's `/recap`.
pub fn typed_builtin_override(canonical: &str, token: &str, args: &str) -> Option<CommandResult> {
    if !hides(canonical, CommandSource::Builtin) {
        return None;
    }
    Some(if canonical.eq_ignore_ascii_case("recap") {
        CommandResult::PassThrough(format!("/recap {args}").trim_end().to_string())
    } else {
        CommandResult::Message(format!("/{token} isn't available in Claude sessions"))
    })
}

/// Sort rank for a Claude session menu; lower sorts first. `0` outside Claude sessions.
pub fn trigger_rank(registry: &CommandRegistry, trigger: &CommandTrigger) -> usize {
    if !registry.is_claude_session() {
        return 0;
    }
    let aliases = registry
        .commands_by_index(trigger.command_index)
        .map_or(&[] as &[&str], |cmd| cmd.aliases());
    let is_skill = matches!(
        trigger.provenance,
        super::command::CommandProvenance::Skill { .. }
    );
    menu_rank(
        &trigger.canonical,
        aliases,
        trigger.source == CommandSource::Acp,
        is_skill,
    )
}

/// Whether the trigger belongs in the skill group of a Claude session menu.
pub fn trigger_is_skill(registry: &CommandRegistry, trigger: &CommandTrigger) -> bool {
    registry.is_claude_session() && trigger_rank(registry, trigger) == usize::MAX
}

fn menu_rank(canonical: &str, aliases: &[&str], is_acp: bool, skill_meta: bool) -> usize {
    let top = std::iter::once(canonical)
        .chain(aliases.iter().copied())
        .filter_map(|name| TOP.iter().position(|t| t.eq_ignore_ascii_case(name)))
        .min();
    match top {
        Some(i) => i,
        None if skill_meta || (is_acp && !listed(ADAPTER_COMMANDS, canonical)) => usize::MAX,
        None => TOP.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_commands_keep_listed_order_via_alias_too() {
        let compact = menu_rank("compact", &[], false, false);
        let exit = menu_rank("quit", &["exit"], false, false);
        let plugin = menu_rank("plugins", &["plugin"], false, false);
        assert!(compact < exit && exit < plugin);
        assert_eq!(menu_rank("usage", &["cost"], true, false), 1);
    }

    #[test]
    fn unlisted_commands_sit_between_top_and_skills() {
        let other_builtin = menu_rank("theme", &[], false, false);
        let other_adapter = menu_rank("fast", &[], true, false);
        let skill = menu_rank("small-business:tax-prep", &[], true, false);
        let bundled = menu_rank("simplify", &[], true, false);
        assert_eq!(other_builtin, TOP.len());
        assert_eq!(other_adapter, TOP.len());
        assert_eq!(skill, usize::MAX);
        assert_eq!(bundled, usize::MAX);
        assert_eq!(menu_rank("whatever", &[], false, true), usize::MAX);
    }

    #[test]
    fn hides_grok_only_builtins_but_not_adapter_commands() {
        assert!(hides("privacy", CommandSource::Builtin));
        assert!(hides("login", CommandSource::ExternalCommonStub));
        assert!(!hides("login", CommandSource::Acp));
        assert!(!hides("compact", CommandSource::Builtin));
        assert!(hides("__remote-workflow", CommandSource::Acp));
        assert!(!hides("__remote-workflow", CommandSource::Builtin));
    }

    #[test]
    fn typed_hidden_builtin_gets_a_notice_and_recap_goes_to_the_adapter() {
        assert!(matches!(
            typed_builtin_override("share", "share", ""),
            Some(CommandResult::Message(m)) if m == "/share isn't available in Claude sessions"
        ));
        assert!(matches!(
            typed_builtin_override("recap", "summarize", "short"),
            Some(CommandResult::PassThrough(t)) if t == "/recap short"
        ));
        assert!(typed_builtin_override("theme", "theme", "").is_none());
        // Pager-local entries dropped from HIDDEN run normally.
        for name in ["announcements", "tutorial", "docs", "voice"] {
            assert!(typed_builtin_override(name, name, "").is_none(), "{name}");
        }
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
        ) -> crate::slash::command::CommandResult {
            crate::slash::command::CommandResult::Handled
        }
    }

    fn menu_names(registry: &CommandRegistry) -> Vec<&str> {
        registry
            .triggers()
            .iter()
            .map(|t| t.display.as_str())
            .collect()
    }

    #[test]
    fn claude_session_hides_grok_only_and_frees_names_for_the_adapter() {
        use std::sync::Arc;
        let builtins: Vec<Arc<dyn crate::slash::command::SlashCommand>> = vec![
            Arc::new(Dummy("privacy", &[])),
            Arc::new(Dummy("config-agents", &["agents"])),
            Arc::new(Dummy("theme", &[])),
        ];
        let adapter = [
            agent_client_protocol::AvailableCommand::new("agents".to_string(), "x".to_string()),
            agent_client_protocol::AvailableCommand::new(
                "reload-plugins".to_string(),
                "x".to_string(),
            ),
        ];

        let mut native = CommandRegistry::new(builtins.clone());
        native.set_acp_commands(&adapter);
        assert!(menu_names(&native).contains(&"/privacy"));
        assert!(!menu_names(&native).contains(&"/reload-plugins"));

        let mut claude = CommandRegistry::new(builtins);
        claude.set_external_agent_with_label(true, Some("claude-agent".into()));
        claude.set_acp_commands(&adapter);
        let names = menu_names(&claude);
        assert!(!names.contains(&"/privacy"));
        assert!(!names.contains(&"/config-agents"));
        assert!(names.contains(&"/theme"));
        assert!(names.contains(&"/agents"), "adapter owns the freed alias");
        assert!(names.contains(&"/reload-plugins"));
        assert!(
            claude.get_for_dispatch("privacy").is_some(),
            "typed in full still resolves"
        );
    }

    fn acp(name: &str) -> agent_client_protocol::AvailableCommand {
        agent_client_protocol::AvailableCommand::new(name.to_string(), "x".to_string())
    }

    fn claude_registry(
        builtins: Vec<std::sync::Arc<dyn crate::slash::command::SlashCommand>>,
    ) -> CommandRegistry {
        let mut r = CommandRegistry::new(builtins);
        r.set_external_agent_with_label(true, Some("claude-agent".into()));
        r
    }

    #[test]
    fn shared_names_resolve_to_the_pager_builtin_once() {
        use std::sync::Arc;
        let mut r = claude_registry(vec![
            Arc::new(Dummy("model", &[])),
            Arc::new(Dummy("rename", &[])),
            Arc::new(Dummy("rewind", &[])),
        ]);
        r.set_acp_commands(&[acp("model"), acp("rename"), acp("rewind"), acp("mode")]);
        for name in ["model", "rename", "rewind"] {
            assert!(r.is_builtin(name), "/{name} must stay the pager builtin");
            let shown = r.triggers().iter().filter(|t| t.canonical == name).count();
            assert_eq!(shown, 1, "/{name} listed exactly once");
        }
        assert!(!r.is_builtin("mode"), "adapter-only names stay the adapter's");
    }

    #[test]
    fn recap_is_released_to_the_adapter_only_when_advertised() {
        use std::sync::Arc;
        let builtins: Vec<Arc<dyn crate::slash::command::SlashCommand>> =
            vec![Arc::new(Dummy("recap", &["summarize"]))];

        let mut with = claude_registry(builtins.clone());
        with.set_acp_commands(&[acp("recap")]);
        assert!(menu_names(&with).contains(&"/recap"));
        assert!(!with.is_builtin("recap"));

        let mut without = claude_registry(builtins.clone());
        without.set_acp_commands(&[]);
        assert!(!menu_names(&without).contains(&"/recap"));

        let mut native = CommandRegistry::new(builtins);
        native.set_acp_commands(&[acp("recap")]);
        assert!(!menu_names(&native).contains(&"/recap"), "native stays gated");
    }

    #[test]
    fn label_turning_claude_after_the_catalog_re_releases_names() {
        use std::sync::Arc;
        let mut r = CommandRegistry::new(vec![Arc::new(Dummy("config-agents", &["agents"]))]);
        r.set_external_agent_with_label(true, Some("agent".into()));
        r.set_acp_commands(&[acp("agents")]);
        assert!(r.is_builtin("agents"));
        r.set_external_agent_label("claude-agent");
        assert!(!r.is_builtin("agents"));
        assert!(menu_names(&r).contains(&"/agents"));
    }

    #[test]
    fn bare_slash_lists_top_commands_in_order_then_commands_then_skills() {
        use crate::acp::model_state::ModelState;
        use crate::slash::{SlashController, SlashState};
        use std::sync::Arc;

        let builtins: Vec<Arc<dyn crate::slash::command::SlashCommand>> = vec![
            Arc::new(Dummy("theme", &[])),
            Arc::new(Dummy("new", &[])),
            Arc::new(Dummy("quit", &["exit"])),
            Arc::new(Dummy("copy", &[])),
            Arc::new(Dummy("model", &[])),
        ];
        let mut registry = claude_registry(builtins);
        registry.set_acp_commands(&[
            acp("zeta-skill"),
            acp("fast"),
            acp("usage"),
            acp("alpha-skill"),
            acp("compact"),
        ]);
        let mut ctrl = SlashController::new(registry, std::path::PathBuf::from("."));
        ctrl.set_mru(std::rc::Rc::new(std::cell::RefCell::new(
            crate::slash::mru::SlashMru::new_in_memory(),
        )));
        let state = SlashState::default();
        ctrl.refresh(&state, "/", 1, &ModelState::default());
        let order: Vec<String> = state
            .snapshot()
            .matches
            .iter()
            .map(|r| r.display.clone())
            .collect();

        let at = |name: &str| order.iter().position(|d| d == name).expect(name);
        let top = ["/compact", "/usage", "/model", "/copy", "/quit", "/new"];
        assert_eq!(order[..top.len()], top, "{order:?}");
        for command in ["/theme", "/fast"] {
            let pos = at(command);
            assert!(pos >= top.len() && pos < at("/alpha-skill"), "{order:?}");
        }
        assert_eq!(order[order.len() - 2..], ["/alpha-skill", "/zeta-skill"], "{order:?}");
    }
}
