# SPEC: `grok live` screen

Rev 2 (Fable 5.1, 2026-10-21) after `SPEC-live-review.md` (Opus lead, 7390998f). All 15 state findings and the data findings are folded in. (U) = not verified by anyone yet; step 0 below closes them. Paths under `crates/codegen/`. Read `MAP.md` first.

## The tool

A full-screen, always-on view on a second screen that answers, per running agent: what it is doing now, whether it is waiting on me, what it dispatched and what those children are doing, and how full its context is. One key jumps to the agent's pane.

## Architecture (unchanged by review)

- No daemon, watcher or second store. Hooks call `grok-traces hook <source>`, which records a receipt and re-indexes the transcript. The screen polls the index at 1 s (`poll_live_at`).
- Context comes from the index, which the hook already refreshes per event. The fix is in `normalize`, not a new reader. Latency is one hook plus `index_file` (stat, 64 KB tail hash, parse new lines, 250 ms busy timeout); measure it in step 0, do not quote a number.
- States are a function of receipts (what the harness is doing) with the transcript as a second clock (what it has written since).
- One modal, two hosts: `TracesModalState` gains `Mode::Live`. `/traces` opens in list mode as today. `grok live` boots the pager straight into `Mode::Live` full screen and Esc exits the process. Soft subcommand beside `Command::Dashboard` in the pager's `cli.rs` (find it with `rg -n "Dashboard" crates/codegen/xai-grok-pager*/src`; the review cites `cli.rs:144`). Named `live`, not `traces`, because `Command::Trace` exists.

## Step 0: capture before building

Add `grok-traces hook <source> --dump <dir>` (writes each raw payload to `<dir>/<source>-<event>-<n>.json`, otherwise identical) and run one Claude session with a subagent, a denied permission, a compaction, and a parallel tool pair. Confirm from the files, then delete the flag or keep it as a debug aid:

- (U) Claude `PreToolUse`/`PostToolUse` carry `tool_use_id`.
- (U) Receipts fired from inside a subagent carry `agent_id` (the review verified they land on the parent's `session_id`; the separating field is unconfirmed). If no field separates them, the parent's state ignores receipts whose `tool_use_id` is unknown to the parent transcript, and child state comes from the child transcript only.
- (U) `PermissionRequest` and `Notification` payloads carry `transcript_path`. If not, `session_id` resolves through `native_session` → `trace_sessions.native_id` (needed anyway for Cursor, whose path may be null).
- (U) Whether an `async` hook is killed by the 1.5 s `SessionEnd` budget. Only affects how often `Done` is missed; rule 9 covers it.
- (U) Codex herdr `agent_session.value` format (herdr shows Claude's as the session uuid; verified on this pane).
- Measure hook-to-index latency: `at_ms` of the receipt against `last_event_at` after the poll.

## Data (`xai-grok-traces`)

Schema 7 plus `PARSER_VERSION` bump. There is no ALTER path; an old index is replaced through the existing `.rebuild.lock` path (`index.rs` `rebuild`). Say so in MAP.md.

### Receipts

`trace_receipts` gains `detail TEXT`, `tool_use_id TEXT`, `agent_id TEXT`. `Receipt` and the insert in `receipts.rs` change with it. `hook::extract` adds key lists `["tool_input","toolInput","input"]`, `["tool_use_id","toolUseId"]`, `["agent_id","agentId"]`, `["notification_type","notificationType"]`. `detail` = `classify::call_detail(tool, tool_input)` for tool and permission events; for `Notification` it is the notification type. When `path` does not resolve, `session_id` falls back to `native_session` matched against `trace_sessions.native_id` in the same source.

`SessionRow` gains `native_id: Option<String>` (column exists; add to `SESSION_COLS`, `session_row`, `session_to_json/from_json`) and `context_tokens: Option<i64>`, `context_window: Option<i64>`.

New queries:
- `live_receipts(now_ms) -> Vec<LiveReceipt>`: every allowlisted receipt in the last `LIVE_WINDOW_MS` (10 min) for sessions with at least one, ordered by `(session_id, at_ms, id)`. Not "last two": the state machine folds the whole window (finding 6). Receipts with `agent_id` set are returned tagged, never folded into the parent.
- `receipt_count_since(ms) -> i64` for the empty state.
- `last_receipt` and `last_receipt_at` order by `(at_ms, id)`, not `id` (finding 1).

### Usage

`NormalizedEvent.context_tokens`, `context_window`:
- Claude: `usage.input_tokens + cache_read_input_tokens + cache_creation_input_tokens` of that message. No window field.
- Codex: `payload.info.last_token_usage.input_tokens` (per message; `total_token_usage` is cumulative and `cached_input_tokens` is a subset, so neither is added). Window = `info.model_context_window`. This record is noise (`classify.rs` `event_msg/token_count`), so it is tracked in `Aggregate::add` on its own, not in the `now_text` pass.
- Others: `None`.

`trace_event_usage` and `trace_sessions` gain both columns; the session value is the last event that had one.

### Hook events per harness

Installer additions. The `,1` flag means "takes `matcher: *`", nothing else.
- Claude `CLAUDE_EVENTS`: `PermissionRequest`, `PermissionDenied`, `Notification`, `SubagentStart`, `PreCompact`, `PostCompact`, `StopFailure`.
- Codex `CODEX_EVENTS`: `PermissionRequest`, `PreCompact`, `PostCompact`, `Interrupt`. Payload per the Codex hooks docs (review Q2): `session_id`, `transcript_path`, `tool_name`, `tool_input` (`.command`, optional `.description`).
- Grok Build: the shell has no permission *request* hook (`xai-hooks-plugins-types` `HookEvent` has `PermissionDenied` only). Route: `[[ui.notifications.hooks]]` with `events = ["approval_required"]`, `only_unfocused = false` (`xai-grok-pager/src/notifications/config.rs`). That hook gets **no stdin** and env `GROK_EVENT`, `GROK_MESSAGE`, `GROK_SESSION_ID` (`notifications/hooks.rs`, verified). So `grok-traces hook grok --env` reads those three vars instead of stdin; `GROK_SESSION_ID` resolves via `native_id`. Installer writes that block into `~/.grok/config.toml` under the same begin/end-comment convention as the DSH block. Add `PermissionDenied`, `StopFailure`, `PreCompact`, `PostCompact` to `hooks/grok/traces.json`.
- OMP, Pi, DSH, Cursor, bb: no change. Their reachable states are listed in the normalizer table.

### Event normalizer

`live::norm(source, event, detail) -> Option<Ev>`. Anything not in the table is dropped (that covers `unparsed`, `unknown`, `auth_success`, elicitation responses, every unlisted Notification type).

| `Ev` | claude | codex | grok | omp/pi | dsh | cursor | bb |
|---|---|---|---|---|---|---|---|
| `Start` | SessionStart | SessionStart | SessionStart | session_start (U) | session/start (U) | — | thread.created |
| `Prompt` | UserPromptSubmit | UserPromptSubmit | UserPromptSubmit | — | — | — | thread.active |
| `Pre(tool_use_id)` | PreToolUse | PreToolUse | — | — | — | — | — |
| `Post(tool_use_id)` | PostToolUse, PostToolUseFailure, PermissionDenied | PostToolUse, PostToolUseFailure | PostToolUse, PermissionDenied | tool_result | tool (U) | postToolUse, postToolUseFailure | — |
| `Ask` | PermissionRequest; Notification/{permission_prompt, elicitation_dialog, elicitation_url_dialog, agent_needs_input} | PermissionRequest | approval_required (env route) | — | — | — | — |
| `Idle` | Stop, StopFailure(✗), Notification/idle_prompt | Stop, Interrupt | Stop, StopFailure(✗), StopCancelled | turn_end, agent_end | turn end (U) | stop, afterAgentResponse | thread.idle |
| `ChildStart`/`ChildStop` | SubagentStart/SubagentStop | SubagentStart/SubagentStop | SubagentStart/SubagentStop | — | — | — | — |
| `CompactStart`/`CompactEnd` | PreCompact/PostCompact | PreCompact/PostCompact | PreCompact/PostCompact | — | — | — | — |
| `End` | SessionEnd | SessionEnd | SessionEnd | session_shutdown | session/end (U) | — | thread.archived, thread.failed(✗) |

OMP/Pi, DSH, Cursor and bb never produce `Pre` or `Ask`, so they never show `Tool` or `Waiting`. The screen shows that honestly: their rows carry `●`/`○`/`◌` only.

## Live state machine (`live.rs` in the traces crate)

Inputs per session: its own `Ev` list in the window (parent: `agent_id IS NULL`), `open_tools` = `Pre` ids without a later `Post` of the same id, `open_children` = `ChildStart` − `ChildStop` floored at 0, `last_ev` = the newest `Ev` that is not `ChildStart`/`ChildStop`, and the transcript clock `grew_since(t)` = `last_event_at > t`, `age` = `now − max(newest Ev at, last_event_at, file mtime)`.

Precedence, first match wins:

| # | State | Rule | Row text |
|---|---|---|---|
| 1 | `Failed` | `SessionRow.failed` | `✗ <now>` |
| 2 | `Done` | `last_ev == End`, or `status == Completed` with no newer `Ev` | leaves the screen 60 s after |
| 3 | `Waiting` | `last_ev == Ask` and not `grew_since(ask.at)` | `⚠ permission: <tool> <detail> · <age>` |
| 4 | `Compacting` | newest `CompactStart` has no later `CompactEnd` | `⟳ compacting · <age>` |
| 5 | `Tool` | `open_tools` non-empty and not `grew_since(oldest open Pre.at)` past that tool's result (see note) | `▶ <tool> <detail> · <elapsed since Pre>`; `⏳` after 30 s. Oldest open tool is shown; `+N` when more are open. Tool `Agent`/`Task`: `▶ Agent <label>`, no `⏳` while `open_children > 0` |
| 6 | `Prompt` | `last_ev ∈ {Idle}` | `○ your turn · <age>`; `· N bg` when `open_children > 0`; `✗` glyph when the Idle came from StopFailure/thread.failed |
| 7 | `Working` | `last_ev ∈ {Start, Prompt, Post, CompactEnd}` and `age < 2 min` | `● <now>` |
| 8 | `Quiet` | any `last_ev` and `age` 2–10 min | `◌ quiet <age> · <now>` |
| 9 | stale mark | applied on top of 3–5: no `Ev` and no transcript growth for 10 min | state text gains `?` (`⚠ permission? 12m`, `⏳ Bash cargo build? 11m`); row stays |
| 10 | off screen | `age > 10 min` and state not 3–5; or 60 s after `Done`/`Failed` | — |
| 11 | no receipts | source has no receipt in the window but `status == Active` | `● <now>` with a dim glyph; otherwise not live |

Note on 5: a `Post` lost to the busy timeout or recorded before its `Pre` (async hooks race, finding 1) would hold `Tool` forever. `Tool` therefore also clears when the transcript shows a `tool_result` newer than the `Pre` (`last_event_at > pre.at` and the session's last non-noise event is a tool result). Ordering by `(at_ms, id)` handles the common race; this handles the drop.

Remote hosts: `live --json` on the remote bin returns `LiveReceipt`s with `age_ms` computed remotely, clamped at 0 locally (finding 13). An older remote binary exits 2 on the unknown subcommand; the local side then shows that host's rows from `list --json --active` with rule 11 and `Fetched.note` says `live states unavailable`.

Children: a child row's state comes from its own receipts (`agent_id` tagged) when present, else rule 11 from its transcript. A parent's own state never reads child receipts (finding 3).

### Sequence tests (`live.rs`, one named test each)

Times in seconds. Expected state after the last line.

1. `Pre A@0, Post A@0.05` recorded as ids (Post, Pre) → Working, not Tool.
2. `Pre A, Pre B, Post A` → Tool B (`+0`), elapsed from B.
3. parent `Pre Agent`, child (`agent_id`) `Pre Read`, child `Post Read` → parent Tool Agent; child Working.
4. `Pre Bash, Ask` → Waiting; then transcript grows past ask → Tool Bash (the grant fired no event).
5. `Pre Bash, Ask`, user denies: `PermissionDenied` → Working; without it, transcript growth → clears.
6. `Prompt, Post, Notification/auth_success, Notification/auth_success` → Working (notifications dropped).
7. `Idle, ChildStop` → Prompt, `0 bg`.
8. `Prompt@0`, nothing for 8 min, transcript mtime at 7:50 → Working/Quiet by `age` 10 s, not off screen. `Pre Bash@0`, 12 min, no growth → `⏳ … ? 12m`, on screen.
9. OMP `turn_end` with `status == Completed` and a later `tool_result` → Working, not Done.
10. Cursor `postToolUse`, `stop` → Working then Prompt; never Tool.
11. `CompactStart, (no end)` → Compacting; `CompactEnd` → Working.
12. `Ask@0`, 11 min, no growth → `⚠ permission? 11m`.
13. remote receipt with `age_ms = -400` → 0.
14. Pi session, no receipts, `status == Active` → Working, dim.
15. `unparsed`, `unknown`, `Interrupt` (codex) → first two dropped; Interrupt → Prompt.
16. `Idle`, then `End` → Done; 61 s later → off screen.

## The live screen

`Mode::Live`. Full width `W`, height `H`.

```
 grok live · 3 agents · 1 waiting                                sab-mini ● sab-air ●     ← header
 ⚠ claude  Traces packet on Mini             permission: Bash cargo build --release  2m ⌖ ← root
   ● sonnet  review spec                     Read docs/traces/SPEC-live.md            4s   ← child, indent 2
   ○ sonnet  snapshot tests                  your turn · 1m
 ● codex   fork lead  ctx 142k/258k          Edit src/views/traces_modal.rs          1s ⌖
 ◌ grok    air: khoj worker  ctx 31k         quiet 4m · Bash rg --files
 enter pane   o open   j/k   c children   q quit                                           ← hints
```

Row = `glyph source title [ctx] state-text age [⌖]`. Columns: glyph 1, source 7 (`source_label`), title 28 (`clean_name`; `label` for children; `host:` prefix for remote), ctx 13 (`ctx Nk` or `ctx Nk/Mk`; blank when `None`), state text fills, age 4 right-aligned (`fmt_age`), pane mark 1. Truncation with `…` uses `modal_window.rs` `word_wrap` on one line (never split a token). Title and state text are identical collapsed and expanded (header-stable rule).

Order: `Waiting`, then `Prompt`, then by `age` ascending. Children directly under their parent, same order. A child whose parent is not live is a root titled `↑ <parent title>`. Selection kept by session id across polls (`keep_selection`).

Keys:

| Key | Action |
|---|---|
| `j`/`k`, arrows, wheel | move |
| `c` | collapse/expand the selected root (default expanded; collapsed shows `+N`; a collapsed root with a `Waiting` child shows `⚠` on the root) |
| `o`, or `Enter` with no pane known | open trajectory (`Mode::Trajectory`, `include_children`); Esc returns to live |
| `Enter` with a pane known | `herdr agent focus <pane_id>` |
| `q`, Esc | `grok live`: exit process. `/traces`: back to list |
| `/` | existing search over title and state text |

### Trajectory with children inline

`EventFilter.include_children`. `list_events` unions the children's non-noise events by timestamp, each tagged `child: Option<(session_id, label)>`. A child event renders as the existing event row one depth deeper with `label ·` before the header. Collapsed turns count child events in `+N`. Children are never reordered among themselves. Snapshots: two interleaved children collapsed and expanded; one child with zero events.

### Jump to pane

If `herdr` is on `PATH`: poll `herdr pane list` every 10 s on its own worker, modelled on `poll_remote` (own rx, own `Instant`), never on `spawn_refresh`. Map `agent_session.value` to `SessionRow.native_id`. Matched rows show `⌖`. `Enter` runs `herdr agent focus <pane_id>` (pane ids are valid agent targets: `herdr agent get w36:pB` works; `focus` with a pane id is (U), test in step 0). Failure → one-line `TracesNotice`.

### Render states (snapshot tests, `TestBackend`, `render_tests.rs` pattern)

1. Empty: `no live agents · <N> hook receipts in the last hour · grok-traces doctor`; N = 0 ever → add the installer line.
2. One row per state 1–9 and 11 in one ledger, W=120.
3. Same at W=80: ctx column drops, title 20.
4. Overflow: 50 rows, H=20, selection at bottom.
5. Children expanded; collapsed; collapsed with a `Waiting` child.
6. Index tripped: header `index tripped · grok-traces doctor --reset`, body still renders.
7. Remote host unreachable: header `sab-air ✗`, local rows unaffected.
8. `⌖` present and absent.
9. Parent `▶ Agent review spec` with `2 bg`; parent `○ your turn · 1 bg`.

pty proof: launch `grok live`; pipe `PreToolUse` then `PermissionRequest` for one session into `grok-traces hook claude`; assert `⚠` within 2 s; append a line to the transcript fixture; assert `▶` on the next poll.

## Non-goals

No web UI, websocket or notification center. No live tool output (inspector exists). No transcript reading outside the index. No model window table; only harness-supplied windows (Codex) are shown.

## Delivery order

0. Captures and measurements above; write results into `MAP.md`. Blocks step 1.
1. Schema 7, receipt columns and fallbacks, `live_receipts`, `receipt_count_since`, usage fields, `norm` + `live.rs` + the 16 sequence tests, installer events, Grok `--env` route, remote `live --json`. `cargo test -p xai-grok-traces`.
2. `Mode::Live` render, ordering, snapshots 1–7 and 9.
3. `grok live` subcommand, Esc exits, pty proof.
4. Trajectory `include_children` + snapshots.
5. Herdr jump, snapshot 8.

Each step merges alone. 0–3 are the deliverable.

## Decisions on the review's open questions

1. Live window 10 min on combined `age`; `Quiet` is 2–10 min; no fourth window. The three windows (2 min receipt-live, 5 min active, 10 min live screen) go into MAP.md.
2. Codex `PermissionRequest` carries `tool_name`/`tool_input`; `⚠` shows detail.
3. Grok waiting state goes through the notification hook env route above; it is in scope, not a gap.
4. `/traces` does not default to live.
