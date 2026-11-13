# Fix: image paste into Rex panes (2026-11-13)

## Problem

Sab runs Rex.app on sab-air. His panes (grok-fork, claude) run on the Mini. An image copied on sab-air and pasted with Cmd+V reached nothing, because the program reads the Mini's clipboard. Scout: `docs/SCOUT-image-paste-2026-11-13.md`.

## How Herdr does it

Read from the Herdr binary (`~/.local/bin/herdr`), not from source. Herdr has no written docs for this path.

- The client machine reads its own clipboard when a paste key fires. The protocol has a `ClipboardImage` client message.
- The image bytes are sent to the server host, staged as a file in a temp `herdr-clipboard-images-*` directory, and the staged path is pasted into the pane.
- Image formats: png, jpg, gif, bmp. Oversized images are refused ("local image is too large to bridge").
- Config key `remote_image_paste = "ctrl+v"`, documented as "only active in herdr --remote". The raw-key trigger is what makes plain paste work.

Confidence: medium. The mechanism is inferred from strings in the binary.

## What Rex already had

`~/.config/rex/init.lua` (on the Mini, the file the Mini Rex server loads):

- `paste_clip_image` action: runs `rex-clip image`, which saves the sab-air clipboard PNG on the Mini and prints its path. Wraps the path in bracketed paste when the pane asks for it.
- Bound to `ctrl+k>y>i` and `ctrl+k>r>i`. Only the prefix chord, not Cmd+V.

## What changed

Live config: `/Users/sab-mini/.config/rex/init.lua` (not a git repo). Backup: `/Users/sab-mini/.config/rex/init.lua.bak-2026-11-13`.

1. New action `paste_clip_or_image`, placed before the "Close pane" action:
   - runs `rex-clip image`.
   - If it returns a path, writes the path to the focused block, bracketed if the block asks for it.
   - Otherwise, `rex.client.queue("pane.paste")`, which is the client's normal text paste.
2. New binding after `rex.bind("f13", "espanso_paste")`:
   ```lua
   rex.bind("cmd+v", "paste_clip_or_image")
   ```

The sab-air server has its own copy of `init.lua` (750 lines, git repo at `~/.config/rex`). It serves panes that run on sab-air, which already paste fine. I did not change it.

Side effect: a text Cmd+V now waits about 0.36 s for the image check (`rex-clip image` on a text clipboard) before the text pastes.

## Verification

Done:

- `rex config check`: OK, 128 bindings, 15 actions.
- `rex config reload`: generation 3. `rex keymap` shows `cmd+v  paste_clip_or_image  init.lua:546`.
- Image path: a 64x64 PNG on sab-air's clipboard, run `rex do paste_clip_or_image` against a `cat` pane on the Mini. Result: `ok: true`, path `/var/folders/.../T//rex-clip-1794586090-69683.png`. `file` reports `PNG image data, 64 x 64`.

Not verified:

- **Text paste through the action did not show up in any pane.** The action returned `pasted: text`, but the text did not appear in the test pane or any visible block on either Rex server. Client-side `pane.paste` was also run directly and produced no visible result. Cause unknown.
- **The real Cmd+V keypress was never tested.** All tests invoked the action through the CLI.

Clipboard: sab-air's clipboard was restored to its original text. `cmp` against the saved copy matched.

## Commits

Pending. See the message at the end of this session.

## Open items

- The test pane `49e1f3` (session `rex-image-paste`) is still open.
- Text paste needs one manual check: press Cmd+V on sab-air with text copied, into a pane you don't care about, with no Enter pressed.
