# Brief: leader publishes `--agent-cmd` sessions to the shared roster

Worktree: `~/repos/grok-build-wt/ext-roster`, branch `feat/leader-external-roster` (pushed). Commit + push often.
Commit trailer: `Co-authored-by: cursor-agent <cursor-agent@cursor.com>`
Subagents: 0.
Build cache: `export CARGO_TARGET_DIR=$HOME/repos/grok-build/target` (shared with another agent; builds may wait on the lock — that's fine). Only test the crates you touch (`-p xai-grok-shell`, plus `cargo check -p xai-grok-pager`).

## Verified problem (live probe on this VM)
One leader (`grok agent leader --leader-socket ~/.grok/leader-spike.sock`), two pager clients with `--leader --leader-socket <same>`:
client A native, client B `--agent-cmd 'cursor-agent acp'`. In A's `/dashboard`, B's Cursor session is absent (not even Inactive).

## Where (read first)
- `crates/codegen/xai-grok-shell/src/leader/roster_merge.rs`: `ExternalRoster` + `ExternalRosterPublisher` already merge extra rows into `x.ai/sessions/list` responses and broadcast `x.ai/sessions/changed`. Today only the Cursor worker door publishes (publisher is `dead_code` outside tests).
- `crates/codegen/xai-grok-shell/src/leader/server.rs` ~2079: on a `session/new|load` response, `session_backend.entry(session_id).or_insert(SessionRoute { backend, .. })`. `backend` is `BackendId::External(cmd)` for agent-cmd sessions. Nothing publishes them.
- `RosterEntry` in `agent/roster.rs`.
- External process lifecycle: `leader/external_backend.rs` (`AgentTraffic::Exited`).

## Do
1. Give the leader loop an `ExternalRosterPublisher` for external-backend sessions (from the same `ExternalRoster` that `roster_merge` uses).
2. When an External session route is first registered, publish a `RosterEntry`: session_id, cwd (capture it from the originating `session/new`/`session/load` request params), title = short agent name derived from the cmd (e.g. `cursor-agent`), `session_kind = Some("external")` (check the field's doc — pick the value consistent with it), activity idle, `resident = true` if that field exists.
3. Remove the row when the session is closed/unregistered and when its backend process exits (all its sessions).
4. Keep native-session behaviour byte-identical.
Optional only if trivial: mark activity working/idle from `session/prompt` request/response. Otherwise note it in OBSERVATIONS.md as follow-up.

## Eval first
Failing tests before code (match existing style in `leader/roster_merge_tests.rs` / `leader/server_tests.rs`): external session registered → row present in merged `sessions/list` + a `sessions/changed` broadcast; backend exit → row gone; native session → no external row.

## Verify (required)
- Tests above + existing `leader` tests pass.
- Live: build the pager binary (`cargo build --release -p xai-grok-pager` → `$CARGO_TARGET_DIR/release/xai-grok-pager`), then repeat the probe in a detached tmux with an isolated socket `~/.grok/leader-roster-test.sock`: native client A + `--agent-cmd 'cursor-agent acp'` client B, one short prompt in B, `Ctrl+\` dashboard in A → B's session row must appear. Then try opening that row from A and sending one prompt: record whether it reaches the Cursor agent. Tear down tmux/leader/socket after. Native Grok auth on this VM is expired; that's expected, don't fix it.
- Record exact commands/results in `OBSERVATIONS.md` (committed). Don't touch `~/.grok/bin`, Mini, or Air. Don't merge. Stop and report.
