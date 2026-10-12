# Observations — gb-tui-controls (Task B)

- `COMMON.md` referenced in BRIEF is not in this worktree; implemented from BRIEF acceptance criteria directly.
- First `cargo test` failed: `bin/protoc` needs `dotslash` on PATH (`cargo install dotslash`).
- pi-acp changes are in `~/repos/Pi-Coding-Agent/pi-acp-patched/index.js` (bundled output; no separate `.ts` source in repo).

WORKER DONE 26eeb31e

REJECTED: on-screen proof was not run. `df -h /` showed Avail 8.8G (under the 15G cargo-build stop), and this worktree has no `target/debug` pager/`grok` binary (`grok`, `xai-grok-pager`, `xai-grok-pager-bin` all missing). Did not start a build. Did not launch `grok pi` or `grok claude` in a pty, so the slash popup, `/model` selection, and `/usage` were not seen. The pager unit-test objects under `target/debug/deps/xai_grok_pager-*` are from 10:57, before `registry.rs` (11:02) and commit `26eeb31e` (11:06), so they were not re-run. Disk hogs to clear before a retry, not deleted: `~/repos/grok-build/target` 15G, `~/repos/grok-build/.worktrees/gb-session-resume/target` 15G, `~/repos/grok-build/.worktrees/gb-tui-controls/target` 7.8G. Not merged to main.
