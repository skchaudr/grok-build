# /traces map

Read this before touching `/traces`. Paths are under `crates/codegen/`. `P` = `xai-grok-pager/src`.

## The feature

| Piece | Where |
|---|---|
| Index crate | `xai-grok-traces/src/{roots,normalize,lines,index,query,receipts,hook}.rs`. Tests in `tests/index.rs`. Real-data smoke: `cargo run --release -p xai-grok-traces --example scan -- <db>` |
| Hook CLI | `xai-grok-traces/src/bin/grok-traces.rs`: `hook <source>` (stdin JSON, always exit 0, `{}` for cursor), `scan`, `list --json`, `doctor` (exit 1 unless healthy). Field extraction and the receipt+index step are `hook::{extract,ingest}` |
| Slash command | `P/slash/commands/traces.rs`, registered in `P/slash/commands/mod.rs` `builtin_commands()`. Reserved in `xai-grok-shell/src/session/slash_commands.rs` `PAGER_COMMAND_KEYS` (test `pager_builtin_triggers_are_reserved_in_shell`) |
| Modal state, keys, render | `P/views/traces_modal.rs` |
| `ActiveModal::Traces` | `P/views/modal.rs` (variant, `hint_pairs`, `message`) |
| Key/mouse/render routing | `P/app/modals.rs`: the Traces branch before MemoryBrowser in `handle_modal_key_with_registry`, the mouse handler, `draw_active_modal`, and the `unreachable!` list |
| Actions | `P/app/actions.rs`: `OpenTraces`, `TracesRescan`, `TracesCopy`, `TracesNotice`, `TracesResume`, `Effect::ScanTraces`, `TaskResult::TracesScanned` |
| Dispatch | `P/app/dispatch/router.rs` (the arms beside `OpenMemoryModal`), `P/app/effects/mod.rs` `Effect::ScanTraces` (spawn_blocking scan), `P/app/dispatch/task_result.rs` `TracesScanned` |
| Resume | `P/app/dispatch/session/load.rs`: `dispatch_traces_resume` routes to `dispatch_foreign_resume`, shared with the session picker. Claude and Codex go through `/resume-claude` or `/resume-codex <id>`. Grok goes through `resolve_local_session_any_cwd` and then `dispatch_load_session` |
| pty proof | `xai-grok-pager-pty-harness/tests/pty_e2e/traces_modal_ledger_trajectory_inspector.rs` (family `pty_e2e_config_ui`) |
| User doc | `xai-grok-pager/docs/user-guide/04-slash-commands.md` `### /traces` |

## Data model (ported from bb-plugin-traces `indexer.ts`)

- **Roots** (`roots.rs`):
  - dsh `~/.dsh/sessions` (zstd)
  - claude `$CLAUDE_CONFIG_DIR|~/.claude/projects`
  - pi `~/.pi/agent/sessions`
  - omp `~/.omp/agent/sessions`
  - codex `$CODEX_HOME|~/.codex/{sessions,archived_sessions}`
  - grok `$GROK_HOME/sessions/**/chat_history.jsonl` (cwd from sibling `summary.json`)
  - Walk caps: 50k files, depth 32. Symlinks are skipped.
- **Session id:** `source:` + `sha1(source \0 path)[..24]`.
- **Normalize:** type is `top.type/payload.type`, where `payload` = `payload|data|message`.
  - Role: explicit role first, else from the type name.
  - Kind is one of telemetry, tool, step, turn, reasoning, message, system.
  - Depth 0–2.
  - Tokens come from `usage` / `info.total_token_usage` (the total for `token_count`).
  - Departures from the TS:
    - `content[]` tool blocks (`tool_use`, `tool_result`, `toolCall`, `toolResult`) set the tool role, so Claude and Pi tool counts are right.
    - An explicit `error` field or `is_error: true` marks an error.
    - User lines that start with `<` (injected context) are never titles.
- **Index** (`index.rs`):
  - Skip when size and mtime are unchanged. If the size matches but mtime changed, compare the full hash and only touch mtime.
  - Append from `indexed_bytes` when the sha256 of the 64KB before it (`content_hash`, `TAIL_WINDOW`) matches, so append cost does not grow with file size. Otherwise (shrink, tail changed) do a full reparse. zstd always reparses. An in-place edit further back than 64KB with unchanged size is not seen.
  - An unterminated tail line is never indexed.
  - `byte_offset` per event makes `raw_line` a seek.
  - A corrupt db is renamed `*.corrupt.<nanos>` and rebuilt. There is no `quick_check` on open, because it is O(db) and the open runs on the UI thread.
- **Status:**
  - `completed` if the last event type is terminal (complete, turn end, session end, run end).
  - `active` if not terminal and the file mtime is within 5 min.
  - Otherwise `idle`.
- **Storage diet:** events keep a `preview` (whitespace-collapsed, 160 chars), not the text. Titles come from the full text before clipping. The inspector re-reads the raw line via `byte_offset`; nothing is copied.
- **Versions:** `trace_meta` holds `schema_version` and `parser_version` (both 3). A mismatch on open renames the db and sidecars to `*.old.<nanos>` and deletes them, then starts fresh (a smaller row does not shrink a SQLite file).
- **Caps (fail loud):**
  - `MAX_DB_BYTES` = 256 MiB over db + `-wal`. `scan`, `index_file` and `record_receipt` call `check_cap()` first and return `Error::OverCap`, writing `trace_meta.health`. `TraceIndex::health()` is `None` when ok; the ledger shows it as a red line on the search row.
  - `MAX_EVENTS_PER_FILE` = 200k, recorded in `parse_error`.
  - Path guard: `roots::root_for` / `resolve` need the canonical path inside the root and a name that passes the root's rules. `.db`, `.sqlite`, `-wal`, `-shm`, `-journal` and `.corrupt.` names are rejected. `index_file` also rejects anything under the db's own directory.
- **Receipts:** `trace_receipts` (one row per hook event, pruned to 100k every 1000th insert). `record_receipt`, `receipts_watermark`, `last_receipt`. A receipt under 2 min old whose event is not terminal (`SessionEnd`, `Stop`, `stop`, `session_shutdown`, `agent_end`) makes a session Active regardless of mtime.
- **Live modal:** `event_loop.rs` arms `traces_poll_at` (1s, same pattern as `dashboard_poll_at`) while the active agent's modal is `Traces`; the arm calls `TracesModalState::poll_live`. It compares `receipts_watermark()`; on change it requeries the ledger keeping the selected session by id, and requeries an open trajectory only if that session's `event_count` grew (follows the bottom row, otherwise keeps selection). Header shows `● live` while the newest receipt is under 2 min old. `last_receipt_at()` backs it. Tests: `live_poll_*` in `traces_modal.rs`; the pty scenario runs `grok-traces hook claude` mid-ledger (binary: `GROK_TRACES_BIN` or next to the pager binary; build `-p xai-grok-traces --bin grok-traces` first).
- **Hook rules:** busy timeout 250 ms. Failures go to one stderr line; over-cap also posts a macOS notification at most hourly (stamp file `overcap.notified` beside the db). Grok's `updates.jsonl` path maps to the sibling `chat_history.jsonl`. No watcher or daemon exists; the hook is the only live feed.
- The index lives at `~/.grok/traces/traces.sqlite`. It is disposable and never uploaded.

## Pager rules learned

- The modal runs SQLite queries synchronously; the index makes them cheap. Live `/` search covers session columns only. Enter adds the event-content scan (`SessionFilter.deep`).
- The pty harness copy check reads `$GROK_COPY_FILE`. OSC52 is not emitted in that harness for this path.
- The first pty launch of a freshly built binary can time out on welcome (macOS first exec). Rerun before debugging.
- The test baseline on main 7eb90d57 is pager lib 10389/0 and shell lib 6 failures (with `RUST_MIN_STACK=33554432`; without it the run aborts on stack overflow). `GROK_HOME` must be empty for pager tests.
- The binary comes from `-p xai-grok-pager-bin` (bin `xai-grok-pager`). `--version` prints `grok <ver> (<12-char HEAD sha>)`.
