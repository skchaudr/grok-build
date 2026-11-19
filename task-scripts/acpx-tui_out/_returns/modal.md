```rust
pub(crate) mod inspector;
pub(crate) mod ledger;
pub(crate) mod run;

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{self, Run, RunSummary};
use crate::acpx::sessions::{self, SessionSummary};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{
    self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowOutcome, ModalWindowState,
    Shortcut,
};

use self::inspector::{InspectorOutcome, InspectorState};
use self::ledger::{LedgerOutcome, LedgerState};
use self::run::{RunOutcome, RunViewState};

const MAX_TITLE_WIDTH: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ledger,
    Run,
    Step,
}

/// A conversation to replay as a transcript; the wiring feeds it to `crate::acp::acpx_feed::convert`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayRequest {
    pub events_path: PathBuf,
    pub range: Option<(u64, u64)>,
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AcpxOutcome {
    Unchanged,
    Changed,
    Close,
    Copy(String),
    Replay(ReplayRequest),
}

pub struct AcpxModalState {
    pub window: ModalWindowState,
    runs_base: Option<PathBuf>,
    sessions_base: Option<PathBuf>,
    runs_snapshot: Vec<RunSummary>,
    ledger: LedgerState,
    run_view: Option<RunViewState>,
    inspector: Option<(usize, InspectorState)>,
    mode: Mode,
    help_open: bool,
    notice: Option<String>,
}

impl AcpxModalState {
    pub fn open() -> Self {
        Self::open_at(bundle::runs_base(), sessions::sessions_base())
    }

    pub fn open_at(runs_base: Option<PathBuf>, sessions_base: Option<PathBuf>) -> Self {
        let mut state = Self {
            window: ModalWindowState::new(),
            runs_base,
            sessions_base,
            runs_snapshot: Vec::new(),
            ledger: LedgerState::new(),
            run_view: None,
            inspector: None,
            mode: Mode::Ledger,
            help_open: false,
            notice: None,
        };
        state.rescan_runs();
        state.rescan_sessions();
        state
    }

    pub fn open_run(&mut self, dir: &Path) -> bool {
        match bundle::load_run(dir) {
            Ok(run) => {
                self.run_view = Some(RunViewState::new(run));
                self.inspector = None;
                self.mode = Mode::Run;
                self.help_open = false;
                self.notice = None;
                true
            }
            Err(err) => {
                self.notice = Some(format!("cannot open run: {err}"));
                false
            }
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn wants_poll(&self) -> bool {
        match self.mode {
            Mode::Ledger => self.ledger.any_live(),
            Mode::Run | Mode::Step => self
                .run_view
                .as_ref()
                .is_some_and(|rv| rv.run().summary.status.is_live()),
        }
    }

    pub fn poll(&mut self) -> bool {
        match self.mode {
            Mode::Ledger => self.rescan_runs(),
            Mode::Run | Mode::Step => {
                let Some(rv) = self.run_view.as_mut() else {
                    return false;
                };
                if bundle::refresh(rv.run_mut()) {
                    rv.sync();
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn wants_esc(&self) -> bool {
        self.help_open || self.mode != Mode::Ledger || !self.ledger.esc_closes()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let had_notice = self.notice.take().is_some();

        if self.help_open {
            self.help_open = false;
            return AcpxOutcome::Changed;
        }

        let captures = self.mode == Mode::Ledger && self.ledger.captures_text();
        if !captures && matches!(key.code, KeyCode::Char('?')) {
            self.help_open = true;
            return AcpxOutcome::Changed;
        }

        let outcome = match self.mode {
            Mode::Ledger => self.handle_ledger_key(key),
            Mode::Run => self.handle_run_key(key),
            Mode::Step => self.handle_step_key(key),
        };

        if outcome == AcpxOutcome::Unchanged && had_notice {
            AcpxOutcome::Changed
        } else {
            outcome
        }
    }

    pub fn handle_mouse(&mut self, mouse: &MouseEvent) -> AcpxOutcome {
        match modal_window::handle_modal_mouse(
            &mut self.window,
            mouse.kind,
            mouse.column,
            mouse.row,
        ) {
            ModalWindowOutcome::CloseRequested => AcpxOutcome::Close,
            ModalWindowOutcome::Handled
            | ModalWindowOutcome::TabChanged(_)
            | ModalWindowOutcome::ShortcutActivated(_)
            | ModalWindowOutcome::CollapseGroup
            | ModalWindowOutcome::ExpandGroup
            | ModalWindowOutcome::CollapseDetails
            | ModalWindowOutcome::ExpandDetails
            | ModalWindowOutcome::JumpToParent(_) => AcpxOutcome::Changed,
            ModalWindowOutcome::Unhandled => {
                if self.help_open {
                    return AcpxOutcome::Unchanged;
                }
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => -3,
                    MouseEventKind::ScrollDown => 3,
                    _ => return AcpxOutcome::Unchanged,
                };
                let moved = match self.mode {
                    Mode::Ledger => self.ledger.scroll(delta),
                    Mode::Run => self.run_view.as_mut().is_some_and(|rv| rv.scroll(delta)),
                    Mode::Step => self
                        .inspector
                        .as_mut()
                        .is_some_and(|(_, ins)| ins.scroll(delta)),
                };
                if moved {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
        }
    }

    fn rescan_runs(&mut self) -> bool {
        let runs = self
            .runs_base
            .as_deref()
            .map(bundle::list_runs)
            .unwrap_or_default();
        let changed = runs != self.runs_snapshot;
        self.runs_snapshot = runs.clone();
        self.ledger.set_runs(runs);
        changed
    }

    fn rescan_sessions(&mut self) {
        let list = self
            .sessions_base
            .as_deref()
            .map(sessions::list_sessions)
            .unwrap_or_default();
        self.ledger.set_sessions(list);
    }

    fn handle_ledger_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        match self.ledger.handle_key(key) {
            LedgerOutcome::Unchanged => AcpxOutcome::Unchanged,
            LedgerOutcome::Changed => AcpxOutcome::Changed,
            LedgerOutcome::OpenRun(dir) => {
                self.open_run(&dir);
                AcpxOutcome::Changed
            }
            LedgerOutcome::OpenSession(s) => AcpxOutcome::Replay(session_replay(s)),
            LedgerOutcome::Copy(text) => AcpxOutcome::Copy(text),
            LedgerOutcome::Rescan => {
                self.rescan_runs();
                self.rescan_sessions();
                AcpxOutcome::Changed
            }
            LedgerOutcome::Close => AcpxOutcome::Close,
        }
    }

    fn handle_run_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some(rv) = self.run_view.as_mut() else {
            self.mode = Mode::Ledger;
            return AcpxOutcome::Changed;
        };
        match rv.handle_key(key) {
            RunOutcome::Unchanged => AcpxOutcome::Unchanged,
            RunOutcome::Changed => AcpxOutcome::Changed,
            RunOutcome::Back => {
                self.mode = Mode::Ledger;
                self.run_view = None;
                self.inspector = None;
                self.rescan_runs();
                AcpxOutcome::Changed
            }
            RunOutcome::OpenStep(idx) => {
                if self.open_step_at(idx) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Replay(idx) => self.replay_step(idx),
            RunOutcome::Copy(text) => AcpxOutcome::Copy(text),
        }
    }

    fn handle_step_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some((step_idx, ins)) = self.inspector.as_mut() else {
            self.mode = Mode::Run;
            return AcpxOutcome::Changed;
        };
        let idx = *step_idx;
        match ins.handle_key(key) {
            InspectorOutcome::Unchanged => AcpxOutcome::Unchanged,
            InspectorOutcome::Changed => AcpxOutcome::Changed,
            InspectorOutcome::Back => {
                self.inspector = None;
                self.mode = Mode::Run;
                if let Some(rv) = self.run_view.as_mut() {
                    rv.select_step(idx);
                }
                AcpxOutcome::Changed
            }
            InspectorOutcome::Prev => {
                let target = idx.saturating_sub(1);
                if target != idx && self.open_step_at(target) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            InspectorOutcome::Next => {
                let total = self
                    .run_view
                    .as_ref()
                    .map_or(0, |rv| rv.run().steps.len());
                let max_idx = total.saturating_sub(1);
                let target = (idx + 1).min(max_idx);
                if target != idx && self.open_step_at(target) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            InspectorOutcome::Replay => self.replay_step(idx),
            InspectorOutcome::Copy(text) => AcpxOutcome::Copy(text),
        }
    }

    fn open_step_at(&mut self, idx: usize) -> bool {
        let Some(rv) = self.run_view.as_mut() else {
            return false;
        };
        let total = rv.run().steps.len();
        let Some(step) = rv.run().steps.get(idx).cloned() else {
            return false;
        };
        let node = rv
            .run()
            .flow
            .nodes
            .iter()
            .find(|n| n.id == step.node_id)
            .cloned();
        let run_dir = rv.run().summary.dir.clone();
        rv.select_step(idx);
        let inspector = InspectorState::new(&run_dir, step, node, (idx + 1, total));
        self.inspector = Some((idx, inspector));
        self.mode = Mode::Step;
        true
    }

    fn replay_step(&mut self, idx: usize) -> AcpxOutcome {
        let Some(rv) = self.run_view.as_ref() else {
            return AcpxOutcome::Unchanged;
        };
        let run = rv.run();
        let Some(step) = run.steps.get(idx) else {
            self.notice = Some("no event log for this step".to_string());
            return AcpxOutcome::Changed;
        };
        let Some(conv) = step.conversation.as_ref() else {
            self.notice = Some("no event log for this step".to_string());
            return AcpxOutcome::Changed;
        };
        let Some(events_path) = run.session_events_path(&conv.session_id) else {
            self.notice = Some("no event log for this step".to_string());
            return AcpxOutcome::Changed;
        };
        let title = format!("{} › {}", run.summary.title, step.node_id);
        AcpxOutcome::Replay(ReplayRequest {
            events_path: events_path.to_path_buf(),
            range: Some((conv.event_start_seq, conv.event_end_seq)),
            session_id: conv.session_id.clone(),
            title,
        })
    }

    fn title_text(&self) -> String {
        let raw = match self.mode {
            Mode::Ledger => "acpx".to_string(),
            Mode::Run => match self.run_view.as_ref() {
                Some(rv) => format!("acpx › {}", rv.run().summary.title),
                None => "acpx".to_string(),
            },
            Mode::Step => match (self.run_view.as_ref(), self.inspector.as_ref()) {
                (Some(rv), Some((idx, _))) => {
                    let run_title = &rv.run().summary.title;
                    let node_id = rv
                        .run()
                        .steps
                        .get(*idx)
                        .map_or("?", |s| s.node_id.as_str());
                    format!("acpx › {run_title} › {node_id}")
                }
                _ => "acpx".to_string(),
            },
        };
        truncate_str(&raw, MAX_TITLE_WIDTH)
    }

    fn active_shortcut_labels(&self) -> Vec<&'static str> {
        let mut labels = match self.mode {
            Mode::Ledger => self.ledger.shortcuts(),
            Mode::Run => self
                .run_view
                .as_ref()
                .map_or_else(Vec::new, |rv| rv.shortcuts()),
            Mode::Step => self
                .inspector
                .as_ref()
                .map_or_else(Vec::new, |(_, ins)| ins.shortcuts()),
        };
        labels.push("? keys");
        labels
    }
}

fn session_replay(s: SessionSummary) -> ReplayRequest {
    ReplayRequest {
        events_path: s.stream_path,
        range: None,
        session_id: s.acp_session_id,
        title: format!("acpx · {}", s.title),
    }
}

pub fn render_acpx_modal(
    buf: &mut Buffer,
    full_area: Rect,
    state: &mut AcpxModalState,
    compact: bool,
) {
    if full_area.width == 0 || full_area.height == 0 {
        return;
    }
    let theme = Theme::current();
    let sizing = ModalSizing {
        width_pct: 1.0,
        max_width: u16::MAX,
        min_width: 44,
        v_margin: 0,
        h_pad: 2,
        v_pad: 0,
        footer_lines: 2,
    }
    .with_compact(compact);

    let title = state.title_text();
    let labels = state.active_shortcut_labels();
    let shortcuts: Vec<Shortcut<'_>> = labels
        .iter()
        .map(|label| Shortcut {
            label,
            clickable: false,
            id: 0,
        })
        .collect();

    let config = ModalWindowConfig {
        title: &title,
        tabs: None,
        shortcuts: &shortcuts,
        sizing,
        fold_info: None,
    };

    let Some(ModalContentArea { content, .. }) =
        modal_window::render_modal_window(buf, full_area, &mut state.window, &config, &theme)
    else {
        return;
    };

    if content.height < 4 || content.width < 20 {
        return;
    }

    let now = now_ms();

    if state.help_open {
        render_help_overlay(buf, content, state.mode, &theme);
    } else {
        match state.mode {
            Mode::Ledger => state.ledger.render(buf, content, &theme, now),
            Mode::Run => {
                if let Some(rv) = state.run_view.as_mut() {
                    rv.render(buf, content, &theme, now);
                }
            }
            Mode::Step => {
                if let Some((_, ins)) = state.inspector.as_mut() {
                    ins.render(buf, content, &theme, now);
                }
            }
        }
    }

    if let Some(notice) = state.notice.as_deref() {
        render_notice_row(buf, content, notice, &theme);
    }
}

fn render_notice_row(buf: &mut Buffer, content: Rect, notice: &str, theme: &Theme) {
    if content.height == 0 || content.width == 0 {
        return;
    }
    let y = content.y + content.height - 1;
    let row_rect = Rect::new(content.x, y, content.width, 1);
    let warn_style = Style::default()
        .fg(theme.warning)
        .bg(theme.bg_base)
        .add_modifier(Modifier::BOLD);
    buf.set_style(row_rect, warn_style);
    let text = truncate_str(&format!("! {notice}"), content.width as usize);
    let line = Line::from(Span::styled(text, warn_style));
    buf.set_line(content.x, y, &line, content.width);
}

const LEDGER_KEYS: &[(&str, &str)] = &[
    ("Tab / 1 / 2", "Switch between Flows and Sessions"),
    ("j / k / ↑ / ↓", "Move selection"),
    ("g / G", "Jump to first / last row"),
    ("Enter", "Open selected flow run or replay session"),
    ("/ or f", "Filter rows by text"),
    ("s", "Cycle status filter"),
    ("y", "Copy selected id or path"),
    ("R", "Rescan flows and sessions on disk"),
    ("?", "Toggle this key reference"),
    ("Esc", "Clear filter or close modal"),
];

const RUN_KEYS: &[(&str, &str)] = &[
    ("j / k / ↑ / ↓", "Move step selection"),
    ("g / G", "Jump to first / last step"),
    ("Enter", "Inspect selected step"),
    ("Space / z", "Fold or unfold step details"),
    ("r", "Replay ACP session transcript for step"),
    ("y", "Copy attempt or run id"),
    ("?", "Toggle this key reference"),
    ("Esc / Backspace", "Back to ledger"),
];

const STEP_KEYS: &[(&str, &str)] = &[
    ("Tab / 1-5", "Switch inspector tab (Output / Prompt / Stdout / Stderr / Events)"),
    ("[ / ]", "Previous / next step in run"),
    ("j / k / ↑ / ↓", "Scroll active pane"),
    ("PgUp / PgDn", "Page up / down"),
    ("r", "Replay ACP conversation for this step"),
    ("y", "Copy current tab content"),
    ("?", "Toggle this key reference"),
    ("Esc / Backspace", "Back to run view"),
];

fn render_help_overlay(buf: &mut Buffer, area: Rect, mode: Mode, theme: &Theme) {
    let base_style = Style::default().fg(theme.text_primary).bg(theme.bg_base);
    buf.set_style(area, base_style);

    let (screen_label, table) = match mode {
        Mode::Ledger => ("Ledger", LEDGER_KEYS),
        Mode::Run => ("Run", RUN_KEYS),
        Mode::Step => ("Step Inspector", STEP_KEYS),
    };

    let width = area.width as usize;
    let header_style = Style::default()
        .fg(theme.accent_assistant)
        .bg(theme.bg_base)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme.gray).bg(theme.bg_base);
    let key_style = Style::default()
        .fg(theme.command)
        .bg(theme.bg_base)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme.text_primary).bg(theme.bg_base);

    let title = truncate_str(
        &format!("Keyboard Shortcuts · {screen_label} (press any key to close)"),
        width,
    );
    buf.set_line(
        area.x,
        area.y,
        &Line::from(Span::styled(title, header_style)),
        area.width,
    );

    let key_col_width = table
        .iter()
        .map(|(k, _)| UnicodeWidthStr::width(*k))
        .max()
        .unwrap_or(8)
        + 3;

    for (idx, (k, desc)) in table.iter().enumerate() {
        let row_offset = (idx as u16).saturating_add(2);
        if row_offset >= area.height {
            break;
        }
        let y = area.y + row_offset;
        let pad_len = key_col_width.saturating_sub(UnicodeWidthStr::width(*k));
        let padded_key = format!("  {k}{}", " ".repeat(pad_len));
        let key_w = UnicodeWidthStr::width(padded_key.as_str());
        let rem = width.saturating_sub(key_w);
        let cut_desc = truncate_str(desc, rem);
        let line = Line::from(vec![
            Span::styled(padded_key, key_style),
            Span::styled(cut_desc, desc_style),
        ]);
        buf.set_line(area.x, y, &line, area.width);
    }

    let footer_row = (table.len() as u16).saturating_add(3);
    if footer_row < area.height {
        let hint = truncate_str("  Press any key to dismiss help.", width);
        buf.set_line(
            area.x,
            area.y + footer_row,
            &Line::from(Span::styled(hint, dim_style)),
            area.width,
        );
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use tempfile::TempDir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    struct TestFixture {
        _tmp: TempDir,
        runs_base: PathBuf,
        sessions_base: PathBuf,
        run_dir: PathBuf,
        events_path: PathBuf,
        session_stream_path: PathBuf,
    }

    fn write_fixture(status: &str) -> TestFixture {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_base = tmp.path().join("runs");
        let sessions_base = tmp.path().join("sessions");
        let run_dir = runs_base.join("run-001");
        let proj_dir = run_dir.join("projections");
        fs::create_dir_all(&proj_dir).expect("create projections dir");
        fs::create_dir_all(&sessions_base).expect("create sessions dir");

        let events_path = run_dir.join("sess-1.events.ndjson");
        fs::write(&events_path, "{}\n").expect("write events");

        let live_json = format!(
            r#"{{
                "runId": "run-001",
                "flowName": "deploy-flow",
                "runTitle": "Deploy Production",
                "status": "{status}",
                "startedAt": "2025-03-01T10:00:00Z",
                "updatedAt": "2025-03-01T10:01:00Z",
                "currentNode": "verify"
            }}"#
        );
        fs::write(proj_dir.join("live.json"), live_json).expect("write live.json");

        let flow_json = r#"{
            "name": "deploy-flow",
            "startAt": "plan",
            "nodes": {
                "plan": { "nodeType": "acp", "summary": "Draft deployment plan" },
                "verify": { "nodeType": "action", "summary": "Run verification suite", "actionExecution": "shell" }
            },
            "edges": [
                { "from": "plan", "to": "verify" }
            ]
        }"#;
        fs::write(run_dir.join("flow.json"), flow_json).expect("write flow.json");

        let steps_json = r#"[
            {
                "attemptId": "plan#1",
                "nodeId": "plan",
                "nodeType": "acp",
                "outcome": "ok",
                "startedAt": "2025-03-01T10:00:00Z",
                "finishedAt": "2025-03-01T10:00:30Z",
                "promptText": "Create plan",
                "output": "Plan ready",
                "agent": { "agentName": "claude" },
                "trace": {
                    "conversation": {
                        "sessionId": "sess-1",
                        "eventStartSeq": 1,
                        "eventEndSeq": 12
                    }
                }
            },
            {
                "attemptId": "verify#1",
                "nodeId": "verify",
                "nodeType": "action",
                "outcome": "ok",
                "startedAt": "2025-03-01T10:00:31Z",
                "finishedAt": "2025-03-01T10:00:55Z",
                "output": { "passed": true },
                "trace": {
                    "action": {
                        "actionType": "shell",
                        "command": "cargo",
                        "args": ["test"],
                        "exitCode": 0,
                        "durationMs": 2400
                    }
                }
            }
        ]"#;
        fs::write(proj_dir.join("steps.json"), steps_json).expect("write steps.json");

        let trace_ndjson = r#"{"seq":1,"at":"2025-03-01T10:00:00Z","scope":"node","type":"node_started","nodeId":"plan","attemptId":"plan#1"}
{"seq":2,"at":"2025-03-01T10:00:31Z","scope":"node","type":"node_started","nodeId":"verify","attemptId":"verify#1"}
"#;
        fs::write(run_dir.join("trace.ndjson"), trace_ndjson).expect("write trace.ndjson");

        let manifest_json = format!(
            r#"{{
                "runId": "run-001",
                "flowName": "deploy-flow",
                "startedAt": "2025-03-01T10:00:00Z",
                "sessions": [
                    {{
                        "id": "sess-1",
                        "handle": "main",
                        "eventsPath": "{}",
                        "agentName": "claude"
                    }}
                ]
            }}"#,
            events_path.display()
        );
        fs::write(run_dir.join("manifest.json"), manifest_json).expect("write manifest.json");

        let session_stream_path = sessions_base.join("rec-100.stream.ndjson");
        fs::write(&session_stream_path, "{}\n").expect("write session stream");
        let session_json = r#"{
            "acpx_record_id": "rec-100",
            "acp_session_id": "acp-sess-100",
            "name": "triage",
            "title": "Bug Triage Session",
            "agent_command": "claude-code",
            "cwd": "/tmp/project",
            "created_at": "2025-03-01T09:00:00Z",
            "last_used_at": "2025-03-01T09:30:00Z",
            "closed": false,
            "messages": [
                { "role": "user" },
                { "role": "assistant" }
            ],
            "cumulative_token_usage": { "total_tokens": 1500 }
        }"#;
        fs::write(sessions_base.join("rec-100.json"), session_json).expect("write session record");

        TestFixture {
            _tmp: tmp,
            runs_base,
            sessions_base,
            run_dir,
            events_path,
            session_stream_path,
        }
    }

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn open_at_lists_runs_and_sessions_and_navigates_modes() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert_eq!(state.mode(), Mode::Ledger);
        assert_eq!(state.runs_snapshot.len(), 1);

        // Enter opens the run -> Mode::Run
        let out = state.handle_key(&key(KeyCode::Enter));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Run);

        // Enter again opens the selected step -> Mode::Step
        let out = state.handle_key(&key(KeyCode::Enter));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Step);

        // Esc returns to Mode::Run
        let out = state.handle_key(&key(KeyCode::Esc));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Run);

        // Esc again returns to Mode::Ledger
        let out = state.handle_key(&key(KeyCode::Esc));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Ledger);

        // Esc in Ledger with empty filter returns Close
        let out = state.handle_key(&key(KeyCode::Esc));
        assert_eq!(out, AcpxOutcome::Close);
    }

    #[test]
    fn wants_esc_reflects_mode_and_help_state() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.wants_esc());

        // Help overlay makes wants_esc true even in Ledger
        state.handle_key(&key(KeyCode::Char('?')));
        assert!(state.wants_esc());
        state.handle_key(&key(KeyCode::Char('a')));
        assert!(!state.wants_esc());

        // Mode::Run wants_esc
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);
        assert!(state.wants_esc());

        // Mode::Step wants_esc
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Step);
        assert!(state.wants_esc());
    }

    #[test]
    fn replay_on_acp_step_returns_expected_request() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);

        // Select step 0 ("plan", which has conversation sess-1)
        if let Some(rv) = state.run_view.as_mut() {
            rv.select_step(0);
        }

        let out = state.handle_key(&key(KeyCode::Char('r')));
        assert_eq!(
            out,
            AcpxOutcome::Replay(ReplayRequest {
                events_path: fx.events_path,
                range: Some((1, 12)),
                session_id: "sess-1".to_string(),
                title: "Deploy Production › plan".to_string(),
            })
        );
    }

    #[test]
    fn opening_session_in_ledger_returns_replay_with_none_range() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        // Switch to Sessions tab in Ledger
        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(state.ledger.tab(), self::ledger::LedgerTab::Sessions);

        let out = state.handle_key(&key(KeyCode::Enter));
        assert_eq!(
            out,
            AcpxOutcome::Replay(ReplayRequest {
                events_path: fx.session_stream_path,
                range: None,
                session_id: "acp-sess-100".to_string(),
                title: "acpx · Bug Triage Session".to_string(),
            })
        );
    }

    #[test]
    fn step_next_bracket_advances_and_esc_preserves_run_selection() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);

        // Start at step 0 and open it
        if let Some(rv) = state.run_view.as_mut() {
            rv.select_step(0);
        }
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Step);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(0));

        // Press `]` to move to step 1
        let out = state.handle_key(&key(KeyCode::Char(']')));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(1));

        // Esc back to Run view lands on step 1
        state.handle_key(&key(KeyCode::Esc));
        assert_eq!(state.mode(), Mode::Run);
        assert_eq!(
            state.run_view.as_ref().and_then(|rv| rv.selected_step()),
            Some(1)
        );
    }

    #[test]
    fn open_run_missing_dir_sets_notice_and_next_key_clears_it() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        let missing = fx.runs_base.join("does-not-exist");
        assert!(!state.open_run(&missing));
        assert!(
            state
                .notice()
                .is_some_and(|n| n.starts_with("cannot open run:"))
        );

        // Next key press clears the notice
        let out = state.handle_key(&key(KeyCode::Down));
        assert_eq!(out, AcpxOutcome::Changed);
        assert!(state.notice().is_none());
    }

    #[test]
    fn poll_detects_live_json_status_change_on_disk() {
        let fx = write_fixture("running");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(state.wants_poll());

        // Open the running run
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);
        assert!(state.wants_poll());
        assert!(!state.poll());

        // Update live.json on disk to completed
        let updated_live = r#"{
            "runId": "run-001",
            "flowName": "deploy-flow",
            "runTitle": "Deploy Production",
            "status": "completed",
            "startedAt": "2025-03-01T10:00:00Z",
            "updatedAt": "2025-03-01T10:02:00Z",
            "finishedAt": "2025-03-01T10:02:00Z"
        }"#;
        fs::write(
            fx.run_dir.join("projections").join("live.json"),
            updated_live,
        )
        .expect("rewrite live.json");

        assert!(state.poll());
        assert!(!state.wants_poll());
    }

    #[test]
    fn wants_poll_is_false_for_waiting_run() {
        let fx = write_fixture("waiting");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.wants_poll());

        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);
        assert!(!state.wants_poll());
    }

    #[test]
    fn question_mark_toggles_help_overlay_and_any_key_closes_it() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.help_open);

        let out = state.handle_key(&key(KeyCode::Char('?')));
        assert_eq!(out, AcpxOutcome::Changed);
        assert!(state.help_open);

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 30));
        render_acpx_modal(&mut buf, buf.area, &mut state, false);
        let text = buffer_text(&buf);
        assert!(text.contains("Keyboard Shortcuts"));

        // Any key closes the help overlay without triggering underlying action
        let out = state.handle_key(&key(KeyCode::Enter));
        assert_eq!(out, AcpxOutcome::Changed);
        assert!(!state.help_open);
        assert_eq!(state.mode(), Mode::Ledger);
    }

    #[test]
    fn render_at_120x30_draws_title_and_tiny_sizes_do_not_panic() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 30));
        render_acpx_modal(&mut buf, buf.area, &mut state, false);
        let rendered = buffer_text(&buf);
        assert!(rendered.contains("acpx"));

        // 10x3 and 0x0 must not panic
        let mut tiny = Buffer::empty(Rect::new(0, 0, 10, 3));
        render_acpx_modal(&mut tiny, tiny.area, &mut state, true);

        let mut zero = Buffer::empty(Rect::new(0, 0, 0, 0));
        render_acpx_modal(&mut zero, zero.area, &mut state, false);
    }
}
```