# Harness facts

One place for per-harness facts that agents keep rediscovering. Check here before researching; update here when you learn something. Every row names its source and whether it was **live**-tested, read from **code/schema**, or only **documented**.

Last updated 2026-10-26.

## Session logs

| Harness | Path |
|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` |
| Pi | `~/.pi/agent/sessions/**/*.jsonl` (the mini has almost no tool calls; the Air has real ones) |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl.zst` (`zstd -dc`), also `archived_sessions` |
| Cursor | `~/.cursor/projects/*/agent-transcripts/**/*.jsonl` |
| Grok | `~/.grok/sessions/<cwd>/<id>/updates.jsonl` (`rawInput`) |

## Per-call intent field (what the call is for)

Sampled 2026-10-26 from real logs on the mini.

| Harness | Field | Populated | Tools |
|---|---|---|---|
| Claude Code | `input.description` | ~78% of Bash; Agent/Monitor 100% | Bash, Agent, Monitor; not Edit/Write/Read |
| Cursor | `input.description` | 99.6% of Shell | Shell, Task, Subagent |
| Grok | `rawInput.description` | 100% | run_terminal_command |
| Codex | `justification` | ~1-2.5% | exec_command; reasoning `summary` items (~21%) are not tied to a call id |
| Pi | `task_name` | only with the pi-grok-bash extension | bash |

## Pi tool-call argument shapes

| Tool | Shape | Source | Status |
|---|---|---|---|
| `edit` | `{path, edits: [{oldText, newText}]}`; legacy top-level `{path, oldText, newText}` also accepted and normalized | Pi 1.4.0 `dist/core/tools/edit.js:5-30` (same in 1.3.0); legacy `dist/bundle/chunks/chunk-66G7ITPS.js:759`; grok-pi `pi-grok-adapter/src/tool_projection.rs:93-111` (also accepts `old_text`/`new_text`, `filePath`/`file_path`/`target_file`) | code/schema + real Air log (2026-07-06) |
| `write` | `{path, content}` | Pi `dist/core/tools/write.js:8-11`; adapter `tool_projection.rs:110` | code/schema + real Air log (2026-07-27) |
| `bash` | `{command, task_name?}` (`task_name` from pi-grok-bash) | adapter `tool_projection.rs:349, 402-413, 455`; `background_bash_bridge.rs:237-240` (`description` first, then `task_name`) | code + real Air log (2026-10-25) |

Pi paths are under `~/.pi/agent/install/releases/<ver>/node_modules/@earendil-works/pi-coding-agent/`.

## Pre-tool hooks: blocking a call

| Harness | Hook | Deny output | Reason reaches model? | Source | Status |
|---|---|---|---|---|---|
| Claude Code | `PreToolUse` | stdout `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"…"}}`, exit 0 | not proven in any repo | `~/.claude/hooks/guard-bash.sh`; `docs/traces/MAP.md` hooks row 5 (5 hook-denied calls on the mini, 2026-10-21) | **live** (denials fire) |
| Codex | `PreToolUse` (shell only) in `~/.codex/hooks.json` | stdout `{"decision":"deny","reason":"…"}`, exit 2 | not proven | `skchaudr/agent-harness-hooks` `adapters/grok_codex/entry.py:46-48`, `docs/02-adapter-grok-codex.md:45-52` | documented; offline test only (`scripts/test-hook.sh:51`) |
| Pi | extension `pi.on("tool_call")` | return `{ block: true, reason }` (undefined = allow) | not proven | `agent-harness-hooks` `adapters/pi/extension.template.ts:11-16`, `docs/04-adapter-pi.md:19-24`; Pi `docs/extensions.md` | documented |
| Cursor | `preToolUse` / `beforeShellExecution` in `~/.cursor/hooks.json` | **no deny path exists anywhere**; all local hooks reply `{"permission":"allow"}` (`xai-grok-traces/src/bin/grok-traces.rs:59-65`; Orca `cursor-hook.sh` cannot deny via stdout) | n/a | allow path: `MAP.md` (CLI works, app unverified) | allow **live** (CLI); deny unknown |
| Grok | none | grok-build has no PreToolUse | n/a | `~/.grok/hooks/*.json` uses UserPromptSubmit, PostToolUse, SessionStart, Stop, SessionEnd only | code |

Open: for every harness, one live session proving the deny **reason** reaches the model and the model retries.

## Pi workflow packages

_Pending: filled from the workflow-packages lookup (2026-10-26)._
