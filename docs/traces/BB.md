# bb threads as a /traces source

Status: wired. `bb.rs` reads and maps; `index/bb_index.rs` indexes; tests are `tests/bb.rs` (synthetic db).

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
| `turn/started`, `turn/completed` | turn; `turn/completed` ends a turn, not the thread (not terminal; only session/run/agent end are) |
| `provider/error`, `system/error` | system, error |

- A tool call is one event, not call plus result: bb writes the call and its output in one `item/completed` row.
- Error: `item.status == "failed"`, a non-empty `item.error`, or `turn/completed` with an error or `failed`/`error` status.
- `item/completed` rows get `event_type = "item/<kind>"`. Otherwise every finished tool call would match `is_terminal` ("complete") and mark the session done.
- Skipped at read time: `*/delta` and `*Delta`, `item/started` (the completed row has the same item), `provider/unhandled`, and bookkeeping (`provider.env-resolved`, `system/thread-provisioning`, `thread/extensionState/updated`, `thread/contextWindowUsage/updated`, `client/thread/start`, `turn/input/accepted`). On sab-air that drops about three quarters of the rows (57k events, 29k of them `provider/unhandled`).
- Never read or stored: tool arguments beyond the clipped preview, `aggregatedOutput`, `result`. Only the preview text goes in the index.

## Wiring (as built)

- Root `bb-threads` (`Format::Sqlite`, path `~/.bb/bb.db`), source `bb`, tab label `bb`. A missing db is an inaccessible root, no error; an unreadable one keeps cached rows. `SOURCES` ends with `bb`.
- `scan_with_limit` hands a Sqlite root to `scan_bb_root`: open read-only (500 ms busy), list threads, index each, drop the handle. Nothing is held between scans and nothing is written to bb.db.
- Per thread: `trace_files.path = trace_sessions.file_path = "<db>#<thread_id>"`, `format = sqlite`, `size_bytes` = last sequence, `mtime_ms` = `updated_at`, `indexed_bytes` = last examined sequence, `indexed_lines` = events indexed. Skipped when sequence and `updated_at` are unchanged; append reads `sequence > indexed_bytes`; reset on parser version change or a sequence that went backwards. Threads with no events are not indexed; deleted threads are pruned.
- `trace_events.line_number = sequence`, `byte_offset = 0`. `raw_line` for `format = sqlite` opens bb.db and returns `Bb::raw_event` (the row as JSON).
- Caps: `check_cap` before every thread (health/`MAX_DB_BYTES`), and `MAX_EVENTS_PER_FILE` per thread with a `parse_error` note.
- Hook: `grok-traces hook bb` records the receipt (with the thread's session id when `thread_id` is given) and indexes just that thread via `index_bb_thread_id`. A bb payload has no file, so `root_for` never matches it.
- Modal: the tab comes from `SOURCES`; `resume_target` is `None` for bb.

## Caveats

- bb truncates old completed item output in place (`events_completed_item_truncation_idx`, `truncation` field on command items). A re-read of an old event may return less than was indexed; the index keeps its preview.
- `visibility = 'hidden'` threads (10 of 77 on sab-air) are returned by `threads()`; filter in wiring if the ledger should not show them.
- `archived` is exposed, not applied.
- Subagent threads carry `parent_thread_id`; the ledger can group on it later.
