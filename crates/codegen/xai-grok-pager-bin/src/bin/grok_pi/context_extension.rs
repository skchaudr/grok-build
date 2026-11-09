use anyhow::{Context, Result};
use std::{fs::File, io::Write, path::Path};
use tempfile::NamedTempFile;

/// Process-unique paths for the injected context breakdown bridge.
///
/// The extension source stays loaded for the Pi child lifetime. The breakdown
/// JSON path is rewritten on each `/context` / session-info request.
pub(super) struct ContextExtension {
    source: NamedTempFile,
    breakdown: NamedTempFile,
}

impl ContextExtension {
    pub(super) fn source_path(&self) -> &Path {
        self.source.path()
    }

    pub(super) fn breakdown_path(&self) -> &Path {
        self.breakdown.path()
    }
}

/// Materialize the private grok-pi context breakdown extension + output file.
pub(super) fn write_context_extension() -> Result<ContextExtension> {
    let mut source = tempfile::Builder::new()
        .prefix("pi-grok-context-")
        .suffix(".ts")
        .tempfile()
        .context("create Pi context extension tempfile")?;
    const SOURCE: &str = include_str!("../../../../../../extensions/pi-grok-context/index.ts");
    source
        .write_all(SOURCE.as_bytes())
        .context("write Pi context extension source")?;
    source
        .flush()
        .context("flush Pi context extension source")?;
    File::open(source.path())
        .and_then(|file| file.sync_all())
        .ok();

    let breakdown = tempfile::Builder::new()
        .prefix("pi-grok-context-breakdown-")
        .suffix(".json")
        .tempfile()
        .context("create Pi context breakdown tempfile")?;
    Ok(ContextExtension { source, breakdown })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_extension_source_is_a_loadable_typescript_module() {
        let extension = write_context_extension().expect("write extension");
        let source = std::fs::read_to_string(extension.source_path()).expect("read extension");
        assert!(source.contains("__pi_context_breakdown"));
        assert!(source.contains("PI_GROK_CONTEXT_BREAKDOWN"));
        assert!(source.contains("getSystemPrompt"));
        assert!(source.contains("getSystemPromptOptions"));
        assert!(source.contains("getAllTools"));
        assert!(source.contains("writeFileSync"));
        assert_eq!(
            extension
                .source_path()
                .extension()
                .and_then(|value| value.to_str()),
            Some("ts")
        );
        assert_eq!(
            extension
                .breakdown_path()
                .extension()
                .and_then(|value| value.to_str()),
            Some("json")
        );
    }
}
