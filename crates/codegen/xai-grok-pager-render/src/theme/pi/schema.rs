//! Pi theme JSON schema (mirrors pi-main ThemeJsonSchema).

use std::collections::HashMap;

use serde::Deserialize;

/// A single color value as accepted by Pi theme JSON.
///
/// Formats: `#rrggbb`, empty string (terminal default), 256-color index,
/// or a `vars` name reference (still a string until resolved).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ColorValue {
    /// Hex (`#rrggbb`), empty `""`, or var reference (`"primary"`).
    Text(String),
    /// xterm 256-color palette index (0–255).
    Index(u64),
}

/// Optional HTML export colors from Pi themes.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiExportColors {
    pub page_bg: Option<ColorValue>,
    pub card_bg: Option<ColorValue>,
    pub info_bg: Option<ColorValue>,
}

/// Full Pi theme document.
#[derive(Debug, Clone, Deserialize)]
pub struct PiThemeJson {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub name: String,
    #[serde(default)]
    pub vars: HashMap<String, ColorValue>,
    pub colors: PiThemeColors,
    #[serde(default)]
    pub export: Option<PiExportColors>,
}

/// All Pi color tokens. `thinking_max` is optional (falls back to xhigh).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiThemeColors {
    // Core UI
    pub accent: ColorValue,
    pub border: ColorValue,
    pub border_accent: ColorValue,
    pub border_muted: ColorValue,
    pub success: ColorValue,
    pub error: ColorValue,
    pub warning: ColorValue,
    pub muted: ColorValue,
    pub dim: ColorValue,
    pub text: ColorValue,
    pub thinking_text: ColorValue,
    // Backgrounds & content
    pub selected_bg: ColorValue,
    pub user_message_bg: ColorValue,
    pub user_message_text: ColorValue,
    pub custom_message_bg: ColorValue,
    pub custom_message_text: ColorValue,
    pub custom_message_label: ColorValue,
    pub tool_pending_bg: ColorValue,
    pub tool_success_bg: ColorValue,
    pub tool_error_bg: ColorValue,
    pub tool_title: ColorValue,
    pub tool_output: ColorValue,
    // Markdown
    pub md_heading: ColorValue,
    pub md_link: ColorValue,
    pub md_link_url: ColorValue,
    pub md_code: ColorValue,
    pub md_code_block: ColorValue,
    pub md_code_block_border: ColorValue,
    pub md_quote: ColorValue,
    pub md_quote_border: ColorValue,
    pub md_hr: ColorValue,
    pub md_list_bullet: ColorValue,
    // Diffs
    pub tool_diff_added: ColorValue,
    pub tool_diff_removed: ColorValue,
    pub tool_diff_context: ColorValue,
    // Syntax
    pub syntax_comment: ColorValue,
    pub syntax_keyword: ColorValue,
    pub syntax_function: ColorValue,
    pub syntax_variable: ColorValue,
    pub syntax_string: ColorValue,
    pub syntax_number: ColorValue,
    pub syntax_type: ColorValue,
    pub syntax_operator: ColorValue,
    pub syntax_punctuation: ColorValue,
    // Thinking borders
    pub thinking_off: ColorValue,
    pub thinking_minimal: ColorValue,
    pub thinking_low: ColorValue,
    pub thinking_medium: ColorValue,
    pub thinking_high: ColorValue,
    pub thinking_xhigh: ColorValue,
    #[serde(default)]
    pub thinking_max: Option<ColorValue>,
    // Bash mode
    pub bash_mode: ColorValue,
}
