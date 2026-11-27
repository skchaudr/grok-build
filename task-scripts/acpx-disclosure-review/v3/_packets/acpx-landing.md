Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, ledger routing, or release behavior.

=== REPOSITORY CONTEXT (10 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1-242 of 1571 (repository grok-build)
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 291-376 of 1571 (repository grok-build)
````
    fn rescan_runs(&mut self) -> bool {
        let runs = runtime::list_runs_with_mirror(self.runs_base.as_deref());
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

    /// A mirrored run was opened: keep mirroring it in full.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 484-541 of 1571 (repository grok-build)
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 631-668 of 1571 (repository grok-build)
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 730-895 of 1571 (repository grok-build)
````

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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1123-1193 of 1571 (repository grok-build)
````

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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 105-168 of 2990 (repository grok-build)
````
enum SectionKind {
    NeedsYou,
    Running,
    Finished,
}

impl SectionKind {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "NEEDS YOU",
            Self::Running => "RUNNING",
            Self::Finished => "FINISHED",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::NeedsYou => theme.warning,
            Self::Running => theme.accent_running,
            Self::Finished => theme.gray_dim,
        }
    }
}

#[derive(Debug, Clone)]
enum FlowListRow {
    Header { kind: SectionKind, count: usize },
    Blank,
    Run(usize), // index in `runs`
    Continuation { indent: usize, text: String },
}

impl FlowListRow {
    fn is_selectable(&self) -> bool {
        matches!(self, Self::Run(_))
    }
}

pub struct LedgerState {
    tab: LedgerTab,
    runs: Vec<RunSummary>,
    extras: HashMap<String, RowExtra>,
    answered: HashMap<String, String>,
    sessions: Vec<SessionSummary>,

    // Per-tab filter, order, active-only, selection & scroll
    flow_filter: String,
    flow_active_only: bool,
    flow_order: FlowOrder,
    flow_selected_run_id: Option<String>,
    flow_scroll: usize,

    session_filter: String,
    session_active_only: bool,
    session_order: SessionOrder,
    session_selected_id: Option<String>,
    session_scroll: usize,

    filter_open: bool,
    last_render_height: usize,
    last_render_width: usize,
}

impl Default for LedgerState {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 259-310 of 2990 (repository grok-build)
````
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

    fn is_stalled(&self, run: &RunSummary) -> bool {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs` lines 377-445 of 2990 (repository grok-build)
````
    fn run_section(&self, run: &RunSummary) -> SectionKind {
        let stalled = self.is_stalled(run);
        if run.status == RunStatus::Waiting && !self.is_answered(run) {
            SectionKind::NeedsYou
        } else if run.status == RunStatus::Running && !stalled {
            SectionKind::Running
        } else {
            SectionKind::Finished
        }
    }

    /// Returns indices into `self.runs` in display order.
    fn filtered_flow_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .runs
            .iter()
            .enumerate()
            .filter_map(|(i, r)| self.matches_flow_filter(r).then_some(i))
            .collect();

        match self.flow_order {
            FlowOrder::Grouped => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    let sa = match self.run_section(ra) {
                        SectionKind::NeedsYou => 0,
                        SectionKind::Running => 1,
                        SectionKind::Finished => 2,
                    };
                    let sb = match self.run_section(rb) {
                        SectionKind::NeedsYou => 0,
                        SectionKind::Running => 1,
                        SectionKind::Finished => 2,
                    };
                    sa.cmp(&sb)
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Recent => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    rb.updated_ms
                        .cmp(&ra.updated_ms)
                        .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
            FlowOrder::Name => {
                indices.sort_by(|&a, &b| {
                    let ra = &self.runs[a];
                    let rb = &self.runs[b];
                    ra.title
                        .to_lowercase()
                        .cmp(&rb.title.to_lowercase())
                        .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                        .then_with(|| ra.run_id.cmp(&rb.run_id))
                });
            }
        }
        indices
    }

    /// Build visual rows for the Flows tab. `list_height` is used to decide whether blank rows
    /// separate sections when the list is shorter than the area.
    fn build_flow_rows(&self, list_height: usize, now_ms: i64) -> Vec<FlowListRow> {
````

### `crates/codegen/xai-grok-pager/src/acpx/bundle.rs` lines 261-295 of 1936 (repository grok-build)
````
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
````

=== TASK SPECIFICATION ===

Slice ID: acpx-landing

Produce an operator-facing first-screen prototype, not a product patch.
Inputs: the current AcpxModalState routes and full Flows/Sessions lists in
this packet; a genuinely waiting run asking 'Approve integration?', a
failed beta8-merge run with cargo_check exit 101, and a running fixture.
The screen's first job is to identify work needing attention, open a run,
reach the full lists, or find how to start work. Opening is inspection only.
Do not invent a start route if the attached sources do not establish one.

Design against the current, ungrouped ledger shown here; grouping finished
runs is a separate packet. Which runs need attention depends on a separate
gate-state contract (for example, an answer the engine accepted while the run
still reports waiting). State the attention predicate you assume and mark it
as a dependency on that contract, not a decision.

Return one proposed screen at 80x24, the same state at 50x14, and an empty
case. Give the exact Tab/Shift+Tab, arrow, Enter, and Esc transitions, return
focus when closing a run, and an explicit route to both full lists. Name
which current mode, render branch, and handlers would change. Recommend
whether the full ledger keeps its compact preview and justify the choice
using these cases; a deferred preview packet waits on this recommendation.
Separate any design assumptions from observed behavior.
No changes to the current full ledger, gate authority, or keymap outside
this proposed entry route. List the interactive checks a reviewer can run.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-returns/acpx-landing.md ===
```markdown
<complete Markdown artifact>
```
