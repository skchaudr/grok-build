# Live feed follow-ups (2026-11-08)

Repo: /Volumes/Mini-SSD/offload/repos/grok-build, branch gc/claude-parity (main == ab3d1656, the installed build). Work in place.

Context: grok-live's live feed (live Claude/Pi calls merged into one row per call; `crates/codegen/xai-grok-traces/src/merge.rs`, `crates/codegen/xai-grok-pager/src/acp/live_feed.rs`, `app/dispatch/standalone_traces.rs`) was fixed in c1b5dd81, 6e3182b4, ab3d1656 (read their messages). A previous agent was stopped mid-work and left **uncommitted changes** in `live_feed.rs`, `tracker.rs`, `merge.rs`, `scrollback/state/{mod,nav}.rs`, `app/dispatch/tests/live_traces.rs`. Review them first (`git diff`); keep what is right, finish or revert the rest.

## 1. Running marker one call behind (in progress in the uncommitted diff)
On screen: the previous, finished call kept `›` while the call actually running showed `◆`.

## 2. "◆ Thought for 0.0s" right after a tool call (in progress)
ab3d1656 folds a trailing empty thought after another thought; this one follows a tool call.

## 3. Hooks carry full tool-call content
Sab's ruling: tool calls come first and must be complete live from hooks alone; assistant text and thinking from the transcript are a bonus and may lag.
`grok-traces hook` (`crates/codegen/xai-grok-traces/src/hook.rs`, `FIELD_CHARS = 512`, `detail()`) keeps only a 512-char command/path and drops the rest of `tool_input` (e.g. Claude's `description`) and PostToolUse `tool_response`. Keep the description and a bounded slice of the result (also Pi's `tool_execution_end` result; `~/.pi/agent/extensions/traces.ts` is read-only). Schema bump is fine (update every test asserting it). Cap sizes; receipts are pruned at 100k rows; state the cap. The live feed shows description and result the moment the receipt lands; the transcript only adds text/thinking. Test a call whose transcript line never arrives.

## Rules
- Root cause each item with file:line; regression test for each.
- Tests run visibly in herdr pane w3E:p4 (tab "tests"): `herdr pane run w3E:p4 "clear; cd /Volumes/Mini-SSD/offload/repos/grok-build && nice -n 10 cargo test --release -p xai-grok-traces && nice -n 10 cargo test --release -p xai-grok-pager -- traces live follow_keeps && echo TESTS-DONE || echo RUN-FAILED"`, block with `herdr pane wait-output w3E:p4 --regex 'TESTS-DONE|RUN-FAILED' --timeout 3600000`, read with `herdr pane read w3E:p4 --source recent`. Use --release. One cycle at a time. Known unrelated: `tests/remote.rs` fake-ssh tests can time out under load (rerun); `insert_block_before_rejects_an_already_committed_anchor` fails only in release.
- Verify on the real screen: run `scripts/install-fork.sh` in w3E:p4, then run `target/release/xai-grok-pager traces` in your own herdr tab (`herdr tab create --workspace w3E --cwd <repo> --label live-fix`), Enter on a running Claude session, `v`, read with `herdr pane read`. Close your tab when done.
- Do not install binaries, touch ~/.grok/bin, or edit ~/.claude or ~/.pi.
- Commit on gc/claude-parity and push; every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Report tightly: root cause per item, fix, tests read from the pane, what you saw on screen, commit hashes. Flag anything unverified.
