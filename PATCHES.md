# Patches

## `crates/codegen/xai-grok-pager/src/acp/spawn.rs`
- **Purpose**: Added a subprocess mode via the `GROK_ACP_AGENT_CMD` environment variable. Added a `NewlineFramedWrite` adapter to ensure that outgoing JSON-RPC requests to the subprocess are correctly framed with newlines (NDJSON), as required by `agent_client_protocol::ClientSideConnection`. The stdio streams are then routed into the same `acp_channels()` logic.
- **Anchor**: "Simplified to only support GrokShell (in-process) mode. Subprocess and remote modes can be added later if needed."

## `crates/codegen/xai-grok-pager/Cargo.toml`
- **Purpose**: Registered the `acp_subprocess` integration test to ensure Cargo discovers and runs it.

## `crates/codegen/xai-grok-pager/tests/acp_subprocess.rs`
- **Purpose**: Added an integration test using a mock ACP agent fixture (a small inline Python script). This mock answers `initialize` and `session/new`, emits a canned `session/update` notification, sends a `session/request_permission`, and consumes the client's decision over the stdio interface.

Total files touched: 3.