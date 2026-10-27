# QW5: small tool runs start expanded in the trajectory

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 5 — the single biggest "teaser" fix.

## Problem

Opening a session in `/traces` shows user turns and collapsed `N tool calls` run
rows: every actual tool call is hidden behind a manual `l`. Default fold rules live
in `rebuild_outline` (`P/views/traces_modal.rs` ~1194-1215): "turns open, runs and
event rows closed" (`if fresh && !turn && !(filtering && run) { collapse }`).

## Change

A run with few calls starts expanded; a long run stays collapsed.

- Add `const RUN_OPEN_MAX_CALLS: usize = 5;`.
- In the default-fold loop, when the node is `OutlineKey::Run(_)`, look up its
  `OutlineItem::Run { count, .. }` from `outline_items`; skip the collapse when
  `count <= RUN_OPEN_MAX_CALLS`. All other rules (turns, filtering behavior, the
  `oseen`/`fresh` logic that preserves manual folds across live polls) are unchanged.

## Tests

- Update any existing test that pins "runs closed by default" for small runs
  (`rg "tool calls" P/views/traces_modal.rs` in the tests module; also check the pty
  suite `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/` for assertions on
  collapsed run rows and update them if they use small runs).
- New test: a turn with one 3-call run shows the call rows on first render; a turn
  with a 12-call run still shows `12 tool calls` collapsed; manually collapsing the
  small run survives a `poll_live_at` requery.
- `E` toggle-all and `<`/`>` subtree folds behave as before.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
  and, if pty tests were touched,
  `cargo test -p xai-grok-pager-pty-harness --test pty_e2e_config_ui traces`.
- Diff budget: under ~5k chars excluding tests.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
