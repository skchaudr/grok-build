# Haiku scout: map the areas for "5h usage in the footer"

Read-only. You change no source code. Report by 10 minutes after you start; if not done, write what you have and stop.

## Sab's words (the intent; nothing below overrides them)

2026-11-14: "Right now, the footer of grok claude: 4 useless keybindings that `?` and `Ctrl-,` easily overrule in terms of value. I think that the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok, fuck it. I think we could fit, fit them all if we. We're able to abbreviate or use symbols or whatever. Like right now I'm literally, split screen."

2026-11-15: "Send out a Haiku scout to map out the likely areas -have it return a structured answer"

## Already known (read, do not redo)

- `docs/SCOUT-claude-usage-freshness-2026-11-15.md`: the Claude SDK has a no-turn usage call (`get_usage`); the adapter `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js` does not call it; rate-limit events are dropped until a turn reports usage (around line 4285).
- `docs/HARNESS-FACTS.md` (Claude plan usage section, Codex sections).
- Pager crate: `crates/codegen/xai-grok-pager/src`. Rate-limit handling is in `app/acp_handler/mod.rs` (`UsageUpdate` branch) and `views/side_panel.rs` (`claude_five_hour_pct`, rate-limit meta parsing). The footer hint row comes from `views/shortcuts_bar.rs` and `app/agent_view/render.rs`; hints are declared in `actions/defaults.rs` (`hint_priority`, `hint_key_display`).
- Codex session rollout files (`~/.codex/sessions/YYYY/MM/DD/*.jsonl`) carry `rate_limits` with a 300-minute window, percent and reset time.

## Map these areas

A. **Adapter**: exactly where a `get_usage` call would go (session start, turn end), how its result would be sent to the pager, and how `grok-fork` finds and launches the adapter (so a patch survives an adapter upgrade: is there a vendored copy, a patch script, a pinned version?).
B. **Pager state**: where 5h/7d values live today, which struct, who writes and reads them, and what must change to hold both windows with reset times and survive a restart.
C. **Footer**: which code builds the bottom hint row for the agent screen, which four hints show under `grok claude` today, how width is handled when the screen is narrow (half width), and where a usage segment could be placed.
D. **Codex**: the function (if any) that reads Codex rollouts in this repo, and where a Codex 5h figure would be read for the same footer.
E. **OpenCode Go and Grok**: any usage/limit source in this repo or on disk on this Mac. "None found" with where you looked is a valid answer.
F. **Tests**: tests and snapshot files that render the footer or the side-panel usage lines and would change.

## Rules

- No source edits, no cargo, no installs, no model turns. Never kill by name (no pkill/killall). Never tmux.
- Search with `rg` scoped to a folder; recursive `grep` and searches rooted at `/` are blocked by a hook.
- Every row cites file:line that you opened. Mark anything inferred as inferred.

## Output (structured)

Write `docs/SCOUT-usage-footer-areas-2026-11-15.md` with exactly one table per area A-F, columns:

`file:line | what is there now | what would change | rough lines changed | unknowns`

Then a last section `## Slices`: the change cut into the smallest independent pieces, each with the files it touches and whether it needs the adapter.

Commit only that file by path and push to main (`git pull --rebase` first if the push is rejected), message ending exactly:

Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Print the commit SHA and stop.
