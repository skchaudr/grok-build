# QW2: traces status line auto-clears after 4s; scan notice only when real

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 11.

## Problem

`TracesModalState.status` (`P/views/traces_modal.rs` ~line 93) never expires:
`indexed N changed files` set on every `/traces` open stays on screen until another
message overwrites it, and a scan with 0 changes still sets it.

## Change

1. Keep the field type `Option<String>`; add `status_at: Option<Instant>` beside it.
2. Add `fn set_status(&mut self, msg: impl Into<String>)` on `TracesModalState` that
   sets both, and convert every writer (`rg "status = Some" P/views/traces_modal.rs`)
   to use it. Sites that clear (`status = None`, e.g. the Live-mode arm ~3205) also
   clear `status_at`.
3. Expiry: at the top of `poll_live_at` (~489) and at the top of `handle_traces_key`
   (~3059), clear `status`/`status_at` when the stamp is older than 4 seconds
   (const `STATUS_TTL: Duration = Duration::from_secs(4)`).
4. Scan summary: find where `indexed N changed files` is produced
   (`rg "changed files" P/app`). Emit it only when N > 0; failure messages
   (`scan failed`, `files failed`) are still always emitted.
5. `render_status` (~3750) needs no change.

Note: another in-flight packet (QW7) adds a status writer; it was told to use
`set_status` when present. Keep the helper `pub(crate)` or private as fits.

## Tests

- Unit test with the existing test style: set a status, advance no time → visible;
  simulate `poll_live_at` with a status 5s old (construct state, set `status_at` to
  `Instant::now() - Duration::from_secs(5)`) → cleared.
- Any-navigation-key-clears is covered by the poll test; do not add pty tests.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Diff budget: under ~5k chars excluding tests. No behavior change to what messages
  say, only when they clear.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
