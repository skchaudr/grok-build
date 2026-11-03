//! The read-only past-session view: a made-up Claude transcript goes through the session-event handler, then is drawn at full and half width with vim mode on.

use super::{
    AgentPane, AgentView, AppRenderParams, BannerSlotParams, ComposerRoute, InputMode, OverlayHeader,
    test_fixtures,
};
use crate::acp::model_state::ModelState;
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::agent::AgentId;
use crate::app::app_view::{ActiveView, AppView, InputOutcome};
use crate::app::dispatch::{SwitchCause, standalone_traces, switch_to_agent};
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::ToolCallBlock;
use crate::scrollback::render::ScratchBuffer;
use crate::views::modal::ActiveModal;
use crate::views::traces_modal::TracesModalState;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::{Value, json};

fn at(sec: u64) -> String {
    format!("2026-01-01T00:{:02}:{:02}.000Z", sec / 60, sec % 60)
}

fn line(kind: &str, sec: u64, content: Value) -> Value {
    json!({ "type": kind, "timestamp": at(sec), "promptId": "p-1", "message": { "id": format!("m{sec}"), "role": kind, "content": content } })
}

fn tool_use(sec: u64, id: &str, name: &str, input: Value) -> Value {
    line(
        "assistant",
        sec,
        json!([{ "type": "tool_use", "id": id, "name": name, "input": input }]),
    )
}

fn result(sec: u64, id: &str, text: &str, tur: Value) -> Value {
    let mut l = line(
        "user",
        sec,
        json!([{ "type": "tool_result", "tool_use_id": id, "content": text }]),
    );
    l["toolUseResult"] = tur;
    l
}

/// A short session: a prompt, thinking, a command that took 2s, two reads, a search, an edit, a plan, a finished subagent and an answer.
fn transcript() -> Vec<Value> {
    let mut thinking = line(
        "assistant",
        1,
        json!([{ "type": "thinking", "thinking": "", "signature": "sig" }]),
    );
    thinking["thinkingDurationMs"] = json!(3000);
    vec![
        line(
            "user",
            0,
            json!("Find where the parser reads config and fix the typo."),
        ),
        thinking,
        line(
            "assistant",
            2,
            json!([{ "type": "text", "text": "I'll look at the build first." }]),
        ),
        tool_use(
            3,
            "t1",
            "Bash",
            json!({ "command": "cargo build", "description": "Build the crate" }),
        ),
        result(5, "t1", "Compiling demo\nFinished", Value::Null),
        tool_use(
            6,
            "t2",
            "Read",
            json!({ "file_path": "/work/demo/src/config.rs" }),
        ),
        result(
            7,
            "t2",
            "1→fn load() {}",
            json!({ "file": { "content": "fn load() {}", "numLines": 1, "startLine": 1, "totalLines": 1 } }),
        ),
        tool_use(
            8,
            "t3",
            "Read",
            json!({ "file_path": "/work/demo/src/main.rs" }),
        ),
        result(
            9,
            "t3",
            "1→fn main() {}",
            json!({ "file": { "content": "fn main() {}", "numLines": 1, "startLine": 1, "totalLines": 1 } }),
        ),
        tool_use(10, "t4", "Grep", json!({ "pattern": "recieve" })),
        result(
            11,
            "t4",
            "Found 1 file",
            json!({ "filenames": ["/work/demo/src/config.rs"], "numFiles": 1 }),
        ),
        tool_use(
            12,
            "t5",
            "Edit",
            json!({ "file_path": "/work/demo/src/config.rs", "old_string": "recieve", "new_string": "receive" }),
        ),
        result(13, "t5", "updated", Value::Null),
        tool_use(
            14,
            "t6",
            "TodoWrite",
            json!({ "todos": [{ "content": "Fix typo", "status": "completed" }, { "content": "Add a test", "status": "pending" }] }),
        ),
        tool_use(
            15,
            "t7",
            "Agent",
            json!({ "description": "Review the diff", "subagent_type": "explore", "prompt": "review" }),
        ),
        result(
            20,
            "t7",
            "Looks right.",
            json!({ "status": "completed", "agentId": "r1", "totalDurationMs": 5000 }),
        ),
        line(
            "assistant",
            21,
            json!([{ "type": "text", "text": "Fixed the typo in config.rs." }]),
        ),
    ]
}

fn app() -> AppView {
    app_with(true)
}

fn app_with(vim: bool) -> AppView {
    // Thinking blocks follow the machine's `[ui]` setting; the view is checked with them on.
    crate::appearance::cache::set_show_thinking_blocks(true);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = AppView::new(
        tx,
        ModelState::default(),
        Vec::new(),
        crate::render::draw::EscapeWriter::disconnected(),
    );
    let id = AgentId(0);
    let mut agent = test_fixtures::make_agent();
    agent.vim_mode = vim;
    if vim {
        agent.set_input_mode(InputMode::Vim);
    } else {
        agent.set_active_pane(AgentPane::Prompt, true);
        agent.set_input_mode(InputMode::Simple);
    }
    agent.active_modal = Some(ActiveModal::Traces {
        state: Box::new(TracesModalState::open(None)),
    });
    app.agents.insert(id, agent);
    switch_to_agent(&mut app, id, SwitchCause::New);
    app
}

fn row(tmp: &tempfile::TempDir) -> xai_grok_traces::SessionRow {
    let path = tmp.path().join("s1.jsonl");
    let body: Vec<String> = transcript().iter().map(Value::to_string).collect();
    std::fs::write(&path, body.join("\n")).unwrap();
    xai_grok_traces::SessionRow {
        id: "claude:s1".into(),
        source: "claude".into(),
        file_path: path,
        cwd: Some("/work/demo".into()),
        ..Default::default()
    }
}

fn opened(tmp: &tempfile::TempDir) -> AppView {
    let mut app = app();
    standalone_traces::open_claude_in(&mut app, row(tmp), true);
    app
}

fn agent(app: &AppView) -> &AgentView {
    let ActiveView::Agent(id) = app.active_view else {
        panic!("agent view")
    };
    &app.agents[&id]
}

fn blocks(agent: &AgentView) -> Vec<&RenderBlock> {
    (0..agent.scrollback.len())
        .filter_map(|i| agent.scrollback.get(i))
        .map(|e| &e.block)
        .collect()
}

fn draw(agent: &mut AgentView, width: u16) -> String {
    agent.last_terminal_size = (width, 40);
    let area = Rect::new(0, 0, width, 40);
    let mut buf = Buffer::empty(area);
    let mut scratch = ScratchBuffer::new();
    agent.draw(
        area,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut scratch,
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams {
            overlay_header: OverlayHeader::default(),
            ..AppRenderParams::default()
        },
    );
    // Entry times are local-time clock strings; the snapshot must not depend on the machine's zone.
    let clock = regex::Regex::new(r"\d{1,2}:\d{2} [AP]M").unwrap();
    (0..area.height)
        .map(|y| {
            let row: String = (0..area.width)
                .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                .collect();
            clock.replace_all(row.trim_end(), "<time>").into_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_transcript_lands_in_grok_blocks_with_no_composer() {
    let tmp = tempfile::tempdir().unwrap();
    let app = opened(&tmp);
    let a = agent(&app);
    assert!(a.active_modal.is_none(), "the list steps aside");
    assert!(a.trace_return.is_some());
    assert_eq!(a.composer_route(), ComposerRoute::Hidden);
    assert_eq!(a.session.cwd.to_str(), Some("/work/demo"));

    let b = blocks(a);
    assert!(b.iter().any(|x| matches!(x, RenderBlock::UserPrompt(_))));
    assert!(b.iter().any(|x| matches!(x, RenderBlock::Thinking(_))));
    assert!(b.iter().any(|x| matches!(x, RenderBlock::AgentMessage(_))));
    assert!(
        b.iter().any(|x| matches!(x, RenderBlock::Subagent(_))),
        "the subagent row appears"
    );

    let build = b
        .iter()
        .find_map(|x| match x {
            RenderBlock::ToolCall(ToolCallBlock::Execute(e)) => Some(e),
            _ => None,
        })
        .expect("the command's own block");
    assert_eq!(build.command, "cargo build");
    assert_eq!(
        build.elapsed_ms(),
        Some(2000),
        "the call carries its real duration, not a live clock"
    );
    assert!(
        build
            .output
            .as_deref()
            .is_some_and(|o| o.contains("Finished"))
    );
    assert!(
        b.iter()
            .any(|x| matches!(x, RenderBlock::ToolCall(ToolCallBlock::Edit(_))))
    );
    assert!(
        b.iter()
            .any(|x| matches!(x, RenderBlock::ToolCall(ToolCallBlock::Search(_))))
    );
    assert_eq!(
        b.iter()
            .filter(|x| matches!(x, RenderBlock::ToolCall(ToolCallBlock::Read(_))))
            .count(),
        2
    );
    assert_eq!(a.todo.todos().len(), 2, "TodoWrite fills the plan pane");
    assert_eq!(a.subagent_sessions.len(), 1);
}

#[test]
fn esc_goes_back_to_the_sessions_list_and_the_view_empties() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = opened(&tmp);
    let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let InputOutcome::Action(Action::TracesLeaveView) = app.handle_input(&esc) else {
        panic!("Esc in the read-only view leaves it")
    };
    standalone_traces::leave_view(&mut app);
    let a = agent(&app);
    assert!(
        matches!(a.active_modal, Some(ActiveModal::Traces { .. })),
        "the sessions list is back"
    );
    assert!(a.trace_return.is_none());
    assert_eq!(a.scrollback.len(), 0);
    assert!(a.subagent_sessions.is_empty());
    assert_eq!(a.composer_route(), ComposerRoute::RootSession);
}

#[test]
fn inside_a_grok_session_the_view_opens_beside_it_and_esc_leaves_the_session_as_it_was() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app();
    let origin = AgentId(0);
    // The fixture inserts its agent by hand; real creation paths advance the counter.
    app.next_agent_id = 1;
    {
        let a = app.agents.get_mut(&origin).unwrap();
        a.bind_session_id(agent_client_protocol::SessionId::new("grok-live"));
        a.scrollback
            .push_block(RenderBlock::system("the user's own entry".to_string()));
    }
    standalone_traces::open_claude_in(&mut app, row(&tmp), false);
    assert_ne!(app.active_view, ActiveView::Agent(origin), "a view of its own");
    assert!(agent(&app).trace_return.is_some());
    assert!(blocks(agent(&app)).iter().any(|x| matches!(x, RenderBlock::ToolCall(_))));
    let o = &app.agents[&origin];
    assert_eq!(o.session.session_id.as_ref().map(|s| &*s.0), Some("grok-live"));
    assert_eq!(o.scrollback.len(), 1);

    standalone_traces::leave_view(&mut app);
    assert_eq!(app.active_view, ActiveView::Agent(origin));
    assert_eq!(app.agents.len(), 1, "the read-only view is closed");
    let o = &app.agents[&origin];
    assert!(matches!(o.active_modal, Some(ActiveModal::Traces { .. })), "the sessions list is back");
    assert_eq!(o.session.session_id.as_ref().map(|s| &*s.0), Some("grok-live"));
    assert_eq!(o.scrollback.len(), 1);
    assert!(matches!(o.scrollback.get(0).map(|e| &e.block), Some(RenderBlock::System(_))));
    assert_eq!(o.composer_route(), ComposerRoute::RootSession);
}

#[test]
fn esc_goes_back_with_vim_mode_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(false);
    standalone_traces::open_claude(&mut app, row(&tmp));
    assert_eq!(agent(&app).active_pane, AgentPane::Scrollback);
    let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(
        app.handle_input(&esc),
        InputOutcome::Action(Action::TracesLeaveView)
    ));
}

#[test]
fn cancel_and_rewind_do_nothing_in_a_subagent_opened_from_the_view() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = opened(&tmp);
    let ActiveView::Agent(id) = app.active_view else {
        panic!()
    };
    let parent = app.agents.get_mut(&id).unwrap();
    parent.insert_test_child("child".into(), Box::new(test_fixtures::make_agent()));
    parent.active_subagent = Some("child".into());
    for action in [Action::Rewind, Action::RewindShowPicker, Action::CancelTurn] {
        assert!(crate::app::dispatch::dispatch(action, &mut app).is_empty());
        assert!(agent(&app).subagent_views["child"].rewind_state.is_none());
    }
}

#[test]
fn cancel_and_rewind_do_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = opened(&tmp);
    for action in [Action::Rewind, Action::RewindShowPicker, Action::CancelTurn] {
        assert!(crate::app::dispatch::dispatch(action, &mut app).is_empty());
        assert!(agent(&app).rewind_state.is_none());
    }
}

#[test]
fn opening_twice_does_not_stack_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = opened(&tmp);
    let first = agent(&app).scrollback.len();
    standalone_traces::leave_view(&mut app);
    standalone_traces::open_claude_in(&mut app, row(&tmp), true);
    assert_eq!(agent(&app).scrollback.len(), first);
}

#[test]
fn an_unreadable_transcript_stays_on_the_list_with_a_status() {
    let mut app = app();
    let missing = xai_grok_traces::SessionRow {
        id: "claude:x".into(),
        source: "claude".into(),
        file_path: "/nope/x.jsonl".into(),
        ..Default::default()
    };
    standalone_traces::open_claude_in(&mut app, missing, true);
    let a = agent(&app);
    assert!(a.trace_return.is_none());
    let Some(ActiveModal::Traces { state }) = &a.active_modal else {
        panic!("list stays")
    };
    assert!(
        state
            .status
            .as_deref()
            .is_some_and(|s| s.contains("can't read"))
    );
}

#[test]
fn snapshot_full_width_vim() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = opened(&tmp);
    let ActiveView::Agent(id) = app.active_view else {
        panic!()
    };
    insta::assert_snapshot!(
        "trace_view_full_width",
        draw(app.agents.get_mut(&id).unwrap(), 100)
    );
}

#[test]
fn snapshot_half_width_vim() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = opened(&tmp);
    let ActiveView::Agent(id) = app.active_view else {
        panic!()
    };
    insta::assert_snapshot!(
        "trace_view_half_width",
        draw(app.agents.get_mut(&id).unwrap(), 50)
    );
}
