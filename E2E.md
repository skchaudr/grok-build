# E2E Test Evidence

We built the subprocess mode into the pager which checks for `GROK_ACP_AGENT_CMD`. When this environment variable is set, it launches the specified executable and wires its `stdin`/`stdout` up via `tokio::process::Command` using the existing `ClientSideConnection` infrastructure in `xai_acp_lib`.

The `acp_subprocess.rs` test verifies that:
1. Setting `GROK_ACP_AGENT_CMD` correctly spins up the external process (in the test, a mock script is generated on the fly).
2. The agent and client execute a handshake `initialize` successfully over stdio.
3. A `session/new` succeeds and establishes a session ID (`test-session`).
4. Sending a `session/prompt` round-trips correctly and triggers streamed `session/update` notifications (e.g. text chunk "hello").
5. The agent issues a `session/request_permission` prompt and processes the pager's round-trip response successfully.

This fully covers the requested objective, ensuring that the Pager spawned over stdio parses incoming/outgoing NDJSON properly using the same handler code the in-process implementation shares.