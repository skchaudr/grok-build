# Review: SPEC-subagent-rows.md (Sonnet 5.5, 2026-11-12)

Checked against main at 8af717b7. Paths are under `crates/codegen/`. `P` = `xai-grok-pager/src/views/traces_follow.rs`, `M` = `xai-grok-pager/src/views/traces_modal.rs`, `D` = `xai-grok-pager/src/views/traces_modal/deck.rs`, `T` = `xai-grok-traces/src/follow.rs`. Read-only: no cargo, no edits to code.

## Departures from Sab's words (for Sab to rule on)

Already in the spec: (1) do both navigation features, (2) `f`/`p` keys, (3) the mockup layout, (4) the timer bug. Added by this review:

5. The Agent-row summary (`{model} · {N} calls · ✓/● duration`) is the lead's. Sab said "immediate details on the subagent" and "know the model". Call count, live/done mark and duration are the lead's reading of "details".
6. The tag's size rules (widest tag + 2, capped at `width / 4`, model dropped first) and the tag colours are the lead's. Sab's only words are that the batch 1 tag "does not signal this is a subagent".
7. The tag colour is a pick of this review (spec's colour source is wrong, see Row 1). It must be seen in every theme before it counts.
8. Linking an Agent call to its child is a label match, not an id match. No item carries a call-to-child id (details under Row 3). A wrong link shows another agent's numbers on a row.
9. The prompt band loses its leading space on tagged rows only (Row 2). Root prompt bands keep ` › `, so a subagent's prompt sits one column left of a root prompt.
10. `p` lands on the spawn row by label and start time. With two same-label dispatches it can pick the wrong one.
11. The `↳ … started` row stays under the tag (the spec's mockup drops it). It is the only row a receipt-born child has before its model arrives, and `pty_e2e/traces_modal_follow_receipts.rs:39` waits for `↳ started`.

## Row 1: tag. Verdict: needs change

- Claim wrong: "`↳` and `{n}` use the theme's accent the band already uses for `↳ … started`". The `↳ ` in that row is `gray_dim` (`P:681`). The only accent on it is `theme.accent_model` on the model name (`P:683`). The band is not accented.
- `lead()` (`P:528-536`) returns a `String` used as one `gray_dim` span (`P:541`, `plain` at `P:545`, the prompt arms). Colour needs spans.
- Column width counts visible subagents: `lead()` is called several times per row per frame (`render` at `P:760-790`, `bodies` at `P:~737`). Use all subagents (`self.agents[1..]`), so the column does not jump when Tab filters.
- Batch 1 repeats the tag on every wrapped line and every inline-body line. Keep that.
- `T = (width/5).clamp(6,16)` at `P:533` is what the cap replaces.

Replace Row 1 with:

> 1. **Subagent tag** (`traces_follow.rs`). Add `Agent.started: Option<i64>`, `Agent.ended: bool`, `Agent.end_ms: Option<i64>`. In `apply`: `agent_start` sets `started` from the item's `ts` (`:305`); `agent_end` sets `ended = true`, `end_ms = ts` (`:311`); `agent_resume` sets `ended = false` (`:320`), for every agent, not only the root.
> - `fn tag_text(&self, at: usize) -> (String, String)` returns `(head, model)`: `head = format!("↳ {at} {}", self.tag_name(at))`. `model = short_model(m)` when the agent has a model and its label is non-empty, else `""` (an unlabelled agent's name already is the model, so it shows once).
> - `fn tag_w(&self, width: usize) -> usize` = (widest `head + " · " + model` over `agents[1..]`) + 2, clamped to `6..=(width / 4).max(6)`.
> - `fn lead(&self, i, width) -> String` keeps its signature. For a subagent row it returns the tag fitted to `T - 1` columns and padded to `T`, then `"  ".repeat(depth - 1)` capped by `MAX_INDENT`. Fit: the full `head · model` if it fits, else `head` alone, else `truncate_str(head, T - 1)`. Root rows are unchanged.
> - Add `fn lead_spans(&self, i, width, theme) -> Vec<Span<'static>>` with the same text: `↳ {n}` in `theme.accent_model` bold, the name in `theme.gray`, ` · {model}` in `theme.accent_model`, padding and indent in `gray_dim`. `row_lines` uses it for `plain` (`:545`) and for the three `ind.clone()` spans in the prompt arm (`:567-580`). `body_lines` (inline call bodies) keeps the plain `lead()` string in `gray_dim`.
> - No new theme field. The look is checked on screen in every shipped theme at full and half width.

## Row 2: alignment. Verdict: cause confirmed, fix is narrower than the spec says

- Cause: the prompt arm builds `" › "` for the first line and `"   "` for the others (`P:~570`). `◆` and `↳` sit at the tag column, `›` one past it. The snapshot shows it: `◆ Read` at col 16, `›` at col 17.
- On root rows the same space is the grok look and `l_opens_a_long_prompt…` and `the_selected_row_is_grok_selection_in_every_theme` assert `" › fix the parser"`. Do not touch root.

Replace Row 2 with:

> 2. **Alignment.** In the `Body::User` arm (`:565`), when `row.agent != self.root.id`: the first line's lead is `"› "`, the others `"  "`, the `… +N lines` line starts `"  … +"`, and the wrap width is `w.saturating_sub(3 + beside)` (root keeps `4 + beside`). The band background still starts at the tag column. Root prompt bands are unchanged.

## Row 3: Agent call row. Verdict: the link claim is wrong; rewrite

- The claim "the link is the receipt that makes a receipt-born child (`T:706`)" is wrong. No item carries a call-to-child id. `agent_item` (`T:221`) has agent, model, label only; the `ChildStart` receipt (`T:568`) has an `agent_id` and no `tool_use_id`. The pager gets a `call` item (`call_id`) and an `agent_start` item, and nothing ties them.
- Precedent in the code: the trajectory screen already pairs a dispatch row with its child by label, then by position (`M:1793-1815`, `is_dispatch` at `M:3550`: tool `agent` or `task`, any case). Claude's `.meta.json` has the exact `toolUseId` (`links.rs claude_children`, used by `acp/claude_feed.rs:66`), but nothing in the follow items carries it. Exact linking is a later upgrade.
- Wrong assumption: "✓ once the call has a result". A background agent's `Agent` call gets its result at launch. Live data, session `claude:243c71321e1bb59f55eeba07` (`grok-traces follow --json --backlog 400`): five `Agent` calls, each with a result of 338-350 ms, text "Async agent launched successfully". So call result is not child done. Child done comes from `agent_end` only.
- "`■ ended`": a child's end row is `↳ ended` (`P:~700`). `■ ended` is root.
- A child that is not Active when the follow starts is never adopted (`T:~758`, `kid.status != Active`). Its Agent call has no agent to link to. That state keeps today's right side.
- Counts come from rows the pager holds. A child followed mid-run holds at most the 200-row backlog, so `N` can undercount. Accept; the spec's wording "the pager holds" is right.
- Redraw: `has_open_call` (`P:229`) is what makes the 1 s poll redraw a running timer. A linked Agent call with a result and a live child has no open call, so its `● 1m50s` would freeze. Add the live-child case.

Replace Row 3 with:

> 3. **Agent call row** (`◆ Agent  {label}`).
> - Link: `fn links(&self) -> HashMap<usize, usize>` (row index to agent index). A call row counts when its tool, lowercased, is `agent` or `task`. For each subagent with a non-empty label, take the call rows whose normalised `what` equals, starts with, or is started by its normalised label (`norm` as in `M:3554`: one line, lowercase), whose `started_ms <= agent.started + 5000`, and pick the one with the largest `started_ms`. Later agents win a shared call. Computed once per `render`, passed to `row_lines`.
> - A linked row's right side is `{model} · {N} calls · {status}` where `N` is the count of `Body::Call` rows whose `agent` is the child's id (`1 call` for one), `model` is `short_model` (omitted when unknown), and `status` is `● {span_text(now - started_ms)}` while `!agent.ended`, else `✓ {took(end_ms - started_ms)}` (just `✓` when `end_ms` is `None`). The call's own `result` is ignored for the mark, so a background agent shows `●` until it ends. A failed result (`!r.ok`) keeps today's red `✗ …` and the unlinked right side.
> - Diamond: `accent_running` while the child is live, else today's dimmed green.
> - Width: the label gets at least 12 columns. If it would get less, drop `N calls`; if still less, drop `model`; if still less, keep `status` only.
> - An unlinked Agent call keeps today's right side.
> - `has_open_call` (`:229`) also returns true when `!self.ended` and any subagent has `!ended`.

## Row 4: timer bug. Verdict: cause found for the snapshot; the live one is not proven

- The `… 0s` in Sab's snapshot is the fixture. `the_subagent_tags_at_two_widths` (`P:1143-1165`) opens two `Agent` calls (`:1151-1152`) and never gives them a `done`, and `plain` (`:1145`) nulls every `ts`, so `started_ms` = `now` (`:263`, `last_ts` starts at `now_ms`) and the running arm prints `… 0s` (`:664`). The code is right for that input.
- The prior review's `… 8m43s` on a finished call in the real deck is not reproduced. Possible cause, not confirmed: a call that opened from a `Pre` receipt (`T:~600`) whose `Post` receipt never came and whose file result never matched, in a live parent, where `agent_end` marking `no_result` (`P:311`) never runs. A live-session run is needed.
- Row 3 fixes the `Agent` case for linked calls (the child's end overrides).

Replace Row 4 with:

> 4. **Timer.** In the snapshot test, give the two `Agent` calls a `done(...)` or let them link (Row 3). Do not change the running arm at `:664`. The builder reports in its commit message that the live `… 8m43s` is unexplained; no code change for it.

## Navigation. Verdict: needs change

Keys, from code:
- `f` and `p` are free in follow. Deck `handle_key` (`D:90-127`) takes Esc/q, Enter, `o`, `v` (`D:107`), Tab/BackTab, then `Follow::key` (`P:457`: j k PgDn PgUp g Home G End Tab BackTab e l Right h Left). The non-deck arm (`M:3989-4003`) takes Esc/q, Enter, `o`, then `Follow::key`. `traces_key` ignores Ctrl/Alt chords first (`M:3868`). `f` at `M:3899` is List and `f`/`p` at `M:4047`/`M:4119` are Trajectory, not Follow.
- Tab in the real UI is `jump` (`D:129`): `roster` (`D:50`) is each Active root followed by its Active children, so Tab already opens a running subagent as its own follow and cycles through them. `Follow::cycle_agent`/`filter` is reachable only without the board (tests), so `f` cannot use `filter`; the spec wants the brief at the top, which only a follow of the child's own row gives.
- `p`, `f` do not conflict with `SPEC-traces-agent.md` rev 5. Ask keys (F3, Ctrl+N, Enter in the block viewer) live in the trace view; `v` (rev 5 cites `deck.rs:107`, still right) is the only way from follow to it.
- `state.status` (`set_status`) is not drawn on the deck (`D:190-300` never reads it). Spec text "the status line says `spawn row not loaded`" would be invisible. The follow screen's own status row (`P:~745`) is the place.

Smallest path: reuse `open_follow_row` (`M:1019`), which Tab and the strip click already call.

Replace Navigation with:

> - **State.** `Follow.note: Option<String>` joins the status line in `render` (`:~745`, with paused/agent/index) and is cleared at the start of `Follow::key`. `Follow.want: Option<(String, Option<i64>)>`.
> - **`f`.** `Follow::cursor_agent(&self) -> Option<&str>`: the highlighted row's agent when it is not the root; else, for a highlighted Agent call in `links()`, that child. Nothing is highlighted while the tail is pinned (as for Enter); `f` then does nothing, as on a root row. `TracesModalState::follow_into` (next to `follow_inspect`, `M:1042`): `index.session(&id)`, else `index.session(call.session)` from any of that agent's calls (`Follow::session_of(&id)`); on a row call `open_follow_row(row)`; with none set `f.note = "no transcript yet"` and return true.
> - **`p`.** `follow_parent`: `pid = root.parent_id` (none: do nothing); `parent = index.session(pid)` (none: `note = "parent not indexed"`). Take `(root.label, root.started_at)`, `open_follow_row(parent)`, then `follow_view.want = Some(..)`.
> - `Follow::resolve_want`, called in `poll` after `apply` on the first `Ok` tick (and by tests directly): find the Agent-or-Task call row by Row 3's rule (label match, `started_ms <= started_at + 5000`, largest `started_ms`); found: `cursor` = that row, `paused_len` set as `pause_at` does; not found: `note = "spawn row not loaded"`, tail stays pinned. `want` is cleared either way.
> - Wire both keys in `deck::handle_key` (before the `code =>` arm) and in the `Mode::Follow` arm at `M:3989`.
> - Hints: `deck.rs render_footer` (`D:299`) adds `f` "subagent" and `p` "parent" after `v`; `M:4333` adds `plain("f subagent")`, `plain("p parent")` before `Esc back`.
> - Docs: in `TOOL-traces-modal.md` Keys (Follow screen), add `f` (on a subagent row or its Agent row: follow that subagent) and `p` (follow the parent, cursor on the Agent row that spawned it). The same Keys line says Tab cycles "root alone, each subagent alone"; with the board (the real UI) Tab is "next live session" (`D:108`). Correct it in the same edit.

## States and tests

Missing states, each with a rule above: pinned tail (no highlight), receipt-born child with no row, parent not in the index, child finished before the follow opened (call unlinked), background agent (result at launch, `●` until end), child resumed (`●` again), same-label dispatches, grandchildren (tag keeps the depth indent), Tab filter (tags stay, column from all agents), widths 60/40.

Tests that break:
- `P:971` (`items_become_rows_…`): `starts_with("1 Check")` becomes `starts_with("↳ 1 Check")`.
- `P:1134-1135` (`subagent_rows_start_with_a_numbered_tag_…`, width 40): `starts_with("1 UI")` and `starts_with("2 ob")` become `"↳ 1 U"` and `"↳ 2 o"`. At 40 the cap is 10 columns, so `"↳ 1 UI/U…"` is the text.
- Snapshots `subagent_tags_width_120` and `subagent_tags_width_60` (replaced, as the spec says). `traces_modal_follow_receipts.rs:39` (`↳ started`) still passes with the Start row kept.
- `the_selected_row_is_grok_selection_in_every_theme` and `the_diamond_carries_the_outcome…` stay as they are (root prompt untouched, `◆` found by search).

Add (written, not run), in `P` unless noted:
- the spec's five snapshots, plus a linked live and a linked done Agent row at 120 and 60, and an Agent row whose call has a result while the child is live (must show `●`).
- `links()` pairs "Audit traces: UI/UX lens" and "…observability lens" to their calls; an unlabelled child links nothing.
- `a_prompt_band_on_a_tagged_row_starts_at_the_tag_column`: the `›` column equals the `◆` column; a root prompt still starts ` › `.
- `resolve_want` finds the spawn row, and with no match sets the note and leaves the tail pinned.
- in `M`, next to `enter_on_a_followed_call_opens_its_payload…` (`:7938`): `f` on the subagent's row opens a follow whose `root.id` is the child; `f` on a root row does nothing; `p` from there opens the parent with the cursor on the Agent row.

## Builder can finish from code

Yes, every row and key above. The batch cycle settles: the snapshot text (tag truncation glyph, model dropped at 60), the tag colours and the aligned band in every theme (`GROK_THEME=<name>`, full and half width), whether the two new footer hints fit at 100 columns, and the live `… 8m43s` (needs a running session; report only).
