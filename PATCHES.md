# ACP External Agent Subprocess Wire

- `crates/codegen/xai-grok-pager/src/acp/spawn.rs`
  - Adds `GROK_ACP_AGENT_CMD` external-agent mode in `spawn_grok_shell`.
  - Shlex-splits the command string into argv and spawns the child with piped stdin/stdout/stderr.
  - Reuses the existing `AcpClientChannel` + `ClientSideConnection` + `AcpGatewayReceiver` ACP client wiring for NDJSON stdio.
  - Keeps env-unset behavior on the existing in-process GrokShell path.
  - Kills the external child on guard cancellation/drop via the existing spawned-thread lifecycle.
  - Captures external-agent stderr at `~/.grok/logs/acp-external-agent-stderr.log`, falling back to `$TMPDIR/grok-acp-external-agent-stderr.log` if the Grok log directory cannot be created.

