# TASK: Leader hosts external ACP agents (DSH, Cursor, Pi, Codex)

**Repo / branch:** `~/repos/grok-build` (skchaudr fork), land on `main`. One worktree: `.worktrees/leader-external-agents`.
**Outcome in one line:** `grok --leader --agent-cmd "dsh acp"` works, from any machine attached to a leader, with sessions shared and resumable like native Grok sessions.

## Why this exists (settled — don't re-investigate)

- Commit `8b2977f1` added `--agent-cmd` only to the **embedded** path: `acp::connect()` (`crates/codegen/xai-grok-pager/src/acp/mod.rs:179`) spawns `spawn_external_subprocess` instead of `spawn_grok_shell`.
- `acp::connect_via_leader()` (`acp/mod.rs:297`) was never touched, so in leader mode `--agent-cmd` would be silently ignored. The commit papered over that with `if args.agent_cmd.is_some() { use_leader = false; }` (`app/mod.rs:816`). No rationale was recorded. Shortest path, not a constraint.
- The result: every external agent loses the leader. That means no shared sessions, no multi-client, and no cross-machine attach.
- Cross-machine attach already works for native Grok. `ssh -L <local.sock>:~/.grok/leader.sock` plus `grok --leader --leader-socket <local.sock>` was verified sab-air → khoj-38 on 2026-10-03 (see `~/.dsh/handoffs/2026-10-03-grok-pager-multimachine.md`, `gk` in sab-air dotfiles). This task makes that same path carry external agents.

## Where the code is

- Leader server: `crates/codegen/xai-grok-shell/src/leader/` (`server.rs` 2.6k lines, `mod.rs`, `protocol.rs`, `client.rs`, `in_process.rs`; tests in `server_tests.rs`, `tests/test_leader_stdio_integration.rs`).
- Today the leader runs **one** backend: `in_process::spawn_agent` builds a single `MvpAgent` and pipes raw JSON lines (`UnboundedSender<String>`). The server multiplexes clients onto it by `sessionId` (see `session/load`/`session/resume` handling around `server.rs:423-566`).
- Per-client settings already flow via `ClientCapabilities` (`protocol.rs:179`: `yolo_mode`, `default_model`, `terminal`, …). The leader injects them into `session/new` and `session/load`.
- External spawn to reuse: `crates/codegen/xai-grok-pager/src/acp/spawn.rs` (`spawn_external_subprocess`). Move it, or a shared version, into `xai-grok-shell`, since the leader can't depend on the pager crate.
- Pager-side agent-cmd special cases to keep working: `app/session_startup.rs` (`has_agent_cmd` skips the local session registry), `headless.rs`.

## Required design

1. **Backend registry in the leader.** Key by agent identity: `native` or the exact `agent_cmd` string. The native backend is today's `MvpAgent`. External backends are ACP stdio subprocesses spawned lazily on first use, one process per distinct command, shared by every client that asks for that command.
2. **Client chooses the backend.** Add `agent_cmd: Option<String>` to `ClientCapabilities` (serde default `None`, so old clients still work). The pager's `connect_via_leader` sends it.
3. **Route by session.** `session/new` from a client goes to that client's backend. Record `sessionId → backend`. Every later session-scoped request or notification (`session/prompt`, `session/cancel`, `session/load`, `session/resume`, `session/close`, ext methods) routes by that map. A `session/load` for an id not in the map goes to the client's backend.
4. **Agent → client traffic.** Fan out from every backend through the existing per-session client routing. Agent-initiated requests (permission prompts, fs, terminal) must reach the right client.
5. **Initialize/auth per backend.** Send the client's `initialize` to the external backend once, when it's spawned, and cache the result for later clients. **Leader startup and native auth must not block external agents.** The 2026-10-03 analysis found the leader's Grok login check would block them. Gate it so a leader with no Grok login still serves external backends, and native-only paths still require login.
6. **Lifecycle.** Backend crash: fail that backend's sessions with a visible error to attached clients (no silent hang), and respawn on the next request. Leader shutdown: kill child processes. `--no-exit-on-disconnect` semantics unchanged.
7. **Delete the forced false.** Remove `use_leader = false` for `agent_cmd` in `app/mod.rs`. Leader mode now follows normal precedence for external agents too.
8. **No silent fallback to a different agent.** The pager already falls back to embedded when the leader connect fails (`app/mod.rs` ~1111, "leader connect failed; falling back to embedded agent"). That's acceptable only if the fallback runs the **same** `agent_cmd`, and it must show a visible notice in the TUI saying it's running locally on `<hostname>`. Per DSH AGENTS.md, never swallow this.

## Acceptance (write these as failing tests FIRST)

Use a tiny fake ACP agent fixture (a script or test binary) that echoes its pid, hostname and command. Don't depend on real DSH in unit tests.

- [ ] Leader + client with `agent_cmd=fake-a`: `session/new` → prompt → response comes from fake-a, not MvpAgent.
- [ ] Two clients, same `agent_cmd`: one backend process (same pid). Client B can `session/load` A's session and receives its live updates.
- [ ] Two clients, `fake-a` and `fake-b`: two processes, and sessions never cross.
- [ ] Native client and external client on one leader at the same time both work.
- [ ] Leader with no Grok login: external client works, native client gets the existing auth error.
- [ ] Kill fake-a mid-session: the client gets a visible error, and the next `session/new` respawns it.
- [ ] Old client with no `agent_cmd` field: unchanged behavior (native).
- [ ] Pager: `--agent-cmd` with leader enabled connects via the leader (assert the connect target is `Leader`), and the embedded fallback shows its notice.
- [ ] Existing leader and pager test suites stay green.

## Manual verification (do it, don't hand it back)

1. khoj-38: `systemctl --user restart grok-leader` (unit: `~/.dsh/systemd/grok-leader.service`) running the new build.
2. On khoj-38: `grok --leader --agent-cmd "dsh acp"`. Ask the agent to run `hostname`, and confirm the footer/model shows DSH's route, not Grok.
3. From sab-air: `gk --agent-cmd "dsh acp"` (tunnel window `hermes-proxies:grok-khoj`). `hostname` must print `khoj-38w`. Then resume the session from step 2.
4. Repeat step 3 with Cursor (`cursor-agent acp`) or Codex. Use whatever command the `gb` picker uses today (`~/.dsh/scripts/gb`).

## Out of scope

- grok.com relay / headless web for external agents.
- `gk <host>` multi-host and a launchd unit for sab-mini (separate small task).
- Upstreaming. Keep the diff clean enough to send upstream later, but don't open the PR.

## Done means

All acceptance tests green, manual steps 1–3 pass, merged to fork `main` and pushed, and the worktree removed. Add a 5-line update to `~/.dsh/handoffs/2026-10-03-grok-pager-multimachine.md`. Log surprises in `OBSERVATIONS.md` on the branch, and fold them into the handoff before merging.
