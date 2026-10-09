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
||||||| e6568d65
## 2026-10-09 external roster publish

`bin/protoc` needs `dotslash`, which this VM does not have. Tests and the release build used `PROTOC=$HOME/.local/protoc-29.3/bin/protoc` and that directory on `PATH`. `CARGO_TARGET_DIR=$HOME/repos/grok-build/target`.

The debug compile filled the volume. `rm -rf $HOME/repos/grok-build/target/release/incremental` freed about 11 GB. Release incremental is a rebuild cache.

`cargo test -p xai-grok-shell --lib leader::` — 293 passed, 0 failed.
`cargo test -p xai-grok-shell --test test_leader_external_agents` — 8 passed.

`-p xai-grok-pager` builds the library. The pager binary is package `xai-grok-pager-bin` (artifact name still `xai-grok-pager`). `cargo build --release -p xai-grok-pager-bin` produced `$HOME/repos/grok-build/target/release/xai-grok-pager`, `grok 1.0.45 (240a0045b45e) [stable]`, mtime 2026-10-09 02:22:04.

Live probe, tmux session `roster-probe`, socket `$HOME/.grok/leader-roster-test.sock` (torn down after):

```
$BIN agent leader --leader-socket $SOCK --no-auto-update --no-exit-on-disconnect --debug-file /tmp/roster-probe/leader.log
$BIN --leader --leader-socket $SOCK --cwd /tmp/ext-roster-live --always-approve
$BIN --leader --leader-socket $SOCK --cwd /tmp/ext-roster-live --always-approve --agent-cmd 'cursor-agent acp'
```

Client B prompt `reply with the single word pong` returned `pong` (Worked for 2.7s). Leader log: `spawned external ACP backend cmd=cursor-agent acp pid=192840`. Native Grok relay auth failed; left as-is.

A direct ACP `sessions/list` on the test socket (length-prefixed `register` then `_x.ai/sessions/list`) returned 66 sessions, one external:

`sessionId=abf16d53-dc97-4da2-bb86-fd1ace76f2c3 title=cursor-agent cwd=/tmp/ext-roster-live sessionKind=external activity=idle resident=true`.

Remote settings turn on the workspace dashboard. That view (`Open Previous /resume`, `Idle N`) does not read `leader_roster`, so the first `Ctrl+\` on A did not show the Cursor row. `GROK_WORKSPACE_DASHBOARD=0` on a fresh native client opens the fleet dashboard. Inactive starts collapsed (`Idle` roster activity maps to `Inactive`). Expanding it showed `◇ cursor-agent` (12m) under `▾ Inactive 40`.

Opening that row on A loaded B's transcript (`pong`). Prompt from A, `reply with the single word roster-bridge`, returned `roster-bridge` on A (Worked for 3.3s). B's pane updated at the same time; the capture drew the new reply against the previous `pong` line as `pongroster-bridge`. The prompt reached the Cursor agent.

Follow-up, not done: `session/prompt` does not flip the row between working and idle. `session/close` while the client stays connected does not drop the route, so the row stays until the last subscriber disconnects or the backend process exits. Workspace-dashboard mode still ignores leader roster rows.

## 2026-10-09 roster hardening

`df` before the builds: 6.8G free on `/`. `target/release/incremental` and `target/debug/incremental` were each 4K, so nothing was removed.

`RosterActivity::Idle` means resident and no turn in flight. The pager's `roster_activity_to_state` maps both `Idle` and `Dormant` to `RowState::Inactive`, and no activity value maps to `RowState::Idle`, so the leader cannot ask for the Idle group by itself. The fleet row builder now paints `RowState::Idle` only when the row is `resident`, `activity == idle`, and `session_kind == "external"`. A native idle row stays Inactive.

`session/prompt` routed to an external backend sets that row to working and broadcasts it. The prompt response, including an error, sets it back to idle. `session/cancel` sets it back to idle immediately. `session/close` and a routed `session/delete` / `x.ai/session/delete` (bare or `_x.ai` wrapped) drop the row while the client stays connected. Native prompts and native sessions still publish nothing.

`cargo test -p xai-grok-shell --lib leader::` — 297 passed, 0 failed.
`cargo test -p xai-grok-pager --lib resident_external_idle_row_renders_idle` — 1 passed.

`cargo build --release -p xai-grok-pager-bin` finished in 5m 26s. The binary reports `grok 1.0.45 (20c62ebbb360)` because the version stamp was taken before commit `19b78e4d`; the compile included the pager and shell changes from that commit.

Live probe, same socket `$HOME/.grok/leader-roster-test.sock`, torn down after. Native client A was started with `GROK_WORKSPACE_DASHBOARD=0`. Client B was `--agent-cmd 'cursor-agent acp'`. `sessions/list` returned `caef6db9-7ae6-474f-aaec-ea9f0ed11eb7`, title `cursor-agent`, idle, resident, `sessionKind=external`. A's fleet dashboard showed `▾ Idle 1` / `◇ cursor-agent` above a collapsed `▸ Inactive 39`. B's prompt `Run sleep 4 in the shell, then reply with the single word bridge` moved the row to `▾ Working 1` for the whole turn, then back to Idle. B printed `bridge` (Worked for 10s). A third client sent `session/close` for that id while B's pager stayed up. cursor-agent replied `-32601` `"Method not found": session/close`. The row was already removed on route: the dashboard dropped to `Inactive 39` only, and `sessions/list` had no external row.

Fleet dashboard switch: `GROK_WORKSPACE_DASHBOARD=0` (also `false` / `off` / `no` / `disabled`). `app/event_loop.rs` reads that env first, then remote settings `workspace_dashboard_enabled`, then `false`. There is no `config.toml` key. The remote field is `RemoteSettings.workspace_dashboard_enabled` in `xai-grok-config`. Workspace-dashboard code was not changed; that view still ignores `leader_roster`.

## 2026-10-09 external prompt ack

External ACP agents never stamp a prompt id and never send `x.ai/queue/changed`, so the 120s ack watch aborted turns that were already streaming. A session is treated as external when this pager was started with `--agent-cmd`, or when the leader roster row for that session has `sessionKind: external` (a dashboard open of another client's external session). The first live non-replay `session/update` disarms the watch. A replay does not. A native session still requires the stamped prompt id, and an unstamped update still expires the watch. Headless uses the same rule through `HeadlessOptions.external_agent`. The CLI still rejects `--agent-cmd` in headless mode, so that flag stays false on the `-p` path.

`df` before the builds: 6.6G free on `/`. `target/release/incremental` was 4K. Builds used `CARGO_TARGET_DIR=$HOME/repos/grok-build/target`, `CARGO_INCREMENTAL=0`, and `PROTOC=$HOME/.local/protoc-29.3/bin/protoc`.

Failing tests first (watch still armed / headless classifier returned `None`), then the same filters after the disarm:

```
CARGO_TARGET_DIR=$HOME/repos/grok-build/target CARGO_INCREMENTAL=0 \
  PROTOC=$HOME/.local/protoc-29.3/bin/protoc \
  cargo test -p xai-grok-pager --lib -- \
  external_unstamped_update_disarms native_unstamped_update_leaves \
  external_replay_update_does_not leader_attached_external_session \
  headless_external_unstamped headless_ack_signal_classifies prompt_ack
```

```
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 10251 filtered out; finished in 0.20s
```

`disarms_the_child` and `disarms_the_watch` on the same binary: 6 passed.

Live, tmux, `GROK_PROMPT_ACK_TIMEOUT_SECS=10`, `--agent-cmd 'cursor-agent acp' --always-approve --trust --no-alt-screen --no-auto-update`, prompt `Run sleep 30 in the shell, then reply done`. Both tmux sessions were killed after the capture. `~/.grok/bin` was not touched.

Before: copied `$HOME/repos/grok-build/target/release/xai-grok-pager` (mtime 2026-10-09 04:26, before this release link) to `/tmp/xai-grok-pager-before-ack`. cwd `/tmp/epa-before`, tmux `epa-before`. The pane showed `◆ Run sleep 30` and `I'll run sleep 30 and reply once it finishes.`, then at 12.8s after send:

`The agent did not accept your prompt within 10s. The turn was stopped; send the prompt again to retry.`

`Turn cancelled.` Toast: `Prompt not accepted, turn stopped`.

After: `cargo build --release -p xai-grok-pager-bin` finished in 2m 30s. Binary `$HOME/repos/grok-build/target/release/xai-grok-pager`, mtime 2026-10-09 04:33, commit `a73fac48`. cwd `/tmp/epa-after`, tmux `epa-after`. At about 22s the pane still showed `⠴ Run sleep 30 22s`. The turn ended with `Done.` and `Worked for 35s` (39.3s after send). The timeout notice was not on the pane.
||||||| 19a5f94e
## 2026-10-09 external context indicator (before code)

The token readout the user means is the top status-bar row (`AgentViewLayout` lays `status_bar` down first). `draw` pushes `context_bar_line_for_session` into that row. Default text is `used / total` (`fmt_tokens`), which is the `54K / 500K` shape. Hover swaps in a bar and a percentage. Gateway/chat sessions suppress it.

Two inputs fill `context_state`:

- `session/update` with `sessionUpdate: "usage_update"` (`UsageUpdate`, feature `unstable_session_usage`): `used` and `size` go to `apply_context_used`.
- `_meta.totalTokens` on any session notification: `confirm_context_used` stores `used` and takes the denominator from the current model's `meta.totalContextTokens` (`get_context_window`). Missing window becomes `0`.

`session/prompt` `PromptResponse.usage` (`totalTokens` / `inputTokens` / `outputTokens`) is deserialized and then ignored. The boxed prompt border (`chrome: true`, session title on `╭─╮`, model on `╰─╯`) has no token slot. Compact mode (`appearance.prompt.compact`) only drops padding and the prompt gap. It does not gate the status-bar readout. The earlier note that only compact chrome reads `token_usage` is stale: both chromes call the same `draw` path. A missing numerator, or a numerator with total `0` and no model window, makes `context_bar_line_for_session` return `None`, so the slot is absent.

`claude-code-acp` 0.16.2 is `~/.local/lib/node_modules/@zed-industries/claude-code-acp` (`dist/acp-agent.js`). npm latest is the same version. It has no usage hook. `streamEventToAcpNotifications` returns `[]` for `message_start` and `message_delta` (where the SDK puts per-message usage). The `result` arm returns `{ stopReason }` and drops `message.usage` and `message.modelUsage` (`contextWindow` lives on that SDK type). `/context` text that contains `Context Usage` is forwarded as a normal agent message, not as `usage_update`.

Live capture, one short turn each, probe in `/tmp/ctx-probe` (not committed). Prompt: `Reply with the single word pong. Do not use tools.`

Claude (`CLAUDE_CODE_EXECUTABLE=$(command -v claude) claude-code-acp`), session `72c10895-4b90-4f41-9860-3b56e3277fd7`:

- `session/new` models have `modelId` / `name` / `description` only. No `totalContextTokens`, no `_meta`.
- Updates: `available_commands_update`, then `agent_message_chunk` text `""`, `"p"`, `"ong"`. No `_meta`, no `usage_update`.
- Prompt result: `{"stopReason":"end_turn"}`.
- stderr: adapter logged `Unexpected case` for an SDK `rate_limit_event` (utilization fractions, not a context count) and did not put it on the wire.

The Claude transcript on disk (`~/.claude/projects/-tmp-ctx-probe/72c10895-….jsonl`) does have assistant `message.usage`: `input_tokens` 2, `cache_creation_input_tokens` 26347, `cache_read_input_tokens` 0, `output_tokens` 4. `cost-state.modelUsage` has no `contextWindow`. That file is not ACP traffic. The pager does not read it, and these numbers are not invented into the header.

Cursor (`cursor-agent acp`), session `c2551294-f753-4aaf-a658-877ba10d5ee7`:

- Updates: `available_commands_update`, `agent_thought_chunk`, `agent_message_chunk` `"pong"`, `session_info_update` `{"title":"Pong Game"}`. No `_meta`, no `usage_update`.
- Prompt result: `{"stopReason":"end_turn"}`.
- `session/new` model ids embed a window in the id string, e.g. `grok-4.7[context=256k,reasoning_effort=high,fast=true]`. There is no used-token field. That string is not a numerator.

Neither agent sent a usable context count. The header stays empty until one of them sends `usage_update`, `_meta.totalTokens`, or `PromptResponse.usage`. A window parsed out of a Cursor model id, with no used count, would be a fake denominator and is not shown.

## 2026-10-09 external context indicator (after the pager change)

`df` before the release build: 6.6G free on `/`. No other `cargo` was running. `CARGO_INCREMENTAL=0`, `CARGO_TARGET_DIR=$HOME/repos/grok-build/target`, `PROTOC=$HOME/.local/protoc-29.3/bin/protoc`.

`cargo test -p xai-grok-pager --lib` filtered to the new cases: 9 passed, 0 failed. The four new assertions failed first (context bar and both headers omitted `54K`; prompt-response `usage` and `_meta.totalTokens` were ignored). The captured `{"stopReason":"end_turn"}` case passed before the change and still passes.

`cargo build --release -p xai-grok-pager-bin` finished in 11m 01s. Binary `$HOME/repos/grok-build/target/release/xai-grok-pager`, `grok 1.0.45 (9343c5af84ed) [stable]`.

Live, tmux `ctx-live` 140×40, then torn down:

```
$BIN --cwd /tmp/ctx-probe --trust --always-approve --no-auto-update \
  --agent-cmd 'env CLAUDE_CODE_EXECUTABLE=$HOME/.local/bin/claude claude-code-acp'
```

Prompt `Reply with the single word pong. Do not use tools.` returned `pong` (Worked for 1.0s). The default header row was:

` /tmp/ctx-probe                                                                                                                 [Dashboard]`

No used count and no ` / ` denominator. The boxed prompt chrome was the one on screen (`╭─ ❯ ─╮`, `Opus 5.5 · always-approve`). Compact mode was not on. That matches the wire capture: this adapter still sends `{"stopReason":"end_turn"}` and nothing else, so the pager has no number to print.

## 2026-10-09 roster titles for external sessions

`external_agent_title` only kept the executable basename, so `ssh … user@host /path/grok-khoj-worker` was `ssh`. The fallback is now `host: script` (user stripped at the last `@`, last non-flag remote token's basename). The live khoj command therefore reads `100.75.255.75: grok-khoj-worker`, not a nickname. `<bin> agent … stdio` stays the bin name (`grok-team-hub`).

An ACP `session_info_update` title replaces that label, including one that arrives before the roster row exists. Otherwise the first `session/prompt` text is used: first line, whitespace collapsed, 60 characters. A later prompt does not replace it. Cursor's captured update `{"sessionUpdate":"session_info_update","title":"Pong Game"}` is the shape the row follows.

An unanswered external `session/new` is published immediately as `pending:{namespaced request id}` so a hung start is visible. That row is removed when the owning client disconnects, when `session/new` returns an error, or when that in-flight request's process exits. A success that arrives after the client is gone does not put the row back.

Publishing that provisional row makes `_x.ai/sessions/changed` show up on the external client before `session/new`'s result. The changed line is injected on the native response channel and then broadcast to every client. The second-session helper already skipped those lines; the first-session helper had to as well.

`cargo test -p xai-grok-shell --lib leader::` : 305 passed. `df` was not tight enough to stop; nothing was deleted.

## 2026-10-09 cross-machine delegation

Stock `grok` 1.0.50 (`~/.grok/bin/grok`) has no `--agent-cmd`. `grok leader` is `list` / `info` / `kill`. `grok agent` is `stdio`, `headless`, `serve`, `leader`. This branch's debug `xai-grok-pager` accepts `--agent-cmd` and then refuses the one-shot:

```
$ xai-grok-pager -p hi --leader --leader-socket /tmp/delegate-does-not-exist.sock --agent-cmd 'python3 -c true'
grok: --agent-cmd is only supported in interactive mode, not headless mode
```

A leader started with the test helper's default (`no_exit_on_disconnect = false`) unlinks the socket when the readiness probe disconnects. `scripts/leader_delegate.py` then fails with `FileNotFoundError` on connect. `spawn_leader_server_persistent` and `grok agent leader --no-exit-on-disconnect` stay up. External clients are marked ready even when the native backend is not.

`ssh` is spawned by shlex, then OpenSSH joins argv for the remote shell. A `-c` script whose quotes were consumed by shlex is word-split, and zsh globs the parentheses (`no matches found: exec(base64...)`). The argv that reaches `ssh` has to still contain the remote single quotes.

Live private leader, khoj `khoj-38w`, binary `target/debug/xai-grok-pager` (debug, not release). Mini hub was not restarted. Worker B was `ssh -T -o BatchMode=yes sab-mini@100.66.99.64 env WORKER_MODE=hostname python3 -u -c '<base64 of scripts/delegate_acp_worker.py>'` (no file written on the Mini). Worker A was `env DELEGATE_SOCKET=... DELEGATE_TARGET=<that ssh command> python3 scripts/delegate_acp_worker.py delegate`.

```
$ python3 scripts/leader_delegate.py --leader-socket /tmp/delegate-spike.sock \
    --agent-cmd "$A_CMD" --prompt "run hostname and report it" --timeout 90
worker-a delegated; worker-b said: hostname=sab-mini
```

Debug log: `Leader server listening`; client 1 spawned the local delegate worker (pid 183875); client 2 spawned the ssh command (pid 183877). `df` after the debug pager build: 13G free. Nothing deleted.

Live follow-up, same day: `grok-team-client -p … --leader --leader-socket … --agent-cmd …` prints `grok: --agent-cmd is only supported in interactive mode, not headless mode`. That is the core gap. `~/.dsh/scripts/gk-smoke --deep` (`deep_run`) is the caller: from Air it runs that `grok-team-client -p` against `~/.grok/leader-mini-hub.sock` for the mini and khoj workers. When the delegate one-shot lands, `--deep` should switch to it.

Tests, both ok:

```
cargo test -p xai-grok-shell --features test-support --test test_leader_external_agents delegate_cli_prints_the_named_workers_final_answer
cargo test -p xai-grok-shell --features test-support --test test_leader_external_agents session_on_worker_a_delegates_to_worker_b
```
