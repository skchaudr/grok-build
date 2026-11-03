# dpi agents as a /traces source

Status: wired. `dpi.rs` reads and maps; `index/dpi_index.rs` indexes; tests are `tests/dpi.rs` and `dpi::tests` (synthetic dbs). Modeled on `BB.md`.

## Store

`~/.dpi/backend` (`$DPI_DATA_DIR`, else `$DPI_HOME/backend`, same resolution as the backend). One `sessions/<session-uuid>/agent.sqlite` per dpi session (WAL, opened read-only with a 500 ms busy timeout; never written). `catalog.sqlite` is optional enrichment; sessions are found on disk.

| Table | Columns used |
|---|---|
| `agent.sqlite` `conversations` | `id`, `owner_conversation_id` (set = child/subagent of that conversation) |
| `agent.sqlite` `entries` | `id`, `conversation_id`, `head`, `commit_seq`, `record` (JSON) |
| `catalog.sqlite` `sessions` | `id`, `name`, `cwd`, `model`, `provider`, `createdAt` |
| `catalog.sqlite` `conversation_names` | `sessionId`, `conversationId`, `name` |

## Session model

Each conversation is one agent and one grok-traces session. `native_id` = `<session uuid>:<conversation id>`; `trace_files.path = trace_sessions.file_path = "<agent.sqlite>#<conversation id>"`, `format = sqlite`, source `dpi`.

- Title: `conversation_names` entry, else (top-level conversation) the session name, else the first user prompt.
- `cwd`, default `model` (a fallback; the model the messages name wins) and `agent_type` (= provider) come from the catalog.
- Parent: `owner_conversation_id` becomes `parent_native = <uuid>:<owner id>`; the conversation's name is its `label`.
- Conversations with no entries are not indexed. A vanished session is pruned; an unreadable `agent.sqlite` keeps its cached rows.

## Pointer

`entries.id` (monotonic in a db) is the pointer and resume watermark: `trace_events.line_number = entries.id`, `byte_offset = 0`. Append reads `id > indexed_bytes`; reset on parser version change or a last id that went backwards. `size_bytes` = last entry id, `mtime_ms` = newest message timestamp among the last 8 entries (catalog `createdAt` if none). `raw_line` returns the entry row as JSON (`id`, `conversation_id`, `head`, `commit_seq`, `record`).

## Mapping (`dpi::map_entry`)

`record.model[0]` is a Pi-shaped message, normalized exactly like a Pi session line (`{"type":"message","message":...}`), so roles, tool calls (`detail` = argument summary), tool results (`isError`), thinking, usage (context tokens) and call-to-result `duration_ms` match Pi sessions.

| `record.kind` | Normalized |
|---|---|
| `pi.user`, `pi.assistant`, `pi.tool-result` | the Pi message |
| `pi.system` | skipped at read time (the injected system prompt, large) |
| anything without `model` (`pi.reset`) | system, noise, event type = the kind |

- Only `model[0]` is read; every entry seen so far holds exactly one message.
- Like Pi, `input_tokens`/`output_tokens` stay unset (Pi `usage.input`/`output` are not read); `context_tokens` = `input + cacheRead + cacheWrite`.
- `tool_count` counts calls and results, as for Pi.

## Hook

`grok-traces hook dpi` reads stdin JSON:

```
{ "event": "<name>", "tool": "<tool>", "cwd": "<dir>",
  "session_id": "<session-uuid>:<conversation-id>",
  "session_file": "<path to agent.sqlite>", "tool_use_id": "...", "tool_input": {...} }
```

Only `session_id` selects work. It must be `<uuid>:<int>` with a plain id (`[A-Za-z0-9-]`); the db is located from the dpi root and the uuid, not from `session_file`, so it keys the same as a scan. The hook records the receipt and indexes just that conversation (`index_dpi_conversation_id`), then wakes viewers. A `session_id` without `:<int>`, an unknown conversation, or a path-shaped id records the receipt only.

The dpi backend (`apps/dpi-backend/src/observe.ts` in durable-pi) sends its own pid as `harness_pid` and Pi's event names (`agent_start`, `tool_execution_start`/`_end`, `agent_end`), one hook at a time. `live` folds dpi with Pi's rules (`boundary`, `norm`, `strict_ids` in `live.rs`). One backend pid hosts every agent, so dpi is left out of the same-process supersede that Claude and Pi get.
