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
use crate::scrollback::blocks::subagent_preview::SubagentPreview;
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

/// One header line, plus a live preview of the child's transcript when the agent sends child updates.
/// Not foldable while there is no preview (stock Grok Build); with one, it folds between the header alone
/// and header plus preview, and the scrollback opens it unfolded when the first preview row lands.
/// Groupable only without a preview. Enter or Ctrl-F opens the subagent view.
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
    /// Live tail of the child's transcript, drawn under the header (capped at `[ui].subagent_preview_rows`; unset is 5, or 24 in acpx mode). Empty for agents
    /// that send no child updates, so those blocks stay one line.
    pub preview: SubagentPreview,
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
            preview: SubagentPreview::default(),
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
            preview: SubagentPreview::default(),
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
            preview: SubagentPreview::default(),
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
            preview: SubagentPreview::default(),
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
        // acpx mode: the "subagent" is a workflow or acpx step run by a peer agent, so the card
        // says "Step"; a normal Grok Build session keeps the stock "Subagent" wording.
        let noun = if crate::appearance::cache::load_acpx_mode() {
            "Step"
        } else {
            "Subagent"
        };
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
                // "Subagent running: " / "Subagent started: " = 18 chars; "Step …" is 5 shorter.
                let overhead =
                    noun.len() + 1 + verb.len() + meta.width() + activity_suffix.width() + stats.width();
                let desc = quoted_desc(&self.description, w.saturating_sub(overhead));
                let mut spans = vec![
                    Span::styled(format!("{noun} "), bold),
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
                let prefix = format!("{noun} completed in {time_str}: ");
                let prefix_len = prefix.len() + stats.len();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                let mut spans = vec![
                    Span::styled(format!("{noun} "), bold),
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
                let prefix = format!("{noun} failed in {time_str}{detail}: ");
                let prefix_len = prefix.len() + stats.len();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                let mut spans = vec![
                    Span::styled(format!("{noun} "), bold),
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
                let prefix = format!("{noun} cancelled in {time_str}: ");
                let prefix_len = prefix.len() + stats.len();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                let mut spans = vec![
                    Span::styled(format!("{noun} "), bold),
                    Span::styled(format!("cancelled in {time_str}: "), muted),
                    Span::styled(desc, muted),
                ];
                if !stats.is_empty() {
                    spans.push(Span::styled(stats, muted));
                }
                Line::from(spans)
            }
        };

        let mut lines = vec![line.into()];
        // Folded (and foldable): the header alone.
        let folded = self.is_foldable() && ctx.mode != DisplayMode::Expanded;
        if !self.preview.is_empty() && !folded {
            let rail = Span::styled("\u{2502} ", theme.muted());
            let rows = self
                .preview
                .render(
                    ctx.width.saturating_sub(2),
                    usize::from(crate::appearance::cache::load_subagent_preview_rows()),
                    &theme,
                );
            lines.extend(rows.into_iter().map(|row| {
                let mut spans = vec![rail.clone()];
                spans.extend(row.spans);
                Line::from(spans).into()
            }));
        }
        BlockOutput { lines }
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
        // Only a block with a preview has anything to unfold.
        !self.preview.is_empty()
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
        // A block showing a live preview is a step to read, not a row to fold into a group.
        self.preview.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::AppearanceConfig;

    fn test_ctx() -> BlockContext {
        ctx_in(DisplayMode::Collapsed)
    }

    fn ctx_in(mode: DisplayMode) -> BlockContext {
        BlockContext {
            mode,
            is_running: false,
            width: 120,
            raw: false,
            max_lines: None,
            appearance: AppearanceConfig::default(),
            is_selected: false,
            cwd: None,
        }
    }

    fn line_text(block: &SubagentBlock) -> String {
        block.output(&test_ctx()).lines[0]
            .content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    /// Rows of the block below the header line, with a preview of 30 lines and the setting at `rows`.
    fn preview_height_with_setting(rows: u8) -> usize {
        preview_height(Some(rows), false)
    }

    /// Rows below the header for a 30-line preview, with `[ui].subagent_preview_rows` at `setting` (`None` = unset).
    fn preview_height(setting: Option<u8>, acpx_mode: bool) -> usize {
        std::thread::spawn(move || {
            crate::appearance::cache::set_subagent_preview_rows_setting(setting);
            crate::appearance::cache::set_subagent_preview_acpx_mode(acpx_mode);
            let mut block = SubagentBlock::started(
                "read the logs",
                "child-1",
                "general-purpose",
                None,
                None,
                None,
                false,
            );
            let text = (0..30).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
            let update: agent_client_protocol::SessionUpdate = serde_json::from_value(serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text }
            }))
            .expect("valid session update");
            block.preview.ingest(&update);
            block.output(&ctx_in(DisplayMode::Expanded)).lines.len() - 1
        })
        .join()
        .expect("render thread")
    }

    #[test]
    fn configured_preview_rows_change_the_preview_height() {
        assert_eq!(preview_height_with_setting(5), 5);
        assert_eq!(preview_height_with_setting(12), 12);
        // The default of 24 still caps a longer preview.
        assert_eq!(preview_height_with_setting(24), 24);
    }

    #[test]
    fn unset_preview_rows_is_5_in_a_normal_session_and_24_in_acpx_mode() {
        assert_eq!(preview_height(None, false), 5);
        assert_eq!(preview_height(None, true), 24);
    }

    #[test]
    fn explicit_preview_rows_win_in_both_modes() {
        assert_eq!(preview_height(Some(9), false), 9);
        assert_eq!(preview_height(Some(9), true), 9);
    }

    fn block_with_preview(with_preview: bool) -> SubagentBlock {
        let mut block =
            SubagentBlock::started("read the logs", "child-1", "general-purpose", None, None, None, false);
        if with_preview {
            let update: agent_client_protocol::SessionUpdate = serde_json::from_value(serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "first\nsecond\nthird" }
            }))
            .expect("valid session update");
            block.preview.ingest(&update);
        }
        block
    }

    #[test]
    fn block_with_a_preview_folds_between_header_and_header_plus_preview() {
        let block = block_with_preview(true);
        assert!(block.is_foldable());
        let folded = block.output(&ctx_in(DisplayMode::Collapsed)).lines.len();
        let open = block.output(&ctx_in(DisplayMode::Expanded)).lines.len();
        assert_eq!(folded, 1, "folded shows the header line only");
        assert!(open > 1, "unfolded shows the preview under the header");
        assert!(!block.is_groupable());
    }

    #[test]
    fn block_without_a_preview_is_unchanged() {
        let block = block_with_preview(false);
        assert!(!block.is_foldable());
        assert!(block.is_groupable());
        assert_eq!(block.default_display_mode(), DisplayMode::Collapsed);
        for mode in [DisplayMode::Collapsed, DisplayMode::Expanded] {
            assert_eq!(block.output(&ctx_in(mode)).lines.len(), 1);
        }
    }

    #[test]
    fn compact_tokens_scales_units() {
        assert_eq!(compact_tokens(0), "0");
        assert_eq!(compact_tokens(42), "42");
        assert_eq!(compact_tokens(999), "999");
        assert_eq!(compact_tokens(1_234), "1.2k");
        assert_eq!(compact_tokens(12_345), "12k");
        assert_eq!(compact_tokens(129_540), "130k");
        assert_eq!(compact_tokens(1_234_567), "1.2M");
        assert_eq!(compact_tokens(12_345_678), "12M");
    }

    #[test]
    fn format_stats_combines_turns_and_tokens() {
        // Neither field set - no suffix.
        assert_eq!(format_stats(None, None), "");
        // Zero values are treated as absent (matches Finished handler which
        // uses `(turns > 0).then_some(turns)`).
        assert_eq!(format_stats(Some(0), Some(0)), "");
        assert_eq!(format_stats(Some(0), Some(1_500)), " \u{2014} 1.5k tokens");
        assert_eq!(format_stats(Some(1), None), " \u{2014} 1 turn");
        assert_eq!(format_stats(Some(2), None), " \u{2014} 2 turns");
        assert_eq!(
            format_stats(Some(3), Some(4_500)),
            " \u{2014} 3 turns \u{00b7} 4.5k tokens"
        );
    }

    #[test]
    fn started_line_renders_stats_after_activity() {
        let mut block = SubagentBlock::started(
            "research the parser",
            "child-1",
            "general-purpose",
            None,
            None,
            None,
            false,
        );
        block.activity_label = Some("Thinking".into());
        block.turn_count = Some(2);
        block.tokens_used = Some(4_500);

        let text = line_text(&block);
        assert!(
            text.contains("Subagent running: “research the parser”"),
            "missing prefix, got: {text}"
        );
        assert!(
            text.contains("Thinking"),
            "missing activity label, got: {text}"
        );
        assert!(
            text.contains("2 turns \u{00b7} 4.5k tokens"),
            "missing stats suffix, got: {text}"
        );
    }

    #[test]
    fn completed_line_renders_stats_without_activity() {
        let mut block =
            SubagentBlock::completed("do the thing", "child-2", Duration::from_secs(43));
        block.turn_count = Some(7);
        block.tokens_used = Some(129_540);

        let text = line_text(&block);
        assert!(
            text.contains("completed in 43s"),
            "missing elapsed, got: {text}"
        );
        assert!(
            text.contains("7 turns \u{00b7} 130k tokens"),
            "missing stats suffix, got: {text}"
        );
    }

    /// The lifecycle card's noun. A normal Grok Build session says "Subagent" (stock wording);
    /// acpx mode (the workflow/flow driver) says "Step", because what runs is a step of the run,
    /// not a subagent. Runs in a fresh thread because the acpx-mode flag is a sticky thread-local.
    #[test]
    fn card_noun_is_step_in_acpx_mode_and_subagent_in_a_normal_session() {
        let step_card = std::thread::spawn(|| {
            crate::appearance::cache::set_subagent_preview_acpx_mode(true);
            let mut block = SubagentBlock::started(
                "grok \u{00b7} grok-build",
                "child-1",
                "acpx step",
                None,
                None,
                None,
                false,
            );
            block.activity_label = Some("Thinking".into());
            (
                line_text(&block),
                line_text(&SubagentBlock::completed(
                    "grok \u{00b7} grok-build",
                    "child-1",
                    Duration::from_secs(37),
                )),
                line_text(&SubagentBlock::failed(
                    "grok \u{00b7} grok-build",
                    "child-1",
                    Duration::from_secs(5),
                    Some("boom".into()),
                )),
                line_text(&SubagentBlock::cancelled(
                    "grok \u{00b7} grok-build",
                    "child-1",
                    Duration::from_secs(5),
                )),
            )
        })
        .join()
        .expect("acpx-mode render thread");
        let (started, completed, failed, cancelled) = step_card;
        assert!(
            started.starts_with("Step running: \u{201c}grok \u{00b7} grok-build\u{201d}"),
            "acpx started card must name a step, got: {started}"
        );
        assert!(
            completed.starts_with("Step completed in 37s: "),
            "acpx completed card must name a step, got: {completed}"
        );
        assert!(
            failed.starts_with("Step failed in 5.0s (boom): "),
            "acpx failed card must name a step, got: {failed}"
        );
        assert!(
            cancelled.starts_with("Step cancelled in 5.0s: "),
            "acpx cancelled card must name a step, got: {cancelled}"
        );

        let stock_card = std::thread::spawn(|| {
            crate::appearance::cache::set_subagent_preview_acpx_mode(false);
            let mut block = SubagentBlock::started(
                "research the parser",
                "child-1",
                "general-purpose",
                None,
                None,
                None,
                false,
            );
            block.activity_label = Some("Thinking".into());
            (
                line_text(&block),
                line_text(&SubagentBlock::completed(
                    "do the thing",
                    "child-2",
                    Duration::from_secs(43),
                )),
            )
        })
        .join()
        .expect("stock render thread");
        let (started, completed) = stock_card;
        assert!(
            started.starts_with("Subagent running: \u{201c}research the parser\u{201d}"),
            "normal session keeps the stock wording, got: {started}"
        );
        assert!(
            completed.starts_with("Subagent completed in 43s: "),
            "normal session keeps the stock wording, got: {completed}"
        );
    }
}
