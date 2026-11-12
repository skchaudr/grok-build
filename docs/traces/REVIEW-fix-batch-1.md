# Review: briefs/fix-batch-1.md (Opus, 2026-11-12)

Checked against main at 638d5719. The installed grok-fork is 4c3ace53. `traces_follow.rs`, `follow.rs` and `deck.rs` have not changed since then, so what the installed binary shows is what this code does. Paths below are under `crates/codegen/`.

## Departures from Sab's words (for Sab to rule on)

1. Task 2's premise is wrong. "the band where the brief belongs is empty" does not reproduce. Sab was told this (his message quotes "Subagent briefs show empty") and he reacted to it, so he needs the correction: the brief shows, but only its first 3 lines, and nothing opens the rest. It is dropped entirely only when a long subagent is followed live. Details under Task 2.
2. "in full" (task 2) is the lead's wording. Sab said "seeing what a subagent received". It is consistent with his point about a "slightly incorrect brief", so keep it.
3. Task 1's tag look (below) is a UI decision made by this review. Sab should see it on the real screen before it counts as done.

## Task 1: root follow names each subagent. Verdict: needs change

- The cause is real. Locations corrected: `visible()` is at `xai-grok-pager/src/views/traces_follow.rs:357-360`, the depth indent at `:504`, the only agent marker `Body::Start` at `:644`. `Agent::name` at `:94` already gives label, else short model, else id.
- What the spec misses: in the real UI (board present, `traces_modal.rs:456`), Tab is rebound to "next live session" (`traces_modal/deck.rs:108-109`). `Follow::cycle_agent`/`filter` is unreachable, so `filter` is always 0 and kid rows are always merged. Seen live: my own rows sat in root follow with a 4-column indent and no name. The audit's alternative ("default filter root-only") would hide subagents entirely. Do not use it.
- Labels collide on their prefix. The audited run's subagents are "Audit traces: UI/UX lens", "… observability lens" and "… buildability lens", so truncating the label alone gives identical tags. Model-name fallbacks (unlabelled children) collide outright. The tag therefore needs an ordinal plus the part of the label that differs.
- Missing from the spec: the tag format, the body-width coupling at `:699-700`, the tests that break, and the theme check.

Replace task 1 with:

> 1. **Root follow names each subagent.** In `xai-grok-pager/src/views/traces_follow.rs`, every row whose `agent != self.root.id` starts with a tag in place of its depth indent: `"{n} {name}"`, truncated with `truncate_str` to `T-1` columns and padded to `T`, where `T = (width / 5).clamp(6, 16)`. `n` is the agent's index in `self.agents` (root is 0, so subagents count from 1). `name` is `Agent::name(false)` minus the longest word-boundary prefix it shares with any other subagent's name; if that leaves nothing, use the whole name. Deeper rows add `"  ".repeat(depth - 1)` (capped by `MAX_INDENT`) after the tag. Root rows are unchanged. The tag uses `theme.gray_dim`; add no new colour.
> - Add one helper, `fn lead(&self, i: usize, width: usize) -> String`. Use it for `ind` at `:504` (so the band, `plain()` and `body_lines(.., &ind, ..)` all get it) and for the body width at `:699-700` (replace `2 * depth` with the lead's display width).
> - Names are looked up at render time, so a receipt-born child shows its id until its `model` item brings the label, then shows the label everywhere.
> - Tests (written, not run): update `items_become_rows_and_results_merge_into_their_call`. Its `"  ↳ haiku-4-5 started"`, `"  ◆ Read  src/lexer.rs"` and `starts_with("  ◆ Read")` assertions become "the line with `◆ Read  src/lexer.rs` starts with `1 Check`". Add a test with subagents labelled "Audit traces: UI/UX lens" and "Audit traces: observability lens" drawn at width 40: their rows start `1 UI` and `2 ob`, and root rows have no tag.
> - Verification in the batch cycle: root follow with 2+ subagents in every shipped theme (`GROK_THEME=<name>`), at full and half width.

## Task 2: subagent follow shows the brief, in full. Verdict: wrong as written; rewrite

Trace:

- The brief is indexed. In the audited subagent (`claude:bb5500…`, file `…/subagents/agent-af17612901abf271f.jsonl`), line 0 is `message`/`user` with noise=false. Its raw content is a 2,890-char string.
- `json_event` (`xai-grok-traces/src/follow.rs:275-278`) emits it as a `user` item with full text (`full_text`→`raw_text`, `:98-133`). `Follow::apply` turns it into `Body::User` (`traces_follow.rs:255`).
- On the real screen (tmux, installed binary, `f` on that same subagent, then `g`), the band shows `› You are one of three auditors …`, 3 lines, then `… +42 lines`, then `◆ Read rex/SKILL.md`. That is the audit's layout, but with text in it. At 40 columns it shows 3 lines and `… +150 lines`. Following my own live subagent showed the same.

Root causes of "can't see what it received":

1. **Clipped, with no way to open it.** `USER_LINES = 3` (`traces_follow.rs:35`). `fold` excludes `Body::User` (`:467`), so `l` does nothing on the brief. Enter (`deck.rs:96-102` → `traces_modal.rs:1042-1044`) acts only on calls.
2. **Dropped when the backlog cuts it.** On a follow's first tick, a running session is read from `resume_after(…, Some(backlog))` (`follow.rs:429-438`). The callers are at `:689` (the followed session), `:706` (a receipt-born child) and `:738`/`:743` (children found by file). The TUI backlog is 200 non-noise rows (`traces_follow.rs:30`); the CLI default is 10 (`follow.rs:22`). A subagent with more than 200 rows, followed live or seen in a root follow opened mid-run, has no brief at all. The CLI shows it: `grok-traces follow <my subagent> --json --once` printed 0 `user` items.

Replace task 2 with:

> 2. **Subagent follow shows the brief it received, in full.** Two causes, both fixed:
> - (a) `xai-grok-pager/src/views/traces_follow.rs`: add `Body::User(_)` to `foldable` in `fold` (`:467`). In the `Body::User` arm (`:531`), cap at `if self.open.contains(&i) { INLINE_MAX_LINES } else { USER_LINES }`.
> - Add `pub fn selected_prompt(&self) -> Option<(&str, Option<i64>)>` (text, ts of the highlighted `Body::User` row) next to `selected_call` (`:371`).
> - In `traces_modal.rs` `follow_inspect` (`:1042`), when there is no selected call but there is a selected prompt: `raw = Some(text)`, `raw_source = None`, `payload.open(Some(Ok(text)), Shape::Prose)`, `raw_scroll = 0`, `follow_return = true`, `mode = Mode::Inspector`, return true. Esc already returns to Follow (`:4137`).
> - In `render_follow_payload` (`:5347-5364`), when the selection is a prompt, the head title is `prompt` and its time is the row's ts.
> - (b) `xai-grok-traces/src/follow.rs`: the brief is never lost to the backlog. `resume_after` returns `(i64, Option<EventRow>)`, where the row is the first non-noise `message`/`user` row, when it is at or before the cut. Store it in a new `Followed.brief: Option<EventRow>`:
>   - `adopt` (`:440`) sets it only when `depth > 0 || s.parent_id.is_some()`, so root sessions keep today's backlog.
>   - `:706` and `:738` (children) always set it.
>   - `agent_at` (`:464`) sets `None`.
>   - In `tick`, after the announce/meta block and before the events loop (`:754`), `brief.take()` is emitted through the same path as loop rows: `raw_line`, then `json_event` (with `session = rid` when `rid != id`) or `format_event`.
> - Applies to every harness (it keys on the first user row, not line 0) and to the `grok-traces follow` CLI.
> - Tests (written, not run): in `traces_follow.rs`, `l` on a 10-line user row shows all 10 lines and `h` folds it back; after `g`, `selected_prompt()` returns its text. `the_inspect_target_is_the_highlighted_call_only` stays as is. In `traces_modal.rs`, set a `Follow::bare` with a user item as `follow_view`, step onto the prompt, then `follow_inspect()`: mode is Inspector, the payload holds the full text, and Esc returns to Follow (pattern: tests near `:7920-7955`). In `xai-grok-traces/tests/follow.rs`, follow the `CHILD` session directly with `with_backlog(1)`: kinds start `["model", "user", …]` and the user text is "look at the lexer". The existing root tests (`follow_backlog_limits_the_first_rows`, `…_zero_…`, the `with_backlog(2)` ended-session test) must still pass unchanged, which is why the gate is `parent_id`/depth.

## Task 3: trace views offer no dashboard. Verdict: needs change (small)

- The causes are real. Locations corrected:
  - `[Dashboard]` gate: `agent_view/render.rs:1417-1431` (`dashboard_available` at `:1417-1423`).
  - Hit area: `:1450-1451`.
  - Guard arm: `dispatch/router.rs:1096-1101`.
  - Dispatch: `router.rs:1595`.
  - `role.rs:73-76` and `standalone_traces.rs:189` are correct.
  - `trace_return` is also set at `standalone_traces.rs:244` for live sessions opened from traces. Those views are trace views too and get the same fix.
- `mouse.rs` needs no edit. `hit_dashboard` is set from `areas.get("dashboard")`, which is `None` once the button is not pushed, so the click at `mouse.rs:229-234` cannot fire.
- Normal (non-trace) agent views have `trace_return == None`, so their button and Ctrl+\ are unchanged. `header_tests.rs` (`dashboard_button_follows_the_feature_gate_outside_the_overlay` and others) still covers them.
- The other `OpenDashboard` dispatchers run before any trace view exists, so the guard does not affect them: `event_loop.rs:1844` (`GROK_OPEN_DASHBOARD_AT_STARTUP`) and `dispatch/session/lifecycle.rs:1153`.
- The traces ledger itself does not open the dashboard on Ctrl+\ (checked on screen).
- Missing from the spec: two snapshots contain `[Dashboard]` and break, and builders cannot run `cargo insta`.

Replace task 3 with:

> 3. **Trace views offer no dashboard.**
> - `app/dispatch/router.rs:1096-1101`: add `| Action::OpenDashboard` to the `in_trace_view` guard arm and extend its comment. This covers Ctrl+\ (and its Ctrl+4 alt) and a click.
> - `app/agent_view/render.rs:1417-1423`: `dashboard_available = in_dashboard_overlay || (self.trace_return.is_none() && self.child_link().is_none() && …dashboard_dispatchable())`.
> - Hand-edit line 5 of `app/agent_view/snapshots/xai_grok_pager__app__agent_view__trace_view_tests__trace_view_full_width.snap` and `…trace_view_half_width.snap` to `  /work/demo` (button and trailing spaces gone). The `side_panel` snapshots are normal sessions and keep the button.
> - Test (written, not run), in `app/agent_view/trace_view_tests.rs`, next to `cancel_and_rewind_do_nothing`: in `opened(&tmp)`, `draw(.., 100)` has no `[Dashboard]` and `hit_dashboard.rect` is `None`; `dispatch(Action::OpenDashboard, &mut app)` is empty and `active_view` is still `ActiveView::Agent(_)`.
> - Esc and `v` stay the way out.

One question for Sab, not added to the spec: Ctrl+N (`NewSession`, `When::Always`, `actions/defaults.rs:766-775`, press twice) is also unguarded in trace views. I have not traced what it starts there. Should it join task 3?

## Build policy section

Matches Sab's words (standing for the whole sweep, 638d5719). Builders can finish all three tasks from code with the edits above. The only things they cannot confirm are the snapshot text in task 3 and the tag truncation glyph in task 1. The single batch cycle settles both (`cargo insta review` if the snapshot differs).

Seen while checking (side issues, not in scope): in the deck, a finished call kept a running timer (`… 8m43s`) while later calls had results, and at 40 columns the deck header line did not draw.
