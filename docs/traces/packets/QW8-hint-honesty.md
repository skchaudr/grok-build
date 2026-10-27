# QW8: hint bar lists only keys that work now, plus the unlisted real ones

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issue 10 (hint half only; resume commands are
out of scope here).

## Problem

`shortcuts()` (`P/views/traces_modal.rs` ~3401) is static per mode:

- `H host` is listed with zero configured hosts (the key is guarded at ~3143).
- `r resume` is listed for sessions whose source has no resume target
  (`resume_target` ~351 supports only claude/codex/grok; on others `r` prints
  "no resume for …").
- Working keys are unlisted: `g`/`G` (ends) in List and Trajectory, `y` (copy prompt)
  in Trajectory, `c`/`o`/`q` in Live, PageUp/PageDown everywhere.

## Change

Make `shortcuts(state)` state-driven:

- `H host` only when `state.hosts` is non-empty.
- `r resume` in List only when the selected session's `resume_target` is `Some`;
  in Trajectory only when the open session's is. (No change to what `r` does.)
- Add: `g/G ends` to List and Trajectory; `y copy` to Trajectory; `c fold`,
  `o open` to Live (match the actual key behavior in `handle_traces_key`; `q` stays
  unlisted, Esc covers it).
- Keep total hint count within the two footer lines: when adding keys, drop in this
  order until it fits: `E`, `</>`, `n`, `x`. Do not change the footer height.

## Tests

- Table test over (mode × state): List with/without hosts; List with a pi row
  selected (no `r resume`) vs a claude row (`r resume`); Trajectory with a dsh
  session vs a grok session; Live hints contain `c fold` and `o open`.
- No test may assert a hint for a key that is a no-op in that state.

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Diff budget: under ~5k chars excluding tests.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
