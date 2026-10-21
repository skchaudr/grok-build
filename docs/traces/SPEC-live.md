# SPEC: `grok live` screen

Rev 5.1 (Fable 5.1, 2026-10-21) after reviews 1–5 (7390998f, 5d43a539, ef69d3f7, 5a50b946, ad989f55) and Sab's amendments in review 3. Rev 5 folded review 4's B1–B6 and m1–m8; rev 5.1 folds review 5's four lead decisions (D1–D4) and five mechanical fixes. Ready for the implementer after the lead's grep check. (U) = not verified; Step 0 closes each before code. Paths under `crates/codegen/`. Read `MAP.md` first. Implementer: a compacted Opus lead with Sonnet 5.5 subagents.

## Scope (Sab, 2026-10-21)

v1 is **Claude Code, Pi and Cursor** (Cursor matters uniquely this month), all three completely right: every state, every test row, verified on real sessions. Codex comes later. Other sources stay in `/traces` history and never appear on the live screen. The data model stays source-agnostic (a later source adds normalizer rows and tests, nothing else). **sab-air is in v1**: it runs as many sessions as the mini, already has Claude, Cursor and Pi hooks feeding its own index, and `/traces` already pulls its session list every 10 s. Cut from this spec: the Grok notification route and every OMP/Grok/bb/DSH/Codex rule.

## The tool

A full-screen, always-on view on a second screen that answers, per running agent: what it is doing now, whether it is waiting on me, what it dispatched and what those children are doing, and how full its context is. One key jumps to the agent's pane when Herdr is present.

## Architecture

- No daemon, watcher or second store. Hooks call `grok-traces hook <source>`, which records a receipt and re-indexes the transcript. The screen polls the index at 1 s (`poll_live_at`).
- **Liveness is the harness process, not a timer.** Each receipt records the harness PID and its start time. A session is on the screen from its first receipt until its newest boundary is `End`, that process is gone, or a newer session of the same source has taken over the same process (Claude `/clear`, `/resume`, Pi `session_shutdown(new|resume|fork)` switch sessions inside one PID; review 4, B2). There is no presence window and no cap. Timers exist only for display: `⏳` at 30 s, `⚠?` at 30 s, the `?` mark at 2 min, the 2 s `Stop` debounce.
- State is a fold over the session's current turn of receipts. Transcript growth clears nothing (review 2, E).
- Context comes from the index, refreshed per hook.
- One modal, two hosts: `TracesModalState` gains `Mode::Live`. `/traces` opens in list mode as today. `grok live` boots the pager straight into `Mode::Live` full screen; Esc exits the process. Soft subcommand beside `Command::Dashboard` (`rg -n "Dashboard" crates/codegen/xai-grok-pager*/src`). Named `live` because `Command::Trace` exists.
- Herdr is optional. Its only use is the jump key; without it `Enter` opens the trajectory.
- **Remote hosts fold their own state.** `grok-traces live --json` on the Air runs the same fold against the Air's index, checks the Air's PIDs, and measures ages on the Air's clock, returning finished rows with durations as numbers. The mini adds its own time since the fetch, renders the durations, and never reads the Air's receipts or checks a remote PID. The wire envelope carries a schema number; a mismatch gets one notice row, not guessed states.

## Step 0: hard gate

### 0a: the capture mechanism (exists: `hooks/capture.sh`, 43810bdf)

`capture.sh <source> [event]` saves the raw stdin payload plus the hook's process chain (`pid ppid lstart comm` up to PID 1) to `~/.grok/traces/capture/<source>/<ts>-<pid>-<event>.txt`, prints only Cursor's neutral replies, exits 0. The output holds real session text and is never committed. No `--dump` flag is built.

**Wired on both Macs (2026-10-21, Sab: in scope = go).** Installed as `~/.grok/bin/traces-capture.sh`. Claude: 15 events, async, matcher `*`, in `~/.claude/settings.json`; captures start with new sessions. Cursor: 18 events in `~/.cursor/hooks.json`. Pi: `~/.pi/agent/extensions/traces-capture.ts` (from `hooks/pi/capture.ts`; sends the event plus `process.pid`, strings clipped to 300). Backups `*.bak-capture-20261021`. Output in `~/.grok/traces/capture/<source>/` on each machine. The Cursor file `20261021T041502-53488-stop.txt` on the mini is a smoke test, ignore it. Nothing in steps 1–5 starts until every capture below has its answer in `MAP.md` and the (U) markers here are replaced.

On Cursor's gating events `capture.sh` prints the neutral reply (`{"continue":true}` for `beforeSubmitPrompt`, `{"permission":"allow"}` for `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `{}` otherwise). How Cursor combines several hooks on one event is capture 11. If Cursor blocks anyway, the capture lines come off the gating events first.

### 0b: captures. Each has a command, a yes/no closing, and the fallback if the answer is no

Capture sessions to run on each machine (wiring is done; nothing to install):
- Claude: a new session that dispatches a subagent, gets a permission granted, gets one denied by Sab, gets a Bash call denied by a `PreToolUse` hook (`grep -r`), runs `/compact`, and issues two tools in one assistant message, then exits.
- Pi: `pi`, two tool calls, `/exit`.
- Cursor: one agent chat with a shell command, a file read, a file edit, an MCP call, a subagent if offered; close the chat; then quit Cursor.
- Reading: every command below runs from `cd ~/.grok/traces` in zsh. A capture file is the JSON payload on line 1, then a `--- process chain …` line and the chain, so `head -1 "$f"` is the payload alone. `rg --files-without-match` (not `rg -L`, which is `--follow`) lists files that lack a field; an empty list means every file has it. On the Air, run `ssh sab-air 'cd ~/.grok/traces && <command>'`.

| # | Question | Command | Yes when | Fallback if no |
|---|---|---|---|---|
| 1 | Claude: `tool_use_id` on `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionRequest`, `PermissionDenied` | `for e in PreToolUse PostToolUse PostToolUseFailure PermissionRequest PermissionDenied; do rg --files-without-match '"tool_use_id"' -g "*-$e.txt" capture/claude; done` | prints nothing | pairing by the orphan rule only |
| 2 | Claude: `agent_id` on `SubagentStart`/`SubagentStop` equals the `agent_id` on the tool receipts fired inside that child | `rg -I -o '"agent_id":"[^"]+"' -g '*-SubagentStart.txt' -g '*-SubagentStop.txt' -g '*-PreToolUse.txt' -g '*-PostToolUse.txt' capture/claude \| sort \| uniq -c` | one id has a count ≥ 3 and appears in a Start, a Stop and a tool file (`rg -l` that id to confirm) | child receipts cannot be separated: parent `Tool` state is disabled while `open_children` is non-empty; child rows use the no-receipt rule |
| 3 | Claude: any receipt between `PermissionRequest` and `PostToolUse` on a grant | `ls capture/claude \| sort \| sed -n '/PermissionRequest/,/PostToolUse/p'` | a line with another event name sits between the two | `⚠?` form stands; row 4 as written |
| 4 | Claude: `transcript_path` on `PermissionRequest`, `PermissionDenied`, `Notification` | `for e in PermissionRequest PermissionDenied Notification; do rg --files-without-match '"transcript_path"' -g "*-$e.txt" capture/claude; done` | prints nothing | `session_id` from `native_session` → `trace_sessions.native_id` |
| 5 | Claude: a hook-denied call emits `PostToolUseFailure` or `PermissionDenied` | `ls capture/claude \| sort \| rg -A1 'PreToolUse'` after the `grep -r` denial; the denied call's file is the one whose payload has `"command":"grep -r` | the next line is a `PostToolUseFailure` or `PermissionDenied` file with a timestamp within 1 s | close-older rule stays for Claude; if yes, Claude pairs strictly by id (fix 5) |
| 6 | Claude: process chain reaches `claude` | `for f in capture/claude/*-UserPromptSubmit.txt; do tail -n 12 "$f" \| awk '{print $NF}' \| tr '\n' ' '; echo; done` | the first name whose basename (leading `-` stripped) is not in `sh bash zsh dash fish` is `claude` or `node`, in every line; `tail -n 12 \| rg claude` shows one `lstart` per session | PID unknown: row stays until `End`, mark `pid?` |
| 7 | Claude: `SessionEnd` receipts arrive for async hooks | `ls capture/claude \| rg -c SessionEnd` against the number of sessions exited in the capture | counts match | PID liveness and the same-process supersede rule cover it |
| 8 | Herdr: `herdr agent focus <pane_id>` focuses the pane whose session matches a live row | `herdr agent list` (gives `pane_id` and `agent_session.value` per row), then `herdr agent focus <pane_id>` | the pane takes focus | `Enter` opens the trajectory |
| 9 | Pi: tool-call id field | closed from Pi source: `toolCallId` on `tool_execution_start` and `tool_execution_end`, optional `parentToolCallId` | — | — |
| 9b | Pi: `getSessionId()` and `getSessionFile()` are non-empty in every event's `ctx`, and Pi exposes no approval event | `rg -l '"session_file":""' capture/pi; rg -l '"session_id":""' capture/pi` | both print nothing (Pi sessions need a restart to load `traces-capture.ts` (closed: yes, Gate 0)) | Pi receipts key by `session_id` → `native_id`; Pi has no `Waiting` state either way |
| 10 | Hook-to-index latency | after step 1 exists: `grok-traces list --json --limit 1` polled at 1 s during a session, receipt `at_ms` against `last_event_at`; ten samples | median and max written in `MAP.md` | none; it is a number |
| 11 | Cursor: gating events still run with three hooks on one event (moshi, grok-traces, capture) all replying | in the capture chat ask for `ls` and a file read, then `ls capture/cursor \| rg -c 'beforeShellExecution\|beforeReadFile'` | both ran in Cursor and the count is ≥ 2 | remove `capture.sh` from the gating events; capture only `after*`, `post*`, `stop`, `session*` |
| 12 | Cursor: per event, the fields `conversation_id`, `generation_id`, `workspace_roots`, `cwd`, `transcript_path` (null or not), `hook_event_name`, the tool-call id if any, `tool_name` where present (`beforeShellExecution` has none; record its command field name), and the firing order of `beforeReadFile` vs `preToolUse` for one read (m2) | `for f in capture/cursor/*.txt; do echo "$f"; head -1 "$f" \| jq -c keys; done` and `ls capture/cursor \| sort \| rg 'beforeReadFile\|preToolUse'` | every event has `conversation_id`; the id and order facts are recorded | no tool id: Cursor pairs FIFO by name; the order fact decides which `Pre` the orphan rule keeps |
| 13 | Cursor: `conversation_id` equals the transcript file stem under `~/.cursor/projects/*/agent-transcripts/<id>/<id>.jsonl` | `rg --files ~/.cursor/projects \| rg "<captured id>"` | one file matches | Cursor rows are receipt-only (title = `cwd` basename, no `ctx`, no `now`) |
| 14 | Cursor: which ancestor is stable for the life of a chat | `for f in $(ls capture/cursor/*.txt \| sort \| sed -n '1p;$p'); do tail -n 12 "$f"; done` | the first non-shell ancestor has the same `pid` and `lstart` in both | the first non-shell ancestor is used, whatever it is; `sessionEnd` is the only reliable end for Cursor (B3) |
| 15 | Cursor: `sessionEnd` fires when a chat is closed or switched, not only when Cursor quits | `ls capture/cursor \| rg sessionEnd` after closing the chat, before quitting | prints a file | a finished Cursor chat stays `○ your turn` until Cursor quits; Sab decides whether that is acceptable or Cursor gets a display-only hide after `stop` Decided 2026-10-21 (traces lead): keep this fallback; Sab tunes after use. |
| 16 | Cursor: `subagentStart`/`subagentStop` child id field, and whether child tool events carry a different `conversation_id` | `for f in capture/cursor/*-subagent*.txt; do head -1 "$f" \| jq -c '{keys: keys, conversation_id}'; done` | fields recorded | Cursor children are not separated; parent `Tool` state is disabled while `open_children` is non-empty |
| 17 | Air: captures 1–7, 9b and 11–16 repeated on sab-air (already wired there) | each command above inside `ssh sab-air 'cd ~/.grok/traces && …'` | each item's own closing, per host | per-item fallbacks above apply per host |

(`\|` in the command column is a markdown escape; type a plain pipe.)

Capture 18 (the Air's binary and `live --json` over ssh) needs the step-3 binary, so it lives in step 3, not here (B4). Any further change to the Air's hook configs needs Sab's OK as an exception to the Air read-only rule; show the diff first.

## Data (`xai-grok-traces`)

Schema 7 and a `PARSER_VERSION` bump. No ALTER path exists; an old index is replaced through the existing `.rebuild.lock` path. Note it in MAP.md.

### `is_terminal` (live `/traces` bug, fix first)

`index.rs` `is_terminal` matches `complete` and `turnend`, so Codex `task_complete` and Pi `turn_end` mark a session `completed` on every turn. New rule: the squashed type ends with one of `sessionend`, `sessionshutdown`, `runend`, `agentend`; `complete` and `turnend` are gone. Tests: `task_complete`, `turn_end`, `turn/complete` are not terminal; `session_end`, `session/end`, `agent_end`, `session_shutdown` are. Ledger statuses change on rebuild; that is the fix.

### Receipts

`trace_receipts` gains `detail TEXT`, `tool_use_id TEXT`, `agent_id TEXT`, `harness_pid INTEGER`, `harness_start INTEGER` (process start, Unix seconds), `harness_chain TEXT` (see "Harnesses run by bb"). `Receipt` and the insert in `receipts.rs` change with it. `hook::extract` adds key lists `["tool_input","toolInput","input"]`, `["tool_use_id","toolUseId","toolCallId"]`, `["agent_id","agentId"]`, `["notification_type","notificationType"]`, `["harness_pid"]`. For source `cursor` the session key list is `["conversation_id"]` first (B6). `detail` = `classify::call_detail(tool, tool_input)` for tool and permission events; for `Notification` it is the notification type; for Claude `SessionStart` it is the payload's `source` (`startup`, `resume`, `clear`, `compact`). Cursor `before*` events carry no `tool_name`; the receipt's `tool` is set from the event: `beforeShellExecution` and `afterShellExecution` → `Shell` (detail = the command field, first line), `beforeMCPExecution` and `afterMCPExecution` → `MCP` (detail = `server.tool` when present), `beforeReadFile` → `Read` (detail = the path), `preToolUse`, `postToolUse`, `postToolUseFailure` → their `tool_name` (B6; review 5 fix 2).

Harness process: if the payload carries `harness_pid` (Pi sends `process.pid`), use it. Otherwise walk up from the hook's parent PID, skipping every ancestor whose `comm` basename, with a leading `-` stripped, is one of `sh`, `bash`, `zsh`, `dash`, `fish` (B3: the real chain is `sh`, `/bin/zsh`, `claude`, `-zsh`; literal matching stopped at `/bin/zsh`). Start time on macOS from `proc_pidinfo(PROC_PIDTBSDINFO).pbi_start_tvsec` (libproc; no new crate if `sysinfo` or `libc` is already a dependency, else `ps -o lstart= -p`). `live::alive(pid, start)` = `kill(pid, 0)` succeeds and the start time matches. Linux: `/proc/<pid>/stat` field 22; not needed for v1. Unit test: the chain from the Claude smoke test resolves to `claude`; a chain `sh, -zsh, Cursor Helper (Plugin)` resolves to the helper.

`trace_sessions` gains `harness_pid`, `harness_start` (from the latest receipt), `context_tokens`, `context_window`. `ended` is not a stored flag: it is "the session's newest boundary receipt normalizes to `End`" (B2: Claude `--resume`, Pi `reload` and a reopened Cursor chat reuse the id; a sticky flag would hide them forever). `SessionRow` gains the four columns plus `native_id` (column exists; add to `SESSION_COLS`, `session_row`, `session_to_json/from_json`) and `mtime_ms` (already selected).

Same-process supersede (B2): for sources `claude` and `pi`, when two sessions share `(source, harness_pid, harness_start)`, only the one with the newest receipt is live; the older is treated as gone (its process was reused by `/clear`, `/resume`, `session_shutdown(new|resume|fork)`). Not applied to `cursor`, where one process hosts many chats.

When `path` does not resolve, `session_id` falls back to `native_session` matched against `trace_sessions.native_id` in the same source. For Cursor this is the primary path: `transcript_path` may be null (review 1, data #2), so every Cursor receipt is keyed by `conversation_id`. `links.rs` `path_links` has no `cursor` arm today (all 300 Cursor rows have `native_id = NULL`); it gains one with `native_id` = the transcript file stem (`agent-transcripts/<id>/<id>.jsonl`, layout confirmed; capture 13 says whether it equals `conversation_id`). A Cursor receipt whose `conversation_id` matches no session row yet is stored with `session_id = NULL` and `native_session` set; `live_sessions` groups such receipts by `(source, native_session)` into a receipt-only row (title = `cwd` basename, no `ctx`, no `now`), and `turn_receipts` accepts either key. When the transcript appears, the session row takes over and the receipts match it by `native_id`. Receipt-only rows leave when their newest boundary is `End` or the Cursor process is gone, and their receipts are pruned with everyone else's at 100k (m5).

`last_receipt` and `last_receipt_at` order by `(at_ms, id)`.

New queries:

- `live_sessions() -> Vec<SessionRow>`: sessions in sources `claude`, `pi`, `cursor` with at least one receipt and whose newest boundary receipt is not `End`, plus receipt-only Cursor rows (above). The pager then drops rows that fail `alive(harness_pid, harness_start)` and rows superseded by a same-process newer session; a session with `harness_pid IS NULL` stays until its newest boundary is `End`.
- `turn_receipts(session_id) -> Vec<LiveReceipt>`: the session's newest turn-boundary receipt (events that normalize to `Idle`, `Prompt`, `End`) **and** every receipt after it, by `(at_ms, id)`; when no boundary exists, every receipt. **Plus every `ChildStart`/`ChildStop` receipt of the session at any age** (B1: a child started before the boundary runs across `Stop`). Rows with `agent_id` set are included and tagged. Fields: `session_id, agent_id, event, tool, detail, tool_use_id, at_ms`. Because the boundary itself is returned, `last_ev` is never undefined (review 3, blocker 2).
- `receipt_count_since(ms) -> i64` for the empty state.
- `live::fold_all(index, now, alive) -> Vec<LiveRow>`: `live_sessions` → `turn_receipts` → state per session, children attached. `LiveRow { session_id, parent_id, source, title, label, state: LiveState, text, age_ms, since_ms: Option<i64>, quiet_ms: i64, bg: u32, bg_quiet: bool, failed: bool, pid_unknown: bool, native_id, context_tokens, context_window, host: Option<String> }`. The row carries **state and numbers only, no glyphs and no durations in `text`** (D1): `since_ms` is the state's own duration (`since(pre)`, `since(ask)`, `since(compact start)`), `quiet_ms` = `now − max(ev_at, last_event_at, mtime_ms)` feeds the `◌` text and the `?` mark, `bg_quiet` is D2's `bg?`. `live::render(row, extra_ms) -> (glyph, text, marks)` computes glyph, `⏳`/`⚠?`/`◌`, `?` and `bg?` from the numbers plus `extra_ms` (0 for local rows, `fetched_at.elapsed()` for remote). One render path for both. `stale` is dropped (D3). Serde both ways with `#[serde(default)]` on every field; this is the wire format.

### Remote (`remote.rs`, bin)

- Bin: `grok-traces live --json` prints the envelope `{"live_schema": 1, "rows": [LiveRow…]}` for this host, `host` = `None`. `LIVE_SCHEMA` is a const in `live.rs`, bumped on any `LiveRow` change (B4: the Cargo version string does not change between builds).
- `remote::fetch_live(target) -> Result<Fetched<LiveRow>>`: `ssh … '~/.grok/bin/grok-traces live --json'` with the existing `BatchMode`, 2 s connect, 4 s kill. Rows get `host = Some(label)` (label = the part after `@`, so `sab-air`) and `sab-air:`-prefixed `session_id`/`parent_id`, exactly as `fetch_sessions` tags them. The result is stored with `fetched_at: Instant`. Cached in memory per host like `cached_sessions`.
- **Clock:** on every render the mini calls `live::render(row, fetched_at.elapsed())`, which adds that to `age_ms`, `since_ms` and `quiet_ms`, clamps at 0, and recomputes glyph and marks (D1). Cached rows therefore keep advancing and change state marks between fetches and across a failed fetch (B4).
- **Schema gate:** exit 2 (unknown subcommand), unparseable output, or `live_schema != LIVE_SCHEMA` yields no rows and `Fetched.note = "<host>: update grok-traces (live_schema <n or none>)"`, rendered as one dim row under the header. Never guessed states.
- **Header:** `<host> ●` after a successful fetch within the last poll; `<host> ✗` when the latest fetch failed, with the cached rows still shown and advancing. There is no third glyph (B4 contradiction resolved; `◌` was dropped).
- Poll cadence: the existing `poll_remote` 10 s, on its worker.

### Usage

`NormalizedEvent.context_tokens`, `context_window`: Claude = `usage.input_tokens + cache_read_input_tokens + cache_creation_input_tokens` of that message, no window. Pi: its session file records usage per message (closed: yes, Gate 0; `usage` keys `input`, `output`, `cacheRead`, `cacheWrite`, `totalTokens`, no window); the window stays `None`. Codex's `last_token_usage` rule from rev 3 is kept in the normalizer for later but has no live-screen test in v1. `trace_event_usage` and `trace_sessions` gain both columns; the session value is from the last event that had one.

### Hook events

Claude installer `CLAUDE_EVENTS` adds `PermissionRequest`, `PermissionDenied`, `Notification`, `SubagentStart`, `PreCompact`, `PostCompact`, `StopFailure` (`,1` = takes `matcher: *`).

Pi extension `hooks/pi/traces.ts` is rewritten:
- subscribes to `session_start`, `agent_start`, `tool_execution_start`, `tool_execution_end`, `agent_end`, `session_shutdown`; `turn_end` and `tool_result` are not sent; the debounce is removed.
- payload: `event`, `tool`, `tool_use_id` (field name per step 0.9), `cwd`, `session_id`, `session_file`, `harness_pid: process.pid`.
- `hooks/test.sh` covers the new payload shape.

Cursor installer: `CURSOR_EVENTS` becomes `sessionStart`, `beforeSubmitPrompt`, `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `postToolUse`, `postToolUseFailure`, `afterShellExecution`, `afterMCPExecution`, `afterFileEdit`, `afterAgentThought`, `afterAgentResponse`, `subagentStart`, `subagentStop`, `preCompact`, `stop`, `sessionEnd` (the set present in `~/.cursor/hooks.json` today). The gating ones are wired only once `hook cursor` replies per event: `{"continue":true}` for `beforeSubmitPrompt`, `{"permission":"allow"}` for `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile` (the last is closed: yes, Gate 0; the read completed after the neutral reply), `{}` for everything else. The reply is written before the receipt is recorded, so an index failure never blocks Cursor. Test in `hooks/test.sh` and a unit test on the reply function. Cursor hooks carry no `harness_pid`; the ancestry walk finds the Cursor process (capture 14).

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

`SessionStart` is `Idle` (review 2, G), except a Claude `SessionStart` whose `detail` is `compact`, which is dropped: it fires mid-turn after auto-compaction and must not end the turn (m3, closed: yes, Gate 0). `(✗)` sets the failed mark. `Tick` is a non-boundary event that sets `ev_at` and yields `Working`; it closes nothing. A `Tick` that lands within 2 s after an `Idle` does not become `last_ev` (m4: Cursor's `afterAgentResponse` can arrive after `stop`). Pi never shows `Waiting`, children or compaction. Cursor never shows `Waiting`. A Cursor `Pre` that is both `preToolUse` and `beforeShellExecution` for one call produces two `Pre`s; the matching `Post`s (`postToolUse`, `afterShellExecution`) close them by id, or by the orphan rule when ids are missing (capture 12). Cursor's `preCompact` has no end event, so `Compacting` clears on the next `Ev`, as rule 3 already says.

## Live state machine (`live.rs` in the traces crate)

### Inputs per session

- `evs`: `turn_receipts` normalized, `None`s dropped. The parent fold uses rows with `agent_id IS NULL`, plus `ChildStart`/`ChildStop` whatever their `agent_id`. A child row folds its own `agent_id` rows.
- `open_tools`: ordered list of `Pre`s not yet closed. Pairing is per source, `strict_ids(source)`:
  - **Strict** (ids exist and orphans are guaranteed closed): a `Post` closes only the `Pre` with its `tool_use_id`/`toolCallId`. Pi is strict today (`toolCallId`, no hook denial). Claude and Cursor become strict when capture 1 (ids present) and capture 5 (a hook-denied call emits `PostToolUseFailure`/`PermissionDenied`) both say yes for that source.
  - **Close-older** (otherwise): a `Post` closes its own `Pre` (by id, or with a NULL id the oldest open `Pre` of the same tool name) **and every open `Pre` older than that `Pre`** (review 3, blocker 1: 22 of 151 real turn ends carried an orphan Bash `Pre`). A `Pre` of the same name as an open NULL-id `Pre` closes the older one. Known cost (m1): with real parallel calls `Pre A, Pre B, Post B` closes A early, which mostly costs the `+N` count (37 such closes in the replay).
  - Every boundary closes all, in both modes. Tests 18 and 49 state both outcomes.
- `open_children`: `agent_id`s with a `ChildStart` and no later `ChildStop`, computed over the whole session (B1), not the current turn. `End` clears it; `Idle` and `Prompt` do not (children run across `Stop`). A lost `ChildStop` never expires the child (D2): when an open child's `agent_id` has had no receipt for 10 min, the parent's count renders `· N bg?` (display only, same `?` convention as the marks).
- `last_ev`: the newest `Ev` that is not `ChildStart`/`ChildStop`. `Tick` counts. Always defined, because the boundary is in `evs`; if the session has no boundary at all, `last_ev` is the newest `Ev` of any kind, or `Prompt` when there is none.
- Clocks, local ms: `ev_at` = `at_ms` of `last_ev` (`ChildStart`/`ChildStop` never set it); `last_event_at` and `mtime_ms` from the session row. `age = now − max(ev_at, last_event_at, mtime_ms)`. `since(x) = now − x.at_ms`.

Boundaries: `<` on the left, `≥` on the right. `⏳` at `since(pre) ≥ 30 s`; `⚠?` at `since(ask) ≥ 30 s`; Prompt shows at `since(idle) ≥ 2 s`; Quiet text at `age ≥ 2 min`; `?` mark at `≥ 2 min`.

### States

First match wins. `<now>` = `SessionRow.now`. Every row has an age column showing combined `age`.

| # | State | Rule | State text |
|---|---|---|---|
| 1 | gone | newest boundary is `End`, or `!alive(harness_pid, harness_start)`, or superseded by a same-process newer session (claude, pi) | row removed on the next poll |
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

Ordering: `Waiting`, `Prompt`, then `age` ascending, then `session_id` ascending; local and remote rows interleave by these keys alone, the host is not a sort key. Children directly under their parent in the same order; a remote child's parent is matched by the prefixed `parent_id`.

### Sequence tests (`live.rs`, one named test each; `@` is `at_ms` in seconds; ids in listed order unless said; `now` as stated; all sessions alive)

| # | Sequence | Expected |
|---|---|---|
| 1 | `Prompt@0`, `Pre A@1.00`, `Post A@1.05` inserted as ids (Post, Pre); now=2 | Working `● <now>`; `open_tools` empty ("later" is by `(at_ms, id)`) |
| 2 | `Prompt@0`, `Pre A@1`, `Pre B@2`, `Post A@3`; now=4 | Tool `▶ B · 2s`, no `+N` |
| 3 | `Prompt@0`, `Pre Agent@1`, `Post Agent@1.1`, `ChildStart x@1.1`; child rows (agent x) `Pre Read@2`; now=3 | parent Working `● 1 bg · <now>`; child Tool `▶ Read · 1s` |
| 4 | `Prompt@0`, `Pre Bash@1`, `Ask Bash@1`; now=31 | `⚠? permission: Bash cargo build`, age 30 s |
| 5 | `Prompt@0`, `Pre Bash@1`, `Ask Bash@1`, `PermissionDenied id@3`; now=4 | Working `● <now>`; `open_tools` empty |
| 6 | `Prompt@0`, `Post@1`, `Notification/auth_success@2` ×2; now=3 | Working `● <now>` |
| 7 | `Prompt@0`, `ChildStart x@1`, `Idle@5`, `ChildStop x@6`; now=9 | Prompt `○ your turn`, no `bg`; age 4 s (`ChildStop` does not set `ev_at`; the `ChildStart` before the boundary is returned by `turn_receipts`, B1) |
| 8 | `Prompt@0`, `mtime_ms`=470; now=480 | Working, age 10 s |
| 9 | Pi: `agent_start@0`, `tool_execution_start A@1`, `tool_execution_end A@2`, `agent_end@3`; now=6 | Prompt `○ your turn`, age 3 s |
| 10 | `Prompt@0`, `CompactStart@1`; now=5 → Compacting `⟳ compacting · 4s`; then `CompactEnd@6`, now=7 | Working |
| 11 | `Idle@0`, `End@1` | gone |
| 12 | `Pre A@0` (orphan, no boundary before), `Idle@5`; now=8 | Prompt; `open_tools` empty |
| 13 | `SessionStart@0`; now=180 | Prompt `○ your turn`, age 3 min |
| 14 | `Idle@0`, `Pre A@1`; now=1.5 | Tool `▶ A`; and with only `Idle@0`, now=0.5 → Working (2 s debounce) |
| 15 | `SessionRow.failed`, `Prompt@0`; now=1 | Working `● <now>` with `✗` mark |
| 16 | `Prompt@0`, `Pre A@1` NULL id, `Post A@2` NULL id; now=3 | paired FIFO; Working `● <now>` |
| 17 | `Prompt@0`, `Pre Bash@1`, `mtime_ms`=1s; now=121 | `⏳ Bash … · 2m` with `?` mark |
| 18 | `Prompt@0`, `Pre Bash id1@1` (hook-denied), `Pre Edit id2@2`, `Post Edit id2@3`; now=4 | close-older: Working, `open_tools` empty. Strict (capture 5 yes, so a `PostToolUseFailure id1` would also have arrived; without it in this row): Tool `▶ Bash · 3s` (flips with capture 5, like 49) |
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
| 32 | `Prompt@0`, `Pre A@300`; now=1200 | returned by `turn_receipts` (no boundary after it); Tool `⏳ A · 15m` with `?` mark; on screen |
| 33 | `Prompt@0`, `Pre Bash id1@1` (hook-denied), `PostToolUseFailure id1@2` (if step 0.5 says yes); now=3 | Working; `open_tools` empty |
| 34 | Cursor: `beforeSubmitPrompt@0`, `beforeShellExecution@1`, `preToolUse@1` (same call, ids per capture 12), `afterShellExecution@3`, `postToolUse@3`; now=4 | Working; `open_tools` empty |
| 35 | Cursor: `beforeSubmitPrompt@0`, `afterAgentThought@1`, `afterFileEdit@2`; now=3 | Working `● <now>`, age 1 s (`Tick` sets `ev_at`) |
| 36 | Cursor: `beforeSubmitPrompt@0`, `beforeReadFile@1` (no Post ever), `afterAgentResponse@2`, `stop@3`; now=6 | Prompt; `open_tools` empty (boundary closes) |
| 37 | Cursor: receipts only, `transcript_path` null, `conversation_id = c1`, no session row; `beforeSubmitPrompt@0`, `beforeShellExecution@1`; now=2 | receipt-only row titled from `cwd`, Tool `▶ Shell … · 1s`, no `ctx` |
| 38 | Cursor: as 37, then a session row with `native_id = c1` appears | the same row, now with title and `ctx` from the transcript; `live_sessions` counts the receipts matched by `native_id` as that session's, and the receipt-only grouping excludes receipts whose `native_session` matches any session row; no duplicate row |
| 39 | Cursor: `stop@0`, `sessionEnd@1` | gone |
| 40 | remote: `fetch_live` returns a Waiting row `age_ms=5000`, `since_ms=5000`, fetched 3 s ago; a local Working row age 1 s | order: remote Waiting first; its age renders `8s`; `session_id` is `sab-air:…` |
| 41 | remote: `age_ms=−400`, fetched 0 s ago | renders 0 |
| 42 | remote: ssh exits 2, or output has `live_schema: 0` | no rows; note `sab-air: update grok-traces (live_schema …)`; local rows unaffected |
| 43 | remote: unreachable after one good fetch 12 s ago, row had `age_ms=1000` | cached row kept, age renders `13s`, header `sab-air ✗` |
| 44 | remote: a child row with `parent_id = sab-air:<p>` and its parent in the same fetch | nested under it |
| 45 | `fold_all` envelope round-trips through JSON (`serde`) unchanged; a row missing a new field deserializes with the default | equality; default |
| 46 | `Prompt@0`, `ChildStart x@1`, `Idle@5`; now=9 | Prompt `○ your turn · 1 bg` (B1) |
| 47 | `End@0`, `Prompt@1`; now=2 | on screen, Working (B2: `ended` is the newest boundary, not a flag) |
| 48 | Claude sessions S1 (receipts to @10) and S2 (first receipt @11), same `harness_pid`/`harness_start`, both alive | S1 gone, S2 on screen (B2 supersede); the same with source `cursor` → both on screen |
| 49 | `Prompt@0`, `Pre A id1@1`, `Pre B id2@2`, `Post B id2@3`; now=4 | close-older: Working, A closed (m1, documented cost). Strict: Tool `▶ A · 3s`. Pi with `toolCallId`s is strict today, so the Pi variant of this row expects `▶ A` |
| 53 | remote cached `Tool` row, `since_ms=25000`, fetch failed, `fetched_at` 6 s ago | renders `⏳`, header `sab-air ✗` (D1) |
| 54 | `Prompt@0`, `ChildStart x@1`; child x's last receipt @1; now=601 | Working `● 1 bg?` (D2); at now=599 → `● 1 bg` |
| 55 | `live::render` on a local `Waiting` row with `since_ms=29999`, `extra_ms=0` → `⚠`; same row, `extra_ms=1` → `⚠?` | boundary through the shared render path |
| 50 | Cursor: `beforeSubmitPrompt@0`, `stop@5`, `afterAgentResponse@5.5`; now=8 | Prompt (m4: a `Tick` within 2 s after `Idle` is not `last_ev`); with `afterAgentResponse@8`, now=9 → Working |
| 51 | Claude: `Prompt@0`, `Pre A@1`, `SessionStart(detail=compact)@2`; now=3 | Tool `▶ A` (the compact `SessionStart` is dropped, m3) |
| 52 | chain `sh`, `/bin/zsh`, `claude`, `-zsh`, `herdr` | harness = `claude` (B3) |

Tests 40–45 and 53 live in `tests/remote.rs` (fake ssh scripts: good envelope, exit 2, bad schema, timeout) and the render parts in the pager's `traces_modal.rs` live tests. Tests 1–39, 46–52, 54 and 55 live in `live.rs`.

Tests 7, 12, 20, 32, 37, 38, 46, 47 and 48 go through a real index (`turn_receipts`, `live_sessions`), not a hand-built `Vec`. Tests 24, 25 and 48 inject `alive` as a closure.

Real-session replay: the review 3 replay script is rewritten in Rust as a test helper over a copy of the index (`tests/live_replay.rs`, ignored by default, path from an env var). Step 1 ends with the three Claude sessions review 3 replayed showing no `▶ Bash ⏳` after a later `Post`, one Pi session showing `▶`/`○` transitions, and one Cursor chat from the Step 0 capture showing `▶`/`●`/`○` and leaving on `sessionEnd`.

## The live screen

`Mode::Live`. Full width `W`, height `H`.

```
 grok live · 5 agents · 1 waiting                               sab-mini ● sab-air ●  ← header
 ⚠ claude  Traces packet on Mini             permission: Bash cargo build --release  2m ⌖
   ● claude  review spec                     Read docs/traces/SPEC-live.md            4s
   ○ claude  snapshot tests                  your turn                                1m
 ● pi      sab-air: khoj worker              Bash rg --files                          1s
 ▶ cursor  grok-build                        Shell cargo test -p xai-grok-traces      8s
 ◌ claude  sab-air: fork lead  ctx 142k      quiet · Edit src/views/traces_modal.rs   4m ✗
 enter pane   o open   j/k   c children   q quit                                          ← hints
```

Header hosts: `●` latest fetch succeeded, `✗` latest fetch failed (cached rows stay, ages keep advancing); the schema-gate note is one dim row under the header. Row = `glyph source title [ctx] state-text age marks`. Columns: glyph 1, source 7 (`source_label`), title 28 (`clean_name`; `label` for children; `host:` prefix for remote rows), ctx 13 (`ctx Nk`, or `ctx Nk/Mk` when a window is known; blank when `None`), state text fills, age 4 right-aligned (`fmt_age`), marks 4. Truncation with `…` uses `modal_window.rs` `word_wrap` on one line, never splitting a token. Title and state text are identical collapsed and expanded.

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
10. Remote: header `sab-air ●` with interleaved `sab-air:` rows; `sab-air ✗` with cached rows; the `update grok-traces` note row. `⌖` never appears on a remote row (Herdr panes are local).

pty proof: launch `grok live`; pipe `UserPromptSubmit`, `PreToolUse`, `PermissionRequest` for one session into `grok-traces hook claude` (the pty harness process is the "harness PID"; the hook walks up to it); assert `⚠` within 2 s; pipe `PostToolUse`; assert `●` on the next poll; pipe `SessionEnd`; assert the row is gone.

## Harnesses run by bb (checked 2026-10-21, both Macs)

Facts:
- bb's server runs on the Air (`~/.bb/config.json` on the mini points at it through Tailscale); the mini's bb is a client. Zed talks to bb through `~/.bb/plugin-dev/bb-acp/bin/bb-acp.mjs`, which drives the bb CLI. The server was not running at check time.
- bb runs Claude through the Agent SDK (`entrypoint: sdk-cli`, `permissionMode: bypassPermissions` in the transcripts), with no `settingSources` in the bundle. Result: **none of the 17 bb-run Claude sessions on the Air has a single hook receipt**, including ones with hundreds of tool calls. The transcripts still land in `~/.claude/projects/<cwd>` (cwd `~/.bb`), so they show in `/traces` history, never on the live screen. Pi, Codex, Cursor, Grok and OMP under bb run through bb's own providers or ACP; their hook behaviour is (U).
- bb records the Claude session id as `events.provider_thread_id` per thread, so a bb-thread-to-Claude-session link exists in `bb.db` today. bb's hook plugin (`hooks/bb/server.ts`) sends only `event` and `thread_id`.
- Several bb Claude threads have no transcript on the Air: bb runs threads on whichever host the environment names (`host_daemon_sessions`, hosts sab-mini, khoj-38, ibp), through `bb-host-daemon.js`. On that host the Claude process's parent is `node … bb-host-daemon.js`, whose `comm` is just `node`.

Step 1 addition (small, in scope):
- `trace_receipts.harness_chain TEXT`: the process chain above the harness as a `>`-joined list of basenames, taking the script basename when `comm` is `node`/`bun`/`python` (so `claude>bb-host-daemon.js>launchd`, `claude>zsh>herdr`, `claude>bb-acp.mjs>zed`). `capture.sh` already records the chain; add `args=` to its `ps` so the script name is visible. `SessionRow.via: Option<String>` = the first chain entry after the harness that matches `bb-host-daemon.js`, `bb-acp.mjs`, `zed`, `herdr`, `Cursor`, else `None`; rendered dim after the title as `via bb` / `via zed` / `via herdr`. Test: the three chains above.

Post-v1 (Sab decides; not in this packet). Source checked in Sab's fork `~/repos/bb` on the Air (skchaudr/bb, db23ea4):
- The provider is `plugins/provider-claude-code/src/bridge/bridge.ts`. It never sets `settingSources`, so user hooks are off, but it already registers its own in-process SDK hooks for `PermissionRequest`, `PermissionDenied`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `SubagentStop` (`addPermissionEscalationTrackingHooks`). The cheapest bridge is one more callback in that merge that spawns `grok-traces hook claude` with the SDK hook input (it carries `session_id`, `transcript_path`, `tool_use_id`, `agent_id`), plus `SessionStart`/`UserPromptSubmit`/`Stop`/`SessionEnd`. That is a fork patch of about 20 lines and gives bb-run Claude the same receipts as a terminal session. Turning on `settingSources: ["user"]` instead would also load every other user hook and setting into bypass-permission runs; not recommended.
- The bb-thread link needs no plugin: `bb.db` `events.provider_thread_id` is the Claude session id, and the index already reads `bb.db` in place, so `links.rs` can set the Claude session's `parent_native` to the bb thread on the host that holds `bb.db`. Threads bb runs on another host through the host daemon stay unlinked (the transcript and the database are on different machines).
- bb's plugin events (`thread.created/active/idle/failed/archived/deleted`) carry the thread DTO without `providerThreadId`, so the existing `hooks/bb` plugin cannot supply the link itself.
- Pi under bb (`plugins/provider-pi`) and the ACP providers: whether the Pi extension loads and whether Cursor/Grok/OMP hooks fire is (U).

## Non-goals (v1)

Codex, OMP, Grok, bb, DSH rows. A Cursor `Waiting` state. Reading remote receipts or checking remote PIDs from the mini. Jump-to-pane for remote rows. Web UI, websocket, notification center. Live tool output (the inspector exists). Transcript reading outside the index. Model window tables.

## Delivery order

0. **Gate.** Capture is wired on both Macs. Run the three capture sessions per machine, close captures 1–17, answers in `MAP.md`, (U) markers here replaced. Capture 10 is the one exception: it needs step 1's query and closes at the end of step 1.
1. `is_terminal` fix + tests. Schema 7, receipt columns, PID capture and `alive` (B3 walk), `harness_chain` + `via`, same-process supersede, Cursor `links.rs` arm, `conversation_id` keying, `before*`/`after*` tool names and receipt-only rows, `live_sessions`, `turn_receipts` (with whole-session child events), `receipt_count_since`, usage fields, `norm` + `live.rs` (`strict_ids`, `fold_all`, `render`, `LIVE_SCHEMA`) + the 55 sequence tests + the replay helper, `live --json` envelope + `fetch_live` + schema gate + `tests/remote.rs` cases, Claude installer events, Cursor per-event reply + installer events, Pi extension rewrite + `hooks/test.sh`. `cargo test -p xai-grok-traces`.
2. `Mode::Live` render, ordering, remote merge with `fetched_at` clock, snapshots 1–6 and 8–10.
3. `grok live` subcommand, Esc exits, pty proof. Deploy to the Air (B4; in scope, no separate OK needed, D4): `ssh sab-air uname -m` is `arm64`; back up `~/.grok/bin/grok-traces` and `grok-fork` to `*.bak-<ts>` on the Air; `rsync` to a temp name then `mv` into place (atomic, no `--inplace`); `grok-traces version` and `live --json` over ssh return the envelope inside the 4 s kill (this is capture 18, recorded in `MAP.md` with its wall time). Rollback note in `MAP.md`: the old binary cannot open a schema 7 index, so rolling back also means moving `~/.grok/traces/traces.sqlite` aside and letting the old binary rebuild.
4. Trajectory `include_children` + snapshots.
5. Herdr jump, snapshot 7.

Each step merges alone. 0–3 are the deliverable.

## Decisions log

1. Liveness = harness PID + start time, or an `End` boundary, or a same-process newer session. No presence timers (Sab, review 3; B2).
2. v1 = Claude Code + Pi + Cursor, fully correct; Codex next; others stay in history (Sab, review 3 and its Cursor amendment).
3. Herdr optional, jump only (Sab, review 3).
4. Done is gone: an `End` boundary removes the row on the next poll; nothing lingers. `ended` is computed from the newest boundary, never stored (B2).
5. Transcript growth clears nothing; `⚠?` is the honest display for a granted permission until step 0.3 finds a grant signal.
6. Orphan `Pre`: per source. Strict id pairing where ids exist and orphans are guaranteed closed (Pi now; Claude and Cursor if captures 1 and 5 say yes); otherwise a `Post` closes its own `Pre` and every older open `Pre` (review 3, blocker 1; review 5 fix 5).
11. Remote rows carry state and numbers; the mini renders glyphs and marks through the same `live::render` as local rows, advanced by the time since the fetch. `LiveRow.stale` is dropped (D1, D3).
12. A lost `ChildStop` never expires the child; after 10 min without a receipt from it the count shows `bg?` (D2).
13. The Air binary deploy in step 3 is in scope and needs no separate OK (D4).
7. `/traces` does not default to live.
8. Cursor is keyed by `conversation_id`; a null `transcript_path` yields a receipt-only row, never a dropped receipt. Cursor liveness ends on `sessionEnd` or the Cursor process; no supersede rule (B3).
9. Captures use `hooks/capture.sh`, not a `--dump` flag (Opus, 43810bdf).
10. sab-air is in v1. The Air folds its own `LiveRow`s (own PIDs, own clock) via `grok-traces live --json`; the mini merges, advances the ages from `fetched_at`, and renders. A `live_schema` mismatch shows a notice row, never guessed states (Sab, review 3 Air amendment; B4).
