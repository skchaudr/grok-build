# Observations — gb-tui-controls (Task B)

- `COMMON.md` referenced in BRIEF is not in this worktree; implemented from BRIEF acceptance criteria directly.
- First `cargo test` failed: `bin/protoc` needs `dotslash` on PATH (`cargo install dotslash`).
- pi-acp changes are in `~/repos/Pi-Coding-Agent/pi-acp-patched/index.js` (bundled output; no separate `.ts` source in repo).

WORKER DONE 26eeb31e

REJECTED: on-screen proof was not run. `df -h /` showed Avail 8.8G (under the 15G cargo-build stop), and this worktree has no `target/debug` pager/`grok` binary (`grok`, `xai-grok-pager`, `xai-grok-pager-bin` all missing). Did not start a build. Did not launch `grok pi` or `grok claude` in a pty, so the slash popup, `/model` selection, and `/usage` were not seen. The pager unit-test objects under `target/debug/deps/xai_grok_pager-*` are from 10:57, before `registry.rs` (11:02) and commit `26eeb31e` (11:06), so they were not re-run. Disk hogs to clear before a retry, not deleted: `~/repos/grok-build/target` 15G, `~/repos/grok-build/.worktrees/gb-session-resume/target` 15G, `~/repos/grok-build/.worktrees/gb-tui-controls/target` 7.8G. Not merged to main.

## Loop owner (Opus 5.5), 2026-10-12

- Shared-target trap: `cargo build -p xai-grok-pager` only builds the lib. The `xai-grok-pager` binary comes from `xai-grok-pager-bin`. Until that package is built, `~/.cargo-target/debug/xai-grok-pager` is whatever another worktree last linked. The first pty runs here used gb-session-resume's binary. Build `-p xai-grok-pager-bin`, copy the binary out, and `grep -a` the copy for a string from your change.
- pi-acp SDK 0.26 has no `session/set_model` route, so Grok's `/model` pick got "Method not found". Fixed in pi-acp via `extMethod` (Pi-Coding-Agent e3dd9ed); `pi-acp-patched/test/set-model.integration.mjs` failed first, then passed.
- claude-acp 0.16.2 logs `/usage` output and drops it. It only forwards `/context` output when the text contains "Context Usage", which current Claude output does not. The pager now pushes `/<cmd> returned no output from <agent> (not supported through ACP)` on every turn-end path when a forwarded slash turn completes with no agent text.
- In the tmux run, putting a `pi-acp` PATH shim first made the embedded shell's ACP initialize fail ("server shut down unexpectedly") before the shim ran. Launching the pager with `--agent-cmd 'node …/pi-acp-patched/index.js'` worked. Not investigated further.
- Open: a successful `/model` under an external agent also persists `default_model` globally, so a Pi model id becomes Grok's default. A failed switch does not roll back the persisted value; only the in-session footer is rolled back.
- Open: in one claude run a stray `/resume-claude <id>` prompt was sent at startup, and two prompts queued behind it never drained after "Turn completed." It did not reproduce in a fresh directory. Possibly pre-existing; not investigated.
- `minimal_slash_dropdown_still_consumes_tab` and `drain_after_editing_sends_correct_prompt` fail when run alone but pass in the full lib run (order-dependent).

VERIFIED (pager-bin built at 98f97b77, binary checked for the notice string; pi-acp e3dd9ed)
- `grok pi` (patched pi-acp), tmux 160x45:
  - The slash popup lists /usage /context /compact /login /model plus Pi-advertised /dirs /add-dir /suggest-dirs.
  - `/model` lists Pi's models. Picking "deepseek/DeepSeek V4 Pro" changed the footer. The session file `~/.pi/agent/sessions/*pi2*/2026-10-12T22-13-15-329Z_*.jsonl` shows `model_change deepseek deepseek-v4-pro`, then `assistant deepseek deepseek-v4-pro ['PONG']`.
  - `/usage` rendered "Cost: 0.03583404 Tokens: in 27138, out 3, total 27141".
- Failed switch (stock /usr/bin/pi-acp): the screen shows `Couldn't switch model: "Method not found": session/set_model` and the footer stays on "google-antigravity/Gemini 3.6 Flash (Antigravity)".
- `grok claude` via gb:
  - The popup lists /usage /context /compact /login /model.
  - `/usage` showed "Worked for 0.3s", then "/usage returned no output from claude (not supported through ACP)".
  - `/context` showed "Worked for 0.6s", then "/context returned no output from claude (not supported through ACP)".
- `cargo test -p xai-grok-pager --lib`: 10288 passed, 19 failed. All 19 are paste_key_tests / scrollback_paste / doctor_cmd tests (pre-existing, untouched by this branch).
