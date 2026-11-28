# Run2 integration report

Applied with an exact-once OLD/NEW script (`check_edits.py` parser); no cargo before all three packets were in.

## Packet 6 — bare `/open` + `/flow` picker — `2d1887f9`
- Edits applied: 10 of 10 (`grok-acpx-acp/agent.rs`, `driver.rs`). Coordinator follow-ups: none listed.
- Fix: `agent::tests::available_commands_contents` asserted the old `<flow file>` hint; packet Edit 7 makes the argument optional, so the assertion now expects `[flow file] [task or JSON input]`.
- Test: `cargo test --release -p xai-grok-pager-bin --bin grok-acpx-acp` → `test result: ok. 87 passed; 0 failed`.

## Packet 3 — attempt digest — `9090cabc`
- Edits applied: 5 of 5 (`views/acpx_modal/detail.rs`). Coordinator follow-ups: none listed.
- Test: covered by the pager run below.

## Packet 2 — Flows ledger — `1f7fe9b1`
- Edits applied: 15 of 15 (`ledger.rs`, `row_extra` in `acpx_modal.rs`).
- Follow-ups done: `executed: None` added to the 8 remaining `RowExtra` literals in `ledger.rs` tests (compiler-listed); `Space` row added to `LEDGER_KEYS` in `acpx_modal.rs`. `RowExtra` literals in `acpx_modal.rs` needed none.
- Fix: `stalled_running_and_answered_waiting_same_flow_name_stay_individual` (existing test) started on `r-ans` instead of `r-next`: `set_runs` selected the then-waiting `r-ans`, and Edit 7 keeps the selection across `set_extras`. The test now calls `set_extras` before `set_runs`.
- Test: `cargo test --release -p xai-grok-pager acpx_modal` → `test result: ok. 118 passed; 0 failed`.

## Not done
- `cargo fmt` skipped: the packet files already fail `rustfmt --check` at HEAD (agent.rs 60 hunks, ledger.rs 17, detail.rs 21, driver.rs 3); formatting would mix unrelated churn into the packet commits, and rustfmt follows `mod` children into Sol's `run.rs`.
- The ten known failing `xai-grok-pager --lib` tests were not run (the filter `acpx_modal` does not select them).
- Fix rounds used: 2 (pager), 1 (bin); limit is 3.

## Install
`scripts/install-fork.sh`: release build 2m 38s, exit 0, checksums match here and on sab-air:
- grok-fork 99e8cd736747
- grok-traces 53d9dd665414
- grok-acpx-acp 2d2f2a74cdba
- grok-pi-acp 7a8554a40d7f
