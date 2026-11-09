# grok-pi harness: progress against the map (2026-11-09)

Brief: `docs/traces/briefs/grok-pi-harness.md`. Map: `docs/grok-pi-dual-agent-map.md`.

## Step 1: adapter, extensions, `grok-pi-acp`

- `crates/codegen/pi-grok-adapter` and `extensions/` are Dwsy's 719ba57f, unedited.
- `xai-grok-shell/src/session/workflow/*` and `session/mod.rs` are Dwsy's 719ba57f (our side had not touched them since 2bdd1d6a), except `registry.rs` keeps `.grok` instead of `project_config_dir`, which our `xai-grok-config` lacks. `spawn.rs` wraps the subagent sender in `GrokSubagentBackend`, as Dwsy's does.
- `xai-grok-pager-bin/src/bin/grok-pi-acp.rs` is the stdio host. It reuses Dwsy's `grok_pi/*` extension writers unedited, except `session_paths.rs`, which inlines the Pi agent dir lookup.
- Unlike Dwsy's `grok-pi.rs`, the host has no F2 resource admission policy (Pi auto-discovers its own extensions, prompts and themes), no Pi theme discovery, tutorial, updater or extension self-heal, and no web-config or config-skill extensions. Those belong to step 6 (settings).
- Host-feature switches read `[ui].pi_*` from the grok config, using the `grok-pi.json` defaults: subagents and todo on, the rest off.

Picker entry (Sab's config, not edited by the agent):

```toml
[[agent_choice]]
name = "grok-pi"
cmd = "/path/to/grok-pi-acp"
```

Or `GB_PI_ACP=/path/to/grok-pi-acp gb claude pi`.

### On screen (unchanged pager)

Standalone (`--agent-cmd grok-pi-acp`):
- Works: the session opens, the footer shows Pi's model, `/model` lists Pi's 542 models, and Pi's skills, `/plan`, `/plan-view` and `/subagents-*` are in the slash menu. `/exit` cleans up the adapter and its `pi` child.

Picker (private leader, `claude` + `grok-pi`):
- Works: both adapters run under one leader, and a grok-pi session opens cold from the picker, with the header showing `grok-pi`.

Port list:
1. In the picker's grok-pi session, the slash menu has Claude's shape: 242 entries, Claude plugin commands, and none of Pi's `/skill:*` or `/subagents-*`. The menu profile is app-wide (step 2).
2. `/tree`, `/tree-map`, `/fork`, `/clone`, `/reload`, `/notify`, `/pi-models`, `/pi-config` and `/plan-mode` are missing. grok-pi provides them in its pager (step 5).
3. Grok-only builtins (`/usage`, `/settings`, `/theme`, …) show for Pi (step 5).
4. `/resume` lists no Pi sessions; grok-pi lists them through `pi/session/list` in its pager.
5. The welcome screen shows Grok branding, not Pi's (grok-pi's `ExternalUiProfile`, a pager feature).
6. Unverified: tool cards, `pi/ui/*` widgets and subagent rendering all need a live turn, and paid turns are Sab's.

## Open items from the map

1. Answered: the adapter compiles against grok-fork once Dwsy's `session/workflow` changes are in (8 errors before, all in workflow host types).
2. Answered: `new_session` works cold. It asks the running Pi for a new session, and it opened from the leader's picker on screen.
3. Answered: no forwarder is needed. `PiAgent::new` takes `UnboundedSender<AcpClientMessage>`, which is `AgentSide::OutMessage`, the type `AcpAgentGatewayReceiver` drains.
4. Answered from code: the leader relays agent-to-client messages without looking at the method. Anything carrying `params.sessionId` goes to that session's subscribers, and anything without one goes to the last active client (`leader/server.rs` ~3232-3380). Not seen live.
5. Answered from code: a backend the picker spawns gets a bare `initialize {"protocolVersion":1}` from the leader (`server.rs` ~2623). So a Claude that is not the launch agent gets no `jetbrains` native-subagent meta.
6. Partly answered: the defaults load 12 bridges, plus tools and rollback when their switches are on. Which of them a faithful Pi needs is unverified until there is a live turn.
