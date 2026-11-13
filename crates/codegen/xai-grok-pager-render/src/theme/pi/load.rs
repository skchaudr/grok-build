//! Parse and validate Pi theme JSON documents.

use std::path::Path;

use super::map::{MapError, map_pi_theme};
use super::schema::PiThemeJson;
use crate::theme::Theme;

/// Errors loading a Pi theme file or string.
#[derive(Debug)]
pub enum LoadError {
    Io(String),
    Json(String),
    InvalidName(String),
    Map(MapError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Json(e) => write!(f, "json error: {e}"),
            Self::InvalidName(n) => write!(f, "invalid theme name: {n}"),
            Self::Map(e) => write!(f, "map error: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Parse Pi theme JSON text into the validated document model.
pub fn load_from_str(json: &str) -> Result<PiThemeJson, LoadError> {
    let doc: PiThemeJson =
        serde_json::from_str(json).map_err(|e| LoadError::Json(e.to_string()))?;
    validate_name(&doc.name)?;
    Ok(doc)
}

/// Load a Pi theme JSON file from disk.
pub fn load_from_path(path: &Path) -> Result<PiThemeJson, LoadError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| LoadError::Io(format!("{}: {e}", path.display())))?;
    load_from_str(&text)
}

/// Load and map a theme file into a Grok [`Theme`] palette.
pub fn load_theme_palette(path: &Path) -> Result<(String, Theme), LoadError> {
    let doc = load_from_path(path)?;
    let name = doc.name.clone();
    let theme = map_pi_theme(&doc).map_err(LoadError::Map)?;
    Ok((name, theme))
}

/// Load and map theme JSON text.
pub fn load_theme_palette_from_str(json: &str) -> Result<(String, Theme), LoadError> {
    let doc = load_from_str(json)?;
    let name = doc.name.clone();
    let theme = map_pi_theme(&doc).map_err(LoadError::Map)?;
    Ok((name, theme))
}

fn validate_name(name: &str) -> Result<(), LoadError> {
    if name.is_empty() || name.contains('/') {
        return Err(LoadError::InvalidName(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_dark_and_light_builtins() {
        let dark = load_from_str(include_str!("../../../assets/pi-themes/dark.json")).unwrap();
        assert_eq!(dark.name, "dark");
        let light = load_from_str(include_str!("../../../assets/pi-themes/light.json")).unwrap();
        assert_eq!(light.name, "light");
    }

    #[test]
    fn rejects_slash_in_name() {
        let json = r#"{"name":"a/b","colors":{}}"#;
        // Fails either on missing colors or name — name is checked after parse.
        // Minimal valid-ish parse failure is fine; dedicated name check:
        assert!(matches!(
            validate_name("a/b"),
            Err(LoadError::InvalidName(_))
        ));
        let _ = json;
    }

    #[test]
    fn maps_dark_palette() {
        let (name, theme) =
            load_theme_palette_from_str(include_str!("../../../assets/pi-themes/dark.json"))
                .unwrap();
        assert_eq!(name, "dark");
        assert!(theme.is_dark());
    }
}
