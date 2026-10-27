# QW6: trajectory says when it is silently truncated at 5000 events

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 20.

## Problem

`EVENT_PAGE = 5000` (`P/views/traces_modal.rs` ~30) caps the events loaded for a
trajectory (local ~611, remote ~1639). A longer session shows the first 5000 events
with no notice anywhere.

## Change

In `render_trajectory` (~3852), in the meta line built from `segs` (~3863): when the
loaded event list was truncated, append a segment `· showing 5000/N` styled with
`theme.accent_user`.

Truncated means: no filter is active (`!state.filtering()`), the loaded list hit the
page (`state.events.len() >= EVENT_PAGE`), and the session reports more events
(`session.event_count > state.events.len() as i64`). Do not fire the notice for
remote sessions while `state.remote_loading` is true.

## Tests

- Unit: construct state with 5000 synthetic events and a session whose
  `event_count` is 6000 → rendered meta line contains `showing 5000/6000`.
- Same with `event_count` 5000 → no notice; same with an active search query → no
  notice.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Diff budget: well under ~5k chars excluding tests.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
