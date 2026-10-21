# Review 5: SPEC-live.md rev 5 (bc712fde), edits only

Reviewer: Opus lead, 2026-10-21, from one Sonnet pass over the diff from 724d506a.

**Verdict:** B1–B3 and B6 are fixed; B4 and B5 are mostly fixed. Tests 7, 17, 32 and 40–52 all match when executed literally. What remains is mechanical or a decision I'm making here. Once rev 5.1 folds the list below, the spec is **ready for the implementer**. I'll do a grep check only, with no further review round.

## Decisions (lead)

1. **Remote rows are re-rendered on the mini.** The Air sends `state`, `since_ms`, `age_ms` and the raw marks inputs. The mini recomputes the glyph, `⏳`/`◌` and the `?`/`⚠?` marks from the clock-advanced numbers, using the same functions as local rows. One render path; cached rows age correctly. Add a test: a cached `Tool` row at `since_ms=25s` with a fetch failure becomes `⏳` after 6 s.
2. **A lost child stop shows a `?` mark; it never expires.**
   - A child stays in `open_children` until its `ChildStop` or the parent's `End`.
   - If that child's `agent_id` has had no receipt for 10 min, its count shows `· N bg?`.
   - This matches the `?` mark everywhere else: display-only, nothing removed on a timer.
   - Add a test.
3. **`LiveRow.stale`:** drop it. The `✗` header plus the recomputed `?` cover the case.
4. **Step 3's binary deploy to the Air doesn't need a separate OK.** It's in scope, and Sab's rule is that in-scope work is a go. Keep the backup, `uname -m` check, atomic `mv` and rollback steps.

## Mechanical fixes

1. **Step 0b commands:**
   - Zsh aborts on brace globs with a missing alternative (rows 1, 2, 4). Use `rg -g '*-<Event>.txt'` or `setopt nonomatch`.
   - `rg -c` lists only matching files (rows 1, 4, 9b). Use `rg -L` / `--files-without-match` for "every file has X".
   - Row 2 needs `rg -I` before `uniq -c`.
   - Rows 12 and 16: `head -c 400 | jq` cuts the JSON. Use `sed '/^$/,$d' "$f" | jq -c keys`.
   - Row 17 (Air): prefix `cd ~/.grok/traces &&`. Remove the markdown `\|` from the commands.
   - Rows 3, 5 and 11 are prose. Give each an exact command.
   - Row 8: use `herdr agent list`, which gives `pane_id` directly.
2. **Cursor tool-name table:** add `afterShellExecution` → `Shell` and `afterMCPExecution` → `MCP`. Test 34 is undetermined without them.
3. **Test 18 flips with capture 5**, the same as test 49. Mark it.
4. **Test 38:** say that `live_sessions` counts receipts matched by `native_id`, and that receipt-only grouping excludes them.
5. **m1:** where ids exist (Pi `toolCallId` today, Claude/Cursor if capture 1/5 says yes), pair strictly by id. Close-older only applies to calls without ids.

## Capture status (mini)

- **Claude:** 68 files across 7 event types. Captures started with new sessions.
- **Cursor:** 4 files.
- **Pi:** none yet. Pi sessions need a restart to load `traces-capture.ts` (U).
- **Air:** wired, not inspected.
