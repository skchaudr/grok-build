# Landing review: ed01975c (subagent rows) (Sonnet 5.5, 2026-11-12)

Read from code, no cargo. Paths under `crates/codegen/xai-grok-pager/src/views/`. `F` = `traces_follow.rs`, `M` = `traces_modal.rs`, `D` = `traces_modal/deck.rs`. Goal: `SPEC-subagent-rows.md` as amended by `REVIEW-subagent-rows.md`.

## Verdict

Land. I found no must-fix. Every row, key, hint and doc line in the review is built, and nothing outside it is built. The batch cycle still has to generate the five snapshots (`subagent_rows_width_{120,60}`, `subagent_unlabelled_width_60`, `subagent_no_model_width_60`, `subagent_long_name_width_60`). Insta fails those tests until they are accepted.

## Must-fix

None.

What I checked, so the batch cycle can skip it:
- Compile: `norm` is `pub(super)` and `F:27` imports it like its siblings. `Follow.note`, `want`, `cursor_agent` and `session_of` are visible to `M`. `follow_into`/`follow_parent` (`M:1073`, `M:1091`) end their borrows before `open_follow_row`. The `row_lines` signature has one caller (`F:923`). The `Agent {..}` literals (`F:196`, `F:252`) are the only two. The `Call` arm's tuple match is exhaustive. `lead_spans` byte-slices `head` only at a prefix it just checked (`F:669`), so it cannot panic.
- Link: an empty label returns `None` (`F:521`); duplicate labels give a later agent the shared call, as specified. The empty-label and unlabelled cases are tested.
- Status: a background agent's call result is ignored (`F:811-835`); `●` runs until `agent_end`, `agent_resume` flips it back. The tests assert both.
- Width: `tag_w` (`F:627`) reads only `agents` and `width`, so it cannot change as rows scroll. `t - 1` cannot underflow (`t >= 6`).
- Keys: `f` returns nothing on a root row or a pinned tail, and `links()` only maps dispatch rows. `p` returns false with no parent id, `parent not indexed` with no parent row, `spawn row not loaded` (tail stays pinned) with no spawn row. `started_at` and item `ts` are both ms.
- Tests: the claude fixture row order is [root rows, kid rows], so `k` from the tail is the kid's Bash call, as `M:7995` already assumes. `claude_fixture`'s dispatch `what` is the description, so `p` finds the Agent row. `P:971` and `P:1134-1135` were updated to `↳ 1 Check` / `↳ 1 U` / `↳ 2 o` and hold at widths 70 and 40.

## Nice-to-have

1. `F:519-536` `spawn_row`: prefer an exact label match over a prefix match. Children `Explore` and `Explore the cache` dispatched together can both link the longer call.
2. `F:244` `has_open_call`: the new clause redraws every second while any non-root agent has not ended, even when no Agent row is linked to it. A child that never emits `agent_end` keeps a live root redrawing. Narrow it to children present in `links()`.
3. Tag at width 120: the `observability lens` tag is 34 columns against a cap of 30, so that row drops the model while the `UI/UX lens` row keeps it. This follows the spec (cap, then drop the model). Look at it in the snapshot and ask Sab if he wants the model kept and the name cut instead.
4. `M:1073`, `M:1091` and `D:108-109`: `f`/`p` clear `note` and then return false, so a stale note is not redrawn until the next poll. Return true when `note` was set.
5. Missing tests: the new `has_open_call` clause, and `p` with the spawn row absent at the modal level (only `resolve_want` is tested directly).
6. `tag_w` is recomputed three or four times per row per frame (`lead`, `lead_spans`, `tag_fit`), each O(agents squared). Cache it per `render` if a session has dozens of subagents.
