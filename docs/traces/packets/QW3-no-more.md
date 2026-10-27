# QW3: delete the `(no more)` phantom row under expanded events

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 12.

## Problem

Expanding a trajectory event whose header already holds its full text still produces
a body row reading `(no more)` (`NO_MORE`, `P/views/traces_modal.rs:44`, produced in
`body_lines` ~1437, color-checked at ~4052). One wasted, selectable line per
expansion.

## Goal

An expanded row with nothing more to show takes zero extra screen lines and can never
be selected. Expanded rows with real content are unchanged.

## Traps (read first)

- Body rows are real outline nodes: `j/k` selection steps over `oshown` positions, so
  a zero-height row must not be landable on the selection, and `fit_rows`/`row_span`
  must treat it as height 0.
- Bodies load lazily: whether there is "more" is known only after `settle_body`
  caches the text. The outline structure cannot know in advance.

## Suggested shape (free to adjust, acceptance below is the contract)

- `body_lines` returns an empty vec for the no-more case (delete the `NO_MORE` arm);
  delete the const and its render color branch.
- `row_heights` maps `Some(empty)` to 0; the render loop skips 0-height rows.
- Selection: store the last computed heights (or the lines of no-more events) on the
  state during render, and make `step`/`goto` skip positions with height 0.
- Empty-body information must not break `fit_rows`, `row_gaps`, or the
  expand/collapse fold behavior of the parent event row.

## Tests

- Expand an event whose text fits its header: no body row is drawn (assert on the
  rendered buffer via the existing `draw` test helper ~4583), and `j` from the event
  row lands on the next visible row, never on a phantom.
- Expand an event with more text: body rows appear exactly as before.
- Existing fold/scroll tests keep passing.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Diff budget: under ~5k chars excluding tests.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
