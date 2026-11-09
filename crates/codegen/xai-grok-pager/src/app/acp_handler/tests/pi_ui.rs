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
fn pi_reset_clears_and_remote_tui_widgets_are_not_rendered() {
    let mut app = make_app_with_agent("sess-pi");
    handle_ext_notification(&pi("pi/ui/status", serde_json::json!({ "key": "a", "text": "b" })), &mut app);
    assert!(handle_ext_notification(
        &pi("pi/ui/widget", serde_json::json!({ "widgetKey": "remote_tui", "widgetLines": ["frame"] })),
        &mut app,
    ));
    assert!(test_agent(&app, AgentId(0)).pi_widgets.is_empty());
    assert!(handle_ext_notification(&pi("pi/ui/reset_extension_ui", serde_json::json!({})), &mut app));
    assert!(test_agent(&app, AgentId(0)).pi_statuses.is_empty());
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
