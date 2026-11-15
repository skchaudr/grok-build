# Review: claude-settings-finish (commit 19cc2388), 2026-11-14

Verdict: PASS

## Findings
None blocking. Read-only review; nothing compiled or run.

## Verified correct (from code)
- `config_options_changed` is declared before the `if/else` chain, set only in the `ConfigOptionUpdate` branch from `apply_config_options`' return, and read after the `agent` borrow ends (acp_handler/mod.rs ~234, 281, 446-448). `refresh_open_settings_modals(app)` is `pub(crate)` and calls `refresh_open_pi_settings`, which rebuilds `state.claude` from `agent.session.models.config_options` and calls `rebuild_rows`. `UsageUpdate` branch and `claude/sdkMessage` arm untouched. A refused set does not reach this path.
- Server-push test: `claude_app` sets session id `sess-claude` and a model-category option, so `model_config_id` is Some and the branch condition holds; mode default->plan makes `apply_config_options` report a change (config_options differ). Without the fix the panel keeps `default`, so the assertion fails (not vacuous). `handle`, `AcpArgs`, `new_for` (5 args), `registry_with_claude_rows(&SettingsRegistry)` via deref of `Arc`, `build_pager_snapshot`, `value_of(&'static str)`, `SettingValue: PartialEq + Debug`, and pub `state.claude` all match their definitions.
- Hiding test: all three keys exist in `SettingsRegistry::defaults()` (defs.rs 1164, 1197, 1830) with `AgentScope::Any`; Grok and Pi stay visible, Claude hidden. Fails if the new block is removed. `row_visible` and `AgentKind` (Debug, PartialEq, Copy) are imported in `claude_rows`.
- `row_visible` change touches only the Claude case; Pi and stock Grok unchanged. Existing Claude row tests count only `claude.*` keys, unaffected.
- Only allowed files changed; doc "Not done" section matches the code.

## Not checked
No tests were run; `PiSettingsState` field privacy was checked only for `claude` (pub).
