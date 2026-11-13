//! Color parsing and variable resolution for Pi themes.

use std::collections::{HashMap, HashSet};

use ratatui::style::Color;

use super::schema::ColorValue;

/// Errors while resolving Pi color values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorError {
    InvalidHex(String),
    InvalidIndex(u64),
    UnknownVar(String),
    CircularVar(String),
}

impl std::fmt::Display for ColorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHex(s) => write!(f, "invalid hex color: {s}"),
            Self::InvalidIndex(i) => write!(f, "color index out of range 0-255: {i}"),
            Self::UnknownVar(v) => write!(f, "unknown theme variable: {v}"),
            Self::CircularVar(v) => write!(f, "circular theme variable reference: {v}"),
        }
    }
}

impl std::error::Error for ColorError {}

/// Resolve a color value through `vars`, then convert to ratatui [`Color`].
///
/// Empty string → [`Color::Reset`] (terminal default).
pub fn resolve_color(
    value: &ColorValue,
    vars: &HashMap<String, ColorValue>,
) -> Result<Color, ColorError> {
    let leaf = resolve_var_refs(value, vars, &mut HashSet::new())?;
    color_value_to_color(&leaf)
}

fn resolve_var_refs(
    value: &ColorValue,
    vars: &HashMap<String, ColorValue>,
    visited: &mut HashSet<String>,
) -> Result<ColorValue, ColorError> {
    match value {
        ColorValue::Index(_) => Ok(value.clone()),
        ColorValue::Text(s) if s.is_empty() || s.starts_with('#') => Ok(value.clone()),
        ColorValue::Text(name) => {
            if !visited.insert(name.clone()) {
                return Err(ColorError::CircularVar(name.clone()));
            }
            let Some(next) = vars.get(name) else {
                return Err(ColorError::UnknownVar(name.clone()));
            };
            resolve_var_refs(next, vars, visited)
        }
    }
}

fn color_value_to_color(value: &ColorValue) -> Result<Color, ColorError> {
    match value {
        ColorValue::Text(s) if s.is_empty() => Ok(Color::Reset),
        ColorValue::Text(s) if s.starts_with('#') => hex_to_color(s),
        ColorValue::Text(s) => Err(ColorError::InvalidHex(s.clone())),
        ColorValue::Index(i) => {
            if *i > 255 {
                return Err(ColorError::InvalidIndex(*i));
            }
            Ok(Color::Indexed(*i as u8))
        }
    }
}

/// Parse `#rrggbb` (case-insensitive) into [`Color::Rgb`].
pub fn hex_to_color(hex: &str) -> Result<Color, ColorError> {
    let cleaned = hex.trim().trim_start_matches('#');
    if cleaned.len() != 6 {
        return Err(ColorError::InvalidHex(hex.to_string()));
    }
    let r = u8::from_str_radix(&cleaned[0..2], 16)
        .map_err(|_| ColorError::InvalidHex(hex.to_string()))?;
    let g = u8::from_str_radix(&cleaned[2..4], 16)
        .map_err(|_| ColorError::InvalidHex(hex.to_string()))?;
    let b = u8::from_str_radix(&cleaned[4..6], 16)
        .map_err(|_| ColorError::InvalidHex(hex.to_string()))?;
    Ok(Color::Rgb(r, g, b))
}

/// Convert color to RGB triple when possible.
pub fn color_to_rgb(c: Color) -> Option<(u8, u8, u8)> {
    match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(n) => Some(crate::render::color::indexed_to_rgb(n)),
        Color::Reset => None,
        // Named ANSI — approximate with standard VGA-ish values.
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((205, 0, 0)),
        Color::Green => Some((0, 205, 0)),
        Color::Yellow => Some((205, 205, 0)),
        Color::Blue => Some((0, 0, 238)),
        Color::Magenta => Some((205, 0, 205)),
        Color::Cyan => Some((0, 205, 205)),
        Color::Gray => Some((229, 229, 229)),
        Color::DarkGray => Some((127, 127, 127)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((92, 92, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
    }
}

/// Relative luminance (ITU-R BT.709, 0.0–1.0).
pub fn relative_luminance(r: u8, g: u8, b: u8) -> f32 {
    0.2126 * (r as f32 / 255.0) + 0.7152 * (g as f32 / 255.0) + 0.0722 * (b as f32 / 255.0)
}

/// Blend `a` toward `b` by `t` (0 = a, 1 = b). Falls back to `a` if either is Reset.
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (color_to_rgb(a), color_to_rgb(b)) {
        (Some((ar, ag, ab)), Some((br, bg, bb))) => {
            let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            Color::Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
        }
        (Some(_), None) => a,
        (None, Some(_)) => b,
        (None, None) => Color::Reset,
    }
}

/// Shift luminance of a color by `delta` (-255..255) per channel.
pub fn shift_luminance(c: Color, delta: i16) -> Color {
    let Some((r, g, b)) = color_to_rgb(c) else {
        return c;
    };
    let nudge = |v: u8| (v as i16 + delta).clamp(0, 255) as u8;
    Color::Rgb(nudge(r), nudge(g), nudge(b))
}

/// Prefer concrete RGB; if `preferred` is Reset, use `fallback`.
pub fn or_fallback(preferred: Color, fallback: Color) -> Color {
    match preferred {
        Color::Reset => fallback,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses() {
        assert_eq!(hex_to_color("#00d7ff").unwrap(), Color::Rgb(0, 215, 255));
        assert_eq!(
            hex_to_color("#AbCdEf").unwrap(),
            Color::Rgb(0xab, 0xcd, 0xef)
        );
    }

    #[test]
    fn empty_is_reset() {
        let c = resolve_color(&ColorValue::Text(String::new()), &HashMap::new()).unwrap();
        assert_eq!(c, Color::Reset);
    }

    #[test]
    fn index_and_var_chain() {
        let mut vars = HashMap::new();
        vars.insert("primary".into(), ColorValue::Text("#ff0000".into()));
        vars.insert("alias".into(), ColorValue::Text("primary".into()));
        let c = resolve_color(&ColorValue::Text("alias".into()), &vars).unwrap();
        assert_eq!(c, Color::Rgb(255, 0, 0));
        let i = resolve_color(&ColorValue::Index(42), &vars).unwrap();
        assert_eq!(i, Color::Indexed(42));
    }

    #[test]
    fn circular_var_errors() {
        let mut vars = HashMap::new();
        vars.insert("a".into(), ColorValue::Text("b".into()));
        vars.insert("b".into(), ColorValue::Text("a".into()));
        let err = resolve_color(&ColorValue::Text("a".into()), &vars).unwrap_err();
        assert!(matches!(err, ColorError::CircularVar(_)));
    }
}
