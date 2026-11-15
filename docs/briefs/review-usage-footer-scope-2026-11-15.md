# Sonnet reviewer: validate size and scope of "5h usage in the footer"

Review only. You change no source code. Report by 10 minutes after you start.

## Sab's words (the intent and the pass bar)

2026-11-14: "I think that the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok, fuck it. I think we could fit, fit them all if we. We're able to abbreviate or use symbols or whatever."

2026-11-15: "handoff to a sonnet 'reviewer' to validate the size and scope of the change, if it makes sense, add it to a queue of changes to make before we do the next install run"

## Input

- The Haiku scout's map: `docs/SCOUT-usage-footer-areas-2026-11-15.md`.
- Its brief: `docs/briefs/scout-usage-footer-areas-2026-11-15.md`.
- Earlier finding: `docs/SCOUT-claude-usage-freshness-2026-11-15.md`.

## Do

1. Check the premise first: does the map serve Sab's words above (footer, 5h, Claude and Codex and OpenCode Go and Grok, fits at half width)? Anything the scout swapped or dropped is finding A0.
2. Open every file:line the map cites for areas A, B and C and at least half of the rest. Mark each row right, wrong or unverifiable.
3. Judge size and scope from the code, not from the scout's numbers: what must change, what the scout missed, what it included that is not needed. Say whether the adapter patch is required for a first slice or whether a pager-only slice already delivers something Sab would see.
4. Verdict per slice: QUEUE (makes sense as scoped), RESCOPE (say how), or DROP (say why).

## Rules

- No source edits, no cargo, no installs, no model turns. Never kill by name. Never tmux.
- Search with `rg` scoped to a folder; recursive `grep` and searches rooted at `/` are blocked by a hook.

## Output

- Write `docs/review/usage-footer-scope-2026-11-15.md`: verdict line first, then findings most severe first with file:line, then the row-by-row check.
- For every slice with verdict QUEUE, add one entry under `## Queued` in `docs/QUEUE-next-install.md` in the format that file shows, and remove the matching entry from `## Being scouted`. Put RESCOPE and DROP slices under `## Not queued` with one line of reason.
- Commit only those two files by path and push to main (`git pull --rebase` first if rejected), message ending exactly:

Co-authored-by: claude-code sonnet-5.5
Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>

Print the commit SHA and the verdict line, and stop.
