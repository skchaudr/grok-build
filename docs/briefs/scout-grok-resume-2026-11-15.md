# Haiku scout: which sessions can `grok-fork` resume

Read-only. You change no source code. Report by 10 minutes after you start; if not done, write what you have and stop.

## Sab's words (the intent)

2026-11-15: "grok-fork apparently cannot resume normal grok sessions, which is kind of lame, in fact i do not know on sab-air if any sessions are currently resumable"

## Already known

- sab-air is Sab's primary machine (`ssh sab-mini@sab-air`); this Mac (the Mini) is the build box. Both run the same `~/.grok/bin/grok-fork` build from main.
- On sab-air, `~/.grok/bin/grok-fork sessions list` prints 22 Grok sessions (checked 2026-11-15 03:24).
- The resume picker's loading code is `crates/codegen/xai-grok-pager/src/app/dispatch/session/load.rs`; commit `eca9236f` changed it so a Claude host lists through ACP and only a Pi host uses the Pi catalog. Tests: `app/dispatch/tests/session/load.rs`.
- Launch forms: plain `grok-fork` (Grok agent); `grok claude` = `grok-fork --agent-cmd "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=$HOME/.local/bin/claude $HOME/.local/bin/claude-agent-acp"`; Pi via the Pi adapter.

## Questions

1. Under plain `grok-fork`, does the resume picker (ctrl+r) list normal Grok sessions? Open it in a Rex tab on this Mac and capture the screen. Also run `grok-fork sessions list` here.
2. Under `grok claude`, what does the picker list (Claude sessions only, or Grok ones too)? Capture.
3. From the code: for each host kind (Grok, Claude, Pi), which sessions are listed and which can actually be loaded. If Grok sessions fail to list or load under plain `grok-fork`, name the line that causes it.
4. On sab-air, read-only: `sessions list` row count, and whether `~/.grok/sessions` holds the files those rows point at. Do not open a TUI on sab-air and do not run anything heavy there.

## Rules

- No source edits, no cargo, no installs, no model turns. Never kill by name (no pkill/killall). Never tmux.
- TUI checks go in a Rex tab you open (`rex run -s "$REX_SESSION" --label resume-scout-tui --focus=false --json -- <command>`), read with `rex capture -s "$REX_SESSION" -b <block> --trim`, keys with `rex send-key -s "$REX_SESSION" -b <block> <key>`. You may press ctrl+r, arrows and escape. Never press Enter in a TUI and never type into its prompt. Close your tab when done (`rex block close <block>`).
- Search with `rg` scoped to a folder; recursive `grep` and searches rooted at `/` are blocked by a hook.
- Every fact cites file:line or the command output.

## Output (structured)

Write `docs/SCOUT-grok-resume-2026-11-15.md`: first a table `host | lists Grok sessions | lists own sessions | loads | evidence`, then the cause with file:line if something is broken, then the smallest fix and the files it touches. Paste the captured picker screens.

Commit only that file by path and push to main (`git pull --rebase` first if the push is rejected), message ending exactly:

Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Print the commit SHA and stop.
