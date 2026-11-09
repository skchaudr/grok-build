//! `pi/ui/*` notifications from grok-pi's adapter (`grok-pi-acp`), projected onto native pager surfaces.
//! Ported from Dwsy's grok-pi pager (719ba57f `acp_handler/mod.rs`). Remote TUI frames and the extension
//! shortcut registry have no pager surface here yet: they are acknowledged and dropped.

use agent_client_protocol as acp;

use super::routing::mcp_target_agent;
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
            let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref()) else {
                return false;
            };
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
            let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref()) else {
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
            // Remote TUI layout/session and the shortcut catalog ride on widgets; no pager surface for them yet.
            if key.starts_with("__pi_") || key == "remote_tui" {
                return true;
            }
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
            let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref()) else {
                return false;
            };
            match lines {
                Some(lines) => agent.pi_widgets.insert(key, lines),
                None => agent.pi_widgets.remove(&key),
            };
            true
        }
        "pi/ui/title" => {
            let Some(title) = str_of(&["title"]) else {
                return false;
            };
            crate::app::set_terminal_title(&title);
            // An extension's `ctx.ui.setTitle()` is temporary terminal UI; only Pi's session name renames the session.
            if str_of(&["source"]).as_deref() != Some("extension")
                && let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref())
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
            }
            true
        }
        "pi/ui/editor_text" => {
            let Some(text) = str_of(&["text"]) else {
                return false;
            };
            let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref()) else {
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
            let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref()) else {
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
            let Some((_, agent)) = mcp_target_agent(app, session_id.as_deref()) else {
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
        // Acknowledged; their pager surfaces (Remote TUI overlay, shortcut manager, Pi session picker,
        // Pi plan file) are not ported.
        "pi/ui/remote_tui" | "pi/ui/shortcuts" | "pi/ui/session_catalog" | "pi/ui/plan_file" => true,
        _ => false,
    }
}

fn dismiss(agent: &mut AgentView, tool_call_id: &str) -> bool {
    let mut changed = agent.dismiss_resolved_interaction(tool_call_id);
    for child in agent.subagent_views.values_mut() {
        changed |= child.dismiss_resolved_interaction(tool_call_id);
    }
    changed
}
