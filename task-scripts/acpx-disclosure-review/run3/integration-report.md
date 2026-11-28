# Run3 integration report: packet 3b (detail-pane focus and linewise copy)

## Edits
16 of 16 applied, each matching exactly once (`views/acpx_modal/detail.rs` Edits 1–10, `views/acpx_modal.rs` Edits 11–16). No coordinator follow-ups.

## Fixes (3 rounds used, limit 3)
All three changed the packet's new test `wrapped_failure_line_copies_exact_source_on_second_row`, not code.
1. Compile errors: the test used `Outcome::Error` (no such variant; the enum is `Ok, TimedOut, Failed, Cancelled, Unknown`) and `ArtifactRef { sha256: None }` (the struct has `path, media_type, bytes`). Now `Outcome::Failed` and `media_type: "text/plain"`.
2. The test failed at `second_line_row > stderr_path_row + 1`. A temporary dump of `panel.lines` showed "second failure line" twice (row 7 in the Failure section, row 19 in stderr) and the stderr path at rows 16–18. The test's `position` found row 7. A diagnostic run printed the rows; the print was removed afterwards.
3. `position` became `rposition` so the test takes the stderr occurrence (row 19). The code under test was not changed.

## Test
`cargo test --release -p xai-grok-pager acpx` → `test result: ok. 220 passed; 0 failed; 1 ignored`.

## Commit
`92f80438` (`detail.rs` and `acpx_modal.rs` only), pushed to `main`. It ships with `d0eb35a3`.

## Install
`scripts/install-fork.sh`: release build 2m 36s, exit 0, checksums match here and on sab-air:
- grok-fork 3e36a47a8df9
- grok-traces 846a645a06b5
- grok-acpx-acp b02f883bcf3b
- grok-pi-acp f9d3af2716f7

## Not done
`cargo fmt` skipped as in run2. The acceptance flow (Tab into a failed stage's detail pane, select the compiler-error line, copy it, return to the same stage) was covered by tests only; I have not run it by hand in the installed build.
