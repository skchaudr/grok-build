# Observations — sync/upstream-2026-10

## Merge

Merged `upstream/main` (`2bdd1d6a`) into `sync/upstream-2026-10`. Five content conflicts, 13 hunks. `acp/spawn.rs` and `app/cli.rs` auto-merged.

## Auth

`initialize_connection` takes `external_agent: bool`.

- `connect()` passes `flags.agent_cmd.is_some()`.
- `connect_via_leader()` passes `false`.

Empty auth methods skip client auth only on the external-agent path. The built-in agent, including leader mode, still fail-closes to login. `app/mod.rs` still forces `use_leader = false` when `--agent-cmd` is set.

## Headless session load

Our `--agent-cmd` commit replaced headless `open_session`'s `"Session does not exist"` bail with `"Session load failed over ACP: {e}"`. Upstream later removed that bail and falls through to `NewSession` when `LoadSession` fails, and added `OpenedSession.cwd`.

Resolution keeps upstream's fallthrough and the `cwd` field. The ACP error bail is gone, so a failed headless load creates a new session. That matches current upstream built-in behavior.

## Struct fields kept from both sides

`ConnectFlags` has upstream `status_line` and our `agent_cmd`.

`MaterializeCtx` has our `has_agent_cmd` plus upstream `restore_code`, `recent_session_selection`, and `restore_progress_on_stdout`. `remote_miss_ctx` was upstream-only and needed `has_agent_cmd: false` added by hand; it was not a conflict hunk.

The external-agent early return in `resolve_existing_session` also omitted upstream's new `ResolvedExisting.suppress_code_restore` field. Set it to `false`, matching the local-hit path.

## Missing internal docs

`cargo test -p xai-grok-pager` compiles `tests/registered_features_are_documented.rs`, which `include_str!`s `docs/internal/25-enterprise.md` and `docs/internal/22-environment-variables.md`. Those paths are not in `upstream/main` either. Added both as tables generated from `xai_grok_config_types::FEATURES` (20 rows) so the test can compile and still checks every registered key.

## External-agent AuthManager

`AuthManager::new` is `cfg(any(test, feature = "test-support"))` after the upstream login split. `cargo test` compiled `spawn_external_subprocess`; `cargo build --release` did not. The external spawn now calls `new_with_proxy_base_url` with `CLI_CHAT_PROXY_BASE_URL_DEFAULT`, which is what `new` did.

## Upstream tests that fail here

`cargo test -p xai-grok-pager` ran 10213 passed, 19 failed, 4 ignored. The three external-agent auth tests passed. The 19 failures are paste/`file://` tests and `doctor_cmd::fake_standalone_facts_compose_through_shared_view`. `paste.rs`, `input.rs`, and `doctor_cmd/tests.rs` have no diff against `upstream/main`.

## Disk

The shared `target/` dir filled the 99 GB volume during the debug build (linker bus error, os error 28). Removed `target/debug/incremental` and `target/release/incremental` (rebuild caches, not the release binaries) to free about 13 GB. Tests were rerun with `CARGO_INCREMENTAL=0`.

---

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
