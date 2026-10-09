# Brief: external ACP agents get their turns killed at 120s

Worktree `~/repos/grok-build-wt/external-prompt-ack`, branch `fix/external-prompt-ack` (pushed). Commit + push often.
Trailer: `Co-authored-by: cursor-agent <cursor-agent@cursor.com>`. Subagents: 0.
Build: `export CARGO_TARGET_DIR=$HOME/repos/grok-build/target CARGO_INCREMENTAL=0 PROTOC=$HOME/.local/protoc-29.3/bin/protoc` (shared with one other agent). Check `df -h /` before building; khoj disk is tight. Test only `-p xai-grok-pager` filters.

## Symptom (user, daily)
In the pager with `--agent-cmd` agents (Claude via claude-code-acp, Cursor via `cursor-agent acp`), turns end with
"Turn cancelled." and "The agent did not accept your prompt within 120s. The turn was stopped; send the prompt again to retry."

## Root cause (traced, verify it)
`crates/codegen/xai-grok-pager/src/app/prompt_ack.rs`: a sent prompt arms a `PromptAckWatch`; it is only disarmed by
`x.ai/queue/changed` naming the prompt id, a `session/update` whose `NotificationMeta.prompt_id` names it
(`app/acp_handler/mod.rs` `ack_prompt_from_update`), or turn end. Those are Grok-native extensions. External ACP agents
never stamp a prompt id and never send `x.ai/queue/changed`, so any turn longer than 120s is aborted by
`app/dispatch/prompt_ack.rs` `reconcile_overdue_prompt_acks` even while the agent is streaming. Headless has the same path (`headless/prompt_ack.rs`).

## Do
For sessions backed by an external agent (the pager knows: `has_agent_cmd` / `--agent-cmd`, also leader-hosted external sessions), treat the first live non-replay `session/update` for that session while a watch is armed as the acknowledgment (or skip arming entirely if that's cleaner and you can justify it). Native behaviour must stay byte-identical. Cover TUI and headless. Make sure a leader client attached to an external session (dashboard open of another client's external session) gets the same treatment.

## Eval first
Failing tests first: external session + unstamped session/update → watch disarmed, no abort past 120s (inject clock); native session + unstamped update → unchanged behaviour.

## Verify
Tests pass. Live on khoj: `GROK_PROMPT_ACK_TIMEOUT_SECS=10` with a release build of `xai-grok-pager-bin` and `--agent-cmd 'cursor-agent acp'`, prompt `Run sleep 30 in the shell, then reply done` → must complete without the timeout notice; before the fix it should fail at 10s (prove both). Tear down. Record commands/results in OBSERVATIONS.md (append a dated section). Don't touch ~/.grok/bin, Mini, Air. Don't merge. Report.
