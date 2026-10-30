# SPEC: agent fleet: see every agent live, control any of it

Draft by claude-code opus-5.5, 2026-10-30. Every "today" line was read from disk this session. Lines marked UNVERIFIED have not been.

## Product

One tool you run agents through and watch them in. It shows every agent on sab-mini and sab-air: the ones you start, and every subagent they start. Each agent appears the moment its harness fires a hook, under its parent, labelled with its model. You can follow any agent's tool calls live, then stop, kill or steer it from the same screen. Afterwards you can open its full transcript.

Done means: a 40-agent fan-out shows 40 parented, model-labelled rows as each agent starts, and any one of them can be stopped from the tool.

## Parts, grounded

| Part | Today (path) | Gap |
|---|---|---|
| Event source | Claude hooks in `~/.claude/settings.json` → `~/.grok/bin/grok-traces hook claude` for PreToolUse, PostToolUse, PostToolUseFailure, SessionStart, SessionEnd, Stop, SubagentStop, UserPromptSubmit | `SubagentStart` goes only to `~/.grok/bin/traces-capture.sh`, never to grok-traces |
| Ingestion | `crates/codegen/xai-grok-traces/src/hook.rs`: `extract` keeps event, session, agent_id, tool, a 512-char `detail` (l.44-60); `ingest_inner` writes `trace_receipts` (l.129), then re-indexes the parent transcript file (l.131) | Drops `tool_input` and `tool_response`; never indexes a subagent's own file |
| Store | `~/.grok/traces/traces.sqlite`: `trace_receipts`, `trace_sessions` (parent_native, model), `trace_events`, `trace_spawn` | Children exist only after a scan or a folder probe |
| Wake | `xai-grok-traces/src/wake.rs`: hook → `$GROK_HOME/traces/wake.sock` | Viewer wakes, then re-reads files |
| Live follow | `xai-grok-traces/src/follow.rs`: subagents found by `probe_subagents` (folder guessing, l.377) plus `idx.children` | Missed every Workflow agent (`subagents/workflows/wf_<id>/`) |
| Viewers | `grok-fork traces` (pager `views/traces_modal.rs`, `traces_follow.rs`); trace-deck (`/Volumes/Mini-SSD/offload/repos/trace-deck`, `~/.local/bin/trace-deck`) | Never saw one subagent during today's swarms |
| Control | Herdr, Rex | UNVERIFIED: what each exposes to launch, stop, send input to a pane |

## What Claude's hooks carry (25,970 payloads in `~/.grok/traces/capture/claude/`)

| Hook | Fields |
|---|---|
| SubagentStart | agent_id, agent_type, session_id (parent), transcript_path (parent) |
| PreToolUse in a subagent | agent_id, agent_type, tool_input (full), tool_use_id, transcript_path (parent) |
| PostToolUse | tool_response (full), duration_ms, tool_use_id |
| SubagentStop | agent_id, agent_transcript_path (child file) |

No payload carries the model. Options: read the child transcript (path known only at SubagentStop, or as `<parent stem>/subagents/**/agent-<agent_id>.jsonl`), or Claude Code telemetry (UNVERIFIED).

## Open

- Herdr / Rex control surface (next to ground).
- Other harnesses' hook payloads (Codex, Pi, Cursor, Grok): captures exist under `~/.grok/traces/capture/<source>/`, not yet read.
- trace-deck's data path: which `grok-traces` commands it calls (UNVERIFIED).
