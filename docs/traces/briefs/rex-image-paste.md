Make image paste in Rex work the way it does in Herdr. Report by 40 minutes after you start.

Problem (docs/SCOUT-image-paste-2026-11-13.md): Sab runs the Rex app on sab-air (ssh sab-mini@sab-air); his panes (grok-fork, claude) run on the Mini. When he copies an image on sab-air and pastes into a Mini pane, nothing arrives: the program reads the Mini's clipboard. Herdr handles this exact case (an image copied on the client machine reaches the remote pane). A Rex workaround exists: `ctrl+k r i` / `ctrl+k y i` runs `rex-clip image`, which pastes the image's file path from the Rex host.

Do:
1. Find how Herdr does it (Herdr is installed; read its docs/config/source on either Mac). One paragraph.
2. Find Rex's config (init.lua, on sab-air and/or the Mini; `rex config` command) and the existing `ctrl+k r i` binding + `rex-clip`.
3. Make a plain paste (Cmd+V) in Rex do what Herdr does when the sab-air clipboard holds an image: get the image to the Mini and paste its path (grok-fork turns a pasted image path into an `[Image #N]` chip). Text paste must keep working unchanged.
4. Back up any config file before editing it (`<file>.bak-2026-11-13`). Reload Rex config only through Rex's own reload command; never restart or kill Rex.
5. Verify without spending model calls: put a small PNG on sab-air's clipboard, paste into a plain shell pane on the Mini (e.g. a `cat` in a Rex tab), and confirm a valid image path arrives; then copy text and confirm text paste still works. Restore sab-air's clipboard to text afterwards. Never paste into an agent's chat and never press Enter in one.

Write docs/FIX-rex-image-paste-2026-11-13.md: how Herdr does it, what you changed (file paths, diff), the verification output. Commit that doc by path in /Volumes/Mini-SSD/offload/repos/grok-build, push to main, trailers exactly:
Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
If a Rex config lives in a git repo, commit it there too with the same trailers.

Kill only your own PIDs; never pkill/killall; never tmux. When done, print SHA(s), what changed, and stop.
