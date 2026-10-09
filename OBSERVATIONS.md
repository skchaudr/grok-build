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

## 2026-10-04 leader external agents

1. The handoff's second forced-off site is stale. `headless.rs` around the cited line sets `has_agent_cmd: false` on a `MaterializeCtx`. It does not set `use_leader = false`. The only forced `use_leader = false` for `--agent-cmd` was `app/mod.rs`.

2. "No Grok login" is not a leader startup failure. `run_leader` marks the server ready after bounded auth even when that auth returns `None`. The gate an external client skips is the pre-ready `leader_starting` error. An external registration is reported `ready: true`, so `LeaderClient::connect` does not wait out the native auth timeout. A native client still gets `Registered { ready: false }` and `leader_starting` until auth finishes. There is no post-ready "no credentials" rejection to add.

3. A failed send on the native agent channel must stay best-effort (`let _ = acp_tx.send`). Several leader tests drop `acp_rx` and inject responses on `response_tx`. Turning that closed channel into a client-visible JSON-RPC error delivered a stray message ahead of the notification those tests assert on. External backends still return a visible error when their process is dead.

4. `cargo test -p xai-grok-shell --lib` did not compile on this branch before the external-agent work. `tool_layer_images_bridge_tests.rs` calls `base64::engine::general_purpose::STANDARD.encode` without `use base64::Engine`. That import was added so `--lib leader::` could run (287 passed).

5. The shared `target/` filled the disk (debug incremental was about 28G) while the pager tests compiled. Incremental was removed. Later builds used `CARGO_INCREMENTAL=0`.

6. `dsh acp` is `dsh --profile acp`. The `gb` picker does not use that command for DSH. It runs `dsh --profile acp-enhanced` with `DSH_ACP_PROVIDER` and `DSH_ACP_MODEL`.

## 2026-10-05 real Air → khoj DSH session and resume

- Passed on existing `89cae842` binaries, with live OpenRouter Mercury 2.5: Air pager → forwarded socket → khoj leader → DSH. Session `6b31df5c-91d6-4f31-80c0-392d8eda1767` produced raw tool output `khoj-38w`, survived client exit, loaded from a fresh client, remembered ORBIT-6274, and ran hostname/uname again. Full reproduction/evidence: `docs/leader-external-agents-proof.md`.
- `grok agent stdio` still sets `agent_cmd: None`; it is not a substitute for the actual pager acceptance test. The tested pager logged `use_leader=true embedded_fallback=false`.
- Mercury's summary dropped the trailing `w` twice; the DSH tool-result events retained the exact hostname. Inspect tool results, not model paraphrases.
- Darwin binary is stamped 1.0.45 while the VM build is stamped 1.0.46; both identify `89cae842`. Air default uses a tiny `--no-auto-update` launcher to preserve the local build; stock binary remains available.
- Existing test executables rerun without rebuilding: external agents 8 passed; stdio integration 47 passed/1 ignored; leader unit tests 287 passed. Local MCP paths and optional native `_x.ai/*` methods remain distinct compatibility concerns; neither prevented the tested DSH turns or explicit-ID resume.

## 2026-10-09 resume hint launcher

Quit hints take an optional launcher (`GROK_RESUME_CMD`) instead of a hardcoded leading `grok`. The quit path reads the env var once and passes it into `print_exit_resume_hint` and `print_relaunch_failure_hint`. Formatting tests pass the string directly, so they do not mutate process env. Unset, empty, and whitespace-only stay `grok`. A trimmed non-empty value replaces that token, including inside the relaunch fallback (`GROK_SCREEN_MODE=fullscreen grok cursor --fullscreen --resume …`). `~/.dsh/scripts/gb` was not edited.

`session_title_resolve` still prints `Resume by session id instead: grok --resume <session-id>`. The brief named plain quit, the `--minimal` variant, and the relaunch-failure fallback, so that other line was left alone.

The root volume had 2.4G free. Removed `target/debug/incremental` (7.6G, rebuild cache) before compiling. Builds used `CARGO_TARGET_DIR=$HOME/repos/grok-build/target` and `CARGO_INCREMENTAL=0`. `bin/protoc` is a dotslash wrapper and `dotslash` is not installed; `PROTOC=$HOME/.local/protoc-29.3/bin/protoc` is what made the build scripts run.

Hint tests were extended before the formatter accepted the new argument. That compile failed with 19× `E0061` (unexpected `resume_cmd` argument). After the parameter was wired through:

```
CARGO_TARGET_DIR=$HOME/repos/grok-build/target CARGO_INCREMENTAL=0 \
  PROTOC=$HOME/.local/protoc-29.3/bin/protoc \
  cargo test -p xai-grok-pager --lib -- \
  print_exit_resume_hint print_relaunch_failure_hint failed_relaunch_hint print_hints_survive
```

```
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 10248 filtered out; finished in 0.00s
```

```
CARGO_TARGET_DIR=$HOME/repos/grok-build/target CARGO_INCREMENTAL=0 \
  PROTOC=$HOME/.local/protoc-29.3/bin/protoc \
  cargo check -p xai-grok-pager
```

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4m 00s
```

Exit code 0. No pager warnings on that check.
