# SPEC: `grok traces` live screen

Status: draft for review (Fable 5.1, 2026-10-21). Reviewer: Opus lead. Implementer: next Opus lead + Sonnet subagents.
Read `MAP.md` first. Nothing here replaces the data path; it adds one view, one state machine, two hook events and two usage fields.

## The tool Sab wants

A full-screen, always-on view on a second screen that answers, per running agent, at a glance:

1. What is it doing right now (tool + argument, or thinking, or waiting for me).
2. Is it stuck, waiting on a permission, or finished and waiting for a prompt.
3. What did it dispatch, and what are those children doing, inline.
4. How full is its context.

Then one key to jump to that agent's pane.

## Architecture position

- **No daemon, no watcher, no second store.** Hooks already call `grok-traces hook <source>` on every event, which records a receipt and re-indexes the transcript file within one hook latency. agent-obs died of a watcher (55 GB). The live screen reads the index at 1 s, exactly as `/traces` does today (`poll_live_at`).
- **Context does not need live transcript reading.** `hook::ingest` indexes the transcript on every receipt, so the last assistant `usage` is in `trace_event_usage` about 10 ms after each tool call. The gap is that `normalize` reads `input_tokens` only; for Claude the bulk is `cache_read_input_tokens`. Fix is in normalize, not a new reader.
- **"Real states" come from receipts, not from transcripts.** The transcript tells you what happened; the hook event name tells you what the harness is doing now. Receipts are already per event with `tool`; two more events (permission, notification) and one more column (`detail`) make every state below derivable with one query.
- **One modal, two hosts.** `TracesModalState` gains a `live` mode. `/traces` keeps opening in list mode. `grok traces` (soft subcommand, same pattern as `Command::Dashboard` in `xai-grok-pager-bin/src/main.rs`) boots the pager straight into the modal in live mode, full screen, and Esc quits the process. No second renderer.

## Data changes (`xai-grok-traces`)

### Receipts

`trace_receipts` gains `detail TEXT` (schema 7). On `PreToolUse`/`PostToolUse`/`PermissionRequest`, `detail` is `classify::tool_detail(tool, tool_input)` (the same `Bash` first line / `parent/base` path / `server.tool` that `trace_events.detail` holds; extract it from the hook payload's `tool_input`/`toolInput`). `hook::extract` adds key lists `["tool_input","toolInput","input"]` and `["notification_type","notificationType"]`; a Notification receipt stores the notification type in `detail`.

New query: `live_receipts(now_ms) -> Vec<LiveReceipt { session_id, event, tool, detail, at_ms, prev_event, prev_at_ms }>`: the last two receipts per `session_id` with a receipt in the last `LIVE_WINDOW_MS` (10 min). One SQL with a window function, indexed by `trace_receipts_session`.

### Hook events

Installer `CLAUDE_EVENTS` adds `["PermissionRequest",1]` and `["Notification",1]`. Codex `hooks.json` already has `PermissionRequest`; add it to `CODEX_EVENTS`. Grok Build: check whether the shell emits a permission hook (`extensions/hooks.rs` lists `PreToolUse` only); if not, the live screen shows Grok's waiting state as `?` and the spec records that as a known gap, not a blocker. OMP/Pi/DSH/Cursor: no change; their states are the subset derivable from the events they send.

### Usage

`normalize` adds `cache_read_input_tokens`, `cache_creation_input_tokens` (Claude), `cached_input_tokens` (Codex `total_token_usage`) to a new `context_tokens: Option<f64>` = input + cache_read + cache_creation of that one message (never the running total). `trace_event_usage` gains `context_tokens` (schema 7). `trace_sessions` gains `context_tokens` = the last assistant event's value, updated in the same pass that sets `now_text`. `SessionRow.context_tokens: Option<i64>`.

## Live state machine (`query::LiveState`)

Computed per session from its last receipt `(event, at_ms)`, the previous receipt, and `SessionRow`. `now` = poll time. Precedence top to bottom; first match wins.

| State | Rule | Row glyph + text |
|---|---|---|
| `Waiting` | last event ∈ {`PermissionRequest`, `Notification` with detail `permission_prompt`} | `⚠ permission: <tool> <detail>` |
| `Prompt` | last event ∈ {`Stop`, `Notification`/`idle_prompt`} and no `SessionEnd` after it | `○ your turn · <age>` |
| `Tool` | last event is `PreToolUse` | `▶ <tool> <detail> · <elapsed since at_ms>`; after 30 s the glyph is `⏳` |
| `Working` | last event ∈ {`UserPromptSubmit`, `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `SubagentStop`, `SessionStart`} and age < 2 min | `● <now_text>` |
| `Quiet` | any non-terminal last event, age 2–10 min | `◌ quiet <age> · <now_text>` |
| `Done` | last event ∈ `TERMINAL_EVENTS` or `SessionRow.status == Completed` | row leaves the live screen after 60 s (stays in `/traces` history) |
| `Failed` | `SessionRow.failed` | `✗ <now_text>`; leaves after 60 s |

Sessions with no receipt in the last 10 min are not live. Remote hosts: same rules on the rows `fetch_sessions --active` returns; the remote bin gains `live --json` returning `LiveReceipt`s, with the existing fallback (`Fetched.note`) when the remote binary is older.

`Tool` elapsed for a child session dispatched by Agent tool: the parent shows `▶ Agent <label>` and the child row underneath shows its own state. The parent's `Tool` elapsed does not get an `⏳` while any child is not `Done`/`Failed`.

Unit tests (`query.rs`): one test per row of the table, plus the transitions Stop→UserPromptSubmit (back to Working), PermissionRequest→PreToolUse (Waiting clears), PreToolUse at 31 s (⏳), and a `Notification` whose detail is not a permission (ignored, previous state holds).

## The live screen

`Mode::Live`. Layout, full width `W`, full height `H`:

```
 grok traces · live · 3 agents · 1 waiting                          sab-mini ● sab-air ●   ← header, 1 row
 ⚠ claude  Traces packet on Mini            permission: Bash cargo build --release   2m   ← root row
   ● sonnet  review spec                    Read docs/traces/SPEC-live.md            4s   ← child, indent 2
   ○ sonnet  snapshot tests                 your turn · 1m
 ● codex   fork lead  ctx 142k              Edit src/views/traces_modal.rs           1s
 ◌ grok    air: khoj worker  ctx 31k        quiet 4m · Bash rg --files
                                                                                          ← empty rows
 enter pane   o open   j/k   c children   q quit                                           ← hints, 1 row
```

Row = `glyph source  title [ctx Nk]  state-text  age`. Columns: glyph 1, source 7 (`source_label`), title 28 (`clean_name`, `label` for children, `host:` prefix for remote), ctx 9 (blank when `context_tokens` is `None`), state text fills, age 4 right-aligned (`fmt_age`). Truncate title and state text with `…` using the existing word-aware wrap helper from `traces_outline.rs`; never split a token. Title and state text never change between collapsed and expanded (the header-stable rule from `traces-polish`).

Rows are ordered: `Waiting` first, then `Prompt`, then by `last receipt at` descending. Children sit directly under their parent in that same order; a child whose parent is not live is shown as a root with `↑ <parent title>` in the title column. Selection is kept by session id across polls (`keep_selection`).

### Keys

| Key | Action |
|---|---|
| `j`/`k`, arrows, mouse wheel | move selection |
| `c` | collapse/expand the selected root's children (default expanded; collapsed shows `+N` after the title; a collapsed root with a `Waiting` child shows `⚠` on the root too) |
| `o` / `Enter` on a row when no pane is known | open the trajectory (existing `Mode::Trajectory`, `include_children` on; see below); Esc returns to live |
| `Enter` when a pane is known | focus the agent's pane (below) |
| `q` / Esc in live mode | `grok traces`: exit process. `/traces`: back to list mode |
| `/` | existing search, over title and state text |

### Trajectory with children inline

`EventFilter.include_children: bool`. When set, `list_events` unions the children's non-noise events, ordered by timestamp, each tagged with `child: Option<(session_id, label)>`. The outline renders a child event as the existing event row at depth+1 with `label ·` prefixed to the header. Collapsed turn rows count child events in their `+N`. Child rows fold under the parent's turn by timestamp; they are never reordered within a child. Snapshot tests cover: parent with two interleaved children collapsed, expanded, and one child with zero events.

### Jump to pane

If `herdr` is on `PATH`, the live screen runs `herdr pane list` once every 10 s (off the UI thread, same worker pattern as `spawn_refresh`) and maps `agent_session.value` to `SessionRow.native_id`. A matched row gets `⌖` after the age. `Enter` runs `herdr agent focus <pane_id>`; failure shows a one-line notice (`TracesNotice`). Without herdr, `Enter` opens the trajectory. This is the only action; no sending keys, no prompting.

### States the renderer must define (snapshot tests, `TestBackend`, `render_tests.rs` pattern)

1. Empty: no live sessions. Body: `no live agents · hooks: <N> receipts in the last hour · grok-traces doctor` (N from `receipts_watermark`-style count; 0 receipts ever → add `install: crates/codegen/xai-grok-traces/hooks/install.sh --apply`).
2. One of each state in the table, in one ledger, at W=120.
3. Same at W=80 (ctx column drops, title 20).
4. Overflow: 50 rows at H=20, selection at the bottom, scrolled.
5. Children expanded and collapsed, including a collapsed root with a waiting child.
6. Index tripped (`health` red): header shows `index tripped · grok-traces doctor --reset`, body still renders.
7. Remote host unreachable: header shows `sab-air ✗`, local rows unaffected.
8. Pane known (`⌖`) and unknown.

pty proof (`pty_e2e`): launch `grok traces`, pipe a `PermissionRequest` payload into `grok-traces hook claude`, assert the `⚠` row within 2 s; pipe `PreToolUse` for the same session, assert the row changes to `▶`.

## Non-goals

- No web UI, websocket, or notification center. Herdr's notifications already exist.
- No per-event live rendering of tool output; the inspector is the existing view.
- No reading of transcripts outside the index.
- No per-model context window percentage; raw `ctx Nk` only until the model window table exists somewhere real.

## Delivery order

1. Data: schema 7 (receipt `detail`, usage `context_tokens`), `live_receipts`, `LiveState` + tests, installer events, remote `live --json`. Separate commit, `cargo test -p xai-grok-traces`.
2. `Mode::Live` render + state ordering + snapshot tests 1–7.
3. `grok traces` subcommand + Esc-quits + pty proof.
4. Trajectory `include_children` + snapshot tests.
5. Herdr jump (state 8).

Each step merges on its own. Steps 1–3 are the deliverable; 4–5 are the next packet if the lead runs out of road.

## Open questions for the reviewer

- Is 10 min the right live window, or should `Quiet` run to the existing 5 min `ACTIVE_WINDOW_MS` and drop?
- Codex: confirm which hook events its `PermissionRequest` payload carries (`tool_name`, `tool_input`?). If absent, `Waiting` text is `permission` with no detail.
- Should `/traces` default to live mode when any session is live? I say no: history is the modal's job, live is the subcommand's.
