# SPEC-MVP: the traces follow mode

Draft 2026-11-01, written by Claude (Fable 5.1) from facts checked on the mini that night, for Sab's review. Supersedes nothing: SPEC-live.md (state machine, live screen) and SPEC-usable.md (rows, semantic layer) stay; this file is the bar they now serve. Phoenix and trace-deck are consumers of the stream this produces, not part of it.

## The bar (Sab, 2026-11-01)

> In the grok TUI, follow one agent live and know without guessing which agent is acting, what it is running right now, and whether it is alive, with the real command and output on each row. Then the same stream feeds trace-deck and sab-air. Subagents are visible and are as accessible as the parent agent. Navigating agent to agent is intuitive. The UI is not crowded, does not feature untested and non-functional UI chrome, every function listed that presents itself as working, works. The TUI is purpose built for fast paced information processing and keyboard navigation; the renderer already does this, the frontend does not take advantage of it. The only signal presented to the user is the one that matters the most during follow mode, with all else only aiding in the success of follow mode. A full TUI screen can be used, or a half; the TUI ought to follow responsive web design philosophy and work even if cut in half. This is the minimum foundation required so that eventually, tools like Rex can launch floating panes that become windows into observability, where steering, cancelling, interrupting the agent through the interface becomes possible.

## Where it stands (mini, 2026-11-01)

**The state machine exists and is starved.** `live.rs::norm` maps hook events to `Pre, Post, Idle, Prompt, End, Ask, ChildStart, ChildStop, CompactStart, CompactEnd`. `trace_receipts` has `agent_id`, `tool_use_id`, `tool`, `detail` (the command or path, via `classify::call_detail`), `cwd`, `harness_pid`, indexed by `(session_id, at_ms, id)`. The table is capped by row count (`receipts.rs:136`). A child's receipts arrive under the parent's session with `agent_id` set. This session had 456 receipts, all keyed.

**Claude.** `~/.claude/settings.json` runs `grok-traces hook claude` on 8 events: PreToolUse, PostToolUse, PostToolUseFailure, SessionStart, SessionEnd, Stop, SubagentStop, UserPromptSubmit. Capture-only (never ingested): SubagentStart, Notification, PermissionRequest, PermissionDenied, StopFailure, PreCompact, PostCompact. So `ChildStart`, `Ask` and `Compacting` never fire from real data. Payload facts from 26k captured files: every tool event carries `session_id, transcript_path, cwd, tool_name, tool_use_id, tool_input`; inside a subagent also `agent_id, agent_type`; `SubagentStart` carries `agent_id` but not the child's path; `SubagentStop` carries `agent_transcript_path`. Child files live at `<stem>/subagents/agent-<agent_id>.jsonl` or `<stem>/subagents/workflows/<wf>/agent-<agent_id>.jsonl`.

**Pi.** `~/.pi/agent/extensions/traces.ts` emits `session_start, turn_end, agent_end, tool_result (debounced), session_shutdown`. `norm` expects `tool_execution_start, tool_execution_end, agent_start`. Pi 1.3.0 has them: `ToolExecutionStartEvent { toolCallId, toolName, args, parentToolCallId? }`. `hook.rs` already accepts `toolCallId` as the tool-use id. pi-subagents passes `parentSessionId, parentSessionFile, parentToolCallId` to children. Pi tool calls therefore never reach the state machine today, and Pi children are never linked.

**grok.** `~/.grok/hooks/traces.json` registers UserPromptSubmit, PostToolUse, SessionStart, Stop, SessionEnd. grok-build has `HookEventName::PreToolUse` (`xai-grok-shell/src/extensions/hooks.rs`, `session/acp_session/hooks.rs`; `agent/subagent/mod.rs` says subagents inherit it). It is not registered. `norm` has no grok rows at all, so even grok's PostToolUse receipts are ignored. `docs/HARNESS-FACTS.md` row 47 ("grok-build has no PreToolUse") is wrong.

**Follow mode ignores receipts.** `follow.rs::Follower` polls transcript files each tick, finds children by `probe_subagents`: one level under `<stem>/subagents/`, mtime within `ACTIVE_WINDOW_MS`. Workflow children are never found. The TUI pane (`xai-grok-pager/src/views/traces_follow.rs`) derives live/ended from "any call in the last 64 rows without a result" and `Body::End`. This is why who is live, how fast rows arrive, and whether the agent is alive are all ambiguous: the pane is reading file flushes, not events.

## Stage 1: wiring

Every harness's events reach `trace_receipts` with the names `norm` maps and the ids the join needs.

| Harness | Change | File | Done when (do the action, then run the query below) |
|---|---|---|---|
| Claude | register `grok-traces hook claude` on the 7 capture-only events | `~/.claude/settings.json` | dispatch an Agent: a `SubagentStart` row with `agent_id`; the child's tool rows carry the same `agent_id`; a PermissionRequest yields a row |
| Pi | subscribe `tool_execution_start`, `tool_execution_end`, `agent_start`; emit `toolCallId` as the tool-use id and `toolName`. Child linkage: the builder decides where the parent is knowable. pi-subagents is not always active, so the receipt must be emittable from whatever does know (`traces.ts` if the extension context exposes the parent, pi-subagents at spawn if only it does); the receipt shape is one child-start row naming the parent either way | `~/.pi/agent/extensions/traces.ts`, possibly pi-subagents | run a tool: `tool_execution_start` then `_end` with the same `tool_use_id`; run a pi-subagents child: a child-start row naming the parent |
| grok | register PreToolUse; confirm the payload carries a tool-use id (name the field); check whether `agent/subagent/mod.rs` fires any start or stop hook; fix HARNESS-FACTS row 47 | `~/.grok/hooks/traces.json`, `docs/HARNESS-FACTS.md` | run a tool in grok-fork: `PreToolUse` with an id, before its `PostToolUse` |

```
sqlite3 ~/.grok/traces/traces.sqlite "select source,event,tool,tool_use_id,agent_id from trace_receipts order by id desc limit 10"
```

A missing row localises the fault to that harness's wiring. Nothing in Rust is involved.

## Stage 2: follow from receipts

The Follower takes structure from receipts and content from files.

**`live.rs`.** `norm` gains grok rows (`PreToolUse` Pre, `PostToolUse` Post, `UserPromptSubmit` Prompt, `Stop` Idle, `SessionStart` Idle, `SessionEnd` End) and the Pi child-start name chosen in Stage 1 as `ChildStart`. `strict_ids` stays: pi true, claude false.

**`follow.rs`.** Each `tick`:

1. Read the root's receipts with id greater than the last seen (`branch_receipts` already selects by session or native id). Map each through `norm`.
2. `ChildStart` with `agent_id`: adopt a `Followed` for that agent now, depth +1, no file. Emit the `agent_start` item. On later ticks attach the file when `<stem>/subagents/**/agent-<agent_id>.jsonl` exists; at `ChildStop` fall back to the receipt's `agent_transcript_path` if `hook.rs` is taught to keep it in `detail`.
3. `Pre` with `tool_use_id`: emit the call item now, with `tool` and `detail` from the receipt, under the agent the receipt names (`agent_id`, else root). Remember it as open.
4. `Post` with the same `tool_use_id`: close it with `duration = post.at_ms - pre.at_ms`.
5. File rows: a `tool_result` whose call id matches an open call attaches as its output; a call row whose id was already emitted from a receipt is not emitted twice; assistant and user text pass through as today.
6. `probe_subagents` is deleted. Everything else in the Follower is untouched.

`follow --json` keeps its item shapes; new fields are additive with `#[serde(default)]` so trace-deck keeps parsing.

**Done when.** One pty test beside `traces_modal_follow.rs`: a `SubagentStart` receipt makes the child row appear before any child file exists; a `PreToolUse` receipt makes the call row appear with the command before the transcript has the call; the matching `PostToolUse` closes it with a duration. Then the three real actions from Stage 1, watched in the pane after build and install.

**Not in this stage.** The badge, the other lifecycle states, any change to layout or keys, Phoenix, trace-deck, any refactor beyond the six points above.

**Kill criteria (Kimi, agreed).** Stop and reassess if the Follower needs a rewrite rather than the adapter above, or if the real child and tool rows are not visible in the pane by the end of the first working session. Fallback is the direct Phoenix live adapter in trace-deck.

## Stage 3: follow mode, the product

Each line below is a requirement and its test. The renderer stays. `traces_follow.rs` and the follow parts of `traces_modal.rs` are rebuilt to this list. This section is reviewed on its own before any frontend work is dispatched.

**Rule (Sab, 2026-11-01): use grok-build's existing key bindings faithfully before inventing new ones.** A new binding is allowed only for an action grok-build has no key for, and it must follow the conventions already in `xai-grok-pager` (vim keys and arrows equivalent, Esc back, Enter open). Anyone touching the frontend reads the pager's key handling and TOOL-traces-modal.md first.

**One agent per screen.** The followed agent's rows fill the pane. Other agents appear as one line each where they start and end. Test: follow a parent that dispatches two children; the parent screen shows two start lines and two end lines, no child rows.

**Agent to agent (Sab, 2026-11-01).** Following an agent that has subagents, Left (or `h`) opens the subagent dashboard: one row per subagent with its state line, model, current call and age, built for the case where there are many. From the dashboard, `j`/`k` select and Enter (or `l`) opens that subagent's full-screen follow; Esc (or `h`) returns to the parent's follow. Left on an agent with no subagents does nothing. `Tab`/`Shift-Tab` keep cycling agents as today. `h`/`j`/`k`/`l` and the arrow keys are equivalent everywhere. The header is a breadcrumb: root › child › grandchild, with each agent's model (it is in the transcript, not the panel; see `subagent-model-is-hidden`). Test: pty script opens the dashboard on a parent with two children, enters one, returns; Left on a childless agent changes nothing; header text asserted at each step.

**One loud signal.** Exactly one state line, from the `Ev` stream: `running <tool> <detail> Ns`, `thinking Ns`, `waiting on you`, `idle Ns`, `compacting`, `ended`. It is the only line in the accent colour. Errors are the only other colour. Everything else is one dim weight. Test: snapshot per state from receipts alone.

**Rows carry the real thing.** A tool call is one line, tool plus detail. The highlighted call shows its command and result inline under the row (as `traces_body` does today, 40-line cap) and Enter opens the full payload view (#29), unchanged. Assistant text folds to its first line and opens the same way. Noise rows (`classify.rs` already names them) are hidden by default and one key shows them. Test: highlight a Bash row; the inline command and output and the payload view are byte-equal to the transcript.

**No dead chrome.** Every key in the footer is exercised by a pty test or removed. Every pane, column and badge that does not serve following is absent from follow mode. The current inventory to cut from is ISSUES-ux-audit.md P1. Test: the footer's key list equals the set of keys with a passing test.

**Half a screen works.** The tool-call stream is the primary region and always survives. Side regions (Sab's example sketch, 2026-11-01, not binding: git state, tool-call preview, session info) exist only if they serve following, and they collapse first when width is cut. At 80 columns the detail truncates with an ellipsis and the inline body still shows. At 20 rows the header and state line stay and rows scroll. Test: the same pty script at 200×50 and 80×20 passes both.

**Keyboard first.** Today's bindings stay: `j`/`k` and arrows scroll and pause the tail, `g` first row, `G` resume tailing, `Tab`/`Shift-Tab` cycle agents, `e` next failed call, Enter opens the payload view, `o` the trajectory, `Esc`/`q` back. Added: Left/`h` subagent dashboard, Enter/`l` enter the selected subagent, Esc/`h` back to the parent. `h`/`j`/`k`/`l` and arrows are equivalent everywhere. No mouse-only action exists. Test: each key in a pty test; the hint line lists exactly the keys that have one.

**Event-driven redraw.** The hook wake socket (#38) exists; the modal still polls at 1 s. Follow mode redraws on wake, polling only as a fallback. Test: a receipt arriving redraws within one tick without waiting for the poll.

**Foundation for control, not control.** Every agent row carries the harness pid and session id from its receipts, so a later pane can address the process it watches. Steering, cancelling and interrupting are out of scope here.

**Done when** every test above passes on mini and sab-air, the screenshots are in the PR, and following a real Claude session with a dispatched child answers who, what and alive without the user asking.

## Delivery

Stages 1 and 2 are one spike, run at once: a Sonnet worker on `live.rs` and `follow.rs` (the only one touching those files), a Haiku worker on `traces.ts`, a Haiku worker on grok's hook registration, payload check and HARNESS-FACTS row 47, and the lead editing `~/.claude/settings.json`. Each worker returns a diff and one test on a branch. The lead builds grok-fork and grok-traces, installs via `.new` then `mv`, rsyncs to sab-air. Sab runs the three actions per harness and looks at the pane.

Stage 3 is dispatched only after this section is reviewed, one lane per bold requirement, each lane a pty test first.
