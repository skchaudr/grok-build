# Execution list: usage limits and running-call visibility (2026-11-16)

Sources: Sab's messages today (13:20 specimen; 14:22 "does Fable usage not show up like 5h and 7d"; 14:24 "I 100% have a Fable usage limit"; 14:37 "it 100% is reported, we just didn't capture"), `docs/INVESTIGATE-running-tool-visibility-2026-11-16.md`, `docs/SCOUT-claude-usage-freshness-2026-11-15.md`, `docs/QUEUE-next-install.md`, a live `get_usage` call made 14:24. Sizes say what a task touches, never how long.

Status marks: [ ] not started · [~] in flight · [x] done.

## Decisions made without Sab (reverse any of them)

- **D1. The pager gets usage from a helper script in this repo, not from a patch to the adapter.** `scripts/claude-usage.mjs` opens the Agent SDK `query()` from `~/.local/share/claude-agent-acp/current/node_modules` with an idle prompt (no turn), calls `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` and writes the whole `rate_limits` object to `~/.grok/cache/claude-usage.json`. Reason: the adapter lives outside the repo and a patch has no stated home (the `~/.dsh` question in the queue); a repo script ships with `install-fork.sh` and survives adapter upgrades through the `current` link. The call was measured at 1.8 s with no model turn.
- **D2. The file is the contract.** The pager reads `claude-usage.json` the way it reads the statusline files today; no new ACP message, no adapter change. Footer and side panel share one reader.
- **D3. "not reported" is retired.** The value exists on the account. The panel says `loading…` until the first read and `unavailable: <reason>` only when the helper failed, with the reason.
- **D4. A scoped weekly limit (Fable) gets its own row, named from `scope.model.display_name`.** Whatever scoped entries `limits[]` returns are shown, active one first; none are hard-coded to Fable.

## A. Usage limits: capture, store, show

### A1. Helper script (new file `scripts/claude-usage.mjs`)
- [ ] A1.1 Port the 14:24 probe (scratchpad `u.mjs`) into the script: `query()` with an idle prompt stream, one `usage_...()` call, `process.exit`.
- [ ] A1.2 Output: write `{fetched_at, rate_limits}` where `rate_limits` is the SDK's object unchanged (`five_hour`, `seven_day`, `limits[]`, `extra_usage`). Write to a temp file then rename, so a reader never sees half a file.
- [ ] A1.3 Failure path: on any error write `{fetched_at, error: "<one line>"}`; never leave the previous good value looking fresh.
- [ ] A1.4 Do not log or write any credential, token or account id. Check the output file for them before committing a sample.
- [ ] A1.5 Resolve the SDK path through `~/.local/share/claude-agent-acp/current`; if it is missing, write the error above.
- [ ] A1.6 A self-check: run it twice, confirm the second overwrites the first and both parse.

### A2. Who runs it and when
- [ ] A2.1 The side panel's 1 s poll (`views/side_panel/mod.rs` `poll`) starts the helper when the file is missing or `fetched_at` is older than the refresh interval; never two at once; never on the render path (a background thread, as `look_up` does).
- [ ] A2.2 Refresh interval: pick one constant, state it in code, no config. Also refresh right after a turn ends (the percent moves most then).
- [ ] A2.3 Only for Claude-host sessions. Grok, Pi and Codex sessions do not spawn it.
- [ ] A2.4 A second grok claude session sees the first one's fresh file and does not spawn.
- [ ] A2.5 Kill by PID only when the panel closes mid-run (no `pkill`).

### A3. Pager reader (replaces `claude_five_hour_pct`, `views/side_panel/mod.rs:594`)
- [ ] A3.1 Parse `claude-usage.json` into `Info.rate_limits` (existing `RateWindow {pct, resets_at}`); convert `utilization` 0-100 and ISO `resets_at` to the pager's percent and epoch seconds.
- [ ] A3.2 Add scoped weekly windows from `limits[]` (`kind: weekly_scoped`): key `weekly:<display_name>`; keep `severity` and `is_active`.
- [ ] A3.3 Keep the live adapter event (`parse_rate_limit`, `app/acp_handler/mod.rs:261`) as the fresher source when it arrives; the file fills in before it and for windows it never sends (7d, scoped).
- [ ] A3.4 Keep the statusline-file read as a last fallback, now with `resets_at` for both windows (the file has it).
- [ ] A3.5 Drop a window past its reset (existing rule, `session.rs:90`) but show `loading…`, not `not reported`.
- [ ] A3.6 Show data age when over a few minutes old (`as of 4m ago`), so a stale figure never reads as live.

### A4. Side panel Session page (`views/side_panel/session.rs:85-107`)
- [ ] A4.1 Rows: `5h usage`, `7d usage`, then one row per scoped weekly limit (`Fable usage`), each with the meter, percent and `resets 2h32m`.
- [ ] A4.2 Reset time on every row, including the file-sourced ones (today only the live path has it).
- [ ] A4.3 Colour by the API's `severity` (`normal`, `warning`) in every theme; the warning must be distinct from normal.
- [ ] A4.4 Narrow panel (43 and 21 columns): the reset text truncates before the percent does.
- [ ] A4.5 Replace the `(None, None)` arm: `loading…` / `unavailable: <reason>`.
- [ ] A4.6 Snapshot tests: both windows live, file-only, scoped present, scoped absent, stale file, helper error, narrow width.

### A5. Footer status slot (queue item "Footer shows status", slice 3)
- [ ] A5.1 Show `5h 46% · 7d 69% · Fable 75%` in the status slot from the same reader; order and abbreviations settled once, used in both places.
- [ ] A5.2 Slices 1, 2 and 4 of that queue item (hint trim, status slot, cache age) stay as written in `docs/QUEUE-next-install.md`; this task only supplies slice 3's data.
- [ ] A5.3 Half-width render test; every theme looked at on screen.

### A6. Install and ship
- [ ] A6.1 `scripts/install-fork.sh` copies `claude-usage.mjs` to `~/.grok/bin/` on both Macs with the checksum check it already does for the binaries.
- [ ] A6.2 Remove the queue items this closes: "7d usage has no fallback" and "Fresh Claude usage via adapter (RESCOPE)".
- [ ] A6.3 Update `docs/HARNESS-FACTS.md` with the `limits[]` shape and the helper (the `limits[]` fact is already recorded).

## B. Running and background tool calls (from the investigation, smallest first)

### B1. Elapsed time on the running Bash card
- [ ] B1.1 Draw `elapsed_ms()` in the Execute card header while running (`scrollback/blocks/tool/execute.rs`; it is never read there today).
- [ ] B1.2 Same for the other tool cards that have `started_at` (Read, Edit, Search, WebFetch) only if one shared helper covers them; otherwise Bash only.
- [ ] B1.3 Snapshots: running, finished, collapsed, expanded.

### B2. Keep `backgroundTaskId` on the receipt
- [ ] B2.1 `xai-grok-traces/src/hook.rs` `extract`/`response_text`: when `tool_response.backgroundTaskId` is set, store `moved to background (ID <id>)` (and `timedOutAfterMs` when present) as `result`.
- [ ] B2.2 Test with the two captured payloads (`~/.grok/traces/capture/claude/20261116T134320-26867-PostToolUse.txt`, `...20261021T234637-5143-PostToolUse.txt`) reduced to fields only.

### B3. A call with a Pre and no Post gets closed
- [ ] B3.1 Specimen: calls at 13:24:47 and 13:28:42 have no `PostToolUse` (receipts by `tool_use_id`).
- [ ] B3.2 Close an open call when the transcript's result line for it arrives, or the next `UserPromptSubmit`/`Stop` receipt for the session lands (`follow.rs` Post arm, `live.rs` fold).
- [ ] B3.3 Tests: Pre only then Stop; Pre then transcript result; Pre then new prompt.

### B4. A backgrounded call stays open until its task completes
- [ ] B4.1 Depends on B2. A `Post` whose result carries the background marker leaves the call open (`follow.rs`, `live.rs:447`).
- [ ] B4.2 Close it on the transcript `queue-operation` line carrying `<task-notification>` with the same `<tool-use-id>`; `normalize.rs`/`classify.rs` stop treating that line as noise.
- [ ] B4.3 Steps shows `running` with a live timer from the original start until then.
- [ ] B4.4 Test with a transcript excerpt (ids and timestamps only).

### B5. Show the call's `description` on the Steps row
- [ ] B5.1 `follow.rs` adds `description` to the `call` item; `traces_follow.rs` `Call` carries it; `steps.rs` shows it before the command's first line.
- [ ] B5.2 This is the only place a Rex tab's label can appear; no Rex query is added.

### B6. Output-file path in the call body
- [ ] B6.1 Depends on B4. Show the `.output` path (named in the result text) in the opened body of a backgrounded call (`traces_follow.rs`, optionally a column in `steps.rs`).

### B7. Claude background commands in the Agents page and dock (largest)
- [ ] B7.1 Pager advertises the JetBrains AIR `asyncTasks` capability in its ACP initialize `_meta`.
- [ ] B7.2 Decode `async_task_spawned`, `async_task_progress`, `async_task_state_update` as ext updates (not in the `acp` crate's `SessionUpdate`; model on `headless/ext_protocol.rs`).
- [ ] B7.3 Fill `session.bg_tasks` (`app/agent.rs:764`) so `side_panel_jobs` (`app/agent_view/side_panel.rs:99`), the dock and the status row count it.
- [ ] B7.4 Kill path: `canStop` → `Effect::KillBgTask` (`app/actions.rs:1901`) must reach the adapter's stop; check what grok's kill assumes.
- [ ] B7.5 Resolve the open question first: does advertising `asyncTasks` change anything else the adapter sends (the 11-16 scout read the adapter at `async-tasks.js` and `acp-agent.js:1880`, not run)?

## C. Open diagnoses (answers decide whether B is complete)

- [~] C1. What does a live `grok claude` screen draw during a minutes-long call? Two watchers are on it (section D). Outcome decides whether the card, status row and Steps row all appear as the code says.
- [ ] C2. Why did the follow view stay on `(waiting for the transcript)` when the `tool_use` line was already on disk? Trace `follow.rs:786-830` against the specimen's index rows; reproduce with the traces CLI on the specimen session.
- [ ] C3. Why did `grok-traces follow --once` hang for 120 s on the live session? Read the `--once` path in `bin/grok-traces.rs` and `follow.rs`; reproduce with a short timeout.
- [ ] C4. What does the dock show for a running call? Read `views/dock`; confirm on a watcher snapshot.
- [ ] C5. Do `tool_progress` heartbeats (`elapsedTimeSeconds`) arrive for a foreground Bash? If yes, B1 can use them instead of a local clock.

## D. Watchers in flight (started 14:29)

- [~] D1w. watch-a (block `17e421`) and watch-b (block `b8db76`): Haiku 5.5, minimized floats, brief `docs/briefs/watch-grok-claude-running-calls-2026-11-16.md`.
- [ ] D2w. 14:50: first files `docs/WATCH-running-calls-A-2026-11-16.md` and `...-B-...` due; read them in full.
- [ ] D3w. 15:30: final files due; fold the findings into C1, C4, C5 and correct the investigation doc where the screen disagrees with the code.
- [ ] D4w. Stop both snapshotters by PID (`/private/tmp/claude-501/watch/{watch-a,watch-b}/pid`), close both floats, report the size of `/private/tmp/claude-501/watch/` and delete it on Sab's say.
- [ ] D5w. Check that no conversation text reached a committed file (grep the two WATCH files for anything beyond interface chrome).

## E. Process for building A and B

- [ ] E1. One brief per task group (A1-A3, A4-A5, B1, B2-B4, B5-B6, B7), each reviewed before dispatch (spec review: every state defined, snapshot tests named).
- [ ] E2. Builders run no cargo; they edit and commit with the `Co-authored-by` trailer. One `--release` test run for the batch, then one `scripts/install-fork.sh` cycle on the warm `target` (say fresh-vs-warm and builds-at-once before it starts).
- [ ] E3. Every colour or layout change is looked at on screen in every shipped theme (`GROK_THEME=<name>`, AGENTS.md rule) and at half width.
- [ ] E4. Install on the Mini, then sab-air (primary); look at the sab-air panel with a real Claude session.
- [ ] E5. Do not touch the three known footer-snapshot failures (terminal-dependent); note them again in the report, do not mask them.
- [ ] E6. Housekeeping: add `scripts/claude-usage.mjs` output path to `.gitignore` if any sample file lands in the repo; at the commit checkpoint ask what to do with the unrelated dirty `.pi/` files.

## Order

1. A1 → A2 → A3 → A4 (usage values captured and shown; this is the live complaint).
2. A5 and A6 with the footer queue item's single build cycle.
3. B1, B2, B3, B5 (independent), then B4, B6.
4. C1-C5 as the watchers and traces reproductions report; B7 last, after C5 and B7.5.
