# Haiku scout: what it takes to drop the default-key hints from both footers

Read-only. You change no source code. Report by 10 minutes after you start; if not done, write what you have and stop.

## Sab's words (the intent; nothing below overrides them)

2026-11-14: "Right now, the footer of grok claude: 4 useless keybindings that `?` and `Ctrl-,` easily overrule in terms of value."

2026-11-15: "Its even worse: y : copy Enter: open j/k:nav Shift+l/h:turn Ctrl+x: shortcut. There is the input editr footer and then navigational one where it shows defaults. I already have a command palette, what I do not have is a warm cache signal. Why not include cache, claude usage, code, opencode"

## Already known (read, do not redo)

- Pager crate: `crates/codegen/xai-grok-pager/src`.
- Typing footer hints are built in `views/agent.rs` (around 1286-1302). Navigation (scrollback) footer hints come from actions with `When::ScrollbackFocused` in `actions/defaults.rs` (`hint_priority`, `hint_key_display`).
- The row is drawn by `views/shortcuts_bar.rs`; it has a right-hand text slot (`with_right_text`, around 190-192, painted around 292-303) that disappears at narrow width. Called from `app/agent_view/render.rs` (around 3888-3900).
- `docs/SCOUT-usage-footer-areas-2026-11-15.md` and `docs/review/usage-footer-scope-2026-11-15.md` (covers adding Claude usage only).

## Map these

A. **Every footer state**: list each state in which the bottom row shows hints on the agent screen (typing, scrollback navigation, any other: modal open, approval prompt, selection, search, side panel focused). For each: the code that picks the hints (file:line) and the hints shown.
B. **Which hints are not plain defaults**: any hint that carries state the user needs in the moment (for example an approval choice, a queued-message key, a "press again to quit" notice). These must be listed separately so they are not removed by mistake.
C. **Removal**: the smallest change that stops the default-key hints showing in the typing and navigation states while leaving the row itself (for status text) and the items from B. Files, lines, rough lines changed.
D. **Row as status**: what `shortcuts_bar.rs` needs so status text fills the row from the left and still shows at half width (the right-hand slot vanishes when narrow). Rough lines changed.
E. **Tests**: every test and snapshot file that renders these footers and would change (path and count).

## Rules

- No source edits, no cargo, no installs, no model turns. Never kill by name (no pkill/killall). Never tmux.
- Search with `rg` scoped to a folder; recursive `grep` and searches rooted at `/` are blocked by a hook.
- Every row cites file:line that you opened. Mark anything inferred as inferred.

## Output (structured)

Write `docs/SCOUT-footer-hints-removal-2026-11-15.md` with one table per area A-E, columns:

`file:line | what is there now | what would change | rough lines changed | unknowns`

Then `## Slices`: the change cut into the smallest independent pieces, each with the files it touches.

Commit only that file by path and push to main (`git pull --rebase` first if the push is rejected), message ending exactly:

Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Print the commit SHA and stop.
