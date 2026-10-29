# /traces map

Read this before touching `/traces`. Paths are under `crates/codegen/`. `P` = `xai-grok-pager/src`.

## The feature

| Piece | Where |
|---|---|
| Index crate | `xai-grok-traces/src/{roots,normalize,classify,links,lines,index,query,receipts,hook}.rs`. Tests in `tests/{index,legibility,bb,cursor}.rs`. Real-data smoke: `cargo run --release -p xai-grok-traces --example scan -- <db>` |
| Hook CLI | `xai-grok-traces/src/bin/grok-traces.rs`: `hook <source>` (stdin JSON, always exit 0, `{}` for cursor), `scan`, `list --json`, `doctor [--reset]` (exit 1 unless healthy; `--reset` clears a tripwire). Field extraction and the receipt+index step are `hook::{extract,ingest}` |
| Follow | `grok-traces follow [<id>] [--no-children] [--json] [--once] [--backlog N]`: `tail -f` for live sessions (default: newest active root), plain stdout, children indented under their parent. With no live session (or `<id>` not live yet) it prints `… waiting for an agent` / `… waiting for session <id>` to stderr and attaches when one appears; when the followed root ends it prints the end line and waits for the next agent. `--once` exits on the first end instead (and exits 1 at once if nothing is live). `--json`: one item per line (`user text reasoning call result model`, plus `agent_start agent_end agent_resume` for subagents and `agent_end` for the root), no header or idle line; `--backlog` sets the rows shown at start (default 10). Pure formatters and the poll loop are `xai-grok-traces/src/follow.rs` (`format_event`, `Follower::tick`); tests there and in `tests/follow.rs` |
| Slash command | `P/slash/commands/traces.rs`, registered in `P/slash/commands/mod.rs` `builtin_commands()`. Reserved in `xai-grok-shell/src/session/slash_commands.rs` `PAGER_COMMAND_KEYS` (test `pager_builtin_triggers_are_reserved_in_shell`) |
| Modal state, keys, render | `P/views/traces_modal.rs` |
| `ActiveModal::Traces` | `P/views/modal.rs` (variant, `hint_pairs`, `message`) |
| Key/mouse/render routing | `P/app/modals.rs`: the Traces branch before MemoryBrowser in `handle_modal_key_with_registry`, the mouse handler, `draw_active_modal`, and the `unreachable!` list |
| Actions | `P/app/actions.rs`: `OpenTraces`, `TracesRescan`, `TracesCopy`, `TracesNotice`, `TracesResume`, `Effect::ScanTraces`, `TaskResult::TracesScanned` |
| Dispatch | `P/app/dispatch/router.rs` (the arms beside `OpenMemoryModal`), `P/app/effects/mod.rs` `Effect::ScanTraces` (spawn_blocking scan), `P/app/dispatch/task_result.rs` `TracesScanned` |
| Resume | `P/app/dispatch/session/load.rs`: `dispatch_traces_resume` routes to `dispatch_foreign_resume`, shared with the session picker. Claude and Codex go through `/resume-claude` or `/resume-codex <id>`. Grok goes through `resolve_local_session_any_cwd` and then `dispatch_load_session` |
| pty proof | `xai-grok-pager-pty-harness/tests/pty_e2e/traces_modal_ledger_trajectory_inspector.rs` (family `pty_e2e_config_ui`) |
| User doc | `xai-grok-pager/docs/user-guide/04-slash-commands.md` `### /traces` |
| `grok live` | `P/live_cmd.rs` (`run`, `step`), `Command::Live` in `P/app/cli.rs`, handled at the top of `async_main` in BIN. pty proof: `xai-grok-pager-pty-harness/tests/pty_e2e/traces_live.rs` (family `pty_e2e_config_ui`). Docs: `24-monitoring-usage.md` `## Live agents (grok live)` |

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
  - `completed` if the last event type is terminal: the squashed type ends with `sessionend`, `sessionshutdown`, `runend` or `agentend` (`task_complete`, `turn_end`, `turn/complete` are not).
  - `active` if not terminal and the file mtime is within 5 min.
  - Otherwise `idle`.
- **Storage diet:** events keep a `preview` (whitespace-collapsed, 100 chars), not the text. Row layout (schema 4):
  - `trace_events` is `WITHOUT ROWID`, `PRIMARY KEY (sk, line_number)`; no text id, no extra index, no per-event cwd.
  - `kind`/`role` are `Kind::code`/`Role::code`. Event type, title and model are interned in `trace_names` (rows hold ids).
  - `preview` is `''` when it equals the title (read back as the title).
  - Duration, tokens, turn and step live in `trace_event_usage`, only for events that have them.
  - Schema 6 adds `noise`, `detail` (events) and `native_id`, `parent_native`, `label`, `agent_type`, `done`, `signal_count`, `now_text`, `last_event_at`, `failed` (sessions), `aux_sig` (files).
  - Schema 7 adds `harness_pid`, `harness_start`, `context_tokens`, `context_window` (sessions), `context_tokens`, `context_window` (event usage) and the receipt columns below. Session `harness_*` come from the newest receipt (refreshed on every receipt that carries a pid, since an unchanged transcript is not re-indexed); `context_*` from the last event that had one. Claude context = `input + cache_read + cache_creation` of the message, Pi = `input + cacheRead + cacheWrite`, DSH = `inputTokens + cacheReadTokens + cacheWriteTokens` (`reasoningTokens` is inside `outputTokens`; `totalTokens` is not a context size). DSH's window is `request/context.data.contextWindow`. A failed attempt has no top-level `usage`; the last embedded stream chunk `{type:"usage"}` is the sample (v0/v1: `assistant/chunk`). Cursor context = composer `usage.inputTokens` or hook `input_tokens` (cache read/write already included, so not added); window from `contextTokenLimit` / `context_window` on that line. `tokenCount` and a `turn_ended` line with those counts count the same way. `promptTokenBreakdown.totalUsedTokens` (else `contextTokensUsed`) fills context only, not the input/output sum. A Cursor line with none of these stays unset — message length is not a stand-in. bb `thread/contextWindowUsage/updated` sets `context_tokens` from `usedTokens` and `context_window` from `modelContextWindow` (root turns only; a null or zero does not clear a previous positive value). Running totals (Codex `total_token_usage`, bb `tokenUsage.total`) set none. Claude and Pi record no window.
  - Measured on the Mini's real data (608 files, 102k events): 338 -> 122 B/event all-in at schema 5; schema 6 on the same data: 9.7 MB / 102k events = 95 B/event (schema 5: 9.9 MB, 97 B), with 49% of events noise.
  - Scan order: all roots are discovered first, then files are indexed newest mtime first across roots, bb threads by `updated_at`. If the cap is hit, the oldest sessions are the ones missing. Titles come from the full text before clipping. The inspector re-reads the raw line via `byte_offset`; nothing is copied.
- **Dispatch tree** (`links.rs`): a session stores `native_id` and `parent_native`; `SessionRow.parent_id` and `children` resolve them inside one source at query time (indexes on `(source_id, native_id)` and `(source_id, parent_native)`), so scan order does not matter and an evicted parent leaves a root. `roots_only` = no resolvable parent. Found per harness:
  - claude: `<session>/subagents/agent-*.jsonl` -> `<session>.jsonl`; sibling `agent-*.meta.json` gives `description` (label), `agentType`, `finished` (-> status done; `error`/`status: failed` -> failed). The meta's size+mtime is `trace_files.aux_sig`, so a meta change refreshes without reparsing the transcript. No error field exists in today's meta files.
  - codex: `session_meta` `payload.parent_thread_id` / `source.subagent.thread_spawn.parent_thread_id`; label = `agent_path` leaf or nickname, type = `agent_role`. (`forked_from_id` alone is a fork, not a dispatch.)
  - omp: task subagents live at `<cwd>/<parent stem>/<Name>.jsonl` (parent = the sibling `<parent stem>.jsonl`, label = `<Name>`); `session.parentSession` (a path) is a branch. pi: no parent field.
  - dsh: header `parentSession` (an id; `seedLength` = fork). grok: `summary.json` `parent_session_id` when `session_kind` starts with `subagent` (none on the Mini yet); other kinds (fork, headless) are not dispatches. bb: `threads.parent_thread_id`. cursor: no layout for subagents.
- **Session signals** (computed at index time, no scan of events at query time): `now_text` (last `Tool: detail` or assistant text), `last_event_at` (non-noise), `signal_count` (drives `hide_trivial`: <3 non-noise events and 0 tools), `failed` (the last non-noise event is an error, so nothing followed it; cleared by any later event; bb status `error`; sidecar finished-with-error).
- **Versions:** `trace_meta` holds `schema_version` and `parser_version` (parser 7, schema 7). Parser 7 re-reads DSH context size and window, the Cursor context read above, and bb `thread/contextWindowUsage/updated` (parser 6 skipped it); the mismatch rebuilds the index. There is no ALTER path: an older index (schema 6 and below) is replaced on open, and the old indexes (`trace_receipts_session` is now `(session_id, at_ms, id)`) go with it via `.rebuild.lock`. An older index on open is replaced: `rebuild` takes `<db>.rebuild.lock` (exclusive create; stale after 60 s), re-probes under it, renames the db and sidecars to `*.old.<nanos>` and deletes them, then builds fresh. A process that loses the race waits at most 2 s, then returns an error (hook: one stderr line, exit 0; it never blocks or builds a second db). Creating a new db and quarantining a corrupt one use the same lock. An index from a NEWER version is never replaced (open errors), so two binaries cannot rebuild each other.
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
- **Receipts:** `trace_receipts` (one row per hook event, pruned to 100k every 1000th insert). Schema 7 adds `detail` (call summary, `Notification` type, Claude `SessionStart` source), `tool_use_id`, `agent_id`, `harness_pid`, `harness_start` (Unix s); indexes `(session_id, at_ms, id)` and `(source, native_session)`. A receipt whose path does not resolve takes `session_id` from `trace_sessions.native_id` in the same source, else stays NULL with `native_session` set (Cursor `native_id` = transcript file stem). One match rule everywhere: `session_id = s.id`, or `session_id IS NULL` and `native_session = s.native_id` in the source. `record_receipt`, `receipts_watermark`, `last_receipt` (ordered `(at_ms, id)`), `turn_receipts`, `receipt_count_since`; `live_sessions` and the boundary/child tables are in `live.rs`. A receipt under 2 min old whose event is not terminal (`SessionEnd`, `Stop`, `stop`, `session_shutdown`, `agent_end`) makes a session Active regardless of mtime.
- **`harness_chain` / `via`:** `trace_receipts.harness_chain` is the harness, then each ancestor up to pid 1 (`launchd`, never the harness), as `a>b>c` basenames, at most 12 entries (the harness search is not capped). Shells stay (`claude>zsh>herdr`); a leading `-` is stripped. The harness keeps its `comm`; later `node`/`bun`/`python*` entries are named by their script (argv from `sysctl KERN_PROCARGS2`, no `ps`; `-e`/`-p` give no script, `-r`/`--require`/`--import`/`--loader` skip a value, `-m` names the module; EPERM, zombie or unparsable gives `comm`). Non-macOS: none. A payload `harness_pid` (Pi) starts the chain at that pid. `live::via_of(chain)` names the first entry after the harness among `bb-host-daemon.js` -> `bb`, `herdr`, `Cursor*` -> `cursor`, and `bb-acp.mjs`/`zed`/`Zed` -> `zed` (the lead's reading of spec 5.2: bb's ACP bridge is Zed's). `trace_sessions.via` is computed at write time from the newest receipt that carries a chain (independent of the pid pair); receipt-only Cursor rows do the same. No schema bump: the DDL of schema 7 changed in place (both Macs' indexes are still 6). bb-run Claude sessions (Agent SDK) fire no hooks, so they never appear live (spec Harnesses run by bb).
- **Live modal:** `event_loop.rs` arms `traces_poll_at` (1s, same pattern as `dashboard_poll_at`) while the active agent's modal is `Traces`; the arm calls `TracesModalState::poll_live`. It compares `receipts_watermark()`; on change it requeries the ledger keeping the selected session by id, and requeries an open trajectory only if that session's `event_count` grew (follows the bottom row, otherwise keeps selection). Header shows `● live` while the newest receipt is under 2 min old. `last_receipt_at()` backs it. Tests: `live_poll_*` in `traces_modal.rs`; the pty scenario runs `grok-traces hook claude` mid-ledger (binary: `GROK_TRACES_BIN` or next to the pager binary; build `-p xai-grok-traces --bin grok-traces` first).
- **Hook rules:** busy timeout 250 ms. Failures go to one stderr line; a tripped index posts a macOS notification at most hourly (stamp file `tripped.notified` beside the db). Grok's `updates.jsonl` path maps to the sibling `chat_history.jsonl`. No watcher or daemon exists; the hook is the only live feed.
- The index lives at `~/.grok/traces/traces.sqlite`. It is disposable and never uploaded.

## Remote (read-only, ssh)

`xai-grok-traces/src/remote.rs`. Hosts: `~/.grok/traces/hosts`, one ssh target per line (`sab-mini@sab-air`), `#` comments; absent = none. Remote rows: `host = Some(label after '@')`, id `label:<remote id>`, `parent_id` prefixed the same. `fetch_sessions/fetch_events/fetch_raw` run `ssh -o BatchMode=yes -o ConnectTimeout=2 -o ServerAliveInterval=2 -o ServerAliveCountMax=1 <target> '~/.grok/bin/grok-traces list|events|raw ...'`, child killed at 4s. Errors are `remote::Error` values (`"sab-air unreachable (...)"`). Last good result per host cached in memory only (`cached_sessions`). Old remote binary (rejects a flag): `fetch_sessions` falls back to plain `list --json`, filters locally, `Fetched.note` says so. Bin: `list --json [--since MS] [--active] [--roots] [--limit N]`, `events <id> --json [--noise] [--limit N] [--since-line N]` (line exclusive), `raw <id> <line>` (exit 3 = none), `version`. Live: bin `live --json` prints one line `{"live_schema":N,"rows":[LiveRow]}` (`live::LiveEnvelope`; no `--json` = usage, exit 2). `fetch_live` runs it, tags rows like sessions (`host`, `session_id`, `parent_id`; never `native_id`), caches the last good tagged result in `LIVE_CACHE` (`cached_live` -> `(elapsed, rows)`; the pager adds that as `extra` to `render`/`age_ms`). Schema gate: exit 2, empty/unparseable stdout, missing `live_schema` (`none`) or a mismatch (`<n>`) gives `Ok` with no rows and `note = "<label>: update grok-traces (live_schema <n|none>)"`, never cached; 255/timeout = `Unreachable`, 127 = `Remote` not installed, other nonzero = `Remote`, bad `rows` under a matching schema = `Parse`. `Fetched<T = SessionRow>`. Cross-fetch ordering: `live::order` (children nested under their parent, orphans stay roots). Tests: `tests/remote.rs` (fake ssh scripts). Smoke: `--example remote_smoke -- user@host`.

## Pager rules learned

- The modal runs SQLite queries synchronously; the index makes them cheap. Live `/` search covers session columns only. Enter adds the event-content scan (`SessionFilter.deep`).
- The pty harness copy check reads `$GROK_COPY_FILE`. OSC52 is not emitted in that harness for this path.
- The first pty launch of a freshly built binary can time out on welcome (macOS first exec). Rerun before debugging.
- The test baseline on main 7eb90d57 is pager lib 10389/0 and shell lib 6 failures (with `RUST_MIN_STACK=33554432`; without it the run aborts on stack overflow). `GROK_HOME` must be empty for pager tests.
- `grok live` is standalone: no agent, no auth, no sandbox, no tracing init (stderr is redirected to /dev/null). `live_cmd` runs its own ratatui loop straight into `Mode::Live`; the 1s tick is `poll_live()` called from `live_cmd`, not from `event_loop`, and every tick redraws. Intentional reading of SPEC-live line 19 (which has `grok live` booting the agent/TUI); the spec is unedited.
- The binary comes from `-p xai-grok-pager-bin` (bin `xai-grok-pager`). `--version` prints `grok <ver> (<12-char HEAD sha>)`.

## Hooks

Per-harness wiring so sessions index as they happen. Templates and installer: `xai-grok-traces/hooks/`. Every path calls `~/.grok/bin/grok-traces hook <source>` (one JSON payload on stdin, exit 0, silent).

| Harness | Mechanism | Installed to |
|---|---|---|
| claude | `settings.json` hooks: SessionStart, UserPromptSubmit, Pre/PostToolUse, PostToolUseFailure, Stop, SubagentStop, SessionEnd, PermissionRequest, PermissionDenied, Notification, SubagentStart, PreCompact, PostCompact, StopFailure | edited in place via jq |
| codex | `hooks.json`, same list minus SessionEnd (Codex has none) | edited in place via jq |
| grok | `hooks/grok/traces.json` (async): SessionStart, UserPromptSubmit, PostToolUse, Stop, SessionEnd | `~/.grok/hooks/traces.json` |
| omp | `hooks/omp/traces.ts` extension, tool_result debounced 500ms with a trailing spawn | `~/.omp/agent/extensions/` |
| pi | `hooks/pi/traces.ts` extension, one spawn per event: session_start, agent_start, tool_execution_start/end (`tool`, `tool_use_id` from `toolName`/`toolCallId`), agent_end, session_shutdown; payload carries `harness_pid`; no debounce | `~/.pi/agent/extensions/` |
| dsh | `hooks/dsh/traces.mjs` Cordis plugin on `session/event`, throttled 1s | `install.sh --dsh-profile NAME`: copied to `~/.dsh/plugins/grok-traces.mjs`, mounted from that profile's `cordis.patch.yml` |
| cursor | `hooks.json` (flat, `version: 1`): all 18 events (sessionStart, beforeSubmitPrompt, preToolUse, beforeShellExecution, beforeMCPExecution, beforeReadFile, postToolUse, postToolUseFailure, afterShellExecution, afterMCPExecution, afterFileEdit, afterAgentThought, afterAgentResponse, subagentStart, subagentStop, preCompact, stop, sessionEnd). Gating events work with several hooks in the Cursor CLI (Gate 0 row 11); the Cursor app is unverified | edited in place via jq |
| bb | `hooks/bb/` plugin (`bb.events.on` thread.created/active/idle/failed/archived), throttled 1s per thread | `bb plugin install ~/.grok/hooks/bb`, only on the host running the bb server |

- `install.sh`: `--dry-run` (default, prints `diff -u`), `--apply`, `--remove`, `--check`. `--apply` refuses while the binary is missing (`--even-if-missing` overrides). JSON files get a `.bak-traces-<ts>` backup, are built in a temp file, and are validated with `jq empty` before the move.
- Claude/Codex: removes hook entries containing `log-commands.sh`, plus any previous grok-traces entry, then appends one per event. Re-runs are idempotent. Atuin, Moshi, Orca, Herdr and other hooks are untouched.
- `pi-observability.ts` (OMP symlink) is only moved aside with `--apply --retire-pi-observability`. `--remove --retire-pi-observability` puts it back.
- Grok's hook payload carries `transcriptPath`/`transcript_path`, but it is `<session dir>/updates.jsonl` (set only if the file exists), not `chat_history.jsonl`. The session dir is the same, so the trace side should map it to the sibling `chat_history.jsonl`.
- Cursor payload (moshi-hook and Orca relay as evidence): `hook_event_name`, `conversation_id`, `generation_id`, `workspace_roots`, `transcript_path` (path to `agent-transcripts/<id>/<id>.jsonl`; may be null/absent), plus per-event fields (`text` on afterAgentResponse, `tool_name`/`tool_input` on tool events, `status` on stop). All of event/session/path/cwd are already in `hook.rs` key lists.
- Cursor stdout contract (Orca's table): `{}` is the reply for afterAgentResponse, postToolUse, postToolUseFailure, stop. beforeSubmitPrompt needs `{"continue":true}`, preToolUse/beforeShellExecution/beforeMCPExecution need `{"permission":"allow"}`. `grok-traces hook cursor` now replies per event (`cursor_reply` in `src/bin/grok-traces.rs`): `{"continue":true}` for beforeSubmitPrompt, `{"permission":"allow"}` for preToolUse, beforeShellExecution, beforeMCPExecution, beforeReadFile, `{}` otherwise. The reply is written once, before the index is opened, and the panic hook replies only if none was sent. Multi-hook gating behavior in the Cursor app is unverified (row 11: needs Sab).
- bb: the plugin API is `export default async function(bb)` with `bb.events.on(name, ({thread}) => ...)`; handlers are fire-and-forget. It spawns `grok-traces hook bb` with `{"event","thread_id"}`. Install/remove go through `bb plugin install|remove` (id `grok-traces-hook`). A bb CLI whose `~/.bb/config.json` has a `serverUrl` (the Mini's points at the Air) is a client, so the installer skips it: run it on the Air.
- DSH: the block is wrapped in `# grok-traces begin/end` comments, injected into the profile's existing top-level `- insert:` list or appended as a new one; `--remove` deletes the block (byte-identical restore) and the plugin copy once no profile uses it. Profiles are git-synced between Macs, so the plugin file must be installed on both before a synced patch is used. Profiles in use: `acp-enhanced` (gb `dsh`, khoj worker), `acp` (gb `dsh-stock`), `web`, `tui`.
- DSH logs are `~/.dsh/sessions/<project-key>/<session id>/session[.vN].jsonl.zstd`. The plugin finds the file by scanning project dirs for the session id.
- Tests: `hooks/test.sh` (synthetic `HOME`, includes the JS templates under node >= 22). Real-config dry-runs: `docs/traces/HOOKS-dryrun-mini.txt`, and for cursor/dsh/bb on both Macs `docs/traces/HOOKS-dryrun-wire.txt`.

## Live captures (Gate 0, 2026-10-21)

Source: `~/.grok/traces/capture/` on the mini, from the headless runs below plus the interactive Claude sessions already captured. Air: only `capture/cursor/` with one empty smoke file; no Claude or Pi captures, and nothing was run there (read-only). Every Air cell is therefore "no data; needs Sab". Counts are files; no payload text is recorded here.

| # | mini | air | evidence | fallback in use |
|---|---|---|---|---|
| 1 | yes (Pre/Post/PostToolUseFailure); needs Sab (PermissionRequest/Denied) | no data | `tool_use_id` present in 154 Pre, 144 Post, 3 PostToolUseFailure files (none missing). 0 PermissionRequest and 0 PermissionDenied files exist | orphan rule for the two permission events |
| 2 | yes | no data | headless A: one `agent_id` appears in SubagentStart, a Pre, a Post and SubagentStop; the parent's tool receipts carry no `agent_id`. Start keys: `agent_id, agent_type`; Stop adds `agent_transcript_path`. The subagent tool is `tool_name` `Agent` | none needed |
| 3 | needs Sab | no data | 0 PermissionRequest files; a granted prompt cannot occur headless | `⚠?` form stands; row 4 as written |
| 4 | yes (Notification); needs Sab (PermissionRequest/Denied) | no data | 5 Notification files, all `idle_prompt`, all have `transcript_path`. No permission-type Notification seen | `native_session` → `trace_sessions.native_id` for the two permission events |
| 5 | no | no data | 5 hook-denied Pre calls (incl. headless C) are followed by no `PostToolUseFailure` or `PermissionDenied` for their `tool_use_id`; they have no Post file at all | close-older rule stays for Claude |
| 6 | yes | no data | 13 UserPromptSubmit files: first non-shell ancestor is `claude` in all; one `lstart` per session (3 interactive sessions). Caveat: headless `-p` chains show two `claude` (the run, then the lead's) | none needed |
| 7 | yes | no data | headless A, B, C each have one SessionStart and one SessionEnd (3/3). The lead session has a Start and no End (still running) | PID liveness + supersede rule remain as backstop |
| 8 | needs Sab | n/a | `herdr agent list` row fields: `agent, agent_session{agent,kind,source,value}, agent_status, completion_seq, cwd, focused, foreground_cwd, pane_id, revision, state_change_seq, tab_id, terminal_id, terminal_title, terminal_title_stripped, workspace_id`. `focus` not run | `Enter` opens the trajectory |
| 9 | closed from source | closed | `toolCallId` on `tool_execution_start`/`_end` (observed in Pi capture payloads: `args, toolCallId, toolName, type` / `durationMs, isError, result, toolCallId, toolName, type`) | n/a |
| 9b | yes | no data | 15 Pi files: 0 empty `session_file`, 0 empty `session_id`. Envelope keys `event, payload, pid, session_file, session_id`. No approval event in the 9 event names seen | Pi has no `Waiting` either way |
| 10 | open until step 1 | open | needs `grok-traces list --json` | none; it is a number |
| 11 | yes (CLI only); needs Sab (app) | no data | Cursor CLI run: 2 `beforeShellExecution` + 2 `beforeReadFile` fired and the tool calls completed after the neutral replies (count 4 ≥ 2) | `capture.sh` stays on gating events; app unverified |
| 12 | recorded (CLI) | no data | see "Row 12 detail" below | Cursor pairs by `tool_use_id`; no FIFO needed |
| 13 | yes (CLI only); needs Sab (app) | no data | 1 file matches `projects/*/agent-transcripts/<id>/<id>.jsonl` for the CLI `conversation_id` | receipt-only rows if the app differs |
| 14 | yes (CLI only); needs Sab (app) | no data | first non-shell ancestor is `cursor-agent`, same pid and `lstart` in first and last file of the chat | first non-shell ancestor used; `sessionEnd` the reliable end |
| 15 | needs Sab (decision) | no data | `sessionEnd` fired once, when the CLI process ended (`reason` `error`, the run lost its connection). Chat close in the app untested | finished chat stays `○ your turn` until Cursor quits; Sab decides, or display-only hide after `stop` |
| 16 | needs Sab (no data) | no data | 0 `subagent*` files; the CLI prompt dispatched none | children not separated; parent `Tool` disabled while `open_children` non-empty |
| 17 | n/a | needs Sab | Air has no Claude/Pi captures and 1 empty Cursor file; running sessions there is outside read-only | per-item fallbacks per host |
| 18 | step 3 | step 3 | needs the step-3 binary | none |

Schema 7: no ALTER path; an old index is replaced through the `.rebuild.lock` rebuild.

### Row 12 detail (Cursor CLI, conversation d4bc8505)

- Every event has `conversation_id`, `generation_id`, `workspace_roots`, `hook_event_name`, `session_id` (equals `conversation_id` in this run), `transcript_path`, `model`, `cursor_version`, `user_email`.
- `transcript_path` is null on `sessionStart` and the first two `preToolUse`, then a string on all later events.
- `cwd` is present on `beforeShellExecution` and on `preToolUse`/`postToolUse` for Shell only; absent on Read and on `beforeReadFile`, `afterShellExecution`, `sessionStart`, `sessionEnd`.
- Tool-call id field: `tool_use_id` on `preToolUse`, `beforeShellExecution`, `beforeReadFile`, `postToolUse`, `afterShellExecution`; the same id across each Pre/before/after/Post for one call (Shell ids are UUIDs, Read ids are `tool_…`).
- `beforeShellExecution` has no `tool_name`; its command field is `command` (also `cwd`, `sandbox`). `afterShellExecution` has `command, duration, output, sandbox`. `beforeReadFile` has `file_path, content, attachments` and no `tool_name`.
- Order for one read: `preToolUse` (Read) then `beforeReadFile`, then `postToolUse`; no `afterReadFile`. For one shell call: `preToolUse`, `beforeShellExecution`, `afterShellExecution`, `postToolUse`. So each call is two Pre and two Post receipts, matched by the same id.
- `beforeReadFile`: observed: the read completed after the neutral reply (yes).
- Other keys: `sessionStart`/`sessionEnd` add `is_background_agent`; `sessionEnd` adds `duration_ms, final_status, reason`; `afterAgentThought` adds `text, duration_ms, model_id, model_params`.
- No `beforeSubmitPrompt`, `stop`, `afterFileEdit` or MCP events occurred (the edit step never ran; the run failed with a connection error).
- Label: all `cli` (`cursor-agent` in the chain; `workspace_roots` is the throwaway dir).

### Needs Sab

Rows 1 and 4 (PermissionRequest/PermissionDenied), 3 (granted prompt), 8 (`herdr agent focus`), 11 and 13-14 (Cursor app), 15 (chat close; decision), 16 (subagents), 17 (whole Air side), 10 and 18 (later steps).

### Surprises

- The spec's `rg -L` is `--follow` in ripgrep, not "files without match": rows 1 and 4 must use `--files-without-match`. Capture files have no blank line before the process chain; the separator is a line starting `--- process`, so `sed '/^$/,$d'` prints nothing useful (use `head -1`).
- The Cursor CLI also fires the Claude hook config: the same chat produced `SessionStart`/`PreToolUse`/`PostToolUse`/`SessionEnd` files under `capture/claude/` (camelCase `hook_event_name`, `session_id` = the Cursor `conversation_id`, chain through `cursor-agent`). A Cursor chat can therefore be indexed under source `claude` as well as `cursor`.
- A Claude `SessionStart` with `source: compact` was observed (lead session): `PreCompact`, then `SessionStart`, then `PostCompact`, with no `UserPromptSubmit` between the surrounding tool calls (mid-turn).
- Headless `-p` Claude B (`run ls`, no allowedTools) was allowed; no permission events fired.

### Headless runs (mini, UTC, one-line prompts, hooks fired in all)

| Run | Command | Session / conversation id | Start - end |
|---|---|---|---|
| Claude A | `claude -p --output-format json --allowedTools 'Task Bash(ls:*)' --max-turns 6 '<subagent runs ls; also ls and ls -a in one message>'` | 2740b20d-a74f-4fef-ac50-9a159ff1df2e | 11:42:54 - 11:43:36 |
| Claude B | `claude -p --output-format json --max-turns 3 'run ls'` | 639519a8-e048-4389-a3a7-2d4c46d3e73c | 11:43:36 - 11:43:45 |
| Claude C | `claude -p --output-format json --allowedTools 'Bash' --max-turns 3 '<recursive grep>'` | e256be2d-40b2-48b2-babc-c9da2e4145c8 | 11:43:45 - 11:44:05 |
| Pi | `pi -p 'run ls, then read f.txt'` | 01a15e46-c4c8-7056-82cb-69d09fabd968 | 11:44:18 - 11:44:25 |
| Cursor CLI | `cursor-agent -p --force --output-format json '<ls, read f.txt, append a word>'` | d4bc8505-5df3-4908-9226-dc1a468cdee6 | 11:44:25 - 11:46:11 (connection errors, edit never ran) |

Each run used a throwaway dir under `$TMPDIR`, wrapped in a 120-180 s alarm (no `timeout` on macOS).

## Live fold

Code: `crates/codegen/xai-grok-traces/src/live.rs` (`norm`, `strict_ids`, `fold_session`, `fold_children`, `fold_all`, `render`); tests `live::seq::seq_NN_*` and `tests/live.rs` (rows through a real index); replay `tests/live_replay.rs` (ignored, `GROK_TRACES_REPLAY_DB`). Spec: `SPEC-live.md` §Event normalizer and §Live state machine.

- `strict_ids`: Pi yes (row 9); Claude no (row 5 = no); Cursor no (row 5 covered Claude only; row 12 has ids).
- Child mapping: `child.parent_id == parent.id` and the last component of `native_id` is exactly `agent-<agent_id>`.
- `LiveRow` adds `extra_tools: u32` (the `+N`, state `Tool` only) beyond the spec's field list; `LIVE_SCHEMA` stays 1 since nothing shipped. `stale` is dropped.
- `LiveRow.via` (`#[serde(default)]`, from `SessionRow.via`; children copy the parent's) was added the same way, `LIVE_SCHEMA` still 1.
- `LiveRow.model` and `started_at` (`#[serde(default)]`, from `SessionRow`; children use their own) were added the same way. `LIVE_SCHEMA` stays 1: `remote::fetch_live_with` requires equality, so only a breaking change bumps it.
- `turn_receipts` also returns the newest own-thread `End` at any age; the fold drops children started before it.
- `fmt_age` (age column) prints `0s`..`59s`, `Nm`, `Nh`, `Nd`; `fmt_dur` (inside text) `0s`..`59s`, `Nm`, `Nh`.
- `fmt_age` never prints `now`, by lead decision: the spec mock and rows 40/43 require seconds.
- Cursor duplicate Pre: `preToolUse` and `beforeReadFile`/`beforeShellExecution` share a `tool_use_id` (and `beforeReadFile` has no `after*`). A `Pre` whose non-NULL id is already open is not pushed again (the first keeps its `since`); a `Post` with an id closes every open `Pre` with that id (close-older also closes the older ones). Strict and close-older both.
- `fold_all`'s `alive` takes `(pid, Option<start>)`: `(pid, None)` is the pid-only `alive_pid` (signal 0 or EPERM, not a zombie).
- Decision-model judgments (Jev/Laya) are specified in trace-deck DECISION-MODELS.md; the traces crate only needs to emit full tool args on follow call items.

## Tool pages

- `TOOL-grok-traces-cli.md` the `grok-traces` binary.
- `TOOL-traces-modal.md` the `/traces` modal.
- `TOOL-lookout.md` and `TOOL-aliases.md` in trace-deck (`~/repos/trace-deck`): `lookout` and the `lookt`/`follo` functions.
