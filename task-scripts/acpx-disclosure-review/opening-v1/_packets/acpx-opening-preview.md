Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, or release behavior. Change ledger routing only
where the task explicitly owns a reviewed presentation or focus change.

=== REPOSITORY CONTEXT (21 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1-310 of 1621 (repository grok-build)
````
pub(crate) mod detail;
pub(crate) mod inspector;
pub(crate) mod ledger;
pub(crate) mod modal_view;
pub(crate) mod run;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{self, Run, RunSummary};
use crate::acpx::sessions::{self, SessionSummary};
use crate::piw::gate::{self as piw_gate, GateAnswer as PiwGateAnswer, GateAnswerer};
use crate::piw::mirror;
use crate::piw::runtime;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{
    self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowOutcome, ModalWindowState,
    Shortcut,
};

use self::detail::{DetailPanel, DetailTarget};
use self::inspector::{InspectorOutcome, InspectorState};
use self::ledger::{GateAnswer, LedgerOutcome, LedgerState, RowExtra};
use self::run::{RunOutcome, RunViewState};

const MAX_TITLE_WIDTH: usize = 72;
/// Below this content width the runs list has no room for a preview beside it.
const PREVIEW_MIN_WIDTH: u16 = 120;
/// Rails and agents are loaded for this many of the newest runs.
const EXTRAS_LIMIT: usize = 80;

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
    /// A gate was answered: start this flow with this input.
    RunFlow { flow_file: PathBuf, input_json: String },
}

struct ModalNotice {
    text: String,
    warning: bool,
}

impl ModalNotice {
    fn warning(text: String) -> Self {
        Self { text, warning: true }
    }
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
    notice: Option<ModalNotice>,
    /// The right-hand panel: the selected run beside the list, the selected stage beside a run.
    detail: DetailPanel,
    /// The run selected in the list, loaded for the preview.
    preview: Option<Run>,
    /// Rail and agents per run id, with the `updated_ms` they were read at.
    extras: HashMap<String, (i64, RowExtra)>,
    /// Gate answers per run id for runs beyond `EXTRAS_LIMIT`.
    answers: HashMap<String, Option<GateAnswer>>,
    /// Answers being sent to pi-workflows gates, polled on the one-second tick.
    answerers: Vec<GateAnswerer>,
    /// The mirrored run whose folder the run view is open on, so closing the
    /// view can release it.
    open_mirror_run: Option<String>,
    /// Where the detail panel was last drawn, for the mouse wheel.
    detail_rect: Option<Rect>,
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
            detail: DetailPanel::new(),
            preview: None,
            extras: HashMap::new(),
            answers: HashMap::new(),
            answerers: Vec::new(),
            open_mirror_run: None,
            detail_rect: None,
        };
        state.rescan_runs();
        state.rescan_sessions();
        state
    }

    pub fn open_run(&mut self, dir: &Path) -> bool {
        self.release_open_mirror_run();
        match bundle::load_run(dir) {
            Ok(run) => {
                let run_id = run.summary.run_id.clone();
                self.watch_open_mirror_run(dir, &run_id);
                let mut view = RunViewState::new(run);
                view.set_answered(self.ledger.answered_choice(&run_id).map(str::to_string));
                view.set_settings(bundle::node_settings(dir), bundle::step_settings(dir));
                self.run_view = Some(view);
                self.inspector = None;
                self.mode = Mode::Run;
                self.help_open = false;
                self.notice = None;
                true
            }
            Err(err) => {
                self.set_notice(format!("cannot open run: {err}"));
                false
            }
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn wants_poll(&self) -> bool {
        match self.mode {
            // Runs are started outside the pager, so the list is always re-read.
            Mode::Ledger => true,
            // A gate answer in flight must keep ticking even when the parked
            // run itself shows nothing new.
            Mode::Run | Mode::Step => {
                !self.answerers.is_empty()
                    || self
                        .run_view
                        .as_ref()
                        .is_some_and(|rv| rv.run().summary.status.is_live())
            }
        }
    }

    pub fn poll(&mut self) -> bool {
        let answerers_changed = self.poll_answerers();
        match self.mode {
            Mode::Ledger => self.rescan_runs() | answerers_changed,
            Mode::Run | Mode::Step => {
                let Some(rv) = self.run_view.as_mut() else {
                    return answerers_changed;
                };
                if bundle::refresh(rv.run_mut()) {
                    let dir = rv.run().summary.dir.clone();
                    rv.set_settings(bundle::node_settings(&dir), bundle::step_settings(&dir));
                    rv.sync();
                    true
                } else {
                    answerers_changed
                }
            }
        }
    }

    pub fn wants_esc(&self) -> bool {
        self.help_open || self.mode != Mode::Ledger || !self.ledger.esc_closes()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|notice| notice.text.as_str())
    }

    /// One line shown at the bottom until the next key.
    pub fn set_notice(&mut self, text: String) {
        self.notice = Some(ModalNotice::warning(text));
    }

    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
        let confirmed = matches!(delivery, crate::clipboard::CopyDelivery::Clipboard { result, .. }
            if result.delivery.is_confirmed());
        self.notice = Some(ModalNotice {
            text: delivery.toast_message().into_owned(),
            warning: !confirmed,
        });
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let had_notice = self.notice.take().is_some();

        if self.help_open {
            self.help_open = false;
            return AcpxOutcome::Changed;
        }

        let captures = match self.mode {
            Mode::Ledger => self.ledger.captures_text(),
            Mode::Run => self.run_view.as_ref().is_some_and(|rv| rv.captures_keys()),
            Mode::Step => false,
        };
        if !captures && matches!(key.code, KeyCode::Char('?')) {
            self.help_open = true;
            return AcpxOutcome::Changed;
        }
        // Beside the list, J / K scroll the preview (the list itself owns j / k).
        if !captures && self.mode == Mode::Ledger {
            let delta = match key.code {
                KeyCode::Char('J') => 3,
                KeyCode::Char('K') => -3,
                _ => 0,
            };
            if delta != 0 {
                return if self.detail.scroll(delta) { AcpxOutcome::Changed } else { AcpxOutcome::Unchanged };
            }
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
                let over_detail = self.detail_rect.is_some_and(|r| {
                    mouse.column >= r.x && mouse.column < r.x + r.width && mouse.row >= r.y && mouse.row < r.y + r.height
                });
                if over_detail && self.mode != Mode::Step {
                    return if self.detail.scroll(delta) { AcpxOutcome::Changed } else { AcpxOutcome::Unchanged };
                }
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 354-396 of 1621 (repository grok-build)
````
    fn sync_preview(&mut self) {
        let Some(selected) = self.ledger.selected_run() else {
            self.preview = None;
            return;
        };
        let fresh = self
            .preview
            .as_ref()
            .is_some_and(|p| p.summary.dir == selected.dir && p.summary.updated_ms == selected.updated_ms);
        if !fresh {
            self.preview = bundle::load_run(&selected.dir).ok();
        }
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

    /// A mirrored run was opened: keep mirroring it in full.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 504-558 of 1621 (repository grok-build)
````
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
                self.release_open_mirror_run();
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
            RunOutcome::ScrollDetail(delta) => {
                if self.detail.scroll(delta) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Gate { choice, flow_file } => {
                // A pi-workflows gate answers in place instead of starting a
                // flow; the target is checked before any relative-path join.
                if let Some((run_id, choice_key)) = piw_gate::parse_answer_target(&flow_file) {
                    return self.answer_mirror_gate(&run_id, &choice, &choice_key);
                }
                let run = rv.run();
                // A relative flow file is beside the flow that declared it.
                let declared = PathBuf::from(&flow_file);
                let flow_file = match bundle::flow_path(&run.summary.dir).as_deref().and_then(Path::parent) {
                    Some(dir) if declared.is_relative() => dir.join(declared),
                    _ => declared,
                };
                let input_json = bundle::gate_input(run, &choice).to_string();
                // Back to the list, where the new run appears.
                self.set_notice(format!("{choice}: started {}", flow_file.display()));
                self.mode = Mode::Ledger;
                self.run_view = None;
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 651-694 of 1621 (repository grok-build)
````
    fn title_text(&self) -> String {
        // The pop-up lists pi-workflows runs too, not only acpx flows, so it is titled "runs".
        let raw = match self.mode {
            Mode::Ledger => "runs".to_string(),
            Mode::Run => match self.run_view.as_ref() {
                Some(rv) => format!("runs › {}", rv.run().summary.title),
                None => "runs".to_string(),
            },
            Mode::Step => match (self.run_view.as_ref(), self.inspector.as_ref()) {
                (Some(rv), Some((idx, _))) => {
                    let run_title = &rv.run().summary.title;
                    let node_id = rv
                        .run()
                        .steps
                        .get(*idx)
                        .map_or("?", |s| s.node_id.as_str());
                    format!("runs › {run_title} › {node_id}")
                }
                _ => "runs".to_string(),
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
        if !labels.contains(&"? keys") {
            labels.push("? keys");
        }
        labels
    }
}

/// What the list shows for one run beyond its summary. Reads the run folder.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 742-921 of 1621 (repository grok-build)
````
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

    // Nothing of the screen behind may show around the frame.
    let blank = Style::default().fg(theme.text_primary).bg(theme.bg_base);
    for y in full_area.y..full_area.y + full_area.height {
        buf.set_stringn(full_area.x, y, " ".repeat(full_area.width as usize), full_area.width as usize, blank);
    }
    let Some(ModalContentArea { content, .. }) =
        modal_window::render_modal_window(buf, full_area, &mut state.window, &config, &theme)
    else {
        return;
    };

    if content.height < 4 || content.width < 20 {
        return;
    }

    let now = now_ms();

    state.detail_rect = None;
    if state.help_open {
        render_help_overlay(buf, content, state.mode, &theme);
    } else {
        match state.mode {
            Mode::Ledger => {
                state.sync_preview();
                match (state.preview.as_ref(), content.width >= PREVIEW_MIN_WIDTH) {
                    (Some(run), true) => {
                        // List on the left, the selected run on the right, a rule between.
                        let list_w = (content.width * 9 / 20).clamp(56, 110);
                        let list = Rect::new(content.x, content.y, list_w, content.height);
                        let right = Rect::new(content.x + list_w + 2, content.y, content.width - list_w - 2, content.height);
                        state.ledger.render(buf, list, &theme, now);
                        let rule = Style::default().fg(theme.gray_dim);
                        for y in content.y..content.y + content.height {
                            buf.set_stringn(content.x + list_w, y, "│", 1, rule);
                        }
                        state
                            .detail
                            .set_answered(state.ledger.answered_choice(&run.summary.run_id));
                        state.detail.render(buf, right, run, &DetailTarget::Run, &theme, now);
                        state.detail_rect = Some(right);
                    }
                    _ => state.ledger.render(buf, content, &theme, now),
                }
            }
            Mode::Run => {
                if let Some(rv) = state.run_view.as_mut() {
                    rv.render(buf, content, &theme, now);
                    if let Some(area) = rv.detail_area() {
                        let area = area.intersection(content);
                        state.detail.set_answered(rv.answered());
                        state.detail.render(buf, area, rv.run(), &rv.detail_target(), &theme, now);
                        state.detail_rect = Some(area);
                    }
                }
            }
            Mode::Step => {
                if let Some((_, ins)) = state.inspector.as_mut() {
                    ins.render(buf, content, &theme, now);
                }
            }
        }
    }

    if let Some(notice) = state.notice.as_ref() {
        render_notice_row(buf, content, notice, &theme);
    }
}

fn render_notice_row(buf: &mut Buffer, content: Rect, notice: &ModalNotice, theme: &Theme) {
    if content.height == 0 || content.width == 0 {
        return;
    }
    let y = content.y + content.height - 1;
    let row_rect = Rect::new(content.x, y, content.width, 1);
    let (style, text) = if notice.warning {
        (
            Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
            format!("! {}", notice.text),
        )
    } else {
        (Style::default().fg(theme.text_primary), notice.text.clone())
    };
    let style = style.bg(theme.bg_base);
    buf.set_style(row_rect, style);
    let text = truncate_str(&text, content.width as usize);
    let line = Line::from(Span::styled(text, style));
    buf.set_line(content.x, y, &line, content.width);
}

const LEDGER_KEYS: &[(&str, &str)] = &[
    ("Tab / h / l", "Switch between Flows and Agent Turns"),
    ("j / k / ↑ / ↓", "Move selection"),
    ("g / G", "Jump to first / last row"),
    ("J / K", "Scroll the preview of the selected run"),
    ("Enter", "Open selected flow run or replay agent turn"),
    ("/", "Filter rows by text"),
    ("a", "Only running and waiting runs"),
    ("s", "Cycle order: grouped, recent, name"),
    ("y", "Copy selected id or path"),
    ("R", "Rescan flows and agent turns on disk"),
    ("?", "Toggle this key reference"),
    ("Esc", "Clear filter or close modal"),
];

const RUN_KEYS: &[(&str, &str)] = &[
    ("j / k / ↑ / ↓", "Move through stages and their substeps"),
    ("[ / ]", "Previous / next stage"),
    ("J / K, D / U", "Scroll the detail panel by a few rows / by a page"),
    ("Enter", "Inspect the selected stage; on a gate row, answer it"),
    ("1-9", "Answer the gate with that choice (asks first)"),
    ("F5", "Jump to the gate"),
    ("Space, h / l, E", "Fold or unfold a stage; E folds all"),
    ("c / !", "Jump to the current stage / the next failed one"),
    ("v / n", "Show or hide the graph / heartbeat noise"),
    ("r", "Replay the stage's conversation as a transcript"),
    ("y", "Copy the stage's output"),
    ("?", "Toggle this key reference"),
    ("Esc", "Back to the list"),
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 998-1158 of 1621 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1187-1243 of 1621 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1269-1290 of 1621 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1409-1431 of 1621 (repository grok-build)
````
    fn render_at_120x30_draws_title_and_tiny_sizes_do_not_panic() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 30));
        let area = buf.area;
        render_acpx_modal(&mut buf, area, &mut state, false);
        let rendered = buffer_text(&buf);
        // The pop-up lists pi-workflows runs too, so it is titled "runs".
        assert!(rendered.contains("runs"));

        // 10x3 and 0x0 must not panic
        let mut tiny = Buffer::empty(Rect::new(0, 0, 10, 3));
        let area = tiny.area;
        render_acpx_modal(&mut tiny, area, &mut state, true);

        let mut zero = Buffer::empty(Rect::new(0, 0, 0, 0));
        let area = zero.area;
        render_acpx_modal(&mut zero, area, &mut state, false);
    }

    #[test]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 18-56 of 2990 (repository grok-build)
````
pub enum LedgerTab {
    Flows,
    Sessions,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LedgerOutcome {
    Unchanged,
    Changed,
    OpenRun(PathBuf),
    OpenSession(SessionSummary),
    Copy(String),
    Rescan,
    Close,
}

/// The gate this run answered: read from this run's own input, written by `bundle::gate_input` when a gate answer
/// starts the next flow file. `projections/run.json` -> `input.gate = { "runId": <the waiting run>, "node": .., "choice": .. }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateAnswer {
    pub run_id: String,   // input.gate.runId: the run that was waiting at the gate
    pub choice: String,   // input.gate.choice: the label that was picked
}

/// How many step marks a runs-list row shows at most. A workflow with
/// hundreds of nodes would otherwise stretch the marks column past every
/// other column; the run view keeps the full picture.
const RAIL_MARKS_CAP: usize = 24;

/// Per-run facts the parent loads (cached) so the ledger does no I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
    pub answers: Option<GateAnswer>,
}

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 201-309 of 2990 (repository grok-build)
````
    pub fn set_runs(&mut self, runs: Vec<RunSummary>) {
        self.runs = runs;
        self.rebuild_answered();
        self.reconcile_flow_selection();
    }

    pub fn set_extras(&mut self, extras: HashMap<String, RowExtra>) {
        self.extras = extras;
        self.rebuild_answered();
        self.reconcile_flow_selection();
    }

    pub fn answered_choice(&self, run_id: &str) -> Option<&str> {
        if self
            .runs
            .iter()
            .find(|r| r.run_id == run_id)
            .is_some_and(|r| r.status != RunStatus::Waiting)
        {
            return None;
        }
        self.answered.get(run_id).map(String::as_str)
    }

    fn rebuild_answered(&mut self) {
        let mut winners: HashMap<String, (&str, Option<i64>, &str)> = HashMap::new();
        let mut keys: Vec<&String> = self.extras.keys().collect();
        keys.sort();
        for answering_id in keys {
            if let Some(extra) = self.extras.get(answering_id)
                && let Some(ref ans) = extra.answers
            {
                let started = self
                    .runs
                    .iter()
                    .find(|r| &r.run_id == answering_id)
                    .map(|r| r.started_ms);
                let replace = match winners.get(&ans.run_id) {
                    None => true,
                    Some(&(prev_id, prev_started, _)) => match (prev_started, started) {
                        (Some(ps), Some(ns)) => (ns, answering_id.as_str()) > (ps, prev_id),
                        _ => answering_id.as_str() > prev_id,
                    },
                };
                if replace {
                    winners.insert(
                        ans.run_id.clone(),
                        (answering_id.as_str(), started, ans.choice.as_str()),
                    );
                }
            }
        }
        self.answered = winners
            .into_iter()
            .map(|(k, (_, _, choice))| (k, choice.to_string()))
            .collect();
    }

    pub fn selected_run(&self) -> Option<&RunSummary> {
        if self.tab != LedgerTab::Flows {
            return None;
        }
        let id = self.flow_selected_run_id.as_deref()?;
        let indices = self.filtered_flow_indices();
        indices
            .into_iter()
            .find_map(|idx| self.runs.get(idx).filter(|r| r.run_id == id))
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.sessions = sessions;
        self.reconcile_session_selection();
    }

    pub fn tab(&self) -> LedgerTab {
        self.tab
    }

    pub fn any_live(&self) -> bool {
        self.runs
            .iter()
            .any(|r| r.status.is_live() && !self.is_stalled(r))
    }

    pub fn captures_text(&self) -> bool {
        self.filter_open
    }

    pub fn esc_closes(&self) -> bool {
        !self.filter_open && self.active_filter().is_empty()
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        if self.filter_open {
            vec!["Enter apply", "Esc clear"]
        } else {
            vec![
                "Enter open",
                "Tab switch",
                "/ filter",
                "a active",
                "s sort",
                "y copy",
                "R rescan",
                "Esc close",
            ]
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 727-891 of 2990 (repository grok-build)
````
    pub fn handle_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        if key.kind == KeyEventKind::Release {
            return LedgerOutcome::Unchanged;
        }

        if self.filter_open {
            return self.handle_filter_key(key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half_page = (self.last_render_height.saturating_sub(1) / 2).max(1) as isize;

        match key.code {
            KeyCode::Char('u') if ctrl => {
                if self.move_selection(-half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                if self.move_selection(half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.move_selection(1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.move_selection(-1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                if self.move_selection(half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                if self.move_selection(-half_page) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.jump_start() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.jump_end() {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.tab = match self.tab {
                    LedgerTab::Flows => LedgerTab::Sessions,
                    LedgerTab::Sessions => LedgerTab::Flows,
                };
                match self.tab {
                    LedgerTab::Flows => self.reconcile_flow_selection(),
                    LedgerTab::Sessions => self.reconcile_session_selection(),
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('/') => {
                self.filter_open = true;
                LedgerOutcome::Changed
            }
            KeyCode::Char('a') => {
                match self.tab {
                    LedgerTab::Flows => {
                        self.flow_active_only = !self.flow_active_only;
                        self.reconcile_flow_selection();
                    }
                    LedgerTab::Sessions => {
                        self.session_active_only = !self.session_active_only;
                        self.reconcile_session_selection();
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('s') => {
                match self.tab {
                    LedgerTab::Flows => {
                        self.flow_order = self.flow_order.next();
                        self.reconcile_flow_selection();
                    }
                    LedgerTab::Sessions => {
                        self.session_order = self.session_order.next();
                        self.reconcile_session_selection();
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Enter => match self.tab {
                LedgerTab::Flows => {
                    if let Some(run) = self.selected_run() {
                        LedgerOutcome::OpenRun(run.dir.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
                LedgerTab::Sessions => {
                    if let Some(sess) = self.selected_session() {
                        LedgerOutcome::OpenSession(sess.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('y') => match self.tab {
                LedgerTab::Flows => {
                    if let Some(run) = self.selected_run() {
                        LedgerOutcome::Copy(run.dir.to_string_lossy().into_owned())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
                LedgerTab::Sessions => {
                    if let Some(sess) = self.selected_session() {
                        LedgerOutcome::Copy(sess.record_id.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('R') => LedgerOutcome::Rescan,
            KeyCode::Esc => {
                if !self.active_filter().is_empty() {
                    self.active_filter_mut().clear();
                    match self.tab {
                        LedgerTab::Flows => self.reconcile_flow_selection(),
                        LedgerTab::Sessions => self.reconcile_session_selection(),
                    }
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Close
                }
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn handle_filter_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 943-980 of 2990 (repository grok-build)
````
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.last_render_height = area.height as usize;
        self.last_render_width = area.width as usize;

        // Row 0: Tab header
        let list_area = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        };

        let overflow_counter = self.compute_overflow_counter(list_area.height as usize, now_ms);
        self.render_tab_row(
            buf,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            theme,
            overflow_counter.as_deref(),
        );

        if list_area.height == 0 {
            return;
        }

        match self.tab {
            LedgerTab::Flows => self.render_flows(buf, list_area, theme, now_ms),
            LedgerTab::Sessions => self.render_sessions(buf, list_area, theme, now_ms),
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 1036-1101 of 2698 (repository grok-build)
````
fn append_gate_section(
    out: &mut Vec<Line<'static>>,
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) {
    let choices = bundle::gate_choices(run);
    if choices.is_empty() && answered.is_some() {
        return;
    }
    push_section_rule(out, "Gate", width, theme);
    if choices.is_empty() {
        out.push(dim_line(
            "no next step declared: re-run the flow to continue",
            theme,
        ));
        return;
    }
    for (idx, (label, flow_file)) in choices.iter().enumerate() {
        let spans = match answered {
            // Answered: nothing to pick any more, so no key numbers; the picked one is ticked.
            Some(picked) => {
                let chosen = picked.eq_ignore_ascii_case(label);
                let (mark, mark_color, label_style) = if chosen {
                    (
                        "✓ ",
                        theme.accent_success,
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", theme.gray_dim, Style::default().fg(theme.gray))
                };
                vec![
                    Span::styled(mark, Style::default().fg(mark_color)),
                    Span::styled(format!("{label} "), label_style),
                    Span::styled("→ ", Style::default().fg(theme.gray_dim)),
                    Span::styled(flow_file.clone(), Style::default().fg(theme.gray)),
                ]
            }
            None => {
                let n = idx + 1;
                vec![
                    Span::styled(
                        format!("[{n}] "),
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{label} "),
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("→ ", Style::default().fg(theme.gray_dim)),
                    Span::styled(flow_file.clone(), Style::default().fg(theme.path)),
                ]
            }
        };
        out.push(truncate_line(Line::from(spans), width));
    }
}

````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 1-135 of 1936 (repository grok-build)
````
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use chrono::DateTime;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus {
    Running,
    Waiting,
    Completed,
    Failed,
    TimedOut,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeKind {
    Acp,
    Action,
    Compute,
    Checkpoint,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    TimedOut,
    Failed,
    Cancelled,
    Unknown,
}

/// What a node looks like right now, derived from steps + live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    NotRun,
    Running,
    Waiting,
    Done(Outcome),
}

impl RunStatus {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Unknown => "unknown",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Waiting => "◐",
            Self::Completed => "✓",
            Self::Failed => "✗",
            Self::TimedOut => "✗",
            Self::Unknown => "○",
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Running)
    }
}

impl NodeKind {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "acp" => Self::Acp,
            "action" => Self::Action,
            "compute" => Self::Compute,
            "checkpoint" => Self::Checkpoint,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Action => "action",
            Self::Compute => "compute",
            Self::Checkpoint => "checkpoint",
            Self::Unknown => "?",
        }
    }
}

impl Outcome {
    pub fn from_str_tolerant(s: &str) -> Self {
        match s {
            "ok" => Self::Ok,
            "timed_out" => Self::TimedOut,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }
}

impl NodeState {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::NotRun => "○",
            Self::Running => "●",
            Self::Waiting => "◐",
            Self::Done(Outcome::Ok) => "✓",
            Self::Done(Outcome::Failed) => "✗",
            Self::Done(Outcome::TimedOut) => "✗",
            Self::Done(Outcome::Cancelled) => "⊘",
            Self::Done(Outcome::Unknown) => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 150-187 of 1936 (repository grok-build)
````
pub struct ActionReceipt {
    pub action_type: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}

/// One attempt of one node, from projections/steps.json, in execution order.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub attempt_id: String,
    pub node_id: String,
    pub kind: NodeKind,
    pub outcome: Outcome,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub prompt_text: Option<String>,
    pub raw_text: Option<String>,
    pub output: Value,
    pub error: Option<String>,
    pub agent_name: Option<String>,
    pub action: Option<ActionReceipt>,
    pub conversation: Option<Conversation>,
    pub stdout: Option<ArtifactRef>,
    pub stderr: Option<ArtifactRef>,
    pub output_artifact: Option<ArtifactRef>,
}

impl Step {
    pub fn duration_ms(&self) -> u64 {
        let diff = self.finished_ms.saturating_sub(self.started_ms);
        if diff <= 0 { 0 } else { diff as u64 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 237-361 of 1936 (repository grok-build)
````
pub struct FlowDef {
    pub name: String,
    pub start_at: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    /// Verified root `maxSteps` (configured or engine default 100). `None` when unverified.
    pub max_steps: Option<usize>,
    /// Snapshot `composition.mounts`.
    pub mounts: Vec<FlowMount>,
}

/// One line of trace.ndjson.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    pub seq: u64,
    pub at_ms: i64,
    pub scope: String,
    pub kind: String,
    pub node_id: Option<String>,
    pub attempt_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRef {
    pub id: String,
    pub handle: String,
    pub events_path: PathBuf,
    pub agent_name: Option<String>,
}

/// One row of the runs ledger; cheap to load (projections/live.json, falling back to manifest.json).
#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub dir: PathBuf,
    pub flow_name: String,
    pub title: String,
    pub status: RunStatus,
    pub started_ms: i64,
    pub updated_ms: i64,
    pub finished_ms: Option<i64>,
    pub current_node: Option<String>,
    pub waiting_on: Option<String>,
    pub status_detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub summary: RunSummary,
    pub flow: FlowDef,
    pub steps: Vec<Step>,
    pub events: Vec<TraceEvent>,
    pub input: Value,
    pub sessions: Vec<SessionRef>,
}

impl Run {
    pub fn node_state(&self, node_id: &str) -> NodeState {
        if self.summary.waiting_on.as_deref() == Some(node_id) {
            return NodeState::Waiting;
        }
        if self.summary.status == RunStatus::Running
            && self.summary.current_node.as_deref() == Some(node_id)
        {
            return NodeState::Running;
        }
        if let Some(step) = self.steps.iter().rev().find(|s| s.node_id == node_id) {
            return NodeState::Done(step.outcome);
        }
        NodeState::NotRun
    }

    /// `node_state`, except that a gate which has been answered is done: the
    /// checkpoint is no longer waiting on anyone.
    pub fn node_state_answered(&self, node_id: &str, answered: bool) -> NodeState {
        match self.node_state(node_id) {
            NodeState::Waiting if answered => NodeState::Done(Outcome::Ok),
            state => state,
        }
    }

    pub fn taken_path(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.node_id.clone()).collect()
    }

    pub fn edge_taken(&self, from: &str, to: &str) -> bool {
        self.steps
            .windows(2)
            .any(|pair| pair[0].node_id == from && pair[1].node_id == to)
    }

    pub fn step_events(&self, attempt_id: &str) -> Vec<&TraceEvent> {
        self.events
            .iter()
            .filter(|e| e.attempt_id.as_deref() == Some(attempt_id))
            .collect()
    }

    pub fn progress(&self) -> (usize, usize) {
        let distinct: HashSet<&str> = self.steps.iter().map(|s| s.node_id.as_str()).collect();
        (distinct.len(), self.flow.nodes.len())
    }

    pub fn duration_ms(&self, now_ms: i64) -> u64 {
        let end_ms = if let Some(fin) = self.summary.finished_ms {
            fin
        } else if self.summary.status == RunStatus::Running {
            now_ms.max(self.summary.updated_ms)
        } else {
            self.summary.updated_ms
        };
        let diff = end_ms.saturating_sub(self.summary.started_ms);
        if diff <= 0 { 0 } else { diff as u64 }
    }

    pub fn session_events_path(&self, session_id: &str) -> Option<&Path> {
        self.sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.events_path.as_path())
    }
}

````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 617-625 of 1936 (repository grok-build)
````
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

// ==== Private Helpers ====
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 1356-1370 of 1936 (repository grok-build)
````
/// as (label, flow file) in key order. Empty when the run is not waiting or the checkpoint declares none.
pub fn gate_choices(run: &Run) -> Vec<(String, String)> {
    if run.summary.status != RunStatus::Waiting {
        return Vec::new();
    }
    let Some(node) = run.summary.waiting_on.as_deref() else { return Vec::new() };
    let Some(step) = run.steps.iter().rev().find(|s| s.node_id == node) else { return Vec::new() };
    let Some(next) = step.output.get("next").and_then(Value::as_object) else { return Vec::new() };
    let mut choices: Vec<(String, String)> =
        next.iter().filter_map(|(label, file)| Some((label.clone(), file.as_str()?.to_string()))).collect();
    choices.sort();
    choices
}

/// What a gate hands the next flow: the first run's input and every node's last output, plus which gate was answered.
````

### `crates/codegen/xai-grok-pager/src/acpx/step_digest.rs` lines 157-306 of 1171 (repository grok-build)
````
pub fn non_zero_exit_code(step: &Step) -> Option<i64> {
    step.action
        .as_ref()
        .and_then(|action| action.exit_code)
        .filter(|&code| code != 0)
        .or_else(|| {
            step.output
                .get("exitCode")
                .and_then(Value::as_i64)
                .filter(|&code| code != 0)
        })
}

pub fn effective_outcome(step: &Step) -> Outcome {
    if step.outcome != Outcome::Ok {
        return step.outcome;
    }
    if non_zero_exit_code(step).is_some()
        || step.output.get("ok").and_then(Value::as_bool) == Some(false)
    {
        Outcome::Failed
    } else {
        Outcome::Ok
    }
}

pub fn trim_failure_tail(
    error: Option<&str>,
    stderr: Option<&str>,
    output_tail: Option<&str>,
    stdout: Option<&str>,
    exit_code: Option<i64>,
    max_lines: usize,
    max_chars: usize,
) -> Option<String> {
    if max_lines == 0 || max_chars == 0 {
        return None;
    }

    let source = [error, stderr, output_tail, stdout]
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty());

    if let Some(text) = source {
        let non_empty_lines: Vec<&str> = text
            .lines()
            .map(|line| line.trim_end_matches('\r').trim_end())
            .filter(|line| !line.trim().is_empty())
            .collect();

        if !non_empty_lines.is_empty() {
            let start = non_empty_lines.len().saturating_sub(max_lines);
            let joined = non_empty_lines.get(start..).unwrap_or_default().join("\n");
            return Some(truncate_chars(&joined, max_chars));
        }
    }

    exit_code
        .filter(|&code| code != 0)
        .map(|code| truncate_chars(&format!("exit code {code}"), max_chars))
}

pub fn step_failure_reason(step: &Step) -> Option<String> {
    let outcome = effective_outcome(step);
    if outcome == Outcome::Ok {
        return None;
    }

    let error = step
        .error
        .as_deref()
        .or_else(|| step.output.get("error").and_then(Value::as_str));
    let stderr = step.output.get("stderr").and_then(Value::as_str);
    let output_tail = step
        .output
        .get("outputTail")
        .or_else(|| step.output.get("output_tail"))
        .or_else(|| step.output.get("tail"))
        .or_else(|| step.output.get("message"))
        .and_then(Value::as_str)
        .or(step.raw_text.as_deref());
    let stdout = step.output.get("stdout").and_then(Value::as_str);
    let exit_code = non_zero_exit_code(step);

    if let Some(trimmed) = trim_failure_tail(
        error,
        stderr,
        output_tail,
        stdout,
        exit_code,
        DEFAULT_FAILURE_MAX_LINES,
        DEFAULT_FAILURE_MAX_CHARS,
    ) {
        return Some(trimmed);
    }

    match outcome {
        Outcome::TimedOut => Some(format!("timed out after {}", fmt_duration(step.duration_ms()))),
        Outcome::Cancelled => Some("cancelled".to_string()),
        _ => None,
    }
}

pub fn latest_run_failure(run: &Run) -> Option<(String, Option<i64>, String)> {
    for step in run.steps.iter().rev() {
        let outcome = effective_outcome(step);
        if matches!(outcome, Outcome::Failed | Outcome::TimedOut) {
            let exit_code = non_zero_exit_code(step);
            let reason = step_failure_reason(step).unwrap_or_else(|| match outcome {
                Outcome::TimedOut => {
                    format!("timed out after {}", fmt_duration(step.duration_ms()))
                }
                _ => "step failed".to_string(),
            });
            return Some((step.node_id.clone(), exit_code, reason));
        }
    }

    if let Some(err) = run.summary.error.as_deref()
        && let Some(trimmed) = trim_failure_tail(
            Some(err),
            None,
            None,
            None,
            None,
            DEFAULT_FAILURE_MAX_LINES,
            DEFAULT_FAILURE_MAX_CHARS,
        )
    {
        let node_id = run
            .summary
            .current_node
            .clone()
            .unwrap_or_else(|| run.flow.name.clone());
        return Some((node_id, None, trimmed));
    }

    if run.summary.status == RunStatus::TimedOut {
        let node_id = run
            .summary
            .current_node
            .clone()
            .unwrap_or_else(|| run.flow.name.clone());
        return Some((node_id, None, "run timed out".to_string()));
    }

    None
}

````

### `docs/acpx-modal-opening-screen.md` lines 1-62 of 63 (repository grok-build)
````
# ACPX opening screen proposal

Draft for review. This proposal covers the Flows screen you see when opening `/acpx`. It does not redesign workflow cards in the main pager, the open-run stage view, or the inspector. Those have specific review and evaluation cases already.

## What the screen should answer

1. What work is running, and what is waiting on a decision?
2. What happened on the run I selected?
3. How do I get to the exact run or step when I need the full evidence?

Open directly into Flows. Make that screen useful before opening anything. Keep the full ledger on the same screen, with groups the operator can expand, rather than adding a separate landing page or another Enter before they can inspect a run. Keep Agent Turns as the other tab.

## First view

Use three sections in the default grouped order: **Waiting for input**, **Running**, and **Recent outcomes**. Omit empty sections. The labels describe what the workflow is doing without guessing what the operator cares about.

- **Waiting for input:** Give every recorded wait the same treatment. Group repeated waits by workflow, project, and source when known, and sort groups and their members by most recent update. Each gate remains individually selectable. A group never borrows a member's question or choices. Age is context; it never hides or downgrades a wait. Keep a distinct status for answers in flight or accepted while the engine still reports waiting. Packet 5 owns gate and answer state.
- **Running:** Keep each run individually selectable. Show the known current stage. A stalled run remains here with a stalled label.
- **Recent outcomes:** Use packet 2's newest-first finished grouping and preserve each historical run. Keep the latest outcome visible on the collapsed row. Show failures and cancellations in plain language. The dot rail, executed-attempt count, and named execution path retain the agreed width behavior in packet 2. They describe the selected or representative run, never the sum of all members.

The guidance to place a gate when a result is worth checking or a build depends on it is for workflow authors. The modal applies no priority tier or extra metadata requirement based on that guidance.

Illustration using the reported repeated `ship: slugify` and `mixed-talk` entries and the historical `beta8-merge` failure. The waiting counts are examples from the conversation. The right pane illustrates what to show when recorded evidence is available.

```text
 Flows  |  Agent Turns

 Waiting for input            │ beta8-merge · Failed
 ▸ ship: slugify    5 waiting  │ Failed at cargo_check
 ▸ mixed-talk      2 waiting  │ Compiler error: <saved diagnostic, if present>
                              │
 Recent outcomes              │ Executed path
 > beta8-merge     Failed     │ <recorded attempts in order>
 ▸ mixed-talk      8 finished │
                              │
 Space  Expand group          │ Tab  Focus preview
 /  Filter runs               │ Enter  Open run
```

The example has a failure selected. On a fresh open, select the first row in the current sort order. After inspecting a run or refreshing the list, keep the same selected run by ID while it remains available.

## What gets disclosed on selection and on open

- **Select a group:** Show its count and what it contains. For a finished group, the preview shows the newest run and labels it as such. For a waiting group, show that there are several separate gates and `Space` reveals them. Do not show one member's choices as group actions.
- **Select a run:** At wide widths, the right pane is a compact, read-only summary. Show its real status and one useful piece of evidence: the saved gate question and available choices, the active step, the failed attempt and a short diagnostic, or the result of a completed run. Use the recorded path and outcome rather than unrun nodes or raw JSON wrappers. If the summary has no diagnostic or question, say what is missing. The full output stays in the open run and inspector.
- **Narrow widths:** Put a brief status or question below the selected row, plus packet 2's selected-run path where applicable. Keep that inline preview focusable with the same keys as the side pane. Keep the workflow name and status readable, let the question wrap, and keep long output behind Enter.
- **Focus and copy:** In Flows, `Tab` and `Shift+Tab` move between the list and the selected preview, with a visible focus marker. Use `h`/`l` or Left/Right from the list to switch between Flows and Agent Turns. In the preview, reuse packet 3b's keys: `j`/`k` or arrows move the line cursor, `v` selects a range, `y` copies the line or range, and `Y` copies the preview. Copy original text without line breaks introduced by wrapping. `Esc` clears a range first and then returns focus to the same list row. The list keeps its existing copy behavior.
- **Open:** With the list focused, `Space` expands or folds a group; `Enter` on a waiting group expands it, and `Enter` on an individual run opens it. For a finished group, preserve packet 2's representative-run behavior. Keep `/` filter and existing sort options. Searching by run ID reveals a matching run through folded groups. `Esc` from an open run returns to the same row and expansion state. Previewing or opening a run never submits, starts, or cancels anything.

## Review against these cases

| Case | What should be observable |
| --- | --- |
| Five `ship: slugify` waits and two `mixed-talk` waits | The default view shows two group rows. Expand `ship: slugify`; each member is separately selectable and shows its own question. A five-day-old wait is still reachable. A known build dependency does not change where its gate appears. |
| `beta8-merge` failure | Select the run, Tab into its preview, and copy an exact saved diagnostic line when available. Esc returns focus to the same row; Enter opens the full evidence. Repeat the copy from the inline preview at narrow width. |
| Completed run with retries | Keep the rail, count, and named path; retries occur in recorded order. Narrow width keeps the selected run's full path reachable without widening the screen. |
| Waiting run updates after an answer | The UI distinguishes the answer status from the engine state. The row moves to outcomes only when the engine reports a terminal state. No input is sent by opening the row. |
| Empty list or no matches | Say which case applies. The other tab and clearing a filter stay reachable. |

## Boundary with the current packets

This proposal replaces packet 4's extra-entry-screen approach if accepted. Packet 2's finished grouping and rail/count/path contract stay intact. This proposal changes how repeated waits appear by default: multiple waits share a folded row, and each remains individually selectable after expansion. The compact preview is also new work. Packet 5 still governs permissions and engine state. The main pager, modal stage view, and inspector need their already identified walkthroughs, not a second blanket design pass. Review whether this arrangement solves the clutter and makes the next action clear. Code, slice boundaries, and dispatch are outside this draft.
````

=== TASK SPECIFICATION ===

Slice ID: acpx-opening-preview

This implements the reviewed opening-screen preview and replaces the old
packet 4 design assignment. Own only crates/codegen/xai-grok-pager/src/views/acpx_modal.rs: preview state, selected
preview evidence, Ledger-mode rendering and input, help and relevant tests.
Keep Flows as the initial view. Do not edit ledger.rs, detail.rs or Run/Step
behavior; the ledger supplies the shared interface below. No new landing mode.
Do not edit row_extra or its tests owned by packet 2; attach the new code to
anchors in the preview/input/render regions. The ledger packet adds no APP_MODAL
imports; it uses qualified paths for new helpers there. If a shared function is
edited, leave its non-Ledger arms intact. Packet 3b must render from the merged
base before editing shared input/render functions.

Replace the current ledger DetailPanel render with one compact, read-only
preview built from the existing selected-run bundle cache. Reuse recorded
status and fields; a waiting group shows its label, count and Space hint,
never a member's question/choices. A finished group previews the representative
and identifies it as such; obtain that indicator via the ledger interface.
An individual wait shows its saved question and choices as plain evidence when
present, with no pick shortcuts or answer action; an answered wait remains
waiting until the engine moves it. Running shows the known current step.
Failed shows the failed attempt, available exit status and a short diagnostic
using existing step evidence; prefer a complete original diagnostic line to
a rewritten or summarized copy payload. Completed shows a short recorded
result, without raw structured wrappers; cancellations stay cancellations.
Use actual ordered Step attempts for the path and do not show unrun nodes.
State missing question or diagnostic plainly. Do not read artifacts on every
render, add new fetches, or widen bundle/cache limits. Keep preview identity
separate from DetailPanel; clear/clamp the cursor and range on selection,
refresh or mode changes so stale content cannot be copied.

At wide widths show the summary on the right; at narrow widths pass its
logical lines and selection to the ledger inline hook, with one visible copy
of the selected path (suppress standalone continuation when replaced). Use
the same logical lines for copy even when the screen wraps them. Tab/Shift+Tab
move between list and a nonempty preview with visible focus. When no preview
exists leave navigation usable. h/l or Left/Right from the list still switch
tabs. Agent Turns behavior stays as-is, including its existing Tab route.
Keep filter capture and help above preview input. When focused, j/k and
Up/Down move the line cursor, v starts/ends the range, y copies the exact
logical line/range, Y copies all preview lines, and Esc clears a selection
first then returns to the same list row. No render-only wraps, indentation,
ANSI or truncation in the copied payload. Use AcpxOutcome::Copy. List y keeps
packet 2 behavior. Route wants_esc accordingly, update displayed shortcuts
and help, and keep mouse wheel scrolling the pane under pointer without
mutating the unrelated run-detail panel. Enter from the list still opens the
selected run (after expanding a waiting group if necessary); Esc from Run
returns to the same row. No preview key can answer, launch, or cancel.

Tests: failed cargo_check with a multiline diagnostic, a waiting group and
individual question, running and finished summaries, unavailable evidence,
wide and narrow focus and wrapping, exact y/v/Y, Esc ordering, selection after
refresh/open/back, tab reachability and filter capture, empty/tiny layouts and
no workflow-control outcome from preview input. Coordinator validates the two
opening packets together and checks real clipboard paste and those views at
both widths before shipping; do not claim live verification.

Shared ledger/preview contract (ledger owns the definitions, preview consumes):
- LedgerState::selected_waiting_group(&self) -> Option<(&str, usize)> returns
  the selected waiting group's display label and number of members, only on
  Flows. On that group selected_run() returns None; on a finished group it
  still returns the representative. The group itself has no gate or run ID.
- LedgerState::selected_run_is_finished_group(&self) -> bool is true only for
  a selected finished group representative, not an ordinary run or child.
- LedgerState::set_inline_preview(&mut self, lines: Option<Vec<InlinePreviewLine>>)
  sets the selected row's inline preview for the next render. In ledger.rs,
  pub(super) struct InlinePreviewLine has pub(super) fields text: String,
  cursor: bool, selected: bool. Each item is one logical line. None preserves
  the existing ledger continuation behavior; Some, including Some(empty),
  replaces default question/path continuations so they appear at most once.
  The ledger wraps for display and scrolls the focused cursor into view; the
  modal owns unwrapped copy text and never copies display indentation or wraps.
  Only the modal supplies lines, per render, in the narrow Flows view. No new
  I/O in the ledger, no new dependencies, and no new gate authority.

Edit list format. The file is Markdown. List the edits in application order;
each edit has exactly this shape:

## Edit <N>: <short purpose>
Path: <repository-relative path>
OLD:
~~~rust
<existing lines copied verbatim from one attached excerpt>
~~~
NEW:
~~~rust
<complete replacement>
~~~

Rules:
- Copy OLD character for character from an attached excerpt, including
  indentation, blank lines and comments. Never retype, reformat, abbreviate or
  elide it. OLD must occur exactly once in the file: include enough lines to be
  unique, normally the whole function, impl item, struct or test.
- Every OLD quotes the original attached source, never the result of an
  earlier edit. No two OLD blocks overlap.
- To insert new code (a field, function or test), quote a nearby existing
  anchor as OLD and give the anchor plus the new code as NEW.
- NEW is complete code with no "...", "unchanged" or elided bodies.
- Prefer replacing whole functions and whole test functions to small fragments.
- Inside the file use only ~~~ fences, never triple backticks.
- Never edit code outside the attached excerpts. If the change needs that, add
  a final "## Coordinator follow-ups" section naming each location and reason.

The coordinator matches each OLD exactly once on the pinned base and stops on
zero or several matches, then applies the edits and runs format, compile and
the targeted tests.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-returns/acpx-opening-preview.edits.md ===
```markdown
<complete Markdown edit list>
```
