# Review 3: SPEC-live.md rev 3 (83f5a8d8), sequence tests

Reviewer: Opus lead, 2026-10-21. One Sonnet pass executed all 25 rows literally and replayed 3 real Claude sessions from a copy of the index. The replay script and its output are `/private/tmp/claude-501/-Volumes-Mini-SSD-offload-repos-grok-build/f4c22e45-2c52-4417-9052-54640a4e63b7/scratchpad/replay.py` and `out_*.txt` in the same directory; they are ephemeral.

**Verdict:** close. 21 of 25 rows match, 2 mismatch, and 6 omit `now`. Two blockers come from real data: phantom open tools, and an undefined `last_ev`. Fix those plus the Step 0 mechanics, and rev 4 can go to the implementer without another full review. A spot-check of the changed rows is enough.

## Blockers

**1. Phantom open Bash tools (real data).**
- 22 of 151 real turn ends carry an orphan Bash `Pre` (16/104, 4/41 and 2/6 per session).
- With FIFO-by-name pairing, those orphans pin rule 4 for the rest of the turn. In session `01bda…` the row read `▶ Bash ⏳` after an Edit `Post` 53 times.
- Likely cause (U): a `PreToolUse` hook denial. Sab's setup blocks recursive grep, `rm -rf` and some git operations; a denied call gets a `Pre` but no `Post` or `PostToolUseFailure`.
- Fix:
  - A `Pre` with no `tool_use_id` is closed by the next `Post` or `PostToolUseFailure` of *any* tool that finishes after it, or by any `Pre` of the same name that starts after it.
  - Add a row with the real shape: `Pre Bash(denied)`, then `Pre Edit`, then `Post Edit`, which should yield Working.
  - Add to Step 0: does a hook-denied call emit `PostToolUseFailure` or `PermissionDenied`?

**2. `last_ev` is undefined when the window holds only `ChildStart`/`ChildStop`.** This happened 4 times in real data. Fix: if there's no state-bearing event, take the state from the last one outside the window, or show Prompt when `open_children` is 0 and `● N bg` when it's above 0. Add a row for it.

**3. Step 0 relies on code that doesn't exist yet.** `grok-traces hook --dump` and `--env` aren't in `xai-grok-traces`.
- Make "Step 0a: add a `--dump <dir>` flag that writes the raw payload" an allowed code change: small, merges alone, and reverted or kept as a debug flag.
- Every capture needs three things: the exact command, the output that closes it (a yes/no field), and the fallback if the answer is no.
- Items 1–4, 6, 7 and 9 are missing at least one of these.
- No fallback is stated yet for:
  - 0.1 `tool_use_id`: use blocker 1's rule.
  - 0.2 `agent_id` on child tool receipts: without it, child tools pollute the parent. Fallback: send child events to the child via `transcript_path`, since the child's hooks may carry its own path (U).
  - 0.5 `GROK_SESSION_ID`: Grok `Ask` stays unattributed and shows `?`.
  - 0.7 herdr focus target: Enter opens the trajectory instead.

## Row fixes

- **Row 7:** `○ your turn · 9s` uses `now`, not age. The rules give 4 s, or 3 s if `ChildStop` counts as an event. Define whether `ChildStart`/`ChildStop` set `ev_at` (say no), then correct the expected value.
- **Row 12:** `live_receipts` drops it, because every receipt is older than 10 min. Either the query's "larger set" clause must explicitly return sessions with an open `Pre` or `Ask` at any age, or the expected result is "off screen". Pick one. Retiring an open `Ask` (row 4 below) argues for the cap.
- **Row 2:** the expected text has `+0`, but the rule prints `+N` only when N > 0. Fix the expected text.
- **Rows 1, 3, 5, 6, 21, 22:** state `now`.
- **Row 1:** say that "no later Post" means by `(at_ms, id)`.

## Rule gaps

1. A Compacting session with no `PostCompact` stays forever. Clear it on `Idle`, `Prompt` or `End`, or after 10 min.
2. An open `Ask` or `Pre` in a session that crashed without `SessionEnd` stays forever. Cap it at 30 min with the `?` mark, then drop it.
3. A Prompt row vanishes at 10 min. State whether that's intended. Sab's use case argues for keeping "your turn" rows until they reach 30 min.
4. With several tools open, `since(pre)` for `⏳` is undefined. Use the newest open tool.
5. The `✗` mark is placed after the age column, but Working and Done rows have no age column. Define the column.
6. Rule 14's no-receipt text contradicts rule 7 for a Pi session idle 2–5 min. Pick one.
7. `last_event_kind` is added but no rule reads it. Use it in blocker 1, or drop it.

## Coverage gaps (add rows)

- **C:** a `ChildStart` with a non-NULL `agent_id`, with the parent's count still including it.
- **D:** an old `Pre` kept by the "larger set" clause.
- **B:** a Prompt row with `· N bg`.
- **Untested rules:** rule 7 Quiet, Waiting from a `Notification` (ambiguity 8), the Codex `PermissionRequest` detail, a mixed per-session no-receipt row (ambiguity 6), ordering/tie-break (ambiguity 9), the remote older-binary fallback (ambiguity 10), `+N > 0`, and `End` clearing children.
- **Boundaries:** add one row each at exactly 2 s, 30 s, 60 s and 10 min. Today only 2 min is tested.

## Next

Fable makes rev 4 with these changes. I'll spot-check the changed rows only. Then Step 0a (`--dump`) plus the captures run before anything else. The implementer is a compacted Opus with Sonnet 5.5 subagents.

## Amendments from Sab (2026-10-21)

**Scope: dialed for some agents, not "alright" for all.**
- v1 is Claude Code, Pi and Cursor. Sab: Claude and Pi cover the most ground, and Cursor matters uniquely this month. All three must be completely right: every state, every test row, verified on real sessions.
- Codex comes later. Other sources stay out of the live screen and remain in `/traces` history.
- Cut from rev 4 all normalizer rows and tests for OMP, Grok, bb and DSH, plus the Grok notification-hook route and the remote `live --json`. Keep the data model source-agnostic so later sources only add rows.
- **Pi needs a richer extension.** Today `hooks/pi/traces.ts` sends only `session_start`, `turn_end`, `agent_end`, a debounced `tool_result` and `session_shutdown`, so Pi has no Tool state.
  - Pi's API also offers `agent_start`, `tool_call`/`tool_execution_start`, `tool_execution_end`, `message_end` and `input` (seen in `Pi-Coding-Agent` source).
  - v1 maps:
    - `agent_start` → Prompt event (turn begins)
    - `tool_execution_start` → Pre
    - `tool_execution_end` → Post, with the tool call id if the payload has one
    - `agent_end` → Idle
    - `turn_end` → no-op
  - Drop the `tool_result` debounce for these.
  - Step 0 captures one real Pi payload for each of those events: the tool-call id field and the session id.
  - The extension runs in-process, so its liveness PID is `process.pid`.
- **Cursor**
  - `~/.cursor/hooks.json` today has `beforeSubmitPrompt`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `afterFileEdit`, `postToolUse`, `postToolUseFailure`, `afterAgentResponse` and `stop`.
  - Map:
    - `beforeSubmitPrompt` → Prompt
    - `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile` → Pre
    - `postToolUse` and `postToolUseFailure` → Post
    - `afterFileEdit` and `afterAgentResponse` → Working
    - `stop` → Idle
  - Cursor has no permission event, so Waiting is a known gap. Show `?`, or drop it.
  - **Blocker:** Cursor's `transcript_path` may be null, which gives `session_id = NULL` (review 1, data #2). Key Cursor receipts by `conversation_id` (`native_session`) and join to `trace_sessions.native_id`.
  - Step 0 captures one real payload for each Cursor event: `conversation_id`, `generation_id`, any tool-call id, `transcript_path`, and the hook's parent process chain. Changing `~/.cursor/hooks.json` needs Sab's OK, and he sees the exact diff first.

**Liveness: no timers, no Herdr dependency.**
- Herdr stays optional. Its only use is the jump-to-pane key, and without it Enter opens the trajectory.
- On each receipt, the hook records the harness's process:
  - Walk up from the hook's parent PID past shells (`sh`, `bash`, `zsh`) to the first non-shell ancestor.
  - Store its PID and process start time, so a reused PID doesn't count as alive.
- A session is on the live screen until either a `SessionEnd` receipt arrives or that process is gone (`kill(pid, 0)` plus a start-time check).
- Remove every live-presence timer (10 min window, 30 min caps). "Your turn" rows stay for as long as the agent runs.
- Timers remain only for display: `⏳` at 30 s, the `?` mark, and the 2 s `Stop` debounce.
- Step 0 adds a capture: the process chain from the hook up to `claude`, as seen inside a Claude Code hook.
