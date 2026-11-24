//! A durable Pi agent's live settings and info as the F2 panel's "Durable Pi" tab.
//!
//! Under dpi-grok, grok-pi-acp sits on the dpi-rpc shim, which answers `dpi_settings` with the agent's options and an
//! info block. The adapter advertises the options as ACP session config options tagged `_meta.dpi.category`, writes
//! them back through `session/set_config_option`, and forwards the info as `pi/ui/dpi_info`. Option ids are the
//! shim's and travel verbatim: nothing here knows which settings exist.
//!
//! Rows are built per panel open from whatever the session advertises, so their keys, labels and sidebar sections
//! are runtime strings. The registry wants `&'static str`, so they are interned once ([`intern`]); the set is the
//! handful of options one backend offers, so the pool stays small.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use agent_client_protocol as acp;
use serde_json::Value;

use super::session_option::{ON, SessionOption};
use super::{
    DynamicEnumSource, OwnedEnumChoice, SettingCategory, SettingKey, SettingKind, SettingMeta,
    SettingOwner, SettingValue, SettingsRegistry,
};

/// The `_meta` key grok-pi-acp stamps on every option it took from the shim.
pub const OPTION_META: &str = "dpi";
const KEY_PREFIX: &str = "dpi.";
/// Sidebar section for an option the shim gave no category.
const DEFAULT_SECTION: &str = "Agent";

/// Exported by dpi-grok: grok-pi-acp sits on the dpi-rpc shim, so every Pi session is a durable agent on a dpi or
/// Prime backend that keeps running whether or not this TUI is attached. Real Pi (`grok grok-pi`) never sets it.
pub const HOST_ENV: &str = "GROK_PI_DPI";

#[cfg(test)]
thread_local! {
    static HOST_FOR_TEST: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether this pager drives durable dpi agents (`GROK_PI_DPI=1`).
pub fn dpi_host() -> bool {
    #[cfg(test)]
    if let Some(forced) = HOST_FOR_TEST.with(std::cell::Cell::get) {
        return forced;
    }
    std::env::var(HOST_ENV).is_ok_and(|value| value == "1")
}

/// Pins [`dpi_host`] for this test thread (`None` reads the environment again).
#[cfg(test)]
pub fn set_dpi_host_for_test(forced: Option<bool>) {
    HOST_FOR_TEST.with(|cell| cell.set(forced));
}

/// Whether `key` is one of the dpi rows.
pub fn is_dpi_key(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

/// Whether the agent advertised `option` on behalf of the dpi-rpc shim.
pub fn is_dpi_option(option: &acp::SessionConfigOption) -> bool {
    option
        .meta
        .as_ref()
        .is_some_and(|meta| meta.contains_key(OPTION_META))
}

/// A runtime string as `&'static str`, leaked once per distinct value.
fn intern(text: &str) -> &'static str {
    static POOL: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Default::default);
    let mut pool = POOL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&interned) = pool.get(text) {
        return interned;
    }
    let interned: &'static str = Box::leak(text.to_owned().into_boxed_str());
    pool.insert(interned);
    interned
}

/// Backend text reaches the terminal: one line, no control characters.
fn clean(text: &str) -> String {
    let first_line = text.trim().lines().next().unwrap_or_default();
    crate::views::session_title::sanitize_display_text(first_line.trim()).into_owned()
}

#[derive(Debug, Clone, PartialEq)]
struct DpiRow {
    key: SettingKey,
    label: &'static str,
    description: &'static str,
    section: &'static str,
    option: SessionOption,
}

/// The dpi rows one session offers, in the shim's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DpiSettings {
    rows: Vec<DpiRow>,
}

impl DpiSettings {
    /// The dpi-tagged options among a session's config options; every other option is ignored.
    pub fn from_options(options: &[acp::SessionConfigOption]) -> Self {
        let rows = options
            .iter()
            .filter(|option| is_dpi_option(option))
            .filter_map(|option| {
                let json = serde_json::to_value(option).ok()?;
                let parsed = SessionOption::parse(&json)?;
                let text = |value: Option<&Value>| clean(value.and_then(Value::as_str).unwrap_or_default());
                let section = match text(json.pointer("/_meta/dpi/category")) {
                    category if category.is_empty() => DEFAULT_SECTION.to_owned(),
                    category => category,
                };
                let label = match text(json.get("name")) {
                    name if name.is_empty() => parsed.config_id.clone(),
                    name => name,
                };
                Some(DpiRow {
                    key: intern(&format!("{KEY_PREFIX}{}", parsed.config_id)),
                    label: intern(&label),
                    description: intern(&text(json.get("description"))),
                    section: intern(&section),
                    option: parsed,
                })
            })
            .collect();
        Self { rows }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn row(&self, key: &str) -> Option<&DpiRow> {
        self.rows.iter().find(|row| row.key == key)
    }

    pub fn option(&self, key: &str) -> Option<&SessionOption> {
        self.row(key).map(|row| &row.option)
    }

    pub fn offers(&self, key: &str) -> bool {
        self.row(key).is_some()
    }

    /// Row value: booleans are toggles, everything else a value id.
    pub fn value_of(&self, key: &str) -> Option<SettingValue> {
        let option = self.option(key)?;
        Some(if option.boolean {
            SettingValue::Bool(option.current == ON)
        } else {
            SettingValue::String(option.current.clone())
        })
    }

    /// What the value column shows for a choice row: the value's name, not its id. `None` for a toggle.
    pub fn display(&self, key: &str) -> Option<String> {
        let option = self.option(key)?;
        (!option.boolean).then(|| option.current_label())
    }

    pub fn choices_for(&self, key: &str) -> Vec<OwnedEnumChoice> {
        self.option(key).map(|o| o.choices.clone()).unwrap_or_default()
    }

    /// Sidebar sections in the order the shim first names them.
    pub fn sections(&self) -> Vec<&'static str> {
        let mut sections: Vec<&'static str> = Vec::new();
        for row in &self.rows {
            if !sections.contains(&row.section) {
                sections.push(row.section);
            }
        }
        sections
    }

    pub fn section_of(&self, key: &str) -> Option<&'static str> {
        self.row(key).map(|row| row.section)
    }

    /// Row label, also used in the result toast.
    pub fn label_of(&self, key: &str) -> &'static str {
        self.row(key).map_or("Durable Pi setting", |row| row.label)
    }

    /// Registry rows for the panel, in the shim's order.
    pub fn metas(&self) -> Vec<SettingMeta> {
        self.rows
            .iter()
            .map(|row| SettingMeta {
                key: row.key,
                category: SettingCategory::DurablePi,
                owner: SettingOwner::Pager,
                label: row.label,
                description: row.description,
                keywords: &[],
                kind: if row.option.boolean {
                    SettingKind::Bool { default: false }
                } else {
                    SettingKind::DynamicEnum {
                        default: "",
                        source: DynamicEnumSource::SessionOption,
                        supports_preview: false,
                    }
                },
                restart_required: false,
                hidden_in_minimal: false,
                external_only: false,
            })
            .collect()
    }
}

/// `base` plus the session's dpi rows. Called when the panel opens or refreshes; `base` is left untouched.
pub fn registry_with_rows(base: &SettingsRegistry, dpi: &DpiSettings) -> SettingsRegistry {
    let mut entries = base.all().to_vec();
    entries.extend(dpi.metas().into_iter().filter(|meta| base.find(meta.key).is_none()));
    SettingsRegistry::from_entries(entries)
}

/// The model context in use after the agent's last turn that reported usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiContext {
    pub tokens: u64,
    pub window: u64,
    pub percent: u8,
}

/// The shim's `info` block (`pi/ui/dpi_info`): who and where the durable agent is. Optional fields stay `None`
/// until the backend has them (no branch outside git, no context before the first turn, no playground).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DpiInfo {
    pub device: String,
    pub session: String,
    pub agent: String,
    pub cwd: String,
    pub branch: Option<String>,
    pub model: String,
    pub thinking: Option<String>,
    pub read_only: bool,
    pub tools: Vec<String>,
    pub context: Option<DpiContext>,
    pub backend_url: Option<String>,
    pub playground_url: Option<String>,
    pub hints: Vec<String>,
}

impl DpiInfo {
    pub fn from_json(info: &Value) -> Option<Self> {
        info.as_object()?;
        let text = |key: &str| {
            info.get(key)
                .and_then(Value::as_str)
                .map(clean)
                .filter(|s| !s.is_empty())
        };
        let list = |key: &str| -> Vec<String> {
            info.get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(clean)
                .filter(|s| !s.is_empty())
                .collect()
        };
        let context = info.get("context").and_then(|c| {
            let number = |key: &str| c.get(key).and_then(Value::as_u64);
            let (tokens, window) = (number("tokens")?, number("window").filter(|w| *w > 0)?);
            let percent = number("percent").unwrap_or(tokens.saturating_mul(100) / window);
            Some(DpiContext {
                tokens,
                window,
                percent: percent.min(100) as u8,
            })
        });
        Some(Self {
            device: text("device").unwrap_or_default(),
            session: text("session").unwrap_or_default(),
            agent: text("agent").unwrap_or_default(),
            cwd: text("cwd").unwrap_or_default(),
            branch: text("branch"),
            model: text("model").unwrap_or_default(),
            thinking: text("thinking"),
            read_only: info.get("readOnly").and_then(Value::as_bool).unwrap_or(false),
            tools: list("tools"),
            context,
            backend_url: text("backendUrl"),
            playground_url: text("playgroundUrl"),
            hints: list("hints"),
        })
    }
}

/// `alt+c` as a caption: `Alt+C`.
pub fn key_caption(key: &str) -> String {
    key.split('+')
        .map(|part| {
            let mut chars = part.chars();
            match (chars.next(), chars.as_str()) {
                (Some(c), "") => c.to_uppercase().collect(),
                (Some(c), rest) => c.to_uppercase().chain(rest.chars()).collect(),
                (None, _) => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The dpi shim's action keys from the session's shortcut catalog, as footer captions (`Alt+C compact`), interned
/// because the modal footer borrows `'static` labels. A catalog entry's `label` is preferred; its description is
/// the fallback.
pub fn action_captions(
    shortcuts: Option<&crate::app::extension_shortcuts::ExtensionShortcutRegistry>,
) -> Vec<&'static str> {
    let Some(shortcuts) = shortcuts else {
        return Vec::new();
    };
    shortcuts
        .all()
        .into_iter()
        .filter(|s| s.extension == OPTION_META && s.enabled)
        .map(|s| {
            let key = s.remapped_to.as_deref().unwrap_or(&s.key);
            let label = match s.label.as_deref().map(str::trim) {
                Some(label) if !label.is_empty() => label,
                _ => s.description.trim(),
            };
            intern(&clean(&format!("{} {label}", key_caption(key))))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Options as grok-pi-acp advertises them: the shim's ids, `_meta.dpi.category`.
    fn advertised() -> Vec<acp::SessionConfigOption> {
        serde_json::from_value(json!([
            {"id": "model", "name": "Model", "description": "The agent's model", "type": "select",
             "currentValue": "cliproxy/gpt-5", "_meta": {"dpi": {"category": "Agent"}},
             "options": [{"value": "cliproxy/gpt-5", "name": "gpt-5"}, {"value": "openrouter/x", "name": "x"}]},
            {"id": "read_only", "name": "Read-only", "description": "Blocks write tools", "type": "boolean",
             "currentValue": true, "_meta": {"dpi": {"category": "Agent"}}},
            {"id": "prompt:abc", "name": "Reviewer", "type": "boolean", "currentValue": false,
             "_meta": {"dpi": {"category": "System prompts"}}},
            {"id": "mode", "name": "Mode", "type": "select", "currentValue": "code",
             "options": [{"value": "code", "name": "Code"}]},
        ]))
        .unwrap()
    }

    #[test]
    fn only_dpi_tagged_options_become_rows_keyed_by_the_shims_id() {
        let dpi = DpiSettings::from_options(&advertised());
        let keys: Vec<_> = dpi.metas().iter().map(|m| m.key).collect();
        assert_eq!(keys, ["dpi.model", "dpi.read_only", "dpi.prompt:abc"]);
        assert!(!dpi.offers("dpi.mode"), "an untagged option is not a dpi row");
        assert_eq!(dpi.option("dpi.prompt:abc").unwrap().config_id, "prompt:abc");
        assert!(DpiSettings::from_options(&[]).is_empty());
    }

    #[test]
    fn booleans_are_toggles_and_selects_show_the_value_name() {
        let dpi = DpiSettings::from_options(&advertised());
        assert_eq!(dpi.value_of("dpi.read_only"), Some(SettingValue::Bool(true)));
        assert_eq!(dpi.display("dpi.read_only"), None);
        assert_eq!(dpi.value_of("dpi.model"), Some(SettingValue::String("cliproxy/gpt-5".into())));
        assert_eq!(dpi.display("dpi.model").as_deref(), Some("gpt-5"));
        assert_eq!(dpi.choices_for("dpi.model").len(), 2);
        let metas = dpi.metas();
        assert!(matches!(metas[1].kind, SettingKind::Bool { .. }));
        assert!(matches!(
            metas[0].kind,
            SettingKind::DynamicEnum { source: DynamicEnumSource::SessionOption, .. }
        ));
        assert!(metas.iter().all(|m| m.category == SettingCategory::DurablePi));
        assert_eq!(metas[0].description, "The agent's model");
        assert_eq!(dpi.label_of("dpi.read_only"), "Read-only");
    }

    #[test]
    fn categories_become_sections_in_first_seen_order() {
        let dpi = DpiSettings::from_options(&advertised());
        assert_eq!(dpi.sections(), ["Agent", "System prompts"]);
        assert_eq!(dpi.section_of("dpi.prompt:abc"), Some("System prompts"));
    }

    #[test]
    fn rows_join_a_registry_once() {
        let dpi = DpiSettings::from_options(&advertised());
        let base = SettingsRegistry::defaults();
        assert!(base.find("dpi.model").is_none());
        let with = registry_with_rows(&base, &dpi);
        assert!(with.find("dpi.read_only").is_some());
        assert_eq!(registry_with_rows(&with, &dpi).all().len(), with.all().len());
    }

    #[test]
    fn info_reads_every_field_and_tolerates_missing_optionals() {
        let full = DpiInfo::from_json(&json!({
            "device": "mini", "session": "modal-test", "sessionId": "s1", "agent": "main", "agentId": "a1",
            "cwd": "/Users/me/repo", "branch": "main", "model": "cliproxy/gpt-5", "thinking": "medium",
            "readOnly": true, "tools": ["read", "bash"], "context": {"tokens": 50000, "window": 200000, "percent": 25},
            "backendUrl": "http://mini:8787", "playgroundUrl": "http://mini:5173/s/modal-test",
            "hints": ["F2 opens settings", "  "],
        }))
        .unwrap();
        assert_eq!(full.device, "mini");
        assert_eq!(full.branch.as_deref(), Some("main"));
        assert!(full.read_only);
        assert_eq!(full.tools, ["read", "bash"]);
        assert_eq!(full.context, Some(DpiContext { tokens: 50000, window: 200000, percent: 25 }));
        assert_eq!(full.playground_url.as_deref(), Some("http://mini:5173/s/modal-test"));
        assert_eq!(full.hints, ["F2 opens settings"]);

        let bare = DpiInfo::from_json(&json!({"device": "mini", "playgroundUrl": null, "context": {"tokens": 5}}))
            .unwrap();
        assert_eq!(bare.branch, None);
        assert_eq!(bare.context, None, "context without a window is dropped");
        assert_eq!(bare.playground_url, None);
        assert!(DpiInfo::from_json(&json!("nope")).is_none());
    }

    #[test]
    fn key_captions_read_like_the_shims_labels() {
        assert_eq!(key_caption("alt+c"), "Alt+C");
        assert_eq!(key_caption("ctrl+shift+k"), "Ctrl+Shift+K");
    }
}
