# Claude settings modal: build report (2026-11-14)

Nothing was compiled or run (brief: no cargo). Every line below was checked by reading code and the adapter source only.

## Files changed
- `settings/claude.rs` (new): Claude rows, parsing of the session's config options, `session/set_config_option` request builder, `registry_with_claude_rows`, unit tests.
- `settings/{mod,layout,registry}.rs`: module export, "Claude" sidebar section in the Agent tab, `DynamicEnumSource::ClaudeSession`.
- `acp/model_state.rs`: `ModelState.config_options` keeps the full advertised option list (carried in `SessionModelState.meta["x.ai/configOptions"]`, so create/load/resume need no new plumbing; `apply_config_options` refreshes it on every `config_option_update`).
- `views/pi_settings/{state,actions,input,render,mod}.rs`: `AgentKind` scope on `row_visible`/`build_rows` (Claude rows only for Claude, Pi `external_only` rows never for Claude), `claude` options on the panel state, chooser/toggle/`d`/display wiring.
- `app/actions.rs`, `app/effects/mod.rs`, `app/dispatch/{router,task_result}.rs`, `app/dispatch/settings/{setters,ui}.rs`: `Action::SetClaudeOption`, `Effect::SetClaudeOption`, `TaskResult::ClaudeOptionSet`, open/refresh of the panel with the session's options, host-named restart toast.
- Tests: `settings/claude.rs`, `views/pi_settings/tests.rs` (`claude_rows` module), `app/dispatch/tests/settings.rs`, `setters.rs` (`host_restart_name_tests`).

## Rows and wire calls
All five go out as `session/set_config_option` on the focused Claude session (`Effect::SetClaudeOption`); the reply's `configOptions` is applied with `ModelState::apply_config_options`, so the row shows what the adapter reports, and a refusal toasts and changes nothing.

| Row | configId | value |
| --- | --- | --- |
| Claude permission mode | `mode` | option value id (`default`, `acceptEdits`, `plan`, `auto`, `bypassPermissions` if offered) |
| Model | `model` | option value id |
| Thinking effort | `effort` | option value id |
| Fast mode (only when offered) | `fast` | `on`/`off` select value; a bool if the adapter types it `boolean` |
| Agent (only when offered) | `agent` | option value id |

Mode goes through `set_config_option`, not `Effect::SetSessionMode`: the adapter's `setSessionConfigOption` runs `selectMode` + `publishCurrent`, so `current_mode_update` still reaches the pager, and one wire serves all five rows.

Registry: the Claude rows are appended to the registry when the panel opens (`registry_with_claude_rows`), never to `SettingsRegistry::defaults()`, so the upstream settings modal and its registry-wide invariant tests do not see them. Label is "Claude permission mode" because the Agent tab already has a "Permission mode" row (Grok's own).

## Not done / for the lead
- `app/acp_handler/mod.rs` (yours): after `models.apply_config_options(...)` in the `ConfigOptionUpdate` branch, call `crate::app::dispatch::settings::ui::refresh_open_pi_settings(app)` if you want an open panel to repaint on a server-pushed change. A change made from the panel repaints already.
- Grok's own Approval/Plan-mode rows in the Agent tab still show under `grok claude`. They are not Claude's settings and I left them; hiding them is one more scope value in `agent_scope`.
- Boolean-typed Fast mode is coded and unit-tested at the request level; the pager does not opt into boolean config options, so the live adapter should send the on/off select. Not seen on a live session.
- Nothing rendered on a real screen; the panel render test only checks text in a `Buffer`.

## User-visible "Grok" in the Claude section
None. The sidebar section is the literal "Claude" (not read from the host brand, whose title is "Claude Code"). The restart toast now names the host brand (`grok-pi` when no brand is set).
