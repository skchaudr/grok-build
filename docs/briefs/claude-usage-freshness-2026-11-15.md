# Investigate: how `grok claude` gets the freshest Claude usage numbers

Read-only investigation. You change no source code. Report by 10 minutes after you start; if you are not done, write what you have and stop.

## Sab's words (2026-11-15), the question to settle

> I ran it [cstat], got a false stale number, ran claude real quick, and it got immediately updated to something accurate! Literally the precondition I believe is just for claude to be running. MAYBE that means claude code interactively, OR maybe it just means at all, in which case, grok-claude ought to be able to the freshest #s

He wants the 5h (and 7d) percentage in the `grok claude` footer and for it to be reliable.

## Already established (do not redo; verify only if something contradicts it)

- `cstat` on sab-air (`ssh sab-mini@sab-air`, `~/.local/bin/cstat`, Python) reads `~/.cache/claude-status/*.json`. Those files are written by `~/.claude/statusline-command.sh`, the Claude Code statusline command, from the `rate_limits` object in its stdin.
- Starting interactive `claude` on sab-air with no turn sent wrote a fresh file at once (`07c1c6ae…json`, 9% / 51%).
- A `grok claude` session (pager spawns `claude-agent-acp`, which drives the Claude Agent SDK) that ran for hours on the Mini wrote no file in that folder.
- The adapter `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js` forwards the SDK's `rate_limit_event` as `usage_update` `_meta["_claude/rateLimit"]`, one window per event, only after a turn has reported usage. The pager reads it at `crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs` (the `UsageUpdate` branch) and falls back to the statusline file in `views/side_panel.rs` (`claude_five_hour_pct`).
- Read `docs/HARNESS-FACTS.md` first for anything else already known.

## Questions

1. How does interactive Claude Code have 5h/7d numbers at startup before any turn? Find where they come from (a startup request, response headers of a startup call, a cached file) in the installed Claude Code and SDK (`~/.local/share/claude-agent-acp/0.76.0/node_modules/@anthropic-ai/claude-agent-sdk`, and the `claude` binary/CLI it runs).
2. Can a session driven through the SDK / `claude-agent-acp` get the same numbers with no turn sent: does the SDK emit `rate_limit_event` at init, expose a call for it, or run the statusline command at all? If it can and the adapter drops it, say exactly where.
3. `ccusage` on sab-air (`/opt/homebrew/bin/ccusage`): what does it read, and does it give a 5h or 7d percentage or reset time that matches `cstat`? Run it; it is read-only.
4. Conclude: the smallest change that gives `grok claude` fresh 5h and 7d numbers at startup and between turns, with no interactive Claude Code running. Name the files it would touch and what stays unknown.

## Rules

- No source edits, no cargo, no installs. Never kill by name (no pkill/killall). Never tmux.
- You may start `claude-agent-acp` or an SDK session to watch its startup messages, but never send it a prompt: no model turns.
- Every fact gets a file:line or the command output it came from. Mark anything inferred as inferred.

## Output

Write `docs/SCOUT-claude-usage-freshness-2026-11-15.md`: the answer to question 4 first in three lines or fewer, then questions 1-3 with evidence. Add what you learned to `docs/HARNESS-FACTS.md` under a new heading. Commit only those two files by path and push to main, message ending exactly:

Co-authored-by: claude-code sonnet-5.5
Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>

Then print the commit SHA and the three-line answer, and stop.
