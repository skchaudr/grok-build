//! Pi session operations behind `/reload`, `/clone` and Pi's `/fork` (`pi/session/*` on grok-pi-acp).
//! Ported from Dwsy's grok-pi pager (719ba57f `dispatch/session/pi_fork.rs`), one request shape for every op.

use crate::acp::tracker::AcpUpdateTracker;
use crate::app::actions::{Effect, PiForkMessage, PiSessionOp};
use crate::app::agent::AgentId;
use crate::app::app_view::AppView;
use crate::app::dispatch::ctx::get_active_agent;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::state::ScrollbackState;
use crate::views::modal::ActiveModal;
use agent_client_protocol as acp;

pub(in crate::app::dispatch) fn dispatch_pi_reload(app: &mut AppView) -> Vec<Effect> {
    request(app, "/reload", PiSessionOp::Reload, "Reloading keybindings, extensions, skills, prompts, themes, and context files…")
}

pub(in crate::app::dispatch) fn dispatch_pi_clone(app: &mut AppView) -> Vec<Effect> {
    request(app, "/clone", PiSessionOp::Clone, "Cloning session…")
}

/// `/fork <entry id>` branches there; bare `/fork` lists the session's user messages to pick from.
pub(in crate::app::dispatch) fn dispatch_pi_fork(app: &mut AppView, entry_id: Option<String>) -> Vec<Effect> {
    match entry_id.map(|id| id.trim().to_owned()).filter(|id| !id.is_empty()) {
        Some(entry_id) => request(app, "/fork", PiSessionOp::Fork { entry_id }, "Forking session…"),
        None => request(app, "/fork", PiSessionOp::ForkMessages, "Loading fork messages…"),
    }
}

fn request(app: &mut AppView, command: &str, op: PiSessionOp, toast: &str) -> Vec<Effect> {
    let Some(agent) = get_active_agent(app) else {
        app.show_toast("No active session");
        return vec![];
    };
    let agent_id = agent.session.id;
    if !app.is_pi_agent(agent_id) {
        app.show_toast(&format!("{command} is only available in Pi sessions"));
        return vec![];
    }
    let Some(session_id) = agent.session.session_id.clone() else {
        app.show_toast("Session is still being created");
        return vec![];
    };
    if let Some(agent) = app.agents.get_mut(&agent_id) {
        agent.fork_state = None;
    }
    app.show_toast(toast);
    vec![Effect::PiSessionRequest { agent_id, session_id, op }]
}

pub(in crate::app::dispatch) fn handle_pi_session_reply(
    app: &mut AppView,
    agent_id: AgentId,
    op: PiSessionOp,
    reply: Result<serde_json::Value, String>,
) -> Vec<Effect> {
    let label = match op {
        PiSessionOp::Reload => "Reload",
        PiSessionOp::Clone => "Clone",
        PiSessionOp::ForkMessages => "Loading fork messages",
        PiSessionOp::Fork { .. } => "Fork",
        PiSessionOp::Tree { .. } => "Loading the session tree",
        PiSessionOp::NavigateTree { .. } => "Tree navigation",
        PiSessionOp::TreeLabel { .. } => "Labeling",
        PiSessionOp::RollbackPreview { .. } => "Rollback preview",
        PiSessionOp::RollbackExecute { .. } => "File rollback",
    };
    let payload = match reply {
        Ok(payload) if payload.get("cancelled").and_then(serde_json::Value::as_bool) == Some(true) => {
            app.show_toast(&format!("{label} cancelled"));
            return vec![];
        }
        Ok(payload) => payload,
        Err(error) => {
            if let Some(agent) = app.agents.get_mut(&agent_id) {
                agent.fork_state = None;
                if matches!(op, PiSessionOp::Tree { .. }) {
                    agent.active_modal = None;
                }
            }
            app.show_toast(&format!("{label} failed: {error}"));
            return vec![];
        }
    };
    match op {
        PiSessionOp::Reload => {
            if payload.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
                app.show_toast("Reload failed");
            } else {
                app.show_toast("Reloaded keybindings, extensions, skills, prompts, themes, and context files");
            }
            vec![]
        }
        PiSessionOp::ForkMessages => {
            let messages = fork_messages(&payload);
            let Some(agent) = app.agents.get_mut(&agent_id) else {
                return vec![];
            };
            if messages.is_empty() {
                app.show_toast("No messages to fork from");
            } else if agent.jump_slot_taken() {
                app.show_toast("Cannot open the fork picker right now");
            } else {
                agent.dismiss_jump_picker();
                agent.fork_state = Some(crate::views::fork_picker::ForkPickerState::new(messages));
            }
            vec![]
        }
        PiSessionOp::Tree { .. } => {
            let leaf_id = payload.get("leafId").and_then(serde_json::Value::as_str).map(str::to_owned);
            let nodes = tree_nodes(&payload);
            let Some(agent) = app.agents.get_mut(&agent_id) else {
                return vec![];
            };
            if nodes.is_empty() {
                agent.active_modal = None;
                app.show_toast("Session tree is empty");
                return vec![];
            }
            // A modal closed while the tree was in flight drops it.
            match agent.active_modal.as_mut() {
                Some(ActiveModal::SessionTree { state, .. }) => {
                    state.replace_nodes(nodes, leaf_id);
                    state.status = None;
                }
                Some(ActiveModal::TreeMap { state, .. }) => state.replace_nodes(nodes, leaf_id),
                _ => {}
            }
            vec![]
        }
        PiSessionOp::NavigateTree { .. } => {
            let Some(agent) = app.agents.get(&agent_id) else {
                return vec![];
            };
            let Some(session_id) = agent.session.session_id.as_ref().map(|id| id.0.to_string()) else {
                return vec![];
            };
            // The text of the message navigated away from goes back in an empty prompt, as in Pi.
            let editor_text = payload
                .get("editorText")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty() && agent.prompt.text().trim().is_empty())
                .map(str::to_owned);
            replace_session(app, agent_id, session_id, editor_text, "Navigated to selected point")
        }
        PiSessionOp::TreeLabel { entry_id, label } => {
            let Some(agent) = app.agents.get_mut(&agent_id) else {
                return vec![];
            };
            if let Some(ActiveModal::SessionTree { state, .. }) = agent.active_modal.as_mut() {
                state.focus = crate::views::session_tree::SessionTreeFocus::List;
                state.label_draft.clear();
            }
            let Some(session_id) = agent.session.session_id.clone() else {
                return vec![];
            };
            app.show_toast(&match label.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(text) => format!("Labeled {entry_id}: {text}"),
                None => format!("Cleared label on {entry_id}"),
            });
            vec![Effect::PiSessionRequest { agent_id, session_id, op: PiSessionOp::Tree { map: false } }]
        }
        PiSessionOp::RollbackPreview { .. } => {
            app.show_toast("Rollback preview sent");
            vec![]
        }
        PiSessionOp::RollbackExecute { .. } => {
            app.show_toast("Files rolled back");
            vec![]
        }
        PiSessionOp::Clone | PiSessionOp::Fork { .. } => {
            let Some(session_id) = payload
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                app.show_toast(&format!("{label} reply has no sessionId"));
                return vec![];
            };
            // Pi clears the editor after a clone; a fork puts the chosen message back in it.
            let editor_text = match op {
                PiSessionOp::Clone => Some(String::new()),
                _ => payload.get("text").and_then(serde_json::Value::as_str).map(str::to_owned),
            };
            let done = if label == "Clone" { "Cloned to new session" } else { "Forked to new session" };
            replace_session(app, agent_id, session_id.to_owned(), editor_text, done)
        }
    }
}

/// `/tree` (`map = false`) or `/tree-map`: open the modal loading, then ask Pi for the tree.
pub(in crate::app::dispatch) fn dispatch_show_tree(app: &mut AppView, map: bool) -> Vec<Effect> {
    let effects = request(app, if map { "/tree-map" } else { "/tree" }, PiSessionOp::Tree { map }, "Loading the session tree…");
    if let Some(Effect::PiSessionRequest { agent_id, .. }) = effects.first()
        && let Some(agent) = app.agents.get_mut(agent_id)
    {
        let window = crate::views::modal_window::ModalWindowState::new();
        agent.active_modal = Some(if map {
            ActiveModal::TreeMap { state: crate::views::tree_map::TreeMapState::loading(), window }
        } else {
            let mut state = crate::views::session_tree::SessionTreeState::loading();
            state.skip_summary_prompt = crate::appearance::cache::load_pi_tree_skip_summary_prompt();
            ActiveModal::SessionTree { state, window }
        });
    }
    effects
}

/// Navigate, label or roll back from the tree. Navigating closes the modal; Pi reports the summary step in its reply.
pub(in crate::app::dispatch) fn dispatch_tree_op(app: &mut AppView, op: PiSessionOp) -> Vec<Effect> {
    let toast = match &op {
        PiSessionOp::NavigateTree { summarize: true, custom_instructions: Some(text), .. } if !text.trim().is_empty() => {
            "Navigating · generating branch summary (custom)…"
        }
        PiSessionOp::NavigateTree { summarize: true, .. } => "Navigating · generating branch summary…",
        PiSessionOp::NavigateTree { .. } => "Navigating session tree…",
        PiSessionOp::RollbackPreview { .. } => "Previewing file rollback…",
        PiSessionOp::RollbackExecute { .. } => "Rolling back files…",
        _ => "Saving label…",
    };
    let navigate = matches!(op, PiSessionOp::NavigateTree { .. });
    let effects = request(app, "/tree", op, toast);
    if navigate
        && let Some(Effect::PiSessionRequest { agent_id, .. }) = effects.first()
        && let Some(agent) = app.agents.get_mut(agent_id)
    {
        agent.active_modal = None;
    }
    effects
}

/// `{nodes: [...]}` from `pi/session/tree`; nodes without an id are dropped.
fn tree_nodes(payload: &serde_json::Value) -> Vec<crate::app::actions::SessionTreeNode> {
    let text = |node: &serde_json::Value, key: &str| node.get(key).and_then(serde_json::Value::as_str).map(str::to_owned);
    let flag = |node: &serde_json::Value, key: &str, default: bool| {
        node.get(key).and_then(serde_json::Value::as_bool).unwrap_or(default)
    };
    payload
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|node| {
            Some(crate::app::actions::SessionTreeNode {
                id: text(node, "id")?,
                parent_id: text(node, "parentId"),
                depth: node.get("depth").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize,
                is_leaf: flag(node, "isLeaf", false),
                is_current: flag(node, "isCurrent", false),
                on_active_path: flag(node, "onActivePath", false),
                role: text(node, "role").unwrap_or_default(),
                preview: text(node, "preview").unwrap_or_default(),
                detail: text(node, "detail").unwrap_or_default(),
                label: text(node, "label"),
                label_timestamp: text(node, "labelTimestamp"),
                entry_type: text(node, "entryType").unwrap_or_default(),
                timestamp: text(node, "timestamp"),
                child_ids: node
                    .get("childIds")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect(),
                has_text: flag(node, "hasText", true),
            })
        })
        .collect()
}

/// `{messages: [{entryId, text}]}`; rows without an id are dropped.
fn fork_messages(payload: &serde_json::Value) -> Vec<PiForkMessage> {
    payload
        .get("messages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| {
            let entry_id = message
                .get("entryId")
                .or_else(|| message.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())?
                .to_owned();
            let text = message.get("text").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
            Some(PiForkMessage { entry_id, text })
        })
        .collect()
}

/// Pi switched this session's view to a new session file: reset the transcript and load it on the same Pi.
fn replace_session(
    app: &mut AppView,
    agent_id: AgentId,
    session_id: String,
    editor_text: Option<String>,
    done: &str,
) -> Vec<Effect> {
    // The new id is unknown to the leader; the agent choice routes the load to this session's Pi.
    let agent_choice = app
        .agents
        .get(&agent_id)
        .and_then(|agent| agent.machine_name.as_deref())
        .and_then(|name| app.machine_picker.as_ref()?.choice_named(name).cloned());
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return vec![];
    };
    if let Some(text) = editor_text {
        agent.prompt.set_text(&text);
    }
    while agent.scrollback.in_batch() {
        agent.scrollback.end_batch();
    }
    if let Some(placeholder) = agent.loading_placeholder_id.take() {
        agent.scrollback.remove_entry(placeholder);
    }
    agent.abort_session_reload();
    agent.fork_state = None;
    agent.active_modal = None;
    agent.session.tracker = AcpUpdateTracker::new();
    agent.todo = crate::views::todo_pane::TodoPane::new();
    let mut scrollback = ScrollbackState::new();
    scrollback.set_appearance(agent.scrollback.appearance().clone());
    let placeholder = scrollback.push_block(RenderBlock::system(format!("Loading {session_id}…")));
    agent.scrollback = scrollback;
    agent.loading_placeholder_id = Some(placeholder);
    agent.begin_replay_window();
    agent.scrollback.begin_batch();
    agent.bind_session_id(acp::SessionId::new(session_id.clone()));
    let session_cwd = Some(agent.session.cwd.clone());
    let chat_kind = agent.chat_kind;
    app.show_toast(done);
    vec![Effect::LoadSession { agent_id, session_id, session_cwd, chat_kind, agent_choice }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_messages_read_entry_id_and_text_and_drop_blank_ids() {
        let messages = fork_messages(&serde_json::json!({ "messages": [
            { "entryId": "e1", "text": "hello" },
            { "id": "e2", "text": "world" },
            { "entryId": "  ", "text": "skip" },
        ] }));
        assert_eq!(
            messages,
            vec![
                PiForkMessage { entry_id: "e1".into(), text: "hello".into() },
                PiForkMessage { entry_id: "e2".into(), text: "world".into() },
            ]
        );
    }

    #[test]
    fn tree_nodes_parse_and_label_clears_with_an_empty_label() {
        let nodes = tree_nodes(&serde_json::json!({ "nodes": [
            { "id": "a", "role": "user", "preview": "hi", "childIds": ["b"], "isCurrent": true },
            { "parentId": "a" },
        ] }));
        assert_eq!(nodes.len(), 1);
        assert_eq!((nodes[0].id.as_str(), nodes[0].child_ids.as_slice()), ("a", &["b".to_owned()][..]));
        assert!(nodes[0].is_current && nodes[0].has_text);
        let sid = acp::SessionId::new("s1");
        let clear = PiSessionOp::TreeLabel { entry_id: "a".into(), label: Some("  ".into()) };
        assert_eq!(clear.params(&sid), serde_json::json!({ "entryId": "a", "clear": true, "sessionId": "s1" }));
        let go = PiSessionOp::NavigateTree { entry_id: "a".into(), summarize: true, custom_instructions: Some("why".into()) };
        assert_eq!(
            go.params(&sid),
            serde_json::json!({ "entryId": "a", "summarize": true, "customInstructions": "why", "sessionId": "s1" })
        );
    }

    #[test]
    fn pi_session_ops_carry_the_session_id_for_the_leader() {
        let sid = acp::SessionId::new("s1");
        let fork = PiSessionOp::Fork { entry_id: "e1".into() };
        assert_eq!(fork.method(), "pi/session/fork");
        assert_eq!(fork.params(&sid), serde_json::json!({ "entryId": "e1", "sessionId": "s1" }));
        assert_eq!(PiSessionOp::Reload.params(&sid), serde_json::json!({ "sessionId": "s1" }));
    }
}
