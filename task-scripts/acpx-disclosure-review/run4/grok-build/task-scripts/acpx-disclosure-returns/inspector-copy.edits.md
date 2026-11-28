## Edit 1: Add per-tab line cursor and visual-range anchor state
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
struct TabEntry {
    kind: TabKind,
    text: String,
    is_empty_artifact: bool,
    scroll_y: usize,
    cursor: usize,
    scroll_x: usize,
    /// What `Y` copies when it differs from what the tab shows (a JSON value shown as rows copies its raw JSON).
    copy: Option<String>,
}

pub struct InspectorState {
    step: Step,
    node: Option<FlowNode>,
    position: (usize, usize),
    tabs: Vec<TabEntry>,
    active_tab: usize,
    visual_anchor: Option<usize>,
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
                cursor: 0,
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
                cursor: 0,
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
                cursor: 0,
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
                cursor: 0,
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
                cursor: 0,
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
                cursor: 0,
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
                cursor: 0,
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
            cursor: 0,
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
            visual_anchor: None,
            wrap: true,
            last_body_height: 20,
            last_body_width: 80,
        }
    }
~~~

## Edit 2: Update handle_key for Left/Right step navigation, line cursor, visual range, and y/Y copy
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
~~~
NEW:
~~~rust
    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
        if key.kind == KeyEventKind::Release {
            return InspectorOutcome::Unchanged;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                if self.visual_anchor.take().is_some() {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Back
                }
            }
            KeyCode::Tab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    self.visual_anchor = None;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::BackTab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
                    self.visual_anchor = None;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char(c @ '1'..='7') if !ctrl => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.tabs.len() && idx != self.active_tab {
                    self.active_tab = idx;
                    self.visual_anchor = None;
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
                if self.set_cursor(0) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                if self.set_cursor(usize::MAX) {
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
            KeyCode::Char('v') if !ctrl => {
                let Some(tab) = self.tabs.get(self.active_tab) else {
                    return InspectorOutcome::Unchanged;
                };
                if tab.is_empty_artifact {
                    return InspectorOutcome::Unchanged;
                }
                if self.visual_anchor.is_some() {
                    self.visual_anchor = None;
                } else {
                    self.visual_anchor = Some(tab.cursor);
                }
                InspectorOutcome::Changed
            }
            KeyCode::Char('w') if !ctrl => {
                self.wrap = !self.wrap;
                self.ensure_cursor_visible();
                InspectorOutcome::Changed
            }
            KeyCode::Char('y') if !ctrl => {
                let text = self.copy_selection_text();
                self.visual_anchor = None;
                InspectorOutcome::Copy(text)
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
~~~

## Edit 3: Update shortcuts, cursor helpers, and visual-line source index tracking
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut s = vec![
            "←/→: step",
            "j/k: line",
            "v: range",
            "y: copy line",
            "Y: copy tab",
            "Tab/1-7: tab",
            "w: wrap",
        ];
        if self.step.conversation.is_some() {
            s.push("r: replay");
        }
        s.push("Esc: back");
        s
    }

    fn move_cursor(&mut self, delta: isize) -> bool {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return false;
        };
        if tab.is_empty_artifact {
            return false;
        }
        let last = source_line_count(&tab.text).saturating_sub(1);
        let cur = tab.cursor.min(last);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(last)
        };
        self.set_cursor(next)
    }

    fn set_cursor(&mut self, target: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        if tab.is_empty_artifact {
            return false;
        }
        let last = source_line_count(&tab.text).saturating_sub(1);
        let next = target.min(last);
        let cursor_changed = next != tab.cursor;
        tab.cursor = next;
        let scroll_changed = self.ensure_cursor_visible();
        cursor_changed || scroll_changed
    }

    fn ensure_cursor_visible(&mut self) -> bool {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return false;
        };
        if tab.is_empty_artifact {
            return false;
        }
        let cursor = tab.cursor;
        let height = self.last_body_height.max(1);
        let (visual_lines, total_visual) = self.compute_visual_lines(tab, self.last_body_width);
        let max_scroll = total_visual.saturating_sub(height);

        let mut first_v = None;
        let mut last_v = 0usize;
        for (v_idx, vline) in visual_lines.iter().enumerate() {
            if vline.src_idx == cursor {
                if first_v.is_none() {
                    first_v = Some(v_idx);
                }
                last_v = v_idx;
            }
        }

        let Some(first_v) = first_v else {
            return false;
        };

        let Some(tab_mut) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let prev_scroll = tab_mut.scroll_y;
        let mut scroll_y = prev_scroll.min(max_scroll);

        if first_v < scroll_y {
            scroll_y = first_v;
        } else if last_v >= scroll_y + height {
            let cursor_rows = last_v - first_v + 1;
            if cursor_rows <= height {
                scroll_y = last_v + 1 - height;
            } else {
                scroll_y = first_v;
            }
        }
        scroll_y = scroll_y.min(max_scroll);
        tab_mut.scroll_y = scroll_y;
        scroll_y != prev_scroll
    }

    fn selection_range(&self, tab: &TabEntry) -> (usize, usize) {
        match self.visual_anchor {
            Some(anchor) => (anchor.min(tab.cursor), anchor.max(tab.cursor)),
            None => (tab.cursor, tab.cursor),
        }
    }

    fn copy_selection_text(&self) -> String {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return String::new();
        };
        if tab.is_empty_artifact {
            return tab.copy.clone().unwrap_or_else(|| tab.text.clone());
        }
        let lines = source_lines(&tab.text);
        let last = lines.len().saturating_sub(1);
        let (start, end) = self.selection_range(tab);
        let start = start.min(last);
        let end = end.min(last);
        lines[start..=end].join("\n")
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

    fn compute_visual_lines(
        &self,
        tab: &TabEntry,
        area_width: usize,
    ) -> (Vec<VisualLine>, usize) {
        let src_lines = source_lines(&tab.text);
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
                        src_idx: idx,
                        line_no: if seg_idx == 0 { Some(line_no) } else { None },
                        text: seg,
                    });
                }
            } else {
                let sliced = slice_from_col(raw, tab.scroll_x);
                let cut = truncate_str(&sliced, content_width);
                out.push(VisualLine {
                    src_idx: idx,
                    line_no: Some(line_no),
                    text: cut,
                });
            }
        }
        let len = out.len();
        (out, len)
    }
~~~

## Edit 4: Highlight cursor and visual range across body width and add source_lines helpers
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

        let total_src = source_line_count(&tab_snapshot.text);
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
        let (sel_start, sel_end) = self.selection_range(&tab_snapshot);

        for row_idx in 0..height {
            let v_idx = scroll_y + row_idx;
            let Some(vline) = visual_lines.get(v_idx) else {
                break;
            };
            let y = area.y + (y_off + row_idx) as u16;
            let is_selected = vline.src_idx >= sel_start && vline.src_idx <= sel_end;

            let row_gutter_style = if is_selected {
                gutter_style.bg(theme.bg_highlight)
            } else {
                gutter_style
            };
            let row_text_style = if is_selected {
                text_style.bg(theme.bg_highlight)
            } else {
                text_style
            };

            if is_selected {
                for x in area.x..area.x + area.width {
                    buf[(x, y)].set_bg(theme.bg_highlight);
                }
            }

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
    src_idx: usize,
    line_no: Option<usize>,
    text: String,
}

fn source_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect()
    }
}

fn source_line_count(text: &str) -> usize {
    if text.is_empty() {
        1
    } else {
        text.lines().count().max(1)
    }
}
~~~

## Edit 5: Update JSON output copy test for y (shown row) and Y (raw JSON)
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

        let expected_row = tab.text.lines().next().unwrap().to_string();
        let out_line = state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(out_line, InspectorOutcome::Copy(expected_row));

        let out_tab = state.handle_key(&KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE));
        assert_eq!(out_tab, InspectorOutcome::Copy(value_text(&step.output)));
        let InspectorOutcome::Copy(raw) = out_tab else { unreachable!() };
        assert!(raw.contains("\"verdict\": \"approve\""));
    }
~~~

## Edit 6: Add inspector unit tests for arrow step nav, h/l scroll, cursor copy, wrap highlight, Esc range, tab cursor memory, and G
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs
OLD:
~~~rust
    #[test]
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 3), (5, 3), (4, 4), (60, 2), (2, 30)] {
            render_text("ab\nabcdef", false, w, h, &[]);
            render_text("ab\nabcdef", true, w, h, &[]);
        }
        assert_eq!(centre_offset(10, 3, 0, 0), (0, 0));
        assert_eq!(centre_offset(10, 3, 40, 18), (15, 7));
        assert_eq!(centre_offset(10, 30, 40, 18), (0, 0));
        assert_eq!(centre_offset(50, 3, 40, 18), (0, 0));
    }
}
~~~
NEW:
~~~rust
    #[test]
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 3), (5, 3), (4, 4), (60, 2), (2, 30)] {
            render_text("ab\nabcdef", false, w, h, &[]);
            render_text("ab\nabcdef", true, w, h, &[]);
        }
        assert_eq!(centre_offset(10, 3, 0, 0), (0, 0));
        assert_eq!(centre_offset(10, 3, 40, 18), (15, 7));
        assert_eq!(centre_offset(10, 30, 40, 18), (0, 0));
        assert_eq!(centre_offset(50, 3, 40, 18), (0, 0));
    }

    #[test]
    fn arrow_and_bracket_keys_step_prev_and_next() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = InspectorState::new(dir.path(), sample_acp_step(), None, (1, 2));
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            InspectorOutcome::Next
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
            state.handle_key(&KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE)),
            InspectorOutcome::Prev
        );
    }

    #[test]
    fn wrap_off_hl_scrolls_horizontally_and_arrows_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!("x".repeat(120));
        let mut state = InspectorState::new(dir.path(), step, None, (1, 2));
        state.handle_key(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert!(!state.wrap);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[0].scroll_x, HORIZ_SCROLL_STEP);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            InspectorOutcome::Next
        );
        assert_eq!(state.tabs[0].scroll_x, HORIZ_SCROLL_STEP);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            InspectorOutcome::Prev
        );
        assert_eq!(state.tabs[0].scroll_x, HORIZ_SCROLL_STEP);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[0].scroll_x, 0);
    }

    #[test]
    fn line_cursor_range_and_whole_tab_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!("line 1\nline 2\nline 3\nline 4\nline 5");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));

        // j moves cursor to line 2; y copies line 2.
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[0].cursor, 1);
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            InspectorOutcome::Copy("line 2".to_string())
        );

        // v, j, j, y copies lines 2-4 joined with "\n".
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            InspectorOutcome::Copy("line 2\nline 3\nline 4".to_string())
        );

        // Y copies the whole tab.
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE)),
            InspectorOutcome::Copy("line 1\nline 2\nline 3\nline 4\nline 5".to_string())
        );
    }

    #[test]
    fn wrapped_cursor_line_copies_original_and_highlights_all_three_rows() {
        let dir = tempfile::tempdir().unwrap();
        let long_line = format!("{}{}{}", "a".repeat(16), "b".repeat(16), "c".repeat(8));
        let mut step = sample_acp_step();
        step.output = json!(format!("{long_line}\nshort"));
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        assert!(state.wrap);

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 12)); // body 20x10; gutter 4, content 16
        let area = buf.area;
        state.render(&mut buf, area, &theme, 0);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            InspectorOutcome::Copy(long_line)
        );

        // Body has 4 visual rows (3 for line 1, 1 for line 2), vertically centred at y_off = 3.
        for y in 5..=7 {
            for x in 0..20 {
                assert_eq!(
                    buf[(x, y)].bg,
                    theme.bg_highlight,
                    "expected highlight at ({x}, {y})"
                );
            }
        }
        assert_ne!(buf[(0, 8)].bg, theme.bg_highlight);
    }

    #[test]
    fn esc_ends_visual_range_before_returning_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!("one\ntwo\nthree");
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));

        state.handle_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(state.visual_anchor.is_some());

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert!(state.visual_anchor.is_none());

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InspectorOutcome::Back
        );
    }

    #[test]
    fn tab_switch_keeps_cursor_and_ends_visual_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!("out 1\nout 2\nout 3");
        step.prompt_text = Some("prompt 1\nprompt 2".to_string());
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));

        state.handle_key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        state.handle_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_eq!(state.tabs[0].cursor, 1);
        assert!(state.visual_anchor.is_some());

        // Switch to Prompt tab and back to Output tab.
        state.handle_key(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.active_tab, 1);
        assert!(state.visual_anchor.is_none());
        assert_eq!(state.tabs[1].cursor, 0);

        state.handle_key(&KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
        assert_eq!(state.active_tab, 0);
        assert_eq!(state.tabs[0].cursor, 1);
    }

    #[test]
    fn shift_g_moves_cursor_to_last_line_and_makes_it_visible() {
        let dir = tempfile::tempdir().unwrap();
        let mut step = sample_acp_step();
        step.output = json!(
            (1..=30)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut state = InspectorState::new(dir.path(), step, None, (1, 1));
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10)); // body height = 8
        let area = buf.area;
        state.render(&mut buf, area, &theme, 0);

        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)),
            InspectorOutcome::Changed
        );
        assert_eq!(state.tabs[0].cursor, 29);

        let mut buf = Buffer::empty(area);
        state.render(&mut buf, area, &theme, 0);
        let last_row = row_text(&buf, 9);
        assert!(last_row.contains("line 30"), "last row: {last_row}");
        assert_eq!(buf[(0, 9)].bg, theme.bg_highlight);
    }
}
~~~

## Edit 7: Update STEP_KEYS reference table for step navigation, cursor copy, and 1-7 tabs
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
    (
        "Tab / 1-7",
        "Switch inspector tab (Output / Prompt / Response / Stdout / Stderr / Error / Meta)",
    ),
    ("← / → , [ / ]", "Previous / next step in run"),
    ("j / k / ↑ / ↓", "Move line cursor"),
    ("PgUp / PgDn, g / G", "Page up / down, top / bottom"),
    ("v", "Start / end visual line range"),
    ("y", "Copy cursor line or visual range"),
    ("Y", "Copy whole tab content"),
    ("w", "Toggle line wrapping"),
    ("r", "Replay ACP conversation for this step"),
    ("?", "Toggle this key reference"),
    ("Esc", "End visual range, or back to run view"),
];
~~~

## Edit 8: Add Step-mode test for Left/Right step navigation and Esc landing on the open step
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
    fn step_arrows_navigate_steps_and_esc_lands_on_open_step() {
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

        // Right opens the next step
        let out = state.handle_key(&key(KeyCode::Right));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(1));

        // Left opens the previous step
        let out = state.handle_key(&key(KeyCode::Left));
        assert_eq!(out, AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(0));

        // Move back to step 1, then Esc returns to Run view on step 1
        assert_eq!(state.handle_key(&key(KeyCode::Right)), AcpxOutcome::Changed);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(1));

        state.handle_key(&key(KeyCode::Esc));
        assert_eq!(state.mode(), Mode::Run);

        // Re-opening from Run view confirms step 1 remained selected
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Step);
        assert_eq!(state.inspector.as_ref().map(|(i, _)| *i), Some(1));
    }
~~~
