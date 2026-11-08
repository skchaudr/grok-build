# One grok-build TUI running grok-pi's Pi and Claude side by side: map (2026-11-08, read-only)

Follows `docs/grok-pi-fold-scout.md`. Sources: grok-fork `gc/claude-parity` at `125f59be`, Dwsy branch `719ba57f` (fetched as objects only, no ref). Nothing was built.

## What already exists

- **Switching between agents works today.** The leader routes each session to its own backend process: `resolve_backend` (`xai-grok-shell/src/leader/server.rs:3458`) uses the session's route first, then the request's `_meta["x.ai/agentCmd"]`, then the client's `agent_cmd`. The pager stamps that meta from the `[[agent_choice]]` picker (`xai-grok-pager/src/app/effects/mod.rs:86-96`). `docs/picker-live-proof.md`: on the Air, two Claude and two Pi sessions were opened and re-entered from the dashboard, each answering with its own token.
- **The adapter talks to the pager only over ACP.** `xai-grok-pager` does not depend on `pi-grok-adapter`; only `xai-grok-pager-bin` does (`719ba57f` Cargo.toml files). `PiAgent` implements the full `acp::Agent` surface: `initialize`, `new_session`, `load_session`, `prompt`, `cancel`, `set_session_model`, `set_session_mode`, `ext_method`, `ext_notification` (`pi_adapter/agent.rs`).
- **Most of its traffic is upstream protocol.** The adapter uses about 35 `x.ai/*` methods that the upstream pager already handles (queue, recap, btw, workflows, subagent cancel, scheduler, ask_user_question, models/update). Its 17 `pi/*` methods (`pi/ui/{widget,status,notify,title,editor_text,remote_tui,…}`, `pi/session/{tree,search,tree_label}`) are handled only in grok-pi's `app/acp_handler/mod.rs` and `app/effects/mod.rs`.
- **The stdio host pattern exists.** `xai-grok-shell/src/agent/app.rs:144-155` wraps an `Rc<agent>` in `acp::AgentSideConnection::new(agent, stdout, stdin, spawn_local)` plus `GatewayReceiver::new(gw_rx, conn)`. `PiAgent` is also `Rc` on a `LocalSet` (`grok-pi.rs:333, 1254`).

## The keystone: a stdio `grok-pi-acp` binary

The leader spawns every external agent as a stdio process (`leader/external_backend.rs:1`), local or over `ssh -T`. grok-pi's in-process wiring (`acp_channels()` + `AcpConnection::external`, `grok-pi.rs:1253-1333`) cannot sit behind the leader. So the "remote" item in the pasted brief (#8) is not remote-only: it is the entry point for Pi in the dashboard at all.

With a stdio host:
- **#1 connection seam goes away.** grok-fork does not need `AcpConnection::external` or the in-process path. Pi is one more `[[agent_choice]]`, as Claude is.
- **#7 home directory goes away.** `~/.grok-pi` isolates the grok-pi binary's own pager settings. A stdio agent uses Pi's own `~/.pi`, and the pager stays on `~/.grok`.
- **Remote comes free:** `pi = "ssh -T host grok-pi-acp"`.

Shape: a new bin in `xai-grok-pager-bin` (or a new small crate), about the size of `agent/app.rs:144-155`, plus moving grok-pi's bootstrap out of `bin/grok-pi.rs` and `bin/grok_pi/*` (spawn `pi --mode rpc`, extension materialization, `PiBootstrap`) into a lib, so both binaries share it. Every line of it is an add.

## What is left, in order

| # | Work | Where | Upstream conflict |
|---|---|---|---|
| 1 | Add `pi-grok-adapter` (46 files), the bootstrap lib and the 23 extensions; build `grok-pi-acp`; add it as `[[agent_choice]] pi`. Run it against the **unchanged** grok-fork pager. Whatever renders wrong is the real port list for 3 to 5. | new crate, new bin, `Cargo.toml` members | none (adds) |
| 2 | Make the agent profile per session. `AppView.external_agent` and `external_agent_label` are app-wide, set from the launch command (`app/event_loop.rs:1070-1074`). `AppView::is_claude_session` (`app_view.rs:1726`) reads them for rewind and fork (`dispatch/rewind.rs`, `dispatch/session/fork.rs:39`). The per-session `agent.session_agent_name` already exists and already drives the slash label (`app_view.rs:5424-5430`). Switch `is_claude_session` and the rest to the active agent's name, and add `is_pi_session` the same way. | grok-fork pager only | small, our own code |
| 3 | Port the `pi/*` handlers as a new `acp_handler/pi_ui.rs`, with one dispatch line in `acp_handler/mod.rs` and `effects/mod.rs`. | 2 shared files | 1-line hooks |
| 4 | Tool cards: grok-pi's `scrollback/blocks/tool/{codemode,eval}.rs` against grok-fork's `tracker.rs` (runState `:1086`, `parentToolCallId` `:1990`, terminal deltas `:1556`). Pick one codemode renderer. | `tracker.rs`, `scrollback/state/*` | real conflict, both sides edit `tracker.rs` |
| 5 | Pi slash commands (`/tree`, `/pi-models`, `/plan-mode`, …) as an `is_pi_session` table beside `claude_menu`, and subagents. Pi subagents arrive as `x.ai/subagent/*` from the adapter. Claude's arrive through `claude_subagents.rs`. Both feed the same pager events, so they should coexist; that is untested. | `slash/mod.rs`, `slash/commands/mod.rs` | moderate |
| 6 | Optional: the F2 Pi settings (`modals.rs` +3604 lines, `settings/*`, `views/modal.rs` +1106), 100+ Pi themes, the dpi shim (4 cherry-picks). | settings, theme registry | large but additive; themes are assets plus a list |

Overlap measured now: 90 files changed on both sides since `2bdd1d6a` (84 pager, 2 render, 2 shell, 1 agent, 1 pager-minimal). Dwsy's side of those is almost all additions (for example `modals.rs` +3604/-347, `app_view.rs` +1312/-64). Steps 1 to 3 touch about 4 of the 90. Most of the rest comes in only if step 6 is taken.

## Clean path to upstream

- Adapter, bootstrap lib, extensions and the stdio bin are new paths. An xAI sync never touches them.
- Pager changes go in new files (`pi_ui.rs`, a Pi menu table, card files), with one-line hooks in upstream files. That is the pattern `claude_menu.rs` and `claude_subagents.rs` already follow.
- Take Dwsy's pager work feature by feature, after step 1 shows what is missing. Do not merge his 350 modified files wholesale.
- Future Dwsy adapter updates: the adapter crate can be re-synced from his branch as a subtree, because grok-fork does not edit it.

## Open, unverified

1. Whether `pi-grok-adapter` at `719ba57f` compiles against grok-fork's `xai-grok-shell` (Dwsy changed 43 shell files, +1959; 2 of them overlap ours) and `xai-acp-lib`. One `cargo check -p pi-grok-adapter` in a worktree answers it.
2. Whether `PiAgent::new_session` works when the leader calls it cold. grok-pi creates the session in bootstrap before the pager connects (`grok-pi.rs:1247`, `bootstrap.session_id()`).
3. Whether `PiAgent`'s notification sender (`agent_channel.tx`) is the same type `GatewayReceiver` drains. If not, the host needs a small forwarder.
4. Whether the leader relays agent-to-client `pi/*` ext notifications. It relays `x.ai/*` requests to DSH (`docs/leader-external-agents-proof.md`), but the reverse direction for unknown methods was not read.
5. Claude's subagent capability is stamped by the launch command (`acp/mod.rs:506-513`). When Claude is a picker choice and not the launch agent, whether its backend's `initialize` carries that meta was not read.
6. Which of the 23 extensions are needed for a faithful Pi.
