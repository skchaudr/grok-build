# Observations — gb-session-resume (Task A)

## 2026-10-12 Worker

- pi-acp already exposed `listSessions` / `loadSession`; Grok pager still used `x.ai/session/list` for external agents until `SessionFlags.external_agent` routes `FetchSessionList` through ACP `session/list` (`64c5abeb`).
- `xai-acp-lib`: `ListSessions` request/response + gateway forward.
- Tests: `fetch_session_list_external_agent_uses_acp_session_list`, `agent_acp_list_maps_*`, `test_external_acp_session_list` (`fake-pi-sessions` fixture).
- pi-acp worktree: `loadSession` emits `usage_update` from `getSessionStats().tokens.total` when > 0 (`Pi-Coding-Agent` `8fd2f2a`).
- Build environment: disk hit 100% twice; freed with `cargo clean` on worktree + main `target/` (~40GB). Re-check `df -h /` before large builds (brief: stop if <15GB free).
- Not merged to main.

WORKER DONE 64c5abeb
