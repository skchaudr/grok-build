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

=== REPOSITORY CONTEXT (10 excerpts) ===

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 57-223 of 1366 (repository grok-build)
````
struct TabEntry {
    kind: TabKind,
    text: String,
    is_empty_artifact: bool,
    scroll_y: usize,
    scroll_x: usize,
    /// What `y` copies when it differs from what the tab shows (a JSON value shown as rows copies its raw JSON).
    copy: Option<String>,
}

pub struct InspectorState {
    step: Step,
    node: Option<FlowNode>,
    position: (usize, usize),
    tabs: Vec<TabEntry>,
    active_tab: usize,
    wrap: bool,
    last_body_height: usize,
    last_body_width: usize,
}

impl InspectorState {
    /// `position` is (1-based index of this step, total steps) for the header.
    pub fn new(
        run_dir: &Path,
        step: Step,
        node: Option<FlowNode>,
        position: (usize, usize),
    ) -> Self {
        let mut tabs = Vec::new();

        // 1. Output
        let output_val_text = value_text(&step.output);
        if !output_val_text.is_empty() {
            // A JSON value reads as rows, not braces and quotes; `y` still copies the raw JSON.
            let (text, copy) = if step.output.is_string() {
                (output_val_text, None)
            } else {
                (
                    crate::views::acpx_modal::detail::value_plain_text(&step.output),
                    Some(output_val_text),
                )
            };
            tabs.push(TabEntry {
                kind: TabKind::Output,
                text,
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy,
            });
        } else if let Some(art) = &step.output_artifact {
            let (text, is_empty) = read_artifact_tab(run_dir, art);
            tabs.push(TabEntry {
                kind: TabKind::Output,
                text,
                is_empty_artifact: is_empty,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 2. Prompt
        if let Some(prompt) = &step.prompt_text
            && !prompt.is_empty()
        {
            tabs.push(TabEntry {
                kind: TabKind::Prompt,
                text: prompt.clone(),
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 3. Response
        if let Some(raw) = &step.raw_text
            && !raw.is_empty()
        {
            tabs.push(TabEntry {
                kind: TabKind::Response,
                text: raw.clone(),
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 4. Stdout
        if let Some(art) = &step.stdout {
            let (text, is_empty) = read_artifact_tab(run_dir, art);
            tabs.push(TabEntry {
                kind: TabKind::Stdout,
                text,
                is_empty_artifact: is_empty,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 5. Stderr
        if let Some(art) = &step.stderr {
            let (text, is_empty) = read_artifact_tab(run_dir, art);
            tabs.push(TabEntry {
                kind: TabKind::Stderr,
                text,
                is_empty_artifact: is_empty,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 6. Error
        if let Some(err) = &step.error
            && !err.is_empty()
        {
            tabs.push(TabEntry {
                kind: TabKind::Error,
                text: err.clone(),
                is_empty_artifact: false,
                scroll_y: 0,
                scroll_x: 0,
                copy: None,
            });
        }

        // 7. Meta
        let meta_text = build_meta_text(&step, node.as_ref());
        tabs.push(TabEntry {
            kind: TabKind::Meta,
            text: meta_text,
            is_empty_artifact: false,
            scroll_y: 0,
            scroll_x: 0,
            copy: None,
        });

        let is_failed = matches!(step.outcome, Outcome::Failed | Outcome::TimedOut);
        let preferred = if is_failed {
            TabKind::Error
        } else {
            TabKind::Output
        };

        let active_tab = tabs
            .iter()
            .position(|t| t.kind == preferred)
            .or_else(|| tabs.iter().position(|t| t.kind == TabKind::Output))
            .unwrap_or(0);

        Self {
            step,
            node,
            position,
            tabs,
            active_tab,
            wrap: true,
            last_body_height: 20,
            last_body_width: 80,
        }
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 224-450 of 1366 (repository grok-build)
````
    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
        if key.kind == KeyEventKind::Release {
            return InspectorOutcome::Unchanged;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => InspectorOutcome::Back,
            KeyCode::Tab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::BackTab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char(c @ '1'..='7') if !ctrl => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.tabs.len() && idx != self.active_tab {
                    self.active_tab = idx;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                if self.scroll(1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                if self.scroll(-1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('g') | KeyCode::Home if !ctrl => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y != 0
                {
                    tab.scroll_y = 0;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                let max_scroll = self.max_scroll_y();
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y != max_scroll
                {
                    tab.scroll_y = max_scroll;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Left | KeyCode::Char('h') if !ctrl => {
                if self.wrap {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_x > 0
                {
                    tab.scroll_x = tab.scroll_x.saturating_sub(HORIZ_SCROLL_STEP);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Right | KeyCode::Char('l') if !ctrl => {
                if self.wrap {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.scroll_x = tab.scroll_x.saturating_add(HORIZ_SCROLL_STEP);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('w') if !ctrl => {
                self.wrap = !self.wrap;
                let max_scroll = self.max_scroll_y();
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y > max_scroll
                {
                    tab.scroll_y = max_scroll;
                }
                InspectorOutcome::Changed
            }
            KeyCode::Char('y') if !ctrl => {
                let text = self
                    .tabs
                    .get(self.active_tab)
                    .map(|t| t.copy.clone().unwrap_or_else(|| t.text.clone()))
                    .unwrap_or_default();
                InspectorOutcome::Copy(text)
            }
            KeyCode::Char('r') if !ctrl => {
                if self.step.conversation.is_some() {
                    InspectorOutcome::Replay
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('[') if !ctrl => InspectorOutcome::Prev,
            KeyCode::Char(']') if !ctrl => InspectorOutcome::Next,
            _ => InspectorOutcome::Unchanged,
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        let max_scroll = self.max_scroll_y();
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let cur = tab.scroll_y.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            (cur + delta as usize).min(max_scroll)
        };
        if next != tab.scroll_y {
            tab.scroll_y = next;
            true
        } else {
            false
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, _now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Row 0: Header
        self.render_header(buf, Rect::new(area.x, area.y, area.width, 1), theme);

        if area.height < 2 {
            return;
        }

        // Row 1: Tabs
        self.render_tabs(buf, Rect::new(area.x, area.y + 1, area.width, 1), theme);

        if area.height < 3 {
            return;
        }

        // Rest: Body
        let body_area = Rect::new(area.x, area.y + 2, area.width, area.height - 2);
        self.last_body_height = body_area.height as usize;
        self.last_body_width = body_area.width as usize;
        self.render_body(buf, body_area, theme);
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut s = vec![
            "Tab/1-7: tab",
            "j/k: scroll",
            "w: wrap",
            "y: copy",
            "[/]: step",
        ];
        if self.step.conversation.is_some() {
            s.push("r: replay");
        }
        s.push("Esc: back");
        s
    }

    fn max_scroll_y(&self) -> usize {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return 0;
        };
        if tab.is_empty_artifact {
            return 0;
        }
        let (_, total_visual) = self.compute_visual_lines(tab, self.last_body_width);
        total_visual.saturating_sub(self.last_body_height.max(1))
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 451-489 of 1366 (repository grok-build)
````
    fn compute_visual_lines(
        &self,
        tab: &TabEntry,
        area_width: usize,
    ) -> (Vec<VisualLine>, usize) {
        let src_lines: Vec<&str> = if tab.text.is_empty() {
            vec![""]
        } else {
            tab.text.lines().collect()
        };
        let total_src = src_lines.len().max(1);
        let digits = decimal_digits(total_src).max(2);
        let gutter_width = digits + 2; // "<num>  "
        let content_width = area_width.saturating_sub(gutter_width).max(1);

        let mut out = Vec::with_capacity(src_lines.len());
        for (idx, raw) in src_lines.into_iter().enumerate() {
            let line_no = idx + 1;
            if self.wrap {
                let segments = wrap_line_by_width(raw, content_width);
                for (seg_idx, seg) in segments.into_iter().enumerate() {
                    out.push(VisualLine {
                        line_no: if seg_idx == 0 { Some(line_no) } else { None },
                        text: seg,
                    });
                }
            } else {
                let sliced = slice_from_col(raw, tab.scroll_x);
                let cut = truncate_str(&sliced, content_width);
                out.push(VisualLine {
                    line_no: Some(line_no),
                    text: cut,
                });
            }
        }
        let len = out.len();
        (out, len)
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 581-687 of 1366 (repository grok-build)
````
    fn render_body(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        let width = area.width as usize;
        let height = area.height as usize;
        if width == 0 || height == 0 {
            return;
        }

        let Some(mut tab_snapshot) = self.tabs.get(self.active_tab).cloned() else {
            return;
        };

        if tab_snapshot.is_empty_artifact {
            let msg = "(empty)";
            let msg_cut = truncate_str(msg, width);
            let msg_w = msg_cut.width();
            let x = area.x + ((width.saturating_sub(msg_w)) / 2) as u16;
            let y = area.y;
            buf.set_stringn(x, y, &msg_cut, width, Style::default().fg(theme.gray_dim));
            return;
        }

        let total_src = tab_snapshot.text.lines().count().max(1);
        let digits = decimal_digits(total_src).max(2);
        let gutter_width = digits + 2;

        // A block that already fits the body does not scroll sideways.
        if !self.wrap && gutter_width + widest_source_line(&tab_snapshot.text) <= width {
            tab_snapshot.scroll_x = 0;
        }

        let (visual_lines, total_visual) = self.compute_visual_lines(&tab_snapshot, width);
        let max_scroll = total_visual.saturating_sub(height);
        let scroll_y = tab_snapshot.scroll_y.min(max_scroll);
        if let Some(tab_mut) = self.tabs.get_mut(self.active_tab) {
            tab_mut.scroll_y = scroll_y;
        }

        let widest_text = if self.wrap {
            visual_lines
                .iter()
                .map(|v| v.text.width())
                .max()
                .unwrap_or(0)
        } else {
            widest_source_line(&tab_snapshot.text)
        };
        let (x_off, y_off) = centre_offset(gutter_width + widest_text, total_visual, width, height);

        let gutter_style = Style::default().fg(theme.gray_dim);
        let text_style = if tab_snapshot.kind == TabKind::Error {
            Style::default().fg(theme.accent_error)
        } else {
            Style::default().fg(theme.text_primary)
        };

        for row_idx in 0..height {
            let v_idx = scroll_y + row_idx;
            let Some(vline) = visual_lines.get(v_idx) else {
                break;
            };
            let y = area.y + (y_off + row_idx) as u16;

            let gutter_str = match vline.line_no {
                Some(n) => format!("{n:>digits$}  "),
                None => " ".repeat(gutter_width),
            };

            if width <= gutter_width {
                let cut_g = truncate_str(&gutter_str, width);
                buf.set_stringn(area.x, y, &cut_g, width, gutter_style);
                continue;
            }

            let line = Line::from(vec![
                Span::styled(gutter_str, gutter_style),
                Span::styled(vline.text.clone(), text_style),
            ]);
            buf.set_line(area.x + x_off as u16, y, &line, (width - x_off) as u16);
        }

        // Bottom-right overflow counter `12-40/312`
        if total_visual > height {
            let start = scroll_y + 1;
            let end = (scroll_y + height).min(total_visual);
            let overlay = format!("{start}-{end}/{total_visual}");
            let ov_width = overlay.width();
            if ov_width <= width {
                let ox = area.x + (width - ov_width) as u16;
                let oy = area.y + (height - 1) as u16;
                buf.set_stringn(
                    ox,
                    oy,
                    &overlay,
                    ov_width,
                    Style::default().fg(theme.gray_dim).bg(theme.bg_base),
                );
            }
        }
    }
}

struct VisualLine {
    line_no: Option<usize>,
    text: String,
}

/// Display width of the widest source line.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 1135-1220 of 1366 (repository grok-build)
````
    fn copy_returns_active_tab_full_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 1));
        state.handle_key(&KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE)); // Prompt
        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out, InspectorOutcome::Copy("Write a plan".to_string()));
    }

    #[test]
    fn json_output_tab_reads_as_rows_and_copy_still_gives_raw_json() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!({
            "verdict": "approve",
            "findings": [],
            "notes": ["a", "b"],
            "next": {"approve": "land.flow.ts", "meta": {}}
        });
        let mut state = InspectorState::new(dir.path(), step.clone(), None, (1, 1));
        let tab = state
            .tabs
            .iter()
            .find(|t| t.kind == TabKind::Output)
            .expect("output tab");
        assert!(!tab.text.contains('{') && !tab.text.contains('"'), "{}", tab.text);
        assert!(tab.text.contains("verdict   approve"), "{}", tab.text);
        assert!(tab.text.contains("findings  (empty)"), "{}", tab.text);
        assert!(tab.text.contains("- a") && tab.text.contains("- b"), "{}", tab.text);
        assert!(tab.text.contains("  approve  land.flow.ts"), "{}", tab.text);
        assert!(tab.text.contains("meta     (empty)"), "{}", tab.text);

        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out, InspectorOutcome::Copy(value_text(&step.output)));
        let InspectorOutcome::Copy(raw) = out else { unreachable!() };
        assert!(raw.contains("\"verdict\": \"approve\""));
    }

    #[test]
    fn string_output_tab_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!("plain reply");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let tab = &state.tabs[state.active_tab];
        assert_eq!(tab.text, "plain reply");
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            InspectorOutcome::Copy("plain reply".to_string())
        );
    }

    #[test]
    fn replay_only_on_step_with_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let mut acp_state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 1));
        assert_eq!(
            acp_state.handle_key(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            InspectorOutcome::Replay
        );

        let mut shell_state = InspectorState::new(dir.path(), sample_shell_step(), None, (1, 1));
        assert_eq!(
            shell_state.handle_key(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            InspectorOutcome::Unchanged
        );
    }

    #[test]
    fn prev_next_and_back_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 2));
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE)),
            InspectorOutcome::Prev
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
            InspectorOutcome::Next
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Back
        );
    }

    #[test]
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 634-676 of 1816 (repository grok-build)
````
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

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 750-775 of 1816 (repository grok-build)
````
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
}

/// What the list shows for one run beyond its summary. Reads the run folder.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1004-1014 of 1816 (repository grok-build)
````
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

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1383-1412 of 1816 (repository grok-build)
````
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
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 45-337 of 3778 (repository grok-build)
````
struct CopyUnit {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Debug, Default)]
struct Rows {
    lines: Vec<Line<'static>>,
    units: Vec<CopyUnit>,
}

impl Rows {
    fn new() -> Self {
        Self::default()
    }

    fn push_unit(&mut self, rows: Vec<Line<'static>>, text: String) {
        if rows.is_empty() {
            return;
        }
        let start = self.lines.len();
        self.lines.extend(rows);
        let end = self.lines.len();
        self.units.push(CopyUnit { start, end, text });
    }

    fn append_rows(&mut self, other: Rows) {
        let offset = self.lines.len();
        self.lines.extend(other.lines);
        for mut unit in other.units {
            unit.start += offset;
            unit.end += offset;
            self.units.push(unit);
        }
    }

    fn unit_at(&self, row: usize) -> Option<&CopyUnit> {
        let idx = self.units.partition_point(|u| u.end <= row);
        self.units
            .get(idx)
            .filter(|u| u.start <= row && row < u.end)
    }

    fn copy_range(&self, start_row: usize, end_row: usize) -> String {
        if self.lines.is_empty() || start_row > end_row {
            return String::new();
        }
        let last = end_row.min(self.lines.len() - 1);
        let mut out = Vec::new();
        let mut row = start_row.min(last);
        while row <= last {
            if let Some(unit) = self.unit_at(row) {
                out.push(unit.text.clone());
                row = unit.end;
            } else {
                out.push(line_plain_text(&self.lines[row]));
                row += 1;
            }
        }
        out.join("\n")
    }

    fn copy_all(&self) -> String {
        if self.lines.is_empty() {
            String::new()
        } else {
            self.copy_range(0, self.lines.len() - 1)
        }
    }
}

impl std::ops::Deref for Rows {
    type Target = Vec<Line<'static>>;

    fn deref(&self) -> &Self::Target {
        &self.lines
    }
}

impl std::ops::DerefMut for Rows {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.lines
    }
}

impl IntoIterator for Rows {
    type Item = Line<'static>;
    type IntoIter = std::vec::IntoIter<Line<'static>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.into_iter()
    }
}

impl<'a> IntoIterator for &'a Rows {
    type Item = &'a Line<'static>;
    type IntoIter = std::slice::Iter<'a, Line<'static>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter()
    }
}

fn line_plain_text(line: &Line<'_>) -> String {
    let mut text = String::new();
    for span in &line.spans {
        text.push_str(span.content.as_ref());
    }
    text.trim_end().to_string()
}

fn source_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect()
    }
}

pub struct DetailPanel {
    cache_key: Option<CacheKey>,
    /// The picked choice of an answered gate; `Some("")` when it was answered
    /// but the choice is unknown. `None` while the gate is open.
    answered: Option<String>,
    last_target: Option<(PathBuf, DetailTarget)>,
    lines: Rows,
    scroll: usize,
    tail_follow: bool,
    last_height: usize,
    focused: bool,
    cursor: usize,
    visual_anchor: Option<usize>,
}

impl Default for DetailPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailPanel {
    pub fn new() -> Self {
        Self {
            cache_key: None,
            answered: None,
            last_target: None,
            lines: Rows::new(),
            scroll: 0,
            tail_follow: false,
            last_height: 0,
            focused: false,
            cursor: 0,
            visual_anchor: None,
        }
    }

    /// Tell the panel whether the run's gate has been answered, and with what.
    pub fn set_answered(&mut self, answered: Option<&str>) {
        if self.answered.as_deref() != answered {
            self.answered = answered.map(str::to_string);
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_visual(&self) -> bool {
        self.focused && self.visual_anchor.is_some()
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if !focused {
            self.visual_anchor = None;
            return;
        }
        self.tail_follow = false;
        if self.lines.is_empty() {
            self.cursor = 0;
            return;
        }
        let max_row = self.lines.len() - 1;
        self.cursor = self.cursor.min(max_row);
        if self.last_height > 0 {
            let top = self.scroll.min(max_row);
            let bottom = (top + self.last_height - 1).min(max_row);
            if self.cursor < top || self.cursor > bottom {
                self.cursor = top;
            }
        }
    }

    pub fn page_step(&self) -> isize {
        if self.last_height > 1 {
            (self.last_height - 1) as isize
        } else if self.last_height == 1 {
            1
        } else {
            15
        }
    }

    pub fn move_cursor(&mut self, delta: isize) -> bool {
        if self.lines.is_empty() || delta == 0 {
            return false;
        }
        let max_row = self.lines.len() - 1;
        let cur = self.cursor.min(max_row);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_row)
        };
        self.tail_follow = false;
        let moved = next != self.cursor;
        self.cursor = next;
        self.ensure_cursor_visible(self.last_height);
        moved
    }

    pub fn toggle_visual(&mut self) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.visual_anchor = Some(self.cursor.min(self.lines.len() - 1));
        }
        true
    }

    pub fn handle_esc(&mut self) -> bool {
        if !self.focused {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.focused = false;
        }
        true
    }

    pub fn copy_selection(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        Some(self.lines.copy_range(start, end))
    }

    pub fn copy_all(&self) -> Option<String> {
        if self.lines.is_empty() {
            None
        } else {
            Some(self.lines.copy_all())
        }
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        if !self.focused || self.lines.is_empty() {
            return None;
        }
        let max_row = self.lines.len() - 1;
        let cur = self.cursor.min(max_row);
        match self.visual_anchor {
            Some(anchor) => {
                let a = anchor.min(max_row);
                Some((a.min(cur), a.max(cur)))
            }
            None => Some((cur, cur)),
        }
    }

    fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 || self.lines.is_empty() {
            return;
        }
        let max_row = self.lines.len() - 1;
        self.cursor = self.cursor.min(max_row);
        let max_scroll = self.max_scroll(height);
        self.scroll = self.scroll.min(max_scroll);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
        self.scroll = self.scroll.min(max_scroll);
    }

````

=== TASK SPECIFICATION ===

Slice ID: inspector-focus

Packet 7: Inspector body cursor and range.

Ownership:
- `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs`: edit `TabEntry`, `InspectorState`, `InspectorState::new`, `handle_key`, `scroll`, `render`, `shortcuts`, `max_scroll_y`, `compute_visual_lines`, `render_body`, `VisualLine`, and tests in `mod tests`.
- `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs`: edit only Step-mode items: `handle_step_key`, `active_shortcut_labels`, `STEP_KEYS`, and Step-mode tests in `mod tests`.
- Attached read-only: `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` (the landed packet 3b implementation) as the reference pattern to reuse and mirror.
- Do not edit `detail.rs`, `run.rs`, `ledger.rs`, `modal_view.rs`, `RUN_KEYS`, `LEDGER_KEYS`, `row_extra`, or any Mode::Run / Mode::Ledger routing. Packet 2 owns `row_extra`, Packet 3b owns Run-mode focus and `RUN_KEYS`, and Packet 4 owns modal preview and `LEDGER_KEYS`. Edits in `acpx_modal.rs` must stay strictly disjoint to avoid coordinator integration conflicts.

Goal:
In the inspector (opened when pressing Enter on a step), the operator wants linewise inspection and copying identical to the pager's line viewer and the landed detail pane, plus arrow-key step navigation.
Acceptance sentence: In an open step inspector, the operator can switch between steps using Left/Right arrows without leaving the view, navigate the body text with a line cursor (j/k, Up/Down), select a range with v, copy the exact cursor line or visual range with y (unwrapped without added breaks), and copy the entire tab with Y.

Behaviour contract:
- In Mode::Step, the inspector body is focused.
- Cursor navigation: `j`/`k` and `Down`/`Up` move a line cursor down and up by one visual row and keep it visible (`ensure_cursor_visible`, scrolling `scroll_y` as necessary). `PageDown`/`PageUp` move the cursor by half the body height (or page step); `Ctrl-d`/`Ctrl-u` move by half the body height; `g`/`Home` move cursor to row 0; `G`/`End` move cursor to the last visual row.
- Visual selection: `v` starts or ends a visual line range anchored at the cursor (`toggle_visual`).
- Copy:
  - `y` copies the cursor line (when visual mode is off) or the visual range (when visual mode is on). Copy returns `InspectorOutcome::Copy(text)`, which `handle_step_key` passes through as `AcpxOutcome::Copy(text)` so the standard copy notice is reported. After `y` copies, visual mode is cleared.
  - `Y` copies the whole active tab (`tab.copy` if `Some`, otherwise `tab.text`), preserving the full-tab copy behavior previously on `y`.
  - Copy is plain text: no ANSI styles, line numbers, gutter spacing, indentation, or breaks added by wrapping; a wrapped source line copies as its exact whole source line. A multi-line range joins touched source lines with `\n`.
- Step navigation:
  - `Left` and `[` return `InspectorOutcome::Prev`, switching to the previous step.
  - `Right` and `]` return `InspectorOutcome::Next`, switching to the next step.
  - Horizontal scrolling: `h` and `l` keep horizontal scroll (left and right by `HORIZ_SCROLL_STEP`) when wrapping is disabled (`!self.wrap`).
- Tab switching:
  - `Tab` and `Shift+Tab` (or `BackTab`) cycle forward and backward through inspector tabs.
  - Digit keys `1` through `7` switch directly to that tab index.
  - Switching tabs clears visual mode (`visual_anchor = None`) and clamps the cursor to the visual row count of the newly selected tab.
- Wrapping:
  - `w` toggles wrapping (`self.wrap = !self.wrap`). Visual lines are recomputed, and cursor is clamped and kept visible.
- Escape handling:
  - If visual mode is active (`visual_anchor.is_some()`), `Esc` ends visual mode (`visual_anchor = None`) and stays in the inspector (`InspectorOutcome::Changed`).
  - If visual mode is not active, `Esc` returns `InspectorOutcome::Back`, which returns to Mode::Run and selects the same step row in the run view.
- Footer and Help overlay:
  - `shortcuts(&self)` and `active_shortcut_labels` show step and cursor keys: `Tab/1-7 tab`, `←/→/[/] step`, `j/k line`, `v range`, `y copy`, `Y all`, `w wrap`, (and `r replay` if conversation is present), `Esc back` (or `Esc unselect` when visual mode is active).
  - `STEP_KEYS` in `acpx_modal.rs` is updated to document the new keymap:
    - `"Tab / 1-7"`: `"Switch inspector tab"`
    - `"← / → / [ / ]"`: `"Previous / next step in run"`
    - `"j / k / ↑ / ↓"`: `"Move line cursor"`
    - `"v"`: `"Start or end visual range"`
    - `"y"`: `"Copy cursor line or visual range"`
    - `"Y"`: `"Copy whole tab content"`
    - `"h / l"`: `"Horizontal scroll (when unwrapped)"`
    - `"w"`: `"Toggle line wrapping"`
    - `"PgUp / PgDn"`: `"Page up / down"`
    - `"r"`: `"Replay ACP conversation for this step"`
    - `"?"`: `"Toggle this key reference"`
    - `"Esc"`: `"End visual selection / back to run view"`

Representation guidance (mirroring detail.rs):
- Look at `views/acpx_modal/detail.rs#L45-337`. Mirror `CopyUnit` (`start`, `end`, `text`) and the cursor methods: `move_cursor`, `toggle_visual`, `handle_esc`, `selected_range`, and `ensure_cursor_visible`.
- In `TabEntry` (or `InspectorState`), maintain `cursor: usize` and `visual_anchor: Option<usize>`. If maintained per-tab in `TabEntry`, tabs remember their cursor position; initialize with `cursor: 0` and `visual_anchor: None`.
- In `compute_visual_lines` and `VisualLine`: associate each visual line with its source line unit or maintain a list of `CopyUnit`s for the active tab, where `start..end` are visual line row indices and `text` is the exact source line from `tab.text`.
- When copying with `y`: lookup the unit(s) covering the cursor row (or `selected_range`), and return their source `text` joined by `\n`. Pointers on wrapped continuation lines (where `line_no` is `None`) copy the complete unwrapped source line without wrapping breaks.
- In `render_body`: when row `v_idx` matches the cursor (or falls within `selected_range` when visual mode is active), highlight the row using `theme.bg_highlight`.
- In `handle_step_key`: ensure `InspectorOutcome::Prev`, `Next`, and `Copy(text)` flow into `AcpxOutcome::Changed` and `AcpxOutcome::Copy(text)` as already supported.

Tests:
- In `inspector.rs`:
  - Update `copy_returns_active_tab_full_text`: verify `Y` returns full active tab text; verify `y` returns the cursor line.
  - Update `prev_next_and_back_keys`: verify `KeyCode::Left` returns `InspectorOutcome::Prev` and `KeyCode::Right` returns `InspectorOutcome::Next`, alongside `[` and `]`.
  - Add test `cursor_movement_and_scroll_following`: moving cursor with `j`/`k` and `Down`/`Up` clamps to bounds and updates `scroll_y` via `ensure_cursor_visible`.
  - Add test `visual_range_selection_and_esc_unwind`: pressing `v` sets anchor; moving cursor and pressing `Esc` cancels visual mode without returning `Back`; pressing `Esc` again returns `InspectorOutcome::Back`.
  - Add test `wrapped_line_copies_full_source_unwrapped`: wrap a long line across multiple visual lines; verify pressing `y` on the second visual line copies the complete source line without breaks.
  - Add test `visual_range_multiline_copy`: select across multiple visual lines and verify `y` copies the exact touched source lines joined with `\n`.
  - Add test `json_output_tab_y_copies_line_and_capital_y_copies_raw_json`: on a tab with `copy: Some(raw_json)`, verify `y` copies the current visual line while `Y` copies the full raw JSON.
  - Add test `inspector_render_shows_cursor_highlight`: verify rendered buffer has `theme.bg_highlight` on the cursor row.
- In `acpx_modal.rs`:
  - Update or add test `step_arrow_keys_navigate_steps_and_esc_preserves_selection`: verify Left and Right arrow keys advance and retreat steps in `handle_step_key`.

Stated once:
- `detail.rs` is read-only.
- No new external dependencies, no unsafe code, no panics on small or 0x0 buffer areas.

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
=== FILE: task-scripts/acpx-disclosure-returns/inspector-focus.edits.md ===
```markdown
<complete Markdown edit list>
```
