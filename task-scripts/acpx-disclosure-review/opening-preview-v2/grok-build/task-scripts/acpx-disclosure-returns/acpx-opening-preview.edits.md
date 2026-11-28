## Edit 1: Add separate ledger preview state field to AcpxModalState
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
    /// The right-hand panel for the open run view (selected stage beside a run).
    detail: DetailPanel,
    /// Compact read-only preview for the opening ledger screen (kept separate from `detail`).
    ledger_preview: LedgerPreviewState,
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
    /// Where the right-hand preview or run-detail panel was last drawn, for the mouse wheel.
    detail_rect: Option<Rect>,
}
~~~

## Edit 2: Initialize and clear ledger preview state in open_at and open_run
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
                self.detail.set_focused(false);
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
~~~
NEW:
~~~rust
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
            ledger_preview: LedgerPreviewState::new(),
            preview: None,
            extras: HashMap::new(),
            answers: HashMap::new(),
            answerers: Vec::new(),
            open_mirror_run: None,
            detail_rect: None,
        };
        state.rescan_runs();
        state.rescan_sessions();
        state.sync_ledger_preview();
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
                self.detail.set_focused(false);
                self.ledger_preview.clear_focus();
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
~~~

## Edit 3: Sync ledger preview on poll and include preview focus in wants_esc
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    pub fn poll(&mut self) -> bool {
        let answerers_changed = self.poll_answerers();
        match self.mode {
            Mode::Ledger => {
                let runs_changed = self.rescan_runs();
                self.sync_ledger_preview();
                runs_changed | answerers_changed
            }
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
        self.help_open
            || self.mode != Mode::Ledger
            || self.ledger_preview.is_focused()
            || !self.ledger.esc_closes()
    }
~~~

## Edit 4: Route Ledger J/K and mouse wheel scrolling to ledger_preview without mutating detail
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
                return if self.ledger_preview.scroll(delta) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                };
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
                    mouse.column >= r.x
                        && mouse.column < r.x + r.width
                        && mouse.row >= r.y
                        && mouse.row < r.y + r.height
                });
                if over_detail {
                    match self.mode {
                        Mode::Ledger => {
                            return if self.ledger_preview.scroll(delta) {
                                AcpxOutcome::Changed
                            } else {
                                AcpxOutcome::Unchanged
                            };
                        }
                        Mode::Run => {
                            return if self.detail.scroll(delta) {
                                AcpxOutcome::Changed
                            } else {
                                AcpxOutcome::Unchanged
                            };
                        }
                        Mode::Step => {}
                    }
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
~~~

## Edit 5: Add sync_ledger_preview and route Flows preview focus/copy/Esc keys in handle_ledger_key
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

    /// Synchronize the compact opening-screen preview with the current ledger selection.
    fn sync_ledger_preview(&mut self) {
        if self.ledger.tab() != self::ledger::LedgerTab::Flows {
            self.preview = None;
            self.ledger_preview.sync(PreviewIdentity::None, Vec::new());
            return;
        }

        if let Some((label, count)) = self.ledger.selected_waiting_group() {
            self.preview = None;
            let identity = PreviewIdentity::WaitingGroup {
                label: label.to_string(),
                count,
            };
            let lines = build_waiting_group_preview_lines(label, count);
            self.ledger_preview.sync(identity, lines);
            return;
        }

        self.sync_preview();
        let Some(selected) = self.ledger.selected_run() else {
            self.ledger_preview.sync(PreviewIdentity::None, Vec::new());
            return;
        };

        let run_id = selected.run_id.clone();
        let finished_group = self.ledger.selected_run_is_finished_group();
        let answered = self.ledger.answered_choice(&run_id).map(str::to_string);
        let stalled = self
            .extras
            .get(&run_id)
            .is_some_and(|(_, extra)| extra.stalled);
        let extra_question = self
            .extras
            .get(&run_id)
            .and_then(|(_, extra)| extra.question.as_deref());
        let identity = PreviewIdentity::Run {
            run_id: run_id.clone(),
            updated_ms: selected.updated_ms,
            finished_group,
            answered: answered.clone(),
            stalled,
        };
        let loaded = self
            .preview
            .as_ref()
            .filter(|run| run.summary.run_id == run_id);
        let lines = build_run_preview_lines(
            selected,
            loaded,
            finished_group,
            answered.as_deref(),
            stalled,
            extra_question,
        );
        self.ledger_preview.sync(identity, lines);
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
        if key.kind == crossterm::event::KeyEventKind::Release {
            return AcpxOutcome::Unchanged;
        }

        let captures = self.ledger.captures_text();
        if !captures && self.ledger.tab() == self::ledger::LedgerTab::Flows {
            self.sync_ledger_preview();

            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                if self.ledger_preview.is_focused() {
                    self.ledger_preview.set_focused(false);
                    return AcpxOutcome::Changed;
                }
                if self.ledger_preview.has_content() {
                    self.ledger_preview.set_focused(true);
                    return AcpxOutcome::Changed;
                }
                // No preview exists (e.g. empty list or no filter matches): fall through so Tab
                // keeps Agent Turns reachable.
            }

            if self.ledger_preview.is_focused() {
                return match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        if self.ledger_preview.move_cursor(1) {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if self.ledger_preview.move_cursor(-1) {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::PageDown => {
                        let step = self.ledger_preview.page_step();
                        if self.ledger_preview.move_cursor(step) {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::PageUp => {
                        let step = self.ledger_preview.page_step();
                        if self.ledger_preview.move_cursor(-step) {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        if self.ledger_preview.jump_start() {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        if self.ledger_preview.jump_end() {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::Char('v') => {
                        if self.ledger_preview.toggle_visual() {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    KeyCode::Char('y') => match self.ledger_preview.copy_selection() {
                        Some(text) => AcpxOutcome::Copy(text),
                        None => AcpxOutcome::Unchanged,
                    },
                    KeyCode::Char('Y') => match self.ledger_preview.copy_all() {
                        Some(text) => AcpxOutcome::Copy(text),
                        None => AcpxOutcome::Unchanged,
                    },
                    KeyCode::Esc => {
                        if self.ledger_preview.handle_esc() {
                            AcpxOutcome::Changed
                        } else {
                            AcpxOutcome::Unchanged
                        }
                    }
                    _ => AcpxOutcome::Unchanged,
                };
            }
        }

        match self.ledger.handle_key(key) {
            LedgerOutcome::Unchanged => AcpxOutcome::Unchanged,
            LedgerOutcome::Changed => {
                self.sync_ledger_preview();
                AcpxOutcome::Changed
            }
            LedgerOutcome::OpenRun(dir) => {
                self.open_run(&dir);
                AcpxOutcome::Changed
            }
            LedgerOutcome::OpenSession(s) => AcpxOutcome::Replay(session_replay(s)),
            LedgerOutcome::Copy(text) => AcpxOutcome::Copy(text),
            LedgerOutcome::Rescan => {
                self.rescan_runs();
                self.rescan_sessions();
                self.sync_ledger_preview();
                AcpxOutcome::Changed
            }
            LedgerOutcome::Close => AcpxOutcome::Close,
        }
    }
~~~

## Edit 6: Clear and resync ledger preview when returning from Mode::Run to Mode::Ledger
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
        match rv.handle_key(key) {
            RunOutcome::Unchanged => AcpxOutcome::Unchanged,
            RunOutcome::Changed => AcpxOutcome::Changed,
            RunOutcome::Back => {
                self.detail.set_focused(false);
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
                self.detail.set_focused(false);
                self.mode = Mode::Ledger;
                self.run_view = None;
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
        }
~~~
NEW:
~~~rust
        match rv.handle_key(key) {
            RunOutcome::Unchanged => AcpxOutcome::Unchanged,
            RunOutcome::Changed => AcpxOutcome::Changed,
            RunOutcome::Back => {
                self.detail.set_focused(false);
                self.ledger_preview.clear_focus();
                self.mode = Mode::Ledger;
                self.run_view = None;
                self.inspector = None;
                self.release_open_mirror_run();
                self.rescan_runs();
                self.sync_ledger_preview();
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
                self.detail.set_focused(false);
                self.ledger_preview.clear_focus();
                self.mode = Mode::Ledger;
                self.run_view = None;
                self.sync_ledger_preview();
                AcpxOutcome::RunFlow { flow_file, input_json }
            }
        }
~~~

## Edit 7: Update footer shortcut labels for ledger preview focus and Flows tab
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
    fn active_shortcut_labels(&self) -> Vec<&'static str> {
        let mut labels = match self.mode {
            Mode::Ledger => self.ledger.shortcuts(),
            Mode::Run => {
                let captures = self.run_view.as_ref().is_some_and(|rv| rv.captures_keys());
                if self.detail.is_focused() && !captures {
                    vec!["j/k line", "v range", "y copy", "Y all", "Esc tree"]
                } else {
                    self.run_view
                        .as_ref()
                        .map_or_else(Vec::new, |rv| rv.shortcuts())
                }
            }
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
~~~
NEW:
~~~rust
    fn active_shortcut_labels(&self) -> Vec<&'static str> {
        let mut labels = match self.mode {
            Mode::Ledger => {
                let captures = self.ledger.captures_text();
                if !captures && self.ledger_preview.is_focused() {
                    vec!["j/k line", "v range", "y copy", "Y all", "Esc list"]
                } else if !captures
                    && self.ledger.tab() == self::ledger::LedgerTab::Flows
                    && self.ledger_preview.has_content()
                {
                    let mut out = Vec::new();
                    for label in self.ledger.shortcuts() {
                        if label == "Tab switch" {
                            out.push("Tab preview");
                            out.push("h/l tabs");
                        } else {
                            out.push(label);
                        }
                    }
                    out
                } else {
                    self.ledger.shortcuts()
                }
            }
            Mode::Run => {
                let captures = self.run_view.as_ref().is_some_and(|rv| rv.captures_keys());
                if self.detail.is_focused() && !captures {
                    vec!["j/k line", "v range", "y copy", "Y all", "Esc tree"]
                } else {
                    self.run_view
                        .as_ref()
                        .map_or_else(Vec::new, |rv| rv.shortcuts())
                }
            }
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
~~~

## Edit 8: Replace Ledger DetailPanel render with wide/narrow compact preview, update help keys, and define LedgerPreviewState
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
                    } else {
                        state.detail.set_focused(false);
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
    ("Space", "Expand or collapse a finished-run group (Flows)"),
    ("/", "Filter rows by text"),
    ("a", "Only running and waiting runs"),
    ("s", "Cycle order: grouped, recent, name"),
    ("y", "Copy selected id or path"),
    ("R", "Rescan flows and agent turns on disk"),
    ("?", "Toggle this key reference"),
    ("Esc", "Clear filter or close modal"),
];
~~~
NEW:
~~~rust
pub fn render_acpx_modal(
    buf: &mut Buffer,
    full_area: Rect,
    state: &mut AcpxModalState,
    compact: bool,
) {
    if full_area.width == 0 || full_area.height == 0 {
        return;
    }
    if state.mode == Mode::Ledger {
        state.sync_ledger_preview();
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
                let has_flows_preview = state.ledger.tab() == self::ledger::LedgerTab::Flows
                    && state.ledger_preview.has_content();
                if has_flows_preview && content.width >= PREVIEW_MIN_WIDTH {
                    // Wide Flows: list on the left, compact read-only preview on the right.
                    state.ledger.set_inline_preview(None);
                    let list_w = (content.width * 9 / 20).clamp(56, 110);
                    let list = Rect::new(content.x, content.y, list_w, content.height);
                    let right_w = content.width.saturating_sub(list_w + 2);
                    let right = Rect::new(content.x + list_w + 2, content.y, right_w, content.height);
                    state.ledger.render(buf, list, &theme, now);
                    let rule_color = if state.ledger_preview.is_focused() {
                        theme.warning
                    } else {
                        theme.gray_dim
                    };
                    let rule = Style::default().fg(rule_color);
                    for y in content.y..content.y + content.height {
                        buf.set_stringn(content.x + list_w, y, "│", 1, rule);
                    }
                    state.ledger_preview.render(buf, right, &theme);
                    state.detail_rect = Some(right);
                } else if has_flows_preview {
                    // Narrow Flows: pass unwrapped logical lines to the ledger inline hook so
                    // the path and question render once and stay focusable/copyable.
                    state
                        .ledger
                        .set_inline_preview(Some(state.ledger_preview.inline_lines()));
                    state.ledger.render(buf, content, &theme, now);
                } else {
                    state.ledger.set_inline_preview(None);
                    state.ledger.render(buf, content, &theme, now);
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
                    } else {
                        state.detail.set_focused(false);
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewIdentity {
    None,
    WaitingGroup {
        label: String,
        count: usize,
    },
    Run {
        run_id: String,
        updated_ms: i64,
        finished_group: bool,
        answered: Option<String>,
        stalled: bool,
    },
}

impl PreviewIdentity {
    fn same_target(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::WaitingGroup { label: a, .. },
                Self::WaitingGroup { label: b, .. },
            ) => a == b,
            (Self::Run { run_id: a, .. }, Self::Run { run_id: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewLineKind {
    Title,
    Badge,
    Section,
    Body,
    Diagnostic,
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewLine {
    text: String,
    kind: PreviewLineKind,
}

impl PreviewLine {
    fn new(kind: PreviewLineKind, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
        }
    }
}

/// Compact read-only preview state for the opening Flows ledger screen.
/// Owns logical unwrapped lines for copy and independent cursor/range/scroll state.
struct LedgerPreviewState {
    identity: PreviewIdentity,
    lines: Vec<PreviewLine>,
    focused: bool,
    cursor: usize,
    visual_anchor: Option<usize>,
    scroll: usize,
    max_scroll: usize,
    last_viewport_height: usize,
}

impl LedgerPreviewState {
    fn new() -> Self {
        Self {
            identity: PreviewIdentity::None,
            lines: Vec::new(),
            focused: false,
            cursor: 0,
            visual_anchor: None,
            scroll: 0,
            max_scroll: 0,
            last_viewport_height: 0,
        }
    }

    fn has_content(&self) -> bool {
        !self.lines.is_empty()
    }

    fn is_focused(&self) -> bool {
        self.focused
    }

    #[cfg(test)]
    fn is_visual(&self) -> bool {
        self.visual_anchor.is_some()
    }

    #[cfg(test)]
    fn lines_text(&self) -> Vec<&str> {
        self.lines.iter().map(|line| line.text.as_str()).collect()
    }

    fn set_focused(&mut self, focused: bool) {
        if focused && self.lines.is_empty() {
            self.focused = false;
            self.visual_anchor = None;
            return;
        }
        self.focused = focused;
        if !focused {
            self.visual_anchor = None;
        } else {
            self.cursor = self.cursor.min(self.lines.len().saturating_sub(1));
        }
    }

    fn clear_focus(&mut self) {
        self.focused = false;
        self.visual_anchor = None;
    }

    fn sync(&mut self, identity: PreviewIdentity, lines: Vec<PreviewLine>) {
        if self.identity == identity {
            return;
        }
        let same_target = self.identity.same_target(&identity);
        self.identity = identity;
        self.lines = lines;
        // Always drop any active visual range when content changes so stale lines cannot be copied.
        self.visual_anchor = None;
        if !same_target || self.lines.is_empty() {
            self.focused = false;
            self.cursor = 0;
            self.scroll = 0;
            self.max_scroll = 0;
        } else {
            self.cursor = self.cursor.min(self.lines.len().saturating_sub(1));
            self.max_scroll = self.max_scroll.min(self.lines.len().saturating_sub(1));
            self.scroll = self.scroll.min(self.max_scroll);
        }
    }

    fn page_step(&self) -> isize {
        self.last_viewport_height.saturating_sub(2).max(1) as isize
    }

    fn move_cursor(&mut self, delta: isize) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let max_idx = self.lines.len().saturating_sub(1);
        let next = if delta >= 0 {
            self.cursor.saturating_add(delta as usize).min(max_idx)
        } else {
            self.cursor.saturating_sub(delta.unsigned_abs())
        };
        if next == self.cursor {
            return false;
        }
        self.cursor = next;
        true
    }

    fn jump_start(&mut self) -> bool {
        if self.lines.is_empty() || self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn jump_end(&mut self) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let last = self.lines.len().saturating_sub(1);
        if self.cursor == last {
            return false;
        }
        self.cursor = last;
        true
    }

    fn toggle_visual(&mut self) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.visual_anchor = Some(self.cursor);
        }
        true
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        self.visual_anchor
            .map(|anchor| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    fn copy_selection(&self) -> Option<String> {
        if self.lines.is_empty() {
            return None;
        }
        let max_idx = self.lines.len().saturating_sub(1);
        let (lo, hi) = self
            .selected_range()
            .unwrap_or((self.cursor, self.cursor));
        let lo = lo.min(max_idx);
        let hi = hi.min(max_idx);
        let joined = self.lines[lo..=hi]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Some(joined)
    }

    fn copy_all(&self) -> Option<String> {
        if self.lines.is_empty() {
            return None;
        }
        let joined = self
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Some(joined)
    }

    fn handle_esc(&mut self) -> bool {
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
            return true;
        }
        if self.focused {
            self.focused = false;
            return true;
        }
        false
    }

    fn scroll(&mut self, delta: isize) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let cap = if self.last_viewport_height > 0 {
            self.max_scroll
        } else {
            self.lines.len().saturating_sub(1)
        };
        let next = if delta >= 0 {
            self.scroll.saturating_add(delta as usize).min(cap)
        } else {
            self.scroll.saturating_sub(delta.unsigned_abs())
        };
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
    }

    fn inline_lines(&self) -> Vec<self::ledger::InlinePreviewLine> {
        let range = self.selected_range();
        self.lines
            .iter()
            .enumerate()
            .map(|(idx, line)| self::ledger::InlinePreviewLine {
                text: line.text.clone(),
                cursor: self.focused && idx == self.cursor,
                selected: self.focused
                    && range.is_some_and(|(lo, hi)| idx >= lo && idx <= hi),
            })
            .collect()
    }

    fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let header_style = if self.focused {
            Style::default()
                .fg(theme.warning)
                .bg(theme.bg_base)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray_dim).bg(theme.bg_base)
        };
        let header_text = if self.focused {
            if self.visual_anchor.is_some() {
                "PREVIEW · focused · visual"
            } else {
                "PREVIEW · focused"
            }
        } else {
            "PREVIEW · Tab to focus"
        };
        let header = truncate_str(header_text, area.width as usize);
        buf.set_stringn(area.x, area.y, header, area.width as usize, header_style);

        let body_height = area.height.saturating_sub(1) as usize;
        if body_height == 0 {
            self.last_viewport_height = 0;
            self.max_scroll = 0;
            return;
        }
        self.last_viewport_height = body_height;

        let text_width = (area.width as usize).saturating_sub(2).max(1);
        let mut display_rows: Vec<(usize, bool, String)> = Vec::new();
        for (logical_idx, line) in self.lines.iter().enumerate() {
            for (wrap_idx, chunk) in wrap_display_line(&line.text, text_width)
                .into_iter()
                .enumerate()
            {
                display_rows.push((logical_idx, wrap_idx == 0, chunk));
            }
        }

        let total_rows = display_rows.len();
        self.max_scroll = total_rows.saturating_sub(body_height);
        if self.focused {
            let mut cursor_first = None;
            let mut cursor_last = None;
            for (row_idx, (logical_idx, _, _)) in display_rows.iter().enumerate() {
                if *logical_idx == self.cursor {
                    if cursor_first.is_none() {
                        cursor_first = Some(row_idx);
                    }
                    cursor_last = Some(row_idx);
                }
            }
            if let (Some(first), Some(last)) = (cursor_first, cursor_last) {
                if first < self.scroll {
                    self.scroll = first;
                } else if last >= self.scroll + body_height {
                    self.scroll = last.saturating_add(1).saturating_sub(body_height);
                }
            }
        }
        self.scroll = self.scroll.min(self.max_scroll);

        let range = self.selected_range();
        let blank_style = Style::default().fg(theme.text_primary).bg(theme.bg_base);
        for row_offset in 0..body_height {
            let y = area.y + 1 + row_offset as u16;
            let Some((logical_idx, is_first, text)) = display_rows.get(self.scroll + row_offset)
            else {
                buf.set_stringn(
                    area.x,
                    y,
                    " ".repeat(area.width as usize),
                    area.width as usize,
                    blank_style,
                );
                continue;
            };
            let is_cursor = self.focused && *logical_idx == self.cursor;
            let is_selected = self.focused
                && range.is_some_and(|(lo, hi)| *logical_idx >= lo && *logical_idx <= hi);
            let kind = self
                .lines
                .get(*logical_idx)
                .map_or(PreviewLineKind::Body, |line| line.kind);

            let mut line_style = match kind {
                PreviewLineKind::Title => Style::default()
                    .fg(theme.text_primary)
                    .bg(theme.bg_base)
                    .add_modifier(Modifier::BOLD),
                PreviewLineKind::Badge => Style::default().fg(theme.warning).bg(theme.bg_base),
                PreviewLineKind::Section => Style::default()
                    .fg(theme.gray_dim)
                    .bg(theme.bg_base)
                    .add_modifier(Modifier::BOLD),
                PreviewLineKind::Body => Style::default().fg(theme.text_primary).bg(theme.bg_base),
                PreviewLineKind::Diagnostic => Style::default().fg(theme.warning).bg(theme.bg_base),
                PreviewLineKind::Muted => Style::default().fg(theme.gray).bg(theme.bg_base),
            };
            if is_cursor || is_selected {
                line_style = line_style.add_modifier(Modifier::REVERSED);
            }

            let gutter = if is_cursor && *is_first {
                "▸ "
            } else if is_selected {
                "▎ "
            } else {
                "  "
            };
            let gutter_style = if is_cursor || is_selected {
                Style::default()
                    .fg(theme.warning)
                    .bg(theme.bg_base)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.gray_dim).bg(theme.bg_base)
            };

            let gutter_w = area.width.min(2);
            buf.set_stringn(area.x, y, gutter, gutter_w as usize, gutter_style);
            if area.width > 2 {
                let text_x = area.x + 2;
                let avail = (area.width - 2) as usize;
                buf.set_stringn(text_x, y, text, avail, line_style);
                let drawn = text.width().min(avail);
                if drawn < avail {
                    let pad_x = text_x + drawn as u16;
                    let pad_w = avail - drawn;
                    buf.set_stringn(pad_x, y, " ".repeat(pad_w), pad_w, line_style);
                }
            }
        }
    }
}

fn wrap_display_line(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut cur_w = 0usize;
    for ch in text.chars() {
        let mut utf8 = [0u8; 4];
        let ch_str = ch.encode_utf8(&mut utf8);
        let w = ch_str.width();
        if cur_w > 0 && cur_w + w > max_width {
            out.push(std::mem::take(&mut current));
            cur_w = 0;
        }
        current.push(ch);
        cur_w += w;
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

fn build_waiting_group_preview_lines(label: &str, count: usize) -> Vec<PreviewLine> {
    let summary = if count == 1 {
        "1 gate is waiting for input.".to_string()
    } else {
        format!("{count} separate gates are waiting for input.")
    };
    vec![
        PreviewLine::new(PreviewLineKind::Title, format!("{label} · {count} waiting")),
        PreviewLine::new(PreviewLineKind::Body, summary),
        PreviewLine::new(
            PreviewLineKind::Muted,
            "Press Space or Enter to expand and inspect each gate.",
        ),
    ]
}

fn build_run_preview_lines(
    summary: &RunSummary,
    run: Option<&Run>,
    finished_group: bool,
    answered: Option<&str>,
    stalled: bool,
    extra_question: Option<&str>,
) -> Vec<PreviewLine> {
    let title = if !summary.title.trim().is_empty() {
        summary.title.trim()
    } else {
        summary.flow_name.trim()
    };

    let cancelled = run.is_some_and(is_run_cancelled)
        || (run.is_none()
            && summary
                .status_detail
                .as_deref()
                .is_some_and(|d| d.to_ascii_lowercase().contains("cancel")));

    let status_label = if let Some(choice) = answered {
        format!("Answered: {choice}")
    } else if cancelled {
        "Cancelled".to_string()
    } else {
        match summary.status {
            bundle::RunStatus::Waiting => "Waiting for input".to_string(),
            bundle::RunStatus::Running => {
                if stalled {
                    "Running (stalled)".to_string()
                } else {
                    "Running".to_string()
                }
            }
            bundle::RunStatus::Completed => "Completed".to_string(),
            bundle::RunStatus::Failed => "Failed".to_string(),
            bundle::RunStatus::TimedOut => "Timed out".to_string(),
            bundle::RunStatus::Unknown => "Unknown".to_string(),
        }
    };

    let mut lines = vec![PreviewLine::new(
        PreviewLineKind::Title,
        format!("{title} · {status_label}"),
    )];
    if finished_group {
        lines.push(PreviewLine::new(
            PreviewLineKind::Badge,
            "Latest run in finished group (Space to expand)",
        ));
    }

    let Some(run) = run else {
        lines.push(PreviewLine::new(
            PreviewLineKind::Muted,
            "Run details unavailable on disk.",
        ));
        return lines;
    };

    if let Some(choice) = answered {
        lines.push(PreviewLine::new(
            PreviewLineKind::Body,
            format!("Recorded answer: {choice}"),
        ));
        lines.push(PreviewLine::new(
            PreviewLineKind::Muted,
            "Continuation recorded; no input requested.",
        ));
    } else if cancelled {
        if let Some(step) = run.steps.iter().rev().find(|s| {
            crate::acpx::step_digest::effective_outcome(s) == bundle::Outcome::Cancelled
        }) {
            lines.push(PreviewLine::new(
                PreviewLineKind::Body,
                format!("Cancelled at {}", step.attempt_id),
            ));
            for diag in step_diagnostic_lines(step) {
                lines.push(PreviewLine::new(PreviewLineKind::Diagnostic, diag));
            }
        } else {
            lines.push(PreviewLine::new(
                PreviewLineKind::Body,
                "Run was cancelled.",
            ));
        }
    } else {
        match run.summary.status {
            bundle::RunStatus::Waiting => {
                if let Some(question) = waiting_step_question(run, extra_question) {
                    let q_lines = non_empty_trimmed_lines(&question, 4);
                    if let Some((first, rest)) = q_lines.split_first() {
                        lines.push(PreviewLine::new(
                            PreviewLineKind::Body,
                            format!("Question: {first}"),
                        ));
                        for cont in rest {
                            lines.push(PreviewLine::new(PreviewLineKind::Body, cont.clone()));
                        }
                    } else {
                        lines.push(PreviewLine::new(
                            PreviewLineKind::Muted,
                            "Question: no saved gate question recorded",
                        ));
                    }
                } else {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Muted,
                        "Question: no saved gate question recorded",
                    ));
                }

                let choices = bundle::gate_choices(run);
                if choices.is_empty() {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Muted,
                        "Choices: none recorded",
                    ));
                } else {
                    let labels: Vec<&str> = choices.iter().map(|(label, _)| label.as_str()).collect();
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Body,
                        format!("Choices: {}", labels.join(", ")),
                    ));
                }
            }
            bundle::RunStatus::Running => {
                if let Some(node) = run
                    .summary
                    .current_node
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Body,
                        format!("Current step: {node}"),
                    ));
                } else if let Some(last) = run.steps.last() {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Body,
                        format!("Latest step: {}", last.node_id),
                    ));
                } else {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Muted,
                        "Current step: not yet recorded",
                    ));
                }
                if stalled {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Badge,
                        "Stalled: no recent progress recorded",
                    ));
                }
                if let Some(detail) = run
                    .summary
                    .status_detail
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    lines.push(PreviewLine::new(PreviewLineKind::Muted, detail));
                }
            }
            bundle::RunStatus::Failed | bundle::RunStatus::TimedOut => {
                append_failure_evidence(&mut lines, run);
            }
            bundle::RunStatus::Completed => {
                let result_lines = completed_result_lines(run);
                if result_lines.is_empty() {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Muted,
                        "Result: no saved result recorded",
                    ));
                } else {
                    lines.push(PreviewLine::new(PreviewLineKind::Section, "Result"));
                    for line in result_lines {
                        lines.push(PreviewLine::new(PreviewLineKind::Body, line));
                    }
                }
            }
            bundle::RunStatus::Unknown => {
                if let Some(detail) = run
                    .summary
                    .status_detail
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    lines.push(PreviewLine::new(PreviewLineKind::Body, detail));
                } else {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Muted,
                        "No status evidence recorded.",
                    ));
                }
            }
        }
    }

    lines.push(PreviewLine::new(PreviewLineKind::Section, "Executed path"));
    if let Some(path) = executed_path_line(run) {
        lines.push(PreviewLine::new(PreviewLineKind::Body, path));
    } else {
        lines.push(PreviewLine::new(
            PreviewLineKind::Muted,
            "No steps executed yet",
        ));
    }

    lines
}

fn is_run_cancelled(run: &Run) -> bool {
    if matches!(
        run.summary.status,
        bundle::RunStatus::Running | bundle::RunStatus::Waiting
    ) {
        return false;
    }
    run.steps.last().is_some_and(|step| {
        crate::acpx::step_digest::effective_outcome(step) == bundle::Outcome::Cancelled
    }) || run
        .summary
        .status_detail
        .as_deref()
        .is_some_and(|d| d.to_ascii_lowercase().contains("cancel"))
}

fn append_failure_evidence(lines: &mut Vec<PreviewLine>, run: &Run) {
    let failed_step = run.steps.iter().rev().find(|step| {
        matches!(
            crate::acpx::step_digest::effective_outcome(step),
            bundle::Outcome::Failed | bundle::Outcome::TimedOut
        )
    });

    if let Some(step) = failed_step {
        let outcome = crate::acpx::step_digest::effective_outcome(step);
        let verb = if outcome == bundle::Outcome::TimedOut {
            "Timed out at"
        } else {
            "Failed at"
        };
        let attempt = if !step.attempt_id.trim().is_empty() {
            step.attempt_id.as_str()
        } else {
            step.node_id.as_str()
        };
        let header = match crate::acpx::step_digest::non_zero_exit_code(step) {
            Some(code) => format!("{verb} {attempt} (exit {code})"),
            None => format!("{verb} {attempt}"),
        };
        lines.push(PreviewLine::new(PreviewLineKind::Body, header));

        let diag_lines = step_diagnostic_lines(step);
        if diag_lines.is_empty() {
            if let Some(err) = run.summary.error.as_deref() {
                let err_lines = diagnostic_lines_from_text(err, 4);
                if err_lines.is_empty() {
                    lines.push(PreviewLine::new(
                        PreviewLineKind::Muted,
                        "Diagnostic: no saved diagnostic recorded",
                    ));
                } else {
                    for diag in err_lines {
                        lines.push(PreviewLine::new(PreviewLineKind::Diagnostic, diag));
                    }
                }
            } else {
                lines.push(PreviewLine::new(
                    PreviewLineKind::Muted,
                    "Diagnostic: no saved diagnostic recorded",
                ));
            }
        } else {
            for diag in diag_lines {
                lines.push(PreviewLine::new(PreviewLineKind::Diagnostic, diag));
            }
        }
        return;
    }

    let node = run
        .summary
        .current_node
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(run.flow.name.as_str());
    let verb = if run.summary.status == bundle::RunStatus::TimedOut {
        "Timed out at"
    } else {
        "Failed at"
    };
    lines.push(PreviewLine::new(
        PreviewLineKind::Body,
        format!("{verb} {node}"),
    ));
    if let Some(err) = run.summary.error.as_deref() {
        let err_lines = diagnostic_lines_from_text(err, 4);
        if err_lines.is_empty() {
            lines.push(PreviewLine::new(
                PreviewLineKind::Muted,
                "Diagnostic: no saved diagnostic recorded",
            ));
        } else {
            for diag in err_lines {
                lines.push(PreviewLine::new(PreviewLineKind::Diagnostic, diag));
            }
        }
    } else {
        lines.push(PreviewLine::new(
            PreviewLineKind::Muted,
            "Diagnostic: no saved diagnostic recorded",
        ));
    }
}

fn step_diagnostic_lines(step: &bundle::Step) -> Vec<String> {
    const MAX_DIAGNOSTIC_LINES: usize = 4;
    let error = step
        .error
        .as_deref()
        .or_else(|| step.output.get("error").and_then(serde_json::Value::as_str));
    let stderr = step.output.get("stderr").and_then(serde_json::Value::as_str);
    let output_tail = step
        .output
        .get("outputTail")
        .or_else(|| step.output.get("output_tail"))
        .or_else(|| step.output.get("tail"))
        .or_else(|| step.output.get("message"))
        .and_then(serde_json::Value::as_str)
        .or(step.raw_text.as_deref());
    let stdout = step.output.get("stdout").and_then(serde_json::Value::as_str);

    for source in [error, stderr, output_tail, stdout].into_iter().flatten() {
        let lines = diagnostic_lines_from_text(source, MAX_DIAGNOSTIC_LINES);
        if !lines.is_empty() {
            return lines;
        }
    }
    Vec::new()
}

fn diagnostic_lines_from_text(text: &str, max_lines: usize) -> Vec<String> {
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end_matches('\r').trim_end().to_string())
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() <= max_lines {
        return lines;
    }
    if let Some(err_idx) = lines
        .iter()
        .position(|line| line.trim_start().to_ascii_lowercase().starts_with("error"))
    {
        let end = (err_idx + max_lines).min(lines.len());
        return lines[err_idx..end].to_vec();
    }
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].to_vec()
}

fn waiting_step_question(run: &Run, extra_question: Option<&str>) -> Option<String> {
    if let Some(q) = extra_question.map(str::trim).filter(|q| !q.is_empty()) {
        return Some(q.to_string());
    }
    if let Some(node) = run.summary.waiting_on.as_deref()
        && let Some(step) = run.steps.iter().rev().find(|s| s.node_id == node)
    {
        for key in ["question", "prompt", "summary", "message"] {
            if let Some(s) = step
                .output
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(s.to_string());
            }
        }
        if let Some(s) = step
            .output
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    run.summary
        .status_detail
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn completed_result_lines(run: &Run) -> Vec<String> {
    const MAX_RESULT_LINES: usize = 3;
    for step in run.steps.iter().rev() {
        let lines = extract_plain_value_lines(&step.output, MAX_RESULT_LINES);
        if !lines.is_empty() {
            return lines;
        }
        if let Some(raw) = step.raw_text.as_deref() {
            let lines = non_empty_trimmed_lines(raw, MAX_RESULT_LINES);
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    Vec::new()
}

fn extract_plain_value_lines(value: &serde_json::Value, max_lines: usize) -> Vec<String> {
    match value {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::String(s) => non_empty_trimmed_lines(s, max_lines),
        serde_json::Value::Bool(b) => vec![b.to_string()],
        serde_json::Value::Number(n) => vec![n.to_string()],
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if out.len() >= max_lines {
                    break;
                }
                match item {
                    serde_json::Value::String(s) if !s.trim().is_empty() => {
                        out.push(s.trim().to_string());
                    }
                    serde_json::Value::Bool(b) => out.push(b.to_string()),
                    serde_json::Value::Number(n) => out.push(n.to_string()),
                    _ => {}
                }
            }
            out
        }
        serde_json::Value::Object(map) => {
            for key in ["summary", "result", "message", "text", "answer", "output"] {
                if let Some(serde_json::Value::String(s)) = map.get(key) {
                    let lines = non_empty_trimmed_lines(s, max_lines);
                    if !lines.is_empty() {
                        return lines;
                    }
                }
            }
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Vec::new();
            for key in keys {
                if out.len() >= max_lines {
                    break;
                }
                let formatted = match &map[key] {
                    serde_json::Value::String(s) if !s.trim().is_empty() => {
                        let first = s
                            .lines()
                            .find(|line| !line.trim().is_empty())
                            .unwrap_or("")
                            .trim();
                        Some(format!("{key}: {first}"))
                    }
                    serde_json::Value::Bool(b) => Some(format!("{key}: {b}")),
                    serde_json::Value::Number(n) => Some(format!("{key}: {n}")),
                    _ => None,
                };
                if let Some(line) = formatted {
                    out.push(line);
                }
            }
            out
        }
    }
}

fn non_empty_trimmed_lines(text: &str, max_lines: usize) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_end_matches('\r').trim_end().to_string())
        .filter(|line| !line.trim().is_empty())
        .take(max_lines)
        .collect()
}

fn executed_path_line(run: &Run) -> Option<String> {
    if run.steps.is_empty() {
        return None;
    }
    let parts: Vec<String> = run
        .steps
        .iter()
        .map(|step| {
            let outcome = crate::acpx::step_digest::effective_outcome(step);
            let glyph = bundle::NodeState::Done(outcome).glyph();
            format!("{glyph} {}", step.node_id)
        })
        .collect();
    Some(parts.join(" → "))
}

const LEDGER_KEYS: &[(&str, &str)] = &[
    (
        "Tab / Shift+Tab",
        "Focus or leave the selected run preview (Flows); switch tabs when empty or on Agent Turns",
    ),
    ("h / l / ← / →", "Switch between Flows and Agent Turns"),
    ("j / k / ↑ / ↓", "Move selection"),
    ("g / G", "Jump to first / last row"),
    ("J / K", "Scroll the preview of the selected run"),
    ("Preview: j / k, PgUp / PgDn", "Move the preview cursor by a row / a page"),
    ("Preview: v, y / Y", "Select line range; copy exact line/range or whole preview"),
    ("Enter", "Open selected flow run, expand waiting group, or replay agent turn"),
    ("Space", "Expand or collapse a waiting or finished-run group (Flows)"),
    ("/", "Filter rows by text"),
    ("a", "Only running and waiting runs"),
    ("s", "Cycle order: grouped, recent, name"),
    ("y", "Copy selected id or path"),
    ("R", "Rescan flows and agent turns on disk"),
    ("?", "Toggle this key reference"),
    ("Esc", "End preview selection / focus, clear filter, or close modal"),
];
~~~

## Edit 9: Update session replay test to switch tabs with l from Flows with a preview
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    #[test]
    fn opening_session_in_ledger_returns_replay_with_none_range() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        // Switch to Sessions tab in Ledger via h/l (Tab in Flows focuses the run preview).
        state.handle_key(&key(KeyCode::Char('l')));
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
~~~

## Edit 10: Add opening-preview tests covering diagnostics, groups, widths, copy, Esc, refresh, tabs, filter, and safety
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
    #[test]
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
~~~
NEW:
~~~rust
    #[test]
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

    fn write_custom_run(
        runs_base: &Path,
        run_id: &str,
        live_json: &str,
        flow_json: &str,
        steps_json: &str,
        run_json: Option<&str>,
    ) -> PathBuf {
        let run_dir = runs_base.join(run_id);
        let proj_dir = run_dir.join("projections");
        fs::create_dir_all(&proj_dir).expect("create projections dir");
        fs::write(proj_dir.join("live.json"), live_json).expect("write live.json");
        fs::write(run_dir.join("flow.json"), flow_json).expect("write flow.json");
        fs::write(proj_dir.join("steps.json"), steps_json).expect("write steps.json");
        if let Some(rj) = run_json {
            fs::write(proj_dir.join("run.json"), rj).expect("write run.json");
        }
        run_dir
    }

    #[test]
    fn failed_cargo_check_preview_copies_exact_multiline_diagnostic_wide_and_narrow() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_base = tmp.path().join("runs");
        let sessions_base = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_base).expect("sessions dir");

        let exact_err = "error[E0308]: mismatched types in acpx_modal opening preview diagnostic line";
        let exact_loc = "  --> crates/codegen/xai-grok-pager/src/views/acpx_modal.rs:42:9";
        let stderr = format!("Compiling xai-grok-pager v0.1.0\n{exact_err}\n{exact_loc}\n   |\nerror: aborting due to 1 previous error");

        let live_json = r#"{
            "runId": "run-fail",
            "flowName": "beta8-merge",
            "runTitle": "beta8-merge",
            "status": "failed",
            "startedAt": "2025-03-01T10:00:00Z",
            "updatedAt": "2025-03-01T10:05:00Z",
            "currentNode": "cargo_check"
        }"#;
        let flow_json = r#"{
            "name": "beta8-merge",
            "startAt": "prepare",
            "nodes": {
                "prepare": { "nodeType": "action" },
                "cargo_check": { "nodeType": "action" },
                "never_ran": { "nodeType": "action" }
            },
            "edges": [
                { "from": "prepare", "to": "cargo_check" },
                { "from": "cargo_check", "to": "never_ran" }
            ]
        }"#;
        let steps_json = serde_json::json!([
            {
                "attemptId": "prepare#1",
                "nodeId": "prepare",
                "nodeType": "action",
                "outcome": "ok",
                "startedAt": "2025-03-01T10:00:00Z",
                "finishedAt": "2025-03-01T10:01:00Z",
                "output": "ok"
            },
            {
                "attemptId": "cargo_check#1",
                "nodeId": "cargo_check",
                "nodeType": "action",
                "outcome": "failed",
                "startedAt": "2025-03-01T10:01:01Z",
                "finishedAt": "2025-03-01T10:05:00Z",
                "output": { "stderr": stderr },
                "trace": {
                    "action": {
                        "actionType": "shell",
                        "command": "cargo",
                        "args": ["check"],
                        "exitCode": 101,
                        "durationMs": 4200
                    }
                }
            }
        ])
        .to_string();
        write_custom_run(&runs_base, "run-fail", live_json, flow_json, &steps_json, None);

        let mut state = AcpxModalState::open_at(Some(runs_base), Some(sessions_base));

        // Wide layout renders side preview and hides unrun nodes.
        let wide = Rect::new(0, 0, 140, 30);
        let mut wide_buf = Buffer::empty(wide);
        render_acpx_modal(&mut wide_buf, wide, &mut state, false);
        let wide_text = buffer_text(&wide_buf);
        assert!(wide_text.contains("beta8-merge · Failed"));
        assert!(wide_text.contains("cargo_check#1 (exit 101)"));
        assert!(!wide_text.contains("never_ran"));
        assert!(state.detail_rect.is_some());

        // Focus preview with Tab, verify wants_esc and footer shortcuts.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        assert!(state.ledger_preview.is_focused());
        assert!(state.wants_esc());
        let shortcuts = state.active_shortcut_labels();
        assert!(shortcuts.contains(&"Esc list"));

        // Move to exact diagnostic line (line 2: 0=title, 1=Failed at..., 2=exact_err).
        assert_eq!(state.handle_key(&key(KeyCode::Char('j'))), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Char('j'))), AcpxOutcome::Changed);
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            AcpxOutcome::Copy(exact_err.to_string())
        );

        // Visual range across two exact diagnostic lines.
        assert_eq!(state.handle_key(&key(KeyCode::Char('v'))), AcpxOutcome::Changed);
        assert!(state.ledger_preview.is_visual());
        assert_eq!(state.handle_key(&key(KeyCode::Char('j'))), AcpxOutcome::Changed);
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            AcpxOutcome::Copy(format!("{exact_err}\n{exact_loc}"))
        );

        // Y copies all logical lines without render wraps or indentation.
        match state.handle_key(&key(KeyCode::Char('Y'))) {
            AcpxOutcome::Copy(all) => {
                assert!(all.contains(exact_err));
                assert!(all.contains(exact_loc));
                assert!(all.contains("✓ prepare → ✗ cargo_check"));
                assert!(!all.contains("never_ran"));
            }
            other => panic!("expected Copy, got {other:?}"),
        }

        // First Esc clears visual range while staying focused; second Esc returns to list.
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert!(state.ledger_preview.is_focused());
        assert!(!state.ledger_preview.is_visual());
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert!(!state.ledger_preview.is_focused());
        assert!(!state.wants_esc());

        // Narrow layout wraps for display but copies the exact same unwrapped diagnostic line.
        let narrow = Rect::new(0, 0, 72, 26);
        let mut narrow_buf = Buffer::empty(narrow);
        render_acpx_modal(&mut narrow_buf, narrow, &mut state, false);
        assert!(state.detail_rect.is_none());
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        render_acpx_modal(&mut narrow_buf, narrow, &mut state, false);
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            AcpxOutcome::Copy(exact_err.to_string())
        );
    }

    #[test]
    fn waiting_group_and_individual_wait_and_answered_wait_preview_evidence() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_base = tmp.path().join("runs");
        let sessions_base = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_base).expect("sessions dir");

        let flow_json = r#"{
            "name": "ship-slugify",
            "startAt": "gate",
            "nodes": { "gate": { "nodeType": "checkpoint" } },
            "edges": []
        }"#;

        for (id, updated, question) in [
            ("wait-1", "2025-03-01T10:02:00Z", "Ship release candidate 1?"),
            ("wait-2", "2025-03-01T10:01:00Z", "Ship release candidate 2?"),
            ("wait-answered", "2025-03-01T10:00:00Z", "Old gate question?"),
        ] {
            let live = serde_json::json!({
                "runId": id,
                "flowName": "ship-slugify",
                "runTitle": format!("ship: {id}"),
                "status": "waiting",
                "startedAt": "2025-03-01T09:50:00Z",
                "updatedAt": updated,
                "waitingOn": "gate"
            })
            .to_string();
            let steps = serde_json::json!([{
                "attemptId": "gate#1",
                "nodeId": "gate",
                "nodeType": "checkpoint",
                "outcome": "ok",
                "startedAt": "2025-03-01T09:50:00Z",
                "finishedAt": updated,
                "output": {
                    "question": question,
                    "next": { "approve": "next.flow.ts", "reject": "stop.flow.ts" }
                }
            }])
            .to_string();
            write_custom_run(&runs_base, id, &live, flow_json, &steps, None);
        }

        // Continuation run answering `wait-answered`.
        let cont_live = r#"{
            "runId": "cont-1",
            "flowName": "next-flow",
            "runTitle": "next-flow",
            "status": "completed",
            "startedAt": "2025-03-01T10:03:00Z",
            "updatedAt": "2025-03-01T10:04:00Z"
        }"#;
        let cont_flow = r#"{
            "name": "next-flow",
            "startAt": "done",
            "nodes": { "done": { "nodeType": "action" } },
            "edges": []
        }"#;
        let cont_steps = r#"[{
            "attemptId": "done#1",
            "nodeId": "done",
            "nodeType": "action",
            "outcome": "ok",
            "startedAt": "2025-03-01T10:03:00Z",
            "finishedAt": "2025-03-01T10:04:00Z",
            "output": "deployed"
        }]"#;
        let cont_run_json = r#"{
            "input": { "gate": { "runId": "wait-answered", "node": "gate", "choice": "approve" } }
        }"#;
        write_custom_run(
            &runs_base,
            "cont-1",
            cont_live,
            cont_flow,
            cont_steps,
            Some(cont_run_json),
        );

        let mut state = AcpxModalState::open_at(Some(runs_base), Some(sessions_base));

        // First selected item is the folded waiting group of 2 (answered wait excluded).
        let group_lines = state.ledger_preview.lines_text().join("\n");
        assert!(group_lines.contains("2 waiting"), "got:\n{group_lines}");
        assert!(group_lines.contains("Space"), "got:\n{group_lines}");
        assert!(!group_lines.contains("Ship release candidate"));
        assert!(!group_lines.contains("approve"));

        // Expand waiting group with Space, move to first member, inspect plain question/choices.
        assert_eq!(state.handle_key(&key(KeyCode::Char(' '))), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Char('j'))), AcpxOutcome::Changed);
        let member_lines = state.ledger_preview.lines_text().join("\n");
        assert!(member_lines.contains("Question: Ship release candidate 1?"));
        assert!(member_lines.contains("Choices: approve, reject"));
        assert!(!member_lines.contains("[1]"));

        // Focusing preview and pressing workflow-control keys produces no control outcomes.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        for code in [
            KeyCode::Enter,
            KeyCode::Char('1'),
            KeyCode::Char(' '),
            KeyCode::Char('R'),
            KeyCode::Char('a'),
            KeyCode::Char('s'),
        ] {
            assert_eq!(state.handle_key(&key(code)), AcpxOutcome::Unchanged);
        }
        assert_eq!(state.mode(), Mode::Ledger);
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);

        // Navigate to the answered native ACPX wait and verify it shows history, not a prompt.
        let mut found_answered = false;
        for _ in 0..8 {
            if state
                .ledger
                .selected_run()
                .is_some_and(|r| r.run_id == "wait-answered")
            {
                found_answered = true;
                break;
            }
            state.handle_key(&key(KeyCode::Char('j')));
        }
        assert!(found_answered, "expected wait-answered to be selectable");
        let answered_lines = state.ledger_preview.lines_text().join("\n");
        assert!(answered_lines.contains("Answered: approve"), "got:\n{answered_lines}");
        assert!(answered_lines.contains("Recorded answer: approve"));
        assert!(!answered_lines.contains("Choices:"));
    }

    #[test]
    fn running_finished_group_cancelled_retries_and_missing_evidence_summaries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_base = tmp.path().join("runs");
        let sessions_base = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_base).expect("sessions dir");

        let flow_json = r#"{
            "name": "mixed-talk",
            "startAt": "check",
            "nodes": {
                "check": { "nodeType": "action" },
                "ship": { "nodeType": "action" },
                "unrun": { "nodeType": "action" }
            },
            "edges": [
                { "from": "check", "to": "ship" },
                { "from": "ship", "to": "unrun" }
            ]
        }"#;

        // Running run
        write_custom_run(
            &runs_base,
            "run-live",
            r#"{
                "runId": "run-live",
                "flowName": "live-flow",
                "runTitle": "Live Build",
                "status": "running",
                "startedAt": "2025-03-01T10:10:00Z",
                "updatedAt": "2025-03-01T10:11:00Z",
                "currentNode": "ship"
            }"#,
            flow_json,
            r#"[{
                "attemptId": "check#1",
                "nodeId": "check",
                "nodeType": "action",
                "outcome": "ok",
                "startedAt": "2025-03-01T10:10:00Z",
                "finishedAt": "2025-03-01T10:10:30Z",
                "output": "ok"
            }]"#,
            None,
        );

        // Two finished runs in same flow (forms a finished group) with retries in recorded order.
        write_custom_run(
            &runs_base,
            "fin-new",
            r#"{
                "runId": "fin-new",
                "flowName": "mixed-talk",
                "runTitle": "Mixed Talk Latest",
                "status": "completed",
                "startedAt": "2025-03-01T10:05:00Z",
                "updatedAt": "2025-03-01T10:06:00Z",
                "finishedAt": "2025-03-01T10:06:00Z"
            }"#,
            flow_json,
            r#"[
                {
                    "attemptId": "check#1",
                    "nodeId": "check",
                    "nodeType": "action",
                    "outcome": "failed",
                    "startedAt": "2025-03-01T10:05:00Z",
                    "finishedAt": "2025-03-01T10:05:10Z",
                    "output": {}
                },
                {
                    "attemptId": "check#2",
                    "nodeId": "check",
                    "nodeType": "action",
                    "outcome": "ok",
                    "startedAt": "2025-03-01T10:05:11Z",
                    "finishedAt": "2025-03-01T10:05:20Z",
                    "output": { "passed": true }
                },
                {
                    "attemptId": "ship#1",
                    "nodeId": "ship",
                    "nodeType": "action",
                    "outcome": "ok",
                    "startedAt": "2025-03-01T10:05:21Z",
                    "finishedAt": "2025-03-01T10:06:00Z",
                    "output": { "passed": true }
                }
            ]"#,
            None,
        );
        write_custom_run(
            &runs_base,
            "fin-old",
            r#"{
                "runId": "fin-old",
                "flowName": "mixed-talk",
                "runTitle": "Mixed Talk Older",
                "status": "completed",
                "startedAt": "2025-03-01T09:00:00Z",
                "updatedAt": "2025-03-01T09:01:00Z",
                "finishedAt": "2025-03-01T09:01:00Z"
            }"#,
            flow_json,
            r#"[]"#,
            None,
        );

        // Cancelled run
        write_custom_run(
            &runs_base,
            "run-cancel",
            r#"{
                "runId": "run-cancel",
                "flowName": "cancel-flow",
                "runTitle": "Cancelled Run",
                "status": "failed",
                "startedAt": "2025-03-01T08:00:00Z",
                "updatedAt": "2025-03-01T08:01:00Z"
            }"#,
            flow_json,
            r#"[{
                "attemptId": "check#1",
                "nodeId": "check",
                "nodeType": "action",
                "outcome": "cancelled",
                "startedAt": "2025-03-01T08:00:00Z",
                "finishedAt": "2025-03-01T08:01:00Z",
                "output": {}
            }]"#,
            None,
        );

        // Missing diagnostic failure
        write_custom_run(
            &runs_base,
            "run-nodata",
            r#"{
                "runId": "run-nodata",
                "flowName": "empty-fail",
                "runTitle": "Empty Failure",
                "status": "failed",
                "startedAt": "2025-03-01T07:00:00Z",
                "updatedAt": "2025-03-01T07:01:00Z"
            }"#,
            flow_json,
            r#"[{
                "attemptId": "check#1",
                "nodeId": "check",
                "nodeType": "action",
                "outcome": "failed",
                "startedAt": "2025-03-01T07:00:00Z",
                "finishedAt": "2025-03-01T07:01:00Z",
                "output": {}
            }]"#,
            None,
        );

        let mut state = AcpxModalState::open_at(Some(runs_base), Some(sessions_base));

        // 1. Running run first
        assert_eq!(
            state.ledger.selected_run().map(|r| r.run_id.as_str()),
            Some("run-live")
        );
        let live_text = state.ledger_preview.lines_text().join("\n");
        assert!(live_text.contains("Current step: ship"));

        // 2. Finished group representative next
        state.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(
            state.ledger.selected_run().map(|r| r.run_id.as_str()),
            Some("fin-new")
        );
        let fin_text = state.ledger_preview.lines_text().join("\n");
        assert!(fin_text.contains("Latest run in finished group"));
        assert!(fin_text.contains("passed: true"));
        assert!(!fin_text.contains('{'));
        assert!(fin_text.contains("✗ check → ✓ check → ✓ ship"));
        assert!(!fin_text.contains("unrun"));

        // 3. Cancelled run stays cancelled
        state.handle_key(&key(KeyCode::Char('j')));
        let cancel_text = state.ledger_preview.lines_text().join("\n");
        assert!(cancel_text.contains("Cancelled"));
        assert!(cancel_text.contains("Cancelled at check#1"));

        // 4. Missing diagnostic stated plainly
        state.handle_key(&key(KeyCode::Char('j')));
        let nodata_text = state.ledger_preview.lines_text().join("\n");
        assert!(nodata_text.contains("no saved diagnostic recorded"));
    }

    #[test]
    fn preview_refresh_open_back_tab_reachability_filter_and_empty_layouts() {
        let fx = write_fixture("running");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));

        // Focus preview and start a visual range; refresh updates content and clears stale range.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Char('v'))), AcpxOutcome::Changed);
        assert!(state.ledger_preview.is_visual());

        let updated_live = r#"{
            "runId": "run-001",
            "flowName": "deploy-flow",
            "runTitle": "Deploy Production",
            "status": "completed",
            "startedAt": "2025-03-01T10:00:00Z",
            "updatedAt": "2025-03-01T10:09:00Z",
            "finishedAt": "2025-03-01T10:09:00Z"
        }"#;
        fs::write(fx.run_dir.join("projections/live.json"), updated_live).expect("update live");
        assert!(state.poll());
        assert!(!state.ledger_preview.is_visual());
        assert!(state
            .ledger_preview
            .lines_text()
            .join("\n")
            .contains("Completed"));

        // Unfocus preview, open run, press Esc -> returns to same row in Ledger with preview unfocused.
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Enter)), AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Run);
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Ledger);
        assert!(!state.ledger_preview.is_focused());
        assert_eq!(
            state.ledger.selected_run().map(|r| r.run_id.as_str()),
            Some("run-001")
        );

        // Filter capture stays above preview input.
        assert_eq!(state.handle_key(&key(KeyCode::Char('/'))), AcpxOutcome::Changed);
        assert!(state.ledger.captures_text());
        assert_eq!(state.handle_key(&key(KeyCode::Char('z'))), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Char('z'))), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Enter)), AcpxOutcome::Changed);
        assert!(!state.ledger.captures_text());
        // With no matches, preview is empty and Tab still reaches Agent Turns.
        assert!(!state.ledger_preview.has_content());
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        assert_eq!(state.ledger.tab(), self::ledger::LedgerTab::Sessions);
        // Agent Turns keeps its existing Tab route back to Flows.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        assert_eq!(state.ledger.tab(), self::ledger::LedgerTab::Flows);

        // Empty + tiny renders do not panic.
        for (w, h) in [(0, 0), (10, 3), (50, 12), (140, 30)] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            render_acpx_modal(&mut buf, area, &mut state, false);
        }
    }
~~~
