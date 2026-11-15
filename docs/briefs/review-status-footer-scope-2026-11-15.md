# Sonnet reviewer: validate size and scope of "footer shows status, not key hints"

Review only. You change no source code. Report by 10 minutes after you start; if not done, write what you have and stop. Do the work yourself in this session: no background or hidden subagents.

## Sab's words (the intent and the pass bar; nothing below overrides them)

2026-11-14: "Right now, the footer of grok claude: 4 useless keybindings that `?` and `Ctrl-,` easily overrule in terms of value. I think that the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok, fuck it. I think we could fit, fit them all if we. We're able to abbreviate or use symbols or whatever. Like right now I'm literally, split screen."

2026-11-15: "Its even worse: y : copy Enter: open j/k:nav Shift+l/h:turn Ctrl+x: shortcut. There is the input editr footer and then navigational one where it shows defaults. I already have a command palette, what I do not have is a warm cache signal. Why not include cache, claude usage, code, opencode"

2026-11-15: "handoff to a sonnet 'reviewer' to validate the size and scope of the change, if it makes sense, add it to a queue of changes to make before we do the next install run"

sab-air (`ssh sab-mini@sab-air`) is Sab's primary machine; the Mini is the build box. A tool that exists on sab-air counts as present.

## Input

- Three Haiku scout reports: `docs/SCOUT-cache-warmth-2026-11-15.md`, `docs/SCOUT-opencode-codex-usage-2026-11-15.md`, `docs/SCOUT-footer-hints-removal-2026-11-15.md`, and their briefs in `docs/briefs/scout-*-2026-11-15.md`.
- Earlier: `docs/SCOUT-usage-footer-areas-2026-11-15.md`, `docs/review/usage-footer-scope-2026-11-15.md`, `docs/SCOUT-claude-usage-freshness-2026-11-15.md`.
- The queue: `docs/QUEUE-next-install.md`.

## Do

1. Check the premise first: do the three reports serve Sab's words above (both footers lose the default-key hints; the row carries cache warmth, Claude usage, Codex, OpenCode; readable at half width)? Anything a scout swapped, dropped or decided on Sab's behalf is finding A0.
2. Open every file:line cited in the cache-warmth report and in areas A-D of the hint-removal report. Mark each row right, wrong or unverifiable. For the sab-air report, re-run at most three of its read-only commands to spot-check (no writes, no model turns, no session text in your report).
3. Judge size and scope from the code, not the scouts' numbers. In particular:
   - Cache warmth: is "derivable in the pager alone" true? What is the cache lifetime for a `grok claude` session and where could it be read rather than assumed?
   - Hint removal: which hints carry state the user needs in the moment and must stay; is the scout's keep-list right?
   - Status row: what `views/shortcuts_bar.rs` needs so status shows from the left at half width.
   - Codex: is reading the newest rollout with a rate-limit record a sound source when the footer belongs to a `grok claude` session?
   - OpenCode: with tokens only and no plan percentage, is there anything worth a footer segment?
4. Cut the whole change into slices in build order. Verdict per slice: QUEUE (makes sense as scoped), RESCOPE (say how), or DROP (say why). List separately the decisions only Sab can make, each as one plain question with your pick.

## Rules

- No source edits, no cargo, no installs, no model turns. Never kill by name (no pkill/killall). Never tmux.
- Search with `rg` scoped to a folder; recursive `grep` and searches rooted at `/` are blocked by a hook.

## Output

- Write `docs/review/status-footer-scope-2026-11-15.md`: verdict line first, then findings most severe first with file:line, then the slices, then the questions for Sab, then the row-by-row check.
- In `docs/QUEUE-next-install.md`: rewrite the queued footer entry as the QUEUE slices in build order (keep Sab's quoted words there as they are); put RESCOPE and DROP slices under `## Not queued` with one line of reason; update the OpenCode and Codex lines to match what was found.
- Commit only those two files by path and push to main (`git pull --rebase` first if rejected), message ending exactly:

Co-authored-by: claude-code sonnet-5.5
Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>

Print the commit SHA and the verdict line, and stop.
