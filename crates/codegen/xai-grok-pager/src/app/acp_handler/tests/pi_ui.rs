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
fn dpi_info_lands_on_the_agent_for_the_settings_header() {
    let mut app = make_app_with_agent("sess-pi");
    assert!(handle_ext_notification(
        &pi(
            "pi/ui/dpi_info",
            serde_json::json!({ "device": "mini", "session": "modal-test", "agent": "Main", "cwd": "~/w",
                                "model": "gpt-5", "readOnly": true, "tools": ["read"], "playgroundUrl": null,
                                "hints": ["F2 opens settings"] }),
        ),
        &mut app,
    ));
    let info = test_agent(&app, AgentId(0)).dpi_info.clone().expect("info stored");
    assert_eq!((info.session.as_str(), info.read_only, info.playground_url), ("modal-test", true, None));
}

#[test]
fn only_the_durable_agents_keys_reach_pi_through_the_settings_panel() {
    use crate::views::modal::ActiveModal;
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = make_app_with_agent("sess-pi");
    handle_ext_notification(
        &pi(
            "pi/ui/shortcuts",
            serde_json::json!({ "shortcuts": [
                { "key": "alt+c", "description": "Compact the context", "label": "compact", "extension": "dpi" },
                { "key": "alt+t", "description": "tutor", "extension": "pi-tutor" },
            ] }),
        ),
        &mut app,
    );
    let panel = || ActiveModal::PiSettings {
        state: Box::new(crate::views::pi_settings::PiSettingsState::new(
            std::sync::Arc::new(crate::settings::SettingsRegistry::defaults()),
            Default::default(),
            Default::default(),
        )),
    };
    app.agents.get_mut(&AgentId(0)).unwrap().active_modal = Some(panel());
    app.handle_input(&key(KeyCode::Char('t'), KeyModifiers::ALT));
    assert!(pi_inputs(&mut app).is_empty(), "another extension's key stays out while the panel is open");
    assert!(test_agent(&app, AgentId(0)).active_modal.is_some());

    app.handle_input(&key(KeyCode::Char('c'), KeyModifiers::ALT));
    assert_eq!(pi_inputs(&mut app), vec![("pi/ui/shortcut_dispatch", serde_json::json!({ "key": "alt+c" }))]);
    assert!(test_agent(&app, AgentId(0)).active_modal.is_none(), "the panel closes so the result is in view");
    assert_eq!(
        crate::settings::dpi::action_captions(test_agent(&app, AgentId(0)).pi_shortcuts.as_ref()),
        ["Alt+C compact"]
    );
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
    app.external_agent = true;
    app.external_agent_label = Some("pi".into());
    let mut modal = crate::views::modal::session_picker_modal(None);
    // `/resume` under Pi opens the picker on the External source filter (see dispatch_show_session_picker).
    if let crate::views::modal::ActiveModal::SessionPicker { source_filter, .. } = &mut modal {
        *source_filter = crate::views::session_picker::SourceFilter::External;
    }
    app.agents.get_mut(&AgentId(0)).unwrap().active_modal = Some(modal);
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
    // The scoped catalog keeps the session name in `name`; `summary` is the message preview.
    assert_eq!((rows[0].id.as_str(), rows[0].name.as_deref(), rows[0].source.as_str()), ("pi-1", Some("refactor"), "pi"));
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

/// The shim re-sends the attach card after each turn with `refresh`: the shown card updates in place, and a refresh
/// with no card shown adds nothing.
#[test]
fn attach_card_refresh_updates_the_shown_card_in_place() {
    let mut app = make_app_with_agent("sess-pi");
    let card = |activity: &str, refresh: bool| {
        pi(
            "pi/ui/attach_card",
            serde_json::json!({ "title": "dpi · a on mini", "rows": [["activity", activity]], "refresh": refresh }),
        )
    };
    assert!(handle_ext_notification(&card("no messages yet", true), &mut app));
    assert_eq!(test_agent(&app, AgentId(0)).scrollback.len(), 0, "nothing shown: a refresh adds no card");

    assert!(handle_ext_notification(&card("no messages yet", false), &mut app));
    assert!(handle_ext_notification(&card("last message just now · 2 messages", true), &mut app));
    let agent = test_agent(&app, AgentId(0));
    let cards: Vec<_> = (0..agent.scrollback.len())
        .filter_map(|i| match &agent.scrollback.entry(i).unwrap().block {
            crate::scrollback::RenderBlock::AttachCard(card) => Some(card.rows.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cards, vec![vec![("activity".to_owned(), "last message just now · 2 messages".to_owned())]]);
}

#[test]
fn pi_notifications_are_kept_for_notify() {
    let mut app = make_app_with_agent("sess-pi");
    handle_ext_notification(&pi("pi/ui/notify", serde_json::json!({ "message": "eval ready", "notifyType": "info" })), &mut app);
    handle_ext_notification(&pi("pi/ui/notify", serde_json::json!({ "message": "quota low", "notifyType": "warning" })), &mut app);
    let kept = &test_agent(&app, AgentId(0)).pi_notifications;
    assert_eq!(kept.len(), 2);
    let list = crate::views::modal::NotificationListState::new(kept.clone());
    assert_eq!(list.filtered()[0].message, "quota low", "newest first");
}

#[test]
fn dpi_catalog_rows_show_their_live_activity_on_the_dashboard() {
    use crate::app::roster::RosterActivity;
    let mut app = make_app_with_agent("s-a:main");
    app.external_agent = true;
    app.external_agent_label = Some("pi".into());
    app.active_view = crate::app::app_view::ActiveView::AgentDashboard;
    assert!(handle_ext_notification(
        &pi(
            "pi/ui/session_catalog",
            serde_json::json!({ "scope": "current", "sessions": [
                { "id": "s-b:main", "cwd": "/w", "activity": "working" },
                { "id": "s-c:main", "cwd": "/w", "activity": "idle" },
                { "id": "local-pi", "cwd": "/w" },
            ] }),
        ),
        &mut app,
    ));
    let row = |id: &str| app.dashboard_local_sessions.iter().find(|row| row.session_id == id).unwrap();
    assert_eq!(row("s-b:main").activity, RosterActivity::Working);
    assert_eq!(row("s-c:main").activity, RosterActivity::Idle);
    assert!(row("s-c:main").resident, "a durable agent is live on its backend");
    assert_eq!(row("local-pi").activity, RosterActivity::Dormant, "no activity: unchanged");
    assert!(!row("local-pi").resident);
}

/// Ctrl+X's "Stopping the agent's turn" toast stays while the roster still shows the agent working, and clears once it
/// goes idle.
#[test]
fn dpi_stopping_toast_clears_when_the_stopped_row_goes_idle() {
    use crate::views::dashboard::state::STOPPING_TURN_TOAST;
    let mut app = make_app_with_agent("s-a:main");
    app.external_agent = true;
    app.external_agent_label = Some("pi".into());
    app.active_view = crate::app::app_view::ActiveView::AgentDashboard;
    let mut dashboard = crate::views::dashboard::DashboardState::default();
    dashboard.error_toast = Some(STOPPING_TURN_TOAST.to_owned());
    dashboard.stopping_turn = Some("s-b:main".to_owned());
    app.dashboard = Some(dashboard);
    let catalog = |activity: &str| {
        pi(
            "pi/ui/session_catalog",
            serde_json::json!({ "scope": "current", "sessions": [
                { "id": "s-b:main", "cwd": "/w", "activity": activity },
                { "id": "s-c:main", "cwd": "/w", "activity": "idle" },
            ] }),
        )
    };
    handle_ext_notification(&catalog("working"), &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(d.error_toast.as_deref(), Some(STOPPING_TURN_TOAST), "abort not landed yet: keep the toast");
    assert_eq!(d.stopping_turn.as_deref(), Some("s-b:main"));

    handle_ext_notification(&catalog("idle"), &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(d.error_toast, None);
    assert_eq!(d.stopping_turn, None);
}

/// A newer toast that replaced the stopping toast is left alone when the stopped row goes idle.
#[test]
fn dpi_stopped_row_going_idle_leaves_a_newer_toast() {
    let mut app = make_app_with_agent("s-a:main");
    app.external_agent = true;
    app.external_agent_label = Some("pi".into());
    app.active_view = crate::app::app_view::ActiveView::AgentDashboard;
    let mut dashboard = crate::views::dashboard::DashboardState::default();
    dashboard.error_toast = Some("\u{2717} Select a session first".to_owned());
    dashboard.stopping_turn = Some("s-b:main".to_owned());
    app.dashboard = Some(dashboard);
    handle_ext_notification(
        &pi(
            "pi/ui/session_catalog",
            serde_json::json!({ "scope": "current", "sessions": [{ "id": "s-b:main", "cwd": "/w", "activity": "idle" }] }),
        ),
        &mut app,
    );
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(d.error_toast.as_deref(), Some("\u{2717} Select a session first"));
    assert_eq!(d.stopping_turn, None);
}

/// dpi rows: main agents (no parent) and extra agents; hidden extra agents stay off the dashboard unless Ctrl+Y shows
/// them, marked hidden. A main agent is never hidden.
#[test]
fn dpi_catalog_hides_hidden_extra_agents_and_records_row_kinds() {
    use crate::views::dashboard::DpiRowKind;
    let mut app = make_app_with_agent("s-a:main");
    app.external_agent = true;
    app.external_agent_label = Some("pi".into());
    app.active_view = crate::app::app_view::ActiveView::AgentDashboard;
    app.dashboard = Some(crate::views::dashboard::DashboardState::default());
    let catalog = pi(
        "pi/ui/session_catalog",
        serde_json::json!({ "scope": "current", "sessions": [
            { "id": "s-b:main", "cwd": "/w", "summary": "beta", "activity": "idle", "hidden": true },
            { "id": "s-b:2", "cwd": "/w", "summary": "beta / helper", "parentSessionPath": "dpi://s-b/main", "activity": "idle", "hidden": true },
            { "id": "s-b:3", "cwd": "/w", "summary": "beta / other", "parentSessionPath": "dpi://s-b/main", "activity": "idle" },
        ] }),
    );
    handle_ext_notification(&catalog, &mut app);
    let ids: Vec<_> = app.dashboard_local_sessions.iter().map(|r| r.session_id.as_str()).collect();
    assert_eq!(ids, ["s-b:main", "s-b:3"]);
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(d.hidden_agent_count, 1);
    assert_eq!(d.dpi_rows.get("s-b:main"), Some(&DpiRowKind::Main));
    assert_eq!(d.dpi_rows.get("s-b:3"), Some(&DpiRowKind::Extra { hidden: false }));
    assert_eq!(d.dpi_rows.get("s-b:2"), None);

    app.dashboard.as_mut().unwrap().show_hidden_agents = true;
    handle_ext_notification(&catalog, &mut app);
    let row = app.dashboard_local_sessions.iter().find(|r| r.session_id == "s-b:2").unwrap();
    assert_eq!(row.title.as_deref(), Some("beta / helper (hidden)"));
    assert_eq!(app.dashboard.as_ref().unwrap().dpi_rows.get("s-b:2"), Some(&DpiRowKind::Extra { hidden: true }));
}
