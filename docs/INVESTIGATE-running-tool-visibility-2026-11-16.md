# Investigate: running and background tool calls (2026-11-16)

Brief: `docs/briefs/investigate-running-tool-visibility-2026-11-16.md`. Read-only: no code edits, no cargo, no install, no prompt sent in any TUI. Specimen: the `grok claude` session whose Claude id starts `be1e228d`, 13:20 to 13:43 PST. Session content is not quoted here; only timestamps, tool names, durations and field names.

Paths are under `crates/codegen/` unless they start with `~`. The adapter is `claude-agent-acp` 0.76.0 (`~/.local/share/claude-agent-acp/current/dist/`).

## What the specimen shows (read from artifacts now)

| Call (Bash) | Transcript `tool_use` | `PreToolUse` receipt | `PostToolUse` receipt | Length (transcript) |
|---|---|---|---|---|
| 1 | 13:20:12.978 | #28255, 13:20:13 | #28258, 13:24:29, `duration_ms` 256169 | 256 s |
| 2 | 13:24:47 | #28261, 13:24:47 | none, by `tool_use_id` | 218 s |
| 3 | 13:28:42 | #28267, 13:28:42 | none, by `tool_use_id` | 187 s |
| 4 | 13:41:19 | #28278, 13:41:19 | #28279, 13:43:20, `duration_ms` 120192 | 120 s, moved to background by the harness |

(`trace_receipts` in `~/.grok/traces/traces.sqlite`, native session `be1e228d…`.) Calls 2 and 3 have a Pre and no Post or PostToolUseFailure receipt at all (call 2 ended at about 13:28:25, five seconds before a `UserPromptSubmit` receipt at 13:28:30). Receipt-only surfaces (Steps, `follow`, board) therefore never see them close: the Steps page shows them `running` until the 10 min live window lapses (`steps.rs:27`) or the transcript's result line arrives. Call 1's input carries `description` and `timeout: 600000`; its command starts a Rex tab (`rex run … --label <tab>`) then polls `rex capture -b <tab>` in a `sleep 15` loop. So the call is one foreground Bash whose real work lives in a Rex tab.

## Case 1: foreground call that runs for minutes

**A. What each surface shows, and when the row first appears**

- **Chat scrollback.** The adapter emits `tool_call` with `status: "pending"` as soon as the streamed `tool_use` begins, before the command runs (`dist/acp-agent.js:7251-7280` `toolCallNotification`; refine update when the input is complete, `:7286`). The pager pushes a card at once and marks it running (`xai-grok-pager/src/acp/tracker.rs:1434-1549`, `set_last_running` at `:1540`, `PendingTool.started_at` at `:1549`). The Execute card shows the Bash `description` (command hidden while collapsed) and an animated accent bar while running (`scrollback/blocks/tool/execute.rs:703-713`). Its header carries no time: `elapsed_ms()` exists (`execute.rs:113`) but is read only by `tool_usage.rs:369`, `codemode.rs:227`, `sent_message.rs:232`. Stdout is not streamed: the adapter streams `terminal_output` only to a client that advertises `_meta.terminal_output` (`acp-agent.js:5198`, `:7313`), and the pager never sets it (no match outside the accumulator, `tracker.rs:1643-2080`). Output lands with the result.
- **Status row.** The tracker turns a pending tool into `TurnActivity::ToolRunning { title, description }` (`tracker.rs:714`). `views/turn_status.rs:672-690` draws a spinner, the Bash `description` (`:488-500`) and a per-activity timer (`:433-436`, `format_turn_timer`). By code this is the surface that tells Sab a call is running and for how long.
- **Dock.** Not read beyond `views/dock` using `fmt_elapsed`; see "Could not establish".
- **F3 side panel, Steps page.** Row appears at the first 1 s poll after the `PreToolUse` receipt is written. Chain: hook (`hook.rs::ingest`, `:132`) writes the receipt; `Follower::tick` reads `receipts_after` (`xai-grok-traces/src/follow.rs:586-604`) and prints a `call` item with `ts = receipt time`, no `line`; `Follow::apply` makes a `Call` with `line = -1`, `started_ms = receipt time` (`views/traces_follow.rs:277-286`); `steps.rs:134` draws `◆ … running … 12s` counting from `started_ms`. Gates: the panel must have found the session in the index by the harness id (`side_panel/mod.rs:372-396`, `look_up` retries every second) and the session must be live, last event ≤ 10 min (`steps.rs:27`). The hook entry is `async: true` in `~/.claude/settings.json`, so the row lags the call by the hook's process spawn.
- **`grok-traces follow`.** Same `Follower`: prints `→ Bash  <first command line>` on the first tick after the receipt, no timer while open, `   ✓ <took>` on the `PostToolUse` (`follow.rs:586-625`). `--once` prints one tick and exits, so a running call prints once with no elapsed.
- **Traces board.** `live_rows` come from `live::fold_session` (`xai-grok-traces/src/live.rs:815`): an open `Pre` with no `Post` gives state `Tool` with the call's text and `since_ms = now − Pre.at_ms` (`live.rs:800`). Receipt-only; no transcript needed.

**B. Data at that moment**

- ACP: pending `tool_call` with `rawInput`, `_meta.claudeCode.title` = `description` (`acp-agent.js:7152-7170`); `tool_progress` heartbeats become `tool_call_update` `in_progress` with `_meta.claudeCode.toolResponse.elapsedTimeSeconds` (`acp-agent.js:4228-4275`). The pager reads no `elapsedTimeSeconds` (no match in `xai-grok-pager/src`).
- Hooks: `PreToolUse` payload has `tool_name`, `tool_input` (`command`, `description`, `timeout`), `tool_use_id`; stored with `at_ms`, `detail`, `description` (`hook.rs:57-86`, table `trace_receipts`).
- Transcript: the `tool_use` line is on disk **during** the call. Live test, this session, 2026-11-16: a Bash call that slept 7 s and then grepped its own transcript for a marker in its own command found it (count 1). That is Claude Code's writer, same SDK as the adapter; the adapter session itself was not tested.

**C. Smallest change.** Nothing is missing in the data. Code says the card, status row, Steps page and board all show it. The Execute card's missing elapsed is the one rendering gap: `execute.rs` header (one file, plus snapshots). Everything else is blocked on a screen check this brief could not do (see below).

**`(waiting for the transcript)`.** Source: `views/traces_follow.rs:586` in `body_rows`, which draws the opened body of a call. It is not on the Steps page (`steps.rs` draws no bodies). It means: the row was created by a `Pre` receipt (`line == -1`) and no `link` item has set its `line` yet. A `link` is emitted when the follower's file pass reads the transcript's `tool` event for the same call id while `rcalls` holds that id (`follow.rs:786-830`, applied at `traces_follow.rs:288-297`). To show the running call it needs: the index to ingest the transcript line carrying that `tool_use` (the follower calls `self.index` on the root file every tick, `follow.rs:735,751`) and `list_events` to return it with role `tool`. Once it does, the body is the command only (`compose_body` with no result). That the state persisted during the specimen is not explained: the line was on disk at call start.

## Case 2: background task that outlives its turn

**A.**
- **Chat.** The Bash call returns at once with text `moved to the background (ID <id>). Output is being written to <path>` (transcript line at 13:43:20; `toolUseResult` fields `backgroundTaskId`, `timedOutAfterMs`, `backgroundCwdHint`). The card finishes (✓) at that moment and shows that text. The lifecycle (`async_task_spawned`, `async_task_progress`, `async_task_state_update`, with `description`, `outputFilePath`, `toolCallId`) is published only if the client advertises the JetBrains AIR `asyncTasks` capability (`dist/async-tasks.js:2-35`, `acp-agent.js:1880`, `:7176-7200`). The pager has no code mentioning `asyncTasks` or AIR (`rg` over `crates`), so it never gets them. Its own background list, `session.bg_tasks` (`app/agent.rs:764`), is filled from grok's `x.ai/task_backgrounded` family (`headless/ext_protocol.rs:107`), which this adapter never sends. So the Agents page (`app/agent_view/side_panel.rs:99-118`), the dock and the status row's background count are empty for a Claude background command.
- **Steps page and `follow`.** The `PostToolUse` closes the call: `✓ 2m` (`follow.rs:605-650`). It reads as finished while the command runs on.
- **Board.** `Ev::Post` ends the open tool (`live.rs:447`); the row goes back to Working/Prompt. Its `bg` count is open child agents only (`live.rs:831-849`), not background commands.

**B.** `PostToolUse` `tool_response` has `backgroundTaskId` (both for `run_in_background` and for the auto-background; auto adds `timedOutAfterMs`). Captured payloads: `~/.grok/traces/capture/claude/20261116T134320-26867-PostToolUse.txt` (auto, 120 s) and `…/20261021T234637-5143-PostToolUse.txt` (explicit). `hook.rs::extract` drops it: `response_text` (`hook.rs:91`) returns `""` for empty stdout/stderr, so the receipt's `result` is NULL (receipt 28279). A backgrounded call and a quick no-output call look the same in `trace_receipts`. Output file: `/private/tmp/claude-501/<project slug>/<session id>/tasks/<task id>.output` (exists on disk for the specimen's tasks; for a subagent it is a symlink to its `subagents/agent-<id>.jsonl`). Completion: the transcript gets a `queue-operation` `enqueue` line whose content is a `<task-notification>` with `<task-id>`, `<tool-use-id>`, `<output-file>` (seen at transcript lines 484 and 491). No hook event carries it in the registered set.

**C.** See changes 2 to 5 below.

## Case 3: work started elsewhere (Rex tab)

**A.** Nothing in the harness knows about the tab. The foreground call (Case 1) is visible as a Bash call whose first line is the `rex run` command; the tab label appears only as text after `--label`. The Rex tab's own state is not read by any of the five surfaces (no `rex` reference in `xai-grok-traces/src` or the pager's traces views).

**B.** `tool_input.command` (in the receipt `detail` and the transcript) and `tool_input.description` (receipt `description` column). The label is not a field. The hook payload has no tab id.

**C.** Smallest: show the call's `description` (already in receipts and in the transcript) next to or instead of the first command line on the Steps row: `follow.rs` adds `description` to the `call` item, `steps.rs` and `traces_follow.rs::Call` carry it. It names the tab only when the author wrote the description that way. Anything stronger (live tab state, output path of the pane) needs a Rex query (`rex capture`) from the pager, which no existing code does; that is a new dependency, not a small change.

## Proposed changes, smallest first

1. **Execute card elapsed.** Draw `elapsed_ms()` in the running card's header. Touches `scrollback/blocks/tool/execute.rs` (header) and its snapshots. Chat only.
2. **Keep `backgroundTaskId` in the receipt.** In `hook.rs::extract`/`response_text`, when `tool_response.backgroundTaskId` is set, store a short result such as `moved to background (ID x)`. Touches `hook.rs` and its test; no schema change. Unblocks 3 and 4.
3. **A backgrounded call does not close at its `Post`.** `follow.rs` Post arm and the `Ev::Post` fold in `live.rs:447` leave a call open when its result carries that marker, and close it on the transcript's `<task-notification>` for the same `tool-use-id` (needs `normalize.rs`/`classify.rs` to surface that `queue-operation` line, now treated as noise). Touches `follow.rs`, `live.rs`, `normalize.rs`; Steps shows `running` until the notification. Size: three traces files plus tests.
4. **Output location on the row.** The result text already names the `.output` path; the Steps/follow body can show it from the linked result line. Touches `traces_follow.rs` (body) and `steps.rs` (optional third column).
5. **`description` on the Steps row** (Case 3). Touches `follow.rs` (call item), `traces_follow.rs` (`Call`), `steps.rs`.
6. **Chat/Agents page for Claude background commands.** Advertise AIR `asyncTasks` in the pager's initialize `_meta`, decode `async_task_*` (not in the `acp` crate's `SessionUpdate`, so an ext decode like `headless/ext_protocol.rs`), and fill `session.bg_tasks`. Touches the ACP client setup, `acp/tracker.rs`, `app/agent.rs`, the dispatch that owns `bg_tasks`, plus the adapter's `canStop`/kill path (`Effect::KillBgTask`, `app/actions.rs:1901`) which assumes grok's. Largest.

## Could not establish

- What `grok claude` actually drew during the four calls. I sent no prompt, so no running call existed to look at, and a Claude-host session directory keeps no ACP log (`~/.grok/sessions/<cwd>/be1e228d…/` holds only `images/`). Code says card + status row + timer should appear; if Sab saw nothing, the cause is not in the paths read here.
- Whether the adapter sends `tool_progress` heartbeats for a foreground Bash (code path exists, `acp-agent.js:4228`).
- Why the follow/Steps body stayed on `(waiting for the transcript)` when the `tool_use` line was on disk.
- Why `grok-traces follow --once` hung 120 s on the live session (`--once` implementation not read; the hung call was the one the harness moved to the background).
- The dock row, beyond `fmt_elapsed`.
- Whether `async: true` hooks on `PreToolUse` delay the receipt by more than a spawn; receipt `at_ms` for call 1 (13:20:13) matches the transcript's 13:20:12.978 to the second.
