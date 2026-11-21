pub(crate) mod detail;
pub(crate) mod inspector;
pub(crate) mod ledger;
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
    /// The right-hand panel: the selected run beside the list, the selected stage beside a run.
    detail: DetailPanel,
    /// The run selected in the list, loaded for the preview.
    preview: Option<Run>,
    /// Rail and agents per run id, with the `updated_ms` they were read at.
    extras: HashMap<String, (i64, RowExtra)>,
    /// Gate answers per run id for runs beyond `EXTRAS_LIMIT`.
    answers: HashMap<String, Option<GateAnswer>>,
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
            detail_rect: None,
        };
        state.rescan_runs();
        state.rescan_sessions();
        state
    }

    pub fn open_run(&mut self, dir: &Path) -> bool {
        match bundle::load_run(dir) {
            Ok(run) => {
                let run_id = run.summary.run_id.clone();
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
            // Runs are started outside the pager, so the list is always re-read.
            Mode::Ledger => true,
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
                    let dir = rv.run().summary.dir.clone();
                    rv.set_settings(bundle::node_settings(&dir), bundle::step_settings(&dir));
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

    /// One line shown at the bottom until the next key.
    pub fn set_notice(&mut self, text: String) {
        self.notice = Some(text);
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

    fn rescan_runs(&mut self) -> bool {
        let runs = self
            .runs_base
            .as_deref()
            .map(bundle::list_runs)
            .unwrap_or_default();
        let changed = runs != self.runs_snapshot;
        self.runs_snapshot = runs.clone();
        self.load_extras(&runs);
        self.ledger.set_runs(runs);
        if let Some(rv) = self.run_view.as_mut() {
            let run_id = rv.run().summary.run_id.clone();
            rv.set_answered(self.ledger.answered_choice(&run_id).map(str::to_string));
        }
        changed
    }

    /// Rail, agents and question for the newest runs. A run is re-read only when acpx has written it since.
    fn load_extras(&mut self, runs: &[RunSummary]) {
        let now = now_ms();
        let mut kept = HashMap::new();
        for summary in runs.iter().take(EXTRAS_LIMIT) {
            let cached = self.extras.remove(&summary.run_id).filter(|(at, _)| *at == summary.updated_ms);
            let (at, mut extra) = cached.unwrap_or_else(|| (summary.updated_ms, row_extra(summary)));
            extra.stalled = bundle::is_stalled(summary, now);
            kept.insert(summary.run_id.clone(), (at, extra));
        }
        let mut all_extras: HashMap<String, RowExtra> = kept
            .iter()
            .map(|(id, (_, extra))| (id.clone(), extra.clone()))
            .collect();
        for summary in runs.iter().skip(EXTRAS_LIMIT) {
            let answers = self
                .answers
                .entry(summary.run_id.clone())
                .or_insert_with(|| read_run_answers(&summary.dir))
                .clone();
            all_extras.insert(
                summary.run_id.clone(),
                RowExtra { answers, ..RowExtra::default() },
            );
        }
        self.ledger.set_extras(all_extras);
        self.extras = kept;
    }

    /// Keep `preview` on the run selected in the list.
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
            RunOutcome::ScrollDetail(delta) => {
                if self.detail.scroll(delta) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            RunOutcome::Gate { choice, flow_file } => {
                let run = rv.run();
                // A relative flow file is beside the flow that declared it.
                let declared = PathBuf::from(&flow_file);
                let flow_file = match bundle::flow_path(&run.summary.dir).as_deref().and_then(Path::parent) {
                    Some(dir) if declared.is_relative() => dir.join(declared),
                    _ => declared,
                };
                let input_json = bundle::gate_input(run, &choice).to_string();
                // Back to the list, where the new run appears.
                self.notice = Some(format!("{choice}: started {}", flow_file.display()));
                self.mode = Mode::Ledger;
                self.run_view = None;
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
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
        if !labels.contains(&"? keys") {
            labels.push("? keys");
        }
        labels
    }
}

/// What the list shows for one run beyond its summary. Reads the run folder.
fn row_extra(summary: &RunSummary) -> RowExtra {
    let Ok(run) = bundle::load_run(&summary.dir) else { return RowExtra::default() };
    let steps = bundle::step_settings(&summary.dir);
    let nodes = bundle::node_settings(&summary.dir);
    // The agent that did the most recent agent step, else the first one the flow declares.
    let agents = run
        .steps
        .iter()
        .rev()
        .filter_map(|step| steps.get(&step.attempt_id))
        .chain(run.flow.nodes.iter().filter_map(|node| nodes.get(&node.id)))
        .map(bundle::AgentSettings::label)
        .find(|label| !label.is_empty())
        .unwrap_or_default();
    RowExtra {
        rail: run.flow.nodes.iter().map(|node| run.node_state(&node.id)).collect(),
        agents,
        stalled: false,
        question: (run.summary.status == bundle::RunStatus::Waiting).then(|| run.summary.status_detail.clone()).flatten(),
        answers: parse_gate_answer(&run.input),
    }
}

fn parse_gate_answer(input: &serde_json::Value) -> Option<GateAnswer> {
    let gate = input.get("gate")?.as_object()?;
    let run_id = gate.get("runId")?.as_str()?;
    if run_id.is_empty() {
        return None;
    }
    let choice = gate
        .get("choice")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(GateAnswer {
        run_id: run_id.to_string(),
        choice,
    })
}

fn read_run_answers(dir: &Path) -> Option<GateAnswer> {
    let text = std::fs::read_to_string(dir.join("projections").join("run.json")).ok()?;
    let val = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let input = val.get("input")?;
    parse_gate_answer(input)
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
    ("Tab / h / l", "Switch between Flows and Sessions"),
    ("j / k / ↑ / ↓", "Move selection"),
    ("g / G", "Jump to first / last row"),
    ("J / K", "Scroll the preview of the selected run"),
    ("Enter", "Open selected flow run or replay session"),
    ("/", "Filter rows by text"),
    ("a", "Only running and waiting runs"),
    ("s", "Cycle order: grouped, recent, name"),
    ("y", "Copy selected id or path"),
    ("R", "Rescan flows and sessions on disk"),
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
        // The list is always re-read: new runs appear from outside.
        assert!(state.wants_poll());

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
        let area = buf.area;
        render_acpx_modal(&mut buf, area, &mut state, false);
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
        let area = buf.area;
        render_acpx_modal(&mut buf, area, &mut state, false);
        let rendered = buffer_text(&buf);
        assert!(rendered.contains("acpx"));

        // 10x3 and 0x0 must not panic
        let mut tiny = Buffer::empty(Rect::new(0, 0, 10, 3));
        let area = tiny.area;
        render_acpx_modal(&mut tiny, area, &mut state, true);

        let mut zero = Buffer::empty(Rect::new(0, 0, 0, 0));
        let area = zero.area;
        render_acpx_modal(&mut zero, area, &mut state, false);
    }

    #[test]
    fn row_extra_reads_gate_answer_and_handles_missing_or_malformed_gate() {
        let fx = write_fixture("completed");
        let run_json_path = fx.run_dir.join("projections").join("run.json");

        fs::write(
            &run_json_path,
            r#"{"input":{"gate":{"runId":"r1","node":"g","choice":"approve"}}}"#,
        )
        .expect("write run.json");
        let runs = bundle::list_runs(&fx.runs_base);
        let summary = runs.first().expect("one run");
        let extra = row_extra(summary);
        assert_eq!(
            extra.answers,
            Some(GateAnswer {
                run_id: "r1".into(),
                choice: "approve".into(),
            })
        );

        fs::write(&run_json_path, r#"{"input":{"other":1}}"#).expect("write run.json without gate");
        let extra = row_extra(summary);
        assert_eq!(extra.answers, None);

        fs::write(&run_json_path, r#"{"input":{"gate":"not-an-object"}}"#)
            .expect("write run.json with string gate");
        let extra = row_extra(summary);
        assert_eq!(extra.answers, None);
    }

    #[test]
    fn run_outside_extras_limit_gets_answers_with_empty_rail() {
        let fx = write_fixture("completed");
        fs::write(
            fx.run_dir.join("projections").join("run.json"),
            r#"{"input":{"gate":{"runId":"waiting-run","node":"g","choice":"ship"}}}"#,
        )
        .expect("write old run.json");

        for i in 0..EXTRAS_LIMIT {
            let dir = fx.runs_base.join(format!("newer-{i:03}"));
            let proj = dir.join("projections");
            fs::create_dir_all(&proj).expect("create newer proj");
            let live = format!(
                r#"{{
                    "runId": "newer-{i:03}",
                    "flowName": "deploy-flow",
                    "runTitle": "Newer {i}",
                    "status": "completed",
                    "startedAt": "2025-03-02T10:{:02}:{:02}Z",
                    "updatedAt": "2025-03-02T10:{:02}:{:02}Z"
                }}"#,
                i / 60,
                i % 60,
                i / 60,
                i % 60
            );
            fs::write(proj.join("live.json"), live).expect("write newer live.json");
            fs::write(
                dir.join("flow.json"),
                r#"{"name":"deploy-flow","startAt":"plan","nodes":{"plan":{"nodeType":"acp"}}}"#,
            )
            .expect("write newer flow.json");
        }

        let state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        assert!(!state.extras.contains_key("run-001"));
        assert_eq!(
            state.ledger.answered_choice("waiting-run"),
            Some("ship")
        );
    }
}
