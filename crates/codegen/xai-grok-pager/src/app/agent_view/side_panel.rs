//! Agent-view glue for the F3 side panel ([`crate::views::side_panel`]): what the Info page reads and where the panel draws.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xai_grok_tools::types::SessionMode;

use super::AgentView;
use crate::app::agent::AgentState;
use crate::theme::Theme;
use crate::views::side_panel::Info;

impl AgentView {
    pub(crate) fn side_panel_info(&self) -> Info {
        let s = &self.session;
        let mut mode = match self.effective_session_mode() {
            SessionMode::Plan => "plan",
            SessionMode::Ask => "ask",
            SessionMode::Default => "agent",
        }
        .to_string();
        if s.is_yolo() {
            mode.push_str(" · yolo");
        } else if s.is_auto() {
            mode.push_str(" · auto");
        }
        let status = match &s.state {
            AgentState::Idle => "idle".to_string(),
            AgentState::TurnRunning => "running".to_string(),
            AgentState::TurnCancelling | AgentState::CommandCancelling { .. } => {
                "cancelling".to_string()
            }
            AgentState::CommandRunning { command, .. } => command.display_name().to_lowercase(),
        };
        let home = std::env::var("HOME").unwrap_or_default();
        let cwd = s.cwd.display().to_string();
        let cwd = match cwd.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => cwd,
        };
        let row = |r: crate::views::dock::DockRow| (r.description, r.meta);
        Info {
            title: self
                .display_name
                .clone()
                .or_else(|| self.generated_session_title.clone()),
            session_id: s.session_id.as_ref().map(|id| id.0.to_string()),
            cwd,
            model: s.models.footer_model_name(),
            effort: s.models.reasoning_effort.map(|e| e.to_string()),
            mode,
            context: self.context_state.as_ref().map(|c| (c.used, c.total)),
            five_hour_pct: s.tracker.five_hour_pct,
            status,
            jobs: self
                .dock_task_rows()
                .into_iter()
                .map(|(_, r)| row(r))
                .collect(),
            subagents: self
                .dock_subagent_rows()
                .into_iter()
                .map(|(_, _, r)| row(r))
                .collect(),
        }
    }

    pub(super) fn draw_side_panel(&mut self, area: Rect, buf: &mut Buffer) {
        let info = self.side_panel_info();
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.side_panel
            .render(buf, area, &info, &Theme::current(), now_ms);
    }

    /// The 1 s traces tick for the panel. True when it should redraw.
    pub(crate) fn poll_side_panel(&mut self) -> bool {
        let native = self.session.session_id.as_ref().map(|id| id.0.to_string());
        self.side_panel
            .poll(native.as_deref(), chrono::Utc::now().timestamp_millis())
    }
}

#[cfg(test)]
#[path = "side_panel_tests.rs"]
mod tests;
