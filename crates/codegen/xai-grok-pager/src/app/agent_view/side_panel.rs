//! Agent-view glue for the F3 side panel ([`crate::views::side_panel`]): what the Info page reads and where the panel draws.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xai_grok_tools::types::SessionMode;

use super::AgentView;
use crate::app::actions::Action;
use crate::app::agent::AgentState;
use crate::app::app_view::InputOutcome;
use crate::theme::Theme;
use crate::views::modal::ActiveModal;
use crate::views::side_panel::{Event, Info, Job, JobState};

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
            rate_limits: self.rate_limits.clone(),
            five_hour_pct: self.side_panel.five_hour_pct,
            status,
            // session page
            branch: self.current_branch.clone().or_else(|| {
                crate::git_info::cwd_git_info_lazy(&s.cwd).and_then(|g| g.branch)
            }),
            started_ms: self.side_panel.session_row().and_then(|r| r.started_at),
            turns: self.context_state.as_ref().map(|c| c.turn_count),
            tool_calls: self.context_state.as_ref().map(|c| c.tool_call_count),
            compactions: self.context_state.as_ref().map(|c| c.compaction_count),
            // end session page
            jobs: self.side_panel_jobs(),
            subagents: self.side_panel_subagents(),
        }
    }

    /// Background commands (monitors are watchers, not listed), running first then oldest first.
    fn side_panel_jobs(&self) -> Vec<Job> {
        use crate::app::agent::BgTaskStatus;
        let mut tasks: Vec<_> = self.session.bg_tasks.values().filter(|t| !t.is_monitor).collect();
        tasks.sort_by_key(|t| (t.status != BgTaskStatus::Running, t.start_time));
        tasks
            .into_iter()
            .map(|t| Job {
                kind: String::new(),
                description: t
                    .description
                    .clone()
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| t.command.clone()),
                state: match t.status {
                    BgTaskStatus::Running => JobState::Running,
                    BgTaskStatus::Done => JobState::Done,
                    BgTaskStatus::Failed => JobState::Failed,
                },
                model: None,
                elapsed: crate::views::dock::fmt_elapsed(t.elapsed().as_secs()),
            })
            .collect()
    }

    /// Non-workflow subagents, finished ones included, running first then oldest first.
    fn side_panel_subagents(&self) -> Vec<Job> {
        let mut infos: Vec<_> = self
            .subagent_sessions
            .values()
            .filter(|i| i.attempt.workflow_run_id.is_none())
            .collect();
        infos.sort_by_key(|i| (!i.is_running(), i.attempt.started_at));
        infos
            .into_iter()
            .map(|i| {
                let (kind, description) = crate::app::subagent::format_subagent_label(i);
                Job {
                    kind,
                    description,
                    state: if i.is_running() {
                        JobState::Running
                    } else {
                        match i.attempt.status.as_deref() {
                            Some("failed") => JobState::Failed,
                            Some("cancelled") => JobState::Cancelled,
                            _ => JobState::Done,
                        }
                    },
                    model: i.attempt.model.as_deref().filter(|m| !m.is_empty()).map(str::to_owned),
                    elapsed: crate::views::dock::fmt_elapsed(i.display_elapsed().as_secs()),
                }
            })
            .collect()
    }

    /// A copy or open the focused panel asked for (`y`, Enter on a row that stands for a session).
    pub(super) fn side_panel_event(&mut self, event: Event) -> InputOutcome {
        match event {
            Event::Copy(text) => {
                let note = match crate::clipboard::copy_text_or_file(&text) {
                    crate::clipboard::CopyDelivery::Clipboard { .. } => "copied".to_string(),
                    crate::clipboard::CopyDelivery::File { path } => {
                        format!("saved to {}", path.display())
                    }
                    crate::clipboard::CopyDelivery::Failed { .. } => "copy failed".to_string(),
                };
                self.side_panel.set_note(note);
            }
            // The /traces modal on that session's trajectory: read-only, no model turn, no harness resume.
            Event::Open(id) => {
                let index = xai_grok_traces::TraceIndex::open(&xai_grok_traces::default_db_path())
                    .map_err(|e| tracing::warn!(%e, "side panel: traces index open failed"))
                    .ok();
                let mut state = crate::views::traces_modal::TracesModalState::open(index);
                if let Err(msg) = state.open_by_prefix(&id) {
                    state.status = Some(msg);
                }
                self.active_modal = Some(ActiveModal::Traces {
                    state: Box::new(state),
                });
                // The same index scan `/traces` starts.
                return InputOutcome::Action(Action::TracesRescan);
            }
        }
        InputOutcome::Changed
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
