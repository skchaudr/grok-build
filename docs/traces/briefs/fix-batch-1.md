# Fix batch 1: traces observability (tasks 1-2 = Opus review text, REVIEW-fix-batch-1.md 3ad79ea2)

Source: AUDIT-2026-11-12-observe.md; tasks 1-2 replaced verbatim by the Opus review (REVIEW-fix-batch-1.md), whose locations were checked against code.

## Sab's words
"seeing what a subagent received is so fucking important because it's like, if it receives a slightly incorrect brief, then it's like we don't have to fucking audit the entire agents run. We just found the issue"
"if I just click the dashboard button, I can suddenly start spawning agents and work trees, and this is supposed to be a traces and observability tool."
"there's obvious fixes for this shit"

## Tasks
1. **Root follow names each subagent.** In `xai-grok-pager/src/views/traces_follow.rs`, every row whose `agent != self.root.id` starts with a tag in place of its depth indent: `"{n} {name}"`, truncated with `truncate_str` to `T-1` columns and padded to `T`, where `T = (width / 5).clamp(6, 16)`. `n` is the agent's index in `self.agents` (root is 0, so subagents count from 1). `name` is `Agent::name(false)` minus the longest word-boundary prefix it shares with any other subagent's name; if that leaves nothing, use the whole name. Deeper rows add `"  ".repeat(depth - 1)` (capped by `MAX_INDENT`) after the tag. Root rows are unchanged. The tag uses `theme.gray_dim`; add no new colour.
- Add one helper, `fn lead(&self, i: usize, width: usize) -> String`. Use it for `ind` at `:504` (so the band, `plain()` and `body_lines(.., &ind, ..)` all get it) and for the body width at `:699-700` (replace `2 * depth` with the lead's display width).
- Names are looked up at render time, so a receipt-born child shows its id until its `model` item brings the label, then shows the label everywhere.
- Tests (written, not run): update `items_become_rows_and_results_merge_into_their_call`. Its `"  ↳ haiku-4-5 started"`, `"  ◆ Read  src/lexer.rs"` and `starts_with("  ◆ Read")` assertions become "the line with `◆ Read  src/lexer.rs` starts with `1 Check`". Add a test with subagents labelled "Audit traces: UI/UX lens" and "Audit traces: observability lens" drawn at width 40: their rows start `1 UI` and `2 ob`, and root rows have no tag.
- Verification in the batch cycle: root follow with 2+ subagents in every shipped theme (`GROK_THEME=<name>`), at full and half width.
2. **Subagent follow shows the brief it received, in full.** Two causes, both fixed:
- (a) `xai-grok-pager/src/views/traces_follow.rs`: add `Body::User(_)` to `foldable` in `fold` (`:467`). In the `Body::User` arm (`:531`), cap at `if self.open.contains(&i) { INLINE_MAX_LINES } else { USER_LINES }`.
- Add `pub fn selected_prompt(&self) -> Option<(&str, Option<i64>)>` (text, ts of the highlighted `Body::User` row) next to `selected_call` (`:371`).
- In `traces_modal.rs` `follow_inspect` (`:1042`), when there is no selected call but there is a selected prompt: `raw = Some(text)`, `raw_source = None`, `payload.open(Some(Ok(text)), Shape::Prose)`, `raw_scroll = 0`, `follow_return = true`, `mode = Mode::Inspector`, return true. Esc already returns to Follow (`:4137`).
- In `render_follow_payload` (`:5347-5364`), when the selection is a prompt, the head title is `prompt` and its time is the row's ts.
- (b) `xai-grok-traces/src/follow.rs`: the brief is never lost to the backlog. `resume_after` returns `(i64, Option<EventRow>)`, where the row is the first non-noise `message`/`user` row, when it is at or before the cut. Store it in a new `Followed.brief: Option<EventRow>`:
  - `adopt` (`:440`) sets it only when `depth > 0 || s.parent_id.is_some()`, so root sessions keep today's backlog.
  - `:706` and `:738` (children) always set it.
  - `agent_at` (`:464`) sets `None`.
  - In `tick`, after the announce/meta block and before the events loop (`:754`), `brief.take()` is emitted through the same path as loop rows: `raw_line`, then `json_event` (with `session = rid` when `rid != id`) or `format_event`.
- Applies to every harness (it keys on the first user row, not line 0) and to the `grok-traces follow` CLI.
- Tests (written, not run): in `traces_follow.rs`, `l` on a 10-line user row shows all 10 lines and `h` folds it back; after `g`, `selected_prompt()` returns its text. `the_inspect_target_is_the_highlighted_call_only` stays as is. In `traces_modal.rs`, set a `Follow::bare` with a user item as `follow_view`, step onto the prompt, then `follow_inspect()`: mode is Inspector, the payload holds the full text, and Esc returns to Follow (pattern: tests near `:7920-7955`). In `xai-grok-traces/tests/follow.rs`, follow the `CHILD` session directly with `with_backlog(1)`: kinds start `["model", "user", …]` and the user text is "look at the lexer". The existing root tests (`follow_backlog_limits_the_first_rows`, `…_zero_…`, the `with_backlog(2)` ended-session test) must still pass unchanged, which is why the gate is `parent_id`/depth.
3. **Dropped (Sab, 2026-11-12).** Blocking the dashboard and Ctrl+N in trace views contradicts `SPEC-traces-agent.md` rev 5: Ask uses the trace view's hidden prompt, keeps the Ask session as a dashboard row, and uses Ctrl+N. The dashboard is also the only route to settings today. Trace-view input and dashboard behaviour belong to the Ask work.

## Build policy, standing for the whole cleanup sweep (Sab, 2026-11-12)
"we cannot have all three agents running tests and running builds and waiting on each other... we just have to do it all at once. They're gonna have to review their work from code first."
Builders run no cargo build/test/check. They review their own change from code, add or update tests without running them, and commit. One test+build cycle runs once for the whole batch, after all slices land.
