# Integrate the run2 ACPX slice returns

Repository: `/Volumes/Mini-SSD/offload/repos/grok-build`, branch `main`. Work in this checkout; no worktree or branch.

## What you integrate

Each code return is an edit list: `## Edit N` sections, each with `Path:`, an `OLD:` block and a `NEW:` block (fenced with `~~~`). OLD quotes current source verbatim. Apply each edit with your Edit tool: OLD must match exactly once in that file. If it matches zero times or more than once, stop that packet and report it; do not guess.

Check matches first:

```bash
python3 -I task-scripts/acpx-disclosure-review/check_edits.py . <edits.md>
```

Returns live under `task-scripts/acpx-disclosure-review/run2/grok-build/task-scripts/acpx-disclosure-returns/`.

| Order | Packet | Edits file | Files it changes | Coordinator follow-ups |
|---|---|---|---|---|
| 1 | 6 bare `/open` + `/flow` picker | `bare-command-picker.edits.md` (10 edits, all match once) | `grok-acpx-acp/agent.rs`, `driver.rs` | none listed |
| 2 | 3 attempt digest | `attempt-detail.edits.md` (5 edits, all match once) | `views/acpx_modal/detail.rs` | none listed |
| 3 | 2 Flows ledger | `finished-run-ledger.edits.md` (not back yet; the lead sends you a message when it is reviewed) | `views/acpx_modal/ledger.rs`, `row_extra` in `views/acpx_modal.rs` | add `executed: None` to every `RowExtra` literal its edits did not update; add the Space row to `LEDGER_KEYS` in `acpx_modal.rs`; anything in its `## Coordinator follow-ups` section |

The packet specs are in `task-scripts/acpx-disclosure-slices.py` at commit `d00448d6` (`git show d00448d6:task-scripts/acpx-disclosure-slices.py`); later commits changed packet 2's spec for a future re-render, not for this return.

Start packets 6 and 3 now. Apply edits only, no cargo yet.

## Test and ship: one cycle for the batch

After all three packets are applied:

1. `pgrep -x cargo` — if another cargo is running (Sol may be building), wait for it; do not start a second build.
2. `cargo fmt` on the files you changed.
3. Targeted release tests, in this checkout:
   ```bash
   cargo test --release -p xai-grok-pager acpx_modal
   cargo test --release -p xai-grok-pager-bin --bin grok-acpx-acp
   ```
4. Fix compile errors and failing tests with the smallest edit inside the packet's own files. At most three fix rounds; if still failing, stop and report the failures verbatim.
5. These ten `xai-grok-pager --lib` tests already fail on `main` and are not yours: `fullscreen_child_open_focuses_scrollback_shortcuts`, `snapshot_full_width_vim`, `snapshot_half_width_vim`, `limited_color_output_is_stable`, `human_mixed_fixture_is_exact`, `json_contract_is_structural_stable_ordered_and_ansi_free`, `json_empty_fixture_pins_null_policy`, `insert_block_before_rejects_an_already_committed_anchor`, `render_footer_multiline_empty_create_uses_shift_or_alt_enter`, `render_footer_multiline_mode_send_uses_shift_or_alt_enter`.
6. Commit each packet separately, staging only that packet's files by path (others may have uncommitted work in this checkout). Message ends with `Co-authored-by: claude-code sonnet-5.5`. Push to `main`.
7. Install: `scripts/install-fork.sh` (one release build, installs on this Mac and sab-air, checks checksums).

## Do not touch

- `views/acpx_modal/run.rs` and `grok-acpx-acp/project.rs`: Sol (GPT Pi) is integrating the timeline pilot and fixing F11 there.
- Running processes: never kill a slice run, build or session; kill only by PID, never `pkill`/`killall`. No tmux.
- No workflow runs, gate answers or `/flow` launches.

## Report

Write `task-scripts/acpx-disclosure-review/run2/integration-report.md`: per packet, edits applied, follow-ups done, test command and its result line, commit hash; then the install result. Commit and push it, then say "done" in this tab.
