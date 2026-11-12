# Prime ACP in the Grok Build TUI: rendering review and gaps

Tested 2026-11-12 on `sab-mini`, in a Rex split beside the live session.

- Pager: `~/.grok/bin/grok-fork`, `grok 1.0.45 (4c3ace53ac8e)`.
- Agent: `prime-agent 0.10.2`, launched as `prime-agent --mode acp`.
- Runs: `--no-leader --trust --fullscreen --no-alt-screen`, once with `--always-approve` and once without; plan mode and normal mode both probed.
- Working model: `deepseek/deepseek-v4-flash`, selected through the pager's `/model` after the ACP default failed. A native `prime-agent --provider deepseek --model deepseek-v4-flash` pane was the rendering baseline.
- Captures and pager debug logs were read from `/tmp/prime-tui-live/`; the findings below name the exact log facts.

Reproduction:

```sh
cd /tmp/prime-tui-live
~/.grok/bin/grok-fork --cwd /tmp/prime-tui-live --trust --no-leader \
  --fullscreen --no-alt-screen --debug-file /tmp/prime-tui-live/pager-debug.log \
  --agent-cmd 'prime-agent --mode acp --provider deepseek --model deepseek-v4-flash'
```

`--provider/--model` bypasses the ACP default-model problem so the turn can run; the pager's `/model` picker was the other path used. For the no-approval probe, drop `--always-approve` and use Normal mode.

## How the path works

1. The pager starts `prime-agent --mode acp` as a stdio ACP process. `initialize` returns `agentInfo {name: "prime-agent", title: "Prime Agent", version: "0.10.2"}`, `loadSession: false`, `sessionCapabilities: {close: {}}`, image/embedded-context prompt support, and no auth methods.
2. The pager sends `session/new` with the cwd, its MCP server list, and `_meta {yoloMode, autoMode, sessionId}`. Prime returns a `sessionId` plus two `configOptions`: `model` (137 select options; values are JSON-encoded `["provider","model"]` strings) and `thought_level` (`high`/`xhigh`).
3. Rendering is standard ACP: `agent_message_chunk` and `agent_thought_chunk` become markdown and thought blocks; `tool_call` / `tool_call_update` become scrollback tool cards; the `session/prompt` response ends the turn with `Worked for N`.
4. In these sessions Prime had one visible ACP tool, `python_repl`, sent as `tool_call` with `title: "Python cell"`, `kind: "execute"`, `rawInput.code`, and a text `content` result. There are no `plan`, `usage_update`, `current_mode_update`, `available_commands_update`, subagent, goal, or permission updates in any run.
5. `config_option_update` is used for model changes; the pager rebuilds its synthesized model catalog and footer from it.
6. The pager has special menus/feeds for Claude and Pi, but no Prime profile. Prime is handled as a generic external agent, so every Grok-only affordance stays visible.

## What works

- Launch, `session/new`, and the turn loop are stable. Markdown streams correctly, including bullets, fenced code, and tables.
- Thought blocks render (`Thought for 0.2s`, expandable) and tool cards show the Python cell output.
- `/model` lists Prime's 137 models by name, selection sends `session/set_config_option`, the footer updates, and the next turn uses the selected model.
- Cancel works: Ctrl+C sends a cancel, the tool becomes `Cancelled Python cell`, and the turn ends `Turn cancelled in 8.3s`.
- A failed turn surfaces as a scrollback error block with Prime's error text.
- `session/close` is advertised, so the ordinary quit path is clean.

## Gaps

### 1. The Python cell's code is never shown (highest visual impact)

The collapsed card is `Run Python cell`; the block viewer shows only the result text. There is no code, no first-line summary, and every cell has the same title, so two cells are indistinguishable in the transcript.

Native Prime in the same turn shows either a derived bash card (`✓ bash · echo hello-native · ↑2 ↓2 lines · 15ms`) or a Python-cell card with details. The pager has no equivalent.

Cause: `kind: "execute"` lands in the `ToolKind::Execute` arm at `crates/codegen/xai-grok-pager/src/acp/tracker.rs:2149`. That arm resolves the header from `raw_input.command`; Prime sends `raw_input.code`, so the fallback is the title `Python cell`. The existing `EvalToolCallBlock` (`scrollback/blocks/tool/eval.rs`) already renders language-highlighted code plus output, but its guard (`raw_input.variant == "Eval"`) is in the catch-all `_` arm, after the `Execute` arm, so it cannot catch a Prime call as-is.

Fix, either side:

- Prime side: add `variant: "Eval"`, `language: "python"` (and optionally a derived `command` for the simple one-shell-call case) to the `python_repl` tool call's raw input.
- Pager side: before the `match tc.kind`, recognise `rawInput.code` + `title == "Python cell"` and build an `EvalToolCallBlock` (or a small `PythonCell` block) with the code and content text. If the cell has a single `bash("...")`/`Bash(...)` call, use that command as the header, matching native Prime's summary.

### 2. Prime's ACP session starts on the wrong model

`~/.prime/agent/settings.json` has `defaultProvider = "deepseek"` and `defaultModel = "deepseek-v4-flash"`, and a fresh `prime-agent -p` answers with it. `prime-agent --mode acp` instead reports `currentValue = ["opencode-go","deepseek-v4-flash"]` — the first enabled model — and the first prompt fails with `400 Upstream request failed: Model is unavailable`. The user must open `/model` and pick a working model before the first turn.

The model list also orders providers alphabetically, with the current model near row 100, so the recovery path is a long scroll or a filter.

Fix (Prime side): honour `defaultProvider`/`defaultModel` in `session/new`, or at least order the current/default model first. The pager cannot fix this without inventing a default.

### 3. No resume, no session list, no session identity

Prime advertises `loadSession: false` and only `sessionCapabilities.close`. Direct probes return `Method not found` for `session/load` and `session/list`. Consequences:

- `grok-fork --continue` prints `No session found for current directory`.
- `/resume` opens the picker and shows `No matches`; the background `session/list` fetch fails.
- The dashboard lists Grok sessions only, not the live Prime session.
- `gb -r <prime-id>` cannot find ACP sessions at all: `scripts/hub/gb` matches Prime only through `~/.prime/agent/sessions/*.info-cache.json`, and ACP sessions write `*.jsonl` + `*.window-cache.json` but no info cache.

Prime already persists the full conversation in its JSONL session store. A real fix needs `session/list`/`session/load` (mapping the store), or an adapter that exposes the store to the pager. `_meta.sessionId` from the pager is ignored: the pager offered `01a1d1cd-3781...`, Prime created `01a1d1cd-3751...` and returned its own id, so the two stores cannot correlate.

### 4. Permission modes are cosmetic; Prime never asks

With `--always-approve` off, Shift+Tab switched the footer to Normal, and a prompt that created `/tmp/prime-tui-live/permission2.txt` ran with no approval UI. A raw ACP probe with `session/new._meta.yoloMode = false` also produced no `session/request_permission`. Prime has no `request_permission` string in its binary.

The pager's permission modal, always-approve toggle, and Normal mode therefore do not gate Prime tool calls. This is a Prime ACP gap first; the pager cannot enforce what the agent never asks.

### 5. Plan mode says it switched, but `session/set_mode` fails

Shift+Tab to Plan shows `Switched to mode: Plan` and the pane still writes files. The debug log has `session/set_mode` -> `"Method not found"`, and `SetSessionModeFailed` (`app/dispatch/task_result.rs:1009`) only clears the optimistic pending state, with no user-visible error. The toast already claimed success.

Fix: Prime either implements `session/set_mode` or advertises no modes; the pager should hide Plan/Always-approve when the agent has no mode support, and the failure path should show a toast instead of silently reverting.

### 6. `thought_level` is advertised and ignored

Prime sends a standard `thought_level` config option (`high` / `xhigh`), and `session/set_config_option {configId: "thought_level", value: "high"}` works. But:

- `/effort high` answers `current model does not support reasoning effort`.
- The footer is `DeepSeek V4 Flash (deepseek)`, while native Prime shows `deepseek-v4-flash:xhigh`.
- `ModelState::models_or_config_option` (`acp/model_state.rs:437`) only reads `SessionConfigOptionCategory::Model`, and `/effort` reads model meta, so the standard `ThoughtLevel` option never reaches the UI.

Fix (pager): store the `thought_level` option beside the model option, show it in the footer, and make `/effort` send `session/set_config_option` with `configId = thought_level`. This is a small, well-scoped change.

### 7. No usage or context bar

Prime sends no `usage_update` and no total-token meta, so the pager's context bar and token accounting stay empty; `/usage` and `/context` are stubs (`/context is not supported by prime-agent`). Native Prime shows `12k (1%)` and `$0.00` in the same session.

Prime's session JSONL already records `usage {input, output, cacheRead, cacheWrite, totalTokens, cost}` per assistant message. The ACP server can send `usage_update` with `used`/`size` after each turn, and optionally cost in `_meta`; the pager already handles both (`acp_handler/mod.rs:253`, `views/context_bar.rs`).

### 8. Prime's own commands are blocked by the generic stubs

Prime intercepts `/compact` when it receives it as prompt text (the session records a `session_slash_command` custom message and the turn ends silently). But Prime sends no `available_commands_update`, so the pager's registry does not know about it. `/compact` is in `EXTERNAL_COMMON_STUB_COMMANDS` (`slash/external_common.rs:19`) and answers `/compact is not supported by prime-agent` without ever reaching the agent.

The same stub layer answers `/usage`, `/context`, and `/login`; `/model` works because it is a pager builtin.

Fix: Prime advertises the session commands it actually intercepts (at least `compact`) via `available_commands_update`, and the pager frees advertised names as it already does for external agents. A `prime_menu.rs` profile can additionally forward `/compact` as text and hide `/usage`/`/context` instead of offering dead stubs.

### 9. Grok-only panels fail with raw ACP errors

`/plugins` opened the Extensions modal and showed `Error: couldn't load plugins: "Method not found": _x.ai/plugins/list`. The debug log records `Method not found` for `_x.ai/hooks/list`, `_x.ai/mcp/list`, `_x.ai/workflows/list`, `_x.ai/plugins/list`, `_x.ai/skills/list`, `_x.ai/marketplace/list`, `_x.ai/billing`, and `_x.ai/suggestPrompt`. Claude and Pi each have a menu profile that hides the panels they cannot answer; Prime has none.

Fix (pager): add a Prime (or generic external) profile in the `claude_menu.rs` / `pi_menu.rs` pattern. Hide `/plugins`, `/hooks`, `/skills`, `/workflows`, `/marketplace`, `/usage`, `/context`; forward commands Prime owns; leave the pager builtins that work.

### 10. The TUI never identifies the agent

The welcome screen says `Grok Build 1.0.45 [stable]`, shows the Grok 4.7 announcement, and tips `Ctrl+O` to toggle auto-approve and Shift+Tab to cycle modes. The footer says only the model. `InitializeResponse.agentInfo` is parsed nowhere in the pager: `InitializedAgent` (`acp/mod.rs:530`) keeps models, auth, commands, and capability flags, but not `agentInfo` or `agentCapabilities.load_session`.

For a Prime session this is misleading, and the native Prime Ctrl+O shortcut (details mode) becomes the pager's auto-approve toggle. Show `Prime Agent 0.10.2` from `agentInfo` in the welcome badge/status line for external agents, and suppress Grok announcements and Grok-specific tips when `external_agent` is set.

### 11. Background send and other dead affordances

While a Python cell was running, `Ctrl+b:send to bg` demoted the execute tool locally, then `_x.ai/terminal/background` returned `Method not found` and the pager logged `Failed to send background request`. Nothing visible changed; the footer still advertises the shortcut. Prime has no background-task ACP surface. Hide the shortcut (or show "not supported by Prime Agent") when the terminal-background method is unavailable.

`_x.ai/suggestPrompt` also fails, so the composer's ghost-suggestion request gets no data; this is silent but worth gating.

### 12. Smaller issues

- Model ids are JSON-array strings. `/model deepseek-v4-flash` answers `Unknown model: deepseek-v4-flash`; only the menu works. Accept the normalized `provider/model` and bare model slug as aliases for the config-option value, or change Prime's values to `provider/model` strings.
- `/model` opens with the cursor on row 0, not the current model; with 137 options the current row is far away.
- The model change toast says `Default model: ...`, but `set_default_model` deliberately does not persist external-agent ids (`settings/setters.rs:1770`). It is a session switch, not a new default.
- Prime's `session_info_update` messages carry no title, so the header stays the cwd path and the dashboard has no summary.
- Prime subagents, goals, heartbeats, and background shells are invisible: no `SubagentSpawned`/`GoalUpdated`/`TaskBackgrounded` updates, so the pager's tasks pane/dashboard stay empty while native Prime's status bar shows `0 subagents · 0 heartbeats · 0 shells · 0 factory`.
- Turn errors render Prime's nested JSON verbatim (`Request failed: Internal error: { "details": ... }`). It is readable, but a one-line sanitized message would fit the scrollback better.
- The `acp-matrix` target exists (`scripts/acp-matrix.sh:53`) but the results table in `docs/acp-matrix.md` has no `prime` row. The target launches the default model, so it currently fails the first prompt before it can test rendering.

## Suggested order

1. Prime: fix the ACP default model (unblocks the first turn).
2. Pager: render Python-cell code (finding 1). Prime can help by adding `variant`/`language` and a derived `command`.
3. Prime: `available_commands_update` for `compact`; pager: Prime menu profile to stop dead stubs and raw `Method not found` panels.
4. Pager: `thought_level` -> `/effort` + footer; hide Plan/background shortcuts when unsupported; fix the `SetSessionModeFailed` toast.
5. Prime: `usage_update` with used/size (and cost in `_meta`).
6. Prime: `session/request_permission` (honour `yoloMode`/`autoMode`) and `session/load`/`session/list` for durable sessions; pager: honest resume messaging.
7. Pager: show `agentInfo` and hide Grok-only chrome for external agents.
8. Both: subagent/goal/title updates, and a Prime row in the ACP matrix with assertions for the code card, thoughts, usage, and `/compact`.

## Verification notes

- Raw ACP probe: `initialize` -> `session/new` -> `session/prompt` with `prime-agent --mode acp --provider deepseek --model deepseek-v4-flash` returned `agent_message_chunk`, `agent_thought_chunk`, `tool_call` (`Python cell`, `kind=execute`, `rawInput.code`), `tool_call_update` (text content), and `session_info_update`; prompt result `{"stopReason":"end_turn"}`.
- `session/load` -> `{"code":-32601,"message":"\"Method not found\": session/load"}`.
- `session/list` -> `Method not found` (pager log: `session list fetch failed`).
- `session/set_mode {modeId:"plan"}` -> `Method not found`; the pager toast still says `Switched to mode: Plan`.
- `session/set_config_option {configId:"thought_level", value:"high"}` -> `currentValue: "high"`.
- `/compact` sent as prompt text -> `session_slash_command` custom message, no assistant text.
- Pager debug: `_x.ai/terminal/background`, `_x.ai/plugins/list`, `_x.ai/hooks/list`, `_x.ai/skills/list`, `_x.ai/workflows/list`, `_x.ai/marketplace/list`, `_x.ai/billing`, `_x.ai/suggestPrompt` all return `Method not found`.
