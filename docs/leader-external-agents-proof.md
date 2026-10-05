# Remote external ACP proof — 2026-10-05

Tested directly after khoj reboot; no rebuild, no delegated worker.

## Versions and topology

- Source/binaries: `89cae8429890` (leader-external-agents).
- Leader: khoj-38w, pid 1390, systemd `grok-leader`, stamped 1.0.46, `--no-exit-on-disconnect --no-auto-update`.
- Client: sab-air.local, reused Darwin binary from sab-mini, stamped 1.0.45.
- SHA-256 on Mini and Air: `0062916b2cea7dba6e3a00376f486db65e81f42668b13b8faa6e20496a1b965c`.
- Air socket `~/.grok/leader-khoj.sock` reports remote pid 1390 and `/home/sab-mini/.grok/leader.sock`.

## Verified invocation on Air

```sh
~/.grok/bin/grok --leader --leader-socket ~/.grok/leader-khoj.sock \
  --agent-cmd 'env DSH_ACP_PROVIDER=openrouter DSH_ACP_MODEL=inception/mercury-2.5 /home/sab-mini/.local/bin/dsh --profile acp-enhanced' \
  --always-approve --trust --cwd /
```

Test used the actual pager in a dedicated tmux PTY, with `--fullscreen --no-alt-screen --debug-file <evidence file>`; not `-p` or `grok agent stdio`. `/` is a common cwd on both hosts; arbitrary Air-only working directories were not tested.

## Evidence

1. Air first client pid 23224 logged `use_leader=true embedded_fallback=false`. ACP `initialize` returned `deepseek-harness-acp-enhanced` version 0.9.1 and `loadSession=true`.
2. `session/new` returned **`6b31df5c-91d6-4f31-80c0-392d8eda1767`** and route `openrouter/inception/mercury-2.5`.
3. khoj had DSH pid 13098, parent leader pid 1390. Its provider/model environment matched that route. Leader logged client 3 registration at 06:58:44Z.
4. First prompt called `bash` with `hostname`. DSH session event 18 recorded tool stdout **`khoj-38w\n`**. Turn finished successfully over live OpenRouter inference.
5. `/quit` disconnected client 3 at 07:00:41Z; DSH process stayed alive.
6. A new Air pager with the identical command plus `--resume 6b31df5c-91d6-4f31-80c0-392d8eda1767` registered as leader client 4 at 07:00:43Z. It sent `session/load` for that exact ID, received a successful response, and replayed the transcript. It did not create a replacement session.
7. Second prompt remembered **ORBIT-6274** without the prompt supplying it again. DSH events 31/33 recorded **`khoj-38w\n`** and **`Linux\n`** from new shell calls. Live inference completed the second turn.
8. Both test clients exited with `/quit`. Leader and DSH backend remained running.

Mercury's prose omitted the trailing `w` twice, despite correct raw tool output. Host proof is the recorded tool result and process ancestry, not the model's summary.

## Default installation on Air

`~/.grok/bin/grok` now points to `grok-89cae842-pinned`, a two-line shell launcher:

```sh
#!/bin/sh
exec "$HOME/.grok/bin/grok-89cae842" --no-auto-update "$@"
```

This keeps stock auto-update from replacing the locally built patch (its stamp is older than stock). The original `grok-1.0.46` is preserved. A deliberate future update must replace this pin; this is local installation configuration, not an upstream source change. `grok --version` and leader info through the default path were verified after switching.

## Existing tests rerun without compilation

Ran already-built test executables in `target/debug/deps`, sequentially:

- `test_leader_external_agents-1845b7aadb46b04d`: 8 passed.
- `test_leader_stdio_integration-1e1f54c4a9917a86`: 47 passed, 1 ignored.
- `xai_grok_shell-2101ef3408545e50 leader::`: 287 passed, 6775 filtered out.

This is a rerun of existing test artifacts, not a fresh source build or full-suite pass.

## Evidence locations and limits

- Air: `~/.grok/proofs/leader-20261005/{invocation.txt,resume-invocation.txt,client-first.log,client-resume.log,first-pane.txt,resume-pane.txt,previous-default.txt}`.
- khoj: `~/.grok/proofs/leader-20261005/` (filtered tool events and test logs).
- DSH durable session: `~/.dsh/sessions/--root--/6b31df5c-91d6-4f31-80c0-392d8eda1767/session.v4.jsonl.zstd`.
- Leader client registrations/disconnects: `~/.grok/logs/unified.jsonl`.

The pager sends local MCP definitions containing Air paths to the remote session. Shell proof did not exercise those MCPs. DSH returns `Method not found` for several optional native `_x.ai/*` extensions; prompt/tool streaming and explicit-ID resume worked.

`grok agent stdio` currently sets `ClientCapabilities.agent_cmd = None` in pager-bin `run_agent_command`; `-p` also did not prove this route. Use the actual pager path above. No claim here about restart persistence, other ACP harnesses, a multi-machine task team, or newly added orchestration primitives.
