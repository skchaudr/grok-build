# SMOKE: Pi DAG + subagents (2026-10-26, mini)

Pi 1.4.0, pi-subagents 0.82.1, @osolmaz/pi-workflows 0.17.7, model cliproxy/gemini-3.8-flash-high. Repo: `/private/tmp/claude-501/-Volumes-Mini-SSD-offload-repos-grok-build/b83718ce-62d7-4a8d-9641-bb7ac83b6995/scratchpad/pi-smoke`. Driver: `pi --mode rpc --no-session`, one JSONL `prompt` on stdin (Node script `rpc-drive.mjs` in the same scratchpad).

## C1 install state: PASS (after one fix)
- Both packages present on the mini (`ls ~/.pi/agent/npm/node_modules/`): pi-workflows 0.17.7, pi-extensible-workflows 6.2.2.
- `settings.json` already excludes pi-extensible-workflows `-dist/src/index.js` (+ starter/subagents/trajectory entries). No settings edit needed.
- `pi -p "reply with the single word PONG"` -> stdout `PONG`; stderr only `Warning: No models match pattern "openai-codex/gpt-5.3-codex-spark"`. No duplicate `workflow` diagnostic.
- Found: the first `/workflow smoke` failed, `ERR_MODULE_NOT_FOUND: Cannot find package '@earendil-works/pi-ai' imported from .../@osolmaz/pi-workflows/src/builtins/pi-agent-group.ts`. Fix (outside smoke repo): symlinked `pi-ai`, `pi-coding-agent`, `pi-tui` from `~/.pi/agent/install/releases/1.4.0/node_modules/@earendil-works/` into `~/.pi/agent/npm/node_modules/@earendil-works/`; `pi-workflows server stop`.

## C2 DAG drives Pi + child: PASS
- `pi -p "/workflow smoke"` (pre-fix): rc 0, empty stdout, no run. Not retried post-fix. Via rpc `prompt` `/workflow smoke`: run `20261027T045847071Z-smoke-0bab5a31` completed; `view --once`: A ok, B ok, output `{"ok":true}`.
- A called `subagent` (smoke-child, async), polled `status`, submitted `{"text":"PONG"}`.
- Child run `a95d5e16-e6fe-4487-95ee-738c5f3b1501`, own session file `.../pi-subagent-session-vzooEK/a95d5e16-.../run-0/session.jsonl`, session id `01a17bb9-c7ae-7438-8f12-e7abaee78fca` != origin `01a17bb9-9ae6-75a1-ab3f-9919e14f6205`.

## C3 codemode in child: PASS
- Run `d48ed040-7a83-4fd4-8da2-49be24be90d7`, agent smoke-code (`tools: codemode, read`). Session has toolCall `codemode` (`const content = await tools.read({ path: "README.md" ...`). Returned `# smoke`.

## C4 child starts its own workflow run: PASS on 2nd attempt
- Attempt 1 (`tools: inherit`): run `71f80371-7877-4a90-93e5-1ba70784f581` had only `contact_supervisor`, reported `workflow` tool missing. `tools: inherit` is not honored in a custom agent file (see HARNESS-FACTS correction). Killed runner pid 1293.
- Attempt 2 (`tools` omitted): child `5b647ca0-b884-43ef-bb59-2be21679cd21` ran `workflow list`, started echo. `pi-workflows runs`: `20261027T050605717Z-echo-662e1b6c`, originSessionId `01a17bbe-4416-73a2-bb98-f6e4d9990503` (= child session header id), completed, output `{"reply":"PONG"}`.

## Files
`.pi/agents/smoke-child.md`
```
---
name: smoke-child
description: Smoke test child, replies PONG
tools:
thinking: low
systemPromptMode: replace
---
Reply with exactly: PONG
```
`.pi/agents/smoke-code.md`
```
---
name: smoke-code
description: Smoke test child using codemode
tools: codemode, read
thinking: low
systemPromptMode: replace
---
Use the codemode tool exactly once to run tools.read on README.md, then reply with only the first line of the file.
```
`.pi/agents/smoke-dag.md` (final; `tools: inherit` removed)
```
---
name: smoke-dag
description: Smoke test child that starts a pi-workflows run
thinking: low
systemPromptMode: replace
---
Start the workflow named echo with the input 'PONG' using the workflow tool, wait for it, and return its output.
```
`.pi/workflows/smoke.workflow.ts`
```ts
import { agent, compute, defineWorkflow } from "@osolmaz/pi-workflows";
export default defineWorkflow({
  name: "smoke", startAt: "A",
  nodes: {
    A: agent({
      prompt: () => "Call the subagent tool once: agent smoke-child, task 'reply PONG', async true, then wait for it and submit its output text as the result",
      expectedOutput: `{ "text": "child output text" }`,
      allowedTools: ["subagent"],
    }),
    B: compute({ run: ({ outputs }) => ({ ok: JSON.stringify(outputs.A).includes("PONG") }) }),
  },
  edges: [{ from: "A", to: "B" }],
});
```
`.pi/workflows/echo.workflow.ts` is a copy of the package example.
