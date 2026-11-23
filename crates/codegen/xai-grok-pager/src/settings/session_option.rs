//! One live ACP session config option, reduced to what a settings row needs.
//!
//! Claude's adapter ([`super::claude`]) and the durable Pi shim ([`super::dpi`]) both advertise their settings as
//! session config options and take changes through `session/set_config_option`. The parsing and the request live
//! here once; each module decides which options become rows and how they are labelled.

use agent_client_protocol as acp;
use serde_json::Value;

use super::OwnedEnumChoice;

/// A boolean option's current value as a row stores it: every option value is a string id here.
pub const ON: &str = "on";
pub const OFF: &str = "off";

/// The stored value for a toggled boolean row.
pub fn bool_value(enabled: bool) -> &'static str {
    if enabled { ON } else { OFF }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionOption {
    pub config_id: String,
    /// Current value id; a boolean is always [`ON`] / [`OFF`], whichever way the agent types it.
    pub current: String,
    /// The agent typed the option as a boolean, so the wire carries a bool rather than a value id.
    pub boolean: bool,
    pub choices: Vec<OwnedEnumChoice>,
}

impl SessionOption {
    /// Display name of the current value, falling back to the raw id.
    pub fn current_label(&self) -> String {
        self.choices
            .iter()
            .find(|c| c.canonical == self.current)
            .map_or_else(|| self.current.clone(), |c| c.display.clone())
    }

    /// Parse one option from its ACP wire JSON; `None` when it has no id or no usable current value.
    pub fn parse(json: &Value) -> Option<Self> {
        let config_id = json.get("id")?.as_str()?.to_owned();
        let boolean = json.get("type").and_then(Value::as_str) == Some("boolean");
        let current = match json.get("currentValue")? {
            Value::String(s) => s.clone(),
            Value::Bool(b) => bool_value(*b).to_owned(),
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
        Some(Self {
            config_id,
            current,
            boolean,
            choices,
        })
    }
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

/// `session/set_config_option` for a row. `value` is the chosen value id, or [`ON`] / [`OFF`] for a toggle.
pub fn set_option_request(
    session_id: acp::SessionId,
    option: &SessionOption,
    value: &str,
) -> acp::SetSessionConfigOptionRequest {
    if option.boolean {
        acp::SetSessionConfigOptionRequest::new(
            session_id,
            option.config_id.clone(),
            acp::SessionConfigOptionValue::boolean(value == ON),
        )
    } else {
        acp::SetSessionConfigOptionRequest::new(
            session_id,
            option.config_id.clone(),
            acp::SessionConfigValueId::new(value.to_owned()),
        )
    }
}
