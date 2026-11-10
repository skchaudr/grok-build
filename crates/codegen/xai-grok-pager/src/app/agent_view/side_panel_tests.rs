//! Render snapshots for every side panel state: closed, open unfocused, focused, zoomed, each page, narrow terminal.

use super::super::{AgentView, AppRenderParams, BannerSlotParams, KeyOwner, test_fixtures};
use crate::actions::ActionRegistry;
use crate::app::app_view::InputOutcome;
use crate::scrollback::render::ScratchBuffer;
use crate::views::side_panel::Page;
use agent_client_protocol as acp;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;
use xai_grok_traces::SessionRow;

fn agent() -> AgentView {
    let mut agent = test_fixtures::make_agent();
    let id = acp::ModelId::new("grok-4.5");
    agent.session.models.available = [(
        id.clone(),
        acp::ModelInfo::new(id.clone(), "Grok 4.5".to_owned()),
    )]
    .into_iter()
    .collect();
    agent.session.models.set_current(id, None);
    agent.session.session_id = Some(acp::SessionId::new("01a1be7d-f425-7ca2-aed9-9097e2168b46"));
    agent.session.cwd = "/work/grok-build".into();
    agent.generated_session_title = Some("side panel".into());
    agent.apply_context_used(84_000, 200_000);
    agent.side_panel.five_hour_pct = Some(22);
    agent
}

fn draw(agent: &mut AgentView, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    let _ = agent.draw(
        area,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut ScratchBuffer::new(),
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(agent: &mut AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    )
}

/// Every call complete, so the snapshot does not depend on the clock.
fn follow(agent: &mut AgentView) {
    let item = |kind: &str, extra: serde_json::Value| {
        let mut v =
            json!({"v": 1, "kind": kind, "agent": "root", "depth": 0, "line": 0, "ts": 1_000});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().cloned().unwrap());
        v
    };
    let root = SessionRow {
        id: "root".into(),
        source: "grok".into(),
        title: "side panel".into(),
        model: Some("grok-4.5".into()),
        ..Default::default()
    };
    agent.side_panel.follow_for_test(
        root,
        &[
            item("user", json!({"text": "add the side panel"})),
            item("call", json!({"line": 3, "call_id": "t1", "tool": "Bash", "what": "cargo test"})),
            item("result", json!({"line": 4, "call_id": "t1", "ok": true, "error": "", "ms": 400})),
            item("call", json!({"line": 5, "call_id": "t2", "tool": "Read", "what": "src/views/side_panel.rs"})),
            item("result", json!({"line": 6, "call_id": "t2", "ok": false, "error": "exit 1  no such file", "ms": 100})),
        ],
        2_000,
    );
}

#[test]
fn closed() {
    let mut agent = agent();
    insta::assert_snapshot!(draw(&mut agent, 110, 24));
}

#[test]
fn focused_traces_page() {
    let mut agent = agent();
    assert!(matches!(
        press(&mut agent, KeyCode::F(3)),
        InputOutcome::Changed
    ));
    assert_eq!(agent.key_owner(), KeyOwner::SidePanel);
    follow(&mut agent);
    insta::assert_snapshot!(draw(&mut agent, 110, 24));
}

#[test]
fn traces_page_before_the_index_has_the_session() {
    let mut agent = agent();
    press(&mut agent, KeyCode::F(3));
    insta::assert_snapshot!(draw(&mut agent, 110, 24));
}

#[test]
fn focused_info_page() {
    let mut agent = agent();
    press(&mut agent, KeyCode::F(3));
    press(&mut agent, KeyCode::Right);
    assert_eq!(agent.side_panel.page, Page::Info);
    insta::assert_snapshot!(draw(&mut agent, 110, 30));
}

#[test]
fn open_unfocused_lets_the_chat_type() {
    let mut agent = agent();
    press(&mut agent, KeyCode::F(3));
    press(&mut agent, KeyCode::Right);
    press(&mut agent, KeyCode::Esc);
    assert!(agent.side_panel.open && agent.key_owner() != KeyOwner::SidePanel);
    insta::assert_snapshot!(draw(&mut agent, 110, 30));
}

#[test]
fn zoomed_swaps_the_sizes() {
    let mut agent = agent();
    press(&mut agent, KeyCode::F(3));
    press(&mut agent, KeyCode::Right);
    press(&mut agent, KeyCode::Char('z'));
    insta::assert_snapshot!(draw(&mut agent, 110, 30));
}

#[test]
fn narrow_terminal_focused_panel_takes_the_width() {
    let mut agent = agent();
    press(&mut agent, KeyCode::F(3));
    press(&mut agent, KeyCode::Right);
    insta::assert_snapshot!(draw(&mut agent, 50, 30));
}

#[test]
fn narrow_terminal_unfocused_panel_hides() {
    let mut agent = agent();
    press(&mut agent, KeyCode::F(3));
    press(&mut agent, KeyCode::Esc);
    insta::assert_snapshot!(draw(&mut agent, 50, 24));
}
