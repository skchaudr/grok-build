=== FILE: task-scripts/acpx-disclosure-returns/detail-focus.edits.md ===
```markdown
## Edit 1: Add Rows copy-unit storage and DetailPanel focus, cursor, visual range, and copy support
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
pub struct DetailPanel {
    cache_key: Option<CacheKey>,
    /// The picked choice of an answered gate; `Some("")` when it was answered
    /// but the choice is unknown. `None` while the gate is open.
    answered: Option<String>,
    last_target: Option<(PathBuf, DetailTarget)>,
    lines: Vec<Line<'static>>,
    scroll: usize,
    tail_follow: bool,
    last_height: usize,
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
            lines: Vec::new(),
            scroll: 0,
            tail_follow: false,
            last_height: 0,
        }
    }

    /// Tell the panel whether the run's gate has been answered, and with what.
    pub fn set_answered(&mut self, answered: Option<&str>) {
        if self.answered.as_deref() != answered {
            self.answered = answered.map(str::to_string);
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        let max_scroll = self.max_scroll(self.last_height);
        let cur = self.scroll.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_scroll)
        };
        if delta < 0 && next < cur {
            self.tail_follow = false;
        }
        if next >= max_scroll && delta > 0 {
            self.tail_follow = true;
        }
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn max_scroll(&self, height: usize) -> usize {
        if height == 0 {
            self.lines.len().saturating_sub(1)
        } else {
            self.lines.len().saturating_sub(height)
        }
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        run: &Run,
        target: &DetailTarget,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width < 10 || area.height == 0 {
            return;
        }
        self.last_height = area.height as usize;

        let target_pair = (run.summary.dir.clone(), target.clone());
        let target_changed = self.last_target.as_ref() != Some(&target_pair);
        if target_changed {
            self.last_target = Some(target_pair);
            let is_running_node = matches!(
                target,
                DetailTarget::Node(id) if run.node_state(id) == NodeState::Running
            );
            self.tail_follow = is_running_node;
            self.scroll = 0;
        }

        let events_file_len = conversation_file_len(run, target);
        let key = CacheKey {
            run_dir: run.summary.dir.clone(),
            target: target.clone(),
            width: area.width,
            updated_ms: run.summary.updated_ms,
            steps_len: run.steps.len(),
            events_len: run.events.len(),
            events_file_len,
            answered: self.answered.clone(),
        };

        if self.cache_key.as_ref() != Some(&key) {
            self.lines = build_lines(
                run,
                target,
                self.answered.as_deref(),
                (area.width as usize).saturating_sub(2),
                theme,
                now_ms,
            );
            self.cache_key = Some(key);
        }

        let max_scroll = self.max_scroll(area.height as usize);
        if self.tail_follow {
            self.scroll = max_scroll;
        } else if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let height = area.height as usize;
        let total = self.lines.len();
        let needs_scrollbar = total > height;
        let text_width = area.width;

        for row in 0..height {
            let line_idx = self.scroll + row;
            if let Some(line) = self.lines.get(line_idx) {
                buf.set_line(area.x, area.y + row as u16, line, text_width);
            }
        }

        if needs_scrollbar && area.width >= 1 {
            let sb_x = area.x + area.width - 1;
            let thumb_h = ((height * height) / total).max(1).min(height);
            let max_top = height.saturating_sub(thumb_h);
            let thumb_top = if max_scroll == 0 {
                0
            } else {
                (self.scroll * max_top) / max_scroll
            };
            let sb_style = Style::default().fg(theme.gray_dim);
            for r in 0..thumb_h {
                let y = area.y + (thumb_top + r) as u16;
                if y < area.y + area.height {
                    buf.set_stringn(sb_x, y, "█", 1, sb_style);
                }
            }
        }
    }
}
~~~
NEW:
~~~rust
#[derive(Debug, Clone)]
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

    pub fn scroll(&mut self, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        let max_scroll = self.max_scroll(self.last_height);
        let cur = self.scroll.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_scroll)
        };
        if delta < 0 && next < cur {
            self.tail_follow = false;
        }
        if next >= max_scroll && delta > 0 {
            self.tail_follow = true;
        }
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn max_scroll(&self, height: usize) -> usize {
        if height == 0 {
            self.lines.len().saturating_sub(1)
        } else {
            self.lines.len().saturating_sub(height)
        }
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        run: &Run,
        target: &DetailTarget,
        theme: &Theme,
        now_ms: i64,
    ) {
        if area.width < 10 || area.height == 0 {
            return;
        }
        self.last_height = area.height as usize;

        let target_pair = (run.summary.dir.clone(), target.clone());
        let target_changed = self.last_target.as_ref() != Some(&target_pair);
        if target_changed {
            self.last_target = Some(target_pair);
            let is_running_node = matches!(
                target,
                DetailTarget::Node(id) if run.node_state(id) == NodeState::Running
            );
            self.tail_follow = is_running_node;
            self.scroll = 0;
            self.cursor = 0;
            self.visual_anchor = None;
        }

        let events_file_len = conversation_file_len(run, target);
        let key = CacheKey {
            run_dir: run.summary.dir.clone(),
            target: target.clone(),
            width: area.width,
            updated_ms: run.summary.updated_ms,
            steps_len: run.steps.len(),
            events_len: run.events.len(),
            events_file_len,
            answered: self.answered.clone(),
        };

        if self.cache_key.as_ref() != Some(&key) {
            self.lines = build_lines(
                run,
                target,
                self.answered.as_deref(),
                (area.width as usize).saturating_sub(2),
                theme,
                now_ms,
            );
            self.cache_key = Some(key);
        }

        let max_scroll = self.max_scroll(area.height as usize);
        if self.lines.is_empty() {
            self.cursor = 0;
            self.visual_anchor = None;
        } else {
            let last_row = self.lines.len() - 1;
            self.cursor = self.cursor.min(last_row);
            if let Some(anchor) = self.visual_anchor.as_mut() {
                *anchor = (*anchor).min(last_row);
            }
        }
        if self.focused {
            self.ensure_cursor_visible(area.height as usize);
        } else if self.tail_follow {
            self.scroll = max_scroll;
        } else if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let height = area.height as usize;
        let total = self.lines.len();
        let needs_scrollbar = total > height;
        let text_width = area.width;
        let highlight = self.selected_range();

        for row in 0..height {
            let line_idx = self.scroll + row;
            let y = area.y + row as u16;
            if let Some(line) = self.lines.get(line_idx) {
                buf.set_line(area.x, y, line, text_width);
            }
            if highlight.is_some_and(|(s, e)| line_idx >= s && line_idx <= e) {
                let hl_style = Style::default().bg(theme.bg_highlight);
                for x in area.x..area.x + area.width {
                    buf[(x, y)].set_style(hl_style);
                }
            }
        }

        if needs_scrollbar && area.width >= 1 {
            let sb_x = area.x + area.width - 1;
            let thumb_h = ((height * height) / total).max(1).min(height);
            let max_top = height.saturating_sub(thumb_h);
            let thumb_top = if max_scroll == 0 {
                0
            } else {
                (self.scroll * max_top) / max_scroll
            };
            let sb_style = Style::default().fg(theme.gray_dim);
            for r in 0..thumb_h {
                let y = area.y + (thumb_top + r) as u16;
                if y < area.y + area.height {
                    buf.set_stringn(sb_x, y, "█", 1, sb_style);
                }
            }
        }
    }
}
~~~

## Edit 2: Return Rows from build_lines and build_run_lines
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn build_lines(
    run: &Run,
    target: &DetailTarget,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    if width < 10 {
        return Vec::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines_at(run, *i, step, answered, width, theme, now_ms)
            } else {
                vec![dim_line("step not found", theme)]
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}

fn build_run_lines(
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let status = run.summary.status;
    // A waiting run whose gate is answered reads as done, like the run header.
    let shown = if status == RunStatus::Waiting && answered.is_some() {
        RunStatus::Completed
~~~
NEW:
~~~rust
fn build_lines(
    run: &Run,
    target: &DetailTarget,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Rows {
    if width < 10 {
        return Rows::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines_at(run, *i, step, answered, width, theme, now_ms)
            } else {
                let mut rows = Rows::new();
                rows.push(dim_line("step not found", theme));
                rows
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}

fn build_run_lines(
    run: &Run,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Rows {
    let mut out = Rows::new();
    let status = run.summary.status;
    // A waiting run whose gate is answered reads as done, like the run header.
    let shown = if status == RunStatus::Waiting && answered.is_some() {
        RunStatus::Completed
~~~

## Edit 3: Preserve copy units when appending value lines in build_run_lines
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
    // Last result
    if let Some(last_step) = run.steps.last() {
        let lines = format_value_lines(&last_step.output, width, theme);
        if !lines.is_empty() {
            push_section_rule(&mut out, "Last result", width, theme);
            out.extend(lines);
        }
    }

    // Input
    let input_lines = format_value_lines(&run.input, width, theme);
    if !input_lines.is_empty() {
        push_section_rule(&mut out, "Input", width, theme);
        out.extend(
            input_lines,
        );
    }

    out
}
~~~
NEW:
~~~rust
    // Last result
    if let Some(last_step) = run.steps.last() {
        let lines = format_value_lines(&last_step.output, width, theme);
        if !lines.is_empty() {
            push_section_rule(&mut out, "Last result", width, theme);
            out.append_rows(lines);
        }
    }

    // Input
    let input_lines = format_value_lines(&run.input, width, theme);
    if !input_lines.is_empty() {
        push_section_rule(&mut out, "Input", width, theme);
        out.append_rows(input_lines);
    }

    out
}
~~~

## Edit 4: Record copy units in format_value_lines, append_value_lines, and build_step_lines builders
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if matches!(val, Value::Null) {
        return out;
    }
    if let Value::String(s) = val {
        if s.trim().is_empty() {
            return out;
        }
    }
    append_value_lines(&mut out, val, 0, width, theme);
    out
}

fn append_value_lines(
    out: &mut Vec<Line<'static>>,
    val: &Value,
    indent: usize,
    width: usize,
    theme: &Theme,
) {
    let val_style = Style::default().fg(theme.text_primary);
    let key_style = Style::default().fg(theme.gray);
    let pad_indent = " ".repeat(indent);
    let body_w = width.saturating_sub(indent).max(1);

    match val {
        Value::Null => {
            if indent > 0 {
                out.push(Line::from(vec![
                    Span::raw(pad_indent),
                    Span::styled("null", val_style),
                ]));
            }
        }
        Value::Bool(b) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(b.to_string(), val_style),
            ]));
        }
        Value::Number(n) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(n.to_string(), val_style),
            ]));
        }
        Value::String(s) => {
            for line in wrap(s, body_w) {
                if indent == 0 {
                    out.push(Line::from(Span::styled(line, val_style)));
                } else {
                    out.push(Line::from(vec![
                        Span::raw(pad_indent.clone()),
                        Span::styled(line, val_style),
                    ]));
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                let bullet_prefix = format!("{pad_indent}- ");
                let item_w = width.saturating_sub(indent + 2).max(1);
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        let mut sub = Vec::new();
                        append_value_lines(&mut sub, item, 0, item_w, theme);
                        if sub.is_empty() {
                            sub.push(Line::from(Span::styled(
                                EMPTY,
                                Style::default().fg(theme.gray),
                            )));
                        }
                        for (idx, sub_line) in sub.into_iter().enumerate() {
                            let mut spans = vec![Span::styled(
                                if idx == 0 {
                                    bullet_prefix.clone()
                                } else {
                                    format!("{pad_indent}  ")
                                },
                                key_style,
                            )];
                            spans.extend(sub_line.spans);
                            out.push(Line::from(spans));
                        }
                    }
                    _ => {
                        let s = match item {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let wrapped = wrap(&s, item_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            let pfx = if idx == 0 {
                                bullet_prefix.clone()
                            } else {
                                format!("{pad_indent}  ")
                            };
                            out.push(Line::from(vec![
                                Span::styled(pfx, key_style),
                                Span::styled(line, val_style),
                            ]));
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            let max_key_w = map
                .keys()
                .map(|k| UnicodeWidthStr::width(k.as_str()))
                .max()
                .unwrap_or(0);
            for (k, v) in map {
                let kw = UnicodeWidthStr::width(k.as_str());
                let key_pad = " ".repeat(max_key_w.saturating_sub(kw) + 2);
                match v {
                    _ if is_empty_container(v) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(format!("{k}{key_pad}"), key_style),
                            Span::styled(EMPTY, Style::default().fg(theme.gray)),
                        ]));
                    }
                    Value::Object(_) | Value::Array(_) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(k.clone(), key_style),
                        ]));
                        append_value_lines(out, v, indent + 2, width, theme);
                    }
                    _ => {
                        let s = match v {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let prefix_w = indent + max_key_w + 2;
                        let val_w = width.saturating_sub(prefix_w).max(1);
                        let wrapped = wrap(&s, val_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            if idx == 0 {
                                out.push(Line::from(vec![
                                    Span::raw(pad_indent.clone()),
                                    Span::styled(format!("{k}{key_pad}"), key_style),
                                    Span::styled(line, val_style),
                                ]));
                            } else {
                                out.push(Line::from(vec![
                                    Span::raw(" ".repeat(prefix_w)),
                                    Span::styled(line, val_style),
                                ]));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn first_non_empty_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn build_step_lines(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    build_step_lines_at(
        run,
        step_idx,
        step,
        answered,
        width,
        theme,
        run.summary.updated_ms,
    )
}

fn build_step_lines_at(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
~~~
NEW:
~~~rust
fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Rows {
    let mut out = Rows::new();
    if matches!(val, Value::Null) {
        return out;
    }
    if let Value::String(s) = val {
        if s.trim().is_empty() {
            return out;
        }
    }
    append_value_lines(&mut out, val, 0, width, theme);
    out
}

fn append_value_lines(
    out: &mut Rows,
    val: &Value,
    indent: usize,
    width: usize,
    theme: &Theme,
) {
    let val_style = Style::default().fg(theme.text_primary);
    let key_style = Style::default().fg(theme.gray);
    let pad_indent = " ".repeat(indent);
    let body_w = width.saturating_sub(indent).max(1);

    match val {
        Value::Null => {
            if indent > 0 {
                out.push(Line::from(vec![
                    Span::raw(pad_indent),
                    Span::styled("null", val_style),
                ]));
            }
        }
        Value::Bool(b) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(b.to_string(), val_style),
            ]));
        }
        Value::Number(n) => {
            out.push(Line::from(vec![
                Span::raw(pad_indent),
                Span::styled(n.to_string(), val_style),
            ]));
        }
        Value::String(s) => {
            for src_line in source_lines(s) {
                let rows: Vec<Line<'static>> = wrap(src_line, body_w)
                    .into_iter()
                    .map(|line| {
                        if indent == 0 {
                            Line::from(Span::styled(line, val_style))
                        } else {
                            Line::from(vec![
                                Span::raw(pad_indent.clone()),
                                Span::styled(line, val_style),
                            ])
                        }
                    })
                    .collect();
                out.push_unit(rows, src_line.to_string());
            }
        }
        Value::Array(arr) => {
            for item in arr {
                let bullet_prefix = format!("{pad_indent}- ");
                let item_w = width.saturating_sub(indent + 2).max(1);
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        let mut sub = Rows::new();
                        append_value_lines(&mut sub, item, 0, item_w, theme);
                        if sub.is_empty() {
                            sub.push(Line::from(Span::styled(
                                EMPTY,
                                Style::default().fg(theme.gray),
                            )));
                        }
                        let offset = out.len();
                        let Rows { lines, units } = sub;
                        for (idx, sub_line) in lines.into_iter().enumerate() {
                            let mut spans = vec![Span::styled(
                                if idx == 0 {
                                    bullet_prefix.clone()
                                } else {
                                    format!("{pad_indent}  ")
                                },
                                key_style,
                            )];
                            spans.extend(sub_line.spans);
                            out.push(Line::from(spans));
                        }
                        for mut unit in units {
                            unit.start += offset;
                            unit.end += offset;
                            out.units.push(unit);
                        }
                    }
                    _ => {
                        let s = match item {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let wrapped = wrap(&s, item_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            let pfx = if idx == 0 {
                                bullet_prefix.clone()
                            } else {
                                format!("{pad_indent}  ")
                            };
                            out.push(Line::from(vec![
                                Span::styled(pfx, key_style),
                                Span::styled(line, val_style),
                            ]));
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            let max_key_w = map
                .keys()
                .map(|k| UnicodeWidthStr::width(k.as_str()))
                .max()
                .unwrap_or(0);
            for (k, v) in map {
                let kw = UnicodeWidthStr::width(k.as_str());
                let key_pad = " ".repeat(max_key_w.saturating_sub(kw) + 2);
                match v {
                    _ if is_empty_container(v) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(format!("{k}{key_pad}"), key_style),
                            Span::styled(EMPTY, Style::default().fg(theme.gray)),
                        ]));
                    }
                    Value::Object(_) | Value::Array(_) => {
                        out.push(Line::from(vec![
                            Span::raw(pad_indent.clone()),
                            Span::styled(k.clone(), key_style),
                        ]));
                        append_value_lines(out, v, indent + 2, width, theme);
                    }
                    _ => {
                        let s = match v {
                            Value::Null => "null".to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        let prefix_w = indent + max_key_w + 2;
                        let val_w = width.saturating_sub(prefix_w).max(1);
                        let wrapped = wrap(&s, val_w);
                        for (idx, line) in wrapped.into_iter().enumerate() {
                            if idx == 0 {
                                out.push(Line::from(vec![
                                    Span::raw(pad_indent.clone()),
                                    Span::styled(format!("{k}{key_pad}"), key_style),
                                    Span::styled(line, val_style),
                                ]));
                            } else {
                                out.push(Line::from(vec![
                                    Span::raw(" ".repeat(prefix_w)),
                                    Span::styled(line, val_style),
                                ]));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn first_non_empty_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn build_step_lines(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Rows {
    build_step_lines_at(
        run,
        step_idx,
        step,
        answered,
        width,
        theme,
        run.summary.updated_ms,
    )
}

fn build_step_lines_at(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Rows {
    let mut out = Rows::new();
~~~

## Edit 5: Preserve copy units when appending result lines in build_step_lines_at
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
    // Result
    let show_result = !suppress_agent_dumps
        || has_extra_result_fields(run, step, note.as_ref().is_some_and(|(_, full)| *full));
    if show_result {
        let result_lines = format_value_lines(&step.output, width, theme);
        if !result_lines.is_empty() {
            push_section_rule(&mut out, "Result", width, theme);
            out.extend(result_lines);
        }
    }
~~~
NEW:
~~~rust
    // Result
    let show_result = !suppress_agent_dumps
        || has_extra_result_fields(run, step, note.as_ref().is_some_and(|(_, full)| *full));
    if show_result {
        let result_lines = format_value_lines(&step.output, width, theme);
        if !result_lines.is_empty() {
            push_section_rule(&mut out, "Result", width, theme);
            out.append_rows(result_lines);
        }
    }
~~~

## Edit 6: Update append_artifact_section and build_node_lines to use Rows
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn append_artifact_section(
    out: &mut Vec<Line<'static>>,
    title: &str,
    run_dir: &Path,
    artifact: &bundle::ArtifactRef,
    width: usize,
    theme: &Theme,
) {
    const MAX_BYTES: usize = 64 * 1024;
    match bundle::read_artifact(run_dir, artifact, MAX_BYTES) {
        Ok(text) => {
            if text.is_empty() {
                return;
            }
            push_section_rule(out, title, width, theme);
            let raw_lines: Vec<&str> = text.lines().collect();
            let start = raw_lines.len().saturating_sub(30);
            for raw in &raw_lines[start..] {
                push_wrapped(out, raw, width, Style::default().fg(theme.text_primary));
            }
            if artifact.bytes > MAX_BYTES as u64 {
                out.push(dim_line("… output cut at 64 KiB", theme));
            }
        }
        Err(_) => {
            push_section_rule(out, title, width, theme);
            out.push(dim_line("artifact unreadable", theme));
        }
    }
}

fn build_node_lines(
    run: &Run,
    node_id: &str,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
~~~
NEW:
~~~rust
fn append_artifact_section(
    out: &mut Rows,
    title: &str,
    run_dir: &Path,
    artifact: &bundle::ArtifactRef,
    width: usize,
    theme: &Theme,
) {
    const MAX_BYTES: usize = 64 * 1024;
    match bundle::read_artifact(run_dir, artifact, MAX_BYTES) {
        Ok(text) => {
            if text.is_empty() {
                return;
            }
            push_section_rule(out, title, width, theme);
            let raw_lines: Vec<&str> = text.lines().collect();
            let start = raw_lines.len().saturating_sub(30);
            for raw in &raw_lines[start..] {
                push_wrapped(out, raw, width, Style::default().fg(theme.text_primary));
            }
            if artifact.bytes > MAX_BYTES as u64 {
                out.push(dim_line("… output cut at 64 KiB", theme));
            }
        }
        Err(_) => {
            push_section_rule(out, title, width, theme);
            out.push(dim_line("artifact unreadable", theme));
        }
    }
}

fn build_node_lines(
    run: &Run,
    node_id: &str,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Rows {
    let mut out = Rows::new();
~~~

## Edit 7: Update append_gate_section signature to take &mut Rows
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
fn append_gate_section(
    out: &mut Rows,
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
~~~

## Edit 8: Update append_conversation signature to take &mut Rows
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn append_conversation(
    out: &mut Vec<Line<'static>>,
    path: &Path,
    seq_range: Option<(u64, u64)>,
    min_exclusive_seq: Option<u64>,
    width: usize,
    theme: &Theme,
) {
~~~
NEW:
~~~rust
fn append_conversation(
    out: &mut Rows,
    path: &Path,
    seq_range: Option<(u64, u64)>,
    min_exclusive_seq: Option<u64>,
    width: usize,
    theme: &Theme,
) {
~~~

## Edit 9: Record plain-text copy units in push_section_rule, push_wrapped, and push_wrapped_capped
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn push_section_rule(
    out: &mut Vec<Line<'static>>,
    name: &str,
    width: usize,
    theme: &Theme,
) {
    out.push(Line::default());
    let prefix = "── ";
    let mid = format!("{name} ");
    let used = UnicodeWidthStr::width(prefix) + UnicodeWidthStr::width(mid.as_str());
    let rem = width.saturating_sub(used);
    let trail = "─".repeat(rem);
    out.push(Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(
            mid,
            Style::default()
                .fg(theme.gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(trail, Style::default().fg(theme.gray_dim)),
    ]));
}

fn title_row_right(
    glyph: &str,
    glyph_color: Color,
    title: &str,
    right: &str,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let right_w = UnicodeWidthStr::width(right);
    let left_max = if right_w > 0 {
        width.saturating_sub(right_w + 2)
    } else {
        width
    };
    let glyph_part = format!("{glyph} ");
    let glyph_w = UnicodeWidthStr::width(glyph_part.as_str());
    let title_cut = truncate_str(title, left_max.saturating_sub(glyph_w));
    let left_used = glyph_w + UnicodeWidthStr::width(title_cut.as_str());

    let mut spans = vec![
        Span::styled(
            glyph_part,
            Style::default().fg(glyph_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title_cut,
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if right_w > 0 && left_used + 1 + right_w <= width {
        let pad = " ".repeat(width - left_used - right_w);
        spans.push(Span::raw(pad));
        spans.push(Span::styled(
            right.to_string(),
            Style::default().fg(theme.gray),
        ));
    }
    Line::from(spans)
}

fn push_wrapped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    style: Style,
) {
    for line in wrap(text, width) {
        out.push(Line::from(Span::styled(line, style)));
    }
}

fn push_wrapped_capped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    max_rows: usize,
    style: Style,
    theme: &Theme,
) {
    let wrapped = wrap(text, width);
    let total = wrapped.len();
    for line in wrapped.into_iter().take(max_rows) {
        out.push(Line::from(Span::styled(line, style)));
    }
    if total > max_rows {
        let more = total - max_rows;
        out.push(dim_line(&format!("… +{more} more rows"), theme));
    }
}
~~~
NEW:
~~~rust
fn push_section_rule(
    out: &mut Rows,
    name: &str,
    width: usize,
    theme: &Theme,
) {
    out.push(Line::default());
    let prefix = "── ";
    let mid = format!("{name} ");
    let used = UnicodeWidthStr::width(prefix) + UnicodeWidthStr::width(mid.as_str());
    let rem = width.saturating_sub(used);
    let trail = "─".repeat(rem);
    let rule = Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.gray_dim)),
        Span::styled(
            mid,
            Style::default()
                .fg(theme.gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(trail, Style::default().fg(theme.gray_dim)),
    ]);
    out.push_unit(vec![rule], name.to_string());
}

fn title_row_right(
    glyph: &str,
    glyph_color: Color,
    title: &str,
    right: &str,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let right_w = UnicodeWidthStr::width(right);
    let left_max = if right_w > 0 {
        width.saturating_sub(right_w + 2)
    } else {
        width
    };
    let glyph_part = format!("{glyph} ");
    let glyph_w = UnicodeWidthStr::width(glyph_part.as_str());
    let title_cut = truncate_str(title, left_max.saturating_sub(glyph_w));
    let left_used = glyph_w + UnicodeWidthStr::width(title_cut.as_str());

    let mut spans = vec![
        Span::styled(
            glyph_part,
            Style::default().fg(glyph_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title_cut,
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if right_w > 0 && left_used + 1 + right_w <= width {
        let pad = " ".repeat(width - left_used - right_w);
        spans.push(Span::raw(pad));
        spans.push(Span::styled(
            right.to_string(),
            Style::default().fg(theme.gray),
        ));
    }
    Line::from(spans)
}

fn push_wrapped(
    out: &mut Rows,
    text: &str,
    width: usize,
    style: Style,
) {
    for src_line in source_lines(text) {
        let rows: Vec<Line<'static>> = wrap(src_line, width)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, style)))
            .collect();
        out.push_unit(rows, src_line.to_string());
    }
}

fn push_wrapped_capped(
    out: &mut Rows,
    text: &str,
    width: usize,
    max_rows: usize,
    style: Style,
    theme: &Theme,
) {
    let mut emitted = 0usize;
    let mut total = 0usize;
    for src_line in source_lines(text) {
        let wrapped = wrap(src_line, width);
        let line_rows = wrapped.len();
        total += line_rows;
        if emitted < max_rows {
            let take = (max_rows - emitted).min(line_rows);
            let rows: Vec<Line<'static>> = wrapped
                .into_iter()
                .take(take)
                .map(|line| Line::from(Span::styled(line, style)))
                .collect();
            emitted += rows.len();
            out.push_unit(rows, src_line.to_string());
        }
    }
    if total > max_rows {
        let more = total - max_rows;
        out.push(dim_line(&format!("… +{more} more rows"), theme));
    }
}
~~~

## Edit 10: Add DetailPanel focus, cursor, wrapped copy-unit, and visual-range tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
    fn test_target_change_resets_scroll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm".to_string()),
            raw_text: None,
            output: json!({"res": "1\n2\n3\n4\n5\n6\n7\n8"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll(5));
        assert!(panel.scroll > 0);

        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, 0);
    }
~~~
NEW:
~~~rust
    fn test_target_change_resets_scroll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm".to_string()),
            raw_text: None,
            output: json!({"res": "1\n2\n3\n4\n5\n6\n7\n8"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll(5));
        assert!(panel.scroll > 0);

        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, 0);
    }

    #[test]
    fn wrapped_failure_line_copies_exact_source_on_second_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        let long_path =
            "/workspace/crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs";
        assert!(long_path.len() >= 70);
        let err_rel = "artifacts/stderr.txt";
        let err_path = tmp.path().join(err_rel);
        std::fs::create_dir_all(err_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&err_path, format!("{long_path}\nsecond failure line\n")).expect("write");

        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            outcome: Outcome::Error,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!(long_path),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: Some(ArtifactRef {
                path: err_rel.to_string(),
                bytes: (long_path.len() + 21) as u64,
                sha256: None,
            }),
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let area = Rect::new(0, 0, 32, 30);
        let mut buf = Buffer::empty(area);
        panel.render(
            &mut buf,
            area,
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        panel.set_focused(true);

        // Find the wrapped rows produced for the 70-character path in Result and stderr.
        let matching_rows: Vec<usize> = panel
            .lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                let text = line_plain_text(line);
                if text.starts_with("/workspace/crates/codegen/") {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();
        assert!(!matching_rows.is_empty(), "expected wrapped path rows");
        for first_row in matching_rows {
            // Second wrapped row belongs to the same copy unit as the first row.
            panel.cursor = first_row + 1;
            assert_eq!(panel.copy_selection().as_deref(), Some(long_path));
        }

        // Visual range over the wrapped path unit and the next unit copies each unit once.
        let stderr_path_row = panel
            .lines
            .iter()
            .enumerate()
            .rposition(|(_, line)| line_plain_text(line).starts_with("/workspace/crates/codegen/"))
            .expect("stderr path row");
        let second_line_row = panel
            .lines
            .iter()
            .position(|line| line_plain_text(line) == "second failure line")
            .expect("second failure line row");
        assert!(second_line_row > stderr_path_row + 1);
        panel.cursor = stderr_path_row + 1;
        assert!(panel.toggle_visual());
        panel.cursor = second_line_row;
        assert_eq!(
            panel.copy_selection().as_deref(),
            Some(format!("{long_path}\nsecond failure line").as_str())
        );

        // Copying all units omits section-rule dashes.
        let all = panel.copy_all().expect("copy all");
        assert!(all.contains(long_path));
        assert!(all.contains("Result"));
        assert!(all.contains("stderr"));
        assert!(!all.contains('─'), "copied text must not include rule dashes:\n{all}");
    }

    #[test]
    fn focus_highlight_visual_range_and_target_change_reset_cursor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: json!("first\nsecond\nthird"),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let theme = Theme::default();
        let area = Rect::new(0, 0, 50, 12);
        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(area);

        // Unfocused: no row receives bg_highlight.
        panel.render(&mut buf, area, &run, &DetailTarget::Step(0), &theme, 5_000);
        for y in 0..area.height {
            assert_ne!(buf[(0, y)].bg, theme.bg_highlight);
        }

        // Focused with visual range across rows 1..=2 highlights rows 1 and 2 only.
        panel.set_focused(true);
        assert!(panel.move_cursor(1));
        assert!(panel.toggle_visual());
        assert!(panel.move_cursor(1));
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Step(0), &theme, 5_000);
        assert_ne!(buf[(0, 0)].bg, theme.bg_highlight);
        assert_eq!(buf[(0, 1)].bg, theme.bg_highlight);
        assert_eq!(buf[(area.width - 1, 1)].bg, theme.bg_highlight);
        assert_eq!(buf[(0, 2)].bg, theme.bg_highlight);
        assert_ne!(buf[(0, 3)].bg, theme.bg_highlight);

        // Switching target resets cursor to 0 and ends visual mode.
        let mut buf = Buffer::empty(area);
        panel.render(&mut buf, area, &run, &DetailTarget::Run, &theme, 5_000);
        assert_eq!(panel.cursor, 0);
        assert!(!panel.is_visual());
        assert_eq!(buf[(0, 0)].bg, theme.bg_highlight);
        assert_ne!(buf[(0, 1)].bg, theme.bg_highlight);
    }
~~~

## Edit 11: Reset detail focus in open_run
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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

## Edit 12: Route Tab, Shift+Tab, and detail-focused keys in handle_run_key
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
    fn handle_run_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some(rv) = self.run_view.as_mut() else {
            self.detail.set_focused(false);
            self.mode = Mode::Ledger;
            return AcpxOutcome::Changed;
        };
        let captures = rv.captures_keys();
        let has_detail = rv.detail_area().is_some();

        if !captures && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            if has_detail {
                self.detail.set_focused(!self.detail.is_focused());
                return AcpxOutcome::Changed;
            }
            return AcpxOutcome::Unchanged;
        }

        if !captures && self.detail.is_focused() {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    return if self.detail.move_cursor(1) {
                        AcpxOutcome::Changed
                    } else {
                        AcpxOutcome::Unchanged
                    };
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    return if self.detail.move_cursor(-1) {
                        AcpxOutcome::Changed
                    } else {
                        AcpxOutcome::Unchanged
                    };
                }
                KeyCode::PageDown => {
                    let step = self.detail.page_step();
                    return if self.detail.move_cursor(step) {
                        AcpxOutcome::Changed
                    } else {
                        AcpxOutcome::Unchanged
                    };
                }
                KeyCode::PageUp => {
                    let step = self.detail.page_step();
                    return if self.detail.move_cursor(-step) {
                        AcpxOutcome::Changed
                    } else {
                        AcpxOutcome::Unchanged
                    };
                }
                KeyCode::Char('v') => {
                    return if self.detail.toggle_visual() {
                        AcpxOutcome::Changed
                    } else {
                        AcpxOutcome::Unchanged
                    };
                }
                KeyCode::Char('y') => {
                    return match self.detail.copy_selection() {
                        Some(text) => AcpxOutcome::Copy(text),
                        None => AcpxOutcome::Unchanged,
                    };
                }
                KeyCode::Char('Y') => {
                    return match self.detail.copy_all() {
                        Some(text) => AcpxOutcome::Copy(text),
                        None => AcpxOutcome::Unchanged,
                    };
                }
                KeyCode::Esc => {
                    if self.detail.handle_esc() {
                        return AcpxOutcome::Changed;
                    }
                }
                _ => {}
            }
        }

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
    }
~~~

## Edit 13: Show detail pane footer shortcuts when detail pane has focus
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
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

## Edit 14: Clear detail focus if Mode::Run has no detail area
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
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
~~~

## Edit 15: Document Tab and detail pane keys in RUN_KEYS
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
OLD:
~~~rust
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
~~~
NEW:
~~~rust
const RUN_KEYS: &[(&str, &str)] = &[
    ("j / k / ↑ / ↓", "Move through stages and their substeps"),
    ("[ / ]", "Previous / next stage"),
    ("Tab / Shift+Tab", "Switch focus between stages and the detail panel"),
    ("J / K, D / U", "Scroll the detail panel by a few rows / by a page"),
    ("Detail: j / k, PgUp / PgDn", "Move the detail cursor by a row / a page"),
    ("Detail: v, y / Y", "Select line range; copy line/range or whole panel"),
    ("Enter", "Inspect the selected stage; on a gate row, answer it"),
    ("1-9", "Answer the gate with that choice (asks first)"),
    ("F5", "Jump to the gate"),
    ("Space, h / l, E", "Fold or unfold a stage; E folds all"),
    ("c / !", "Jump to the current stage / the next failed one"),
    ("v / n", "Show or hide the graph / heartbeat noise"),
    ("r", "Replay the stage's conversation as a transcript"),
    ("y", "Copy the stage's output"),
    ("?", "Toggle this key reference"),
    ("Esc", "End detail selection / focus, or back to the list"),
];
~~~

## Edit 16: Add Run-mode detail focus routing tests in acpx_modal.rs
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
    fn detail_focus_tab_backtab_and_without_detail_area() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);

        // Before rendering, detail_area is None so Tab does nothing.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Unchanged);
        assert!(!state.detail.is_focused());

        // Render wide enough for the detail pane.
        let area = Rect::new(0, 0, 140, 35);
        let mut buf = Buffer::empty(area);
        render_acpx_modal(&mut buf, area, &mut state, false);
        assert!(state.run_view.as_ref().and_then(|rv| rv.detail_area()).is_some());

        // Tab focuses the detail pane and updates footer shortcuts.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        assert!(state.detail.is_focused());
        let labels = state.active_shortcut_labels();
        assert!(labels.contains(&"j/k line"));
        assert!(labels.contains(&"Esc tree"));

        // Shift+Tab returns focus to the stage tree.
        assert_eq!(state.handle_key(&key(KeyCode::BackTab)), AcpxOutcome::Changed);
        assert!(!state.detail.is_focused());
        assert!(!state.active_shortcut_labels().contains(&"Esc tree"));
    }

    #[test]
    fn detail_focus_range_copy_all_esc_unwind_and_tree_keys() {
        let fx = write_fixture("completed");
        let mut state =
            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
        state.handle_key(&key(KeyCode::Enter));
        assert_eq!(state.mode(), Mode::Run);
        if let Some(rv) = state.run_view.as_mut() {
            rv.select_step(0);
        }

        let area = Rect::new(0, 0, 140, 35);
        let mut buf = Buffer::empty(area);
        render_acpx_modal(&mut buf, area, &mut state, false);

        // With the tree focused, y copies the stage output and v toggles the graph.
        let tree_copy = state.handle_key(&key(KeyCode::Char('y')));
        assert!(matches!(tree_copy, AcpxOutcome::Copy(_)));
        assert_eq!(state.handle_key(&key(KeyCode::Char('v'))), AcpxOutcome::Changed);
        assert!(!state.detail.is_focused());
        assert!(!state.detail.is_visual());
        // Toggle graph back so detail layout stays consistent.
        state.handle_key(&key(KeyCode::Char('v')));
        render_acpx_modal(&mut buf, area, &mut state, false);

        // Focus detail pane, move cursor, start visual range, move down, copy 2 units.
        assert_eq!(state.handle_key(&key(KeyCode::Tab)), AcpxOutcome::Changed);
        assert_eq!(state.handle_key(&key(KeyCode::Char('v'))), AcpxOutcome::Changed);
        assert!(state.detail.is_visual());
        assert_eq!(state.handle_key(&key(KeyCode::Char('j'))), AcpxOutcome::Changed);
        let range_copy = state.handle_key(&key(KeyCode::Char('y')));
        match range_copy {
            AcpxOutcome::Copy(text) => {
                assert_eq!(text.lines().count(), 2, "expected 2 copied units:\n{text}");
                assert!(!text.contains('─'));
            }
            other => panic!("expected Copy, got {other:?}"),
        }

        // Y copies the whole pane without section-rule dashes.
        let all_copy = state.handle_key(&key(KeyCode::Char('Y')));
        match all_copy {
            AcpxOutcome::Copy(text) => {
                assert!(!text.is_empty());
                assert!(!text.contains('─'), "unexpected rule dash in:\n{text}");
            }
            other => panic!("expected Copy, got {other:?}"),
        }

        // First Esc ends visual mode while keeping detail focus.
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert!(state.detail.is_focused());
        assert!(!state.detail.is_visual());

        // Second Esc returns focus to the stage tree.
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert!(!state.detail.is_focused());
        assert_eq!(state.mode(), Mode::Run);

        // Third Esc leaves the run, landing on the same run in the ledger.
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Ledger);
        assert!(!state.detail.is_focused());
        assert_eq!(state.handle_key(&key(KeyCode::Enter)), AcpxOutcome::Changed);
        assert_eq!(state.mode(), Mode::Run);
        assert_eq!(
            state.run_view.as_ref().map(|rv| rv.run().summary.run_id.as_str()),
            Some("run-001")
        );
    }
~~~
```