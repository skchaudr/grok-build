# Scout: image paste into grok-fork (2026-11-13)

## Verdict

No fork regression found. The image paste path is unchanged since before 2026-10-15 and works on both Macs. The likely cause is the clipboard host split: grok-fork reads the pasteboard of the machine it runs on, and Sab's Rex.app runs on sab-air while the panes run on the Mini. Confidence about 60%.

## Evidence

Commits since 2026-10-15 touching the paste, attach and send path: none. `git log --since=2026-10-15` on `crates/codegen/xai-grok-pager/src/app/agent_view/paste.rs`, `xai-grok-pager-render/src/{clipboard/mod.rs,prompt_images.rs,input/key.rs}` and `xai-grok-shared/src/clipboard.rs` lists nothing for paste. The nearest commits (caec749c Prime, 5afdbfab Pi shift-arrows, b8f926f1 docs images) do not touch it.

Paste path (all unchanged):
- Ctrl/Cmd+V key: `xai-grok-pager/src/app/agent_view/prompt.rs:633` (`is_paste_key`, `xai-grok-pager-render/src/input/key.rs:251`) -> `paste.rs:184` `handle_paste_key_deferred` -> `clipboard/mod.rs:876` `attachment_probe_gate` -> `Effect::ProbeClipboardAttachment` (`app/effects/mod.rs:2174`) -> `xai-grok-shared/src/clipboard.rs` NSPasteboard snapshot plus osascript read of the pasteboard of the local machine.
- Bracketed paste (Cmd+V in a terminal): `paste.rs:430` `probe_attachment_around_bracketed_insert`, empty payload probes the local pasteboard (`clipboard/mod.rs:783`).
- Path paste: `paste.rs:~300` `try_handle_dropped_paths_paste` -> `prompt_images::try_read_dropped_paths` (`prompt_images.rs:1075`). Skipped only when `terminal_context().is_ssh` (`paste.rs:46`).
- Send: `prompt_images.rs:1372` `build_content_blocks_with_prefixes_and_caps_ref` emits `ContentBlock::Image` (base64 data plus `uri`).
- Capability: `acp/mod.rs:575` stores `promptCapabilities.image`; `acp/model_state.rs:134` `current_model_accepts_images` returns true when the agent advertises it. It only gates the clipboard tip, not the paste. claude-agent-acp 0.76.0 (`dist/acp-agent.js:916`) advertises `image: true` and `promptToClaude` (`:7054`) takes base64 `data` first. No capability gate drops images. Pi: `pi-grok-adapter/src/pi_adapter/agent.rs:17` advertises image, `prompt_bridge.rs:103` forwards it.

Live checks on the installed binaries (Mini `4c3ace53`, sab-air `c7f5903e`), no model calls, a recording stub agent for the send check:
- Ctrl+V with a PNG on the local pasteboard: `[Image #1]` chip appears on Mini and on sab-air.
- Empty bracketed paste (`ESC[200~ESC[201~`, what Cmd+V sends for an image-only clipboard) with a PNG on the local pasteboard: chip appears on both.
- Bracketed paste of `$TMPDIR/rex-clip-*.png`: chip appears (Mini).
- Prompt with two chips sent to the stub agent: wire carries `text` plus two `image` blocks, `image/png`, 10480 base64 chars each.
- `osascript` exists on both Macs; `pngpaste` is not used by the code and is not installed.
- Last image delivered through grok claude (`sdk-ts` entrypoint in Claude transcripts): Mini 2026-11-12 21:30Z, sab-air 2026-11-12 06:17Z. Nothing newer on either, which brackets when it stopped being observed, not why.

Why this points to the host split: `rex client ls` shows the only Rex app is `Rex Beta` on `sab-air`; this pane runs on the Mini. A pager in a Mini pane reads the Mini pasteboard. An image copied on sab-air never reaches it: Cmd+V in Rex sends text only, Ctrl+V probes the Mini pasteboard and finds `NoRaster`, silently (`paste.rs:~230`, `ProbedAttachment::NoRaster -> FullMiss`). Worse, if the Mini pasteboard holds an old image, that stale image is attached. Panes running on sab-air itself are fine (verified).

## Smallest fix

No fork edit. The Rex config already carries the fix: `~/.config/rex/init.lua:608` `ctrl+k r i` and `:602` `ctrl+k y i` run `paste_clip_image` (`rex-clip image` reads the Rex.app host pasteboard over ssh, writes `$TMPDIR/rex-clip-*.png` on the pane host, bracketed-pastes the path). Verified now: with a PNG on sab-air, `rex-clip image` run from the Mini returned a valid 64x64 PNG path, and the pager turns that path into a chip (above).

If Sab wants plain Cmd/Ctrl+V to do this, the change belongs in Rex's `init.lua` (a v-key binding that calls `paste_clip_image` when `rex-clip` finds an image, otherwise passes the key through). I did not verify Rex's pass-through API, so that edit is not review-ready.

Optional fork hardening, only if Sab wants it: a toast on `ProbedAttachment::NoRaster` after Ctrl+V so the miss is not silent (`paste.rs`, the `NoRaster => FullMiss` arm). Not needed for the cause.

## Not verified

- Whether Sab's failing paste was in a Mini pane under sab-air's Rex. That is inferred from the Rex client list and config.
- Claude CLI 2.1.323 on sab-air (installed 2026-11-12 14:42, 55 min before the fork reinstall) is untested for image input; testing needs a model call. It is the second suspect if panes run on sab-air itself and still fail.

Side effect of the checks: sab-air's clipboard was overwritten with a test PNG and then emptied; its previous contents are gone. Mini's clipboard text was restored.
