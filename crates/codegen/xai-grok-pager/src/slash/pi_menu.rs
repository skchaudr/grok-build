//! Slash-menu shaping for Pi sessions (external agent label `pi` / `grok-pi`), beside [`super::claude_menu`].
//!
//! Hides builtins whose action needs the Grok service (an `x.ai/*` request grok-pi-acp does not answer) and frees
//! the names Pi's own extensions own. Builtins the adapter does answer (`/btw`, `/recap`, `/session-info`,
//! `/rename`, `/workflows`, `/compact`) stay.

use super::command::CommandResult;
use super::registry::CommandSource;

const HIDDEN: &[&str] = &[
    // Pi's auth and loop extensions register these names; the adapter's command takes them.
    "login",
    "logout",
    "loop",
    // Grok account and service features.
    "usage",
    "privacy",
    "feedback",
    "marketplace",
    "personas",
    "config-agents",
    "share",
    "delete",
    "skills",
    "mcps",
    "remember",
    "imagine",
    "imagine-video",
    "voice",
    "auto",
];

/// Builtins that only drive Pi (`pi/session/*`); every other session hides them.
const PI_ONLY: &[&str] = &["reload", "clone", "tree", "tree-map", "notify"];

fn listed(name: &str) -> bool {
    HIDDEN.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Whether builtin `name` is Pi-only, so a non-Pi session hides it.
pub fn pi_only(name: &str, source: CommandSource) -> bool {
    source == CommandSource::Builtin && PI_ONLY.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Whether pager builtin `name` is hidden from a Pi session's menu. Adapter commands and the pass-through stubs
/// for common agent commands are never hidden.
pub fn hides(name: &str, source: CommandSource) -> bool {
    source == CommandSource::Builtin && listed(name)
}

/// A typed hidden builtin would send a request Pi cannot answer, so it gets a notice instead of running.
pub fn typed_builtin_override(canonical: &str, token: &str) -> Option<CommandResult> {
    hides(canonical, CommandSource::Builtin)
        .then(|| CommandResult::Message(format!("/{token} isn't available in Pi sessions")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_grok_service_builtins_but_not_adapter_commands() {
        assert!(hides("usage", CommandSource::Builtin));
        assert!(hides("LOGIN", CommandSource::Builtin));
        assert!(!hides("login", CommandSource::Acp));
        assert!(!hides("btw", CommandSource::Builtin));
        assert!(!hides("resume", CommandSource::Builtin));
    }

    #[test]
    fn typed_hidden_builtin_gets_a_notice() {
        assert!(matches!(
            typed_builtin_override("usage", "usage"),
            Some(CommandResult::Message(text)) if text == "/usage isn't available in Pi sessions"
        ));
        assert!(typed_builtin_override("resume", "resume").is_none());
    }
}
