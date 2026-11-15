# Review: size and scope of "5h usage in the footer" (2026-11-15)

**Verdict: Slice 1 QUEUE (footer segment only, hint trim removed). Slice 2 RESCOPE. Slice 3 RESCOPE. Slice 4 DROP.**

Reviewer: Sonnet 5.5, read-only. Input: `docs/SCOUT-usage-footer-areas-2026-11-15.md`. No source edited, no cargo run, no screen rendered.

## Findings, most severe first

**A0. The hint trim is not in Sab's words.** His words (2026-11-14) ask for the 5h percentage at the footer, for Claude, Codex, OpenCode Go and Grok, abbreviated to fit. They never say to drop any hint. "Replacing four keybinding hints" is an agent's phrase in notes. The scout's slice 1 and open decision (c) build on it. Removed from the queued scope until Sab names hints to drop.

**F1. Scout rows C7-C9 are wrong for the prompt footer.** `views/agent.rs:1291-1300` skips `Quit`, `CommandPalette`, `VoiceToggle` (and `SendPrompt`, `CycleThinkingLevel`, `EnableVoiceMode`) with `continue` when building prompt-focused hints. Lowering their `hint_priority` in `actions/defaults.rs` changes nothing there. Only 8 actions have a `hint_priority` at all (`nav`, `turn`, `top/btm`, `fold`, `expand/collapse thinking`, `mic`, `quit`, `commands`), and the first five are `ScrollbackFocused`. Under prompt focus the hints are send/queue, newline, `BackTab:mode` (`views/agent.rs:1269-1289`), plus `?` appended by `compact(5, help)` (`render.rs:3897`). Which four hints Sab sees is not established; needs a `TestBackend` render or a screenshot.

**F2. The footer is not starved of room.** With that short hint list, the usage text fits at full width through the unused `with_right_text` slot (`shortcuts_bar.rs:190-192`, painted at `:292-303`). The slot is only drawn when `rx > x + 1`, so hints win at narrow width. At half width the right text vanishes silently, which fails "fits at half width". Needs either a shorter form (`5h 12%`) chosen by remaining width, or placement before the hints. This is the real work in slice 1 (~20-30 lines, not ~35 across five files).

**F3. Poll gate hides the file fallback with the panel closed.** `views/side_panel.rs:229-231` returns before the `~/.cache/claude-status` read when the panel is closed, so a footer that relies on `Info.five_hour_pct` shows nothing until F3 has been opened. Confirmed; scout's B8 is right. Footer needs its own read or the read moved above the gate.

**F4. Pager-only delivers something visible, but may be stale or empty.** Live windows arrive only via `rate_limit_event`, which the adapter drops until a turn has reported usage (`acp-agent.js:4285-4297`). Before the first turn the footer shows the statusline-file value, which the freshness scout measured as 9% against a live 12%. The file is written by interactive Claude Code's statusline; the scout saw no file from a `grok claude` session on the Mini. So the segment is useful on sab-air only to the extent that file exists there. Fine for a first slice, but report it as "may be stale" when shipped.

**F5. Scout correction in the header is right.** `usage_EXPERIMENTAL...` is called at `acp-agent.js:91` for `/usage` only (`:1422-1425`, `:1982`), not at session start or turn end. The earlier HARNESS-FACTS line is wrong.

**F6. Adapter slice is not required for slice 1 and is the riskiest part.** Three hazards:
- `newSession` (`acp-agent.js:959-972`) has no `lastAssistantTotalUsage`; a startup `usage_update` must carry `used`/`size`, and the pager's `UsageUpdate` branch (`acp_handler/mod.rs:254-268`) calls `apply_context_used(used, size)` with the stale-zero guard only when `context_state.used > 0`. A startup update with `used = 0` is applied on a fresh session; harmless then. A turn-end second update carrying a wrong `used` would corrupt the context meter. The adapter should send `_meta` only on a separate update or reuse the turn's own `used`.
- Shape mismatch: `get_usage` gives `utilization` 0-100 and ISO `resets_at` (`usage-markdown.js`), the pager parser wants 0..1 and epoch `resetsAt` (`views/side_panel.rs:76-89`). Also each `_claude/rateLimit` event carries one window, so two meta keys or two updates are needed.
- Patch lives in `~/.dsh/patches/claude-agent-acp/`, outside this repo, applied after `0001` and `0002`. Conflict with `0002` untested. Sab must say where it is kept.

**F7. Codex slice is under-mapped.** The data source is real (rollout `token_count` events carry `rate_limits.primary/secondary`, confirmed in the 11-03 file) but nothing shows which process runs Codex under grok or which session's rollout to read. "Newest rollout" would show another session's usage when two Codex sessions exist. `codex_feed::convert` is used only by the standalone traces path (`app/dispatch/standalone_traces.rs:101`), not by a Codex host. Needs a scout on the host path before sizing.

**F8. Slice 4 is right to drop.** `opencode` is not installed (`which opencode`: not found); no usage source exists. Grok's `usage_pct` (`credit_bar.rs:13-19`) is a credit allowance, not a 5h window; it should not be labelled 5h. Sab said "even Grok, fuck it", so he may still want the credit figure; that is a separate ask with its own label.

**F9. Test list is over-broad.** The scout lists `tests/settings_e2e.rs`, `key_owner_tests.rs`, `side_panel_tests.rs` snapshots. With no hint trim, only the shortcuts_bar inline tests and one new footer render test (full and half width, every theme) are needed. The 3 terminal-dependent footer snapshots in the queue's "Flagged" section are separate and will still be touched.

## Size and scope, from the code

Slice 1 as queued: `views/shortcuts_bar.rs` (right-text width fallback, ~15 lines), `app/agent_view/render.rs` (compose the text from `agent.rate_limits` with the same `live` filter as `views/side_panel.rs:517-521`, ~20 lines), `views/side_panel.rs` (move the file read above the `open` gate or into the footer, ~5 lines), one or two render tests. About 40-60 lines, pager only, no adapter.

## Row-by-row check

Right = opened and matches. Wrong = opened and contradicts. Unverifiable = not checkable without running or outside what I opened.

| Area | Row | Result |
|---|---|---|
| A1 | `acp-agent.js:959-972` | right |
| A2 | `:3484-3495` | right (condition and fields match) |
| A3 | `:4285-4297` | right |
| A4 | `:82-107`, `:91`, `:1422-1425`, `:1982` | right |
| A5 | `usage-markdown.js:5,31-40,58` | right on shape (checked `parseUsageResponse` use; line numbers not individually confirmed) |
| A6 | `~/.local/bin/claude-agent-acp:1-2` | right |
| A7 | `claude-agent-acp-update:24-29` | right in substance; patch survival not exercised |
| A8 | patch dir contents | right (0001, 0002 present) |
| A9 | `scripts/hub/gb:51-56` | unverifiable (not opened) |
| B1 | `side_panel.rs:48-61` | right |
| B2 | `:69-72` | right |
| B3 | `:76-89` | right |
| B4 | `acp_handler/mod.rs:254-262` | right; child-agent paths `:495`, `:611` unverifiable |
| B5 | `agent_view/mod.rs:951` | right |
| B6 | `session.rs:217` | right |
| B7 | `agent_view/side_panel.rs:52-53` | right |
| B8 | `side_panel.rs:113,125` | right |
| B9 | `:229-236` | right |
| B10 | `:511-530` | right |
| C1 | `render.rs:594-601` | right |
| C2 | `render.rs:692-`, `agent.rs:1150`, hint order | right on order; "registry hints filtered ..." is incomplete, see F1 |
| C3 | `render.rs:3888-3900` | right |
| C4 | `shortcuts_bar.rs:197-308` | right (break on overflow, no ellipsis) |
| C5 | `:190-192` | right; no caller confirmed by `rg` |
| C6 | `:338-365` | unverifiable (not opened) |
| C7 | `defaults.rs:816-831` Quit | right on the def; "reaches the compact list" wrong (F1) |
| C8 | `:839-846` CommandPalette | right on the def; wrong on reach (F1) |
| C9 | `:721-729` VoiceToggle | right on the def; wrong on reach (F1) |
| C10 | `:854-`, `:893-897` | right on defs; claim about Ctrl+, hint unverified |
| C11 | `actions/mod.rs:425-435` | right |
| D1 | `codex_feed.rs:20` | right |
| D2 | rollout file | right (file exists); one file only |
| D3 | `roots.rs:70` | right |
| E1 | opencode on disk | right (`which` empty) |
| E2 | `open_code_highlighter.rs:2` | unverifiable (not opened) |
| E3 | `credit_bar.rs:13-19` | right |
| E4 | `helpers.rs:2217-2266` | unverifiable (not opened) |
| E5 | `render.rs:2986` | right (credit_balance warning at 2986) |
| F rows | tests and snapshots | unverifiable (not opened); scope trimmed per F9 |
