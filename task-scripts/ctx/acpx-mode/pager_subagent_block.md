# Pager: SubagentBlock, child routing site, fullscreen open, spawn/finish handlers, and one setting's touch points

Notes: children are Box<AgentView> in parent.subagent_views keyed by child session id (app/agent_view/mod.rs:1523); the RenderBlock::Subagent row is created in the SubagentSpawned handler unless workflow_run_id is set; Task tool calls are suppressed from scrollback (acp/tracker.rs:1445). The routing site below applies child session/update to child_view.scrollback. A new pager-owned bool setting needs: defs entry, appearance config field, registry get arm, Action variant, router arm, setter, persist arm.

- SubagentBlock (data and render), tests omitted
```rust
// from crates/codegen/xai-grok-pager/src/scrollback/blocks/subagent.rs:1-381
//! Scrollback entries for the subagent lifecycle.
//!
//! Similar to BgTaskBlock: always collapsed, an animated bullet while running, a colored bullet when done.
//! Enter / Ctrl-F opens the subagent view.
//!
//! Two modes:
//! - **Blocking** (sync): one `Started` block; it blinks while running and turns green/red when done. Text: `Subagent "description"`
//! - **Background** (async): the `Started` block stays forever (turns gray) and a separate `Completed`/`Failed` block is added when done.
//!   Started text: `Subagent started: "description"`
//!   Completed text: `Subagent completed in 43s: "description"`

use std::time::Duration;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::subagent::format_subagent_meta;
use crate::appearance::AppearanceConfig;
use crate::render::line_utils::truncate_str;
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{AccentStyle, BlockContext, BlockOutput, DisplayMode};
use crate::theme::Theme;
use crate::util::format_duration;

/// What kind of subagent lifecycle event this block represents.
#[derive(Debug, Clone)]
pub enum SubagentBlockKind {
    /// Subagent is running (or was running; `finish_running` stops animation).
    Started,
    /// Subagent completed successfully.
    Completed { elapsed: Duration },
    /// Subagent failed.
    Failed {
        elapsed: Duration,
        error: Option<String>,
    },
    /// Subagent was cancelled.
    Cancelled { elapsed: Duration },
}

/// Always collapsed and not foldable; groupable and selectable.
/// Enter / Ctrl-F opens the subagent view.
#[derive(Debug, Clone)]
pub struct SubagentBlock {
    /// Human-readable description of the task.
    pub description: String,
    /// Child session ID (for opening the subagent view).
    pub child_session_id: String,
    /// Subagent type (e.g. "general-purpose", "explore").
    pub subagent_type: String,
    /// Named persona applied to this subagent, if any.
    pub persona: Option<String>,
    /// Role that supplied defaults for this subagent, if any.
    pub role: Option<String>,
    /// Effective model ID used by the subagent, if available.
    pub model: Option<String>,
    /// Whether the subagent was launched in background mode.
    pub is_background: bool,
    /// Lifecycle kind.
    pub kind: SubagentBlockKind,
    /// Live activity label from the child session's turn tracker. The user sees interactive progress without opening
    /// the subagent view.
    pub activity_label: Option<String>,
    /// Number of completed child turns, updated on each `SubagentProgress`
    /// tick and stamped at `SubagentFinished`.
    pub turn_count: Option<u32>,
    /// Cumulative tokens used by the child session, updated on each
    /// `SubagentProgress` tick and stamped at `SubagentFinished`.
    pub tokens_used: Option<u64>,
}

impl SubagentBlock {
    /// Create a "Subagent started" block (for both sync and async).
    pub fn started(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        subagent_type: impl Into<String>,
        persona: Option<String>,
        role: Option<String>,
        model: Option<String>,
        is_background: bool,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: subagent_type.into(),
            persona,
            role,
            model,
            is_background,
            kind: SubagentBlockKind::Started,
            activity_label: None,
            turn_count: None,
            tokens_used: None,
        }
    }

    /// Create a "Subagent completed" block (background mode only).
    pub fn completed(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        elapsed: Duration,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: String::new(),
            persona: None,
            role: None,
            model: None,
            is_background: true,
            kind: SubagentBlockKind::Completed { elapsed },
            activity_label: None,
            turn_count: None,
            tokens_used: None,
        }
    }

    /// Create a "Subagent failed" block (background mode only).
    pub fn failed(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        elapsed: Duration,
        error: Option<String>,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: String::new(),
            persona: None,
            role: None,
            model: None,
            is_background: true,
            kind: SubagentBlockKind::Failed { elapsed, error },
            activity_label: None,
            turn_count: None,
            tokens_used: None,
        }
    }

    /// Create a "Subagent cancelled" block (background mode only).
    pub fn cancelled(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        elapsed: Duration,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: String::new(),
            persona: None,
            role: None,
            model: None,
            is_background: true,
            kind: SubagentBlockKind::Cancelled { elapsed },
            activity_label: None,
            turn_count: None,
            tokens_used: None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.kind, SubagentBlockKind::Started)
    }
}

/// Truncate description and wrap in quotes for display.
fn quoted_desc(desc: &str, max_width: usize) -> String {
    // Reserve 2 chars for quotes
    if max_width <= 2 {
        return "\u{201C}\u{2026}\u{201D}".to_string(); // "…"
    }
    let inner = truncate_str(desc, max_width - 2);
    format!("\u{201C}{inner}\u{201D}")
}

/// Compact token count for inline display: `1.2k`, `12k`, `130k`, `1.2M`.
fn compact_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        let k = n as f64 / 1_000.0;
        if k < 10.0 {
            return format!("{k:.1}k");
        }
        return format!("{:.0}k", k);
    }
    let m = n as f64 / 1_000_000.0;
    if m < 10.0 {
        return format!("{m:.1}M");
    }
    format!("{:.0}M", m)
}

/// Build the `— N turns · M tokens` suffix shown after the description.
///
/// Returns an empty string when neither field is set, so callers can always
/// append it. Uses `\u{2014}` (em dash) and `\u{00b7}` (middle dot) to match
/// the existing meta separator style.
fn format_stats(turn_count: Option<u32>, tokens_used: Option<u64>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(turns) = turn_count.filter(|&t| t > 0) {
        parts.push(format!("{turns} turn{}", if turns == 1 { "" } else { "s" }));
    }
    if let Some(tokens) = tokens_used.filter(|&t| t > 0) {
        parts.push(format!("{} tokens", compact_tokens(tokens)));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(" \u{2014} {}", parts.join(" \u{00b7} "))
}

impl BlockContent for SubagentBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        // When selected, lift only the bold "Subagent" label to `text_primary` so it reads as undimmed
        // This mirrors `read.rs` and `search.rs`, which bump only the label and leave the rest at `muted`
        // The detail text (verb, description, meta) stays muted in every state
        let bold = if ctx.is_selected {
            theme.primary().add_modifier(Modifier::BOLD)
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let muted = theme.muted();
        let w = ctx.width as usize;

        let line = match (&self.kind, self.is_background) {
            (SubagentBlockKind::Started, bg) => {
                let verb = if bg { "started: " } else { "running: " };
                let activity_suffix: String = self
                    .activity_label
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|a| format!(" \u{00b7} {a}"))
                    .unwrap_or_default();
                let stats = format_stats(self.turn_count, self.tokens_used);
                let meta = format_subagent_meta(
                    self.persona.as_deref(),
                    self.role.as_deref(),
                    self.model.as_deref(),
                );
                // "Subagent running: " / "Subagent started: " = 18 chars
                let overhead = 18 + meta.width() + activity_suffix.width() + stats.width();
                let desc = quoted_desc(&self.description, w.saturating_sub(overhead));
                let mut spans = vec![
                    Span::styled("Subagent ", bold),
                    Span::styled(verb, muted),
                    Span::styled(desc, muted),
                ];
                if !activity_suffix.is_empty() {
                    spans.push(Span::styled(activity_suffix, muted));
                }
                if !stats.is_empty() {
                    spans.push(Span::styled(stats, muted));
                }
                spans.push(Span::styled(meta, muted));
                Line::from(spans)
            }
            // Completed: Subagent completed in Xs: "description" — N turns · M tokens
            (SubagentBlockKind::Completed { elapsed }, _) => {
                let time_str = format_duration(*elapsed);
                let stats = format_stats(self.turn_count, self.tokens_used);
                // "Subagent completed in Xs: " = 26 + time_str.len()
                let prefix_len = 26 + time_str.len() + stats.len();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                let mut spans = vec![
                    Span::styled("Subagent ", bold),
                    Span::styled(format!("completed in {time_str}: "), muted),
                    Span::styled(desc, muted),
                ];
                if !stats.is_empty() {
                    spans.push(Span::styled(stats, muted));
                }
                Line::from(spans)
            }
            // Failed: Subagent failed in Xs: "description" — N turns · M tokens
            (SubagentBlockKind::Failed { elapsed, error }, _) => {
                let time_str = format_duration(*elapsed);
                let detail = error
                    .as_deref()
                    .map(|e| format!(" ({e})"))
                    .unwrap_or_default();
                let stats = format_stats(self.turn_count, self.tokens_used);
                let prefix_len = 21 + time_str.len() + detail.len() + stats.len();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                let mut spans = vec![
                    Span::styled("Subagent ", bold),
                    Span::styled(format!("failed in {time_str}{detail}: "), muted),
                    Span::styled(desc, muted),
                ];
                if !stats.is_empty() {
                    spans.push(Span::styled(stats, muted));
                }
                Line::from(spans)
            }
            // Cancelled: Subagent cancelled in Xs: "description" — N turns · M tokens
            (SubagentBlockKind::Cancelled { elapsed }, _) => {
                let time_str = format_duration(*elapsed);
                let stats = format_stats(self.turn_count, self.tokens_used);
                // "Subagent cancelled in Xs: " = 26 + time_str.len()
                let prefix_len = 26 + time_str.len() + stats.len();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                let mut spans = vec![
                    Span::styled("Subagent ", bold),
                    Span::styled(format!("cancelled in {time_str}: "), muted),
                    Span::styled(desc, muted),
                ];
                if !stats.is_empty() {
                    spans.push(Span::styled(stats, muted));
                }
                Line::from(spans)
            }
        };

        BlockOutput {
            lines: vec![line.into()],
        }
    }

    fn accent(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match &self.kind {
            SubagentBlockKind::Started if ctx.is_running => {
                Some(AccentStyle::static_color(theme.accent_running))
            }
            _ => None,
        }
    }

    fn bullet(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match &self.kind {
            SubagentBlockKind::Started => {
                if ctx.is_running {
                    Some(AccentStyle::animated_running(ctx, &theme))
                } else {
                    // Finished: gray bullet (same as bg task "started" after completion)
                    None
                }
            }
            SubagentBlockKind::Completed { .. } => {
                Some(AccentStyle::static_color(theme.accent_success))
            }
            SubagentBlockKind::Failed { .. } | SubagentBlockKind::Cancelled { .. } => {
                Some(AccentStyle::static_color(theme.accent_error))
            }
        }
    }

    fn has_vpad_for(&self, _appearance: &AppearanceConfig) -> bool {
        false
    }

    fn has_raw_mode(&self) -> bool {
        false
    }

    fn is_foldable(&self) -> bool {
        false
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Collapsed
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn has_bullet(&self, _ctx: &BlockContext) -> bool {
        true
    }

    fn is_groupable(&self) -> bool {
        true
    }
}

```

- routing site: session/update for a Child session applied to the child view (ends at handle_update)
```rust
// from crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs:466-548
                Some(SessionMatch::Child(parent_id)) => {
                    let child_sid = notif.request.session_id.0.as_ref();
                    let parent_sid = app.agents.get(&parent_id).and_then(|parent| {
                        parent
                            .session
                            .session_id
                            .as_ref()
                            .map(|sid| sid.0.to_string())
                    });
                    let external_agent = session_backed_by_external_agent(app, child_sid)
                        || parent_sid
                            .as_deref()
                            .is_some_and(|sid| session_backed_by_external_agent(app, sid));
                    let is_active = is_matched_agent_active(app, parent_id);
                    let parent = app
                        .agents
                        .get_mut(&parent_id)
                        .expect("find_session_match returned an existing AgentId");
                    let child_key: &str = notif.request.session_id.0.as_ref();
                    record_child_usage_update(parent, child_key, &notif.request.update);
                    let activity_label = {
                        let child_view = parent
                            .descendant_view_for_live_update_mut(child_key)
                            .expect(
                                "find_session_match returned an existing descendant subagent view",
                            );
                        // An overlay prompt arms the watch on this child, so the child's own updates must disarm it
                        ack_prompt_from_update(child_view, &meta, external_agent);
                        apply_reported_context(child_view, &meta, notif.request.meta.as_ref());
                        if let acp::SessionUpdate::UsageUpdate(ref usage) = notif.request.update {
                            child_view.apply_context_used(usage.used, usage.size);
                        }
                        let ended = !meta.is_replay
                            && meta.prompt_id.as_deref().is_some_and(|pid| {
                                child_view.ended_child_prompt_ids.contains(pid)
                                    || child_view.superseded_child_prompt_ids.contains(pid)
                            });
                        if !ended {
                            let is_live = !meta.is_replay && !child_view.session.loading_replay;
                            let apply = !is_live
                                || note_child_live_prompt(
                                    child_view,
                                    meta.prompt_id.as_deref(),
                                    meta.turn_start_ms,
                                    meta.is_replay,
                                );
                            if apply {
                                if is_live {
                                    if let Some(ts) = meta.turn_start_ms {
                                        let named = meta
                                            .prompt_id
                                            .as_deref()
                                            .is_some_and(|pid| !pid.is_empty());
                                        if named
                                            || child_view.turn_start_ms_prompt.is_none()
                                            || child_view.turn_start_ms == Some(ts)
                                        {
                                            child_view.turn_start_ms = Some(ts);
                                            if named {
                                                child_view.turn_start_ms_prompt =
                                                    meta.prompt_id.clone();
                                            }
                                        }
                                    }
                                    backdate_child_turn_clock(child_view);
                                }

                                let had_activity_before =
                                    child_view.session.tracker.activity().is_some();
                                let bg_stdout_routed =
                                    if let acp::SessionUpdate::ToolCallUpdate(ref tcu) =
                                        notif.request.update
                                    {
                                        route_bg_task_stdout(tcu, &mut child_view.session)
                                    } else {
                                        false
                                    };
                                if !bg_stdout_routed {
                                    child_view.session.handle_update(
                                        notif.request.update,
                                        &meta,
                                        &mut child_view.scrollback,
                                    );
```

- open_subagent_fullscreen
```rust
// from crates/codegen/xai-grok-pager/src/app/agent_view/subagent_takeover.rs:33-54
    /// Open the fullscreen subagent view for `child_sid`, replaying child `updates.jsonl` when the child scrollback is still empty (or the child finished).
    pub(crate) fn open_subagent_fullscreen(&mut self, child_sid: String) {
        let parent_session_id = {
            let Some(child) = self.subagent_views.get_mut(&child_sid) else {
                return;
            };
            // A child may be hydrated lazily from persisted state. Opening the
            // takeover is the final ownership boundary, so stamp the child
            // surface before its input/hints are consulted.
            child.mark_as_subagent_view();
            // Open on scrollback so q/Esc/arrows act on the transcript; a persistent child's composer is one Tab away.
            child.set_active_pane(crate::app::agent_view::AgentPane::Scrollback, true);
            child
                .child_link()
                .map(|link| link.parent_session_id().clone())
        };
        if self.active_subagent.as_deref() != Some(child_sid.as_str()) {
            self.close_subagent_fullscreen();
        }
        let replay_outcome = crate::app::subagent::ensure_subagent_child_replayed(self, &child_sid);
        tracing::debug!(
            child_sid = %child_sid,
```

- SubagentBlock construction for a spawned child, tracked by entry id
```rust
// from crates/codegen/xai-grok-pager/src/app/acp_handler/session_notification.rs:770-789
            }
            if workflow_run_id.is_none() {
                let block = crate::scrollback::blocks::SubagentBlock::started(
                    &description,
                    &child_session_id,
                    &subagent_type,
                    persona_display,
                    role_display,
                    model_display,
                    is_background,
                );
                let entry_id = agent.scrollback.push_block(RenderBlock::Subagent(block));
                agent.scrollback.set_last_running(true);
                if let Some(info) = agent.subagent_sessions.get_mut(&child_session_id) {
                    info.attempt.scrollback_entry_id = Some(entry_id);
                    info.attempt.is_background = is_background;
                }
            } else if let Some(info) = agent.subagent_sessions.get_mut(&child_session_id) {
                info.attempt.is_background = is_background;
            }
```

- SubagentBlock update on finish (background branch)
```rust
// from crates/codegen/xai-grok-pager/src/app/acp_handler/session_notification.rs:874-902
            let is_background = info_ref.is_some_and(|s| s.attempt.is_background);
            let description = info_ref.map(|s| s.description.clone()).unwrap_or_default();
            let existing_terminal = info_ref.and_then(|info| info.attempt.terminal_entry_id);
            if let Some(eid) = entry_id {
                agent.scrollback.finish_running(eid);
            }
            sync_subagent_activity(agent, &child_session_id, None);
            if is_background {
                let terminal_entry_id = if let Some(eid) = existing_terminal
                    && let Some(entry) = agent.scrollback.get_by_id_mut(eid)
                {
                    if let RenderBlock::Subagent(ref mut sb) = entry.block {
                        sb.kind = match status.as_str() {
                            "completed" => {
                                crate::scrollback::blocks::SubagentBlockKind::Completed {
                                    elapsed: elapsed_dur,
                                }
                            }
                            "cancelled" => {
                                crate::scrollback::blocks::SubagentBlockKind::Cancelled {
                                    elapsed: elapsed_dur,
                                }
                            }
                            _ => crate::scrollback::blocks::SubagentBlockKind::Failed {
                                elapsed: elapsed_dur,
                                error: error.clone(),
                            },
                        };
                        sb.turn_count = (turns > 0).then_some(turns);
```

- setting: definition (pager-owned bool)
```rust
// from crates/codegen/xai-grok-pager/src/settings/defs.rs:1411-1429
        // PAGER-owned, persisted to `[scrollback.scroll].respect_manual_folds` in pager.toml (NOT config.toml)
        // The live value is the appearance config (`AppView::set_appearance` fans changes out to every agent)
        // The flag is read at use time, so no restart
        SettingMeta {
            key: "respect_manual_folds",
            category: SettingCategory::Appearance,
            owner: SettingOwner::Pager,
            label: "Respect manual folds",
            description: "Keep manually folded blocks as-is while streaming and stop \
                          auto-scroll when expanding a block. Experimental.",
            keywords: &[
                "fold", "pin", "collapse", "expand", "thinking", "follow", "scroll",
            ],
            kind: SettingKind::Bool {
                default: crate::appearance::ScrollConfig::default().respect_manual_folds,
            },
            restart_required: false,
            hidden_in_minimal: false,
            external_only: false,
```

- setting: appearance config field and default (read at render time via appearance)
```rust
// from crates/codegen/xai-grok-pager-render/src/appearance/config.rs:353-368
    /// When false, uses ensure_selected_visible (the block may shift on screen).
    pub anchor_on_fold: bool,
    pub respect_manual_folds: bool,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self {
            margin: 0,
            min_page_fraction: 0,
            follow_indicator: FollowIndicator::Center,
            follow_auto_select: true,
            follow_by_overscroll: true,
            anchor_on_fold: true,
            respect_manual_folds: false,
        }
```

- setting: read at render/layout time
```rust
// from crates/codegen/xai-grok-pager/src/scrollback/state/nav.rs:938-946
        let respect_manual_folds = self.appearance.scrollback.scroll.respect_manual_folds;
        if let Some((id, entry)) = self.entries.get_index_mut(entry_idx)
            && entry.is_foldable()
            && entry.display_mode != DisplayMode::Expanded
        {
            entry.set_display_mode(DisplayMode::Expanded);
            if respect_manual_folds {
                entry.display_mode_pinned = true;
                tracing::debug!(
```

- setting: registry get arm
```rust
// from crates/codegen/xai-grok-pager/src/settings/registry.rs:807-807
        "respect_manual_folds" => Some(SettingValue::Bool(pager.respect_manual_folds)),
```

- setting: Action variant and router arm
```rust
// from crates/codegen/xai-grok-pager/src/app/actions.rs:591-591
    SetRespectManualFolds(bool),
```

- setting: router arm
```rust
// from crates/codegen/xai-grok-pager/src/app/dispatch/router.rs:1362-1362
        Action::SetRespectManualFolds(v) => set_respect_manual_folds(app, v),
```

- setting: setter with live apply and persist effect
```rust
// from crates/codegen/xai-grok-pager/src/app/dispatch/settings/setters.rs:1040-1070
pub(super) fn set_respect_manual_folds_inner(app: &mut AppView, new: bool) {
    let mut config = app.appearance.clone();
    config.scrollback.scroll.respect_manual_folds = new;
    app.set_appearance(config);
}

/// Set `respect_manual_folds` (registry-driven path).
/// PAGER-OWNED: live-applied to every agent via `AppView::set_appearance`.
/// Persisted to `[scrollback.scroll]` in pager.toml via `Effect::PersistSetting`.
pub(in crate::app::dispatch) fn set_respect_manual_folds(
    app: &mut AppView,
    new: bool,
) -> Vec<Effect> {
    let prev = app.appearance.scrollback.scroll.respect_manual_folds;
    if prev == new {
        return vec![];
    }
    set_respect_manual_folds_inner(app, new);
    refresh_open_settings_modals(app);
    tracing::info!(
        target: "settings",
        key = "respect_manual_folds",
        value = new,
        "setting changed",
    );
    app.show_toast(&save_success_toast("Respect manual folds", new));
    vec![Effect::PersistSetting {
        key: "respect_manual_folds",
        value: crate::settings::SettingValue::Bool(new),
        rollback_value: crate::settings::SettingValue::Bool(prev),
    }]
```

- setting: persist arm
```rust
// from crates/codegen/xai-grok-pager/src/app/effects/helpers.rs:1675-1685
        "respect_manual_folds" => {
            let SettingValue::Bool(b) = value else {
                return Err(kind_mismatch("respect_manual_folds", "Bool", &value));
            };
            tokio::task::spawn_blocking(move || crate::appearance::persist_respect_manual_folds(
                    b,
                ))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())
        }
```
