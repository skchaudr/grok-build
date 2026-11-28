# Integrate packet 7: inspector step navigation and linewise copy

Repository: `/Volumes/Mini-SSD/offload/repos/grok-build`, branch `main`. Same method as run3 (`task-scripts/acpx-disclosure-review/run3/integrator-brief.md`).

## What you integrate

`task-scripts/acpx-disclosure-review/run4/grok-build/task-scripts/acpx-disclosure-returns/inspector-copy.edits.md`: 8 edits, all matching `main` (`2dee6724`) exactly once, no overlaps, no coordinator follow-ups. It changes `views/acpx_modal/inspector.rs` (Edits 1–6) and `views/acpx_modal.rs` (STEP_KEYS and one Step-mode test, Edits 7–8).

Spec: the `inspector-copy` task in `task-scripts/acpx-disclosure-slices.py` at `2dee6724`. The operator's acceptance, from his sab-air test: open a past run, Enter on a step, Right/Left move to the next/previous step without backing out; `j` to a line and `y` copies only that line; `v` range copies those lines; `Y` copies the whole tab.

## Test and ship: one cycle

1. `pgrep -x cargo`: wait if another cargo is running.
2. `cargo test --release -p xai-grok-pager acpx`
3. At most three fix rounds, smallest edit inside `inspector.rs` or `acpx_modal.rs`. When a fix changes a test instead of code, say why. Stop and report verbatim if still failing.
4. Skip `cargo fmt` as before.
5. Commit only those two files, message ending `Co-authored-by: claude-code sonnet-5.5`. Push to `main`.
6. `scripts/install-fork.sh`.

## Do not touch

`run.rs`, `ledger.rs`, `detail.rs`, `grok-acpx-acp/`. Never kill processes by name. No new panes in this tab. No workflow runs or gate answers.

## Report

Write `task-scripts/acpx-disclosure-review/run4/integration-report.md` (edits, fixes with reasons, test result line, commit, install checksums), commit and push it, then say "done".
