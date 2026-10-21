# Review: SPEC-live.md (5f2ca074)

Reviewer: Opus lead, 2026-10-21. Facts come from three Sonnet read-only passes plus one live check. Paths are under `crates/codegen/`. (U) = unverified.

**Verdict:** the architecture is right. The hooks write receipts, the screen polls the index, there is no daemon, and there is one modal with two hosts. But the state machine, as written, puts wrong states on screen in common sequences, and several data claims name code that doesn't exist. The spec should be revised before it goes to implementers.

## Verified live

The subagent check came from this session's receipts in `~/.grok/traces/traces.sqlite`, while its Sonnet subagents ran WebFetch:
- **Subagent tool calls land on the parent's session.** `PreToolUse`/`PostToolUse WebFetch` rows have the parent's `native_session`, and I made no WebFetch calls myself. `SubagentStop` also lands on the parent.
- **Consequence:** the rule "the parent shows `▶ Agent <label>`, the child row shows its own state" cannot be built from receipts as the spec defines them.

## Wrong code references

| Spec says | Actual |
|---|---|
| `classify::tool_detail(tool, tool_input)` | `classify::call_detail(name, args)` at `xai-grok-traces/src/classify.rs:235`. The shape is compatible. |
| word-aware wrap helper in `traces_outline.rs` | `modal_window.rs:1032` `word_wrap`. The traces wrapper is the private `wrap_body` at `traces_modal.rs:1637`. |
| `now_text` | The `SessionRow` field is `now`. The column is `now_text`. |
| `SessionRow.native_id` | Not selected. It is in the schema (`index.rs:93`), but `SESSION_COLS`, `session_row` and `session_to_json/from_json` all need it. It's a uuid only for Claude and Codex. |
| remote "existing fallback (`Fetched.note`)" for `live --json` | `Fetched.note` only covers `list --json --active` (`remote.rs:372`). An unknown subcommand exits 2. `live --json` needs its own fallback. |
| "receipts in the last hour" (empty state) | No such query exists. `receipts_watermark` is `MAX(id)`, so this needs a new one. |
| `SubagentStart` → Working | It's not in `CLAUDE_EVENTS` and never arrives. |
| `Done` = last event ∈ `TERMINAL_EVENTS` | `TERMINAL_EVENTS` (`query.rs:18`) contains `Stop`, which overlaps `Prompt`. Define a live-only end set: `SessionEnd`, `session_shutdown`, `agent_end`. |
| herdr poll "same worker pattern as `spawn_refresh`" | `spawn_refresh` is one-shot and only fires when the watermark changes. Model it on `poll_remote` instead (its own rx plus a 10 s `Instant`). |

## Data section

1. **Schema 7 is a full rebuild for every user.** There is no ALTER path (`index.rs:608-664`). `context_tokens` on usage rows also needs a `PARSER_VERSION` bump. Say so, and say that the rebuild is the existing `.rebuild.lock` path.
2. **Receipts without `transcript_path` get `session_id = NULL`** (`hook.rs:56`), so `live_receipts` silently drops them. This covers Cursor (MAP.md: the path may be null) and possibly Notification/PermissionRequest payloads (U). Fall back to `native_session` → `trace_sessions.native_id`.
3. **Codex usage is self-contradictory.**
   - `total_token_usage` is cumulative (MAP.md: `usage_is_total`). The per-message figure is `info.last_token_usage`, which `usage_object` never reads.
   - `cached_input_tokens` is a subset of `input_tokens`, so adding it double-counts. For Codex, context = `last_token_usage.input_tokens`. For Claude, context = input + cache_read + cache_creation.
4. **Codex usage lives on `event_msg/token_count`, which is noise** (`classify.rs:104`). `now_text` is only set from non-noise events, so "update context in the same pass as `now_text`" never fires for Codex. Track it separately in `Aggregate::add`.
5. **Codex writes `info.model_context_window` (258400 in real data).** That's a real window figure, so the non-goal "no window table exists" is only half true. Show `ctx 142k/258k` where the harness supplies the window.
6. **"~10 ms after each tool call" was not measured.** `index_file` costs a stat, a 64 KB tail hash, and parsing the new lines, under a 250 ms busy timeout. Either drop the number or measure it.
7. **New receipt columns are needed beyond `detail`:**
   - `tool_use_id`, for parallel calls (state finding 2).
   - `agent_id`, to keep subagent receipts separate (state finding 3). Claude sends both in the hook input.
   - The insert in `receipts.rs:27-30` and `Receipt` (`receipts.rs:11-22`) both change.

## State machine: real sequences that give a wrong state

Each finding is listed as sequence → spec result → correct result → fix.

1. **Receipts can be recorded out of order.**
   - Claude hooks are installed `async: true` (`install.sh` `add_ours`, Claude passes 1). `Pre A` and `Post A` arrive ~50 ms apart and race.
   - `LAST_RECEIPT` orders by `id` (insert order), while `at_ms` is stamped at process start. If Post commits first, the screen shows `▶`, and after 30 s `⏳`, until the window ends. A Post dropped by the busy timeout does the same.
   - Fix: order by `(at_ms, id)`. Clear `Tool` when the transcript's `last_event_at` is newer than the Pre receipt.
2. **Parallel tools.** `Pre A, Pre B, Post A` shows Working while B is still running. Fix: track the set of open `tool_use_id`s. `Tool` = the set is non-empty.
3. **Subagents (verified above).** A child's `PreToolUse Read` overwrites the parent's state, and a child's Post shows the parent as Working. Fix: compute the parent's state from receipts where `agent_id IS NULL`. The parent's open tool is `Agent`. Child rows come from `agent_id` receipts, or from the child transcript MAP.md already maps.
4. **`PermissionRequest` comes after `PreToolUse` in Claude, not before.**
   - The spec's test "PermissionRequest→PreToolUse clears Waiting" encodes the wrong order.
   - After a grant, nothing fires until `PostToolUse`, so a 3-minute build shows `⚠` the whole time.
   - Fix: clear `⚠` once the transcript grows past the receipt.
5. **Denial.**
   - When the user says No, no Post or Failure fires, so `⚠` holds until `Stop`, or forever if the session is abandoned.
   - `PermissionDenied` is a real Grok and Claude event, but it isn't installed.
   - Fix: transcript-growth clearing, plus install `PermissionDenied`.
6. **"Ignored Notification, previous state holds" can't work with "last two receipts".**
   - Two ignorable Notifications in a row lose the real state.
   - Fix: take the latest *state-bearing* receipt, filtered by an event allowlist.
   - Map `elicitation_dialog`, `elicitation_url_dialog` and `agent_needs_input` to Waiting. Treat `auth_success` and `elicitation_*` responses as no-ops.
7. **`SubagentStop` after the parent's `Stop` flips Prompt to Working.**
   - It's in the Working list and carries the parent's session.
   - Fix: `SubagentStart/Stop` never set the parent's state; they only adjust a running-children count. Prompt with running children reads `○ your turn · N bg`.
8. **Long thinking with no hooks:**
   - after 2 min it shows Quiet, and after 10 min it drops off the screen mid-run.
   - A tool running longer than 10 min also vanishes.
   - Fix: age = `max(receipt at, last_event_at, transcript mtime)`. `Tool` and `Waiting` never age out while a tool is open.
9. **`Done` via `SessionRow.status == Completed`:**
   - `is_terminal` matches `complete`, `turnend` and so on.
   - Pi/OMP emit `turn_end` on every model turn, so a running OMP session is dropped after 60 s.
   - Fix: Done means an end-set receipt only, or Completed with no newer non-terminal receipt. Never treat `Stop` or `turn_end` as Done.
10. **Event names other than Claude's never match the table:**
    - OMP/Pi: `tool_result`, `turn_end`, `agent_end`, `session_shutdown`.
    - Cursor: `postToolUse`, `afterAgentResponse`, `stop`.
    - Fix: add a per-source event normalizer. Say per source which states can never appear (OMP/Pi and Grok have no Pre events, so no `Tool` state).
11. **`PreCompact`/`PostCompact` aren't installed.** A compaction shows Working, then Quiet. Install both and show `⟳ compacting`.
12. **Abandoned or crashed session stuck on `⚠` or Working until the window ends:**
    - The default 1.5 s `SessionEnd` budget can kill the async hook on exit (U).
    - Fix: demote to `◌ <state>? <age>` after 2 min with no receipt and no transcript growth.
13. **Remote clock skew:** `at_ms` is remote, `now` is local. Fix: `live --json` returns `age_ms` computed on the remote host. Clamp negative values to 0.
14. **Transcript-only sources (no receipts) are never live.** Fix: fall back to `SessionRow.status == Active`.
15. **`unparsed`/`unknown` receipts become the state.** Exclude them. Also handle `StopFailure` (Grok, Claude): treat it as Failed for that turn, and show it as Prompt with `✗`.

**Missing installer events** (Claude unless noted):
- `PermissionRequest`, `Notification`, `SubagentStart`, `PermissionDenied`, `PreCompact`, `PostCompact` and `StopFailure`.
- For Codex: `PermissionRequest`, `PreCompact` and `PostCompact`.
- The `,1` flag means "takes `matcher: *`". It is not async.

## Live screen section

- Ordering sorts on "last receipt at", which inherits finding 1. Use the same age as finding 8.
- The `c` key is fine. The tests should cover `+N` with a `⚠` child.
- `grok traces` vs the existing `Command::Trace` (`cli.rs:87`): one letter apart, which is a typo trap. Consider `grok live`, or `grok traces --live` as the only spelling. Add the new command next to `Command::Dashboard` (`cli.rs:144`).
- Herdr jump:
  - `herdr pane list` really does carry `agent_session.value` = the Claude session uuid. Checked on this pane.
  - The Codex value format is (U).
  - `herdr agent focus <target>` exists, but whether it accepts a `pane_id` target is (U). Test it before writing the spec text.
- Snapshot list: add the transitions from findings 1, 3, 4 and 7 as state tests, not only renders.

## Open questions, answered

1. **Live window.**
   - Keep 10 min for presence, but base it on the combined age (finding 8), not on receipt age alone.
   - `Quiet` replaces the 5 min `ACTIVE_WINDOW_MS` idea; don't add a fourth window. Document all three (2 min receipt-live, 5 min active, 10 min live-screen) in MAP.md.
2. **Codex `PermissionRequest`.** Per learn.chatgpt.com/docs/hooks, stdin carries:
   - `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `model`, `permission_mode` and `turn_id`;
   - `tool_name` and `tool_input` (`tool_input.command` for Bash and apply_patch, plus an optional `description`);
   - no top-level `command`.

   So the `⚠` text can carry detail. Codex's full event list: `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`, `Stop`, `Interrupt`, `SessionStart`, `SessionEnd`. `Interrupt` should map to Prompt.
3. **Grok Build permission hook.**
   - There's none in the hook enum (`xai-hooks-plugins-types/src/lib.rs:78-103`, which has `PermissionDenied` but no request).
   - A route exists: `[[ui.notifications.hooks]]` with `events = ["approval_required"]` and `only_unfocused = false` (`xai-grok-pager/src/notifications/config.rs:108-173`) can run `grok-traces hook grok`.
   - Whether its env carries a session id is (U). Check `$GROK_*` vars before relying on it. Until then, Grok shows `?` for waiting, as the spec says.
4. **`/traces` defaults to live?** I agree with Fable: no.

## Before handing to implementers

Revise the state machine so it is:
- driven by an allowlist,
- tracks open tools and open children,
- keeps subagent receipts separate by `agent_id`,
- uses the transcript as a second clock.

Then re-run a Sonnet spec review on the state table alone, with these 15 sequences as test cases. Data step 1 can't start until the receipt columns (`detail`, `tool_use_id`, `agent_id`) and the Codex usage source are settled.
