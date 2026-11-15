# Claude settings packet — re-prepared against new main

**Approved by Sab 2026-11-14 19:21 PST: single builder plus reviewer, Sonnet each, outside the three-packet workflow.** Re-prepared by Claude (Fable 5.2) on 2026-11-14 from the `claude-settings` packet in `packet-review.md`. The preparation agent's wording is kept wherever the facts did not change.

**Base commit:** `c1b43699` (main after F3 and Cass were merged directly)

## What changed from the original packet

1. Base moved from `7637f7cb` to `c1b43699`. F3 and Cass are on main, so the cross-task warnings about them are gone.
2. Context item 6 now names the three rows to hide, with file and line. The original said "Grok Approval/Plan-mode rows" and named none.
3. Context item 4 now says where the refresh call can go. `agent` is borrowed from `app` inside the branch, so the call cannot sit next to `apply_config_options`.
4. Context item 8 is replaced: F3's rate-limit insert is already in the `UsageUpdate` branch directly above; leave it alone.
5. Builder and reviewer are Sonnet 5.5, per Sab.

**Packet ID:** `claude-settings`
**Builder:** Claude Sonnet 5.5
**Reviewer:** Claude Sonnet 5.5, review only
**Starting contribution:** `7d4d917b8c3ae675086af38897fee67e0cf0ca1b` (already on main)

## Goal

Finish the Claude host batch already on main: one settings surface whose live rows match the running Claude session, and close the builder-reported gap where an open panel does not repaint on a server-pushed config update.

## Context and evidence

1. startingRef 7d4d917b is already on main. The batch from eca9236f through 7d4d917b is landed: resume via ACP for Claude, branding/version/changelog, running state, Agents naming, voice, single-page resume, and the settings rows. This packet finishes that landed work; it does not re-land those commits.
2. Brief docs/briefs/claude-settings-modal-2026-11-14.md. Builder report docs/review/claude-settings-modal-build-2026-11-14.md, section "Not done / for the lead" (lines 28-32).
3. Rows in settings/claude.rs: mode, model, effort, fast, agent. All five send session/set_config_option through Effect::SetClaudeOption, not Effect::SetSessionMode. Values come back through apply_config_options. Registry rows are appended by registry_with_claude_rows, not SettingsRegistry::defaults().
4. Unfinished refresh. In app/acp_handler/mod.rs `handle_inner`, the `ConfigOptionUpdate` branch (line 270) calls `models.apply_config_options(&update.config_options)` (line 279) and never calls `refresh_open_pi_settings` (app/dispatch/settings/ui.rs:532). A panel-originated change repaints through TaskResult; a server-pushed config_option_update leaves an open panel stale. Inside that branch `agent` is a mutable borrow taken from `app` at line 193, and `refresh_open_pi_settings` takes `&mut AppView`, so the call has to run after that borrow ends, only when the options changed.
5. Rows to hide for a Claude session. `agent_scope` (views/pi_settings/state.rs:935) returns `AgentScope::Any` for every row that is neither a `claude.*` key nor `external_only`, so these three Grok rows show under grok claude: `permission_mode` "Permission mode" (settings/defs.rs:1164), `remember_tool_approvals` "Remember tool approvals" (settings/defs.rs:1197), `plan_mode` "Plan mode" (settings/defs.rs:1830). `row_visible` (state.rs:949) is where scope meets `AgentKind`. Hide those three rows when the session agent is Claude. Pi rows keep the existing `AgentScope::Pi` rule; stock Grok and Pi sessions still show all three.
6. Existing tests to extend: views/pi_settings/tests.rs `rows_exist_only_for_a_claude_session` (698) and `pi_rows_never_show_for_a_claude_session` (728); app/dispatch/tests/settings.rs helpers `claude_config_options` (4025) and `claude_app` (4046), and `claude_option_refusal_toasts_and_keeps_the_tracked_value` (4146).
7. The `UsageUpdate` branch directly above (lines 253-269) now holds F3's rate-limit insert and the stale-zero guard. The `claude/sdkMessage` arm is at line 751. Neither is part of this packet.
8. Not observed on a live session: whether the adapter sends Fast as an on/off select or a boolean. The report says the pager does not opt into boolean config options. Do not change the Fast row.

## Allowed file paths

- `crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs`
- `crates/codegen/xai-grok-pager/src/app/dispatch/settings/ui.rs`
- `crates/codegen/xai-grok-pager/src/views/pi_settings/state.rs`
- `crates/codegen/xai-grok-pager/src/views/pi_settings/tests.rs`
- `crates/codegen/xai-grok-pager/src/app/dispatch/tests/settings.rs`
- `docs/review/claude-settings-modal-build-2026-11-14.md`

Dropped from the original list because nothing in this packet needs them: `settings/claude.rs`, `settings/layout.rs`, `settings/registry.rs`.

## Constraints

- No cargo, build, test run, install, push, or merge to main.
- Commit only the explicit task paths, never git add -A.
- Do not redo resume, branding, voice, Agents naming, or the single-page picker; they are already on main.
- Claude-only changes must not alter Pi or stock Grok settings, resume, or welcome behaviour.
- Do not touch the UsageUpdate branch or the claude/sdkMessage arm.
- No unrelated repository or workflow changes.

## Acceptance outcomes

1. An open Claude settings panel refreshes when ConfigOptionUpdate applies new config options, and a refused set still changes nothing.
2. Permission mode, model, thinking effort, fast (only when offered), and agent (only when offered) still send session/set_config_option and display the adapter's current value.
3. `permission_mode`, `remember_tool_approvals` and `plan_mode` rows are not shown for a Claude session; Pi rows stay Pi-only and stock Grok is unchanged.
4. Tests cover the server-pushed refresh and the Claude-only hiding of those three rows; the builder report's "Not done" note is updated to what landed.
