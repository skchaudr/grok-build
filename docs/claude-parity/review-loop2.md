# Review: 32f13dec vs 82c72bbe (gc/claude-parity-r2)

Read-only. Nothing compiled or run. Paths under `crates/codegen/xai-grok-pager/src/`.

## Findings

1. **minor (perf; root cause pre-existing)** - `slash/registry.rs:~597` (`self.acp_catalog = commands.to_vec()`), root cause `slash/registry.rs:472-500` + `app/app_view.rs:5428-5441`.
   - The re-apply is every `tick()`, for every external agent (pi, cursor, codex, claude). `set_external_agent_with_label` returns early only for `!flag_changed && !enabled`. With `enabled == true` and nothing changed it falls through and returns `true` (identical in 82c72bbe). `tick()` then sees `external_catalog_changed == true` and runs `sync_acp_commands` (`apply_acp_commands`, `rebuild_triggers`, `refresh_slash`), clones the catalog into `bootstrap_commands_update`, and sets `needs_redraw = true`. The `agent.acp_synced_generation` guard never gets a chance.
   - New in this commit: one full `Vec<AvailableCommand>` clone per tick (name, description, hint, meta per command) on all external agents, not just Claude. The new `hides()` scans (about 30 names x commands, twice per tick) and `is_claude_session()` are negligible next to the existing per-tick `Arc::new(AcpSlashCommand::from(..))` and trigger-string rebuilds.
   - Cheap gate for this commit only: `if self.acp_catalog.as_slice() != commands { self.acp_catalog = commands.to_vec(); }` (`AvailableCommand` is `PartialEq`, schema client.rs:435). That is a compare with no allocation. It cannot be restricted to Claude, because the flip case needs the catalog saved while the session was still non-Claude.
   - Real fix (separate change, alters non-Claude redraw cadence, so not "byte-for-byte"): make `set_external_agent_with_label` return `false` when `!flag_changed` and the label did not change. Then the `acp_synced_generation` gate in `tick()` works as designed.

2. **minor (decision)** - `slash/claude_menu.rs:92`. `announcements` is in `HIDDEN`, which conflicts with the pass bar.
   - `/announcements hide|show` (`slash/commands/announcements.rs`) is pager-local (`Action::AnnouncementsHide/Show`, no x.ai request) and is the control for the banner the bar says must keep showing. It is already self-gating: `visible()` returns `ctx.has_session_announcements`.
   - Fix: drop `"announcements"` from `HIDDEN`.
   - Same class, pager-local but hidden by the "earlier decision" rather than the rule: `tutorial`, `docs`, `release-notes`, `import-claude`, `voice`. Confirm with Sab that these stay hidden; `.claude-menu-undecided.md` already lists them as not-by-rule.

3. **minor (design gap)** - `slash/registry.rs:~716-722` (hide is menu-only). A hidden x.ai builtin typed in full still resolves and sends its x.ai request, so `/workflows`, `/share`, `/session-info`, `/skills` and `/mcps` still give "Method not found". The test at `claude_menu.rs` asserts this is intended ("typed in full still resolves").
   - Alias leak: when the adapter advertises `recap`, builtin `recap` stays in `key_to_index` under alias `summarize` (it is menu-only, not hidden). `/summarize` then runs the builtin and sends `x.ai/recap`. The canonical `recap` key is overwritten by the Acp command, the alias is not.
   - Fix if Sab wants "would fail" to mean "cannot be typed": in `rebuild_triggers`, for Claude and `source != Acp && hides(..)`, skip the key insertion for the canonical name and its aliases (the typed text then goes to the CLI). Otherwise leave it and say so.

4. **minor (unverified keep)** - items in the "kept" table that I traced and could not confirm as x.ai-free.
   - `/history`: `Action::OpenHistorySearch` is local, but the history is filled by `Effect::FetchPromptHistory` (`x.ai/prompt_history`, `app/effects/mod.rs:1927`, emitted at `app/dispatch/session/lifecycle.rs:1488`, `:1630`, `session/load.rs:1361`). On Claude that request fails silently, so the picker likely shows only in-session prompts. Check there is a non-ACP fallback.
   - `/dashboard`: `Effect::FetchRoster` (`x.ai/sessions/list`, `app/effects/mod.rs:862`) and `Effect::FetchDashboardSessions` (`x.ai/session/list`, `:899`) have no external-agent branch (unlike `FetchSessionList` at `:726`). Failures degrade to `RosterFailed` or an empty list, so the dashboard opens but is empty or shows a roster error.
   - `/settings` (alias `config`) and `/cd` (`Action::DashboardChangeLocation`): not traced past the action (same as the table).

5. **minor (nit)** - Claude detection is split. The registry uses `session_agent_name` first (`app_view.rs:5425-5430`) for `is_command_complete` and the menu. Dispatch and fork use `app.external_agent_label` (command-derived). If they ever disagree, `/effort` bare would count as complete in the registry but the dispatch filter would not forward it, and the pager `/effort` would run. Both resolve to "claude" for `grok claude`, so this is latent only. Fix: expose the registry's flag to dispatch, or use the registry's `is_claude_session()` in the `prompt.rs` filter.

6. **minor** - `app/effects/mod.rs:5026-5030`. The `--fork` startup path passes `Some(new_session_id)`, which now fails loudly for Claude ("can't fork into a chosen session id"). That is better than silent. The earlier review's claim that the startup fork path now works for Claude is wrong. Fine to ship; do not claim startup `--fork` works.

## Checked, no problem

- **Compile (by reading):**
  - `claude_menu.rs`: imports resolve (`registry::{CommandRegistry, CommandSource, CommandTrigger}` are pub; `commands_by_index`, `trigger.{canonical,source,provenance,command_index}` exist). `map_or(&[] as &[&str], ..)` unifies. `Dummy` impls the required `SlashCommand` methods only (`name`, `description`, `usage`, `run`).
  - `registry.rs`: the `.zip().filter(|(c, s)| .. **s)` and `flat_map(|(c, _)| ..)` pattern types are right. `std::mem::take` plus re-apply restores `acp_catalog`. `apply_acp_commands` does not rebuild, but both callers (`set_external_agent_label`, `set_external_agent_with_label`) rebuild after it.
  - `mod.rs`: `groups: Vec<(MenuGroup, usize)>` stays aligned with `rows` and `canonicals`, because all three are pushed together on the `seen.insert` path. The `MenuKey::new` call sites are updated (only one exists). `claude_ranks` indexes `visible_triggers`, the same index space as `owns_typed_name`.
  - `prompt.rs`: `Option<&Arc<dyn>>::filter(..).cloned()` type-checks; `external_claude` is defined at `:853` in the same function.
  - `effects/mod.rs`: `ForkSessionRequest::new(SessionId, PathBuf)`, `resp.session_id`, `SessionFlags.external_agent{,_label}` (`effects/helpers.rs:545-548`) and `TaskResult::ForkSessionReady` fields all match. `claude` is a `bool` computed before the `spawn`. The early `return` inside `async move` leaves the non-Claude tail untouched.
  - `rewind.rs`: the private `is_claude_session` has no remaining callers (rg).
- **Tests (read through, expected to pass):**
  - All `claude_menu` unit tests, including the `SlashController` bare-`/` order test: ranks compact 0, usage 1, model 2, copy 3, exit/quit 4, new 6; `fast` and `theme` at `TOP.len()`; skills last and alphabetical. The alias row `/exit` is deduplicated by `command_index`, so it does not appear.
  - The new `fork.rs` tests: `dispatch_fork` returns `[Effect::ForkSession]` for Claude in a git repo with no modal, and toasts for `--worktree`. `show_toast` writes `agent.toast` as `Option<(String, u8)>`, matching `t.0`.
  - The `prompt.rs` forwarding test: it follows the pattern of the existing `/usage` test, which also asserts `effects.first()`. I did not trace every effect that could precede `SendPrompt` in the Claude branch.
  - `external_common` tests: `agent_label_from_cmd` on the `env -u ... claude-agent-acp` line gives "claude-agent". Case-insensitive and basename matching in `is_claude_label` handles "Claude Agent" and `@agentclientprotocol/claude-agent-acp`.
- **Fixes from the prior reviews, applied correctly:**
  - `login` is in `TOP` and `ADAPTER_COMMANDS`; the hide applies only to non-Acp sources.
  - The recap release is gated on whether the adapter advertises it, and recovers on a label flip through `reapply_acp_if_claude_flipped`.
  - The `/fork` worktree modal is skipped for Claude, and `--worktree` gives a toast.
  - A bare `/effort` is complete for Claude (`is_command_complete`, then `is_typed_slash_selected` takes the "send unchanged" path).
  - `is_claude_label` is case-insensitive and uses the basename.
  - Announcements and tips are no longer touched; the diff contains no `event_loop.rs` or `settings.rs` changes, so that surface is fully additive.
- **Kept commands vs x.ai requests:** `/plan` (`SetSessionMode`), `/tasks`, `/queue`, `/find`, `/jump`, `/expand`, `/transcript`, `/export`, `/copy`, `/theme`, `/timeline`, `/doctor`, `/traces` are local. `Effect::TogglePlanMode` (`x.ai/toggle_plan_mode`) has no emitter outside its definition. `/resume` uses the standard `ListSessionsRequest` for external agents (`effects/mod.rs:726`). See finding 4 for the exceptions.
- **Hidden commands that are pager-local:** `announcements` (finding 2), `tutorial`, `docs`, `release-notes`, `import-claude`, `voice`, `loop` (InjectSkill, tool-gated). The rest of `HIDDEN` sends x.ai requests, matching the table. `HIDDEN` names all match real builtin canonical names, including the `IMAGINE_*` constants.
- **Non-Claude regressions:** none in behavior. With `claude == false` the `builtin_keys` filter is the identity, `BLOCKED_ACP_NAMES` still applies, `menu_only` short-circuits to the old expression, `claude_rank` is 0, and the `is_command_complete` and dispatch filters are gated on Claude. The only non-Claude side effect is the `acp_catalog` clone per tick (finding 1).

## Verdict

Compiles by reading, prior-review fixes are applied correctly, and non-Claude behavior is unchanged. Before merge: guard the `acp_catalog` clone (finding 1) and rule on `announcements` (finding 2). Findings 3 to 6 are decisions or follow-ups, none blocking.
