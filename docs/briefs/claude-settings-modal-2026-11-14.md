# Brief: Claude settings modal for grok claude (builder)

You build. NO cargo (no build/check/test); review your own change from code and write tests without running them. Commit and push ONLY the files you touched, by explicit path (never `git add -A`), with the trailer `Co-authored-by: claude-code sonnet-5.5`. Never pkill/killall by name. Report by 45 minutes after you start; write a short report to `docs/review/claude-settings-modal-build-2026-11-14.md`.

## Sab's words (outrank everything below)
- "grok-pi created a killer grok pi settings modal, all featured towards pi, and it really made it seem like it was meant for Pi. now ours has been working surprisingly well ... it's becoming a preferred home to claude code TUI"
- "Unqueue the settings modal"
- "I am no longer allowing things that are almost entirely done already to be done piecemeal"
- Any user-visible "Grok" under a Claude host must come from the host brand, not be hardcoded.

## Goal
Under `grok claude` (Claude is the only agent on offer), the settings modal (F2 / `/settings` / Cmd+P settings, `Action::OpenPiSettings`) shows a Claude section with live rows that change the running Claude session. Under Pi or stock Grok nothing changes.

## Facts from a read-only scout (verify each against code before relying on it)
- Pi's rows are manifest bools (xai-grok-shell `host_features.rs`) that only write grok's config.toml; Claude cannot use that path. Do NOT use the manifest path for Claude.
- Claude's live settings are ACP session config options from the adapter (`~/.local/share/claude-agent-acp/current/dist/acp-agent.js` `buildConfigOptions` ~6923, `setSessionConfigOption` ~4762): ids `mode` (select: default, acceptEdits, plan, auto, bypassPermissions only if allowed), `model` (select), `effort` (select, category thought_level), `fast` (boolean or on/off select, only when the model supports it), `agent` (select, only if custom agents exist).
- Pager already has: `acp/model_state.rs:775-802` `model_switch_wire` (SetSessionConfigOptionRequest for model and thought level), `Effect::SetSessionMode` (`app/effects/mod.rs` ~1942), incoming `config_option_update` handled at `app/acp_handler/mod.rs:272` -> `model_state.rs:319`, which parses only model and thought level. Mode, fast and agent values are not tracked yet.
- Settings registry: `settings/registry.rs` (`SettingMeta`, `host_feature_setting_meta` ~576-626, `external_only`), layout `settings/layout.rs` (`sections_for`), modal state `views/pi_settings/state.rs` (`row_visible` ~899, `build_rows` ~909), actions `views/pi_settings/actions.rs`, dispatch `app/dispatch/settings/ui.rs:450-509`, setters `app/dispatch/settings/setters.rs` (~2619 `set_host_feature_bool`, toast "restart grok-pi to apply" is hardcoded Pi wording at ~2638), router `app/dispatch/router.rs` ~1435.
- Row visibility is global (`external_agent_active()`), not per session. Pi rows would show in Claude sessions and vice versa when both are on offer. Add a per-agent scope so Claude rows show only for a Claude session and Pi rows only for Pi.
- Per-session checks: `AppView::is_claude_session()` (app_view.rs ~1974), `is_pi_session()` (~1982).

## Do
1. Per-agent scope on settings rows (Claude vs Pi vs any), enforced in `row_visible`/`build_rows` using the focused session.
2. A "Claude" section in the settings modal with live rows: Permission mode, Thinking effort, Fast mode (only when the session offers it), Agent (only when the session offers it), Model. Each shows its current value and applies through the existing ACP wire (`SetSessionConfigOptionRequest` / `SetSessionMode`), with the value tracked from `config_option_update` so the row reflects reality. Reuse the existing select/picker machinery of the settings modal if it has one; do not invent a new widget system.
3. Fix the hardcoded "restart grok-pi to apply" toast so it names the host.
4. Tests (unit, written not run): row scoping per agent kind; the Claude rows exist only for Claude sessions; mode/effort/fast selection produces the right request; snapshot-style test for the section if the modal has snapshot tests.
5. Leave these files alone, the lead is editing them: `app/agent_view/session.rs`, `app/acp_handler/mod.rs` (the `_` arm and a new `claude/sdkMessage` arm), `app/app_view.rs` `tick`, `app/effects/helpers.rs` `to_meta`, `slash/claude_menu.rs`, `app/dispatch/prompt.rs`, `xai-grok-pager-bin/src/main.rs`. If you need a change in one of them, put it in your report instead.

## Report
List files changed, each row's wire call, anything you could not finish and why, any user-visible "Grok" left in the Claude section. Nothing else.
