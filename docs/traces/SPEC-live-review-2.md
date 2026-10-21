# Review 2: SPEC-live.md rev 2 (e510085a), state table

Reviewer: Opus lead, 2026-10-21. One Sonnet pass replayed real sequences from a copy of `~/.grok/traces/traces.sqlite`: 2,442 receipts, nearly all Claude. Codex, OMP and Cursor findings rest on code and docs, since that copy has no receipts from them.

**Verdict:** rev 2 fixes the structure. Findings 1, 6, 7, 11, 14 and 15 from review 1 are resolved. But real data breaks six rules, so the spec isn't ready for an implementer yet. All six fixes are narrow.

## High: real data contradicts the table

**A. Orphan `PreToolUse`**
- 19 Pre receipts have no matching Post. One session has 12, and in session `claude:73a6…` (receipts 2244 and 2255) later pairs complete normally.
- Under the table, rule 5 beats rule 6, so the row shows `▶ Bash ⏳` after the turn has ended.
- Fix: `Stop`, `UserPromptSubmit`, `End` and `SessionStart` close every open tool on the parent. Display the newest open tool, not the oldest.

**B. `Agent` tools never stay open**
- All 37 real `Pre Agent→Post Agent` pairs take 55–117 ms, because subagents run in the background.
- Rule 5's "no ⏳ while children run" and test 3 assume the opposite.
- Fix: `open_children > 0` holds the parent in `● N bg` (or `○ your turn · N bg` after `Stop`). It suppresses Quiet and off-screen while children run.

**C. Where `SubagentStart`/`SubagentStop` land**
- If these receipts carry the child's `agent_id`, the parent fold (`agent_id IS NULL`) never sees them, so `open_children` is always 0.
- Also, `start − stop` computed over a 10 min window miscounts once a start ages out.
- Fix: count children with `ChildStart/ChildStop` regardless of `agent_id`, keyed by `agent_id`: open = started and not stopped. Add it to Step 0 (U): record which `agent_id` these events carry.

**D. The window drops what the stale rules need**
- `live_receipts` returns only the last 10 min. Tests 8b and 12, and rule 9 ("10 min with no event"), depend on receipts older than that, so they can never fire.
- Fix: also fetch each session's still-open `Pre`/`Ask`/`CompactStart`, whatever their age. Set the stale threshold below the window, e.g. 2 min as review 1 said.

**E. `⚠` holds for the whole run of a granted tool**
- Order: `Pre`, `Ask`, grant, silent run, `Post`. The transcript next grows at the tool_result, which is the same moment as `Post`. Growth clearing therefore buys nothing, and a 3 min build shows `⚠` for 3 min.
- Fix:
  - Accept this as a known gap: show `⚠ / ▶?` after 30 s with no Post.
  - Or find a grant signal. Put it in Step 0 (U): does Claude emit a Notification or progress line on grant?
  - Test 4's expected state must match whichever is chosen.

**F. Codex counts as Done after every turn**
- `task_complete` sets `status=completed` each turn. In the copy, 74 root Codex sessions are `completed` versus 16 `open`.
- Rule 2 ("Completed with no newer Ev") compares against a status that has no timestamp, so the session drops off screen after 60 s while it's waiting for the user.
- Fix: Done = `End` event only. `Completed` counts only for sources with no receipts.

## Medium

- **G. `SessionStart` maps to Working.** Real gaps from start to first prompt run from 2 min to 5.4 h. Map `Start` to idle/Prompt for claude/codex/grok.
- **H. OMP/Pi `turn_end` maps to Idle.** It fires on every model turn, so the row flickers to `○ your turn` mid-run. Only `agent_end` should be Idle.
- **I. `Failed` sits at rule 1.** `failed` means only that the last tool errored with nothing after it, and 22 of 108 live Claude sessions have it. That makes `✗` beat Waiting, Tool and Prompt, and then auto-drop the row. Lower its precedence to a mark on the row, not a state, and never drop on it.
- **J. A spurious `Stop` mid-turn.** 36 of 68 real `Stop`s are followed by tool receipts with no prompt in between, with gaps up to 46 s. Show `○ your turn` only after 2 s with no newer receipt.
- **K. A Grok/Codex `Ask` with no `Pre`.** Once growth clears it, no rule matches. Add `Ask` to rule 7's list, and add an explicit final fall-through.
- **L. `bb thread.failed`** is mapped both to `End(✗)` and to rule 6's glyph. Pick one.

## Ambiguities an implementer would have to guess

1. Pairing when `tool_use_id` is NULL (old rows, sources that don't send it). Pick one rule: FIFO by tool name.
2. `grew_since(t)` has no defined source. `last_event_at` is non-noise time on the source's clock, `SessionRow` has no mtime, and rule 5 needs the type of the last event, which the query doesn't return. Add `last_event_kind` and file `mtime_ms` to the query.
3. The "60 s after Done" anchor: there is no `ended_at`. Use the `End` receipt's `at`.
4. Rule 5's wording is garbled ("not grew_since(…) past that tool's result"). Rewrite it.
5. Whether the 2 min, 10 min and 30 s boundaries are `<` or `≤`.
6. Rule 11's "no receipt" check is per source; it should be per session.
7. `<age>` in the Prompt text: time since `last_ev`, or the combined age?
8. Waiting text when `Ask` is a Notification: the detail is the notification type, so the tool is empty. Define it as `⚠ <notification type>`.
9. Tie-break for rows with equal state and age: use `session_id`.
10. Remote fold: `LiveReceipt` has only `age_ms`, but the fold also needs `last_event_at` and the file clock. Either fold on the remote (the `live --json` remote returns `LiveState`), or send every clock as an age. Folding remotely is simpler.

## The 16 tests

- **Fully determined:** 1 (if `tool_use_id` is present), 2, 6, 9, 10, 11, 14, 15, 16.
- **Not determined:**
  - 3 (premise false, see B)
  - 4 (see E)
  - 5 (the `PermissionDenied` id is (U), and the state it clears to is unstated)
  - 7 (expects `0 bg` text that the rule hides; depends on C)
  - 8 (Working or Quiet)
  - 8b and 12 (unreachable, see D)
  - 13 (expects an age, not a state)
- **Missing:** orphan Pre then `Stop` (A), background Agent (B), idle `SessionStart` (G), Codex `task_complete` then `Stop` (F), mid-turn `Stop` (J), `failed` with a later prompt (I).

## Contradictions

- Delivery step 1 builds `norm`, `live.rs` and the tests before Step 0 confirms `agent_id`/`tool_use_id`. Make step 0 a gate.
- Render snapshot 2 says "one row per state 1–9 and 11", but 9 is a mark and 10 is not a row. Restate it after fixing I.
- `Fetched.note` covers only `list --json --active` (`remote.rs:372`). `live --json` still needs its own fallback, a point carried over from review 1.

## Next

Fable folds A–L and the ambiguities into rev 3. Step 0 is a gate before any code. It checks:
- the payload fields: `agent_id` on Subagent events, `tool_use_id` on `PermissionDenied`;
- whether a grant signal exists;
- the Grok env vars.

That check needs real hook payloads captured on this machine. Then one more Sonnet pass on the tests alone; after that it's ready for a compacted Opus with Sonnet 5.5 subagents.
