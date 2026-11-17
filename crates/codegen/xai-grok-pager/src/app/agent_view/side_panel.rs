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
use crate::views::side_panel::{Event, Info, Job, JobState, TurnRow};

/// Token totals for the session, summed from each turn's `acp::Usage`. A figure stays `None` until a turn reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TokenTotals {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

impl TokenTotals {
    pub(crate) fn add(&mut self, u: &agent_client_protocol::Usage) {
        fn sum(total: &mut Option<u64>, n: Option<u64>) {
            if let Some(n) = n {
                *total = Some(total.unwrap_or(0).saturating_add(n));
            }
        }
        sum(&mut self.input, Some(u.input_tokens));
        sum(&mut self.output, Some(u.output_tokens));
        sum(&mut self.cache_read, u.cached_read_tokens);
        sum(&mut self.cache_write, u.cached_write_tokens);
    }
}

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
            tokens_in: self.token_totals.input,
            tokens_out: self.token_totals.output,
            cache_read: self.token_totals.cache_read,
            cache_write: self.token_totals.cache_write,
            // end session page
            jobs: self.side_panel_jobs(),
            subagents: self.side_panel_subagents(),
            turn: self.side_panel_turn_row(),
        }
    }

    /// The chat's status row, when the chat shows an activity label for it: not idle, and not the parked or idle cues.
    fn side_panel_turn_row(&self) -> Option<TurnRow> {
        let (state, activity) = self.turn_status_activity();
        let parked = self.renders_parked() && !self.dock_covers_idle_cues(self.dock_on);
        if state.is_idle() || parked {
            return None;
        }
        Some(TurnRow {
            state,
            activity,
            activity_started_at: self.activity_started_at,
            tick: self.scrollback.animation_tick(),
            is_bash_turn: self.bash_turn,
            is_pending_user_input: self.is_awaiting_user_answer(),
            goal_verifying: self.goal_state.as_ref().is_some_and(|g| g.verifying_completion),
        })
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

#[cfg(test)]
mod token_totals_tests {
    use super::TokenTotals;
    use agent_client_protocol as acp;

    #[test]
    fn add_sums_turns_and_keeps_unreported_figures_unknown() {
        let mut t = TokenTotals::default();
        assert_eq!(t, TokenTotals::default());
        let mut turn = acp::Usage::new(150, 100, 50);
        t.add(&turn);
        // The adapter sent no cache figures yet: they stay unknown rather than 0.
        assert_eq!((t.input, t.output, t.cache_read, t.cache_write), (Some(100), Some(50), None, None));
        turn.cached_read_tokens = Some(800);
        turn.cached_write_tokens = Some(0);
        t.add(&turn);
        t.add(&turn);
        assert_eq!(
            (t.input, t.output, t.cache_read, t.cache_write),
            (Some(300), Some(150), Some(1600), Some(0))
        );
    }
}
