# Review 4: SPEC-live.md rev 4 + 4.1 (724d506a), changed rows

Reviewer: Opus lead, 2026-10-21. This review comes from one Sonnet pass. It executed the changed rows literally, replayed real receipts (`scratchpad/replay4.py`, which is ephemeral), and read the code it needed. Paths are under `crates/codegen/xai-grok-traces/`.

**Verdict:** not ready yet, but close. Every blocker below is a small spec edit, and most of the 45 rows now match. The orphan-Pre rule fixes the phantom `▶ Bash ⏳` seen in real data.

## Blockers

**B1. `turn_receipts` drops a `ChildStart` that comes before the boundary.**
- Example: `Prompt@0, ChildStart x@1, Idle@5` shows `your turn` with no `· 1 bg`. This contradicts "children run across `Stop`".
- Test 7 is vacuous: its `ChildStop` has no start in the window.
- Fix: select child events from the whole session, or carry `open_children` across boundaries.
- Add the row `Prompt@0, ChildStart x@1, Idle@5; now=9` → `○ your turn · 1 bg`.

**B2. Ghost rows, and `ended` is never cleared.**
- Real index: 6 `SessionStart` vs 2 `SessionEnd`. An async `SessionEnd` gets lost.
- Claude `/clear` and `/resume`, and Pi `session_shutdown(new|resume|fork)`, switch sessions inside one live PID, so the old id stays "alive" forever.
- Fix: a newer session with the same `(source, harness_pid, harness_start)` supersedes the older one. This doesn't apply to Cursor, where one process hosts many chats.
- `ended` is never reset. Claude `--resume`, Pi `reload` and a reopened Cursor chat all reuse their id and stay hidden. Define `ended` as "the newest boundary is `End`".
- Add the test `End@0, Prompt@1` → on screen.

**B3. The shell-skip walk fails on the real chain.**
- In a Claude hook, `comm` runs `sh`, `/bin/zsh`, `claude`, `-zsh`. Matching the literal names `sh|bash|zsh` stops at `/bin/zsh`, the short-lived hook shell, so the row drops when that shell exits.
- Fix: compare the basename, strip a leading `-`, and use an explicit set: `sh bash zsh dash fish`.
- What resolves per harness:
  - Claude resolves to `claude` at the third hop.
  - Pi needs no walk, since it passes `process.pid`.
  - Cursor has no real chain yet. The files in `capture/cursor/` are smoke tests fired from Claude. One Cursor process hosts many chats, so `sessionEnd` (capture 15) is the only reliable end signal for Cursor.

**B4. Remote gaps (rev 4.1).**
- **No version field.** `LiveRow` is a bare array, and the Cargo version string doesn't change between builds. Fix: send an envelope `{live_schema, rows}` with `serde(default)`, and gate on `live_schema`.
- **Stale times.** Rows carry text the Air already rendered (`▶ B · 2s`) and an `age_ms` frozen at fetch, so they lag up to 10 s and cached rows never advance. Fix: the mini adds the time since its own `fetched_at` to `age_ms` and to any duration in the text. Better still, have the Air send durations as numbers and render on the mini.
- **Wrong prefix.** The host label is `sab-air` (the part after `@`), so the prefix is `sab-air:`, not `air:`. Fix test 40 and the samples.
- **Contradiction.** A failed fetch is `◌` in one place and `✗` in test 43. Pick one.
- **Circular gate.** Capture 18 needs the step-3 binary but sits in gate 0. Move it out.
- **Step 3 deploy.** It needs:
  - a backup of the Air's current binary;
  - a `uname -m` check;
  - an atomic rsync (no `--inplace`);
  - a note that rollback also needs the old index, because an old binary won't open schema 7.

**B5. Step 0 isn't executable yet.**
- **Commands.** Add a command column. Only captures 8 and 18 give a command.
- **Yes/no closings.** These still lack one: 2, 9, 10, 12, 14 and 17.
- **Wiring.** Nothing for Claude or Pi is wired yet. Only Cursor exists in `capture/`.
- **Per-capture fixes:**
  - Capture 9 is already answered from Pi source: `toolCallId` is present on `tool_execution_start` and `tool_execution_end`, plus an optional `parentToolCallId`. Mark it closed.
  - Capture 12 should list `workspace_roots`, `cwd` and the shell command field. `beforeShellExecution` has no `tool_name`.
  - Capture 17 should include 11.
  - Capture 8 hardcodes `w36:p5`. Use "the pane whose `agent_session.value` matches".

**B6. Cursor keying.**
- `links.rs` `path_links` has no `cursor` arm, so all 300 Cursor rows have `native_id = NULL` today.
- Fix: add the arm, with `native_id` = file stem (`agent-transcripts/<id>/<id>.jsonl`, layout confirmed). Whether it equals `conversation_id` depends on capture 13.
- Receipt extraction tries `session_id` before `conversation_id`. Pin Cursor to `conversation_id`.
- Add an event→tool-name table for Cursor's `before*` events: shell → `Shell`, MCP → `MCP`, read → `Read`. Tests 34 and 37 depend on it, and 37 is undetermined without it.

## Rows

- **Match:** 1, 2, 12–14, 17–36, 38–45.
- **Caveats:**
  - 7 is vacuous (B1).
  - 32 needs `now` stated.
  - 33 is conditional on capture 5.
  - 37 is undetermined (B6).
  - 40 uses the wrong prefix (B4).
  - 43 contradicts itself (B4).

## Minor

- **m1.** The same-name and close-older rules close real parallel tools: `Pre A, Pre B, Post B` drops A. The replay shows 37 such closes; they mostly cost the `+N` count. Add a test, and switch to `tool_use_id`/`toolCallId` pairing wherever it exists (Pi already has it).
- **m2.** Cursor `beforeReadFile` and `preToolUse` for the same call fire in an unspecified order, which can leave an orphan. Cover this in capture 12.
- **m3.** Claude `SessionStart` with source `compact` becomes an `Idle` boundary mid-turn. Ignore that source (U).
- **m4.** A Cursor `afterAgentResponse` that lands after `stop` leaves the row Working. Order by generation, or debounce.
- **m5.** Receipt-only Cursor rows (NULL `session_id`) are never evicted.
- **m6.** Add a Decisions-log entry for the Air amendment, and say where tests 40–45 live.
- **m7.** In test 17, write `mtime_ms` as `1s`.
- **m8.** The captures 17–18 table is split from the main table by a blank line.

## Next

Fable folds B1–B6 into rev 5. I check only those edits; no full re-review. Then gate 0 runs. Wiring `capture.sh` into Claude and Pi on the mini is next, and the Air needs Sab's OK first.
