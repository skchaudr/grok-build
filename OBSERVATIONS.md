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

## Disk

The shared `target/` dir filled the 99 GB volume during the debug build (linker bus error, os error 28). Removed `target/debug/incremental` and `target/release/incremental` (rebuild caches, not the release binaries) to free about 13 GB. Tests were rerun with `CARGO_INCREMENTAL=0`.
