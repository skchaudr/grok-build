```rust
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::acpx::bundle::{RunStatus, RunSummary, fmt_duration};
use crate::acpx::sessions::SessionSummary;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerTab {
    Flows,
    Sessions,
}

impl LedgerTab {
    fn toggle(self) -> Self {
        match self {
            LedgerTab::Flows => LedgerTab::Sessions,
            LedgerTab::Sessions => LedgerTab::Flows,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Recent,
    Status,
    Name,
}

impl SortOrder {
    fn next(self) -> Self {
        match self {
            SortOrder::Recent => SortOrder::Status,
            SortOrder::Status => SortOrder::Name,
            SortOrder::Name => SortOrder::Recent,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortOrder::Recent => "recent",
            SortOrder::Status => "status",
            SortOrder::Name => "name",
        }
    }
}

#[derive(Debug, Clone)]
struct TabState {
    selected: usize,
    scroll: usize,
    filter: String,
    active_only: bool,
    sort: SortOrder,
    visible_indices: Vec<usize>,
}

impl TabState {
    fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            filter: String::new(),
            active_only: false,
            sort: SortOrder::Recent,
            visible_indices: Vec::new(),
        }
    }

    fn clamp(&mut self) {
        if self.visible_indices.is_empty() {
            self.selected = 0;
            self.scroll = 0;
        } else if self.selected >= self.visible_indices.len() {
            self.selected = self.visible_indices.len() - 1;
        }
    }

    fn adjust_scroll(&mut self, height: usize) {
        self.clamp();
        if height == 0 || self.visible_indices.is_empty() {
            self.scroll = 0;
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
        let max_scroll = self.visible_indices.len().saturating_sub(height);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }
}

pub struct LedgerState {
    tab: LedgerTab,
    runs: Vec<RunSummary>,
    sessions: Vec<SessionSummary>,
    flows_tab: TabState,
    sessions_tab: TabState,
    filter_open: bool,
    last_list_height: usize,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerState {
    pub fn new() -> Self {
        Self {
            tab: LedgerTab::Flows,
            runs: Vec::new(),
            sessions: Vec::new(),
            flows_tab: TabState::new(),
            sessions_tab: TabState::new(),
            filter_open: false,
            last_list_height: 10,
        }
    }

    pub fn set_runs(&mut self, runs: Vec<RunSummary>) {
        let prev_id = self.selected_run().map(|r| r.run_id.clone());
        self.runs = runs;
        self.recompute_flows(prev_id.as_deref());
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        let prev_id = self.selected_session().map(|s| s.record_id.clone());
        self.sessions = sessions;
        self.recompute_sessions(prev_id.as_deref());
    }

    pub fn tab(&self) -> LedgerTab {
        self.tab
    }

    pub fn any_live(&self) -> bool {
        self.runs.iter().any(|r| r.status.is_live())
    }

    pub fn captures_text(&self) -> bool {
        self.filter_open
    }

    pub fn esc_closes(&self) -> bool {
        !self.filter_open && self.active_tab_state().filter.is_empty()
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

    pub fn scroll(&mut self, delta: isize) -> bool {
        self.move_by(delta)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        if key.kind == KeyEventKind::Release {
            return LedgerOutcome::Unchanged;
        }

        if self.filter_open {
            return self.handle_filter_key(key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('d') if ctrl => {
                let step = (self.last_list_height.max(2) / 2) as isize;
                if self.move_by(step) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let step = (self.last_list_height.max(2) / 2) as isize;
                if self.move_by(-step) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.move_by(1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.move_by(-1) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let step = (self.last_list_height.max(2) / 2) as isize;
                if self.move_by(step) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let step = (self.last_list_height.max(2) / 2) as isize;
                if self.move_by(-step) {
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                let state = self.active_tab_state_mut();
                if !state.visible_indices.is_empty() && state.selected != 0 {
                    state.selected = 0;
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Unchanged
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                let state = self.active_tab_state_mut();
                if !state.visible_indices.is_empty() {
                    let last = state.visible_indices.len() - 1;
                    if state.selected != last {
                        state.selected = last;
                        return LedgerOutcome::Changed;
                    }
                }
                LedgerOutcome::Unchanged
            }
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.tab = self.tab.toggle();
                LedgerOutcome::Changed
            }
            KeyCode::Char('/') => {
                self.filter_open = true;
                LedgerOutcome::Changed
            }
            KeyCode::Char('a') => {
                match self.tab {
                    LedgerTab::Flows => {
                        let prev = self.selected_run().map(|r| r.run_id.clone());
                        self.flows_tab.active_only = !self.flows_tab.active_only;
                        self.recompute_flows(prev.as_deref());
                    }
                    LedgerTab::Sessions => {
                        let prev = self.selected_session().map(|s| s.record_id.clone());
                        self.sessions_tab.active_only = !self.sessions_tab.active_only;
                        self.recompute_sessions(prev.as_deref());
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char('s') => {
                match self.tab {
                    LedgerTab::Flows => {
                        let prev = self.selected_run().map(|r| r.run_id.clone());
                        self.flows_tab.sort = self.flows_tab.sort.next();
                        self.recompute_flows(prev.as_deref());
                    }
                    LedgerTab::Sessions => {
                        let prev = self.selected_session().map(|s| s.record_id.clone());
                        self.sessions_tab.sort = self.sessions_tab.sort.next();
                        self.recompute_sessions(prev.as_deref());
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
                    if let Some(session) = self.selected_session() {
                        LedgerOutcome::OpenSession(session.clone())
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
                    if let Some(session) = self.selected_session() {
                        LedgerOutcome::Copy(session.record_id.clone())
                    } else {
                        LedgerOutcome::Unchanged
                    }
                }
            },
            KeyCode::Char('R') => LedgerOutcome::Rescan,
            KeyCode::Esc => {
                if !self.active_tab_state().filter.is_empty() {
                    self.clear_active_filter();
                    LedgerOutcome::Changed
                } else {
                    LedgerOutcome::Close
                }
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn handle_filter_key(&mut self, key: &KeyEvent) -> LedgerOutcome {
        match key.code {
            KeyCode::Enter => {
                self.filter_open = false;
                LedgerOutcome::Changed
            }
            KeyCode::Esc => {
                self.filter_open = false;
                self.clear_active_filter();
                LedgerOutcome::Changed
            }
            KeyCode::Backspace => {
                match self.tab {
                    LedgerTab::Flows => {
                        let prev = self.selected_run().map(|r| r.run_id.clone());
                        self.flows_tab.filter.pop();
                        self.recompute_flows(prev.as_deref());
                    }
                    LedgerTab::Sessions => {
                        let prev = self.selected_session().map(|s| s.record_id.clone());
                        self.sessions_tab.filter.pop();
                        self.recompute_sessions(prev.as_deref());
                    }
                }
                LedgerOutcome::Changed
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::ALT)
                {
                    return LedgerOutcome::Unchanged;
                }
                match self.tab {
                    LedgerTab::Flows => {
                        let prev = self.selected_run().map(|r| r.run_id.clone());
                        self.flows_tab.filter.push(c);
                        self.recompute_flows(prev.as_deref());
                    }
                    LedgerTab::Sessions => {
                        let prev = self.selected_session().map(|s| s.record_id.clone());
                        self.sessions_tab.filter.push(c);
                        self.recompute_sessions(prev.as_deref());
                    }
                }
                LedgerOutcome::Changed
            }
            _ => LedgerOutcome::Unchanged,
        }
    }

    fn clear_active_filter(&mut self) {
        match self.tab {
            LedgerTab::Flows => {
                let prev = self.selected_run().map(|r| r.run_id.clone());
                self.flows_tab.filter.clear();
                self.recompute_flows(prev.as_deref());
            }
            LedgerTab::Sessions => {
                let prev = self.selected_session().map(|s| s.record_id.clone());
                self.sessions_tab.filter.clear();
                self.recompute_sessions(prev.as_deref());
            }
        }
    }

    fn active_tab_state(&self) -> &TabState {
        match self.tab {
            LedgerTab::Flows => &self.flows_tab,
            LedgerTab::Sessions => &self.sessions_tab,
        }
    }

    fn active_tab_state_mut(&mut self) -> &mut TabState {
        match self.tab {
            LedgerTab::Flows => &mut self.flows_tab,
            LedgerTab::Sessions => &mut self.sessions_tab,
        }
    }

    fn selected_run(&self) -> Option<&RunSummary> {
        let idx = *self.flows_tab.visible_indices.get(self.flows_tab.selected)?;
        self.runs.get(idx)
    }

    fn selected_session(&self) -> Option<&SessionSummary> {
        let idx = *self
            .sessions_tab
            .visible_indices
            .get(self.sessions_tab.selected)?;
        self.sessions.get(idx)
    }

    fn move_by(&mut self, delta: isize) -> bool {
        let state = self.active_tab_state_mut();
        let len = state.visible_indices.len();
        if len == 0 || delta == 0 {
            return false;
        }
        let cur = state.selected as isize;
        let next = (cur + delta).clamp(0, (len - 1) as isize) as usize;
        if next != state.selected {
            state.selected = next;
            true
        } else {
            false
        }
    }

    fn recompute_flows(&mut self, keep_run_id: Option<&str>) {
        let q = self.flows_tab.filter.to_lowercase();
        let active_only = self.flows_tab.active_only;
        let sort = self.flows_tab.sort;

        let mut indices: Vec<usize> = self
            .runs
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                if active_only && !matches!(r.status, RunStatus::Running | RunStatus::Waiting) {
                    return false;
                }
                if !q.is_empty() {
                    let matched = r.title.to_lowercase().contains(&q)
                        || r.flow_name.to_lowercase().contains(&q)
                        || r.status.label().to_lowercase().contains(&q)
                        || r.run_id.to_lowercase().contains(&q);
                    if !matched {
                        return false;
                    }
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        indices.sort_by(|&a, &b| {
            let ra = &self.runs[a];
            let rb = &self.runs[b];
            match sort {
                SortOrder::Recent => rb
                    .updated_ms
                    .cmp(&ra.updated_ms)
                    .then_with(|| rb.started_ms.cmp(&ra.started_ms))
                    .then_with(|| ra.run_id.cmp(&rb.run_id)),
                SortOrder::Status => status_rank(ra.status)
                    .cmp(&status_rank(rb.status))
                    .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                    .then_with(|| ra.run_id.cmp(&rb.run_id)),
                SortOrder::Name => ra
                    .title
                    .to_lowercase()
                    .cmp(&rb.title.to_lowercase())
                    .then_with(|| rb.updated_ms.cmp(&ra.updated_ms))
                    .then_with(|| ra.run_id.cmp(&rb.run_id)),
            }
        });

        self.flows_tab.visible_indices = indices;
        if let Some(id) = keep_run_id {
            if let Some(pos) = self
                .flows_tab
                .visible_indices
                .iter()
                .position(|&i| self.runs.get(i).is_some_and(|r| r.run_id == id))
            {
                self.flows_tab.selected = pos;
            }
        }
        self.flows_tab.clamp();
    }

    fn recompute_sessions(&mut self, keep_record_id: Option<&str>) {
        let q = self.sessions_tab.filter.to_lowercase();
        let active_only = self.sessions_tab.active_only;
        let sort = self.sessions_tab.sort;

        let mut indices: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                if active_only && s.closed {
                    return false;
                }
                if !q.is_empty() {
                    let matched = s.title.to_lowercase().contains(&q)
                        || s.agent.to_lowercase().contains(&q)
                        || s.cwd.to_lowercase().contains(&q)
                        || s.name.to_lowercase().contains(&q);
                    if !matched {
                        return false;
                    }
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        indices.sort_by(|&a, &b| {
            let sa = &self.sessions[a];
            let sb = &self.sessions[b];
            match sort {
                SortOrder::Recent => sb
                    .last_used_ms
                    .cmp(&sa.last_used_ms)
                    .then_with(|| sb.created_ms.cmp(&sa.created_ms))
                    .then_with(|| sa.record_id.cmp(&sb.record_id)),
                SortOrder::Status => sa
                    .closed
                    .cmp(&sb.closed)
                    .then_with(|| sb.last_used_ms.cmp(&sa.last_used_ms))
                    .then_with(|| sa.record_id.cmp(&sb.record_id)),
                SortOrder::Name => sa
                    .title
                    .to_lowercase()
                    .cmp(&sb.title.to_lowercase())
                    .then_with(|| sb.last_used_ms.cmp(&sa.last_used_ms))
                    .then_with(|| sa.record_id.cmp(&sb.record_id)),
            }
        });

        self.sessions_tab.visible_indices = indices;
        if let Some(id) = keep_record_id {
            if let Some(pos) = self
                .sessions_tab
                .visible_indices
                .iter()
                .position(|&i| self.sessions.get(i).is_some_and(|s| s.record_id == id))
            {
                self.sessions_tab.selected = pos;
            }
        }
        self.sessions_tab.clamp();
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Row 0: Tab bar + right summary
        self.render_tab_row(buf, area.x, area.y, area.width, theme);
        if area.height == 1 {
            return;
        }

        let list_height = area.height.saturating_sub(2) as usize;
        self.last_list_height = list_height.max(1);

        match self.tab {
            LedgerTab::Flows => {
                self.flows_tab.adjust_scroll(list_height);
                self.render_flows_header(buf, area.x, area.y + 1, area.width, list_height, theme);
                if area.height > 2 {
                    let list_area = Rect::new(
                        area.x,
                        area.y + 2,
                        area.width,
                        area.height.saturating_sub(2),
                    );
                    self.render_flows_list(buf, list_area, theme, now_ms);
                }
            }
            LedgerTab::Sessions => {
                self.sessions_tab.adjust_scroll(list_height);
                self.render_sessions_header(
                    buf,
                    area.x,
                    area.y + 1,
                    area.width,
                    list_height,
                    theme,
                );
                if area.height > 2 {
                    let list_area = Rect::new(
                        area.x,
                        area.y + 2,
                        area.width,
                        area.height.saturating_sub(2),
                    );
                    self.render_sessions_list(buf, list_area, theme, now_ms);
                }
            }
        }
    }

    fn render_tab_row(&self, buf: &mut Buffer, x: u16, y: u16, width: u16, theme: &Theme) {
        let w = width as usize;
        let flows_style = if self.tab == LedgerTab::Flows {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        let sessions_style = if self.tab == LedgerTab::Sessions {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        let sep_style = Style::default().fg(theme.gray_dim);

        let flows_text = format!(" Flows {} ", self.runs.len());
        let sep_text = "│";
        let sessions_text = format!(" Sessions {} ", self.sessions.len());

        let left_line = Line::from(vec![
            Span::styled(flows_text.clone(), flows_style),
            Span::styled(sep_text, sep_style),
            Span::styled(sessions_text.clone(), sessions_style),
        ]);
        buf.set_line(x, y, &left_line, width);

        let left_w = flows_text.width() + sep_text.width() + sessions_text.width();

        // Build right-aligned summary + filter / active badge
        let mut right_spans: Vec<Span> = Vec::new();
        let state = self.active_tab_state();

        if self.filter_open || !state.filter.is_empty() {
            let filter_display = if self.filter_open {
                format!("/{}_", state.filter)
            } else {
                format!("/{}", state.filter)
            };
            right_spans.push(Span::styled(
                filter_display,
                Style::default()
                    .fg(theme.fuzzy_accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        if state.active_only {
            if !right_spans.is_empty() {
                right_spans.push(Span::styled("  ", Style::default()));
            }
            right_spans.push(Span::styled(
                "active",
                Style::default()
                    .fg(theme.accent_running)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let running_count = self
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Running)
            .count();
        let waiting_count = self
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Waiting)
            .count();

        let mut summary_parts = Vec::new();
        if running_count > 0 {
            summary_parts.push((
                format!("{running_count} running"),
                Style::default().fg(theme.accent_running),
            ));
        }
        if waiting_count > 0 {
            summary_parts.push((
                format!("{waiting_count} waiting"),
                Style::default().fg(theme.warning),
            ));
        }

        if !summary_parts.is_empty() {
            if !right_spans.is_empty() {
                right_spans.push(Span::styled("  ", Style::default().fg(theme.gray_dim)));
            }
            for (i, (text, style)) in summary_parts.into_iter().enumerate() {
                if i > 0 {
                    right_spans.push(Span::styled(" · ", Style::default().fg(theme.gray_dim)));
                }
                right_spans.push(Span::styled(text, style));
            }
        }

        if !right_spans.is_empty() {
            right_spans.push(Span::raw(" "));
            let right_line = Line::from(right_spans);
            let right_w = right_line.width();
            if left_w + 1 + right_w <= w {
                let rx = x + (w - right_w) as u16;
                buf.set_line(rx, y, &right_line, right_w as u16);
            }
        }
    }

    fn render_flows_header(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        width: u16,
        list_height: usize,
        theme: &Theme,
    ) {
        let w = width as usize;
        let cols = FlowsColumns::compute(w);
        let hdr_style = Style::default().fg(theme.gray_dim);

        let title_hdr = format!("Title [sort: {}]", self.flows_tab.sort.label());
        let mut cells: Vec<(&str, usize, bool)> = vec![
            ("St", 2, false),
            (&title_hdr, cols.title_w, false),
        ];
        if cols.flow_w > 0 {
            cells.push(("Flow", cols.flow_w, false));
        }
        if cols.state_w > 0 {
            cells.push(("State", cols.state_w, false));
        }
        if cols.age_w > 0 {
            cells.push(("Age", cols.age_w, true));
        }
        if cols.dur_w > 0 {
            cells.push(("Dur", cols.dur_w, true));
        }

        let line_str = format_columns(&cells, w);
        buf.set_stringn(x, y, &line_str, w, hdr_style);

        self.render_overflow_counter(
            buf,
            x,
            y,
            width,
            &self.flows_tab,
            list_height,
            theme,
        );
    }

    fn render_sessions_header(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        width: u16,
        list_height: usize,
        theme: &Theme,
    ) {
        let w = width as usize;
        let cols = SessionsColumns::compute(w);
        let hdr_style = Style::default().fg(theme.gray_dim);

        let title_hdr = format!("Title [sort: {}]", self.sessions_tab.sort.label());
        let mut cells: Vec<(&str, usize, bool)> = vec![
            ("St", 2, false),
            (&title_hdr, cols.title_w, false),
        ];
        if cols.agent_w > 0 {
            cells.push(("Agent", cols.agent_w, false));
        }
        if cols.cwd_w > 0 {
            cells.push(("Dir", cols.cwd_w, false));
        }
        if cols.msgs_w > 0 {
            cells.push(("Msgs", cols.msgs_w, true));
        }
        if cols.tokens_w > 0 {
            cells.push(("Tokens", cols.tokens_w, true));
        }
        if cols.cost_w > 0 {
            cells.push(("Cost", cols.cost_w, true));
        }
        if cols.age_w > 0 {
            cells.push(("Age", cols.age_w, true));
        }

        let line_str = format_columns(&cells, w);
        buf.set_stringn(x, y, &line_str, w, hdr_style);

        self.render_overflow_counter(
            buf,
            x,
            y,
            width,
            &self.sessions_tab,
            list_height,
            theme,
        );
    }

    fn render_overflow_counter(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        width: u16,
        state: &TabState,
        list_height: usize,
        theme: &Theme,
    ) {
        let total = state.visible_indices.len();
        if total > list_height && list_height > 0 {
            let counter = format!(" {}/{} ", state.selected + 1, total);
            let cw = counter.width();
            if cw <= width as usize {
                let cx = x + width - cw as u16;
                buf.set_stringn(
                    cx,
                    y,
                    &counter,
                    cw,
                    Style::default().fg(theme.gray_dim),
                );
            }
        }
    }

    fn render_flows_list(&self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        if self.flows_tab.visible_indices.is_empty() {
            let msg = if !self.flows_tab.filter.is_empty() || self.flows_tab.active_only {
                "No flow runs match the current filter"
            } else {
                "No acpx flow runs found"
            };
            render_empty_line(buf, area, msg, theme);
            return;
        }

        let w = area.width as usize;
        let cols = FlowsColumns::compute(w);
        let start = self.flows_tab.scroll;
        let end = (start + area.height as usize).min(self.flows_tab.visible_indices.len());

        for (row_offset, vis_idx) in (start..end).enumerate() {
            let y = area.y + row_offset as u16;
            let run_idx = self.flows_tab.visible_indices[vis_idx];
            let Some(run) = self.runs.get(run_idx) else {
                continue;
            };
            let is_selected = vis_idx == self.flows_tab.selected;

            if is_selected {
                buf.set_style(
                    Rect::new(area.x, y, area.width, 1),
                    Style::default()
                        .bg(theme.bg_highlight)
                        .add_modifier(Modifier::BOLD),
                );
            }

            let row_mod = if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            };
            let bg = if is_selected {
                Some(theme.bg_highlight)
            } else {
                None
            };

            let status_color = run_status_color(run.status, theme);
            let status_style = with_bg(Style::default().fg(status_color).add_modifier(row_mod), bg);
            let primary_style = with_bg(
                Style::default().fg(theme.text_primary).add_modifier(row_mod),
                bg,
            );
            let secondary_style =
                with_bg(Style::default().fg(theme.gray).add_modifier(row_mod), bg);
            let state_style = with_bg(
                Style::default()
                    .fg(run_state_color(run.status, theme))
                    .add_modifier(row_mod),
                bg,
            );

            let mut spans: Vec<Span> = Vec::new();
            let glyph_cell = pad_cell(run.status.glyph(), 2, false);
            spans.push(Span::styled(format!(" {glyph_cell}"), status_style));

            spans.push(Span::styled(" ", secondary_style));
            spans.push(Span::styled(
                pad_cell(&run.title, cols.title_w, false),
                primary_style,
            ));

            if cols.flow_w > 0 {
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&run.flow_name, cols.flow_w, false),
                    secondary_style,
                ));
            }

            if cols.state_w > 0 {
                let st_text = format_run_state(run, cols.state_w);
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&st_text, cols.state_w, false),
                    state_style,
                ));
            }

            if cols.age_w > 0 {
                let ago = now_ms.saturating_sub(run.updated_ms).max(0);
                let age_str = fmt_age(ago);
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&age_str, cols.age_w, true),
                    secondary_style,
                ));
            }

            if cols.dur_w > 0 {
                let end_ms = if run.status.is_live() {
                    now_ms
                } else {
                    run.finished_ms.unwrap_or(run.updated_ms)
                };
                let dur_ms = end_ms.saturating_sub(run.started_ms).max(0) as u64;
                let dur_str = fmt_duration(dur_ms);
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&dur_str, cols.dur_w, true),
                    secondary_style,
                ));
            }

            let line = Line::from(spans);
            buf.set_line(area.x, y, &line, area.width);
        }
    }

    fn render_sessions_list(&self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
        if self.sessions_tab.visible_indices.is_empty() {
            let msg = if !self.sessions_tab.filter.is_empty() || self.sessions_tab.active_only {
                "No sessions match the current filter"
            } else {
                "No acpx sessions found"
            };
            render_empty_line(buf, area, msg, theme);
            return;
        }

        let w = area.width as usize;
        let cols = SessionsColumns::compute(w);
        let start = self.sessions_tab.scroll;
        let end = (start + area.height as usize).min(self.sessions_tab.visible_indices.len());

        for (row_offset, vis_idx) in (start..end).enumerate() {
            let y = area.y + row_offset as u16;
            let sess_idx = self.sessions_tab.visible_indices[vis_idx];
            let Some(sess) = self.sessions.get(sess_idx) else {
                continue;
            };
            let is_selected = vis_idx == self.sessions_tab.selected;

            if is_selected {
                buf.set_style(
                    Rect::new(area.x, y, area.width, 1),
                    Style::default()
                        .bg(theme.bg_highlight)
                        .add_modifier(Modifier::BOLD),
                );
            }

            let row_mod = if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            };
            let bg = if is_selected {
                Some(theme.bg_highlight)
            } else {
                None
            };

            let (glyph, glyph_color) = if sess.closed {
                ("○", theme.gray)
            } else {
                ("●", theme.accent_success)
            };
            let status_style = with_bg(Style::default().fg(glyph_color).add_modifier(row_mod), bg);
            let primary_style = with_bg(
                Style::default().fg(theme.text_primary).add_modifier(row_mod),
                bg,
            );
            let secondary_style =
                with_bg(Style::default().fg(theme.gray).add_modifier(row_mod), bg);

            let mut spans: Vec<Span> = Vec::new();
            let glyph_cell = pad_cell(glyph, 2, false);
            spans.push(Span::styled(format!(" {glyph_cell}"), status_style));

            spans.push(Span::styled(" ", secondary_style));
            spans.push(Span::styled(
                pad_cell(&sess.title, cols.title_w, false),
                primary_style,
            ));

            if cols.agent_w > 0 {
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&sess.agent, cols.agent_w, false),
                    secondary_style,
                ));
            }

            if cols.cwd_w > 0 {
                let cwd_short = last_path_component(&sess.cwd);
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(cwd_short, cols.cwd_w, false),
                    secondary_style,
                ));
            }

            if cols.msgs_w > 0 {
                let msgs_str = sess.messages.to_string();
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&msgs_str, cols.msgs_w, true),
                    secondary_style,
                ));
            }

            if cols.tokens_w > 0 {
                let tok_str = fmt_tokens(sess.tokens);
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&tok_str, cols.tokens_w, true),
                    secondary_style,
                ));
            }

            if cols.cost_w > 0 {
                let cost_str = sess
                    .cost_usd
                    .map(|c| format!("${c:.2}"))
                    .unwrap_or_default();
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&cost_str, cols.cost_w, true),
                    secondary_style,
                ));
            }

            if cols.age_w > 0 {
                let ago = now_ms.saturating_sub(sess.last_used_ms).max(0);
                let age_str = fmt_age(ago);
                spans.push(Span::styled(" ", secondary_style));
                spans.push(Span::styled(
                    pad_cell(&age_str, cols.age_w, true),
                    secondary_style,
                ));
            }

            let line = Line::from(spans);
            buf.set_line(area.x, y, &line, area.width);
        }
    }
}

struct FlowsColumns {
    title_w: usize,
    flow_w: usize,
    state_w: usize,
    age_w: usize,
    dur_w: usize,
}

impl FlowsColumns {
    fn compute(total_w: usize) -> Self {
        // Drop duration below 100, flow name below 80, age below 60
        let dur_w = if total_w >= 100 { 8 } else { 0 };
        let flow_w = if total_w >= 80 { 16 } else { 0 };
        let age_w = if total_w >= 60 { 6 } else { 0 };
        let state_w = if total_w >= 45 {
            if total_w >= 90 { 26 } else { 18 }
        } else {
            0
        };

        // Fixed overhead: leading 1 + glyph 2 + space 1 = 4, plus 1 space per optional column + trailing 1
        let mut used = 4;
        for &w in &[flow_w, state_w, age_w, dur_w] {
            if w > 0 {
                used += 1 + w;
            }
        }
        used += 1; // right padding
        let title_w = total_w.saturating_sub(used).max(1);

        Self {
            title_w,
            flow_w,
            state_w,
            age_w,
            dur_w,
        }
    }
}

struct SessionsColumns {
    title_w: usize,
    agent_w: usize,
    cwd_w: usize,
    msgs_w: usize,
    tokens_w: usize,
    cost_w: usize,
    age_w: usize,
}

impl SessionsColumns {
    fn compute(total_w: usize) -> Self {
        let age_w = if total_w >= 60 { 6 } else { 0 };
        let cost_w = if total_w >= 105 { 7 } else { 0 };
        let tokens_w = if total_w >= 92 { 7 } else { 0 };
        let msgs_w = if total_w >= 80 { 5 } else { 0 };
        let cwd_w = if total_w >= 68 { 14 } else { 0 };
        let agent_w = if total_w >= 48 { 12 } else { 0 };

        let mut used = 4;
        for &w in &[agent_w, cwd_w, msgs_w, tokens_w, cost_w, age_w] {
            if w > 0 {
                used += 1 + w;
            }
        }
        used += 1;
        let title_w = total_w.saturating_sub(used).max(1);

        Self {
            title_w,
            agent_w,
            cwd_w,
            msgs_w,
            tokens_w,
            cost_w,
            age_w,
        }
    }
}

fn format_run_state(run: &RunSummary, max_w: usize) -> String {
    match run.status {
        RunStatus::Waiting => {
            let target = run.waiting_on.as_deref().unwrap_or("input");
            truncate_str(&format!("waiting: {target}"), max_w)
        }
        RunStatus::Running => {
            let node = run.current_node.as_deref().unwrap_or("running");
            if let Some(detail) = run.status_detail.as_deref() {
                if !detail.is_empty() {
                    let combined = format!("{node} · {detail}");
                    if combined.width() <= max_w {
                        return combined;
                    }
                }
            }
            truncate_str(node, max_w)
        }
        RunStatus::Failed | RunStatus::TimedOut => {
            let err = run
                .error
                .as_deref()
                .or(run.status_detail.as_deref())
                .unwrap_or(run.status.label());
            let single_line = err.lines().next().unwrap_or(err);
            truncate_str(single_line, max_w)
        }
        RunStatus::Completed => truncate_str("done", max_w),
        RunStatus::Unknown => truncate_str(run.status.label(), max_w),
    }
}

fn status_rank(status: RunStatus) -> u8 {
    match status {
        RunStatus::Running => 0,
        RunStatus::Waiting => 1,
        RunStatus::Failed => 2,
        RunStatus::TimedOut => 3,
        RunStatus::Completed => 4,
        RunStatus::Unknown => 5,
    }
}

fn run_status_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.accent_success,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray,
    }
}

fn run_state_color(status: RunStatus, theme: &Theme) -> Color {
    match status {
        RunStatus::Running => theme.accent_running,
        RunStatus::Waiting => theme.warning,
        RunStatus::Completed => theme.gray,
        RunStatus::Failed | RunStatus::TimedOut => theme.accent_error,
        RunStatus::Unknown => theme.gray,
    }
}

fn with_bg(style: Style, bg: Option<Color>) -> Style {
    match bg {
        Some(c) => style.bg(c),
        None => style,
    }
}

fn pad_cell(s: &str, width: usize, right_align: bool) -> String {
    if width == 0 {
        return String::new();
    }
    let truncated = truncate_str(s, width);
    let w = truncated.width();
    if w >= width {
        truncated
    } else {
        let pad = " ".repeat(width - w);
        if right_align {
            format!("{pad}{truncated}")
        } else {
            format!("{truncated}{pad}")
        }
    }
}

fn format_columns(cells: &[(&str, usize, bool)], max_w: usize) -> String {
    let mut out = String::from(" ");
    for (i, &(text, width, right_align)) in cells.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&pad_cell(text, width, right_align));
    }
    truncate_str(&out, max_w)
}

fn last_path_component(cwd: &str) -> &str {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return cwd;
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(cwd)
}

fn fmt_tokens(tokens: u64) -> String {
    if tokens == 0 {
        "0".to_string()
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn render_empty_line(buf: &mut Buffer, area: Rect, msg: &str, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let y = area.y + area.height / 2;
    let text = truncate_str(msg, area.width as usize);
    let tw = text.width();
    let x = area.x + (area.width.saturating_sub(tw as u16)) / 2;
    buf.set_stringn(
        x,
        y,
        &text,
        area.width as usize,
        Style::default().fg(theme.gray_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_run(
        id: &str,
        title: &str,
        flow: &str,
        status: RunStatus,
        updated_ms: i64,
    ) -> RunSummary {
        RunSummary {
            run_id: id.to_string(),
            dir: PathBuf::from(format!("/tmp/runs/{id}")),
            flow_name: flow.to_string(),
            title: title.to_string(),
            status,
            started_ms: updated_ms - 5_000,
            updated_ms,
            finished_ms: if status.is_live() {
                None
            } else {
                Some(updated_ms)
            },
            current_node: if status == RunStatus::Running {
                Some("implement".to_string())
            } else {
                None
            },
            waiting_on: if status == RunStatus::Waiting {
                Some("review".to_string())
            } else {
                None
            },
            status_detail: None,
            error: if status == RunStatus::Failed {
                Some("exit code 1".to_string())
            } else {
                None
            },
        }
    }

    fn make_session(
        id: &str,
        title: &str,
        agent: &str,
        cwd: &str,
        closed: bool,
        last_used_ms: i64,
    ) -> SessionSummary {
        SessionSummary {
            record_id: id.to_string(),
            acp_session_id: format!("acp-{id}"),
            name: format!("name-{id}"),
            title: title.to_string(),
            agent: agent.to_string(),
            cwd: cwd.to_string(),
            created_ms: last_used_ms - 10_000,
            last_used_ms,
            closed,
            messages: 12,
            tokens: 12_300,
            cost_usd: Some(0.12),
            stream_path: PathBuf::from(format!("/tmp/sessions/{id}.stream.ndjson")),
        }
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        let w = buf.area.width;
        let mut s = String::new();
        for x in 0..w {
            s.push_str(buf[(x, y)].symbol());
        }
        s
    }

    #[test]
    fn navigation_and_clamping() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "First", "pr-triage", RunStatus::Completed, 3000),
            make_run("r2", "Second", "pr-triage", RunStatus::Completed, 2000),
            make_run("r3", "Third", "pr-triage", RunStatus::Completed, 1000),
        ]);

        assert_eq!(state.handle_key(&key(KeyCode::Up)), LedgerOutcome::Unchanged);
        assert_eq!(state.handle_key(&key(KeyCode::Char('j'))), LedgerOutcome::Changed);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        assert_eq!(state.handle_key(&key(KeyCode::Char('G'))), LedgerOutcome::Changed);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r3"));
        assert_eq!(state.handle_key(&key(KeyCode::Down)), LedgerOutcome::Unchanged);

        assert_eq!(state.handle_key(&key(KeyCode::Char('g'))), LedgerOutcome::Changed);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r1"));
    }

    #[test]
    fn tab_switching_preserves_per_tab_selection() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Run 1", "flow", RunStatus::Completed, 2000),
            make_run("r2", "Run 2", "flow", RunStatus::Completed, 1000),
        ]);
        state.set_sessions(vec![
            make_session("s1", "Sess 1", "claude", "/a/b", false, 2000),
            make_session("s2", "Sess 2", "codex", "/a/c", false, 1000),
        ]);

        state.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(state.tab(), LedgerTab::Sessions);
        assert_eq!(
            state.selected_session().map(|s| s.record_id.as_str()),
            Some("s1")
        );

        state.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(
            state.selected_session().map(|s| s.record_id.as_str()),
            Some("s2")
        );

        state.handle_key(&key(KeyCode::Char('h')));
        assert_eq!(state.tab(), LedgerTab::Flows);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
    }

    #[test]
    fn filter_typing_enter_and_esc_ordering() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Deploy prod", "release", RunStatus::Completed, 2000),
            make_run("r2", "Triage bug", "triage", RunStatus::Failed, 1000),
        ]);

        assert!(state.esc_closes());
        assert!(!state.captures_text());

        state.handle_key(&key(KeyCode::Char('/')));
        assert!(state.captures_text());
        assert!(!state.esc_closes());

        // Printable keys including 'a' and 's' go into filter
        state.handle_key(&key(KeyCode::Char('t')));
        state.handle_key(&key(KeyCode::Char('r')));
        state.handle_key(&key(KeyCode::Char('i')));
        assert_eq!(state.flows_tab.visible_indices.len(), 1);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Enter closes the input but keeps the filter
        state.handle_key(&key(KeyCode::Enter));
        assert!(!state.captures_text());
        assert!(!state.esc_closes());
        assert_eq!(state.flows_tab.visible_indices.len(), 1);

        // First Esc clears the filter and returns Changed
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), LedgerOutcome::Changed);
        assert_eq!(state.flows_tab.visible_indices.len(), 2);
        assert!(state.esc_closes());

        // Second Esc closes the modal
        assert_eq!(state.handle_key(&key(KeyCode::Esc)), LedgerOutcome::Close);
    }

    #[test]
    fn active_only_filters_both_tabs() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Done run", "f", RunStatus::Completed, 3000),
            make_run("r2", "Wait run", "f", RunStatus::Waiting, 2000),
            make_run("r3", "Live run", "f", RunStatus::Running, 1000),
        ]);
        state.set_sessions(vec![
            make_session("s1", "Closed sess", "claude", "/tmp", true, 2000),
            make_session("s2", "Open sess", "claude", "/tmp", false, 1000),
        ]);

        state.handle_key(&key(KeyCode::Char('a')));
        assert_eq!(state.flows_tab.visible_indices.len(), 2);

        state.handle_key(&key(KeyCode::Tab));
        state.handle_key(&key(KeyCode::Char('a')));
        assert_eq!(state.sessions_tab.visible_indices.len(), 1);
        assert_eq!(
            state.selected_session().map(|s| s.record_id.as_str()),
            Some("s2")
        );
    }

    #[test]
    fn sort_cycling_changes_order() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "Zebra", "f", RunStatus::Completed, 3000),
            make_run("r2", "Alpha", "f", RunStatus::Running, 1000),
            make_run("r3", "Middle", "f", RunStatus::Waiting, 2000),
        ]);

        // Initial sort: recent (r1, r3, r2)
        assert_eq!(state.flows_tab.visible_indices, vec![0, 2, 1]);

        // Cycle to status: Running (r2), Waiting (r3), Completed (r1)
        state.handle_key(&key(KeyCode::Char('s')));
        assert_eq!(state.flows_tab.visible_indices, vec![1, 2, 0]);

        // Cycle to name: Alpha (r2), Middle (r3), Zebra (r1)
        state.handle_key(&key(KeyCode::Char('s')));
        assert_eq!(state.flows_tab.visible_indices, vec![1, 2, 0]);

        // Cycle back to recent
        state.handle_key(&key(KeyCode::Char('s')));
        assert_eq!(state.flows_tab.visible_indices, vec![0, 2, 1]);
    }

    #[test]
    fn enter_outcomes_on_both_tabs() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run(
            "r1",
            "Run 1",
            "flow",
            RunStatus::Completed,
            1000,
        )]);
        let sess = make_session("s1", "Sess 1", "claude", "/repo", false, 1000);
        state.set_sessions(vec![sess.clone()]);

        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenRun(PathBuf::from("/tmp/runs/r1"))
        );

        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(
            state.handle_key(&key(KeyCode::Enter)),
            LedgerOutcome::OpenSession(sess)
        );
    }

    #[test]
    fn copy_and_rescan_keys() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run(
            "r1",
            "Run 1",
            "flow",
            RunStatus::Completed,
            1000,
        )]);
        state.set_sessions(vec![make_session(
            "s-rec-42",
            "Sess 1",
            "claude",
            "/repo",
            false,
            1000,
        )]);

        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("/tmp/runs/r1".to_string())
        );
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('R'))),
            LedgerOutcome::Rescan
        );

        state.handle_key(&key(KeyCode::Tab));
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('y'))),
            LedgerOutcome::Copy("s-rec-42".to_string())
        );
    }

    #[test]
    fn set_runs_keeps_selected_run_id() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "One", "f", RunStatus::Completed, 3000),
            make_run("r2", "Two", "f", RunStatus::Completed, 2000),
        ]);
        state.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));

        // Now r2 is updated more recently and moves to index 0
        state.set_runs(vec![
            make_run("r1", "One", "f", RunStatus::Completed, 3000),
            make_run("r2", "Two", "f", RunStatus::Running, 5000),
        ]);
        assert_eq!(state.selected_run().map(|r| r.run_id.as_str()), Some("r2"));
        assert_eq!(state.flows_tab.selected, 0);
    }

    #[test]
    fn any_live_tracks_running_runs() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run("r1", "One", "f", RunStatus::Waiting, 1000)]);
        assert!(!state.any_live());

        state.set_runs(vec![make_run("r1", "One", "f", RunStatus::Running, 1000)]);
        assert!(state.any_live());
    }

    #[test]
    fn render_120x12_shows_tabs_and_waiting_row() {
        let mut state = LedgerState::new();
        state.set_runs(vec![
            make_run("r1", "PR Triage #42", "pr-triage", RunStatus::Waiting, 9_000),
            make_run("r2", "Active Build", "ci-flow", RunStatus::Running, 8_000),
        ]);

        let area = Rect::new(0, 0, 120, 12);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        state.render(&mut buf, area, &theme, 10_000);

        let r0 = row_text(&buf, 0);
        assert!(r0.contains("Flows 2"));
        assert!(r0.contains("Sessions 0"));
        assert!(r0.contains("1 running · 1 waiting"));

        let r2 = row_text(&buf, 2);
        assert!(r2.contains("◐"));
        assert!(r2.contains("PR Triage #42"));
        assert!(r2.contains("waiting: review"));
    }

    #[test]
    fn render_50x6_keeps_title_and_does_not_panic() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run(
            "r1",
            "CompactTitle",
            "pr-triage",
            RunStatus::Completed,
            9_000,
        )]);

        let area = Rect::new(0, 0, 50, 6);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        state.render(&mut buf, area, &theme, 10_000);

        let r2 = row_text(&buf, 2);
        assert!(r2.contains("CompactTitle"));
    }

    #[test]
    fn render_0x0_does_not_panic() {
        let mut state = LedgerState::new();
        state.set_runs(vec![make_run(
            "r1",
            "Title",
            "flow",
            RunStatus::Completed,
            1000,
        )]);
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        state.render(&mut buf, area, &theme, 10_000);
    }

    #[test]
    fn render_empty_state_shows_explanatory_line() {
        let mut state = LedgerState::new();
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        state.render(&mut buf, area, &theme, 10_000);

        let mut all_text = String::new();
        for y in 0..area.height {
            all_text.push_str(&row_text(&buf, y));
        }
        assert!(all_text.contains("No acpx flow runs found"));
    }
}
```