# grok-pi into grok-fork: scout report (2026-11-08, Sonnet 5.5, read-only)

Short version: grok-pi is not a Pi variant of grok-build's ACP path. It is a deep fork that runs Pi in-process through its own adapter crate. grok-fork instead drives the third-party `pi-acp` over stdio. Dwsy has a branch on exactly grok-fork's upstream commit (2bdd1d6a). I'd fold from that branch, not from grok-pi `main`.

Nothing here was built or compiled. Overlap counts are by file name, not hunk-level conflicts.

## 1. What grok-pi is

- **Authorship:** it is Dwsy's project, "Pi-Grok Native TUI". 394 of the 398 commits since the xAI base are Dwsy. The other 4 are two by Sab (merge `ff867bf8` plus the dpi-rpc shim) and two by "kyros".
- **Remotes:** `origin` is `skchaudr/grok-pi`; `upstream` is `Dwsy/grok-pi-tui`, not xAI (`git remote -v`).
- **Base:** xAI `37949780` (2026-09-09), recorded in `AGENTS.md`. `SOURCE_REV` `c4ea71cf` is the monorepo hash for that sync (per `docs/upstream/UPSTREAM_CHANGELOG.md:36-41`).
- **Upstream gap:** grok-pi `main` has NOT taken `2bdd1d6a`. The changelog marks that sync "Pending". Its merge-base with `2bdd1d6a` is `37949780`. That gap is 7 xAI syncs, 2484 files.
- **Size of the fork:** `git diff 37949780 main` is 1075 files, +180k/-6k. 614 of those are under `crates/codegen`: 267 added, 344 modified. Pager 259 files, pager-render 118, pager-bin 55, pi-grok-adapter 46, shell 44.
- **Branches:**
  - `main` at `ff867bf8`, the merge of Dwsy's v0.1.12-beta.1.
  - `sab/dpi-rpc-shim` at `8f376ca1`, 4 commits ahead of `main`: `5c374deb`, `01a8039a`, `0859bfec`, `8f376ca1`.
  - `upstream/merge/upstream-2bdd1d6a-static-20261013` at `719ba57f`, which is the one that matters.
- **The 2bdd1d6a branch:**
  - It has `2bdd1d6a` as an ancestor, 16 commits past `1998a17e`.
  - It holds Dwsy's Pi seams already reconciled to `2bdd1d6a`'s API moves: `26df9ce9`, `ea1a80e3`, `30682df7`, `c3ffca0d`.
  - It is not merged into grok-pi `main`.
  - It is 37 commits behind Dwsy's `upstream/main`, but the adapter, extensions and bin differ from `main` by only 42 files.
- **Merge base with grok-fork:** both repos contain `37949780` and `2bdd1d6a`. grok-fork's marker is `scripts/upstream-sync.rev` = `2bdd1d6a`. grok-fork has 601 own commits and 701 files since that point. Sab's own grok-pi commits touch 3 files and are independent of the Dwsy work.
- **`dsh-pi`:** a TypeScript host that runs unmodified Pi extensions inside DeepSeek Harness (DSH), per its `README.md`. It has no Rust and isn't part of the fold.
- **Installed binary:** `sab-air:~/.local/bin/grok-pi` reports `0.1.11 [stable]`, built Oct 25. `pi-grok` and `pig` symlink to it. grok-pi's working tree is clean.

## 2 and 3. Inventory and equivalents

| Item | grok-pi | grok-fork equivalent | Fold type |
|---|---|---|---|
| Spawn and ACP adapter | `pi-grok-adapter` (46 files, about 20k lines); `PiAgent` implements `acp::Agent` (`pi_adapter/agent.rs:4`); spawns `pi --mode rpc` (`pi_rpc.rs:301`); wired through `acp_channels()` and `AcpConnection::external` (pager `acp/mod.rs:218`); no stdio mode | Third-party `pi-acp` over stdio: `spawn_external_subprocess` (`acp/spawn.rs:363`), `gb:30,64-76`. The patched `pi-acp` source is `sab-air:~/repos/Pi-Coding-Agent/pi-acp-patched`. | New-crate add, then a pager seam in `acp/mod.rs` that conflicts with grok-fork's `AgentEndpoint` and external-caps work |
| `ExternalUiProfile` (agent name, command allowlist, logo, welcome brand, host features) | pager `acp/mod.rs:85-205` | None. grok-fork hides commands by agent command string (`acp/mod.rs:469-520`, `docs/claude-parity/claude-menu-undecided.md`). | Manual port; touches Claude's menu-hiding logic |
| Launch, args and config | `bin/grok-pi.rs` (1763 lines) and `bin/grok_pi/*` (about 8k lines total); `--no-extensions` plus allowlist (`grok-pi.rs:634-639`); isolated `~/.grok-pi` (`home.rs`) | `--agent-choice NAME=CMD` (`app/agent_choice.rs:152`), `gb` picker | Clean add of the bin. `home.rs` isolation conflicts with grok-fork sharing `~/.grok`. |
| Embedded Pi extensions (TypeScript) | `extensions/pi-grok-*`, 23 dirs, about 20k lines (bash 5360, subagents 4670, remote-tui 1847, web-config 1412); materialized to temp dirs by `*_extension.rs` | None | Clean add |
| Event feed and tool cards | `tool_projection.rs` (950), `tool_projection/tests.rs`; dedicated `CodemodeToolCallBlock` (`scrollback/blocks/tool/codemode.rs`, `eval.rs`) | Live: `tracker.rs:1086` (runState), `:1990` (codemode `parentToolCallId`), `:1556` (terminal-output deltas). Replay: `pi_feed.rs` (traces and past sessions only). | Conflict in `tracker.rs`; the two codemode renderings must be reconciled. `pi_feed.rs` stays. |
| Subagents | `subagent_projection.rs`, `subagent_transport.rs`, the pi-grok-subagents extension, `views/subagent_catalog_pane.rs` | `claude_subagents.rs` (`subagent_bridge_gate`, called at `spawn.rs:394`); asyncTasks for pi-acp noted at `claude_subagents.rs:15` | Conflict with the Claude work |
| Slash commands | `get_commands` mapped into the slash registry, `PI_GROK_NATIVE_COMMANDS` (`grok-pi.rs:141`); added `/tree`, `/review`, `/pi-config`, `/pi-models`, `/plan-mode`, `/reload`, `/clone`, `/hotkeys`, `/notify` (`slash/commands/*`) | Upstream commands at 2bdd1d6a; Claude kept/hidden table | Per-file; adds are clean, `slash/mod.rs` and `commands/mod.rs` conflict |
| Model, context and session display | `model.rs` (1929), `context_projection.rs`, `cache_metrics.rs`, `views/cache_graph.rs`, `psm_session_catalog.rs`, `/session-info` | `model_state.rs`: `model_config_id` (`:76`), plus commits `53eb6e90`, `89950b5b`, `6a07a53b` | Conflict in `model_state.rs`; the adapter needs `set_model` rather than config option |
| Plan, todo, goal, loop, recap, btw | `plan_mode.rs`, `todo_bridge.rs`, `goal_host.rs`, `loop_host.rs`, `recap_bridge.rs`, `btw_bridge.rs` | Upstream-native pager UI exists (`slash/commands/{plan,loop_cmd,recap,btw}.rs`) but nothing answers it for Pi | Adapter side is a clean add; the pager already has the client side |
| Workflows | `pi_workflow_backend.rs`, `workflow_host.rs`; shell `session/workflow/{backend,external}.rs` | `workflow.rs`/`workflows.rs` slash commands (upstream) | Clean add, 3 shell files |
| Settings, themes | `views/pi_settings/*` (F2), `pi_config`, `pi_models`, 100+ Pi theme JSONs under `pager-render/assets/pi-themes` | grok-fork ported Zed themes (`a8bae868`) | Adds are clean; `theme/mod.rs` and `theme/cache.rs` conflict |
| Durable dpi agents | Sab's shim: `tools/dpi-rpc/dpi-rpc.mjs`, `run-dpi.sh`, `pi_adapter/session.rs` (+50) | `xai-grok-traces/src/dpi.rs` (read-only traces) | Clean cherry-pick of 4 commits, after the adapter lands |
| Binary and install | `grok-pi` (plus `pi-grok`, `pig`); `install.sh` pulls Dwsy/grok-pi GitHub releases; `build.sh` | `grok`; `scripts/hub/{gb-build,grok-swap,grok-hub-sync}` | Drop `install.sh`, use grok-swap. Fork-side install is unverified. |

## Overlap

- Branch `719ba57f` against `2bdd1d6a`: 268 crate files added, 350 modified. The modified files carry +37.5k/-4.9k lines.
- 94 of the branch's files are also changed in grok-fork's own work since `2bdd1d6a`. About 77 of them are pager files, for example `acp/{mod,model_state,tracker}.rs`, `app/{modals,app_view,agent_view/*,dispatch/*,effects/*}`, `slash/{mod,commands/mod}.rs`, and `scrollback/{block,render,state/*}.rs`.
- The Pi-only adds under `pi-grok-adapter`, `bin/grok-pi*`, and `extensions/` are 243 files, 60k lines, and pure additions.

## 4. Proposed fold order

1. **Decide the source.** Use branch `719ba57f`, not `main`. Fetching it into grok-fork means adding `Dwsy/grok-pi-tui` as a remote. That is a git change, so I haven't done it.
2. **Adapter crate, compile only.**
   - Add `crates/codegen/pi-grok-adapter/**`, the root `Cargo.toml` and `Cargo.lock` members, and `xai-workflow`.
   - Add `xai-grok-shell/src/{host_features.rs,session/workflow/backend.rs,session/workflow/external.rs}`.
3. **Connection seam.** Port `ExternalUiProfile` and `AcpConnection::external` into pager `acp/mod.rs`, plus the profile consumers in `app/agent.rs`, `views/welcome/*`, and `slash/mod.rs`. This is the first real conflict with grok-fork.
4. **`grok-pi` bin and the minimum extensions.**
   - Touches `xai-grok-pager-bin/Cargo.toml` and `src/bin/grok-pi.rs` plus `grok_pi/*`.
   - Touches `extensions/` for bash, rpc-compat, native-commands, tools, and ask-user.
   - Which extensions are the minimum for a faithful baseline is unverified.
5. **Tool rendering.** `scrollback/blocks/tool/{codemode,eval}.rs`, `tracker.rs`, and `scrollback/state/nav.rs`. Resolve against grok-fork's `_meta.pi.parentToolCallId` path.
6. **Model, context and session.** `model_state.rs`, `cache_graph`, the session-info and resume catalog, `pi_fork` and `tree`.
7. **Subagents.** Adapter projection and transport, the extension, and `subagent_catalog_pane`. Do this after Claude's bridge is understood, since it is the second big conflict.
8. **Settings and themes.** `pi_settings`, `pi_config`, `pi_models`, and the `pi-themes` assets.
9. **dpi shim.** Cherry-pick `5c374deb`, `01a8039a`, `0859bfec`, `8f376ca1`.
10. **Hub and installer.**
    - Point the `pi` entry in `scripts/hub/gb:76` at the new path and retire `pi-acp-patched`.
    - Remote Pi over ssh needs a stdio ACP entry for the adapter. grok-pi has none, since `PiAgent` is only wired through in-process channels. Whether `xai-acp-lib` can host it is unverified.
    - Update `docs/HARNESS-FACTS.md`, which already cites the adapter's `tool_projection.rs`.
11. **Later, if wanted.** Replay the 37 Dwsy commits that `main` has and the branch lacks. Examples: `28ad8eaa` per-agent models, `66b7d385` goal lifecycle, `7f26cb60` startup attach.

Not verified:
- Whether the branch builds at all.
- Whether the adapter links against grok-fork's `xai-acp-lib` and `xai-grok-shell`.
- Hunk-level conflict severity.
- What `pi-acp-patched` does that the adapter does not.

Risk: the only precedent for a `2bdd1d6a` port is Dwsy's own 7-commit fix chain (`ea1a80e3` through `719ba57f`). Expect that kind of compile-fix work again on top of grok-fork's 60k lines of changes.
