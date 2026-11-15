# Scout: map the areas for "5h usage in the footer"

Read-only scout. Brief: `docs/briefs/scout-usage-footer-areas-2026-11-15.md`. Every row cites a file:line that was opened or matched by search.

Correction to earlier notes: `docs/HARNESS-FACTS.md` and `docs/SCOUT-claude-usage-freshness-2026-11-15.md` say the adapter never calls `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`. It does: `structuredUsageMarkdown` calls it for the `/usage` command (see A4). It is not called at session start or turn end.

## A. Adapter

Adapter root: `~/.local/share/claude-agent-acp/0.76.0`. `~/.local/share/claude-agent-acp/current` is a symlink to `0.76.0`.

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js:959-972` | `newSession` awaits `createSession`, returns its response, and schedules `sendAvailableCommandsUpdate` in a `setTimeout` at 967. No usage call. | Await `session.query.usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` once after creation and emit a `usage_update` with the rate-limit windows. Fire-and-forget so session start is not blocked. | ~15 | Whether a `usage_update` before any turn renders in the pager (scout measured the call at 1.8 s, not the render). The pager's `UsageUpdate` branch also calls `apply_context_used(used, size)` (`acp_handler/mod.rs:262-268`), so the update must carry real `used`/`size` or the context meter shows zero (inferred). |
| `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js:3484-3495` | Turn-end `usage_update`, sent only when `lastAssistantTotalUsage !== null`. | Fire the same `get_usage` at turn end and send the windows in `_meta`. Do not await it inside the turn. | ~15 | Whether a second `usage_update` in the same turn disturbs context accounting (inferred, not run). |
| `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js:4285-4297` | Live `rate_limit_event` forwarded as `_meta["_claude/rateLimit"]` (one window per event). Dropped until a turn has reported usage (scout: `:4285-4297`). | No change needed for the live path. | 0 | None. |
| `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js:82-107` and call at `:91` | `structuredUsageMarkdown(query, signal, logger)` calls `query.usage_EXPERIMENTAL...()` inside a 5 s race and parses the result. Used only for the `/usage` slash command (`:1422-1425` detects it, `:1982` runs it). | Reuse the same call and parser for startup and turn end. Factor the call out of this function. | ~10 if factored | Sab's `/usage` output path must stay unchanged. |
| `~/.local/share/claude-agent-acp/0.76.0/dist/usage-markdown.js:5,31-40,58` | `parseUsageResponse` reads `five_hour` and `seven_day` as `{utilization: 0-100 percent, resets_at: ISO string}`. | The pager's `parse_rate_limit` (`views/side_panel.rs:76-89`) expects `utilization` as 0..1 and `resetsAt` as epoch seconds. Convert in the adapter, or add a second parser in the pager. | ~5 | Which side converts. Sab's choice. |
| `~/.local/bin/claude-agent-acp:1-2` | Launcher: `exec node "$HOME/.local/share/claude-agent-acp/current/dist/index.js" "$@"`. | Nothing, unless the version changes. | 0 | None. |
| `~/.dsh/scripts/claude-agent-acp-update:24-29` | `repo_root` is `~/.dsh`; patch dir defaults to `~/.dsh/patches/claude-agent-acp`; default version `0.76.0`. The updater re-applies patches on each install. | A new patch file `0003-*.patch` in `~/.dsh/patches/claude-agent-acp/` survives an adapter upgrade. Patches are not in the grok-build repo. | 1 patch file | Whether a `~/.dsh` patch is committed anywhere Sab tracks. Decision for Sab. |
| `~/.dsh/patches/claude-agent-acp/` (contents: `0001-auth-required-message.patch`, `0002-steer-settle-at-steered-result.patch`) | Two patches already applied to the 0.76.0 dist. | A `0003` patch applies after them; `0002` touches the steer settle path, so conflicts are possible. | 0 | Conflict with `0002` not tested. |
| `scripts/hub/gb:51-56` | `GB_CLAUDE_ACP` overrides; else `~/.local/bin/claude-agent-acp` if executable; else `claude-agent-acp` on PATH. | None. | 0 | None. |

## B. Pager state

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:48-61` | `Info` (panel snapshot) holds `rate_limits: BTreeMap<String, RateWindow>` (`:57-58`) and `five_hour_pct: Option<u8>` (`:60`, statusline-file fallback). | Footer reads the same fields; no new struct needed. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:69-72` | `RateWindow { pct: u8, resets_at: Option<i64> }`, epoch seconds. | Reuse for 5h and 7d. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:76-89` | `parse_rate_limit` reads `_claude/rateLimit` (one window per call), `utilization * 100`, epoch `resetsAt`. | Reuse if A converts to this shape. | 0 | Shape mismatch with `get_usage` (see A5). |
| `crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs:254-262` | `UsageUpdate` branch: `parse_rate_limit` then `agent.rate_limits.insert(kind, window)`. | No change if A sends the same meta key. | 0 | Child-agent usage path (`:495`, `:611`) does not write `rate_limits` (inferred from read). |
| `crates/codegen/xai-grok-pager/src/app/agent_view/mod.rs:951` | `AgentView.rate_limits: BTreeMap<String, RateWindow>`. In memory only. | Footer reads it. | 0 | Survival across pager restart: nothing persists it. A session-start `get_usage` (A1) would refill it after restart. Persisting it would add about 20 lines. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/session.rs:217` | `rate_limits: Default::default()` on session creation. | None. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/side_panel.rs:52-53` | Copies `rate_limits` and `five_hour_pct` into `Info` on each draw. | None. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:113` and `:125` | `SidePanel.five_hour_pct`, default `None`. | Keep as fallback only. | 0 | Source file `~/.cache/claude-status` is written by the TUI statusline and can be stale (scout: 9% file vs 12% live). |
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:229-236` | `poll()` returns early when the panel is closed (`:230`); otherwise reads `~/.cache/claude-status` for 5h (`:234`). | Footer needs 5h with the panel closed, so the file read must move above the `open` gate or be dropped once A ships. | ~5 | None. |
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:511-530` | Renders "5h usage" (file fallback allowed) and "7d usage" (no file fallback, `:512`). Stale windows are hidden by the `live` filter (`:517-521`). | Footer reuses the same live filter. | ~10 | None. |

## C. Footer

Agent screen bottom row: `ShortcutsBar` (`views/shortcuts_bar.rs`), built in `app/agent_view/render.rs`. There is no usage segment and no abbreviation logic today.

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs:594-601` | `current_shortcut_hints` returns the hints for the surface that owns keys (`ShortcutsBarContent`). | No change; the usage segment is separate from hints. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs:692-` and `crates/codegen/xai-grok-pager/src/views/agent.rs:1150` (`build_hints`) | Prompt-focused hints in order: send or queue (`views/agent.rs:1269`), newline (`:1274-1276`), mode on Root surface (`:1289`), then registry hints filtered by `PromptFocused`, `AgentScreen`, `Always` (`:1291`). | Remove or shorten the hints Sab names. | ~10 | Exact set on screen is inferred from code, not seen. Confirm with a `TestBackend` render. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs:3888-3900` | Pane branch: `ShortcutsBar::new(&hints).compact(5, help_hint)`. Shows 5 unpinned hints plus `?` help. | Change the cap, or reserve a slot for usage. | ~15 | None. |
| `crates/codegen/xai-grok-pager/src/views/shortcuts_bar.rs:197-308` | `render` paints left to right and stops at the first hint that does not fit (`break` on `x + width > area`). No truncation, no ellipsis. At half width the tail hints vanish silently. | A usage segment placed last is dropped first. Place it before hints, or add width-aware abbreviation. | ~20 | How narrow the split-screen chat column gets in practice (`side_panel.rs` clamps with `MIN_CHAT`, not measured here). |
| `crates/codegen/xai-grok-pager/src/views/shortcuts_bar.rs:190-192` | `with_right_text`: right-aligned text slot. No caller in `crates/codegen/xai-grok-pager/src` (only the definition matched). | Usage segment can use this slot. It paints only if there is room after the hints (`rx > x + 1`). | ~5 | Hints win over right text when space is tight. |
| `crates/codegen/xai-grok-pager/src/views/shortcuts_bar.rs:338-365` | `compute_effective_hints`: pinned hints always shown; the rest fill `max_visible`; help appended. | None. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/actions/defaults.rs:816-831` (`Quit`, priority 10, `Always`) | Registry hint, reaches the compact list under prompt focus (inferred). | Drop `hint_priority` to remove it from the footer. | 1-2 | None. |
| `crates/codegen/xai-grok-pager/src/actions/defaults.rs:839-846` (`CommandPalette`, Ctrl+P, alt `?`, priority 5, `AgentScreen`) | Registry hint, reaches the compact list (inferred). | Same as above. | 1-2 | None. |
| `crates/codegen/xai-grok-pager/src/actions/defaults.rs:721-729` (`VoiceToggle`, "mic", priority 11, `Always`) | Registry hint, lowest priority; shown only if the cap leaves room (inferred). | Same as above. | 1-2 | Whether voice is enabled changes this. |
| `crates/codegen/xai-grok-pager/src/actions/defaults.rs:854-` (`ShortcutsHelp`, `?`) and `:893-897` (`OpenSettings`, Ctrl+, alt) | `?` is the help hint appended in compact mode (`render.rs:3900`). `OpenSettings` carries `Ctrl+,` as alt key. | Ctrl+, is not a footer hint today; it is in the registry only (inferred from the priority list). | 0 | Which four hints Sab sees: the mapping above is inferred and must be confirmed on a real render. |
| `crates/codegen/xai-grok-pager/src/actions/mod.rs:425-435` | `hints(contexts)` filters by `hint_priority` and context, sorts by priority. | None. | 0 | None. |

## D. Codex

No code in this repo reads Codex rate limits today. The rollout converter reads transcripts only.

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `crates/codegen/xai-grok-pager/src/acp/codex_feed.rs:20` | `convert(rollout, session_id)` turns a rollout into session events. No `rate_limits` read. Searches for `used_percent`, `window_minutes`, `rate_limits` in `pager/src/acp`, `pi-grok-adapter/src` and `xai-grok-traces/src` found nothing. | New reader: find newest rollout, read its last `token_count` event, map `primary` (300 min) to 5h and `secondary` (10080 min) to 7d. | ~60-90 new | Which process runs `codex` under grok, and whether it feeds this converter. Not located. |
| `~/.codex/sessions/2026/11/03/rollout-2026-11-03T01-52-57-01a1a0d3-7fef-7b40-93b6-ec0ec5138c63.jsonl` | `event_msg` with `payload.type` `token_count` carries `rate_limits` (48 events in this file). Example: `primary` `used_percent` 0.0, `window_minutes` 300, `resets_at` 1793717663; `secondary` `used_percent` 50.0, `window_minutes` 10080. | Source for Codex 5h and 7d values. | 0 | Evidence is one file. Other days not checked. `resets_at` is epoch seconds. |
| `crates/codegen/xai-grok-traces/src/roots.rs:70` | Codex home: `CODEX_HOME` or `~/.codex`. | Reuse to find the sessions directory. | 0-3 | None. |

## E. OpenCode Go and Grok

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| OpenCode, disk | `which opencode` returns nothing. `~/.config/opencode`, `~/.local/share/opencode` and `~/.opencode` do not exist. `~/.grok/traces/proto-c-summaries.json` matches the name "opencode" (name match only, not opened). | None found. | 0 | Whether OpenCode Go has a usage source on this Mac. None found. |
| `crates/codegen/xai-grok-markdown/src/open_code_highlighter.rs:2` | Name match only: a syntax highlighter, not usage. | None. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/views/credit_bar.rs:13-19` | `CreditBalance.usage_pct` (allowance percent, 0-100) and `effective_usage_pct` (including on-demand). | Grok's footer value would be a credit allowance, not a 5h window. Different metric; do not label it "5h". | 0 to show as-is | Whether Sab wants a Grok credit figure in the footer at all. |
| `crates/codegen/xai-grok-pager/src/app/effects/helpers.rs:2217-2266` | `credit_balance_from_config` builds `CreditBalance`, with `usage_pct` from `credit_usage_percent`. | None. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs:2986` | Existing use of `credit_balance` for a warning. | None. | 0 | None. |

## F. Tests and snapshots

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:578-611` | Rate-limit parse tests (`:578` one window and reset formats; `:608` rejects milliseconds and non-finite resets). | Unchanged if parse stays. | 0 | None. |
| `crates/codegen/xai-grok-pager/src/views/side_panel.rs:621-646` | Statusline five-hour file tests (`:621` newest snapshot wins). | Remove or keep, depending on whether the file read stays (B). | -10 to 0 | None. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/side_panel_tests.rs:29` | Fixture sets `five_hour_pct = Some(22)`. Tests run from `:99` to `:160`. | Update if the panel's 5h row changes. | ~5 | None. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/snapshots/xai_grok_pager__app__agent_view__side_panel__tests__focused_info_page.snap` (also `open_unfocused_lets_the_chat_type`, `narrow_terminal_focused_panel_takes_the_width`, `zoomed_swaps_the_sizes`) | Snapshots contain "5h usage" rows (matched by search). | Regenerate if the panel's 5h or 7d rows change. | 1-2 lines per snapshot | None. |
| `crates/codegen/xai-grok-pager/src/views/shortcuts_bar.rs:373-520` | Inline footer tests: helper `h` at `:373`, compact tests `:385-476`, `render_hints` `:495`, label-dimness test `:517`. | Update if hint set or order changes. | ~10 | None. |
| `crates/codegen/xai-grok-pager/src/app/agent_view/key_owner_tests.rs:127` | `hint_labels` helper for footer label assertions (permission tests from `:226`). | Update if prompt-pane hints change. | ~5-10 | None. |
| `crates/codegen/xai-grok-pager/tests/settings_e2e.rs:1874` and `:3721` | Footer text assertions in the settings e2e. | Check only if the settings footer changes. | 0-5 | None. |

No agent-screen footer `.snap` file was found under `crates/codegen/xai-grok-pager` (search for `│` in snapshots matched only side-panel and trace-view snapshots). The "footer-snapshot fails" in the notes may live elsewhere. Not located.

## Slices

Each slice is independently testable. Slice 1 needs no adapter; slices 2 and 3 do not depend on each other.

1. **Footer segment and hint trim (pager only).** Read the 5h/7d windows from `AgentView.rate_limits` (B2, B5), keep the statusline fallback (B7). Move the file read above the `open` gate (B8). Drop the four named hints from prompt-focused hints (C2, C7-C9). Use `with_right_text` or a leading segment (C5). Touches: `views/side_panel.rs:229-236`, `app/agent_view/render.rs:3888-3900`, `views/shortcuts_bar.rs:190-308`, `actions/defaults.rs`, tests from F. Needs adapter: no. Limit: 5h comes from the stale statusline file until slice 2 lands.
2. **Fresh Claude usage via adapter.** Call `usage_EXPERIMENTAL...` at `newSession` (A1) and after each turn (A2), emit `usage_update` with `_meta["_claude/rateLimit"]` per window, convert shape (A5). Touches: `dist/acp-agent.js:959-972` and `:3484-3495`, `dist/usage-markdown.js`, new patch `~/.dsh/patches/claude-agent-acp/0003-*.patch` (A7). Needs adapter: yes. Pager side: none if the shape matches B3.
3. **Codex 5h and 7d (pager only).** New reader for the newest rollout's last `token_count` event (D1, D2, D3). Feed into `Info` (B1) and the footer (C). Touches: new file under `crates/codegen/xai-grok-pager/src/acp/`, `views/side_panel.rs`, footer render. Needs adapter: no. Unknown: which host path runs Codex under grok.
4. **Grok and OpenCode Go: no slice.** OpenCode has no source found (E1). Grok's `usage_pct` is a credit allowance, not a 5h window (E3); add only if Sab asks for it.
5. **Tests per slice.** Use F rows that match the slice. Render footer and panel with `TestBackend` at full and half width, in every shipped theme (`GROK_THEME=<name>`), as AGENTS.md requires for colour changes.

Open decisions for Sab: (a) where the adapter patch is kept, since `~/.dsh` is outside the repo (A7); (b) whether Grok credit % goes in the footer at all (E3); (c) the four hints to drop, once the on-screen set is confirmed (C2).
