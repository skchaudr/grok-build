//! Claude's live session options as settings rows.
//!
//! Claude's adapter advertises its settings as ACP session config options
//! (`mode`, `model`, `effort`, `fast`, `agent`), so these rows read the
//! session's option list and write through `session/set_config_option`. They
//! never touch `config.toml`, which is what the Pi manifest rows do.
//!
//! The rows are appended to the registry only when the settings panel opens
//! ([`registry_with_claude_rows`]); `SettingsRegistry::defaults()` stays free of
//! them, so Pi and stock Grok never register or see them.

use agent_client_protocol as acp;
use serde_json::Value;

use super::{
    DynamicEnumSource, OwnedEnumChoice, SettingCategory, SettingKey, SettingKind, SettingMeta,
    SettingOwner, SettingValue, SettingsRegistry,
};

pub const MODE: SettingKey = "claude.mode";
pub const MODEL: SettingKey = "claude.model";
pub const EFFORT: SettingKey = "claude.effort";
pub const FAST: SettingKey = "claude.fast";
pub const AGENT: SettingKey = "claude.agent";

/// Sidebar section the rows sit in (Agent tab).
pub const SECTION: &str = "Claude";

/// Wire value for a Fast-mode select option (the adapter's `FAST_MODE_ON` / `FAST_MODE_OFF`).
const FAST_ON: &str = "on";
const FAST_OFF: &str = "off";

/// Whether `key` is one of the Claude rows.
pub fn is_claude_key(key: &str) -> bool {
    key.starts_with("claude.")
}

/// The settings row for an adapter config option id.
fn key_for_config_id(id: &str) -> Option<SettingKey> {
    Some(match id {
        "mode" => MODE,
        "model" => MODEL,
        "effort" => EFFORT,
        "fast" => FAST,
        "agent" => AGENT,
        _ => return None,
    })
}

/// One advertised option, reduced to what a row needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeOption {
    pub config_id: String,
    /// Current value id. Fast mode is always `"on"` / `"off"`, whichever way the adapter types it.
    pub current: String,
    /// The adapter typed the option as a boolean, so the wire carries a bool rather than a value id.
    pub boolean: bool,
    pub choices: Vec<OwnedEnumChoice>,
}

impl ClaudeOption {
    /// Display name of the current value, falling back to the raw id.
    pub fn current_label(&self) -> String {
        self.choices
            .iter()
            .find(|c| c.canonical == self.current)
            .map_or_else(|| self.current.clone(), |c| c.display.clone())
    }
}

/// The Claude rows a session currently offers, keyed by setting key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClaudeSettings {
    options: Vec<(SettingKey, ClaudeOption)>,
}

impl ClaudeSettings {
    /// Reduce the session's advertised config options. Unknown ids are skipped.
    pub fn from_options(options: &[acp::SessionConfigOption]) -> Self {
        let options = options
            .iter()
            .filter_map(|option| {
                let json = serde_json::to_value(option).ok()?;
                let key = key_for_config_id(json.get("id")?.as_str()?)?;
                Some((key, parse_option(&json)?))
            })
            .collect();
        Self { options }
    }

    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }

    pub fn option(&self, key: SettingKey) -> Option<&ClaudeOption> {
        self.options.iter().find(|(k, _)| *k == key).map(|(_, o)| o)
    }

    /// Row value for `key`. `None` for a row the session does not offer, which
    /// the panel renders as an unmapped row; visibility keeps that from showing.
    pub fn value_of(&self, key: SettingKey) -> Option<SettingValue> {
        let option = self.option(key)?;
        Some(if key == FAST {
            SettingValue::Bool(option.current == FAST_ON)
        } else {
            SettingValue::String(option.current.clone())
        })
    }

    /// Chooser entries for `key` (empty for Fast, which is a toggle).
    pub fn choices_for(&self, key: SettingKey) -> Vec<OwnedEnumChoice> {
        self.option(key)
            .map(|o| o.choices.clone())
            .unwrap_or_default()
    }

    /// Whether the session offers the row. Mode, Model and Effort show whenever
    /// advertised; Fast and Agent exist only when the adapter lists them.
    pub fn offers(&self, key: SettingKey) -> bool {
        self.option(key).is_some()
    }
}

fn parse_option(json: &Value) -> Option<ClaudeOption> {
    let config_id = json.get("id")?.as_str()?.to_owned();
    let boolean = json.get("type").and_then(Value::as_str) == Some("boolean");
    let current = match json.get("currentValue")? {
        Value::String(s) => s.clone(),
        Value::Bool(true) => FAST_ON.to_owned(),
        Value::Bool(false) => FAST_OFF.to_owned(),
        _ => return None,
    };
    let mut choices = Vec::new();
    if let Some(entries) = json.get("options").and_then(Value::as_array) {
        for entry in entries {
            // Grouped selects nest their entries one level down.
            match entry.get("options").and_then(Value::as_array) {
                Some(group) => group.iter().for_each(|e| push_choice(&mut choices, e)),
                None => push_choice(&mut choices, entry),
            }
        }
    }
    Some(ClaudeOption {
        config_id,
        current,
        boolean,
        choices,
    })
}

fn push_choice(out: &mut Vec<OwnedEnumChoice>, entry: &Value) {
    let Some(value) = entry.get("value").and_then(Value::as_str) else {
        return;
    };
    let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or_default();
    out.push(OwnedEnumChoice {
        canonical: value.to_owned(),
        display: match text("name") {
            "" => value.to_owned(),
            name => name.to_owned(),
        },
        description: text("description").to_owned(),
    });
}

/// `session/set_config_option` for a Claude row. `value` is the chosen value id,
/// or `"on"` / `"off"` for Fast mode.
pub fn set_option_request(
    session_id: acp::SessionId,
    option: &ClaudeOption,
    value: &str,
) -> acp::SetSessionConfigOptionRequest {
    if option.boolean {
        acp::SetSessionConfigOptionRequest::new(
            session_id,
            option.config_id.clone(),
            acp::SessionConfigOptionValue::boolean(value == FAST_ON),
        )
    } else {
        acp::SetSessionConfigOptionRequest::new(
            session_id,
            option.config_id.clone(),
            acp::SessionConfigValueId::new(value.to_owned()),
        )
    }
}

/// Row label, also used in the result toast.
pub fn label_of(key: SettingKey) -> &'static str {
    match key {
        MODE => "Claude permission mode",
        MODEL => "Model",
        EFFORT => "Thinking effort",
        FAST => "Fast mode",
        _ => "Agent",
    }
}

/// The Fast row's wire value for a toggle.
pub fn fast_value(enabled: bool) -> &'static str {
    if enabled { FAST_ON } else { FAST_OFF }
}

fn row(
    key: SettingKey,
    description: &'static str,
    keywords: &'static [&'static str],
    kind: SettingKind,
) -> SettingMeta {
    SettingMeta {
        key,
        category: SettingCategory::Agent,
        owner: SettingOwner::Pager,
        label: label_of(key),
        description,
        keywords,
        kind,
        restart_required: false,
        hidden_in_minimal: false,
        external_only: false,
    }
}

fn choice_row(
    key: SettingKey,
    description: &'static str,
    keywords: &'static [&'static str],
) -> SettingMeta {
    row(
        key,
        description,
        keywords,
        SettingKind::DynamicEnum {
            default: "",
            source: DynamicEnumSource::ClaudeSession,
            supports_preview: false,
        },
    )
}

/// The Claude rows, in panel order.
pub fn claude_rows() -> Vec<SettingMeta> {
    vec![
        choice_row(
            MODE,
            "How Claude asks before editing files or running commands.",
            &[
                "permission",
                "mode",
                "plan",
                "accept",
                "edits",
                "bypass",
                "claude",
            ],
        ),
        choice_row(
            MODEL,
            "The Claude model this session runs on.",
            &["model", "opus", "sonnet", "haiku", "claude"],
        ),
        choice_row(
            EFFORT,
            "How hard Claude thinks before answering.",
            &["effort", "thinking", "reasoning", "think", "claude"],
        ),
        row(
            FAST,
            "Faster output on models that support it.",
            &["fast", "speed", "claude"],
            SettingKind::Bool { default: false },
        ),
        choice_row(
            AGENT,
            "The custom agent persona that runs the main thread.",
            &["agent", "persona", "subagent", "claude"],
        ),
    ]
}

/// `base` plus the Claude rows. Called when the panel opens; the base registry is left untouched.
pub fn registry_with_claude_rows(base: &SettingsRegistry) -> SettingsRegistry {
    let mut entries = base.all().to_vec();
    entries.extend(
        claude_rows()
            .into_iter()
            .filter(|r| base.find(r.key).is_none()),
    );
    SettingsRegistry::from_entries(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select(
        id: &'static str,
        category: Option<acp::SessionConfigOptionCategory>,
        current: &'static str,
        values: &[(&'static str, &'static str)],
    ) -> acp::SessionConfigOption {
        let option = acp::SessionConfigOption::select(
            id,
            id,
            current,
            values
                .iter()
                .map(|(v, n)| acp::SessionConfigSelectOption::new(*v, *n))
                .collect::<Vec<_>>(),
        );
        match category {
            Some(category) => option.category(category),
            None => option,
        }
    }

    fn all_options() -> Vec<acp::SessionConfigOption> {
        vec![
            select(
                "mode",
                Some(acp::SessionConfigOptionCategory::Mode),
                "acceptEdits",
                &[
                    ("default", "Default"),
                    ("acceptEdits", "Accept Edits"),
                    ("plan", "Plan Mode"),
                ],
            ),
            select(
                "model",
                Some(acp::SessionConfigOptionCategory::Model),
                "opus",
                &[("opus", "Opus"), ("sonnet", "Sonnet")],
            ),
            select(
                "effort",
                Some(acp::SessionConfigOptionCategory::ThoughtLevel),
                "high",
                &[("low", "Low"), ("high", "High")],
            ),
            select("fast", None, "off", &[("on", "On"), ("off", "Off")]),
            select(
                "agent",
                None,
                "default",
                &[("default", "Default"), ("reviewer", "reviewer")],
            ),
        ]
    }

    #[test]
    fn every_advertised_option_becomes_a_row_with_its_current_value() {
        let settings = ClaudeSettings::from_options(&all_options());
        assert_eq!(
            settings.value_of(MODE),
            Some(SettingValue::String("acceptEdits".into()))
        );
        assert_eq!(
            settings.value_of(MODEL),
            Some(SettingValue::String("opus".into()))
        );
        assert_eq!(
            settings.value_of(EFFORT),
            Some(SettingValue::String("high".into()))
        );
        assert_eq!(settings.value_of(FAST), Some(SettingValue::Bool(false)));
        assert_eq!(
            settings.value_of(AGENT),
            Some(SettingValue::String("default".into()))
        );
        assert_eq!(
            settings.option(MODE).unwrap().current_label(),
            "Accept Edits"
        );
        assert_eq!(settings.choices_for(MODE).len(), 3);
    }

    #[test]
    fn rows_the_session_does_not_offer_are_absent() {
        let only_mode = vec![all_options().remove(0)];
        let settings = ClaudeSettings::from_options(&only_mode);
        assert!(settings.offers(MODE));
        for key in [MODEL, EFFORT, FAST, AGENT] {
            assert!(!settings.offers(key), "{key} must be absent");
            assert_eq!(settings.value_of(key), None);
        }
        assert!(ClaudeSettings::from_options(&[]).is_empty());
    }

    #[test]
    fn fast_select_and_boolean_both_read_as_a_toggle() {
        let on_select = ClaudeSettings::from_options(&[select(
            "fast",
            None,
            "on",
            &[("on", "On"), ("off", "Off")],
        )]);
        assert_eq!(on_select.value_of(FAST), Some(SettingValue::Bool(true)));
        assert!(!on_select.option(FAST).unwrap().boolean);

        let boolean: acp::SessionConfigOption = serde_json::from_value(serde_json::json!({
            "id": "fast", "name": "Fast mode", "type": "boolean", "currentValue": true
        }))
        .expect("boolean option parses");
        let settings = ClaudeSettings::from_options(&[boolean]);
        assert_eq!(settings.value_of(FAST), Some(SettingValue::Bool(true)));
        assert!(settings.option(FAST).unwrap().boolean);
    }

    #[test]
    fn selecting_a_value_sends_set_config_option_with_the_adapters_id() {
        let settings = ClaudeSettings::from_options(&all_options());
        for (key, id, value) in [
            (MODE, "mode", "plan"),
            (EFFORT, "effort", "low"),
            (MODEL, "model", "sonnet"),
        ] {
            let req = set_option_request(
                acp::SessionId::new("s1"),
                settings.option(key).unwrap(),
                value,
            );
            let json = serde_json::to_value(&req).unwrap();
            assert_eq!(json["configId"], id);
            assert_eq!(json["value"], value);
            assert_eq!(json["sessionId"], "s1");
        }
    }

    #[test]
    fn fast_toggle_sends_on_off_for_a_select_and_a_bool_for_a_boolean() {
        let select_fast = ClaudeSettings::from_options(&[select(
            "fast",
            None,
            "off",
            &[("on", "On"), ("off", "Off")],
        )]);
        let req = set_option_request(
            acp::SessionId::new("s1"),
            select_fast.option(FAST).unwrap(),
            fast_value(true),
        );
        assert_eq!(serde_json::to_value(&req).unwrap()["value"], "on");

        let boolean: acp::SessionConfigOption = serde_json::from_value(serde_json::json!({
            "id": "fast", "name": "Fast mode", "type": "boolean", "currentValue": false
        }))
        .unwrap();
        let bool_fast = ClaudeSettings::from_options(&[boolean]);
        let req = set_option_request(
            acp::SessionId::new("s1"),
            bool_fast.option(FAST).unwrap(),
            fast_value(true),
        );
        assert_eq!(serde_json::to_value(&req).unwrap()["value"], true);
    }

    #[test]
    fn claude_rows_join_a_registry_once_and_only_when_asked() {
        let base = SettingsRegistry::defaults();
        assert!(
            base.find(MODE).is_none(),
            "the default registry must not carry Claude rows"
        );
        let with = registry_with_claude_rows(&base);
        for key in [MODE, MODEL, EFFORT, FAST, AGENT] {
            assert!(with.find(key).is_some(), "{key} missing");
        }
        // A second pass is a no-op instead of a duplicate-key panic.
        assert_eq!(
            registry_with_claude_rows(&with).all().len(),
            with.all().len()
        );
    }
}
