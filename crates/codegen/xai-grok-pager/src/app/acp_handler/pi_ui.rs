//! `pi/ui/*` notifications from grok-pi's adapter (`grok-pi-acp`), projected onto native pager surfaces.
//! Ported from Dwsy's grok-pi pager (719ba57f `acp_handler/mod.rs`).

use agent_client_protocol as acp;

use super::routing::mcp_target_agent;
use crate::app::pi_remote_tui::{self, Layout};
use crate::app::app_view::AppView;
use crate::app::agent_view::AgentView;
use crate::scrollback::block::RenderBlock;

/// Handle one `pi/*` notification; `false` for methods this module does not own.
pub(super) fn handle(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    let method = notif.method.as_ref();
    if !method.starts_with("pi/") {
        return false;
    }
    let Ok(params) = serde_json::from_str::<serde_json::Value>(notif.params.get()) else {
        tracing::warn!(method, "invalid Pi UI notification payload");
        return false;
    };
    let str_of = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| params.get(*key).and_then(serde_json::Value::as_str))
            .map(str::to_owned)
    };
    let session_id = str_of(&["sessionId", "session_id"]);
    match method {
        "pi/ui/notify" => {
            let Some(message) = str_of(&["message"]) else {
                return false;
            };
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            // A forwarded Pi command that only notifies did answer (see `push_silent_forwarded_slash_notice`).
            agent.session.turn_saw_agent_text = true;
            match str_of(&["notifyType", "kind"]).as_deref() {
                Some("info") => {
                    agent.scrollback.push_block(RenderBlock::system(message));
                }
                Some("warning") => agent.show_toast(&format!("Warning: {message}")),
                Some("error") => agent.show_toast(&format!("Error: {message}")),
                _ => agent.show_toast(&message),
            }
            true
        }
        "pi/ui/status" => {
            let Some(key) = str_of(&["statusKey", "key"]) else {
                return false;
            };
            let text = str_of(&["statusText", "text"]).filter(|text| !text.trim().is_empty());
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            match text {
                Some(text) => agent.pi_statuses.insert(key, text),
                None => agent.pi_statuses.remove(&key),
            };
            true
        }
        "pi/ui/widget" => {
            let Some(key) = str_of(&["widgetKey", "key"]) else {
                return false;
            };
            let lines = params
                .get("widgetLines")
                .or_else(|| params.get("lines"))
                .and_then(serde_json::Value::as_array)
                .map(|lines| {
                    lines
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .filter(|lines| lines.iter().any(|line| !line.trim().is_empty()));
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            let first = lines.as_ref().and_then(|lines| lines.first()).map(String::as_str);
            match key.as_str() {
                pi_remote_tui::SESSION_WIDGET_KEY => agent.pi_remote_tui.set_session(first.unwrap_or_default()),
                pi_remote_tui::LAYOUT_WIDGET_KEY => {
                    agent.pi_remote_tui.layout = first
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                        .and_then(|value| Layout::from_json(&value));
                }
                pi_remote_tui::WIDGET_KEY => {
                    // A command that opened a Remote TUI screen (`/login`) answered without text.
                    agent.session.turn_saw_agent_text |= lines.is_some();
                    agent.pi_remote_tui.set_frame(lines);
                }
                // pi-grok-shortcut-manager publishes the catalog as a widget; it feeds dispatch, never a row.
                "__pi_extension_shortcuts__" => {
                    if let Some(value) = first.and_then(|raw| serde_json::from_str(raw).ok()) {
                        set_shortcuts(agent, &value);
                    }
                }
                _ => match lines {
                    Some(lines) => {
                        agent.pi_widgets.insert(key, lines);
                    }
                    None => {
                        agent.pi_widgets.remove(&key);
                    }
                },
            }
            true
        }
        "pi/ui/remote_tui" => {
            let lines = params.get("lines").and_then(serde_json::Value::as_array).map(|lines| {
                lines
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            });
            let (op, id, title) = (str_of(&["op"]).unwrap_or_default(), str_of(&["id"]), str_of(&["title"]));
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            agent.session.turn_saw_agent_text = true;
            agent.pi_remote_tui.apply(&op, id, lines, title)
        }
        "pi/ui/plan_file" => {
            let Some(path) = str_of(&["planFilePath"]) else {
                return false;
            };
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            agent.pi_plan_file_path = Some(path.into());
            true
        }
        "pi/ui/title" => {
            let Some(title) = str_of(&["title"]) else {
                return false;
            };
            crate::app::set_terminal_title(&title);
            // An extension's `ctx.ui.setTitle()` is temporary terminal UI; only Pi's session name renames the session.
            if str_of(&["source"]).as_deref() != Some("extension")
                && let Some(agent) = pi_target(app, session_id.as_deref())
            {
                let clean = crate::views::session_title::sanitize_display_text(title.trim());
                agent.display_name = (!clean.trim().is_empty()).then(|| clean.into_owned());
            }
            true
        }
        "pi/ui/reset_extension_ui" => {
            for agent in app.agents.values_mut() {
                agent.pi_statuses.clear();
                agent.pi_widgets.clear();
                agent.pi_remote_tui.clear();
            }
            true
        }
        "pi/ui/editor_text" => {
            let Some(text) = str_of(&["text"]) else {
                return false;
            };
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            agent.prompt.set_text(&text);
            true
        }
        "pi/ui/cancel_interaction" => {
            let Some(tool_call_id) = str_of(&["toolCallId", "tool_call_id"]) else {
                return false;
            };
            let mut changed = false;
            for agent in app.agents.values_mut() {
                changed |= dismiss(agent, &tool_call_id);
            }
            changed
        }
        "pi/ui/compaction_summary" => {
            let Some(summary) = str_of(&["summary"]).filter(|s| !s.trim().is_empty()) else {
                return false;
            };
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            agent
                .scrollback
                .push_block(RenderBlock::system(format!("Compaction summary\n\n{summary}")));
            true
        }
        "pi/ui/btw_history" => {
            let entries: Vec<(String, String)> = params
                .get("entries")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| {
                    let question = entry.get("question")?.as_str()?.trim();
                    let answer = entry.get("answer")?.as_str()?.trim();
                    (!question.is_empty() && !answer.is_empty())
                        .then(|| (question.to_owned(), answer.to_owned()))
                })
                .collect();
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            for (question, answer) in entries {
                agent
                    .scrollback
                    .push_block(RenderBlock::Btw(crate::scrollback::blocks::BtwBlock::new(
                        question, answer,
                    )));
            }
            true
        }
        "pi/ui/shortcuts" => {
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            set_shortcuts(agent, &params)
        }
        "pi/ui/session_catalog" => {
            let Some(sessions) = params.get("sessions").and_then(serde_json::Value::as_array) else {
                return false;
            };
            let rows = sessions.iter().filter_map(catalog_row).collect();
            let Some(agent) = pi_target(app, session_id.as_deref()) else {
                return false;
            };
            // The rows answer this session's `/resume` (`Effect::PiSessionList`); a closed picker drops them.
            if let Some(crate::views::modal::ActiveModal::SessionPicker { entries, loading, .. }) =
                agent.active_modal.as_mut()
            {
                *entries = Some(rows);
                *loading = false;
            }
            true
        }
        _ => false,
    }
}

/// One Pi session from `pi/ui/session_catalog` as a `/resume` row (source `pi`).
fn catalog_row(session: &serde_json::Value) -> Option<crate::app::app_view::SessionPickerEntry> {
    let text = |key: &str| session.get(key).and_then(serde_json::Value::as_str).map(str::to_owned);
    // Pi sends epoch millis or RFC 3339.
    let time = |key: &str| {
        let value = session.get(key)?;
        value.as_i64().and_then(chrono::DateTime::from_timestamp_millis).or_else(|| {
            chrono::DateTime::parse_from_rfc3339(value.as_str()?)
                .ok()
                .map(|time| time.with_timezone(&chrono::Utc))
        })
    };
    let cwd = text("cwd").unwrap_or_default();
    let updated_at = time("updatedAt").unwrap_or_else(chrono::Utc::now);
    Some(crate::app::app_view::SessionPickerEntry {
        id: text("id")?,
        summary: text("name")
            .or_else(|| text("summary"))
            .or_else(|| text("firstMessage"))
            .unwrap_or_else(|| "(no messages)".to_owned()),
        updated_at,
        created_at: time("createdAt").unwrap_or(updated_at),
        repo_name: crate::views::session_picker::repo_name_from_cwd(&cwd),
        cwd,
        hostname: None,
        source: "pi".to_owned(),
        model_id: text("modelId"),
        num_messages: session.get("messageCount").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize,
        last_active_at: None,
        branch: None,
        worktree_label: None,
        last_turn_summary: text("lastTurnSummary"),
        last_recap: text("lastRecap"),
        session_kind: text("sessionKind"),
        card_detail: None,
    })
}

/// `{shortcuts: [{key, description, extension, enabled, remappedTo}]}` replaces the session's registry.
fn set_shortcuts(agent: &mut AgentView, value: &serde_json::Value) -> bool {
    use crate::app::extension_shortcuts::{ExtensionShortcut, ExtensionShortcutRegistry};
    let Some(list) = value.get("shortcuts").and_then(serde_json::Value::as_array) else {
        return false;
    };
    let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(serde_json::Value::as_str).map(str::to_owned);
    let shortcuts = list
        .iter()
        .filter_map(|v| {
            Some(ExtensionShortcut {
                key: text(v, "key")?,
                description: text(v, "description").unwrap_or_default(),
                extension: text(v, "extension").unwrap_or_else(|| "unknown".to_owned()),
                enabled: v.get("enabled").and_then(serde_json::Value::as_bool).unwrap_or(true),
                remapped_to: text(v, "remappedTo"),
            })
        })
        .collect();
    agent
        .pi_shortcuts
        .get_or_insert_with(ExtensionShortcutRegistry::new)
        .set_shortcuts(shortcuts);
    true
}

/// The session a `pi/*` notification is for. Pi's extension UI (widgets, statuses, Remote TUI frames) carries no
/// `sessionId`: it belongs to the Pi session on screen, else the newest Pi session, so a Claude session in the
/// same pager never receives it.
fn pi_target<'a>(app: &'a mut AppView, session_id: Option<&str>) -> Option<&'a mut AgentView> {
    if session_id.is_none() {
        let on_screen = app.active_view.agent_id().filter(|id| app.is_pi_agent(*id));
        if let Some(id) = on_screen.or_else(|| app.agents.keys().copied().filter(|id| app.is_pi_agent(*id)).max()) {
            return app.agents.get_mut(&id);
        }
    }
    mcp_target_agent(app, session_id).map(|(_, agent)| agent)
}

fn dismiss(agent: &mut AgentView, tool_call_id: &str) -> bool {
    let mut changed = agent.dismiss_resolved_interaction(tool_call_id);
    for child in agent.subagent_views.values_mut() {
        changed |= child.dismiss_resolved_interaction(tool_call_id);
    }
    changed
}
