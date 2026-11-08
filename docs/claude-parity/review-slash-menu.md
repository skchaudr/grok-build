# Review: Claude slash menu / announcements (gc/claude-parity-r2)

Read-only review by reading; no cargo run. Paths under `crates/codegen/xai-grok-pager/src/`.

Compile check by reading: no errors found. Checked the `.zip().filter()` pattern binding (`**s` is `CommandSource`, Copy), the `map_or(&[] as &[&str], ..)` lifetime, the `rebuild_triggers` `&self` call inside the `commands.iter()` loop, the `(MenuGroup, usize)` zip in `mod.rs`, the `app.is_claude_session()` argument order at each assignment, the `pub mod claude_menu` declaration, the `Dummy` trait impl, and the `RemoteAnnouncement: Default + PartialEq`. Non-Claude behavior is unchanged: `claude` is false, so the `builtin_keys` filter and the `BLOCKED_ACP_NAMES` check evaluate as before. `claude_rank` is 0 and `trigger_is_skill` is false, so `MenuKey` and the fuzzy sort are unchanged. `menu_only` short-circuits to the old expression.

## Findings

1. **major** - `slash/claude_menu.rs:10-33`, `:37-72`, `:78`. `/login` is in the user's 30-day list (between /advisor and /statusline) but is missing from `TOP`.
   - The stub and builtin `/login` are in `HIDDEN`, so `/login` does not appear in the menu at all.
   - If the adapter does advertise `login`, it is `Acp`, is not in `ADAPTER_COMMANDS`, and is classified as a skill (rank `usize::MAX`), so it sinks to the bottom.
   - It stays typeable in both cases. With no adapter `login`, it hits the stub ("not supported by claude-agent"). With one, the adapter's command wins: the builtin is excluded from `builtin_keys` and `login` is in `YIELDED`.
   - Fix: add `"login"` to `TOP` after `"advisor"` and to `ADAPTER_COMMANDS`. Decide whether the "not supported" stub should show; if it should stay hidden, `TOP` still gives the right rank when the adapter advertises it.

2. **major (decision for Sab)** - `slash/claude_menu.rs:147-156`, `:159-161`. Pass bar says grok announcements/tips are not shown in Claude sessions. The code keeps every `severity == "critical"` announcement, which are Grok's own notices.
   - `session_tips` also drops the user's own `[tips]` from config, since `resolve_tips` merges user config.
   - Either state the critical exception to Sab or drop the `retain` and clear everything.

3. **major** - `slash/claude_menu.rs:77-93`. "Grok-only commands hidden" is a 15-name deny-list. About 45 other builtins stay visible between the top group and the skills: `remember`, `workflow`, `workflows`, `traces`, `doctor`, `skills`, `mcps`, `tasks`, `plan`, `view-plan`, `session-info`, `timeline`, `btw`, `debug`, `gboom`, `always-approve`, and others (names from `slash/commands/*.rs`).
   - I did not verify which of them work against the adapter.
   - Needs a keep/hide ruling per command from Sab, or an allow-list design.

4. **minor** - `slash/claude_menu.rs:74-76` comment and `HIDDEN` entry `recap`. The claim that hiding `recap` frees the name for the adapter is false in practice.
   - `CommandRegistry::new` puts `recap` in `hidden` (`registry.rs:~174`), and `rebuild_triggers` skips any command whose canonical name is in `hidden`, regardless of source.
   - `get_for_dispatch` also filters it (`registry.rs:~241`).
   - `set_recap_visible` is only called for the welcome prompt and the dashboard (`event_loop.rs:3076-3078`), so the adapter's `/recap` never shows.
   - Fix: in `apply_acp_commands`, for Claude, remove `recap` from `hidden` when the adapter advertises it (same pattern as the `SHELL_GATED_COMMANDS` loop). Or drop the claim and the comment. The test at `claude_menu.rs:232` does not cover this.

5. **minor** - `slash/external_common.rs:47-49`, `app/app_view.rs:5420-5430`, `app/app_view.rs:1725-1729`. `is_claude_label` is case-sensitive.
   - The registry label is `session_agent_name` first, else the command-derived label. `AppView::is_claude_session` (tips/announcements) uses only the command-derived label.
   - If the shell session info ever reports a name like "Claude Agent", the menu stops being shaped while tips/announcements stay Claude.
   - Fix: lowercase in `is_claude_label`, or give the registry the app-level Claude flag.

6. **minor (answers the ordering question)** - the label-after-first-sync risk is not real at startup.
   - Tips and announcements: `app.external_agent` and `app.external_agent_label` are set at `event_loop.rs:1070-1074`, before the announcement/tip resolution at `:1444-1465`. The runtime updates (`settings.rs:334`, `:486`) run after that.
   - Registry: the per-frame loop sets the label (`app_view.rs:5430`) before `set_external_agent` (`:5432`). On the first poll that call returns `true` (false to true), so `sync_acp_commands` reruns with `claude == true`. The initial sync at `app_view.rs:~1060` runs with `external_agent == false` and is superseded on the first tick.
   - Residual: new agent views from `lifecycle.rs:861`/`:991` sync once with the default label and `external == false`, then heal on the next tick. That is a one-frame window.
   - Latent: `set_external_agent_label` (`registry.rs:451-457`) only rebuilds triggers, not `apply_acp_commands`. A label flip after the first sync would apply menu hiding but not the name release (`builtin_keys`). Cheap hardening: re-apply the last ACP commands when `is_claude_session` flips.

7. **minor** - `slash/claude_menu.rs:37-72`, `:142`. `ADAPTER_COMMANDS` is a static allowlist. Any Claude command a newer adapter advertises that is not listed (and not in `TOP`) is classified as a skill and sinks to the skills group.
   - It is still reachable and typeable, so the failure is soft.
   - It needs a re-check on each adapter bump.
   - Not a merge-friendliness issue, since it is isolated in `claude_menu.rs`.

8. **minor** - `slash/external_common.rs:51-62`, `slash/claude_menu.rs:10-33`. `/hooks`, `/plugin` and `/effort` are kept as pager builtins in the menu but forwarded as typed text (the filter at `dispatch/prompt.rs:1022-1030`, outside this scope).
   - Per the code comment, `/hooks` and `/plugin` then print "isn't available in this environment". They are listed per Sab's list but do not function. Worth telling Sab.
   - Every other shared name (`/model`, `/rename`, `/rewind`, `/copy`, `/exit`, `/new`, `/resume`, `/settings`, `/export`, `/fork`) resolves to the pager builtin, because the builtin keeps its `builtin_keys` entry and the adapter copy is skipped as reserved. `/compact`, `/usage`, `/context` and `/login` are in `YIELDED_TO_EXTERNAL_AGENT`, so the builtin is skipped and the adapter's command (or the stub) owns the name. One entry each.
   - `/mode`, `/advisor`, `/status`, `/statusline`, `/artifacts` and `/remote-control` have no pager builtin, so they come from the adapter or are absent. This matches the docs comment at `claude_menu.rs:9`.

9. **minor** - upstream-merge surface in the 6 touched files: +101/-22 lines across about 14 hunks. `claude_menu.rs` is new (277 lines, all Claude data and tests).
   - The costly hunks are the re-indented `resolve_tips(...)` and `filter_expired(...)` wrappers in `event_loop.rs:1444-1466` and `settings.rs:331-343` and `:483-488`, and the `builtin_keys` chain rewrite in `registry.rs:~592-600`. Those will conflict if upstream touches those arguments.
   - Suggested fix: keep the original assignments untouched and add one line after each, for example `if app.is_claude_session() { app.tips.clear(); }` and `app.active_announcements.retain(..)`. That makes the diff purely additive.
   - The `MenuKey::new` signature change in `mod.rs` is unavoidable; keep it.
   - `AppView::is_claude_session` (`app_view.rs:1725`) duplicates the private `is_claude_session` in `dispatch/rewind.rs:20-23`. Merge them later if upstream allows.

10. **minor** - tests (all in `claude_menu.rs:163-277`). They are real:
    - `menu_rank`, `hides`, `session_announcements` and `session_tips` are tested directly.
    - The registry test drives `set_external_agent_with_label` + `set_acp_commands` + `triggers()`, so it covers the hide, name release and typed-in-full resolution paths.
    - I found nothing that should fail.
    - Gaps: no test through `SlashController`'s bare-`/` ordering (the `MenuKey` tuple, the group override, the fuzzy tie-break); no test of `trigger_rank` or `trigger_is_skill`; no test that `/model`, `/rename`, `/rewind` resolve to the builtin while the adapter advertises the same name; no test of the `recap` release (finding 4); no test that a non-Claude registry's `triggers()` is identical before and after.
    - Style: `claude_menu.rs:261` exceeds 100 columns, so `cargo fmt --check` may fail.

## Verdict

Compiles by reading and leaves non-Claude sessions unchanged, but the pass bar is only partly met. `/login` is missing from `TOP`, the "grok-only hidden" list is a partial deny-list, critical Grok announcements still show, and the adapter's `/recap` is still blocked.
