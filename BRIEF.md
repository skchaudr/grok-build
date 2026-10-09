# Brief: context usage never shows for external ACP agents

Worktree `~/repos/grok-build-wt/external-context-usage`, branch `fix/external-context-usage` (pushed). Commit + push often.
Trailer: `Co-authored-by: cursor-agent <cursor-agent@cursor.com>`. Subagents: 0.
Build: `export CARGO_TARGET_DIR=$HOME/repos/grok-build/target CARGO_INCREMENTAL=0 PROTOC=$HOME/.local/protoc-29.3/bin/protoc` (shared with one other agent). Check `df -h /` before building; khoj disk is tight. Test only the crates you touch.

## Symptom (user, daily)
With `--agent-cmd` agents (mainly Claude via `claude-code-acp`, also Cursor `cursor-agent acp`) the pager never shows context usage (e.g. `54K / 500K` in the header), so the user can't tell when to compact. DSH via `gk khoj` does show `0 / 1.1M`, so the pager can render it when it gets data.

## Investigate first (record in OBSERVATIONS.md before coding)
1. What the pager reads for the context indicator: `UsageUpdate` (`unstable_session_usage`), `token_usage`, compact vs boxed header (an earlier note says only compact chrome reads `token_usage` and the boxed header has no token slot — check whether that's still true and whether that alone hides it).
2. What each agent actually sends. Capture raw ACP traffic on khoj (e.g. a tiny tee wrapper as `--agent-cmd`, or the pager's debug log) for one short turn each of `claude-code-acp` (`env CLAUDE_CODE_EXECUTABLE=$(command -v claude) claude-code-acp`) and `cursor-agent acp`: do they send `usage_update` session updates, usage on the `session/prompt` response, or `_meta` token fields? What context window size, if any?
3. Where the claude-code-acp adapter source is (npm package on this machine) and whether it has a usage hook we're not reading.

## Do
Smallest change that makes the context indicator show for Claude (and Cursor if it sends anything) in the default header the user sees, mapping whatever the agent provides (prompt-response usage, `_meta`, etc.) into the pager's existing usage state. If an agent sends nothing usable, say so; do not invent numbers. If a window size isn't provided, show used tokens alone rather than a fake denominator. Native behaviour unchanged.

## Eval first, then verify
Failing unit tests first using the captured payload shapes. Then live on khoj with a release build: one Claude turn → header shows tokens. Record commands/results in OBSERVATIONS.md (append dated section). Don't touch ~/.grok/bin, Mini, Air. Don't merge. Report.
