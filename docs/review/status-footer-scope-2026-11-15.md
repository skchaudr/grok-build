# Review: footer shows status, not key hints (size and scope)

Reviewer: Sonnet 5.5, read-only. Briefs and scouts: `docs/briefs/review-status-footer-scope-2026-11-15.md`, `docs/SCOUT-cache-warmth-2026-11-15.md`, `docs/SCOUT-footer-hints-removal-2026-11-15.md`, `docs/SCOUT-opencode-codex-usage-2026-11-15.md`. Pager paths are under `crates/codegen/xai-grok-pager/src`; adapter paths are `acp-agent.js` in `~/.local/share/claude-agent-acp/0.76.0/dist/`.

**Verdict: QUEUE slices 1, 2 and 3 (hint trim, status row, Claude usage + cache age); RESCOPE cache countdown and Codex; DROP OpenCode.**

## Premise (A0)

The three reports serve Sab's words: both footers lose the default-key hints, the row carries status, half width is considered. Two substitutions I found, each decided for Sab by a scout:

- **A0-1. Cache scout assumes a 1h TTL and says to label it "assumed".** Sab asked for a *warm* signal. A countdown built on a guessed TTL says "warm" when the cache is cold if the SDK uses 5 minutes. That is a wrong signal, not a labelled one. See F1.
- **A0-2. Hint scout leaves `Space:prompt` and `q/Esc:back` in the removable pile or misses them.** Sab did not list them; they are the only cues for leaving a mode. See F3.

## Findings, most severe first

**F1. TTL is readable, not only assumable; "pager alone" is true only for age.**
- Right: counts reach the pager already. Adapter `acp-agent.js:3432-3437` sums cache read/write per turn; `:6530-6537` puts them in the PromptResponse `usage`; the pager drops them at `app/dispatch/prompt.rs:1832-1836` (`response.usage.filter(|_| !usage_is_tally)`).
- Wrong: nothing on the wire says which TTL the SDK used. The Claude binary 2.1.323 holds the env switches `ENABLE_PROMPT_CACHING_1H` and `FORCE_PROMPT_CACHING_5M` (checked with `strings`) and the field names `ephemeral_1h_input_tokens` / `extended_cache_ttl`. The adapter never forwards `cache_creation.ephemeral_*` (`rg ephemeral` over `acp-agent.js`: no match). That split is where the real TTL can be read per turn, and it needs one adapter field.
- So: age since the last response is pager-only and exact. "Warm / N minutes left" needs the adapter field, or Sab accepts a 5m floor shown as age, not warm/cold.

**F2. `last_response_at` already half exists, with a wrong trigger.** `last_active_at` (`app/agent_view/mod.rs:1022`) is set in `mark_turn_finished` (`app/agent_view/session.rs:704`) and at session creation (`session.rs:247`). `mark_turn_finished` also runs on `TurnEnd::Aborted` (`dispatch/session/lifecycle.rs:1748`, `load.rs:1397`, `fork.rs:443`), where no API response happened, so it is not a cache clock. Stamp a new field beside the usage read at `prompt.rs:1832` instead, only when `cached_read_tokens > 0 || cached_write_tokens > 0`. The scout's cite `turn_completion.rs:529` is `app/turn_completion.rs:529`, not under `agent_view/`.

**F3. Keep-list needs state, not labels.** Hints that must stay: `CancelTurn`, `Interject`, `send to bg`, queue `send now`/`queue`, `Tab/Right:accept suggestion`, the cards and viewers (scout list B), plus two the scout missed or mis-filed:
- `q/Esc:back` in a child takeover (`views/agent.rs:1350-1352`): the only exit cue.
- `Space:prompt` focus hint (`agent.rs:1356-1368`): the only cue back to typing.
- `x:kill` (`agent.rs:1467-1469`) only exists while a task is selected and is destructive; keep.
Matching on label strings at `render.rs:661-668` (scout's plan) is fragile (`"copy"`, `"cancel"` appear in both groups). Better: one `state: bool` on `HintItem` (`views/shortcuts_bar.rs:19-34`), set on the push sites above, then `retain(|h| h.state)` at `render.rs:661-668`. About 25 lines across `agent.rs` and `shortcuts_bar.rs`, not the scout's 10. Delete: `y:copy`, `Enter:open`, `j/k:nav`, `l/h:turn`, `g/G`, `thinking`, `expand/collapse`, `trace`, `Enter:send`, `newline`, `lines`, `Shift+Tab:mode`, and the `ShortcutsHelp` hint (`render.rs:3889-3902`).

**F4. A status row already exists in the pager.** `views/status_line/segments.rs:68` `compose_builtin` builds `StatusSegment`s from `StatusLineItem` (`xai-grok-status-line/src/config.rs:325`: Cwd, Model, Context, Cost, TurnTimer, SessionName), drawn as its own row (`app/status_line.rs:37` `draws_a_row`). The scouts planned a second status mechanism inside the shortcuts bar. Adding items there changes a shared crate's config schema and puts status on a separate row, not where Sab pointed ("that footer right there"). I keep the shortcuts-bar approach but Sab should rule (Q1).

**F5. Status row: scout cite is right, plan needs two more points.** `views/shortcuts_bar.rs:190` `with_right_text` has no caller (`rg`: definition only; `render.rs:3884-3903` passes `with_pending` and `compact` only). Hints paint from `area.x` and the loop breaks at the first hint that does not fit (`:259-292`), so a status appended after hints is lost at half width. Plan: delete `right_text` (`:143,169,190-193,295-306`, dead), add `with_status(&str)` painted first from the left with its own width check; hints use the remaining width. Also: the pending "press again" branch (`:229-249`) returns early and hides status, fine. Permission/plan/question cards go through `ShortcutsBarContent::Surface` (`render.rs:3880-3883`); do not paint status there.

**F6. Codex: real source, but account-level and mostly stale.** Pager has `acp/codex_feed.rs` for the traces feed and no `rate_limits` reader (`rg rate_limits`: only `side_panel.rs` Claude code). The rollout `rate_limits` record (`payload.rate_limits.primary/secondary.used_percent`, `resets_at` epoch) is written by Codex sessions of any host, so it describes the account, not the `grok claude` session. Newest figures on sab-air are from 2026-11-11 and 11-10; their `resets_at` is in the past (now 2026-11-15T14:12Z), so the percentages are meaningless today. It is sound only as: scan newest rollouts, show only if `resets_at > now`, otherwise hide. That hides it most days.

**F7. OpenCode has nothing to show.** Spot-check: newest `message` row 2026-10-20 12:24 UTC, 700 messages; cost 0.0; no percentage in db, config, auth or logs. Tokens-only from three weeks ago is not a status.

**F8. Claude usage and 7d: sized by the earlier review.** `views/side_panel.rs:386-405` already reads the statusline 5h file (stale when no turn: 9% vs live 12%, `docs/review/usage-footer-scope-2026-11-15.md` F6). For the footer, reuse `rate_limits` from the live feed in `side_panel.rs:58` and fall back to the file only when empty, as `side_panel.rs:59` does.

**F9. Snapshot tests change by design.** Five snapshots expected to re-record (scout E); three of them already depend on terminal detection (`Ctrl+.` vs `Ctrl+x`) and stop depending on it once the help hint is gone. The 126 hits in `render.rs` tests and the key-owner tests were not read by me either.

**F10. Unrelated, caught while checking:** my `pgrep -lf codex` on sab-air printed a pi process environment (several API keys) into my own tool output. Nothing from it is copied anywhere. Those values sit in a long-lived process env on sab-air and are Sab's to rotate or not.

## Slices, build order

1. **Hint trim** (`app/agent_view/render.rs:661-668`, `3889-3902`; `views/agent.rs` push sites; `views/shortcuts_bar.rs` `HintItem`). QUEUE, rescoped per F3: `state` flag, not label match. About 40 lines with tests. Re-record 5 snapshots.
2. **Status row slot** (`views/shortcuts_bar.rs`, `render.rs:3884-3903`). QUEUE per F5, delete `right_text`. About 40 lines + half-width test.
3. **Claude 5h + 7d in the slot** (reads `side_panel.rs` rate state). QUEUE, about 20 lines on top of 2. Same scope as the earlier "slice 1".
4. **Cache age** ("cache 12m" since last cached response, hidden before any response). QUEUE, about 25 lines (`prompt.rs`, `agent_view/mod.rs`, `session.rs`) per F2. Shows age only, no warm/cold word.
5. **Cache warm/cold + minutes left.** RESCOPE: adapter must forward `cache_creation.ephemeral_1h_input_tokens` / `ephemeral_5m_input_tokens` (about 8 lines in `acp-agent.js` near `:3437` and `:6530`), and Sab decides where the adapter patch lives (same open call as fresh usage).
6. **Codex 5h/7d.** RESCOPE per F6: account-level, hidden unless `resets_at > now`. About 60 lines (reader off the UI thread, newest-file tail scan). Low value on days Codex is unused.
7. **OpenCode.** DROP per F7.
8. **Grok credit usage.** DROP for this batch: `usage_pct` (`views/usage_modal.rs:816`) is a credit allowance, not a window.

## Questions for Sab (pick first)

1. Status row in the shortcuts bar (my pick) or as new items in the existing status-line row (`xai-grok-status-line`)? Pick: shortcuts bar, because it is the row you pointed at.
2. Show cache as age only ("cache 12m") now, with warm/cold after an adapter field? Pick: yes.
3. Keep `Space:prompt` and `q/Esc:back` in the footer (not in your list)? Pick: keep.
4. Does history search and scrollback search keep their key hints (`agent.rs:1235-1251`, `1323-1346`)? Pick: keep, they are modes.
5. Should the help hint (`Ctrl+.`/`Ctrl+x`) go entirely? Pick: yes, the palette (`?`, `Ctrl+p`) covers it.

## Row-by-row check

Cache-warmth report:
| cite | verdict |
|---|---|
| `acp-agent.js:3432-3437` accumulate | right |
| `:3878-3882` lastAssistantUsage | right (`:3876-3881`) |
| `:3889-3894` usage_update used/size only | right |
| `:6530-6537`, `:6547` sessionUsage | right |
| `:6563-6588` quotaTokenCount | right |
| rg `cache_control/ephemeral` none | right (re-ran for `ephemeral`) |
| `prompt.rs:1832-1836` Claude filter | right; `usage_is_tally` at `:1711` right |
| `agent-client-protocol-schema-0.11.4 agent.rs:3130` | unverifiable here (cargo registry not opened) |
| `agent_view/mod.rs:1002` `turn_started_at` | right (`:1002`) |
| `session.rs:697-699` mark_turn_finished | right |
| `turn_completion.rs:529` (as `agent_view/`) | wrong path: `app/turn_completion.rs:529` |
| `last_active_at` "not checked" | now checked: wrong trigger, F2 |
| statusline table (`~/.cache/claude-status`) | unverifiable (not re-read) |
| TTL "unknown" | right; F1 shows where it is readable |

Hint-removal areas A-D:
| cite | verdict |
|---|---|
| `render.rs:599-602` shortcuts_bar_content | right |
| `:661-668` Pane + `prompt` retain | right (`:661-667`) |
| `:683-686` casual comment | right (`:682-686`) |
| `:3876-3880` modal returns before bar | right |
| `:3889-3902` help hint + `compact(5)` | right |
| `agent.rs:1150`, `1253-1290` typing | right |
| `agent.rs:1291-1303` loop adds nothing | right: hint_priority defs 85,111,163,298,328 are `ScrollbackFocused`; 729/831/846 skipped |
| `agent.rs:1347-1471` scrollback | right; Space:prompt and `q/Esc:back` mis-filed or missed (F3) |
| `agent.rs:1473-1492` cancel/interject/bg | right |
| `shortcuts_bar.rs:190-193`, `229-249`, `257`, `259-292`, `295-306` | right |
| `with_right_text` never called | right (re-ran `rg`) |
| `actions/defaults.rs:845-862` palette `?`, help Ctrl+./Ctrl+x | right; `?` is palette alt key at `:845` |
| E snapshot rows | unverifiable (snapshots not opened) |

sab-air report spot-checks (3 commands, read-only): `opencode --version` = 1.19.4 (right); `opencode.db` newest message 2026-10-20 12:24, 700 rows (right, matches 583+117); newest 11-05 rollout has 67 `rate_limits` lines (right).
