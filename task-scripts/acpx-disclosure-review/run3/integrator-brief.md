# Integrate packet 3b: detail-pane focus and linewise copy

Repository: `/Volumes/Mini-SSD/offload/repos/grok-build`, branch `main`. Work in this checkout; no worktree or branch. Same method as run2 (`task-scripts/acpx-disclosure-review/run2/integrator-brief.md`).

## What you integrate

`task-scripts/acpx-disclosure-review/run3/grok-build/task-scripts/acpx-disclosure-returns/detail-focus.edits.md`: 16 edits, all matching `main` (`d0eb35a3`) exactly once, no overlaps. It changes `views/acpx_modal/detail.rs` (Edits 1–10) and `views/acpx_modal.rs` (Edits 11–16). No coordinator follow-ups were listed.

```bash
python3 -I task-scripts/acpx-disclosure-review/check_edits.py . task-scripts/acpx-disclosure-review/run3/grok-build/task-scripts/acpx-disclosure-returns/detail-focus.edits.md
```

Apply each edit with your Edit tool. A zero or double match stops the packet; report it.

The spec is the `detail-focus` task in `task-scripts/acpx-disclosure-slices.py` at `645671a6`. The acceptance the operator asked for: in an open run, Tab into the detail pane of a failed stage, select the compiler-error line, copy it exactly, and come back to the same stage.

## Test and ship: one cycle

1. `pgrep -x cargo`: if another cargo is running, wait for it; never start a second build.
2. Targeted release tests:
   ```bash
   cargo test --release -p xai-grok-pager acpx
   ```
   The `acpx` filter covers the modal and `acpx::step_digest`. The error-line fix `d0eb35a3` is already on `main` and tested; it ships with this install.
3. Fix compile errors and failing tests with the smallest edit inside `detail.rs` or `acpx_modal.rs`. At most three fix rounds. When a fix changes a test rather than code, say why in the report. If still failing after three rounds, stop and report the failures verbatim.
4. Skip `cargo fmt` as in run2 (the files already fail `rustfmt --check` at HEAD).
5. Commit the packet, staging only `detail.rs` and `acpx_modal.rs` by path. Message ends with `Co-authored-by: claude-code sonnet-5.5`. Push to `main`.
6. Install: `scripts/install-fork.sh`.

## Do not touch

- `views/acpx_modal/run.rs`, `grok-acpx-acp/project.rs`, `ledger.rs`: other work.
- Running processes: never kill a slice run, build or session; PID only, never `pkill`/`killall`. No tmux.
- No new panes in this tab. No workflow runs, gate answers or `/flow` launches.

## Report

Write `task-scripts/acpx-disclosure-review/run3/integration-report.md`: edits applied, fixes with reasons, the test command and its result line, the commit hash, the install result. Commit and push it, then say "done" in this tab.
