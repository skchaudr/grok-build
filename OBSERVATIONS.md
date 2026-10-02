# Observations

## 2026-10-02 Task A

- `profiles/acp-enhanced` reads `DSH_ACP_PROVIDER` / `DSH_ACP_MODEL`, but its dumped config does not register a `cliproxy` provider (no base URL). The `:8317` provider block lives on the `web`, `acp`, `tui`, `headless`, and `pi-host` patches, not on `acp-enhanced`. The matrix still sets the env vars the brief names (`cliproxy` / `grok-4.7`).
- An in-flight `cargo build --release -p xai-grok-pager-bin` was already running in `/home/sab-mini/repos/grok-build` (the dirty main checkout). `acp/mod.rs` there differs from this worktree. That binary is not a clean `b6a12331` build. This task will rebuild from the worktree into the shared target dir after that cargo exits.
- Leftover drafts `acp-matrix.leftover.sh` and `docs-acp-matrix.leftover.md` were read and deleted.
- The shell already exports `PAGER=less`. The matrix script uses `ACP_PAGER` so it does not launch `less` as the pager binary.
- A clean `cargo build --release -p xai-grok-pager-bin` from this worktree recompiled the workspace and died at `linking with cc failed: exit status: 1` while producing `xai-grok-pager-minimal`. The captured log was truncated before the linker diagnostic. Root filesystem was at 4.5G free (target dir 23G: release 17G including 8.5G `release/incremental`). `rm -rf` of that incremental cache was blocked by the sandbox. The release binary still on disk is the 10:27 link from the dirty main checkout (`acp/mod.rs` there is 1135 lines; this branch's file is 1198). It is not a clean `b6a12331` pager.
- Another `cargo test -p xai-grok-pager` (toolchain 1.94, debug) started in the shared target dir while the release link was failing. This task waited, then rebuilt with `CARGO_INCREMENTAL=0`. The clean binary is `/home/sab-mini/repos/grok-build/target/release/xai-grok-pager` (2026-10-02 11:06, `Finished release`).
- `:8317` does not list `grok-4.7`. `grok-4.6` returns HTTP 426 ("Grok CLI version 0.2.120 is outdated"). `gpt-6-astra` returned a completed response with no content. `gemini-3.8-flash-high` ran `ls` and replied `done`.
- DSH `session/new` sometimes dies with `server shut down unexpectedly` before a turn. A later session on gemini stayed up.
- `--continue` after a successful turn did not log `session/load` for dsh, pi, or cursor, and the resume capture was empty.
- `codex-acp` reaches the composer and echoes the prompt, then fails the turn: `gpt-5.6-sol` requires a newer Codex. No tool card.
- `grok agent stdio` closes the ACP channel during `initialize` (`recv_failed`). The pager exits 1 before the composer is drawn. With stdin closed and no pager, `grok agent stdio` exits 0 and prints nothing.
- The home screen has no "Type a message" placeholder. Ready is the `❯` composer plus `always-approve`. The first submit opens "Run Grok Build in a project directory?"; option 1 is the throwaway cwd.
