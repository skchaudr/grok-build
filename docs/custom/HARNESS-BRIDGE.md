# Grok Build mechanisms for every agent: feasibility

Read-only design study, 2026-10-21. Nothing built. (U) = unverified.

## Key findings

- **No MCP server ships.** `grok mcp` only manages config entries (`xai-grok-pager/src/mcp_cmd.rs:8-72`), and `xai-grok-mcp` is client-only.
- **Memory capture is bound to `SessionActor`** and its Grok-model extractor (`memory_capture.rs:415,493,946`), so external input can't feed it.
- **Free write path into memory v2:** any `*.md` dropped in `<scope_dir>/observations/_inbox/` is picked up by Dream as a manual note (`xai-grok-memory/src/v2_consolidation.rs:713-790`). Scope comes from the normalized `origin` remote (`storage.rs:714-759`).
- **Session search** is pager-only. Its source is a fn-pointer bound to the JSONL store (`shell/session/storage/search.rs:20-26`).
- **ACP choke point:** `leader/server.rs:2826` (agent→client) and `:2721` (client→agent). It's a large upstream-owned file, and it sees bridged turns only.
- **Hooks don't fire for bridged agents (U).** Dispatch lives in `SessionActor` (`hook_dispatch.rs:255,441`).
- **`/learn` can be shadowed by a User-scope skill** (`skills.rs:452`), but `collect_sessions.py:65-130` moves any user copy of `learn-traces` that differs from the bundled one into `learn/trash/`. Never copy or edit bundled files.
- **Privacy:** `/learn`'s map agents are Grok models, so exported Claude and Cursor text goes to xAI. `redact()` only catches credentials, so any exporter needs a cwd/source allowlist.

## Harness support

| Harness | Hooks | MCP | Skills |
|---|---|---|---|
| Claude Code | yes | yes | yes |
| Codex | yes (`hooks.json`) | yes (`[mcp_servers]`) | yes |
| Cursor | yes (`hooks.json`) | yes (`~/.cursor/mcp.json`) | yes |
| Pi | extension events | none found (U) | not checked |

## Plan

| Phase | Work | Effort |
|---|---|---|
| 1 | A `grok-memory` skill in `~/.agents/skills`: read MEMORY.md and topics, write notes to `_inbox`. A `grok-traces` skill: list/events/raw. A SessionStart hook injecting the memory index (top 5, about 500 chars each) via `hooks/install.sh`. No Rust. | S, about a day |
| 2 | A `learn-all` wrapper skill plus an exporter. The exporter turns traces sessions into a synthetic `GROK_HOME` (`sessions/<enc_cwd>/<sid>/{summary.json,chat_history.jsonl}`, with tool names mapped to Grok's), then runs the bundled `collect_sessions.py --grok-home`. Fill in the Cursor `scan_cursor` stub (`foreign-sessions/src/lib.rs:101,109`) using `traces/cursor.rs`. | M, then S |
| 3 | A `grok memory search/get/remember` CLI wrapping `MemoryBackendImpl` (hybrid search; needs auth for embeddings, text fallback (U)). Then an MCP server as a thin wrapper over it plus the traces CLI. Add FTS search to traces. | M, then M |
| 4 (only if 1-3 leave a gap) | A traces-fed capture extractor; leader-tap hooks for bridged agents. | L each, high upstream conflict |

**Verdict:** integrate through traces, skills, hooks and a CLI, not the ACP bridge. Traces already sees every harness, and the bridge sees only bridged turns, would double-capture against traces, and lives in upstream-owned code. Phases 1 and 2 cover most of the value with almost no merge risk.

**Runtime checks needed:**
- Do Codex and Cursor accept context returned by SessionStart hooks?
- Can Pi host tools?
- Do Grok hooks and permission rules apply to bridged sessions?
