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

## Step 2: per-session agent profile (2003e4e6)

- Each `AgentView` keeps `agent_label`, taken from its picker command. `AppView::agent_label(id)` resolves the picker label first, then the name the backend reports, then the launch agent's. `is_claude_session`, the new `is_pi_session`, rewind, fork, the slash label, child steer, usage tally and the effects' session flags all read it.
- Shortcut: a session resumed from the dashboard has no picker command. If its backend reports no name, it takes the launch agent's label.

Leader fix found on screen (1800daf9): grok-pi-acp sends a new session's commands, model and title before its `session/new` reply. The leader dropped them as unrouted (`server.rs`, "relay-owned session"). It now holds them while an external `session/new` is in flight and sends them after the reply.

On screen (private leader, `claude` + `grok-pi` picker): the grok-pi session shows Pi's 126-entry menu with `/skill:*` and `/subagents-*`, and the Claude session in the same pager keeps Claude's menu, with no Pi skills.

## Step 3: `pi/ui/*` (18f46cb3)

- `acp_handler/pi_ui.rs` is hooked as the fallback arm of `handle_ext_notification`.
- Ported onto native surfaces: status goes to the status bar; widget goes to new rows above the prompt (`pi_widgets` layout row); notify goes to scrollback (info) or a toast (warning, error); title, editor_text, cancel_interaction, compaction_summary (as a system block) and btw_history are handled; reset_extension_ui clears the statuses and widgets.
- Acknowledged with no surface: `remote_tui`, `shortcuts`, `session_catalog` and `plan_file`. Because of that, grok-pi-acp now defaults Remote TUI off (`PI_GROK_REMOTE_TUI=1` turns it back on).
- Not seen on screen: no status or widget appeared at session start, and extensions emit them during turns.

## Open items from the map

1. Answered: the adapter compiles against grok-fork once Dwsy's `session/workflow` changes are in (8 errors before, all in workflow host types).
2. Answered: `new_session` works cold. It asks the running Pi for a new session, and it opened from the leader's picker on screen.
3. Answered: no forwarder is needed. `PiAgent::new` takes `UnboundedSender<AcpClientMessage>`, which is `AgentSide::OutMessage`, the type `AcpAgentGatewayReceiver` drains.
4. Answered from code: the leader relays agent-to-client messages without looking at the method. Anything carrying `params.sessionId` goes to that session's subscribers, and anything without one goes to the last active client (`leader/server.rs` ~3232-3380). Not seen live.
5. Answered from code: a backend the picker spawns gets a bare `initialize {"protocolVersion":1}` from the leader (`server.rs` ~2623). So a Claude that is not the launch agent gets no `jetbrains` native-subagent meta.
6. Partly answered: the defaults load 12 bridges, plus tools and rollback when their switches are on. Which of them a faithful Pi needs is unverified until there is a live turn.
