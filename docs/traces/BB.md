# bb threads as a /traces source

Status: reader and mapping written and tested (`crates/codegen/xai-grok-traces/src/bb.rs`, synthetic db). Not wired into `TraceIndex`: the index is file-shaped end to end, and the wiring below touches `index.rs` and `query.rs`.

## Store

`~/.bb/bb.db`, SQLite, WAL, on sab-air only. Never copy it. `Bb::open` is read-only with a 5 s busy timeout; bb keeps writing. A read-only open needs the `-shm` file or a writable directory, so run the index on the machine that owns the db.

Tables used (structure only; everything else in the db is out of scope):

| Table | Columns used |
|---|---|
| `threads` | `id`, `title`, `title_fallback`, `provider_id`, `model_override`, `status` (starting, active, idle, completed, error), `parent_thread_id`, `archived_at`, `deleted_at`, `created_at`/`updated_at` (epoch ms), `environment_id` |
| `environments` | `id`, `path` (the thread's cwd) |
| `events` | `id`, `thread_id`, `sequence`, `type`, `item_kind`, `created_at` (ms), `data` (JSON). Unique `(thread_id, sequence)` |

## Pointer

`(thread_id, sequence)`. `sequence` is unique and increasing per thread, so it replaces the byte offset: it is both the resume watermark and the re-read key (`Bb::raw_event`). The row id (`evt_...`) is carried in `BbEvent` but not needed to re-read.

## Mapping (`bb::map_event`)

| bb event | Normalized |
|---|---|
| `client/turn/requested` (`input[].text`, `execution.model`) | message, user, model |
| `item/completed` `agentMessage` (`item.text`) | message, assistant |
| `item/completed` `userMessage` | message, user |
| `item/completed` `reasoning` | reasoning |
| `item/completed` `toolCall` `commandExecution` `fileChange` `fileRead` `webFetch` `webSearch` `delegation` | tool, depth 1, title = tool name (`bash`, `edit`, `read` for the three built-ins), `durationMs` |
| `thread/tokenUsage/updated` (`tokenUsage.total.{inputTokens,outputTokens}`) | telemetry, `usage_is_total` |
| `turn/started`, `turn/completed` | turn; `turn/completed` is the terminal event |
| `provider/error`, `system/error` | system, error |

- A tool call is one event, not call plus result: bb writes the call and its output in one `item/completed` row.
- Error: `item.status == "failed"`, a non-empty `item.error`, or `turn/completed` with an error or `failed`/`error` status.
- `item/completed` rows get `event_type = "item/<kind>"`. Otherwise every finished tool call would match `is_terminal` ("complete") and mark the session done.
- Skipped at read time: `*/delta` and `*Delta`, `item/started` (the completed row has the same item), `provider/unhandled`, and bookkeeping (`provider.env-resolved`, `system/thread-provisioning`, `thread/extensionState/updated`, `thread/contextWindowUsage/updated`, `client/thread/start`, `turn/input/accepted`). On sab-air that drops about three quarters of the rows (57k events, 29k of them `provider/unhandled`).
- Never read or stored: tool arguments beyond the clipped preview, `aggregatedOutput`, `result`. Only the preview text goes in the index.

## Wiring needed

All in the file-shaped index; this is why it was not forced.

1. `Format::Sqlite` (or a separate `Root.kind`) and a branch in `scan_with_limit`: for a bb root call `Bb::threads()` instead of `discover`. `discover` on a file path returns the file, which `index_file` would then read as JSONL.
2. A per-thread `index_bb_thread(root, bb, thread)` beside `index_file`:
   - Key: `trace_files.path = "<db>#<thread_id>"`, `format = "sqlite"`. `trace_sessions.file_path` takes the same string (it is `UNIQUE`). `session_id("bb", path)` as usual.
   - Watermarks: `size_bytes = thread.last_sequence`, `mtime_ms = thread.updated_at`, `indexed_bytes = last indexed sequence`, `indexed_lines = event count`. Skip when `last_sequence == indexed_bytes`. Append with `Bb::events(thread, indexed_bytes, N)` in a loop until a short page. Reset (delete the thread's events, reparse) on `parser_version` change or `last_sequence < indexed_bytes`.
   - `trace_events`: `line_number = sequence`, `byte_offset = 0`, id `"{sid}:{sequence}"`.
   - Feed each `BbEvent.event` through `Aggregate::add` (private to `index.rs`; do the wiring there or make it `pub(crate)`). Title is `thread.title`, else the first user message; `cwd = thread.cwd`; model is `thread.model`, else the first event model. `write_aggregate` takes `thread.updated_at` as the mtime.
3. Pruning: a thread missing from `threads()` (deleted) is pruned like a vanished file, only when the listing succeeded.
4. `raw_line` (query.rs): when `source_id = "bb"`, split `file_path` on the last `#` and call `Bb::open(db)?.raw_event(thread, line)` instead of seeking.
5. `roots.rs`: `bb` in `SOURCES`/`source_label`, and a root at `bb::default_db_path(home)` (or `$BB_HOME/bb.db`).

Estimate: about 100 lines across `index.rs`, `query.rs`, `roots.rs`.

## Caveats

- bb truncates old completed item output in place (`events_completed_item_truncation_idx`, `truncation` field on command items). A re-read of an old event may return less than was indexed; the index keeps its preview.
- `visibility = 'hidden'` threads (10 of 77 on sab-air) are returned by `threads()`; filter in wiring if the ledger should not show them.
- `archived` is exposed, not applied.
- Subagent threads carry `parent_thread_id`; the ledger can group on it later.
