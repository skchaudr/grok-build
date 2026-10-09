# Cross-machine delegation — verdict

An agent session can open a session on another machine through a Grok leader and get the answer back. The missing piece was a one-shot command. The leader protocol already hosts one ACP process per `--agent-cmd` string and routes `session/new` and `session/prompt` to it.

## What works now

`scripts/leader_delegate.py` is that command. It connects to a leader socket, registers with `capabilities.agent_cmd` set to the worker command, sends `initialize`, `session/new`, and `session/prompt`, and prints the `agent_message_chunk` text.

Live, 2026-10-09, private leader on khoj (`xai-grok-pager agent leader --leader-socket /tmp/delegate-spike.sock --no-exit-on-disconnect --no-auto-update --relay-on-demand`). Worker A was a local ACP process. Its prompt handler shelled out to the same script, aimed at worker B: `ssh -T sab-mini@100.66.99.64` running a read-only Python ACP that executes `hostname`. Prompt to A: `run hostname and report it`. Printed line:

```
worker-a delegated; worker-b said: hostname=sab-mini
```

khoj's own hostname is `khoj-38w`. The leader log shows two backends: client 1 spawned the local delegate worker, client 2 spawned the ssh command. The Mini hub was not restarted and no file was written on the Mini.

The same shape with two local workers is `session_on_worker_a_delegates_to_worker_b` in `tests/test_leader_external_agents.rs`. `agent-bus` is a separate mailbox. It does not open a leader session.

## What is missing

The core gap is headless mode. It does not exist yet. Live, `grok-team-client -p … --leader --leader-socket … --agent-cmd …` exits with `grok: --agent-cmd is only supported in interactive mode, not headless mode`. Installed `grok` 1.0.50 does not have the flag at all. `grok leader` is only `list`, `info`, and `kill`. The interactive pager can sit on an external agent; it does not print the final answer and exit.

`~/.dsh/scripts/gk-smoke --deep` is the caller that needs this. `deep_run` SSHs to Air and runs that `grok-team-client -p` against `~/.grok/leader-mini-hub.sock` for the mini and khoj workers. When the delegate one-shot lands, `--deep` should switch to it.

The worker id is the exact `agent_cmd` string. Roster titles (`host: script`) are labels, not lookup keys. A default leader exits when the last client disconnects and unlinks the socket, so a readiness probe races a slower client. `--no-exit-on-disconnect` is required. `ssh` plus the leader's shlex split is brittle: quotes have to survive shlex and the remote shell. A script already installed on the far side (the hub's `grok-khoj-worker` pattern) is the durable form. Worker A in the demo always delegates; a model was not asked to decide to call the tool.

## Effort

| Gap | Estimate |
| --- | --- |
| Headless `grok -p --leader --leader-socket --agent-cmd` (the `gk-smoke --deep` call). The bail is in `PagerArgs::apply_cwd`. Headless still forces `has_agent_cmd: false`. Then point `deep_run` at that command. | About 1 day |
| Resolve a short worker name to an `agent_cmd` (config map, or the hub's existing ssh worker) | 2–3 days |
| Leave `agent-bus` as messaging. Do not bridge it into sessions. | — |
| Run the same command from a real model session once the CLI exists | A few hours, not a protocol change |
