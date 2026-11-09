# dpi, Pi results, Codex hooks + parser (2026-11-09)

Repo: /Volumes/Mini-SSD/offload/repos/grok-build, branch main. Work in place.

Sab's words: "the thing that I care the most about is, yeah, dude. Let's do the DPI and Pi, and let's get Codex hooks and the parser !!"

Another agent (tab `grok-pi`) is importing grok-pi in this checkout: `crates/codegen/pi-grok-adapter/`, `extensions/`, `Cargo.toml`, `Cargo.lock`, `xai-grok-shell/src/session/workflow/*`. Do not touch or commit its files; stage only your own paths. Builds share the cargo lock with it, so expect waits.

## 1. dpi on the real screen
81ded84b routed running dpi sessions to the live feed; it is installed but never seen on screen. When you are ready to watch, stop and ask Sab to send a prompt to a dpi session (paid turns are his). Check labels, Thought rows, completion, the tail.

## 2. Pi results in the live feed
`~/.pi/agent/extensions/traces.ts` does not forward the tool result on `tool_execution_end`. Editing it is approved, on the Mini and on sab-air (`ssh sab-mini@sab-air`; keep the two files identical, back up each first). Send a bounded result the way the Claude hook does (`hook.rs` caps); confirm in `trace_receipts` that Pi rows get `result`.

## 3. Codex hooks and parser
Codex has no receipts and no live parser, so running Codex sessions stay on the old live follow path. Make Codex a live-feed harness like Claude and Pi:
- Codex hook events → `grok-traces hook codex` (read `docs/HARNESS-FACTS.md` first and update it with what you learn; `TokenEconomizer/plugin/hooks/codex-hooks.json` is a working example of Codex hook config). Wiring Codex's hook config on both Macs is approved; back up first.
- A Codex transcript parser (`~/.codex/sessions` rollouts) for the merge, the way `dpi::pi_transcript` / `live::transcript` serve Pi and dpi.
- Route running Codex sessions to the live feed.

## Rules
- Root cause / design with file:line; a regression test per item, including a Codex call whose transcript line never arrives.
- Tests and builds run visibly in herdr pane **w3E:pW** (tab `tests-live`; `w3E:p4` belongs to the grok-pi agent): `herdr pane run w3E:pW "clear; cd /Volumes/Mini-SSD/offload/repos/grok-build && nice -n 10 cargo ... && echo TESTS-DONE || echo RUN-FAILED"`, then `herdr pane wait-output w3E:pW --regex 'TESTS-DONE|RUN-FAILED' --timeout 3600000`. --release, one cycle at a time.
- Verify on the real screen in your own herdr tab with `target/release/xai-grok-pager traces`, read it with `herdr pane read`. Never type letters into the traces list (its keys resume sessions as paid turns). Never send a prompt to any agent. Close your tab when done.
- Do not install binaries to ~/.grok/bin.
- Commit on main and push as each item lands; every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Report tightly per item: what's on screen, tests read from the pane, commits, config files changed (with backup paths), anything unverified.
