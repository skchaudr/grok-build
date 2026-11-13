# Rex image paste, round 2 (2026-11-13)

## Cause (confidence: medium-high)

The `cmd+v` binding added this morning never runs when Sab presses Cmd+V in Rex.app on sab-air. The app pastes text through its own `pane.paste`, so an image-only clipboard pastes nothing and the image never leaves sab-air.

The pipe behind the binding is fine. The key that was meant to reach it does not.

## Evidence

Each stage traced with a 64x64 red PNG on sab-air's clipboard and a plain `cat` pane (session `rex-image-paste-2`) on the Mini:

| Stage | Result |
|---|---|
| sab-air clipboard holds PNG | `clipboard info` shows `«class PNGf», 178` |
| `rex-clip image` from the Mini | 0.45 s, valid path `.../T//rex-clip-<ts>-<pid>.png` (178 bytes) on the Mini |
| `rex do paste_clip_or_image -s rex-image-paste-2 -b 0a2c42 -C sab-air` | `ok: true`, path returned |
| Path in the pane | `rex capture` shows the path in the `cat` pane (twice, two runs) |
| Real Cmd+V in Rex.app | **No action run.** A probe line (`date`, `hostname`, `ctx.origin`) added as the first statement of the action wrote to `/tmp/rip2-probe.log` for the CLI run (`sab-mini origin=api`) and wrote nothing, on the Mini or on sab-air, after Sab pressed Cmd+V in the pane. The pane received `https://github.com/Dicklesworthstone/cass_memory_system` (sab-air's clipboard text at that time) — a plain text paste. |

Supporting log facts (`rex server logs --all`, Mini server):

- `client.keymap` from the app is fetched after each reload and at app launch (09:23:11 after relaunch), so the app has the config bindings.
- The server has never logged a successful `client.key` or `client.action` from sab-air (0 with status 200; all 87 `client.key` lines are 404 on stale blocks). The app's key handling for `cmd+v` produced no server request after 09:23.
- sab-air's own `init.lua` (git repo `~/.config/rex`) has no `cmd+v` binding and the app's shortcut settings (`~/Library/Application Support/com.superlogical.rex/Shortcuts/shortcuts.json`) do not bind it either; only `alt+v -> pane.paste` is there. The app's built-in `cmd+v` paste is what ran.

Not verified: whether Sab's press happened with the image or the URL on the clipboard (the clipboard changed to the URL before I checked). The probe stayed silent for the press that pasted the URL, and a key binding is matched by key, not by clipboard content, so the same applies to an image press. Why the app does not route Cmd+V to a Lua binding is also unverified; the likely reason is that Cmd+V is a macOS Edit-menu key equivalent that the app handles before its keymap. That is a guess from the symptom.

## What changed

Nothing remains changed. The live config `/Users/sab-mini/.config/rex/init.lua` is back to this morning's version (`cmd+v` binding present, no probe). Backup of that state: `init.lua.bak-2026-11-13b`. The probe was added and removed within this run (config generations 4 and 5).

## Next step (needs one keypress from Sab)

Bind the action to a key the app does not claim as a menu shortcut and test it with a real press. Candidates:

- `ctrl+v` (what Herdr uses for `remote_image_paste`). Needs a pass-through for the no-image case (`pane.send_key` ctrl+v), because agents use Ctrl+V themselves.
- `ctrl+k>v>v` or similar prefix chord. Chords already work (`ctrl+k y i`, `ctrl+k r i` use the same action).

Test script: put a PNG on sab-air's clipboard, press the chosen key in a `cat` pane, check the pane shows an image path. I did not do this: Sab was actively typing on sab-air during the run.

Other facts:

- sab-air's clipboard was not restored to its original 42-byte text: Sab copied something else during the run, so the clipboard was left alone.
- Run overshot the 30-minute report time: the question waiting for Sab's keypress took about an hour.
