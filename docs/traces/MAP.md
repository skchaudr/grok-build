# /traces map

Read this before touching `/traces`. Paths are under `crates/codegen/`. `P` = `xai-grok-pager/src`.

## The feature

| Piece | Where |
|---|---|
| Index crate | `xai-grok-traces/src/{roots,normalize,classify,links,lines,index,query,receipts,hook}.rs`. Tests in `tests/{index,legibility,bb,cursor}.rs`. Real-data smoke: `cargo run --release -p xai-grok-traces --example scan -- <db>` |
| Hook CLI | `xai-grok-traces/src/bin/grok-traces.rs`: `hook <source>` (stdin JSON, always exit 0, `{}` for cursor), `scan`, `list --json`, `doctor [--reset]` (exit 1 unless healthy; `--reset` clears a tripwire). Field extraction and the receipt+index step are `hook::{extract,ingest}` |
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
  - cursor `~/.cursor/projects/*/agent-transcripts/*/*.jsonl` (user text unwrapped from `<user_query>`)
  - bb `~/.bb/bb.db` threads, read in place (`bb.rs`, `index/bb_index.rs`; pointer = thread id + event sequence; see `BB.md`)
  - grok `$GROK_HOME/sessions/**/chat_history.jsonl` (cwd from sibling `summary.json`)
  - Walk caps: 50k files, depth 32. Symlinks are skipped.
- **Session id:** `source:` + `sha1(source \0 path)[..24]` (API key). Tables use the integer `trace_sessions.sk`.
- **Normalize:** type is `top.type/payload.type`, where `payload` = `payload|data|message`.
  - Role: explicit role first, else from the type name.
  - Kind is one of telemetry, tool, step, turn, reasoning, message, system.
  - Depth 0–2.
  - Tokens come from `usage` / `info.total_token_usage` (the total for `token_count`).
  - Departures from the TS:
    - `content[]` tool blocks (`tool_use`, `tool_result`, `toolCall`, `toolResult`) set the tool role, so Claude and Pi tool counts are right.
    - Errors come from structure only (`classify::structured_error`): `is_error`/`isError`/`isApiErrorMessage` on the record, its payload or a `tool_result` block (never inside a tool's input), a non-empty `error` field, `stopReason: error`, turn `status: error`, a record type ending in `error`/`failed`, and for Codex the exit header (`Process exited with code N`, `Exit code: N`, `Script failed`). Never from words in output. A user abort and DSH `TOOL_OUTCOME_UNKNOWN` are not errors. Mini real data: 3201 -> 801 error events.
    - `noise` (bookkeeping, telemetry, injected context) is `classify::noise_type` (lists per harness), telemetry kind, role developer/system, empty reasoning, and user turns that are `isMeta`/`isCompactSummary`/`synthetic_reason`, entirely `<system-reminder>`/`<local-command-*>`/`<command-name>`/etc. wrappers (`strip_injected`, only at line start so prose is kept), or a known harness prompt (`harness_prompt`: "### Session update", subagent-completion nudges). Wrappers are stripped from previews and titles. The default trajectory hides noise (`EventFilter.include_noise`).
    - `detail`: tool call -> Bash command (first line), file path (`parent/base`), pattern, Agent description, `server.tool` for MCP; tool result -> `exit N` or the first error line. Claude/Pi `content[]` blocks, OpenAI-style `tool_calls` (Grok), flat Codex/DSH calls. Stored in `trace_events.detail`; a call's raw-argument preview is not kept.
    - Harness-recorded titles win over the first user message: Claude `ai-title`, OMP `title`/`title_change`, DSH `session/title`.
- **Index** (`index.rs`):
  - Skip when size and mtime are unchanged. If the size matches but mtime changed, compare the full hash and only touch mtime.
  - Append from `indexed_bytes` when the sha256 of the 64KB before it (`content_hash`, `TAIL_WINDOW`) matches, so append cost does not grow with file size. Otherwise (shrink, tail changed) do a full reparse. zstd always reparses. An in-place edit further back than 64KB with unchanged size is not seen.
  - An unterminated tail line is never indexed.
  - `byte_offset` is kept on every 16th line (`CHECKPOINT_EVERY`); `raw_line` seeks to the nearest earlier checkpoint and scans on.
  - A corrupt db is renamed `*.corrupt.<nanos>` and rebuilt. There is no `quick_check` on open, because it is O(db) and the open runs on the UI thread.
- **Status:**
  - `completed` if the last event type is terminal (complete, turn end, session end, run end).
  - `active` if not terminal and the file mtime is within 5 min.
  - Otherwise `idle`.
- **Storage diet:** events keep a `preview` (whitespace-collapsed, 100 chars), not the text. Row layout (schema 4):
  - `trace_events` is `WITHOUT ROWID`, `PRIMARY KEY (sk, line_number)`; no text id, no extra index, no per-event cwd.
  - `kind`/`role` are `Kind::code`/`Role::code`. Event type, title and model are interned in `trace_names` (rows hold ids).
  - `preview` is `''` when it equals the title (read back as the title).
  - Duration, tokens, turn and step live in `trace_event_usage`, only for events that have them.
  - Schema 6 adds `noise`, `detail` (events) and `native_id`, `parent_native`, `label`, `agent_type`, `done`, `signal_count`, `now_text`, `last_event_at`, `failed` (sessions), `aux_sig` (files).
  - Measured on the Mini's real data (608 files, 102k events): 338 -> 122 B/event all-in at schema 5; schema 6 on the same data: 9.7 MB / 102k events = 95 B/event (schema 5: 9.9 MB, 97 B), with 49% of events noise.
  - Scan order: all roots are discovered first, then files are indexed newest mtime first across roots, bb threads by `updated_at`. If the cap is hit, the oldest sessions are the ones missing. Titles come from the full text before clipping. The inspector re-reads the raw line via `byte_offset`; nothing is copied.
- **Dispatch tree** (`links.rs`): a session stores `native_id` and `parent_native`; `SessionRow.parent_id` and `children` resolve them inside one source at query time (indexes on `(source_id, native_id)` and `(source_id, parent_native)`), so scan order does not matter and an evicted parent leaves a root. `roots_only` = no resolvable parent. Found per harness:
  - claude: `<session>/subagents/agent-*.jsonl` -> `<session>.jsonl`; sibling `agent-*.meta.json` gives `description` (label), `agentType`, `finished` (-> status done; `error`/`status: failed` -> failed). The meta's size+mtime is `trace_files.aux_sig`, so a meta change refreshes without reparsing the transcript. No error field exists in today's meta files.
  - codex: `session_meta` `payload.parent_thread_id` / `source.subagent.thread_spawn.parent_thread_id`; label = `agent_path` leaf or nickname, type = `agent_role`. (`forked_from_id` alone is a fork, not a dispatch.)
  - omp: task subagents live at `<cwd>/<parent stem>/<Name>.jsonl` (parent = the sibling `<parent stem>.jsonl`, label = `<Name>`); `session.parentSession` (a path) is a branch. pi: no parent field.
  - dsh: header `parentSession` (an id; `seedLength` = fork). grok: `summary.json` `parent_session_id` when `session_kind` starts with `subagent` (none on the Mini yet); other kinds (fork, headless) are not dispatches. bb: `threads.parent_thread_id`. cursor: no layout for subagents.
- **Session signals** (computed at index time, no scan of events at query time): `now_text` (last `Tool: detail` or assistant text), `last_event_at` (non-noise), `signal_count` (drives `hide_trivial`: <3 non-noise events and 0 tools), `failed` (the last non-noise event is an error, so nothing followed it; cleared by any later event; bb status `error`; sidecar finished-with-error).
- **Versions:** `trace_meta` holds `schema_version` and `parser_version` (parser 5, schema 6). An older index on open is replaced: `rebuild` takes `<db>.rebuild.lock` (exclusive create; stale after 60 s), re-probes under it, renames the db and sidecars to `*.old.<nanos>` and deletes them, then builds fresh. A process that loses the race waits at most 2 s, then returns an error (hook: one stderr line, exit 0; it never blocks or builds a second db). Creating a new db and quarantining a corrupt one use the same lock. An index from a NEWER version is never replaced (open errors), so two binaries cannot rebuild each other.
- **Size (budget, not cap):**
  - `SIZE_BUDGET` = 256 MiB over db + `-wal`. Passing it is normal and silent: `check_writable()` (start of `scan`, `index_file`, bb thread, `record_receipt`; two stats when under budget) and the end of `scan` call `evict_if_over()`. It deletes the oldest sessions by `updated_at` (events, usage, receipts, file rows) until live pages are under 85%, then `incremental_vacuum` + `wal_checkpoint(TRUNCATE)`. The db is created `auto_vacuum=INCREMENTAL`. No health change, no alert.
  - Evicted files stay out: `trace_meta.evict_floor_ms` = newest mtime among evicted files; a file not already in the index with mtime <= floor is skipped (files and bb threads). A file that changes later has a newer mtime and returns. The floor never drops (the index is a cache; it will not backfill old sessions).
  - Scan is newest-first, so once the budget is spent, evicting trims the oldest of the prefix and the floor skips the rest.
- **Tripwires (fail loud):** health red (`trace_meta.tripped`, sticky, persists across opens), writes refused with `Error::Tripped`, `doctor` exit 1, macOS notification at most hourly from the hook (`tripped.notified`). `grok-traces doctor --reset` (`reset_trip`) clears it and the growth baseline, then evicts if over budget. Thresholds are `Limits` (`TraceIndex::open_with_limits`; tests use small ones).
  - Hard ceiling `HARD_CEILING` = 2x budget: still above it after an eviction attempt.
  - Growth rate: `trace_meta.growth_samples` (`ts:gross,...`, kept 1h, taken at scan end and every 100th receipt, >=60s apart). Gross = db+wal + `evicted_bytes`, so a loop that is evicted as fast as it writes still counts. More than `GROWTH_LIMIT` (128 MiB) above the smallest sample in the hour trips. Nothing is sampled until `trace_meta.built` is set, at the end of the first full scan (no root hit the file cap) of this db; that scan records the baseline. Hook receipts before then neither sample nor trip, and a rebuilt db starts unbuilt again.
  - Event flood: more than `MAX_CAPPED_FILES` (5) files hit `MAX_EVENTS_PER_FILE` in one scan (`ScanReport.tripped`).
  - `MAX_EVENTS_PER_FILE` = 200k, recorded in `parse_error` (the file is still indexed to the cap).
  - Path guard: `roots::root_for` / `resolve` need the canonical path inside the root and a name that passes the root's rules. `.db`, `.sqlite`, `-wal`, `-shm`, `-journal` and `.corrupt.` names are rejected. `index_file` also rejects anything under the db's own directory.
- **Receipts:** `trace_receipts` (one row per hook event, pruned to 100k every 1000th insert). `record_receipt`, `receipts_watermark`, `last_receipt`. A receipt under 2 min old whose event is not terminal (`SessionEnd`, `Stop`, `stop`, `session_shutdown`, `agent_end`) makes a session Active regardless of mtime.
- **Live modal:** `event_loop.rs` arms `traces_poll_at` (1s, same pattern as `dashboard_poll_at`) while the active agent's modal is `Traces`; the arm calls `TracesModalState::poll_live`. It compares `receipts_watermark()`; on change it requeries the ledger keeping the selected session by id, and requeries an open trajectory only if that session's `event_count` grew (follows the bottom row, otherwise keeps selection). Header shows `● live` while the newest receipt is under 2 min old. `last_receipt_at()` backs it. Tests: `live_poll_*` in `traces_modal.rs`; the pty scenario runs `grok-traces hook claude` mid-ledger (binary: `GROK_TRACES_BIN` or next to the pager binary; build `-p xai-grok-traces --bin grok-traces` first).
- **Hook rules:** busy timeout 250 ms. Failures go to one stderr line; a tripped index posts a macOS notification at most hourly (stamp file `tripped.notified` beside the db). Grok's `updates.jsonl` path maps to the sibling `chat_history.jsonl`. No watcher or daemon exists; the hook is the only live feed.
- The index lives at `~/.grok/traces/traces.sqlite`. It is disposable and never uploaded.

## Pager rules learned

- The modal runs SQLite queries synchronously; the index makes them cheap. Live `/` search covers session columns only. Enter adds the event-content scan (`SessionFilter.deep`).
- The pty harness copy check reads `$GROK_COPY_FILE`. OSC52 is not emitted in that harness for this path.
- The first pty launch of a freshly built binary can time out on welcome (macOS first exec). Rerun before debugging.
- The test baseline on main 7eb90d57 is pager lib 10389/0 and shell lib 6 failures (with `RUST_MIN_STACK=33554432`; without it the run aborts on stack overflow). `GROK_HOME` must be empty for pager tests.
- The binary comes from `-p xai-grok-pager-bin` (bin `xai-grok-pager`). `--version` prints `grok <ver> (<12-char HEAD sha>)`.

## Hooks

Per-harness wiring so sessions index as they happen. Templates and installer: `xai-grok-traces/hooks/`. Every path calls `~/.grok/bin/grok-traces hook <source>` (one JSON payload on stdin, exit 0, silent).

| Harness | Mechanism | Installed to |
|---|---|---|
| claude | `settings.json` hooks: SessionStart, UserPromptSubmit, Pre/PostToolUse, PostToolUseFailure, Stop, SubagentStop, SessionEnd | edited in place via jq |
| codex | `hooks.json`, same list minus SessionEnd (Codex has none) | edited in place via jq |
| grok | `hooks/grok/traces.json` (async): SessionStart, UserPromptSubmit, PostToolUse, Stop, SessionEnd | `~/.grok/hooks/traces.json` |
| omp, pi | `hooks/{omp,pi}/traces.ts` extension, tool_result debounced 500ms with a trailing spawn | `~/.omp/agent/extensions/`, `~/.pi/agent/extensions/` |
| dsh | `hooks/dsh/traces.mjs` Cordis plugin on `session/event`, throttled 1s | `install.sh --dsh-profile NAME`: copied to `~/.dsh/plugins/grok-traces.mjs`, mounted from that profile's `cordis.patch.yml` |
| cursor | `hooks.json` (flat, `version: 1`): afterAgentResponse, postToolUse, postToolUseFailure, stop. Gating events are not wired | edited in place via jq |
| bb | `hooks/bb/` plugin (`bb.events.on` thread.created/active/idle/failed/archived), throttled 1s per thread | `bb plugin install ~/.grok/hooks/bb`, only on the host running the bb server |

- `install.sh`: `--dry-run` (default, prints `diff -u`), `--apply`, `--remove`, `--check`. `--apply` refuses while the binary is missing (`--even-if-missing` overrides). JSON files get a `.bak-traces-<ts>` backup, are built in a temp file, and are validated with `jq empty` before the move.
- Claude/Codex: removes hook entries containing `log-commands.sh`, plus any previous grok-traces entry, then appends one per event. Re-runs are idempotent. Atuin, Moshi, Orca, Herdr and other hooks are untouched.
- `pi-observability.ts` (OMP symlink) is only moved aside with `--apply --retire-pi-observability`. `--remove --retire-pi-observability` puts it back.
- Grok's hook payload carries `transcriptPath`/`transcript_path`, but it is `<session dir>/updates.jsonl` (set only if the file exists), not `chat_history.jsonl`. The session dir is the same, so the trace side should map it to the sibling `chat_history.jsonl`.
- Cursor payload (moshi-hook and Orca relay as evidence): `hook_event_name`, `conversation_id`, `generation_id`, `workspace_roots`, `transcript_path` (path to `agent-transcripts/<id>/<id>.jsonl`; may be null/absent), plus per-event fields (`text` on afterAgentResponse, `tool_name`/`tool_input` on tool events, `status` on stop). All of event/session/path/cwd are already in `hook.rs` key lists.
- Cursor stdout contract (Orca's table): `{}` is the reply for afterAgentResponse, postToolUse, postToolUseFailure, stop. beforeSubmitPrompt needs `{"continue":true}`, preToolUse/beforeShellExecution/beforeMCPExecution need `{"permission":"allow"}`. The crate prints `{}` for every cursor event, so those gating events stay unwired until it prints per event (the installer comment says the same).
- bb: the plugin API is `export default async function(bb)` with `bb.events.on(name, ({thread}) => ...)`; handlers are fire-and-forget. It spawns `grok-traces hook bb` with `{"event","thread_id"}`. Install/remove go through `bb plugin install|remove` (id `grok-traces-hook`). A bb CLI whose `~/.bb/config.json` has a `serverUrl` (the Mini's points at the Air) is a client, so the installer skips it: run it on the Air.
- DSH: the block is wrapped in `# grok-traces begin/end` comments, injected into the profile's existing top-level `- insert:` list or appended as a new one; `--remove` deletes the block (byte-identical restore) and the plugin copy once no profile uses it. Profiles are git-synced between Macs, so the plugin file must be installed on both before a synced patch is used. Profiles in use: `acp-enhanced` (gb `dsh`, khoj worker), `acp` (gb `dsh-stock`), `web`, `tui`.
- DSH logs are `~/.dsh/sessions/<project-key>/<session id>/session[.vN].jsonl.zstd`. The plugin finds the file by scanning project dirs for the session id.
- Tests: `hooks/test.sh` (synthetic `HOME`, includes the JS templates under node >= 22). Real-config dry-runs: `docs/traces/HOOKS-dryrun-mini.txt`, and for cursor/dsh/bb on both Macs `docs/traces/HOOKS-dryrun-wire.txt`.
