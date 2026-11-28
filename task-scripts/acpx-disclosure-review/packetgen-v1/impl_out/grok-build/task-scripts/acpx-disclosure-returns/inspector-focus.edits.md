## Edit 1: Add per-tab cursor and visual_anchor state to TabEntry
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
struct TabEntry {
    kind: TabKind,
    text: String,
    is_empty_artifact: bool,
    scroll_y: usize,
    scroll_x: usize,
    cursor: usize,
    visual_anchor: Option<usize>,
    /// What `Y` copies when it differs from what the tab shows (a JSON value shown as rows copies its raw JSON).
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
            // A JSON value reads as rows, not braces and quotes; `Y` still copies the raw JSON.
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
                cursor: 0,
                visual_anchor: None,
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
                cursor: 0,
                visual_anchor: None,
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
                cursor: 0,
                visual_anchor: None,
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
                cursor: 0,
                visual_anchor: None,
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
                cursor: 0,
                visual_anchor: None,
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
                cursor: 0,
                visual_anchor: None,
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
                cursor: 0,
                visual_anchor: None,
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
            cursor: 0,
            visual_anchor: None,
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
~~~

## Edit 2: Add cursor navigation, visual selection, line/tab copy, arrow step keys, and updated shortcuts
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    pub fn is_visual(&self) -> bool {
        self.tabs
            .get(self.active_tab)
            .is_some_and(|t| t.visual_anchor.is_some())
    }

    pub fn move_cursor(&mut self, delta: isize) -> bool {
        let total = self.total_visual_lines();
        if total == 0 || delta == 0 {
            return false;
        }
        let max_row = total - 1;
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let cur = tab.cursor.min(max_row);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_row)
        };
        let moved = next != tab.cursor;
        let old_scroll = tab.scroll_y;
        tab.cursor = next;
        self.ensure_cursor_visible(self.last_body_height);
        let scrolled = self
            .tabs
            .get(self.active_tab)
            .is_some_and(|t| t.scroll_y != old_scroll);
        moved || scrolled
    }

    pub fn toggle_visual(&mut self) -> bool {
        let total = self.total_visual_lines();
        if total == 0 {
            return false;
        }
        let max_row = total - 1;
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        if tab.visual_anchor.is_some() {
            tab.visual_anchor = None;
        } else {
            tab.cursor = tab.cursor.min(max_row);
            tab.visual_anchor = Some(tab.cursor);
        }
        true
    }

    pub fn handle_esc(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        if tab.visual_anchor.is_some() {
            tab.visual_anchor = None;
            true
        } else {
            false
        }
    }

    fn selected_range_for(tab: &TabEntry, total_visual: usize) -> Option<(usize, usize)> {
        if total_visual == 0 || tab.is_empty_artifact {
            return None;
        }
        let max_row = total_visual - 1;
        let cur = tab.cursor.min(max_row);
        match tab.visual_anchor {
            Some(anchor) => {
                let a = anchor.min(max_row);
                Some((a.min(cur), a.max(cur)))
            }
            None => Some((cur, cur)),
        }
    }

    fn ensure_cursor_visible(&mut self, height: usize) {
        let total = self.total_visual_lines();
        if height == 0 || total == 0 {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.cursor = 0;
                tab.scroll_y = 0;
                tab.visual_anchor = None;
            }
            return;
        }
        let max_row = total - 1;
        let max_scroll = total.saturating_sub(height);
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        tab.cursor = tab.cursor.min(max_row);
        if let Some(anchor) = tab.visual_anchor {
            tab.visual_anchor = Some(anchor.min(max_row));
        }
        tab.scroll_y = tab.scroll_y.min(max_scroll);
        if tab.cursor < tab.scroll_y {
            tab.scroll_y = tab.cursor;
        } else if tab.cursor >= tab.scroll_y + height {
            tab.scroll_y = tab.cursor + 1 - height;
        }
        tab.scroll_y = tab.scroll_y.min(max_scroll);
    }

    fn copy_selection(&self) -> Option<String> {
        let tab = self.tabs.get(self.active_tab)?;
        if tab.is_empty_artifact {
            return None;
        }
        let (visual_lines, units) = self.compute_visual_layout(tab, self.last_body_width);
        let (start, end) = Self::selected_range_for(tab, visual_lines.len())?;
        Some(copy_units_range(&units, start, end))
    }

    fn total_visual_lines(&self) -> usize {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return 0;
        };
        if tab.is_empty_artifact {
            return 0;
        }
        let (_, total_visual) = self.compute_visual_lines(tab, self.last_body_width);
        total_visual
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
        if key.kind == KeyEventKind::Release {
            return InspectorOutcome::Unchanged;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                if self.handle_esc() {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Back
                }
            }
            KeyCode::Tab => {
                if !self.tabs.is_empty() {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    self.ensure_cursor_visible(self.last_body_height);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::BackTab => {
                if !self.tabs.is_empty() {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    self.ensure_cursor_visible(self.last_body_height);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char(c @ '1'..='7') if !ctrl => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.tabs.len() && idx != self.active_tab {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    self.active_tab = idx;
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    self.ensure_cursor_visible(self.last_body_height);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                if self.move_cursor(1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                if self.move_cursor(-1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.move_cursor(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.move_cursor(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.move_cursor(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.move_cursor(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('g') | KeyCode::Home if !ctrl => {
                let total = self.total_visual_lines();
                if total == 0 {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && (tab.cursor != 0 || tab.scroll_y != 0)
                {
                    tab.cursor = 0;
                    tab.scroll_y = 0;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                let total = self.total_visual_lines();
                if total == 0 {
                    return InspectorOutcome::Unchanged;
                }
                let last_row = total - 1;
                let max_scroll = total.saturating_sub(self.last_body_height.max(1));
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && (tab.cursor != last_row || tab.scroll_y != max_scroll)
                {
                    tab.cursor = last_row;
                    tab.scroll_y = max_scroll;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('v') if !ctrl => {
                if self.toggle_visual() {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('h') if !ctrl => {
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
            KeyCode::Char('l') if !ctrl => {
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
                self.ensure_cursor_visible(self.last_body_height);
                InspectorOutcome::Changed
            }
            KeyCode::Char('y') if !ctrl => {
                if let Some(text) = self.copy_selection() {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.visual_anchor = None;
                    }
                    InspectorOutcome::Copy(text)
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('Y') if !ctrl => {
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
            KeyCode::Left | KeyCode::Char('[') if !ctrl => InspectorOutcome::Prev,
            KeyCode::Right | KeyCode::Char(']') if !ctrl => InspectorOutcome::Next,
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
            "Tab/1-7 tab",
            "←/→/[/] step",
            "j/k line",
            "v range",
            "y copy",
            "Y all",
            "w wrap",
        ];
        if self.step.conversation.is_some() {
            s.push("r replay");
        }
        if self.is_visual() {
            s.push("Esc unselect");
        } else {
            s.push("Esc back");
        }
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
~~~

## Edit 3: Track CopyUnits alongside visual lines in compute_visual_layout
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn compute_visual_lines(
        &self,
        tab: &TabEntry,
        area_width: usize,
    ) -> (Vec<VisualLine>, usize) {
        let (out, _) = self.compute_visual_layout(tab, area_width);
        let len = out.len();
        (out, len)
    }

    fn compute_visual_layout(
        &self,
        tab: &TabEntry,
        area_width: usize,
    ) -> (Vec<VisualLine>, Vec<CopyUnit>) {
        if tab.is_empty_artifact {
            return (Vec::new(), Vec::new());
        }
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
        let mut units = Vec::with_capacity(src_lines.len());
        for (idx, raw) in src_lines.into_iter().enumerate() {
            let line_no = idx + 1;
            let start = out.len();
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
            let end = out.len();
            if end > start {
                units.push(CopyUnit {
                    start,
                    end,
                    text: raw.to_string(),
                });
            }
        }
        (out, units)
    }
~~~

## Edit 4: Highlight cursor/visual range in render_body and add CopyUnit helpers
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
        let max_row = total_visual.saturating_sub(1);
        tab_snapshot.cursor = tab_snapshot.cursor.min(max_row);
        if let Some(anchor) = tab_snapshot.visual_anchor {
            tab_snapshot.visual_anchor = Some(anchor.min(max_row));
        }
        if let Some(tab_mut) = self.tabs.get_mut(self.active_tab) {
            tab_mut.scroll_y = scroll_y;
            tab_mut.cursor = tab_snapshot.cursor;
            tab_mut.visual_anchor = tab_snapshot.visual_anchor;
        }

        let selected = Self::selected_range_for(&tab_snapshot, total_visual);

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
            let is_selected = selected.is_some_and(|(start, end)| v_idx >= start && v_idx <= end);

            let (row_gutter_style, row_text_style) = if is_selected {
                buf.set_style(
                    Rect::new(area.x, y, area.width, 1),
                    Style::default().bg(theme.bg_highlight),
                );
                (
                    gutter_style.bg(theme.bg_highlight),
                    text_style.bg(theme.bg_highlight),
                )
            } else {
                (gutter_style, text_style)
            };

            let gutter_str = match vline.line_no {
                Some(n) => format!("{n:>digits$}  "),
                None => " ".repeat(gutter_width),
            };

            if width <= gutter_width {
                let cut_g = truncate_str(&gutter_str, width);
                buf.set_stringn(area.x, y, &cut_g, width, row_gutter_style);
                continue;
            }

            let line = Line::from(vec![
                Span::styled(gutter_str, row_gutter_style),
                Span::styled(vline.text.clone(), row_text_style),
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

struct CopyUnit {
    start: usize,
    end: usize,
    text: String,
}

fn copy_units_range(units: &[CopyUnit], start_row: usize, end_row: usize) -> String {
    if units.is_empty() || start_row > end_row {
        return String::new();
    }
    let mut out = Vec::new();
    let mut row = start_row;
    while row <= end_row {
        let idx = units.partition_point(|u| u.end <= row);
        if let Some(unit) = units.get(idx).filter(|u| u.start <= row && row < u.end) {
            out.push(unit.text.as_str());
            row = unit.end;
        } else {
            row += 1;
        }
    }
    out.join("\n")
}
~~~

## Edit 5: Update existing copy/navigation tests and add inspector focus/cursor/range tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn copy_returns_active_tab_full_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.prompt_text = Some("Write a plan\nSecond line".to_string());
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        state.handle_key(&KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE)); // Prompt
        let full = state.handle_key(&KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE));
        assert_eq!(
            full,
            InspectorOutcome::Copy("Write a plan\nSecond line".to_string())
        );
        let line = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(line, InspectorOutcome::Copy("Write a plan".to_string()));
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

        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE));
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
            state.handle_key(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            InspectorOutcome::Prev
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
            InspectorOutcome::Next
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            InspectorOutcome::Next
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Back
        );
    }

    #[test]
    fn cursor_movement_and_scroll_following() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        let lines = (1..=10)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        step.output = serde_json::json!(lines);
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 40, 6); // 4 body rows
        let mut buf = Buffer::empty(area);
        state.render(&mut buf, area, &theme, 0);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
            InspectorOutcome::Unchanged
        );
        assert_eq!(state.tabs[state.active_tab].cursor, 0);
        assert_eq!(state.tabs[state.active_tab].scroll_y, 0);

        for _ in 0..4 {
            assert_eq!(
                state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
                InspectorOutcome::Changed
            );
        }
        assert_eq!(state.tabs[state.active_tab].cursor, 4);
        assert_eq!(state.tabs[state.active_tab].scroll_y, 1);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[state.active_tab].cursor, 5);
        assert_eq!(state.tabs[state.active_tab].scroll_y, 2);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[state.active_tab].cursor, 9);
        assert_eq!(state.tabs[state.active_tab].scroll_y, 6);
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            InspectorOutcome::Unchanged
        );

        for _ in 0..4 {
            assert_eq!(
                state.handle_key(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
                InspectorOutcome::Changed
            );
        }
        assert_eq!(state.tabs[state.active_tab].cursor, 5);
        assert_eq!(state.tabs[state.active_tab].scroll_y, 5);
    }

    #[test]
    fn visual_range_selection_and_esc_unwind() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!("alpha\nbeta\ngamma\ndelta");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));

        assert!(!state.is_visual());
        assert!(state.shortcuts().contains(&"Esc back"));

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert!(state.is_visual());
        assert!(state.shortcuts().contains(&"Esc unselect"));

        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(state.tabs[state.active_tab].visual_anchor, Some(0));
        assert_eq!(state.tabs[state.active_tab].cursor, 2);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert!(!state.is_visual());
        assert!(state.shortcuts().contains(&"Esc back"));

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Back
        );
    }

    #[test]
    fn wrapped_line_copies_full_source_unwrapped() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        let long_line = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        step.output = serde_json::json!(format!("{long_line}\nshort tail"));
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 24, 10);
        let mut buf = Buffer::empty(area);
        state.render(&mut buf, area, &theme, 0);

        // Move to the second visual row (wrapped continuation of the first source line).
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[state.active_tab].cursor, 1);

        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out, InspectorOutcome::Copy(long_line.to_string()));
    }

    #[test]
    fn visual_range_multiline_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!("one\ntwo\nthree\nfour\nfive");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        state.render(&mut buf, area, &theme, 0);

        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // "two"
        state.handle_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // "three"
        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // "four"

        let out = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out, InspectorOutcome::Copy("two\nthree\nfour".to_string()));
        assert!(!state.is_visual());
    }

    #[test]
    fn json_output_tab_y_copies_line_and_capital_y_copies_raw_json() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!({
            "verdict": "approve",
            "findings": []
        });
        let mut state = InspectorState::new(dir.path(), step.clone(), None, (1, 1));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        state.render(&mut buf, area, &theme, 0);

        let line_copy = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let InspectorOutcome::Copy(line_text) = line_copy else {
            panic!("expected Copy outcome");
        };
        assert!(!line_text.contains('{') && !line_text.contains('"'), "{line_text}");
        assert!(!line_text.contains('\n'), "{line_text}");
        assert!(
            line_text.contains("findings") || line_text.contains("verdict"),
            "{line_text}"
        );

        let full_copy = state.handle_key(&KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE));
        assert_eq!(full_copy, InspectorOutcome::Copy(value_text(&step.output)));
    }

    #[test]
    fn inspector_render_shows_cursor_highlight() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = serde_json::json!("first\nsecond\nthird");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 40, 8);

        let mut buf0 = Buffer::empty(area);
        state.render(&mut buf0, area, &theme, 0);
        let highlighted_rows = |buf: &Buffer| -> Vec<u16> {
            (2..area.height)
                .filter(|&y| buf[(0, y)].bg == theme.bg_highlight)
                .collect()
        };
        let rows0 = highlighted_rows(&buf0);
        assert_eq!(rows0.len(), 1);

        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let mut buf1 = Buffer::empty(area);
        state.render(&mut buf1, area, &theme, 0);
        let rows1 = highlighted_rows(&buf1);
        assert_eq!(rows1, vec![rows0[0] + 1]);
    }
~~~

## Edit 6: Update STEP_KEYS help table for inspector cursor, range, copy, and arrow step keys
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
const STEP_KEYS: &[(&str, &str)] = &[
    ("Tab / 1-7", "Switch inspector tab"),
    ("← / → / [ / ]", "Previous / next step in run"),
    ("j / k / ↑ / ↓", "Move line cursor"),
    ("v", "Start or end visual range"),
    ("y", "Copy cursor line or visual range"),
    ("Y", "Copy whole tab content"),
    ("h / l", "Horizontal scroll (when unwrapped)"),
    ("w", "Toggle line wrapping"),
    ("PgUp / PgDn", "Page up / down"),
    ("r", "Replay ACP conversation for this step"),
    ("?", "Toggle this key reference"),
    ("Esc", "End visual selection / back to run view"),
];
~~~

## Edit 7: Add Step-mode arrow-key navigation test in acpx_modal.rs
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
    fn step_arrow_keys_navigate_steps_and_esc_preserves_selection() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);

        if let Some(rv) = state.run_view.as_mut() {
            rv.select_step(0);
        }
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Step);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(0));

        // Right arrow advances to step 1
        let out = state.handle_key(&key(KeyCode::Right));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(1));

        // Left arrow retreats to step 0
        let out = state.handle_key(&key(KeyCode::Left));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(0));

        // Advance again and verify Esc preserves selection in Run view
        assert_eq!(state.handle_key(&key(KeyCode::Right)), AcpxOutcome::Changed);
        state.handle_key(&key(KeyCode::Esc));
        assert_eq!(state.mode(), Mode::Run);
        assert_eq!(
            state.run_view.as_ref().and_then(|rv| rv.selected_step()),
            Some(1)
        );
    }
~~~

## Coordinator follow-ups
- `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` (unattached test lines outside 1135-1220): if any existing unit tests assert horizontal scrolling via `KeyCode::Left`/`KeyCode::Right`, direct `scroll_y` changes on `j`/`k` before the cursor hits the viewport edge, or the previous colon-formatted `shortcuts()` strings (`"Tab/1-7: tab"`, `"Esc: back"`), update those assertions to match `h`/`l` horizontal scroll, cursor-following scroll, and the new shortcut labels.
