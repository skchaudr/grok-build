# Brief: exit resume hint names the launcher command

Worktree: `~/repos/grok-build-wt/resume-hint`, branch `fix/resume-hint-agent` (pushed). Commit + push to it.
Commit trailer: `Co-authored-by: cursor-agent <cursor-agent@cursor.com>`
Subagents: 0. Small change.
Build cache: `export CARGO_TARGET_DIR=$HOME/repos/grok-build/target` (reuse; don't make a new target dir). Only run pager-crate tests, not the workspace.

## Problem
On quit the pager prints `Resume this session with:\n  grok --resume <id>` (`crates/codegen/xai-grok-pager/src/app/mod.rs`, `print_exit_resume_hint`, ~line 1316; also `print_relaunch_failure_hint` / `screen_mode_relaunch_resume_hint`).
Our wrapper launches ACP agents as `grok cursor`, `grok claude`, etc. (`~/.dsh/scripts/gb`), so the right hint for a Cursor session is `grok cursor --resume <id>`.

## Do
- Read an env var `GROK_RESUME_CMD` (e.g. `grok cursor`). When set and non-empty (trimmed), use it in place of the leading `grok` in every resume hint the pager prints (plain quit, `--minimal` variant, relaunch-failure fallback). Unset/empty → exactly today's output.
- Read it once at a sensible place (don't scatter `std::env::var` calls); keep the change small and in the file's style.
- Do NOT edit `~/.dsh/scripts/gb` (another agent owns it); I will add the export there.

## Eval first
Extend the existing hint tests in `app/mod.rs` (~line 2400, `sess-abc`) before coding: unset → unchanged; `grok cursor` → `grok cursor --resume sess-abc`; minimal variant; whitespace-only → default. Avoid racy process-env mutation in tests: pass the value as a parameter into the formatting function and test that.

## Verify
`cargo test -p xai-grok-pager <filter>` for the hint tests, plus `cargo check -p xai-grok-pager`. Paste the commands + results in `OBSERVATIONS.md` (committed). Stop and report; don't merge, don't touch Mini/Air.
