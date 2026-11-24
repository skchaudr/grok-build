# ACPX TUI audit — main thread first

2026-11-24. Read-only audit of the ACPX surfaces on the Air and the Mini, plus the
first main-thread repair. Evidence: live Rex captures, the `grok-acpx-acp`
projector, the pager's ACP handler, and the runs modal source. Build parity at
audit time: `0.0.0-dev (9bb7303e63b4)`, repo `58db6e5d` on both hosts.

## Surfaces

1. **Main thread** — what `grok acpx` puts in scrollback. Produced by the driver
   projector (`crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/project.rs`),
   rendered by the pager as markdown / mermaid / tool cards / subagent previews.
2. **Flow card** — `AcpxFlowBlock` (`crates/codegen/xai-grok-pager/src/scrollback/blocks/acpx_flow.rs`),
   used by the pager-side `/acpx run` command.
3. **Runs modal** — `views/acpx_modal.rs` plus `run.rs`, `detail.rs`, `ledger.rs`.

The modal is the drill-down. The main thread must answer two questions and
nothing else: *should I act?* and *is it moving?*

## Main thread — ranked defects

1. **The same graph is reprinted on every transition.** `RunStarted` prints a
   full mermaid chart; every `Waiting`, answered checkpoint, and `RunFinished`
   printed another full chart plus the growing taken path (`project.rs` before
   this change). One gated run left four stale diagrams in scrollback and pushed
   the live work off screen. Both the acpx `/flow` driver and the piw
   `/workflow` driver feed this one projector, so the fix covers both. **Fixed in
   slice 1.**
2. **No live "what is happening now" line.** The card header says
   `running · 2/6 · 1m23s` but not which node is live; liveness only appears in
   the separate subagent preview. Next slice: one `now` line with node, owner,
   and elapsed.
3. **Failure did not name the node.** `failed after 1m23s` with no location.
   **Fixed in slice 1** (`failed at <node>: <error>`).
4. **Completed run had no result.** Only `done in 92ms`; the path was in a
   duplicated graph above. **Fixed in slice 1** (one finish line carries the
   taken path).
5. **Gate line was a graph, not a gate.** The question and options live in the
   ask panel, so the transcript only needs `◐ <node> · waiting for your answer`.
   **Fixed in slice 1.**
6. **Flow card repeats a dead instruction.** `acpx cannot resume a run; re-run
   the flow to continue` is wrong for the driver path and noise for `/acpx run`.
   Next slice: delete it; the run id line is enough.
7. **Flow card has no width bound.** Prompts and JSON results wrap to 3–5 lines
   and break the column alignment (`acpx_flow.rs::lines` takes no width).
   Next slice: pass `ctx.width`, cap results, keep full text in detail.
8. **Collapsed card hides the gate.** Only the title/status survive folding, so
   `NEEDS YOU` disappears. Next slice: append `· NEEDS YOU` to the collapsed
   header.

## Runs modal — ranked defects

1. **The frame's `[✗]` close button reads as a failed run** next to a `✓`/`◐`
   header (`views/modal_window.rs:325,405-435`, using the same glyph as Failed).
   Use a neutral `×`/`–`; keep the click rect.
2. **Identity and progress repeat 3–4×**: frame title, run header, banner, graph
   boxes, stage glyphs (`acpx_modal.rs:630-652`, `run.rs:1110-1224`). Keep one
   status strip; delete the repeated flow/title facts.
3. **Unlabeled second duration** (`fmt_age`, `run.rs:1140`). Label it `age …` or
   drop it from the header.
4. **Graph is on by default and eats ~12 rows**, then ends in
   `… graph cut (v hides it)` (`run.rs:130,602-606,1282`). Default it off, cap it
   at 4–5 rows, keep `v` to expand.
5. **Detail pane is mostly empty** (60% of width, two lines) because selection
   defaults to the last root, including not-run nodes (`run.rs:935-1022`,
   `detail.rs:1017-1030`). Default to the last *executed* step; show a run
   overview for not-run targets.
6. **Expanded stages repeat their own duration** via `node_started` /
   `node_outcome` children (`run.rs:737-819`). Fold those by default.
7. **Footer shortcut wall** — 9–10 items on one line, and the footer steals body
   rows (`acpx_modal.rs:654-670`, `run.rs:279-296`). Cap at four mode-specific
   keys plus `? keys`; the full table already exists behind `?`.
8. **Ledger preview split truncates the list** at 120 columns: the row's agents,
   age, and duration columns are dropped (`acpx_modal.rs:35,794-810`,
   `ledger.rs:1338-1344`). Raise the preview threshold to 160; below that, list
   only.

## Slice 1 — what changed

`crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/project.rs`:

- `RunStarted` is still the only event that prints the mermaid chart.
- `Waiting` prints `**◐ <node>** · waiting for your answer`.
- An answered checkpoint prints `**✓ <node>** · answered: <choice>`.
- `RunFinished` prints one line: completed carries the taken path
  (`✓ **title** finished in 2m11s · ✓ a → ✓ b`); failed names the node
  (`✗ **title** failed at <node>: <error>`); cancelled stays cancelled.
- Deleted `mermaid_fence()` / `mermaid_states()`; added `failed_node()`.

Tests: `cargo test -p xai-grok-pager-bin --bin grok-acpx-acp project::` — 18
passed. The updated projector tests assert exactly one graph (at run start), no
` ```mermaid ` after it, the gate line, the answered line, the path on completion,
and the failing node on failure.

## Next

1. Flow card: activity line, width bound, drop the resume hint, gate in the
   collapsed header (`acpx_flow.rs`).
2. Modal: graph off by default, neutral close glyph, one status strip, executed
   step selected, footer cap.
3. Driver: one compact rail line per step finish if the todo panel turns out to
   be invisible in the main-thread layout.
