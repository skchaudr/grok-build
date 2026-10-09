# Cross-machine delegation — verdict

An agent session can open a session on another machine through a Grok leader and get the answer back. The one-shot is the grok binary: `grok -p --leader --leader-socket S --agent-cmd C`. The leader hosts one ACP process per `--agent-cmd` string and routes `session/new` and `session/prompt` to it.

## What works now

Headless mode allows `--agent-cmd` when `--leader` is set. `PagerArgs::apply_cwd` still rejects the flag on a headless run that does not pass `--leader`. The headless turn connects with `ClientMode::Stdio` and `capabilities.agent_cmd`, sets `has_agent_cmd` on startup materialization, and skips fail-closed auth when the worker advertises no method. `headless_prints_the_named_workers_answer` prints `command=headless-worker` from a fake ACP worker.

`scripts/leader_delegate.py` is the same protocol in Python. It connects to a leader socket, registers with `capabilities.agent_cmd` set to the worker command, sends `initialize`, `session/new`, and `session/prompt`, and prints the `agent_message_chunk` text.

Live, 2026-10-09, private leader on khoj (`xai-grok-pager agent leader --leader-socket /tmp/delegate-spike.sock --no-exit-on-disconnect --no-auto-update --relay-on-demand`). Worker A was a local ACP process. Its prompt handler shelled out to the same script, aimed at worker B: `ssh -T sab-mini@100.66.99.64` running a read-only Python ACP that executes `hostname`. Prompt to A: `run hostname and report it`. Printed line:

```
worker-a delegated; worker-b said: hostname=sab-mini
```

khoj's own hostname is `khoj-38w`. The leader log shows two backends: client 1 spawned the local delegate worker, client 2 spawned the ssh command. The Mini hub was not restarted and no file was written on the Mini.

The same shape with two local workers is `session_on_worker_a_delegates_to_worker_b` in `tests/test_leader_external_agents.rs`. `agent-bus` is a separate mailbox. It does not open a leader session.

A model chose the command. Native `grok` 1.0.50 returned 401 from the cliproxy (`no auth context`), so the session was `claude-code-acp` started by this binary: `--always-approve --leader --leader-socket /tmp/model-delegate.sock --agent-cmd 'env CLAUDE_CODE_EXECUTABLE=…/claude claude-code-acp' -p …`. Claude's first tool call was the one-shot against `python3 /tmp/mini-hostname-acp.py`. The tool printed `hostname=sab-mini`. A second local `hostname` printed `khoj-38w`. Claude did not ssh to the Mini. Transcript is in OBSERVATIONS.md.

`~/.dsh` branch `fix/smoke-deep` (worktree `.worktrees/smoke-deep`) locks `scripts/gk-smoke` `deep_run` to that command: Air runs `grok-team-client --no-auto-update --leader --leader-socket ~/.grok/leader-mini-hub.sock --agent-cmd … -p …`. `tests/test_gk_smoke.py` rejects a drift to `leader_delegate.py`. The Air binary is still the old client until it is replaced; this branch did not touch Mac binaries or the Mini hub.

## What is missing

Installed `grok` 1.0.50 has no `--agent-cmd`. `grok leader` is only `list`, `info`, and `kill`. The Air `grok-team-client` that `gk-smoke --deep` invokes still has the old bail until that binary is updated.

The worker id is the exact `agent_cmd` string. Roster titles (`host: script`) are labels, not lookup keys. A default leader exits when the last client disconnects and unlinks the socket, so a readiness probe races a slower client. `--no-exit-on-disconnect` is required. `ssh` plus the leader's shlex split is brittle: quotes have to survive shlex and the remote shell. A script already installed on the far side (the hub's `grok-khoj-worker` pattern) is the durable form.

## Effort

| Gap | Estimate |
| --- | --- |
| Headless `grok -p --leader --leader-socket --agent-cmd`, `has_agent_cmd`, and `gk-smoke --deep` pointed at that command | Done on this branch and `dsh-home` `fix/smoke-deep` |
| Ship that binary as Air `grok-team-client` (Mac binaries were left untouched) | Deploy, not a protocol change |
| Resolve a short worker name to an `agent_cmd` (config map, or the hub's existing ssh worker) | 2–3 days |
| Leave `agent-bus` as messaging. Do not bridge it into sessions. | — |
