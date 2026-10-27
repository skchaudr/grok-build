pub(crate) fn pretty(raw: Result<Option<String>, String>) -> String {
    match raw {
        Ok(Some(line)) if !line.is_empty() => serde_json::from_str::<serde_json::Value>(&line)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or(line),
        Ok(_) => "(line no longer in the source file)".to_string(),
        Err(e) => format!("(could not read source: {e})"),
    }
}

/// String values under these keys are body text, in any harness format.
pub(crate) const TEXT_KEYS: [&str; 9] = [
    "text", "thinking", "output", "result", "stdout", "stderr", "command", "cmd", "script",
];
/// Tool inputs under these keys are walked for text. A string value is body text as-is.
pub(crate) const INPUT_KEYS: [&str; 4] = ["input", "arguments", "args", "params"];
