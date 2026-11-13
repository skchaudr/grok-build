Sab, 2026-11-13 10:47: "figure out why pasting an image from sab-air to here is fucked"

"Here" = a Rex pane on the Mini, viewed from Rex.app on sab-air. Report by 30 minutes after you start.

Read first: docs/FIX-rex-image-paste-2026-11-13.md (this morning's fix: Cmd+V in Rex runs `rex-clip image`), docs/SCOUT-image-paste-2026-11-13.md, docs/traces/briefs/rex-image-paste.md.

Find the cause: reproduce, then trace where the image stops (sab-air clipboard -> rex-clip -> file on the Mini -> path pasted into the pane -> the program). Fix it only if the cause is clear; say what you changed.

Rules:
- Verify without spending model calls: paste into a plain shell pane (e.g. `cat` in a Rex tab you open and name), never into an agent's chat; never press Enter in an agent pane.
- Back up any config before editing (`<file>.bak-2026-11-13b`). Reload Rex config only through Rex's own reload; never restart or kill Rex.
- Restore sab-air's clipboard to text afterwards.
- Kill only your own PIDs; never pkill/killall; never tmux.

Write docs/FIX-rex-image-paste-2-2026-11-13.md: cause, evidence, what changed. Commit by path in /Volumes/Mini-SSD/offload/repos/grok-build, push to main, trailers exactly:
Co-authored-by: claude-code sonnet-5.5
Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
Print the SHA and stop.
