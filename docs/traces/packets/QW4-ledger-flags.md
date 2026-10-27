# QW4: ledger header shows only non-default flags

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 13.

## Problem

The ledger's right-aligned flag line (`render_ledger`,
`P/views/traces_modal.rs` ~3596-3627) always prints `sort:updated` and the session
counts, plus every active toggle. With defaults you still get
`sort:updated · 42 sessions`; with one filter on, five items crowd the row.

## Change

In the `flags` block of `render_ledger` only:

- Push `sort:{:?}` (lowercased) only when `state.sort != Sort::Updated` (the default).
- Keep `● live`, `indexing…`, `errors`, `active only`, `all`, `local only`/`host:*`
  exactly as they are (they are already conditional on non-default state).
- Keep the counts segment (`N active · N failed · N sessions`) unchanged and always
  last.

Nothing else in `render_ledger` changes; no state fields change.

## Tests

- Render the ledger with default state (existing `draw` helper style, ~4583): the
  flag line contains `N sessions` and no `sort:`.
- With `sort = Sort::Events`: contains `sort:events`.
- With `errors_only` on and non-default sort: both flags present, counts last.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Diff budget: well under ~5k chars excluding tests.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
