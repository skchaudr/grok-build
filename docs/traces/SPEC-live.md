# SPEC: `grok live` screen

Rev 4 (Fable 5.1, 2026-10-21) after reviews 1–3 (7390998f, 5d43a539, ef69d3f7) and Sab's amendments in review 3. (U) = not verified; Step 0 closes each before code. Paths under `crates/codegen/`. Read `MAP.md` first. Implementer: a compacted Opus lead with Sonnet 5.5 subagents.

## Scope (Sab, 2026-10-21)

v1 is **Claude Code, Pi and Cursor** (Cursor matters uniquely this month), all three completely right: every state, every test row, verified on real sessions. Codex comes later. Other sources stay in `/traces` history and never appear on the live screen. The data model stays source-agnostic (a later source adds normalizer rows and tests, nothing else). Cut from this spec: remote live rows, the Grok notification route, and every OMP/Grok/bb/DSH/Codex rule.

## The tool

A full-screen, always-on view on a second screen that answers, per running agent: what it is doing now, whether it is waiting on me, what it dispatched and what those children are doing, and how full its context is. One key jumps to the agent's pane when Herdr is present.

## Architecture

- No daemon, watcher or second store. Hooks call `grok-traces hook <source>`, which records a receipt and re-indexes the transcript. The screen polls the index at 1 s (`poll_live_at`).
- **Liveness is the harness process, not a timer.** Each receipt records the harness PID and its start time. A session is on the screen from its first receipt until a `SessionEnd` receipt arrives or that process is gone. There is no presence window and no cap. Timers exist only for display: `⏳` at 30 s, `⚠?` at 30 s, the `?` mark at 2 min, the 2 s `Stop` debounce.
- State is a fold over the session's current turn of receipts. Transcript growth clears nothing (review 2, E).
- Context comes from the index, refreshed per hook.
- One modal, two hosts: `TracesModalState` gains `Mode::Live`. `/traces` opens in list mode as today. `grok live` boots the pager straight into `Mode::Live` full screen; Esc exits the process. Soft subcommand beside `Command::Dashboard` (`rg -n "Dashboard" crates/codegen/xai-grok-pager*/src`). Named `live` because `Command::Trace` exists.
- Herdr is optional. Its only use is the jump key; without it `Enter` opens the trajectory.

## Step 0: hard gate

### 0a: the capture mechanism (exists: `hooks/capture.sh`, 43810bdf)

`capture.sh <source> [event]` saves the raw stdin payload plus the hook's process chain (`pid ppid lstart comm` up to PID 1) to `~/.grok/traces/capture/<source>/<ts>-<pid>-<event>.txt`, prints nothing, exits 0. The output holds real session text and is never committed. It is already appended to every event in `~/.cursor/hooks.json` (Sab approved; backup `~/.cursor/hooks.json.bak-capture-20261021`); the file `20261021T041502-53488-stop.txt` is a smoke test, ignore it. No `--dump` flag is built. Wiring it into `~/.claude/settings.json` (every event in `CLAUDE_EVENTS` plus the new ones) and into `~/.pi/agent/extensions/traces.ts` (spawn it beside the hook binary) is a config edit: show Sab the exact diff first. Nothing in steps 1–5 starts until every capture below has its answer in `MAP.md` and the (U) markers here are replaced.

(U) On Cursor's gating events (`beforeSubmitPrompt`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `preToolUse`) `capture.sh` prints nothing. MAP.md says those need `{"continue":true}` or `{"permission":"allow"}`. Whether Cursor treats an empty reply as allow, and how it combines several hooks on one event, is capture 11 below. If Cursor blocks, the capture lines come off the gating events first.

### 0b: captures. Each has the command, the field that closes it, and the fallback if the answer is no

Run on this machine one Claude session that dispatches a subagent, gets a permission granted, gets one denied by Sab, gets a Bash call denied by a `PreToolUse` hook (the recursive-grep guard), triggers a compaction, and issues two tools in one assistant message. Run one Pi session with two tool calls and a clean exit. Run one Cursor agent chat with a shell command, a file read, a file edit, an MCP call, a subagent if Cursor offers one, then close the chat and quit Cursor.

| # | Question | Closes when | Fallback |
|---|---|---|---|
| 1 | Claude: `tool_use_id` on `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionRequest`, `PermissionDenied` | each dump file has the field | pairing by the orphan rule only (below) |
| 2 | Claude: `agent_id` on `SubagentStart`/`SubagentStop`, and the same value on tool receipts fired inside the child; or the child's own `transcript_path` on those (U) | the values match across the child's files | child receipts cannot be separated: parent `Tool` state is disabled while `open_children` is non-empty, child rows use the no-receipt rule |
| 3 | Claude: any receipt between `PermissionRequest` and `PostToolUse` on a grant | a dump file exists between them | `⚠?` form stands; row 4 as written |
| 4 | Claude: `transcript_path` on `PermissionRequest`, `PermissionDenied`, `Notification` | field present | `session_id` from `native_session` → `trace_sessions.native_id` |
| 5 | Claude: a hook-denied call emits `PostToolUseFailure` or `PermissionDenied` | a dump file follows the denied `Pre` | orphan rule only; the Step 0 note says which |
| 6 | Claude: process chain from the hook to `claude` | the first non-shell ancestor's `comm` is `claude` (or `node` running claude) and its `lstart` is stable across receipts | PID unknown: the session stays until `SessionEnd` and the row shows `pid?` in the marks column |
| 7 | Claude: `SessionEnd` receipts arrive for async hooks | count of `SessionEnd` receipts equals sessions exited in the capture | PID liveness covers it; nothing else changes |
| 8 | `herdr agent focus w36:p5` focuses the pane | it does | `Enter` opens the trajectory |
| 9 | Pi: payload of `agent_start`, `tool_execution_start`, `tool_execution_end`, `agent_end`, `session_shutdown`: the tool call id field name, `getSessionId`/`getSessionFile`, and whether Pi exposes an approval event | field names recorded | Pi pairs FIFO by name; Pi has no `Waiting` state |
| 10 | Hook-to-index latency | ten samples of receipt `at_ms` against the transcript's `last_event_at` on the next poll; median and max in `MAP.md` | none; it is a number |
| 11 | Cursor: gating events with an empty reply still run, and with several hooks on one event | the capture session's shell command and file read go through with `capture.sh` wired | remove `capture.sh` from the gating events; capture only `after*`, `post*`, `stop`, `session*` |
| 12 | Cursor: one payload per event in `~/.cursor/hooks.json`: `conversation_id`, `generation_id`, any tool-call id, `tool_name`, `transcript_path` (null or not), `hook_event_name` | field names per event recorded | no tool id: Cursor pairs FIFO by name |
| 13 | Cursor: `conversation_id` equals the transcript file stem under `~/.cursor/projects/*/agent-transcripts/<id>/<id>.jsonl` | one captured id matches one file | Cursor receipts cannot join to a session row; Cursor rows are built from receipts alone (title = `cwd` basename, no `ctx`, no `now`) |
| 14 | Cursor: process chain from the hook to the Cursor process; which ancestor is stable for the life of a chat (`Cursor Helper (Plugin)`, `Cursor`, or a per-chat process) | `comm` and `lstart` recorded across the session | the first non-shell ancestor is used, whatever it is |
| 15 | Cursor: `sessionEnd` fires when a chat is closed or switched, not only when Cursor quits | a `sessionEnd` capture exists after closing the chat | a finished Cursor chat stays as `○ your turn` until Cursor quits; Sab decides whether that is acceptable or Cursor gets a display-only hide after `stop` (not a liveness timer) |
| 16 | Cursor: `subagentStart`/`subagentStop` payload: the child's id field, and whether child tool events carry a different `conversation_id` | fields recorded | Cursor children are not separated; parent `Tool` state is disabled while `open_children` is non-empty |

The Claude smoke test already shows the chain `sh → zsh → claude → -zsh → herdr`, so the shell-skipping walk reaches `claude` at the third hop.

## Data (`xai-grok-traces`)

Schema 7 and a `PARSER_VERSION` bump. No ALTER path exists; an old index is replaced through the existing `.rebuild.lock` path. Note it in MAP.md.

### `is_terminal` (live `/traces` bug, fix first)

`index.rs` `is_terminal` matches `complete` and `turnend`, so Codex `task_complete` and Pi `turn_end` mark a session `completed` on every turn. New rule: the squashed type ends with one of `sessionend`, `sessionshutdown`, `runend`, `agentend`; `complete` and `turnend` are gone. Tests: `task_complete`, `turn_end`, `turn/complete` are not terminal; `session_end`, `session/end`, `agent_end`, `session_shutdown` are. Ledger statuses change on rebuild; that is the fix.

### Receipts

`trace_receipts` gains `detail TEXT`, `tool_use_id TEXT`, `agent_id TEXT`, `harness_pid INTEGER`, `harness_start INTEGER` (process start, Unix seconds). `Receipt` and the insert in `receipts.rs` change with it. `hook::extract` adds key lists `["tool_input","toolInput","input"]`, `["tool_use_id","toolUseId"]`, `["agent_id","agentId"]`, `["notification_type","notificationType"]`, `["harness_pid"]`. `detail` = `classify::call_detail(tool, tool_input)` for tool and permission events; for `Notification` it is the notification type.

Harness process: if the payload carries `harness_pid` (Pi sends `process.pid`), use it. Otherwise walk up from the hook's parent PID past `sh`, `bash`, `zsh` to the first non-shell ancestor. Start time on macOS from `proc_pidinfo(PROC_PIDTBSDINFO).pbi_start_tvsec` (libproc; no new crate if `sysinfo` or `libc` is already a dependency, else `ps -o lstart= -p`). `live::alive(pid, start)` = `kill(pid, 0)` succeeds and the start time matches. Linux: `/proc/<pid>/stat` field 22; not needed for v1.

`trace_sessions` gains `harness_pid`, `harness_start` (from the latest receipt), `ended` (set by an `End` receipt), `context_tokens`, `context_window`. `SessionRow` gains those plus `native_id` (column exists; add to `SESSION_COLS`, `session_row`, `session_to_json/from_json`) and `mtime_ms` (already selected). `last_event_kind` from rev 3 is dropped; no rule reads it.

When `path` does not resolve, `session_id` falls back to `native_session` matched against `trace_sessions.native_id` in the same source. For Cursor this is the primary path, not the fallback: `transcript_path` may be null (review 1, data #2), so every Cursor receipt is keyed by `conversation_id`. `links.rs` therefore sets a Cursor session's `native_id` to the transcript's file stem (the conversation id) instead of the relative path stem (capture 13 confirms they match). A Cursor receipt whose `conversation_id` matches no session row yet (the transcript is written later, or never) is stored with `session_id = NULL` and `native_session` set; `live_sessions` groups such receipts by `(source, native_session)` into a receipt-only row (title = `cwd` basename, no `ctx`, no `now`), and `turn_receipts` accepts either key. When the transcript appears, the session row takes over and the receipts match it by `native_id`.

`last_receipt` and `last_receipt_at` order by `(at_ms, id)`.

New queries:

- `live_sessions() -> Vec<SessionRow>`: sessions in sources `claude`, `pi`, `cursor` with `ended = 0` and at least one receipt, plus receipt-only Cursor rows (above). The pager filters by `alive(harness_pid, harness_start)`; a session with `harness_pid IS NULL` stays until `ended`.
- `turn_receipts(session_id) -> Vec<LiveReceipt>`: the session's newest turn-boundary receipt (events that normalize to `Idle`, `Prompt`, `End`) **and** every receipt after it, by `(at_ms, id)`; when no boundary exists, every receipt. Rows with `agent_id` set are included and tagged. Fields: `session_id, agent_id, event, tool, detail, tool_use_id, at_ms`. Because the boundary itself is returned, `last_ev` is never undefined (review 3, blocker 2).
- `receipt_count_since(ms) -> i64` for the empty state.

### Usage

`NormalizedEvent.context_tokens`, `context_window`: Claude = `usage.input_tokens + cache_read_input_tokens + cache_creation_input_tokens` of that message, no window. Pi: (U) whether its session file records usage per message; if not, `None`. Codex's `last_token_usage` rule from rev 3 is kept in the normalizer for later but has no live-screen test in v1. `trace_event_usage` and `trace_sessions` gain both columns; the session value is from the last event that had one.

### Hook events

Claude installer `CLAUDE_EVENTS` adds `PermissionRequest`, `PermissionDenied`, `Notification`, `SubagentStart`, `PreCompact`, `PostCompact`, `StopFailure` (`,1` = takes `matcher: *`).

Pi extension `hooks/pi/traces.ts` is rewritten:
- subscribes to `session_start`, `agent_start`, `tool_execution_start`, `tool_execution_end`, `agent_end`, `session_shutdown`; `turn_end` and `tool_result` are not sent; the debounce is removed.
- payload: `event`, `tool`, `tool_use_id` (field name per step 0.9), `cwd`, `session_id`, `session_file`, `harness_pid: process.pid`.
- `hooks/test.sh` covers the new payload shape.

Cursor installer: `CURSOR_EVENTS` becomes `sessionStart`, `beforeSubmitPrompt`, `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `postToolUse`, `postToolUseFailure`, `afterShellExecution`, `afterMCPExecution`, `afterFileEdit`, `afterAgentThought`, `afterAgentResponse`, `subagentStart`, `subagentStop`, `preCompact`, `stop`, `sessionEnd` (the set present in `~/.cursor/hooks.json` today). The gating ones are wired only once `hook cursor` replies per event: `{"continue":true}` for `beforeSubmitPrompt`, `{"permission":"allow"}` for `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile` (the last is (U): capture 12 shows what Cursor expects), `{}` for everything else. The reply is written before the receipt is recorded, so an index failure never blocks Cursor. Test in `hooks/test.sh` and a unit test on the reply function. Cursor hooks carry no `harness_pid`; the ancestry walk finds the Cursor process (capture 14).

### Event normalizer

`live::norm(source, event, detail) -> Option<Ev>`. Anything not in the table is dropped (`unparsed`, `unknown`, `auth_success`, elicitation responses, unlisted Notification types).

| `Ev` | claude | pi | cursor |
|---|---|---|---|
| `Idle` (boundary) | SessionStart, Stop, StopFailure(✗), Notification/idle_prompt | session_start, agent_end | sessionStart, stop |
| `Prompt` (boundary) | UserPromptSubmit | agent_start | beforeSubmitPrompt |
| `End` (boundary) | SessionEnd | session_shutdown | sessionEnd |
| `Pre(id)` | PreToolUse | tool_execution_start | preToolUse, beforeShellExecution, beforeMCPExecution, beforeReadFile |
| `Post(id)` | PostToolUse, PostToolUseFailure, PermissionDenied | tool_execution_end | postToolUse, postToolUseFailure, afterShellExecution, afterMCPExecution |
| `Tick` | — | — | afterFileEdit, afterAgentThought, afterAgentResponse |
| `Ask` | PermissionRequest; Notification/{permission_prompt, elicitation_dialog, elicitation_url_dialog, agent_needs_input} | — (step 0.9) | — (Cursor has no permission event; Waiting is a known gap) |
| `ChildStart(agent_id)` / `ChildStop(agent_id)` | SubagentStart / SubagentStop | — | subagentStart / subagentStop (id field per capture 16) |
| `CompactStart` / `CompactEnd` | PreCompact / PostCompact | — | preCompact / — |

`SessionStart` is `Idle` (review 2, G). `(✗)` sets the failed mark. `Tick` is a non-boundary event that sets `ev_at` and yields `Working`; it closes nothing. Pi never shows `Waiting`, children or compaction. Cursor never shows `Waiting`. A Cursor `Pre` that is both `preToolUse` and `beforeShellExecution` for one call produces two `Pre`s; the matching `Post`s (`postToolUse`, `afterShellExecution`) close them by id, or by the orphan rule when ids are missing (capture 12). Cursor's `preCompact` has no end event, so `Compacting` clears on the next `Ev`, as rule 3 already says.

## Live state machine (`live.rs` in the traces crate)

### Inputs per session

- `evs`: `turn_receipts` normalized, `None`s dropped. The parent fold uses rows with `agent_id IS NULL`, plus `ChildStart`/`ChildStop` whatever their `agent_id`. A child row folds its own `agent_id` rows.
- `open_tools`: ordered list of `Pre`s not yet closed. A `Post` closes its own `Pre` (same `tool_use_id`; with a NULL id, the oldest open `Pre` of the same tool name) **and every open `Pre` older than that `Pre`** (review 3, blocker 1: 22 of 151 real turn ends carried an orphan Bash `Pre`). A `Pre` of the same name as an open NULL-id `Pre` closes the older one. Every boundary closes all.
- `open_children`: `agent_id`s with a `ChildStart` and no later `ChildStop`. `End` clears it; `Idle` and `Prompt` do not (children run across `Stop`).
- `last_ev`: the newest `Ev` that is not `ChildStart`/`ChildStop`. `Tick` counts. Always defined, because the boundary is in `evs`; if the session has no boundary at all, `last_ev` is the newest `Ev` of any kind, or `Prompt` when there is none.
- Clocks, local ms: `ev_at` = `at_ms` of `last_ev` (`ChildStart`/`ChildStop` never set it); `last_event_at` and `mtime_ms` from the session row. `age = now − max(ev_at, last_event_at, mtime_ms)`. `since(x) = now − x.at_ms`.

Boundaries: `<` on the left, `≥` on the right. `⏳` at `since(pre) ≥ 30 s`; `⚠?` at `since(ask) ≥ 30 s`; Prompt shows at `since(idle) ≥ 2 s`; Quiet text at `age ≥ 2 min`; `?` mark at `≥ 2 min`.

### States

First match wins. `<now>` = `SessionRow.now`. Every row has an age column showing combined `age`.

| # | State | Rule | State text |
|---|---|---|---|
| 1 | gone | `ended`, or `!alive(harness_pid, harness_start)` | row removed on the next poll |
| 2 | `Waiting` | `last_ev == Ask` | `permission: <tool> <detail>` with glyph `⚠`; a Notification-sourced Ask with no tool reads `<notification type>`. Glyph `⚠?` at ≥ 30 s (granted and running, or still waiting; no grant signal unless step 0.3 finds one) |
| 3 | `Compacting` | `last_ev == CompactStart` | `compacting · <since>`, glyph `⟳`. Any later `Ev` clears it |
| 4 | `Tool` | `open_tools` non-empty | `<newest open tool> <detail> · <since(newest pre)>`, glyph `▶`, `⏳` at ≥ 30 s; `+N` appended when N > 0 more are open |
| 5 | `Prompt` | `last_ev == Idle` and `since(idle) ≥ 2 s` (review 2, J) | `your turn`, glyph `○`; `· N bg` when `open_children` non-empty |
| 6 | `Working` | everything else | `<now>`, glyph `●`; `N bg · <now>` with children; at `age ≥ 2 min` the text becomes `quiet · <now>` with glyph `◌` |

Marks column (two cells after the age column, every row has it):

- `✗` when `SessionRow.failed` or the last `Idle`/`End` carried `(✗)`. A flag, never a state.
- `?` when the state is 2–4 and `since(ev_at) ≥ 2 min` and `now − max(last_event_at, mtime_ms) ≥ 2 min`.
- `pid?` when `harness_pid IS NULL` (step 0.6 fallback).
- `⌖` when a Herdr pane is known.

Sessions with no receipts are never on the live screen (they have no PID). Children: a child row folds its own `agent_id` receipts when any exist, else shows `● <now>` from its transcript while its parent is alive and it has no `End` and is not in `ChildStop`. The parent fold never reads child tool receipts.

Ordering: `Waiting`, `Prompt`, then `age` ascending, then `session_id` ascending. Children directly under their parent in the same order.

### Sequence tests (`live.rs`, one named test each; `@` is `at_ms` in seconds; ids in listed order unless said; `now` as stated; all sessions alive)

| # | Sequence | Expected |
|---|---|---|
| 1 | `Prompt@0`, `Pre A@1.00`, `Post A@1.05` inserted as ids (Post, Pre); now=2 | Working `● <now>`; `open_tools` empty ("later" is by `(at_ms, id)`) |
| 2 | `Prompt@0`, `Pre A@1`, `Pre B@2`, `Post A@3`; now=4 | Tool `▶ B · 2s`, no `+N` |
| 3 | `Prompt@0`, `Pre Agent@1`, `Post Agent@1.1`, `ChildStart x@1.1`; child rows (agent x) `Pre Read@2`; now=3 | parent Working `● 1 bg · <now>`; child Tool `▶ Read · 1s` |
| 4 | `Prompt@0`, `Pre Bash@1`, `Ask Bash@1`; now=31 | `⚠? permission: Bash cargo build`, age 30 s |
| 5 | `Prompt@0`, `Pre Bash@1`, `Ask Bash@1`, `PermissionDenied id@3`; now=4 | Working `● <now>`; `open_tools` empty |
| 6 | `Prompt@0`, `Post@1`, `Notification/auth_success@2` ×2; now=3 | Working `● <now>` |
| 7 | `Prompt@0`, `ChildStart x@1`, `Idle@5`, `ChildStop x@6`; now=9 | Prompt `○ your turn`, no `bg`; age 4 s (`ChildStop` does not set `ev_at`) |
| 8 | `Prompt@0`, `mtime_ms`=470; now=480 | Working, age 10 s |
| 9 | Pi: `agent_start@0`, `tool_execution_start A@1`, `tool_execution_end A@2`, `agent_end@3`; now=6 | Prompt `○ your turn`, age 3 s |
| 10 | `Prompt@0`, `CompactStart@1`; now=5 → Compacting `⟳ compacting · 4s`; then `CompactEnd@6`, now=7 | Working |
| 11 | `Idle@0`, `End@1` | gone |
| 12 | `Pre A@0` (orphan, no boundary before), `Idle@5`; now=8 | Prompt; `open_tools` empty |
| 13 | `SessionStart@0`; now=180 | Prompt `○ your turn`, age 3 min |
| 14 | `Idle@0`, `Pre A@1`; now=1.5 | Tool `▶ A`; and with only `Idle@0`, now=0.5 → Working (2 s debounce) |
| 15 | `SessionRow.failed`, `Prompt@0`; now=1 | Working `● <now>` with `✗` mark |
| 16 | `Prompt@0`, `Pre A@1` NULL id, `Post A@2` NULL id; now=3 | paired FIFO; Working `● <now>` |
| 17 | `Prompt@0`, `Pre Bash@1`, `mtime_ms`=1000; now=121 | `⏳ Bash … · 2m` with `?` mark |
| 18 | `Prompt@0`, `Pre Bash id1@1` (hook-denied), `Pre Edit id2@2`, `Post Edit id2@3`; now=4 | Working; `open_tools` empty (orphan rule) |
| 19 | `Prompt@0`, `ChildStart x@1`, `ChildStart y@2`, `ChildStop x@3`; now=4 | Working `● 1 bg · <now>` |
| 20 | `Idle@0`, `ChildStart x@1` (agent_id = x); now=5 | Prompt `○ your turn · 1 bg` (boundary returned by `turn_receipts`; `ChildStart` counted despite its `agent_id`) |
| 21 | `Prompt@0`, `Pre A@1`, `Pre B@2`, `Pre C@3`; now=4 | Tool `▶ C · 1s +2` |
| 22 | `Prompt@0`, `ChildStart x@1`, `End@2` | gone; `open_children` empty |
| 23 | `Prompt@0`, `Notification/permission_prompt@1` (no tool); now=2 | `⚠ permission_prompt` |
| 24 | `Prompt@0`; `harness_pid` dead | gone |
| 25 | `Prompt@0`; `harness_pid` reused by another process (start time differs) | gone |
| 26 | `Prompt@0`; `harness_pid IS NULL`; now=3600 | Working `◌ quiet · <now>`, mark `pid?`, on screen |
| 27 | three sessions: A Prompt age 5 s, B Waiting age 50 s, C Working age 1 s, D Prompt age 5 s with id < A | order B, D, A, C |
| 28 | `Prompt@0`, `Ask@1`; now=31.000 → `⚠?`; now=30.999 → `⚠` | boundary at 30 s |
| 29 | `Idle@0`; now=2.000 → Prompt; now=1.999 → Working | boundary at 2 s |
| 30 | `Prompt@0`; now=120.000 → `◌ quiet`; now=119.999 → `●` | boundary at 2 min |
| 31 | `Prompt@0`, `Pre A@1`; now=31.000 → `⏳`; now=30.999 → `▶` | boundary at 30 s |
| 32 | `Prompt@−20min`, `Pre A@−15min`; now | returned by `turn_receipts` (no boundary after it); Tool `⏳ A · 15m` with `?` mark; on screen |
| 33 | `Prompt@0`, `Pre Bash id1@1` (hook-denied), `PostToolUseFailure id1@2` (if step 0.5 says yes); now=3 | Working; `open_tools` empty |
| 34 | Cursor: `beforeSubmitPrompt@0`, `beforeShellExecution@1`, `preToolUse@1` (same call, ids per capture 12), `afterShellExecution@3`, `postToolUse@3`; now=4 | Working; `open_tools` empty |
| 35 | Cursor: `beforeSubmitPrompt@0`, `afterAgentThought@1`, `afterFileEdit@2`; now=3 | Working `● <now>`, age 1 s (`Tick` sets `ev_at`) |
| 36 | Cursor: `beforeSubmitPrompt@0`, `beforeReadFile@1` (no Post ever), `afterAgentResponse@2`, `stop@3`; now=6 | Prompt; `open_tools` empty (boundary closes) |
| 37 | Cursor: receipts only, `transcript_path` null, `conversation_id = c1`, no session row; `beforeSubmitPrompt@0`, `beforeShellExecution@1`; now=2 | receipt-only row titled from `cwd`, Tool `▶ Shell … · 1s`, no `ctx` |
| 38 | Cursor: as 37, then a session row with `native_id = c1` appears | the same row, now with title and `ctx` from the transcript; the receipts are attached to it; no duplicate row |
| 39 | Cursor: `stop@0`, `sessionEnd@1` | gone |

Tests 12, 20, 32, 37 and 38 go through a real index (`turn_receipts`, `live_sessions`), not a hand-built `Vec`. Tests 24 and 25 inject `alive` as a closure.

Real-session replay: the review 3 replay script is rewritten in Rust as a test helper over a copy of the index (`tests/live_replay.rs`, ignored by default, path from an env var). Step 1 ends with the three Claude sessions review 3 replayed showing no `▶ Bash ⏳` after a later `Post`, one Pi session showing `▶`/`○` transitions, and one Cursor chat from the Step 0 capture showing `▶`/`●`/`○` and leaving on `sessionEnd`.

## The live screen

`Mode::Live`. Full width `W`, height `H`.

```
 grok live · 3 agents · 1 waiting                                                      ← header
 ⚠ claude  Traces packet on Mini             permission: Bash cargo build --release  2m ⌖
   ● claude  review spec                     Read docs/traces/SPEC-live.md            4s
   ○ claude  snapshot tests                  your turn                                1m
 ● pi      khoj worker                       Bash rg --files                          1s
 ▶ cursor  grok-build                        Shell cargo test -p xai-grok-traces      8s
 ◌ claude  fork lead  ctx 142k               quiet · Edit src/views/traces_modal.rs   4m ✗
 enter pane   o open   j/k   c children   q quit                                          ← hints
```

Row = `glyph source title [ctx] state-text age marks`. Columns: glyph 1, source 7 (`source_label`), title 28 (`clean_name`; `label` for children), ctx 13 (`ctx Nk`, or `ctx Nk/Mk` when a window is known; blank when `None`), state text fills, age 4 right-aligned (`fmt_age`), marks 4. Truncation with `…` uses `modal_window.rs` `word_wrap` on one line, never splitting a token. Title and state text are identical collapsed and expanded.

Selection kept by session id across polls (`keep_selection`). A child whose parent is not on screen is a root titled `↑ <parent title>`.

Keys:

| Key | Action |
|---|---|
| `j`/`k`, arrows, wheel | move |
| `c` | collapse/expand the selected root (default expanded; collapsed shows `+N` after the title; a collapsed root with a `Waiting` child shows `⚠` on the root) |
| `o`, or `Enter` with no pane known | open trajectory (`Mode::Trajectory`, `include_children`); Esc returns to live |
| `Enter` with a pane known | `herdr agent focus <pane_id>` |
| `q`, Esc | `grok live`: exit process. `/traces`: back to list |
| `/` | existing search over title and state text |

### Trajectory with children inline

`EventFilter.include_children`. `list_events` unions the children's non-noise events by timestamp, each tagged `child: Option<(session_id, label)>`. A child event renders as the existing event row one depth deeper with `label ·` before the header. Collapsed turns count child events in `+N`. Children are never reordered among themselves. Snapshots: two interleaved children collapsed and expanded; one child with zero events.

### Jump to pane (optional)

If `herdr` is on `PATH`: poll `herdr pane list` every 10 s on its own worker, modelled on `poll_remote` (own rx, own `Instant`), never on `spawn_refresh`. Map `agent_session.value` to `SessionRow.native_id`. Matched rows show `⌖`. `Enter` runs `herdr agent focus <pane_id>` (step 0.8). Failure → one-line `TracesNotice`. Without herdr nothing is polled and `Enter` opens the trajectory.

### Render states (snapshot tests, `TestBackend`, `render_tests.rs` pattern)

1. Empty: `no live agents · <N> hook receipts in the last hour · grok-traces doctor`; N = 0 ever → add the installer line.
2. One row per state 2–6 plus a quiet row, one `✗`, one `?`, one `pid?`, one `⚠?`, W=120.
3. Same at W=80: ctx column drops, title 20.
4. Overflow: 50 rows, H=20, selection at bottom.
5. Children expanded; collapsed; collapsed with a `Waiting` child.
6. Index tripped: header `index tripped · grok-traces doctor --reset`, body still renders.
7. `⌖` present and absent; herdr absent entirely (no marks, hint reads `enter open`).
8. Parent `● 2 bg · <now>`; parent `○ your turn · 1 bg`.
9. A receipt-only Cursor row (title from `cwd`, blank ctx) beside a Cursor row with a transcript.

pty proof: launch `grok live`; pipe `UserPromptSubmit`, `PreToolUse`, `PermissionRequest` for one session into `grok-traces hook claude` (the pty harness process is the "harness PID"; the hook walks up to it); assert `⚠` within 2 s; pipe `PostToolUse`; assert `●` on the next poll; pipe `SessionEnd`; assert the row is gone.

## Non-goals (v1)

Codex, OMP, Grok, bb, DSH rows. Remote hosts. A Cursor `Waiting` state. Web UI, websocket, notification center. Live tool output (the inspector exists). Transcript reading outside the index. Model window tables.

## Delivery order

0. **Gate.** `capture.sh` wired into Claude and Pi (diffs shown to Sab first); the 16 captures; answers in `MAP.md`; (U) markers here replaced.
1. `is_terminal` fix + tests. Schema 7, receipt columns, PID capture and `alive`, Cursor `native_id` and receipt-only rows, `live_sessions`, `turn_receipts`, `receipt_count_since`, usage fields, `norm` + `live.rs` + the 39 sequence tests + the replay helper, Claude installer events, Cursor per-event reply + installer events, Pi extension rewrite + `hooks/test.sh`. `cargo test -p xai-grok-traces`.
2. `Mode::Live` render, ordering, snapshots 1–6, 8 and 9.
3. `grok live` subcommand, Esc exits, pty proof.
4. Trajectory `include_children` + snapshots.
5. Herdr jump, snapshot 7.

Each step merges alone. 0–3 are the deliverable.

## Decisions log

1. Liveness = harness PID + start time, or `SessionEnd`. No presence timers (Sab, review 3).
2. v1 = Claude Code + Pi + Cursor, fully correct; Codex next; others stay in history (Sab, review 3 and its Cursor amendment).
8. Cursor is keyed by `conversation_id`; a null `transcript_path` yields a receipt-only row, never a dropped receipt.
9. Captures use `hooks/capture.sh`, not a `--dump` flag (Opus, 43810bdf).
3. Herdr optional, jump only (Sab, review 3).
4. Done is gone: an `End` receipt removes the row on the next poll; nothing lingers.
5. Transcript growth clears nothing; `⚠?` is the honest display for a granted permission until step 0.3 finds a grant signal.
6. Orphan `Pre`: a `Post` closes its own `Pre` and every older open `Pre` (review 3, blocker 1).
7. `/traces` does not default to live.
