use anyhow::{Context, Result};
use std::{fs::File, io::Write};
use tempfile::NamedTempFile;

/// Materialize experimental Pi-native slash commands for the RPC host.
///
/// Opt-in via `PI_GROK_NATIVE_COMMANDS`. Uses Pi interactive selectors through
/// Remote TUI. Auth (`/login` / `/logout`) is a separate default-on package.
pub(super) fn write_native_commands_extension() -> Result<NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("pi-grok-native-commands-")
        .suffix(".ts")
        .tempfile()
        .context("create Pi native commands extension tempfile")?;
    const SOURCE: &str =
        include_str!("../../../../../../extensions/pi-grok-native-commands/index.ts");
    file.write_all(SOURCE.as_bytes())
        .context("write Pi native commands extension source")?;
    file.flush()
        .context("flush Pi native commands extension source")?;
    File::open(file.path())
        .and_then(|source| source.sync_all())
        .ok();
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_commands_extension_source_is_a_loadable_typescript_module() {
        let file = write_native_commands_extension().expect("write extension");
        let source = std::fs::read_to_string(file.path()).expect("read extension");
        assert!(source.contains("registerCommand(\"pi-model\""));
        assert!(source.contains("registerCommand(\"pi-resume\""));
        assert!(source.contains("registerCommand(\"pi-reload\""));
        assert!(!source.contains("registerCommand(\"pi-login\""));
        assert!(!source.contains("registerCommand(\"pi-logout\""));
        assert!(!source.contains("registerCommand(\"export-html\""));
        assert!(!source.contains("registerCommand(\"pi-share\""));
        assert!(source.contains("ModelSelectorComponent"));
        assert!(source.contains("SessionSelectorComponent"));
        assert_eq!(
            file.path().extension().and_then(|value| value.to_str()),
            Some("ts")
        );
    }
}
