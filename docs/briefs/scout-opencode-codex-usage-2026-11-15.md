# Haiku scout: OpenCode and Codex usage sources on sab-air

Read-only, on sab-air over ssh (`ssh sab-mini@sab-air '<command>'`). sab-air is Sab's primary machine: read files and run the listed read-only commands only. Report by 10 minutes after you start; if not done, write what you have and stop.

## Sab's words (the intent; nothing below overrides them)

2026-11-14: "I think that the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok, fuck it."

2026-11-15: "Why not include cache, claude usage, code, opencode"

## Already known (read, do not redo)

- On sab-air: `opencode` 1.19.4 at `/opt/homebrew/bin/opencode`, data in `~/.local/share/opencode/` (`opencode.db`), and an `opencode stats` command (tokens and cost).
- Codex rollouts live in `~/.codex/sessions/YYYY/MM/DD/*.jsonl`; some carry a `rate_limits` record (300-minute window, percent, reset time). A quick parse of the newest rollout on sab-air (2026-11-13) found none, which is not conclusive.
- An earlier scout looked only on the Mini and wrongly reported OpenCode as absent. `docs/SCOUT-usage-footer-areas-2026-11-15.md`, `docs/HARNESS-FACTS.md`.

## Answer these

A. **OpenCode**: what usage or limit figures exist on sab-air? Run `opencode stats` and `opencode --help` (read-only). Open `opencode.db` read-only (`sqlite3 -readonly`), list the tables and columns, and say which hold tokens, cost, time, provider/model. Is there any plan-window percentage (5h or weekly) for the OpenCode Go subscription, in the database, a config or auth file, or a log? "None found" with where you looked is a valid answer. Print column names and counts only, never message text.
B. **Codex**: across the last 10 rollout files on sab-air, which contain a `rate_limits` record? Give the record's exact field names and one example with numbers. Say what kind of session writes it and what kind does not (compare the first `session_meta` line of each).
C. **Reader**: for each of A and B, the cheapest way for a program on the same machine to read the newest figure (file path pattern, the query or the JSON path), and how stale it can be.

## Rules

- On sab-air: no writes, no installs, no model turns, no `opencode run`, no `codex` commands that start a session, nothing CPU-heavy. Never kill by name (no pkill/killall). Never tmux.
- Never copy session message text, prompts or credentials into the report: field names, counts and numbers only.
- Searches rooted at `/` and recursive `grep` are blocked by a hook; scope every search to a named folder.
- Mark anything inferred as inferred.

## Output (structured)

Write `docs/SCOUT-opencode-codex-usage-2026-11-15.md` (in this repo, on the Mini) with one table per question A-C, columns:

`source (path or command) | fields | example values | how fresh | unknowns`

Then `## Verdict` per tool: "has a 5h/weekly percentage", "has tokens and cost only", or "nothing usable".

Commit only that file by path and push to main (`git pull --rebase` first if the push is rejected), message ending exactly:

Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Print the commit SHA and stop.
