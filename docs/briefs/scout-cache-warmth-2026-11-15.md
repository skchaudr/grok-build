# Haiku scout: where a cache-warm signal can come from under `grok claude`

Read-only. You change no source code. Report by 10 minutes after you start; if not done, write what you have and stop.

## Sab's words (the intent; nothing below overrides them)

2026-11-15: "I already have a command palette, what I do not have is a warm cache signal. Why not include cache, claude usage, code, opencode"

## Already known (read, do not redo)

- Interactive Claude Code passes its statusline a `prompt_cache` object (`caching_observed`, `warm`, `expires_at`). `~/.claude/statusline-command.sh` (line 16) writes it to `~/.cache/claude-status/*.json`; that is the "cache warm 59m" text.
- `grok claude` runs `claude-agent-acp` (adapter at `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js`), not interactive Claude Code, so the statusline script does not run for it.
- Pager crate: `crates/codegen/xai-grok-pager/src`. Usage updates arrive in `app/acp_handler/mod.rs` (`UsageUpdate` branch); the file fallback is read in `views/side_panel.rs`.
- `docs/SCOUT-claude-usage-freshness-2026-11-15.md` and `docs/HARNESS-FACTS.md`.

## Answer these

A. **Adapter**: does any message the adapter receives from the Claude SDK carry cache data (per-turn `usage.cache_read_input_tokens`, `cache_creation_input_tokens`, cache TTL fields, anything named `prompt_cache`)? Where (file:line), and is it forwarded to the pager today or dropped?
B. **Pager**: does the pager already receive or store cache-read / cache-creation token counts per turn? Where (struct, file:line)? Does it store the time of the last model response?
C. **Derivation**: with what A and B give, can warm/cold and time-left be worked out (last response time + the cache TTL)? State which TTL the session uses (5 minutes or 1 hour) and where that is decided; mark as unknown if you cannot find it.
D. **Statusline file**: does any file in `~/.cache/claude-status/` get written for a `grok claude` session? Check by listing the folder and reading the `session_id`/`cwd`/timestamps of the newest files. Do not start any session to test.

## Rules

- No source edits, no cargo, no installs, no model turns. Never kill by name (no pkill/killall). Never tmux.
- Search with `rg` scoped to a folder; recursive `grep` and searches rooted at `/` are blocked by a hook.
- Every row cites file:line that you opened. Mark anything inferred as inferred.

## Output (structured)

Write `docs/SCOUT-cache-warmth-2026-11-15.md` with one table per question A-D, columns:

`file:line | what is there now | what would change | rough lines changed | unknowns`

Then `## Verdict`: one of "derivable in the pager alone", "needs an adapter patch", "not available", with the two or three facts that decide it.

Commit only that file by path and push to main (`git pull --rebase` first if the push is rejected), message ending exactly:

Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Print the commit SHA and stop.
