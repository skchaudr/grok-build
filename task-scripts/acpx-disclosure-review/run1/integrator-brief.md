# Integrate packet 1: executed timeline (the pilot)

Repository: `/Volumes/Mini-SSD/offload/repos/grok-build`, branch `main`. Same method as run4 (`task-scripts/acpx-disclosure-review/run4/integrator-brief.md`).

## What you integrate

`task-scripts/acpx-disclosure-review/run1/grok-build/task-scripts/acpx-disclosure-returns/executed-timeline.edits.md`: 6 edits to `crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs`. All 6 still match current `main` exactly once (checked at `d5c62eef`; `run.rs` is unchanged since `f52a8905`).

Spec: the `executed-timeline` task in `task-scripts/acpx-disclosure-slices.py`. Review of the return: `run1/review.md`. Keep the behaviour as returned; do not decide its points 1–4, list what you observed for each in the report.

Acceptance: finished attempts start folded and Space/Right expands them; expansion survives refresh and the noise toggle; build → repair → build stays in order with the live attempt as its own row; an untaken switch branch is absent from Skipped; the waiting stage and its choices stay visible.

## Test and ship: one cycle

1. `pgrep -x cargo`: wait if another cargo is running.
2. `cargo test --release -p xai-grok-pager acpx`
3. At most three fix rounds, smallest edit inside `run.rs`. When a fix changes a test instead of code, say why. Stop and report verbatim if still failing.
4. Skip `cargo fmt`.
5. Commit only `run.rs`, message ending `Co-authored-by: claude-code sonnet-5.5`. Push to `main`.
6. `scripts/install-fork.sh`.

## Do not touch

Any file but `run.rs`. Never kill processes by name. No new panes or tabs. No workflow runs or gate answers.

## Report

Write `task-scripts/acpx-disclosure-review/run1/integration-report.md` (edits, fixes with reasons, test result line, commit, install checksums, what you saw for review points 1–4), commit and push it, then say "done".
