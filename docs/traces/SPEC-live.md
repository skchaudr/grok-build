# SPEC: `grok live` screen

Rev 3 (Fable 5.1, 2026-10-21) after `SPEC-live-review.md` (7390998f) and `SPEC-live-review-2.md` (5d43a539). Findings A–L and the ten ambiguities are folded in. (U) = not verified; Step 0 closes every one before code starts. Paths under `crates/codegen/`. Read `MAP.md` first. Implementer: a compacted Opus lead with Sonnet 5.5 subagents.

## The tool

A full-screen, always-on view on a second screen that answers, per running agent: what it is doing now, whether it is waiting on me, what it dispatched and what those children are doing, and how full its context is. One key jumps to the agent's pane.

## Architecture

- No daemon, watcher or second store. Hooks call `grok-traces hook <source>`, which records a receipt and re-indexes the transcript. The screen polls the index at 1 s (`poll_live_at`).
- Context comes from the index, refreshed by the hook per event. Latency is one hook plus `index_file`; Step 0 measures it.
- State is a fold over the session's current turn of receipts (what the harness is doing), with the transcript clock as a tie-breaker for age only. Review 2 showed transcript growth cannot clear a permission wait (the transcript next grows at the tool result), so growth clears nothing.
- One modal, two hosts: `TracesModalState` gains `Mode::Live`. `/traces` opens in list mode as today. `grok live` boots the pager straight into `Mode::Live` full screen; Esc exits the process. Soft subcommand beside `Command::Dashboard` (`rg -n "Dashboard" crates/codegen/xai-grok-pager*/src`). Named `live` because `Command::Trace` exists.

## Step 0: hard gate. No code in steps 1–5 until every line here has a recorded answer in `MAP.md`

Mechanism: `grok-traces hook <source> --dump <dir>` writes each raw stdin payload (or, with `--env`, the `GROK_*` environment) to `<dir>/<source>-<event>-<n>.json`, otherwise behaves as today. Wiring `--dump` into `~/.claude/settings.json` or `~/.grok/config.toml` is a config edit: show Sab the exact diff first. Touching `~/.codex` or `~/.cursor` hooks needs Sab's explicit OK; without it those rows stay (U) and the spec ships with their states limited as the normalizer table says.

Run on this machine one Claude session that dispatches a subagent, gets a permission prompt that is granted, gets one that is denied, triggers a compaction, and runs two tools in one assistant turn. Record:

1. (U) `tool_use_id` on `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionRequest`, `PermissionDenied`.
2. (U) `agent_id` on `SubagentStart` and `SubagentStop`: is it the child's id, and do the tool receipts fired inside the child carry the same value?
3. (U) Any signal between `PermissionRequest` and `PostToolUse` on a grant (a `Notification` type, a second `PreToolUse`, anything). If none, rule 3's `⚠?` form stands and test 4 is as written.
4. (U) `transcript_path` on `PermissionRequest`, `PermissionDenied`, `Notification`.
5. (U) `GROK_EVENT`, `GROK_MESSAGE`, `GROK_SESSION_ID` as received by a `[[ui.notifications.hooks]]` command on `approval_required`, and whether `GROK_SESSION_ID` equals the `summary.json` session id the index stores as `native_id`.
6. (U) Whether a `SessionEnd` hook marked `async` survives the exit budget (count `SessionEnd` receipts against sessions ended).
7. (U) Codex `agent_session.value` in `herdr pane list`, and `herdr agent focus <pane_id>` (pane ids work for `herdr agent get`, verified).
8. Hook-to-index latency: receipt `at_ms` against the transcript's `last_event_at` on the next poll, ten samples, median and max into `MAP.md`.
9. (U) OMP/Pi, DSH: the exact `session_start`, `tool`, `turn end`, `session end` event names their templates emit (`hooks/{omp,pi}/traces.ts`, `hooks/dsh/traces.mjs`).

## Data (`xai-grok-traces`)

Schema 7 and a `PARSER_VERSION` bump. No ALTER path exists; an old index is replaced through the existing `.rebuild.lock` path. Note it in MAP.md.

### `is_terminal` (live `/traces` bug, fix first)

`index.rs` `is_terminal` matches `complete` and `turnend`, so Codex `task_complete` and OMP/Pi `turn_end` mark a session `completed` on every turn (74 of 90 root Codex sessions in the review's copy). New rule: the squashed type must be one of `sessionend`, `sessionshutdown`, `runend`, `agentend`, or end with one of them; `complete` and `turnend` are removed. `Status::Completed` then means the session itself ended. Test: `task_complete`, `turn_end`, `turn/complete` are not terminal; `session_end`, `session/end`, `agent_end`, `session_shutdown` are. This changes `/traces` ledger statuses on rebuild; that is the fix, not a regression.

### Receipts

`trace_receipts` gains `detail TEXT`, `tool_use_id TEXT`, `agent_id TEXT`. `Receipt` and the insert in `receipts.rs` change with it. `hook::extract` adds key lists `["tool_input","toolInput","input"]`, `["tool_use_id","toolUseId"]`, `["agent_id","agentId"]`, `["notification_type","notificationType"]`. `detail` = `classify::call_detail(tool, tool_input)` for tool and permission events; for `Notification` it is the notification type. When `path` does not resolve, `session_id` falls back to `native_session` matched against `trace_sessions.native_id` in the same source. `hook grok --env` builds the receipt from `GROK_EVENT`, `GROK_MESSAGE` (as `detail`) and `GROK_SESSION_ID`.

`SessionRow` gains `native_id`, `context_tokens`, `context_window`, `mtime_ms` (the query already selects `f.mtime_ms`), `last_event_kind` (`Kind` of the last non-noise event, new session column set in the `now_text` pass).

`last_receipt` and `last_receipt_at` order by `(at_ms, id)`.

New queries:

- `live_receipts(now_ms) -> Vec<LiveReceipt>`: for every session with a receipt at `at_ms ≥ now − 10 min`, all its receipts whose `at_ms ≥ now − 10 min` **or** whose `id` is greater than the session's newest turn-boundary receipt (events that normalize to `Idle`, `Prompt`, `End`), whichever set is larger. So an `Ask` or `Pre` from 15 minutes ago is still returned while nothing has ended the turn (finding D). Ordered `(session_id, at_ms, id)`. Fields: `session_id, agent_id, event, tool, detail, tool_use_id, at_ms`.
- `receipt_count_since(ms) -> i64` for the empty state.

### Usage

`NormalizedEvent.context_tokens`, `context_window`:

- Claude: `usage.input_tokens + cache_read_input_tokens + cache_creation_input_tokens` of that message. No window.
- Codex: `payload.info.last_token_usage.input_tokens` (per message; `total_token_usage` is cumulative, `cached_input_tokens` is a subset of input, neither is added). Window = `info.model_context_window`. The record is noise (`classify.rs` `event_msg/token_count`), so `Aggregate::add` tracks it on its own, outside the `now_text` pass.
- Others: `None`.

`trace_event_usage` and `trace_sessions` gain both columns; the session value is from the last event that had one.

### Hook events per harness

Installer additions (`,1` = takes `matcher: *`, nothing more):

- Claude: `PermissionRequest`, `PermissionDenied`, `Notification`, `SubagentStart`, `PreCompact`, `PostCompact`, `StopFailure`.
- Codex: `PermissionRequest`, `PreCompact`, `PostCompact`, `Interrupt`. Payload per the Codex hooks docs: `session_id`, `transcript_path`, `tool_name`, `tool_input` (`.command`, optional `.description`).
- Grok Build: no permission request hook exists (`xai-hooks-plugins-types` `HookEvent`). Route: `[[ui.notifications.hooks]]`, `events = ["approval_required"]`, `only_unfocused = false`, command `"$HOME/.grok/bin/grok-traces" hook grok --env`. The hook gets no stdin (`notifications/hooks.rs`, verified). Installer writes that block into `~/.grok/config.toml` under begin/end comments like the DSH block. Add `PermissionDenied`, `StopFailure`, `StopCancelled`, `PreCompact`, `PostCompact` to `hooks/grok/traces.json`.
- OMP, Pi, DSH, Cursor, bb: no change.

### Event normalizer

`live::norm(source, event, detail) -> Option<Ev>`. Anything not in the table is dropped: `unparsed`, `unknown`, `auth_success`, elicitation responses, every unlisted Notification type, and OMP/Pi `turn_end` (fires per model turn, finding H).

| `Ev` | claude | codex | grok | omp/pi | dsh | cursor | bb |
|---|---|---|---|---|---|---|---|
| `Idle` (turn boundary) | SessionStart, Stop, StopFailure(✗), Notification/idle_prompt | SessionStart, Stop, Interrupt | SessionStart, Stop, StopFailure(✗), StopCancelled | agent_end | turn end (U, step 0.9) | stop, afterAgentResponse | thread.idle |
| `Prompt` (turn boundary) | UserPromptSubmit | UserPromptSubmit | UserPromptSubmit | — | — | — | thread.active |
| `End` (turn boundary) | SessionEnd | SessionEnd | SessionEnd | session_shutdown | session end (U) | — | thread.archived, thread.failed(✗) |
| `Pre(id)` | PreToolUse | PreToolUse | — | — | — | — | — |
| `Post(id)` | PostToolUse, PostToolUseFailure, PermissionDenied | PostToolUse, PostToolUseFailure | PostToolUse, PermissionDenied | tool_result | tool (U) | postToolUse, postToolUseFailure | — |
| `Ask` | PermissionRequest; Notification/{permission_prompt, elicitation_dialog, elicitation_url_dialog, agent_needs_input} | PermissionRequest | approval_required (env route) | — | — | — | — |
| `ChildStart(agent_id)` / `ChildStop(agent_id)` | SubagentStart / SubagentStop | same | same | — | — | — | — |
| `CompactStart` / `CompactEnd` | PreCompact / PostCompact | same | same | — | — | — | — |

`SessionStart` is `Idle` (finding G: 2 min to 5 h passes between start and first prompt). `(✗)` sets the row's failed mark, not a state. OMP/Pi, DSH, Cursor and bb never produce `Pre` or `Ask`, so never show `Tool` or `Waiting`.

## Live state machine (`live.rs` in the traces crate)

### Inputs per session

- `evs`: the session's receipts from `live_receipts`, normalized, dropped `None`s. Parent fold uses `agent_id IS NULL` rows **except** `ChildStart`/`ChildStop`, which are read whatever their `agent_id` (finding C). Child rows fold their own `agent_id` rows.
- `open_tools`: `Pre` ids with no later `Post` of the same id. A `Pre` or `Post` with NULL id pairs FIFO by tool name. Every turn boundary (`Idle`, `Prompt`, `End`) closes all open tools (finding A: 19 orphan `Pre`s in real data).
- `open_children`: the set of `agent_id`s with a `ChildStart` and no later `ChildStop`. `End` clears the set; `Idle` and `Prompt` do not (finding B: children run in the background across `Stop`).
- `last_ev`: the newest `Ev` that is not `ChildStart`/`ChildStop`.
- Clocks, all in local ms: `ev_at` = newest `Ev` `at_ms`; `last_event_at` from the session row (transcript's own clock, accepted as is); `mtime_ms`. `age = now − max(ev_at, last_event_at, mtime_ms)`. `since(x) = now − x.at_ms`.

Boundaries: `<` on the left, `≥` on the right. Working while `age < 2 min`; Quiet while `2 min ≤ age < 10 min`; off screen at `age ≥ 10 min`; `⏳` at `since(pre) ≥ 30 s`; `⚠?` at `since(ask) ≥ 30 s`; Prompt shows at `since(idle) ≥ 2 s`; Done leaves at `since(end) ≥ 60 s`; stale mark at `≥ 2 min`.

### States

First match wins. `<now>` = `SessionRow.now`. `<age>` = combined `age` unless stated.

| # | State | Rule | Row text |
|---|---|---|---|
| 1 | `Done` | `last_ev == End` | `— ended · <since(end)>`; off screen at ≥ 60 s |
| 2 | `Compacting` | newest `CompactStart` has no later `CompactEnd` | `⟳ compacting · <since(start)>` |
| 3 | `Waiting` | `last_ev == Ask` | `⚠ permission: <tool> <detail> · <since(ask)>`; the tool and detail are the Ask's own fields; a Notification-sourced Ask with no tool reads `⚠ <notification type> · <since>`. At ≥ 30 s the glyph becomes `⚠?` (granted and running, or still waiting; no grant signal exists unless step 0.3 finds one) |
| 4 | `Tool` | `open_tools` non-empty | `▶ <newest open tool> <detail> · <since(pre)>`, `+N` when more are open; `⏳` at ≥ 30 s |
| 5 | `Prompt` | `last_ev == Idle` and `since(idle) ≥ 2 s` (finding J: 36 of 68 real `Stop`s are followed by tool receipts within 46 s) | `○ your turn · <age>`; `· N bg` when `open_children` non-empty |
| 6 | `Working` | `age < 2 min`, or `open_children` non-empty | `● <now>`; `● N bg · <now>` with children |
| 7 | `Quiet` | `2 min ≤ age < 10 min` | `◌ quiet <age> · <now>` |
| 8 | fall-through | anything else still on screen | `● <now>` |

Marks, applied to any state and never changing it or dropping the row:

- `✗` after the age column when `SessionRow.failed` or the last `Idle`/`End` carried `(✗)` (finding I: 22 of 108 live sessions have `failed`; it is a flag, not a state).
- `?` appended to the state text of states 2–4 when `since(ev_at) ≥ 2 min` and `now − max(last_event_at, mtime_ms) ≥ 2 min` (no receipt and no transcript movement). Rule 3's 30 s `⚠?` is the same mark with its own threshold.

Off screen: state 1 at ≥ 60 s; or `age ≥ 10 min` with no open tools, no open ask, no compaction and no open children. States 2–4 and sessions with open children stay as long as the turn has not ended.

Sessions with no receipts (per session, not per source) are live only when `status == Active`: `● <now>` with a dim glyph. After the `is_terminal` fix, `Completed` means the session ended, so it is not live.

Children: a child row folds its own `agent_id` receipts when any exist, else the no-receipt rule on its transcript. The parent fold never reads child tool receipts.

Ordering: `Waiting`, then `Prompt`, then `age` ascending, then `session_id` ascending. Children directly under their parent in the same order.

Remote: the fold runs on the remote host. `live --json` returns `LiveRow { session_id, state, text, age_ms, bg, failed, stale, pane: None }`; the local side clamps `age_ms` at 0 and renders. An older remote binary exits 2 on the unknown subcommand; the local side then uses `list --json --active` rows under the no-receipt rule (dim `●`) and sets a new `Fetched.note` of `<host>: live states unavailable (remote <version>)`, separate from the existing filtered-locally note.

### Sequence tests (`live.rs`, one named test each; times in seconds; `at_ms` stamped as listed, ids in listed order unless said)

| # | Sequence | Expected |
|---|---|---|
| 1 | `Pre A@0.00`, `Post A@0.05`, inserted as ids (Post, Pre) | Working; `open_tools` empty |
| 2 | `Prompt@0`, `Pre A@1`, `Pre B@2`, `Post A@3`, now=4 | Tool B, `+0`, since 2 s |
| 3 | `Pre Agent@0`, `Post Agent@0.1`, `ChildStart x@0.1`; child rows `Pre Read@1` (agent x) | parent Working `● 1 bg`; child Tool Read |
| 4 | `Pre Bash@0`, `Ask Bash@0`, now=31 | `⚠? permission: Bash cargo build · 31s` |
| 5 | `Pre Bash@0`, `Ask Bash@0`, `PermissionDenied id@2` (as `Post`) | Working; `open_tools` empty |
| 6 | `Prompt@0`, `Post@1`, `Notification/auth_success@2` ×2 | Working |
| 7 | `ChildStart x@0`, `Idle@5`, `ChildStop x@6`, now=9 | Prompt `○ your turn · 9s`, no `bg` |
| 8 | `Prompt@0`, `mtime_ms`=7:50, now=8:00 | Working (age 10 s) |
| 9 | OMP: `turn_end`, `turn_end`, `tool_result@now−1` | Working (both `turn_end` dropped) |
| 10 | Cursor: `postToolUse@0`, `stop@1`, now=4 | Prompt; never Tool |
| 11 | `CompactStart@0`, now=5 → Compacting; then `CompactEnd@6` | Working |
| 12 | `Prompt@−15min`, `Ask@−11min`, no later receipt, `mtime_ms`=−11min | returned by `live_receipts` (boundary rule); `⚠? permission: … · 11m` with `?` mark; on screen |
| 13 | remote `LiveRow` with `age_ms=−400` | rendered age 0 |
| 14 | Pi session, no receipts, `status == Active` | Working, dim |
| 15 | `unparsed`, `unknown`, Codex `Interrupt@0`, now=3 | first two dropped; Prompt |
| 16 | `Idle@0`, `End@1`, now=30 → Done; now=62 | off screen |
| 17 | `Pre A@0` (orphan), `Idle@5`, now=8 | Prompt; `open_tools` empty (finding A) |
| 18 | `SessionStart@0`, now=3 min | Prompt `○ your turn · 3m` (finding G) |
| 19 | Codex: transcript `task_complete`, receipts `Idle@0`, now=90 | Prompt, on screen (finding F) |
| 20 | `Idle@0`, `Pre A@1`, now=1.5 | Tool A (mid-turn Stop, finding J); at now=0.5 with only `Idle` → Working |
| 21 | `SessionRow.failed`, `Prompt@0`, now=1 | Working with `✗` mark; row stays |
| 22 | `Pre A@0` with NULL id, `Post A@1` with NULL id | paired FIFO; Working |
| 23 | `Pre Bash@0`, no receipt, `mtime_ms`=0, now=2 min | `⏳ Bash … · 2m` with `?` mark |
| 24 | bb: `thread.failed@0` | Done with `✗` mark |
| 25 | Grok: `Ask@0` (env route, no `Pre`), `Post@3` | Working (finding K) |

Test 20's two readings and test 12 require `live_receipts` to be exercised in the same test through a real index, not a hand-built `Vec`.

## The live screen

`Mode::Live`. Full width `W`, height `H`.

```
 grok live · 3 agents · 1 waiting                                sab-mini ● sab-air ●     ← header
 ⚠ claude  Traces packet on Mini             permission: Bash cargo build --release  2m ⌖ ← root
   ● sonnet  review spec                     Read docs/traces/SPEC-live.md            4s   ← child, indent 2
   ○ sonnet  snapshot tests                  your turn · 1m
 ● codex   fork lead  ctx 142k/258k          2 bg · Edit src/views/traces_modal.rs   1s ⌖
 ◌ grok    air: khoj worker  ctx 31k         quiet 4m · Bash rg --files              4m ✗
 enter pane   o open   j/k   c children   q quit                                           ← hints
```

Row = `glyph source title [ctx] state-text age [⌖] [✗]`. Columns: glyph 1, source 7 (`source_label`), title 28 (`clean_name`; `label` for children; `host:` prefix for remote), ctx 13 (`ctx Nk` or `ctx Nk/Mk`; blank when `None`), state text fills, age 4 right-aligned (`fmt_age`), marks 2. Truncation with `…` uses `modal_window.rs` `word_wrap` on one line, never splitting a token. Title and state text are identical collapsed and expanded.

Selection kept by session id across polls (`keep_selection`). A child whose parent is not live is a root titled `↑ <parent title>`.

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

### Jump to pane

If `herdr` is on `PATH`: poll `herdr pane list` every 10 s on its own worker, modelled on `poll_remote` (own rx, own `Instant`), never on `spawn_refresh`. Map `agent_session.value` to `SessionRow.native_id`. Matched rows show `⌖`. `Enter` runs `herdr agent focus <pane_id>` (step 0.7). Failure → one-line `TracesNotice`.

### Render states (snapshot tests, `TestBackend`, `render_tests.rs` pattern)

1. Empty: `no live agents · <N> hook receipts in the last hour · grok-traces doctor`; N = 0 ever → add the installer line.
2. One row per state 1–7 plus one no-receipt dim row, one `✗` mark, one `?` mark, one `⚠?`, W=120.
3. Same at W=80: ctx column drops, title 20.
4. Overflow: 50 rows, H=20, selection at bottom.
5. Children expanded; collapsed; collapsed with a `Waiting` child.
6. Index tripped: header `index tripped · grok-traces doctor --reset`, body still renders.
7. Remote host unreachable: header `sab-air ✗`, local rows unaffected. Remote old binary: dim rows and the note.
8. `⌖` present and absent.
9. Parent `● 2 bg · <now>`; parent `○ your turn · 1 bg`.

pty proof: launch `grok live`; pipe `PreToolUse` then `PermissionRequest` for one session into `grok-traces hook claude`; assert `⚠` within 2 s; pipe `PostToolUse`; assert `●` on the next poll.

## Non-goals

No web UI, websocket or notification center. No live tool output (the inspector exists). No transcript reading outside the index. No model window table; only harness-supplied windows (Codex) are shown.

## Delivery order

0. **Gate.** Captures and measurements above, answers written into `MAP.md`, (U) markers in this file replaced with the finding. Nothing below starts first.
1. `is_terminal` fix + tests. Schema 7, receipt columns and fallbacks, `live_receipts`, `receipt_count_since`, usage fields, `norm` + `live.rs` + the 25 sequence tests, installer events, Grok `--env` route, remote `live --json` and its fallback note. `cargo test -p xai-grok-traces`.
2. `Mode::Live` render, ordering, snapshots 1–7 and 9.
3. `grok live` subcommand, Esc exits, pty proof.
4. Trajectory `include_children` + snapshots.
5. Herdr jump, snapshot 8.

Each step merges alone. 0–3 are the deliverable.

## Decisions on the reviews' open questions

1. Live window 10 min on combined `age`; `Quiet` is 2–10 min; stale mark at 2 min; no fourth window. The windows (2 min receipt-live, 5 min active, 10 min live screen) go into MAP.md.
2. Codex `PermissionRequest` carries `tool_name`/`tool_input`; `⚠` shows detail.
3. Grok waiting goes through the notification hook env route; in scope.
4. `/traces` does not default to live.
5. Done is an `End` receipt only; `Completed` matters only for sessions with no receipts (finding F).
6. Transcript growth clears nothing (finding E). The `⚠?` form is the honest display until step 0.3 finds a grant signal.
