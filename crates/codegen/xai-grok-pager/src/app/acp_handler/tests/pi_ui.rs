//! `pi/ui/*` from grok-pi-acp lands on the session's native surfaces: statuses, widgets, toasts, scrollback.
use super::*;

fn pi(method: &str, params: serde_json::Value) -> acp::ExtNotification {
    acp::ExtNotification::new(method, serde_json::value::to_raw_value(&params).unwrap().into())
}

#[test]
fn pi_status_and_widget_set_and_clear_on_the_session() {
    let mut app = make_app_with_agent("sess-pi");
    assert!(handle_ext_notification(
        &pi("pi/ui/status", serde_json::json!({ "statusKey": "plan", "statusText": "plan mode" })),
        &mut app,
    ));
    assert!(handle_ext_notification(
        &pi("pi/ui/widget", serde_json::json!({ "widgetKey": "todo", "widgetLines": ["[ ] one", "[x] two"] })),
        &mut app,
    ));
    let agent = test_agent(&app, AgentId(0));
    assert_eq!(agent.pi_statuses.get("plan").map(String::as_str), Some("plan mode"));
    assert_eq!(agent.pi_widgets.get("todo").map(Vec::len), Some(2));

    handle_ext_notification(&pi("pi/ui/status", serde_json::json!({ "statusKey": "plan" })), &mut app);
    handle_ext_notification(&pi("pi/ui/widget", serde_json::json!({ "widgetKey": "todo", "widgetLines": null })), &mut app);
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.pi_statuses.is_empty());
    assert!(agent.pi_widgets.is_empty());
}

#[test]
fn pi_reset_clears_and_remote_tui_frames_go_to_the_remote_surface() {
    let mut app = make_app_with_agent("sess-pi");
    handle_ext_notification(&pi("pi/ui/status", serde_json::json!({ "key": "a", "text": "b" })), &mut app);
    assert!(handle_ext_notification(
        &pi("pi/ui/widget", serde_json::json!({ "widgetKey": "remote_tui", "widgetLines": ["frame"] })),
        &mut app,
    ));
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.pi_widgets.is_empty());
    assert_eq!(agent.pi_remote_tui.lines, vec!["frame".to_owned()]);
    assert!(agent.pi_remote_tui.is_open());
    assert!(agent.session.turn_saw_agent_text, "a Remote TUI screen is a forwarded command's answer");
    assert!(handle_ext_notification(&pi("pi/ui/reset_extension_ui", serde_json::json!({})), &mut app));
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.pi_statuses.is_empty());
    assert!(!agent.pi_remote_tui.is_open() && agent.pi_remote_tui.lines.is_empty());
}

fn pi_inputs(app: &mut AppView) -> Vec<(&'static str, serde_json::Value)> {
    std::mem::take(&mut app.pending_effects)
        .into_iter()
        .filter_map(|effect| match effect {
            crate::app::actions::Effect::PiUiInput { method, params, .. } => Some((method, params)),
            _ => None,
        })
        .collect()
}

fn key(code: crossterm::event::KeyCode, modifiers: crossterm::event::KeyModifiers) -> crossterm::event::Event {
    crossterm::event::Event::Key(crossterm::event::KeyEvent::new(code, modifiers))
}

#[test]
fn open_remote_tui_takes_the_keys_and_the_prompt_stays_untouched() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = make_app_with_agent("sess-pi");
    for (key, line) in [
        ("__pi_grok_remote_tui_session__", r#"{"op":"open","id":"login-1"}"#),
        ("remote_tui", "\u{1b}[36m→ xAI\u{1b}[0m"),
    ] {
        handle_ext_notification(&pi("pi/ui/widget", serde_json::json!({ "widgetKey": key, "widgetLines": [line] })), &mut app);
    }
    app.handle_input(&key(KeyCode::Down, KeyModifiers::NONE));
    app.handle_input(&key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(
        pi_inputs(&mut app),
        vec![
            ("pi/ui/remote_tui/input", serde_json::json!({ "id": "login-1", "data": "\u{1b}[B" })),
            ("pi/ui/remote_tui/input", serde_json::json!({ "id": "login-1", "data": "x" })),
        ]
    );
    assert!(test_agent(&app, AgentId(0)).prompt.text().is_empty());

    // Ctrl+Shift+Esc drops the component and hands the keyboard back.
    app.handle_input(&key(KeyCode::Esc, KeyModifiers::CONTROL | KeyModifiers::SHIFT));
    assert_eq!(pi_inputs(&mut app), vec![("pi/ui/remote_tui/cancel", serde_json::json!({ "id": "login-1" }))]);
    assert!(!test_agent(&app, AgentId(0)).pi_remote_tui.is_open());
    app.handle_input(&key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(pi_inputs(&mut app).is_empty());
}

#[test]
fn a_registered_extension_shortcut_is_dispatched_to_pi() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = make_app_with_agent("sess-pi");
    assert!(handle_ext_notification(
        &pi(
            "pi/ui/shortcuts",
            serde_json::json!({ "shortcuts": [{ "key": "alt+t", "description": "tutor", "extension": "pi-tutor" }] }),
        ),
        &mut app,
    ));
    app.handle_input(&key(KeyCode::Char('t'), KeyModifiers::ALT));
    assert_eq!(pi_inputs(&mut app), vec![("pi/ui/shortcut_dispatch", serde_json::json!({ "key": "alt+t" }))]);
    app.handle_input(&key(KeyCode::Char('y'), KeyModifiers::ALT));
    assert!(pi_inputs(&mut app).is_empty());
}

#[test]
fn pi_plan_file_points_view_plan_at_the_sidecar() {
    let mut app = make_app_with_agent("sess-pi");
    assert!(handle_ext_notification(
        &pi("pi/ui/plan_file", serde_json::json!({ "planFilePath": "/tmp/pi-plan.md" })),
        &mut app,
    ));
    assert_eq!(
        test_agent(&app, AgentId(0)).pi_plan_file_path.as_deref(),
        Some(std::path::Path::new("/tmp/pi-plan.md"))
    );
}

#[test]
fn pi_info_notify_goes_to_scrollback() {
    let mut app = make_app_with_agent("sess-pi");
    let before = test_agent(&app, AgentId(0)).scrollback.len();
    assert!(handle_ext_notification(
        &pi("pi/ui/notify", serde_json::json!({ "message": "eval ready", "notifyType": "info" })),
        &mut app,
    ));
    assert_eq!(test_agent(&app, AgentId(0)).scrollback.len(), before + 1);
    assert!(!handle_ext_notification(&pi("pi/unknown", serde_json::json!({})), &mut app));
}

#[test]
fn pi_session_catalog_fills_the_open_resume_picker() {
    let mut app = make_app_with_agent("sess-pi");
    app.agents.get_mut(&AgentId(0)).unwrap().active_modal = Some(crate::views::modal::session_picker_modal(None));
    assert!(handle_ext_notification(
        &pi(
            "pi/ui/session_catalog",
            serde_json::json!({ "scope": "current", "sessions": [
                { "id": "pi-1", "name": "refactor", "cwd": "/w/a", "updatedAt": 1_700_000_000_000_i64, "messageCount": 4 },
                { "summary": "no id, dropped" },
            ] }),
        ),
        &mut app,
    ));
    let Some(crate::views::modal::ActiveModal::SessionPicker { entries: Some(rows), loading, .. }) =
        &test_agent(&app, AgentId(0)).active_modal
    else {
        panic!("picker should hold Pi rows");
    };
    assert!(!loading);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].id.as_str(), rows[0].summary.as_str(), rows[0].source.as_str()), ("pi-1", "refactor", "pi"));
    assert_eq!(rows[0].num_messages, 4);
}

#[test]
fn sessionless_pi_ui_lands_on_the_pi_session_not_the_claude_one_on_screen() {
    let mut app = make_app_with_agent("sess-claude");
    app.external_agent = true;
    app.agents.get_mut(&AgentId(0)).unwrap().agent_label = Some("claude-agent".into());
    let mut pi_agent = make_agent(Some("sess-pi"));
    pi_agent.agent_label = Some("grok-pi".into());
    app.agents.insert(AgentId(1), pi_agent);
    assert!(handle_ext_notification(
        &pi("pi/ui/widget", serde_json::json!({ "widgetKey": "todo", "widgetLines": ["[ ] one"] })),
        &mut app,
    ));
    assert!(test_agent(&app, AgentId(0)).pi_widgets.is_empty());
    assert_eq!(test_agent(&app, AgentId(1)).pi_widgets.len(), 1);
}
