# QW7: `!` / `*` jump to next/previous error in the trajectory

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 17.

## Problem

A failed call in a 5000-event trajectory is findable only by scrolling or by the
`x` errors-only filter. There is no next/prev-error jump.

## Change

In `handle_traces_key`, `Mode::Trajectory` arm (~3225): add
`KeyCode::Char('!')` → next error forward, `KeyCode::Char('*')` → previous error
backward, wrapping at the ends.

- An event counts as an error when `e.is_error`, or when its folded result failed
  (`state.folded[i]` is `Some(f)` with `f.error`).
- Search starts from the currently selected event (`state.selected_event()`), over
  `state.events`, skipping hidden rows (`state.hidden`).
- On a hit: reveal the target row even when a collapsed ancestor hides it (use the
  existing outline machinery — find the outline node whose `OutlineItem::Event(i)`
  is the target, `reveal` its ancestors, select it by key so folds/filters keep it),
  then set status `error K/N` (K = ordinal among errors, N = total).
  If another packet has already added `fn set_status(&mut self, msg: impl Into<String>)`
  to `TracesModalState`, use it; otherwise `state.status = Some(msg)` directly.
- No errors: set status `no errors` and do not move.
- While typing in the `/` search prompt the keys type normally (the typing arms
  already intercept input first — keep that).

## Tests

- Synthetic trajectory with errors at positions 2, 7, 9: from position 0, `!` lands
  on 2 then 7 then 9 then wraps to 2; `*` from 2 lands on 9 (wrap) — assert the
  selected event index each time.
- An error inside a collapsed run becomes visible and selected.
- `no errors` case leaves selection unchanged and sets the status.
- Hint line: add `!/* errors` to the Trajectory shortcuts list in `shortcuts()`
  (~3443); drop `</> more/less` first if the line overflows its two footer lines.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Diff budget: under ~5k chars excluding tests.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
