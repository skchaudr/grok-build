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

Both installed on the Air and on the mini (mini: pi-workflows 0.17.7, pi-extensible-workflows 6.2.2, `ls ~/.pi/agent/npm/node_modules/`, checked 2026-10-26) under `~/.pi/agent/npm/node_modules/`. Air `~/.pi/agent/settings.json` excludes pi-extensible-workflows' `-dist/src/index.js` (the entry that registers `workflow`).

| | pi-extensible-workflows (6.2.2 on Air; 6.3.0 on npm) | @osolmaz/pi-workflows (0.17.7) |
|---|---|---|
| Workflow format | inline JS: `workflow` tool with `name` + `script` or `scriptPath` (`README.md:21-35`) | `.pi/workflows/<name>.workflow.ts` (project) or `~/.pi/agent/workflows/` (global); `defineWorkflow({name, startAt, nodes, edges})` + `agent({prompt, expectedOutput})` (`README.md:94-115`, `examples/workflows/echo.workflow.ts`) |
| Launch | tool `workflow` (background unless `foreground: true`); slash `/workflow`, `/scout`, `/oracle`, `/review`, `/parallel-review`, `/review-loop`, `/deep-research` (`dist/src/host-navigator.js:1136`) | `/workflow <name-or-path> [task or --input-json]` (`src/extension/index.ts:359`); `workflow` tool with actions list/start/status/pause/resume/cancel/answer/update/submit (`index.ts:542`); `/workflow-channel`, `/resource-manager`, `/piw` |
| Tools registered | `workflow`, `workflow_status`, `workflow_retry`, `workflow_resume`, `workflow_respond`, `workflow_stop`, `workflow_catalog`, `workflow_result`, `agent`, `get_subagent_result`, `steer_subagent`, `subagents_*` (`dist/src/index.js`, `dist/src/host.js:32`) | `workflow` |
| Progress / state | `~/.pi/workflows/projects/<slug-hash>/sessions/<sessionId>/runs/<runId>/{state,journal,snapshot,result,summary}.json`, `workflow.js`, `evidence/` (`dist/src/index.js:428-437, 1581`); widget messages `customType: "workflow"` (`dist/src/host.js:147`) | `~/.pi/agent/workflows/state.sqlite` via a global server (`README.md:79, 236-241`); viewers `pi-workflows view`, `runs`, `view --once`; progress `session-run-adapter.ts:176-192` |

Sources: package files on the Air (read 2026-10-26). Status: pi-workflows launched live on the mini, see the smoke section below; pi-extensible-workflows not launched.

**Duplicate `workflow` tool.** Both packages register `workflow` unconditionally (`dist/src/index.js:13773`; `src/extension/index.ts:542`). Pi's `detectExtensionConflicts` (`resource-loader.js:974-987`, read from Pi 1.2.0 on the mini) records a diagnostic and does not throw. The `pi --mode rpc` exit was observed live on 2026-10-13 with pi-extensible-workflows 6.2.0 (`OBSERVATIONS.md:587`, workaround `PI_ACP_PI_COMMAND` pointing at a `pi -ne` wrapper); not re-run on Pi 1.4.0. grok-pi's gate: `pi -ne --mode rpc --extension <bundle>` exits 0 (grok-pi `AGENTS.md:151`).

## Pi DAG + subagent smoke (2026-10-26, mini)

Pi 1.4.0, pi-subagents 0.82.1, pi-workflows 0.17.7, model cliproxy/gemini-3.8-flash-high. Details: `docs/traces/SMOKE-pi-dag.md`.

- Mini `settings.json` already excludes pi-extensible-workflows' `-dist/src/index.js`; `pi -p "reply with the single word PONG"` printed `PONG`, stderr only `Warning: No models match pattern "openai-codex/gpt-5.3-codex-spark"`, no duplicate-tool diagnostic. Not tested without the exclusion.
- pi-workflows failed on the mini until `@earendil-works/pi-ai`, `pi-coding-agent`, `pi-tui` were resolvable from `~/.pi/agent/npm/node_modules/`: `/workflow smoke` raised `ERR_MODULE_NOT_FOUND ... '@earendil-works/pi-ai' imported from .../@osolmaz/pi-workflows/src/builtins/pi-agent-group.ts` (`notify` event, rpc log). Peer deps are not auto-installed there (Air has them). Fix applied: three symlinks into `~/.pi/agent/install/releases/1.4.0/node_modules/@earendil-works/`, then `pi-workflows server stop`.
- `pi -p "/workflow smoke"` (before the symlink fix) exits 0 with empty stdout and creates no run (`pi-workflows runs`: "No workflow runs found."). Not retried after the fix. `pi --mode rpc` + `{"type":"prompt","message":"/workflow smoke"}` drives the run to completion with no TUI.
- `pi-workflows` is not on PATH; use `node ~/.pi/agent/npm/node_modules/@osolmaz/pi-workflows/dist/viewer/cli.js runs|view <id> --once`.
- An agent node with `allowedTools: ["subagent"]` can call `subagent` (async), poll `status`, and `workflow submit` the child's text; the compute node then saw it. Run `20261027T045847071Z-smoke-0bab5a31`: A and B ok, output `{"ok":true}`. Child run `a95d5e16-e6fe-4487-95ee-738c5f3b1501` ran in its own session file (`pi-subagent-session-*/<id>/run-0/session.jsonl`, session id `01a17bb9-c7ae-...` vs origin `01a17bb9-9ae6-...`), result `PONG`.
- Custom agent `tools: codemode, read` works in an async child: session shows a `codemode` toolCall (`tools.read({path:"README.md"})`) and the child returned `# smoke` (run `d48ed040-7a83-4fd4-8da2-49be24be90d7`).
- **Correction to `tools: inherit`:** in a project agent file `.pi/agents/*.md`, `tools: inherit` does NOT give Pi's builtins. The async child (run `71f80371-7877-4a90-93e5-1ba70784f581`) had only `contact_supervisor` and reported the `workflow` tool missing. `inherit` is valid only as a builtin override value (`docs/agents.md:246`, `src/agents/agents.js:1183`); the custom-agent path reads `frontmatter.tools` as a literal list (`agents.js:1922`). Omit `tools` to get builtins plus ambient extensions in an async child.
- A pi-subagents async child can start its own pi-workflows run: with `tools` omitted, child `5b647ca0-b884-43ef-bb59-2be21679cd21` called `workflow list`, started `echo`; `pi-workflows runs` shows run `20261027T050605717Z-echo-662e1b6c` with `originSessionId` `01a17bbe-4416-73a2-bb98-f6e4d9990503` = the child's session id (session header). Output `{"reply":"PONG"}` (sqlite `node_attempts`/`blobs`, queried by the child).
- A killed parent does not stop a detached async child (runner pid 1293 stayed alive, blocked on `contact_supervisor`); kill the runner pid from `status.json`.
