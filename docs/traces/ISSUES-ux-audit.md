# /traces UX audit — issue list for small dispatches

2026-10-26, audit of `crates/codegen/xai-grok-pager/src/views/traces_modal.rs` (`MODAL`, 7058 lines) against Sab's complaints: (1) inline scripts unreadable, Enter shows metadata + a smaller view; (2) chrome flood; (3) crowded, a teaser not a tool; (4) wants watch/tail streaming, collapsible everything, tool calls that come to you.

Each issue: evidence, where SPEC-usable.md already covers it (`T#`), or `NEW`. Suggested step sizes follow AGENTS.md (one file/function, diff under ~5k chars). Quick wins are marked ⚡ (≤ ~30-line diff, no design risk).

## P0 — cannot see the tool call (the teaser)

1. **Enter shows metadata + truncated raw JSON, never the output.** `open_inspector` (MODAL:1487) stores `pretty(raw line)`; `render_inspector` (4306) draws a `type · line · time · duration` header and truncates every line at viewport width — no wrap, no horizontal scroll. The one place that should show a full script is the least readable view in the modal. → **T4**. Steps: (a) payload view = T2 blocks, uncapped, scroll; (b) `m` metadata toggle keeps today's JSON; (c) `w` wrap toggle; (d) `y`/`Y` copy split.
2. **Inline scripts render as JSON with literal `\n`.** `command`/`cmd`/`script` are not in `TEXT_KEYS` (2860), so `input` objects hit `to_string_pretty` (2887) and newlines stay escaped. → T2 root fix. ⚡ Interim step: treat string values of `command|cmd|script` as text in `collect_text` (one match arm + test) — real newlines today, before the full block renderer.
3. **Bare numbers (`3005`, `0`) leak into bodies.** `collect_text` recurses into every object; `usage.input`, `cost.input` match `INPUT_KEYS` and non-strings are pretty-printed (2886-2890). → T2's parse_body kills it at the source. ⚡ Interim step: never descend into `usage|cost|info|metadata` keys (one guard + test).
4. **Double/triple blank spacing.** Chunks joined `\n\n` (2874), separator wrapped `\n\n` (2925), blank row pushed before the result (1440), then `row_gaps` (2808) adds another spacer line around any block taller than one line. Expanded rows read 1 blank line per 2 content lines. → T2 spacing rule. ⚡ Step: single-blank rule in `compose_body`/`body_lines` (join with `\n`, drop the push at 1440); keep `row_gaps` for turn blocks only.
5. **All tool calls are hidden by default.** First-sight defaults collapse runs and event rows (MODAL:1199-1213: "turns open, runs and event rows closed"): opening a session shows prompts and `N tool calls` rows — every actual call needs a manual `l`. This is the biggest single reason it feels like a teaser. `NEW`. ⚡ Step: default runs open when the turn has ≤ 2 runs or ≤ 5 calls (threshold const + tests); `E` semantics unchanged.
6. **25 identical `bash  python3 -c "` rows.** `call_detail` takes the first command line (`T/classify.rs:234-265`); intent fields (`description`, `task_name`) are dropped at index time. → **T1** (schema 8 `intent` + `command_headline`), already spec'd.
7. **Long output is unreachable.** `wrap_body` caps at `BODY_LINES` = 200 with tail `… Enter for inspector` (2757); the inspector then truncates per line (4318). A 500-line result cannot be read anywhere. → T4. ⚡ Interim step tied to issue 1's wrap toggle.

## P1 — chrome flood

8. **Five header rows before the first event.** Trajectory: chip+title (3869-3873), meta line `status · agent · model · age · N evt · tools · dur · ↑parent` (3876-3900), 2-row ACTIVITY strip (3840-3863), filter/cwd line (3926-3927); list starts at `area.y + 5` (3929). On a 24-row terminal the list gets < 80% of the modal. `NEW`. Steps: (a) fold meta into the title line (drop `evt/tools` — they are in the turn counts); (b) ACTIVITY strip behind a key (default off) or down to 1 row; (c) filter line only when a filter is active (cwd moves to the inspector/payload header).
9. **22 fixed columns per event row.** `#line` (6) + role chip (6) + per-row wall-clock time (8) + gaps (TEXT_COL=14, 4041-4049). At 80 cols ~28% of width is row chrome; `#line` is an index-debug detail and per-row `HH:MM:SS` repeats the turn's time. `NEW`. Steps: (a) drop `#line` (keep it in the payload header); (b) drop the time column except on turn rows, or make it `+Ns` relative and 4 wide; (c) chip → 1 char (`› ✓ ✗ ◆`).
10. **Hint bar is a wall, and parts of it lie.** 18 hints in List mode (3424-3442), 13 in Trajectory, over 2 footer lines; `H host` shows with zero hosts (key guarded at 3156), `r resume` shows for pi/omp/cursor/dsh/bb where it prints "no resume" (1509, `resume_target` 351-361). Working keys are unlisted: `Space`, `g/G`, trajectory `y`, live `c`/`o`/`q`, PgUp/PgDn. → **T8** (state-driven hints, resume per harness). Steps per T8's own split: (a) `shortcuts(state)` honesty; (b) status TTL 4s; (c) resume CopyCommand per source.
11. **Status line never auto-clears.** `indexed N changed files` set on every open (`effects/mod.rs:2788`) stays until overwritten. → T8. ⚡ Step: `status: Option<(String, Instant)>`, clear on the 1s poll tick after 4s and on any nav key.
12. **`(no more)` rows.** An expanded row whose header already holds the full text still gets a body row reading `(no more)` (NO_MORE, 1434). One wasted line per expansion, hundreds per session. `NEW`. ⚡ Step: render no body row when the continuation is empty (delete the `vec![NO_MORE]` arm; adjust the `in_result` render check at 4066).
13. **Ledger flag spam.** Right side always prints `sort:updated · N sessions` plus every toggle (`● live · indexing… · errors · active only · all · local only · 2 active · 1 failed`, 3610-3641). Default-state flags are noise. `NEW`. ⚡ Step: print only non-default flags; keep the counts.
14. **Modal frame tax.** `footer_lines: 2` + title bar + `h_pad: 2` (3474) on every mode, including Live which then draws its own footer hints. `NEW` (low priority): 1 footer line in Trajectory/Live; hints truncate with a `?` key for the full list.

## P2 — "tool calls come to me" (stream model)

15. **No follow mode in a trajectory.** The 1s poll appends events and follows only when already at the bottom (test 4442); there is no `tail -f` mode: you cannot park on a running session and watch calls stream with their results. `NEW`. Steps: (a) `f` follow toggle — selection pins to the newest row, runs auto-expand while following, any nav key drops out; (b) follow + `x` = watch errors only.
16. **Live rows are one line; no peek.** `Mode::Live` shows glyph/title/now-text/age per agent (2617-2708); the current tool call's output needs `Enter` → full trajectory → expand. `NEW`. Step: `l` on a live row expands an inline tail of that session's last N body lines (bodies already cached per line; fold worker supplies rows). Depends on nothing in T0-T5.
17. **No error navigation.** `x` filters to errors but there is no next/prev-error jump; in a 5000-event session you scroll. `NEW`. ⚡ Step: `!` next error / `*` previous error over `events` (`is_error` or folded result error), wrapping, with the count in the status line (`error 3/7`).
18. **`t` cycles tools one at a time.** With N distinct tools, reaching the last takes N-1 presses (3287-3295). `NEW`. ⚡ Step: `T` cycles backward; later a `t` popup picker with counts from `index.facets`.
19. **Mouse is dead outside Live.** modals.rs:1523 — "keyboard-first; the chrome takes clicks and the wheel scrolls the live screen". Wheel does nothing in the ledger/trajectory; rows are not click-selectable. `NEW`. Steps: (a) wheel = `j/k` in ledger/trajectory; (b) click selects a row, double-click opens. (Trace-deck will want this regardless.)

## P3 — correctness and honesty gaps

20. **Silent event truncation at 5000.** `EVENT_PAGE` = 5000 (MODAL:30, 611, 1639); a longer session shows the first 5000 events with no notice. The meta line shows `event_count` so a careful user could notice the mismatch; nobody will. `NEW`. ⚡ Step: when `rows.len() == EVENT_PAGE && session.event_count > rows.len()`, suffix the meta line `· showing 5000/N`; later, `G` loads the next page.
21. **Empty `› Assistant` rows.** Assistant records with no text/thinking/tool block fall through to the role title. → **T8** (normalizer marks them noise). One normalize arm + test.
22. **Enter has three meanings, one unlisted.** Run row → toggle; dispatch row → open child; else → inspector (3303-3310). Hint says only "Enter inspect/open agent". → T8 hint step.
23. **Viewport jumps on expand and on body arrival.** `fit_rows` forces the whole selected block on screen (2776); `ensure_bodies` caches mid-render and heights change after paint (3949-3958). → **T3** (anchor scroll), already spec'd.
24. **Remote bodies grow rows after `loading…`.** Same jump, remote path (1370-1374). → T3 covers with 23.
25. **Ledger `+N more (l)` and `(+N)` compete with the name.** Name column truncates to make room for the hidden count (3729-3731); with MAX_KIDS=6 default and deep trees, real names lose ~10 cols to bookkeeping. `NEW` (minor): put `(+N)` in the NOW column, or only on collapsed rows (already) *and* never narrower than 24.

## Coverage map

SPEC-usable Track A already owns: 1 (T4), 2+3+4 (T2), 6 (T1), 7 (T4), 10+11 (T8), 21 (T8), 22 (T8), 23+24 (T3). **New here:** 5 (runs open by default — the single biggest "teaser" fix), 8, 9, 12, 13, 14 (chrome), 15-19 (stream model + mouse), 20, 25.

## Suggested dispatch order (cheap → structural)

All steps below are independent unless noted; each is one packet per AGENTS.md.

1. ⚡ issue 2: `command|cmd|script` as text in `collect_text` (+ test that inverts the escaped-`\n` pin at ~5093).
2. ⚡ issue 3: `usage|cost|info|metadata` descent guard in `collect_text`.
3. ⚡ issue 11: status TTL.
4. ⚡ issue 12: drop NO_MORE rows.
5. ⚡ issue 13: non-default flags only.
6. ⚡ issue 20: 5000-truncation notice.
7. ⚡ issue 5: runs open by default under threshold. (Do after 1-4 so the newly visible rows are readable.)
8. ⚡ issue 17: `!`/`*` error nav.
9. Issue 8+9: header/row-chrome diet (one packet, render-only).
10. T0 extraction (serial), then T2/T4/T3 as spec'd.
11. Issue 15: follow mode (after T3 anchoring).
12. Issues 16, 18, 19: live peek, tool picker, mouse.

Verify each with: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`; pty proof for the render-changing ones (`pty_e2e_config_ui`).
