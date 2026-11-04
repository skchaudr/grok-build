# SPEC: traces, the product

Status: draft for Sab's approval, 2026-11-03. Written by Claude (Opus 5.5) and revised after an Opus review against the code. This isn't a new design. It's everything Sab has asked for, built on grok-build's own session view. Once approved, it's the one spec: SPEC-MVP.md, SPEC-live.md, SPEC-usable.md and PROPOSAL-stage3-agents.md are history.

## Decisions made without Sab (work never waits on these; he can change any of them later)

Newest last. "Departs" means it goes beyond or against his words.

1. grok's top line shows harness · model · folder. Departs: he never said what it shows.
2. With Phoenix unreachable, the status line says "Phoenix offline". Departs: not asked for.
3. OMP and bb sessions open in the same view as plain text, calls and results. Departs: not asked for.
4. Emitter and hook files (Claude `otlp-emitter.sh`/`otlp-ship.py`, Pi `otlp-observe.ts`/`traces.ts`) are tracked in `~/.dotfiles` (`agent-obs/`, `pi/extensions/`) and installed from there.
5. Every slice merges when review and tests pass; grok is rebuilt and installed only at the three checkpoints (after A2, after A4, at the end). B1's hook files install when it merges.
6. Running rows and liveness come from receipts, because Phoenix only holds finished spans (checked live).
7. Pi spans carry Pi's own session id; dpi matches by the uuid in `<uuid>:N`; grok's emitter goes to one span per call in C1; traces queries the Phoenix project named after the harness.
8. The other machine's running calls come from its receipts over ssh (`remote.rs`), since Phoenix has a call only once it ends.
9. grok live uses grok's own viewer attach; if attach can't reach a session started in another terminal, it uses receipts plus that session's `updates.jsonl` (already grok's events, no converter).
10. `/traces` inside a grok session opens a past session in its own agent view, leaving the user's session untouched; Esc returns.
11. `Enter` opens the block viewer on Other and Skill rows (tool name, full input, full output). This also changes grok's own sessions. Departs: grok-build behavior changed.
12. Harness messages in a Claude transcript (`<task-notification>`, `<local-command-stdout>`, `<system-reminder>`, interrupts) are not shown as user prompts; slash commands show as grok shows one.
13. Work is split into pieces of one job each (about one file or function); anything touching grok's handler, view or tracker is built by Opus.
14. grok's hook config (`~/.grok/hooks/traces.json`) is tracked in `~/.dotfiles` as `grok/hooks/traces.json`, beside the Claude and Pi emitters.
15. Pi `grep` shows as Search, like Claude's Grep, so it folds with the other searches (departs: the Pi table below lists only `find`). Codex web searches show as Search and MCP calls as Other rows; Codex extension, subagent-activity and compaction items are not shown.
16. The flaky `tests/remote.rs::events_and_raw` (timed out once in ten full-crate runs) gets its own fix piece in the next batch.

## What traces is

Two views, both drawn by grok-build's own session view:
- **Live follow view:** the session running now, streaming.
- **Past sessions trajectory view:** any earlier session, replayed.

Subagents open the way grok-build opens its own, at every depth. Nothing is re-drawn beside grok-build's view.

## Harnesses

- **Must be excellent:** Claude, grok, Pi, dpi.
- **In Phase 1, after those four:** Codex.
- **Not now:** Cursor.
- **Out:** DeepSeek, which has its own view.
- OMP and bb stay in the sessions list. They open in the same view with plain text, calls and results, so nothing that works today is lost.

## What grok-build already has (checked in the code)

- **Feed.**
  - grok's session-event handler (`app/acp_handler/mod.rs`, `session_notification.rs`) is what fills a real session view.
  - Session updates (user message, assistant text, thinking, tool call, tool result) go through `AcpUpdateTracker::handle_update` into the scrollback.
  - Plans go to the todo pane (`acp_handler/mod.rs:242`).
  - Subagent rows and child views come from `XaiSessionUpdate::SubagentSpawned` (`session_notification.rs:664-760`).
- **Typed blocks** (`tool_call_to_block_inner`, `scrollback/blocks/tool/`):
  - Execute keeps the full command, description, output, error and elapsed time. Collapsed it shows the description; truncated it shows the head and tail of the real output.
  - Read, Edit (with diff), Search (with a WebSearch variant) and ListDir have their own blocks.
  - Runs of reads and searches fold into one line, "Read 2 files, Searched 1 pattern" (`scrollback/state/verb_group.rs`).
- **A traces session view with no agent behind it** already exists: `app/dispatch/standalone_traces.rs`.
- **Subagent takeover** (`ViewSurface::ChildTakeover`, `app/subagent.rs`), the timeline rail (`views/timeline.rs`), `/jump` (`views/jump.rs`), and the shortcuts sheet (`views/shortcuts_help.rs`).
- **Keys** are grok-build's: `e` expands in place, `E` expands or collapses everything (`ToggleExpandAll`), `Enter` opens, `j/k` nav, `Shift+l/h` turn, `Ctrl+e` thinking, `Ctrl+.` the key map sheet, `Ctrl+P` or `?` the command palette (`actions/defaults.rs`), `G` tail.
- **Width:** it works at half screen width, as grok's view does.
  - `e`, `j/k` and `Shift+l/h` are vim-mode keys (`actions/mod.rs:452-465`). Both machines run with vim mode on, and tests set it.
  - The footer shows about six keys and changes with the row.

## What traces already has (reused, never rebuilt)

- **Sessions index** (`index.rs`, `$GROK_HOME/traces/traces.sqlite`): finds sessions for every harness. It fills a call's `duration_ms` from its matching result timestamp.
- **Dispatch tree** (`links.rs`): which session started which. It covers claude (`<session>/subagents/agent-*.jsonl` + `.meta.json`), codex (`parent_thread_id`), grok (`summary.json` / `subagents/<child>/meta.json`), dpi (`owner_conversation_id`), bb, omp and cursor. Pi has no parent field in its files.
- **Hook receipts** (`hook.rs`, `receipts.rs`): event, session, transcript path, tool, `tool_use_id`, `agent_id`, pid.
- **Receipt-to-transcript matching and child discovery** (`follow.rs`): by `tool_use_id`.
- **Wake** (`wake.rs`): pushes a redraw when a receipt or index write lands.
- **Remote** (`remote.rs`): lists the other machine's sessions over ssh and reads their raw transcripts.
- **dpi reader** (`dpi.rs`): reads dpi's `agent.sqlite` in place.

## Phase 1

**Sab can:** open `/traces` (or `traces` in a shell) and see his sessions. He opens the live one in the follow view or any past one in the trajectory view, and reads it exactly as he reads a grok session. He steps into any subagent, and into its subagents, and back out.

### Feed

- **Structure** (which calls, when, how long, status, parent and child) comes from Phoenix spans. A session with no spans has none because it predates the emitter fix or its harness has no emitter. For those, structure comes from what traces already has: the dispatch tree (`links.rs`), receipts, and index durations.
- **Content** comes from each harness's transcript: text, thinking, the call's arguments and output. A converter maps content only. It never re-derives structure that spans or `links.rs` already give.
- Both are turned into the same events grok's handler receives: session updates, plans, and subagent spawns.
- They go through grok's handler, not straight into the scrollback, so plans and subagents work.
- Neither traces' heuristic normalizer nor grok-build's Claude import or resume is used.
- **Full content:**
  - Every call carries its full command, description, output, error and elapsed time.
  - Edits carry their diff. Reads and searches carry grok's structured output where the transcript has the data.
  - The index's 100-character previews are only for the sessions list.
- **Durations:** calls show their real duration, from the span's start and end, else the index's `duration_ms`. The tracker gets a small change so a finished call can carry a duration (today, Execute elapsed time only comes from a live clock: `execute.rs:78-92`, `finish_execute_block`).
- **Filing:** a tool call is filed under the assistant turn that made it, never under the user's prompt.

**Claude** (`~/.claude/projects/*/*.jsonl`):
- User text, assistant `text`, `thinking`, `tool_use`, `tool_result` (`is_error` → failed).
- **Kinds:**

  | Claude tool | grok-build kind |
  |---|---|
  | Bash | Execute |
  | Read | Read |
  | Grep, Glob | Search |
  | Edit, Write, MultiEdit | Edit, with a diff built from old and new text |
  | WebFetch | Fetch |
  | WebSearch | Search (WebSearch variant) |
  | TodoWrite | plan |
  | Task, Agent | subagent spawn |
  | anything else | Other, with its name and input |

- **Skipped:** `api-request-blob` lines, which repeat content.
- **Empty thinking** (signature only) shows as "Thought for Ns" with no body.
- **Large outputs** stored as stubs are read from `<session>/tool-results/*.txt`.
- **Subagents:** from the span tree, else `links.rs` (`<session>/subagents/agent-<id>.jsonl` and its `.meta.json` `toolUseId`).

**Pi** (`~/.pi/agent/sessions/*/*.jsonl`):
- user/assistant `message`, with content `text` / `thinking` / `toolCall {id, name, arguments}`, and `toolResult {toolCallId, content, isError, durationMs}`.
- **Kinds:**

  | Pi tool | grok-build kind |
  |---|---|
  | bash | Execute |
  | read | Read |
  | edit, write | Edit |
  | find | Search |
  | ls | ListDir |
  | subagent | subagent spawn |
  | anything else (including workflow) | Other |
  | codemode | its nested calls show as the calls they are |

- **Branches:** Pi sessions are trees. The view shows the active branch, the chain of `parentId`s from the last entry. Compaction entries show as grok's compaction row.
- **Subagents:** from the span tree. Pi's files have no parent field (`links.rs`). Without spans, the child is `<session-dir>/<runId>/run-0/session.jsonl`, with `runId` in the `toolResult.details` of the subagent call. They nest to any depth.

**grok:**
- **Past:** replay its own `updates.jsonl` with the xAI updates kept, so subagents survive. Today `replay.rs:357` drops them.
- **Live:** grok's existing attach to a running session (session/load, viewer).

**dpi:** its sessions (`~/.dpi/backend/sessions/<uuid>/agent.sqlite`) are read in place, as traces does today, and turned into the same events.

**Codex:**
- Calls come from `event_msg` `item_completed` `CommandExecution` entries (command, output, exit code, duration), not from the `exec` JavaScript wrapper.
- Encrypted reasoning shows as "Thought" with no body.

### The view

- It builds on `standalone_traces.rs`. It's read-only: there's no composer, and cancel and rewind do nothing because there's no agent to send to.
- grok's own top line shows harness · model · folder. grok's status line shows alive or ended, and the running call (`Running Bash … 3.2s`).
- `Esc` goes back: from a subagent to its parent, then from a session to the sessions list. Nothing in a read-only view needs Esc for cancel or rewind.
- **Sessions list:** the existing traces list (`/traces`), live sessions at the top. `Enter` opens a session.

### Live

Phoenix gives the structure, and the transcript gives the content. Traces never has to guess structure from a transcript.

- **Phoenix spans are the structure.** Every call is a span with a real start and end, a status, and a parent: session → turn → call → subagent. That's who ran what, when, for how long, under whom, and whether it finished or failed.
  - The Claude emitter and Pi's `otlp-observe.ts` are fixed so each call is one span: start at Pre, end at Post. Today start equals end.
  - Each span carries `tool_use_id`, `agent_id`, `session.id`, `transcript_path`, and its parent span (the spawning call, for a subagent).
  - Pi spans carry Pi's own session id (today `otlp-observe.ts:103-106` falls back to `pi-<Date.now()>`, which links to nothing). dpi's `<uuid>:N` matches by its uuid.
  - grok's emitter sends one span per call with its call id (today two, Pre and Post, no id). Built in C1.
  - Traces queries the Phoenix project named after the session's harness.
  - Pi on the Mini ships again (`~/phoenix-lab/.venv` is missing).
  - Traces reads spans from Phoenix 21 on old-air (`100.109.233.105:6006`). There's no Rust client today; Phase 1 adds one.
- **Hook receipts are the same structure, locally and instantly.** `grok-traces hook` already records `tool_use_id` and `agent_id`. Receipts cover the moment before Phoenix has the span, and keep traces working when Phoenix is unreachable. Pi's `tool_execution_*` receipts already carry the call id (only Pi sessions started before `traces.ts` was installed lack them; Pi loads extensions at startup). The end receipt also carries `is_error`, `duration_ms` and `parent_tool_use_id`.
- **The transcript is the content.** Full command, output, text and thinking, matched to its span or receipt by `tool_use_id`.
- **One writer, one row per call.** Traces merges spans, receipts and transcript into one stream per session before grok's handler sees it.
  - When a span or receipt reports a call starting, traces first reads the transcript tail (a local read) and emits any text and thinking that came before the call. Then it opens the call's row right away, as running, with the span's real start time.
  - The transcript's arguments and output fill that same row while it's still pending. grok's tracker updates a pending call in place (`tracker.rs:1326-1340`).
  - A call is completed only once its result content is in. The end time and status come from the span or receipt. So grok's tracker never sees a fill after completion and never draws a second row.
  - If the transcript hasn't flushed the earlier text yet, the row still opens on time and the late text follows it. Calls are never held back waiting for the transcript.
- **Subagent links** come from the span tree: the child span's parent is the spawning call. The child's transcript fills its content.
- **Phoenix only ever holds finished spans** (checked live 2026-11-03: 0 of 3,000+ spans had no end; OTLP exports a span when it ends). So a running row and a session's liveness come from receipts; Phoenix gives the call's real end, status and parent once it finishes.
- **Liveness:** alive, ended or failed comes from the session's receipts (last receipt, its pid), never from guessing at transcript silence.
- **Wake:** new spans, receipts and transcript lines wake the view through traces' existing wake notifications (`wake.rs`), not polling. New content reaches the tail within about a second. Scrolling up stops following; `G` goes back to the tail.
- **The other machine's sessions:** finished calls come from Phoenix; running calls from that machine's receipts and content from its transcript, both through the existing ssh remote path (`remote.rs`), in the same view.
- **Phoenix unreachable:** receipts and transcripts keep everything working, and the status line says Phoenix is offline.

### Subagents

- A subagent call shows as grok-build's subagent row. `Enter` opens its session in place. `Esc` steps back.
- grok's Tasks pane (`views/tasks_pane.rs`, running first, then newest) lists the open session's subagents, as it does for a grok session.
- A subagent's own subagents open the same way, to any depth. The one-level limits (`subagent_takeover.rs:53-68`, `acp_handler/routing.rs:118-130`) are lifted.
- Reopening a finished child rebuilds it from its harness transcript through the same feed. It no longer rebuilds only from grok's `updates.jsonl` (`ensure_subagent_child_replayed`, `subagent.rs:591-657`). Closing and reopening never comes up empty.

### Left open for Phase 2

The view is grok's real session view with the composer hidden, not removed. Every session keeps what Phase 2 needs to drive it: harness, pid (receipts already record it), and its resume command. It also keeps the terminal pane it runs in, where Rex or Herdr reports one.

### Build order

Past-session replay first, for Claude and Pi: it proves the converter, the feed and the view, which everything else uses. Then live (spans, receipts, merge), then subagents, then grok, dpi and Codex.

### Removed

The old follow and trajectory drawing code, keys and footers, and tests that only exist for them. This view replaces them.

### Checkpoints (not gates: building never stops for them)

Screenshots of the real window in Rex (or Herdr) are sent to Sab, and building continues.
1. Past Claude and Pi sessions in the trajectory view: one-line calls with verbs, grouped reads, `e` expanding in place, `E` expanding and collapsing everything without losing your place, the footer, `Ctrl+.` and the command palette, at full and half width.
2. The live follow view on a running Claude session and a running Pi session: a call's row appearing as running the moment its hook or span lands, then filling in place. A nested Pi subagent opened two levels deep and back.

### Phase 1 ends when

- Claude, grok, Pi and dpi are usable in both views, and Codex after them.
- Several reviewers (Claude agents and other models) have visually verified it more than once along the way, from screenshots of the real window compared with Sab's grok-build screenshots.
- Tests cover each converter, `e` in place, the merge (span or receipt before transcript, one row per call, completed only with content), Phoenix offline, subagents two levels deep and back, and reopening a closed subagent.
- Nothing regressed, and nothing diverges from grok-build's session view.
- It's installed on the Mini and sab-air.

## Phase 2: interactivity

Sab drives a session from the same view: the composer is shown, and input goes to the harness, either directly or through the pane it runs in (Rex, Herdr). It gets specced when Phase 1 is done.

## Rules

- grok-build's behavior first. A traces-only key or footer entry needs Sab's word.
- If a real blocker comes up during the build, it's reported with its evidence. Everything else is built as written.
- Build on the Mini, install on the Mini and sab-air (copy to `.new`, then `mv`), on every merge.
