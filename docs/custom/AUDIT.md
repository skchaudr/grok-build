# Grok Build native-feature audit

Scope: features Sab can use while routing Claude Code, Codex, Cursor and Grok agents through Grok Build. Branch `custom`, 2026-10-20. Paths are under `crates/codegen/` unless prefixed `~`. Not built or run.

## Memory (legacy + v2)

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Legacy store: `MEMORY.md` global, `{blake3(cwd)[..16]}/MEMORY.md` per workspace, `sessions/*.md`, `index.sqlite` | `xai-grok-memory`; `~/.grok/memory/` | Off unless enabled | Default off |
| v2 store: `topics/`, `observations/_inbox/`, generated `MEMORY.md` | `xai-grok-memory/src/v2.rs`; `~/.grok/memory-v2/` | Preferred store once `[memory_v2] enabled = true` | Enable is an explicit opt-in (see below) |
| Scopes: global + workspace; worktrees of one repo share scope | `xai-grok-memory` | Our worktrees share memory automatically | Verify sharing on the `.worktrees/*` layout before relying on it |
| Index injected once per session | `xai-grok-shell/src/session/helpers/memory_context.rs:16,76` | Automatic | Injection is first-turn only; mid-session facts need `memory_search` |
| Recall tools `memory_search` / `memory_get`: hybrid vector+text, MMR, temporal decay | `xai-grok-tools/src/implementations/memory/mod.rs`; `xai-grok-config-types/src/memory.rs:264-341` | Agent pulls facts on demand | Decay/MMR settings are config-only, no UI |
| Writes: `/remember` (review panel), `/flush` (LLM summary), `/dream` (consolidate to topics) | slash layer in `xai-grok-pager` | Manual curation | `/remember` target under v2 unverified (docs describe legacy target) |
| Automatic per-turn capture to v2 inbox | `xai-grok-shell/src/session/acp_session_impl/memory_capture.rs`; call site `acp_session_impl/run_loop.rs:2343` | Background capture for Grok-native sessions | Gated by `can_capture_v2()` and `!is_subagent` (`memory_capture.rs:493`); see ACP finding below |
| NL "remember" / "forget" | shell | Ad hoc | `forget` is best-effort |
| Config: `[memory] enabled`, `[memory_v2] enabled / capture_enabled / automatic_dream_enabled`, `[memory.session] save_on_end`, `[memory.search]`; env `GROK_MEMORY`; flag `--no-memory` | `xai-grok-config-types/src/memory.rs:676-700` | Turn on v2 explicitly | See default finding |
| `/memory` modal | `xai-grok-pager/src/views/memory_modal.rs` | Inspect and toggle | Toggle is session-scoped, not persisted (`docs/user-guide/13-memory.md:74`) |

Verified in this pass:
- Defaults: `[memory] enabled` and `[memory_v2] enabled` are both off. `v2_enabled` has no default (`memory.rs:681`), so v2 is selected only when set true (`memory.rs:692-700`). `MemoryV2Rollout::Active` (`memory.rs:71`) is the rollout field, not the enable gate. Docs (`13-memory.md:95`) are correct; the "docs vs code" concern is resolved.
- Local `[memory] enabled = false` disables both implementations unless `[memory_v2] enabled = true` is also in the same TOML (`memory.rs:685-695`).
- ACP / external-agent sessions: the only `memory_capture` call site is `run_loop.rs:2343`, inside Grok's own `SessionActor`. External Claude/Codex/Cursor agents are spawned as stdio subprocesses bridged by `xai-grok-pager/src/acp/spawn.rs:362` (`spawn_external_subprocess`). No path from that bridge into `memory_capture` was found. Conclusion: memory capture does NOT cover Claude/Codex/Cursor turns. Not verified end-to-end at runtime.

## /learn (server-delivered skill, not in repo)

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Not in repo source; fetched at runtime from `{cli_chat_proxy}/subagents/bundle` | `xai-grok-shell/src/remote/client.rs:75-90`; sync `xai-grok-shell/src/extensions/bundle.rs:160-215` | Runs `/learn` after sessions | Behavior is opaque: cannot audit the skill source from this repo |
| Cache `~/.grok/bundled/` (manifest `public-2026-10-20-r1`); user-edited files not overwritten | `bundle.rs` | Local edits survive updates | Version pinned by server |
| Skill files: `SKILL.md`, `collect_sessions.py`, `state.py`, `learn-traces.rhai` | `~/.grok/bundled/skills/learn/` | Read to see exactly what it collects | Read them before first run |
| Input: only `<GROK_HOME>/sessions/<enc_cwd>/<sid>/{summary.json,chat_history.jsonl}`; skips subagent, headless, temp | `collect_sessions.py` | Grok-native sessions only | Blind to Claude/Codex/Cursor |
| Workflow: map batches, reduce, 3 verifiers, report (Rhai engine `xai-workflow`, host fns agent/parallel/phase/pause/await_user/budget/log) | `xai-workflow/src/engine.rs` | Gated by user consent at apply | Cost per run not bounded in this audit |
| State: `~/.grok/learn/{state.json,decisions.jsonl,runs/<stamp>/ (30d prune),trash/}` | `~/.grok/learn/` | Audit trail | Run dirs pruned after 30d |
| Output `report.md` + `actions.json`; apply (with consent) creates/edits `~/.grok/skills/*/SKILL.md`, toggles plugins in `config.toml`, trashes skills/workflows/hooks | `learn-traces.rhai` | Review before apply | Does NOT write memory or AGENTS.md |
| Args: `--mode step|auto|report`, `--days`, `--since-last`, `--limit`, `--focus`, `--resume`; unattended `/loop 24h /learn` or `grok -p "/learn"` | slash/loop | Scheduled learning | Unattended `auto` mode can apply changes; keep `report` for unattended runs |

## Overlap: /learn vs our /traces

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| `/traces` (ours): hook-fed index of Claude, Codex, Cursor, bb and Grok sessions | `xai-grok-traces` crate; `/traces` modal | Cross-harness session history | Not consumed by `/learn` |
| `/learn` reads only Grok-native sessions | `collect_sessions.py` | Misses 3 of 4 harnesses | Structural gap |
| Proposal: feed `collect_sessions` from the traces index, or export traces sessions into the `sessions/<enc_cwd>/<sid>/` layout | new adapter (not built) | Learning covers all agents | Layout and `summary.json` schema must be matched; proposal only |

## Foreign sessions and picker

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Read-only metadata listing; `ForeignSessionTool {Claude, Codex, Cursor}` | `xai-grok-foreign-sessions/src/lib.rs` | Browse other-harness sessions from Grok | Read-only |
| Claude: `<config>/projects/<sanitized cwd>` | `foreign-sessions` | Listed | Path sanitization not re-verified |
| Codex: `$CODEX_HOME` or `~/.codex` (rollouts + sqlite) | `foreign-sessions` | Listed | Sqlite schema drift risk |
| Cursor: STUB; `scan_cursor` returns empty | `foreign-sessions/src/lib.rs:101,109` | Nothing | Implement using our traces Cursor reader |
| Gate per vendor: `[compat.<vendor>] sessions` | `xai-grok-config/src/compat.rs:595-607` | Enable Claude/Codex here | Must be set per vendor |
| Pager: `app/foreign_sessions.rs`; welcome picker `views/welcome/mod.rs:713` | `xai-grok-pager` | Picker UI | - |

## Session search

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| FTS5 index at `<grok home>/sessions/session_search.sqlite` | `xai-grok-session-search`; `Feature::SessionSearch` | Search past Grok sessions | Grok-native only |

## /dashboard

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Agent/session dashboard; aliases `agents-dashboard`, `sessions`; Ctrl+\\ | `xai-grok-pager/src/slash/commands/dashboard.rs` | Overview of running agents | Ctrl+\\ key not remappable (see Keybindings) |
| Store `<grok_home>/dashboard/workspace.db`; cap 256 LRU unpinned; grouping State or Directory | `xai-grok-dashboard-store` | Pin keepers | Cap evicts unpinned rows silently |
| `[dashboard].enabled` default true; `GROK_AGENT_DASHBOARD=0` disables | config | - | - |

## Skills

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Discovery: project `.grok/skills`, `.agents/skills`, `.claude/skills`, `.cursor/skills`; user `~/.grok`, `~/.agents/skills`, `~/.claude/skills` | `xai-grok-agent/src/prompt/skills.rs:158-200` | One skill tree serves all four harnesses | Vendor dirs compat-gated |
| Scope precedence: Local > Repo > User > Server > Bundled > Plugin | `skills.rs` | Override per repo | - |
| Frontmatter: name, description, when-to-use, user-invocable, disable-model-invocation, allowed-tools, model, effort, paths, origin | `skills.rs` | Standard SKILL.md | - |
| Invoke via Skill tool or `/name`; `[skills] paths / disabled / ignore`; `/skills` modal | config, pager | - | - |

## Hooks

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| 15 events: session_start, user_prompt_submit, pre/post_tool_use, post_tool_use_failure, permission_denied, stop, stop_failure, stop_cancelled, notification, subagent_start/stop, pre/post_compact, session_end | `xai-grok-hooks` | Hook the same events across harnesses | Claude/Cursor event aliases must map exactly |
| Sources: `~/.grok/hooks/*.json`, `~/.claude/settings.json` (+local), `~/.cursor/hooks.json`, project `.grok/hooks` (trust required), `[hooks.<event>]` in config.toml | `xai-grok-hooks` | Reuse existing Claude/Cursor hooks | Project hooks need trust |
| Fail-open; `pre_tool_use` can allow / ask / deny / updatedInput | `xai-grok-hooks` | Guardrails | Fail-open means a crashing hook does not block |
| `/hooks` modal; disable list `$GROK_HOME/disabled-hooks` | pager | - | - |

## Plugins and marketplace

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Dirs: `.grok/plugins`, `.claude/plugins`, `~/.grok/plugins`, `~/.claude/plugins`, `~/.grok/installed-plugins` | `xai-grok-plugin-marketplace` | Reuse Claude plugins | - |
| `[[marketplace.sources]]`; `/plugins`, `/marketplace` | config, pager | - | Third-party sources unaudited |

## Themes

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Built-in Rust consts | `xai-grok-pager-render/src/theme/` | `[ui].theme`, `GROK_THEME`, `/theme` | No user theme files |
| Auto: `auto_dark_theme` / `auto_light_theme` | config | - | - |
| NEW on branch: kanso-mist, kanagawa-wave, kanagawa, jetbrains-islands-dark | `docs/custom/THEMES.md` | Our palette | Ports, not re-verified against Zed sources |

## Keybindings

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Defaults only | `actions/defaults.rs`; `docs/user-guide/03-keyboard-shortcuts.md:3` | Accept defaults | NOT user-remappable. Biggest gap for multi-harness muscle memory |
| Ctrl+O = yolo toggle | defaults | - | Claude Code's ctrl-o transcript is unavailable |

## Copy and transcript

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| `y` copies block content; `Y` path or command; `/copy N`; block viewer (Enter / Ctrl+F) has `v` visual range + `y` | pager scrollback | Keyboard copy | No keyboard text cursor in scrollback |
| Mouse drag selection | pager | - | - |
| `/transcript` opens transcript in `$PAGER` (less) | pager | Keyboard-copy path with search | Plain pager, no Grok styling |

## Editor integration

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Ctrl+G edits the prompt in `$VISUAL`/`$EDITOR` (minimal mode) | pager | - | - |
| Config files via SuspendForEditor | pager | - | - |
| NEW: `v` on Read/Edit block opens file at line. Order: `$GROK_OPEN_EDITOR` > `$VISUAL` > `$EDITOR` > zed. GUI editors detached; terminal editors in new Herdr pane; else suspend (no line) | `src/app/open_in_editor.rs` | Jump to diff lines | Suspend fallback loses line number |
| OSC8 editor schemes `zed://`, `vscode://` | `xai-grok-pager-render/src/terminal/hyperlinks.rs:29-36` | - | `EditorExtended` has no production caller: dead path |

## Status line

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| `[ui.status_line]` type disabled / builtin / command; command gets JSON on stdin | `xai-grok-status-line`; `docs/user-guide/25-status-line.md` | Command type can show harness name and traces state | - |

## Config

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| `$GROK_HOME/config.toml` (`~/.grok`); project `.grok/config.toml`; managed and requirements layers | config | Layered | - |
| Reference | `docs/user-guide/26-config-reference.md` | - | - |

## Import from Claude

| What | Where | How Sab uses it | Gaps |
|---|---|---|---|
| Imports Claude settings only: permissions, env, hooks, MCP | `xai-grok-external-agent-migration`; `views/import_claude_modal.rs` (`/import-claude`) | One-time migration | Does not import Claude sessions or memory |

## Upstream and history

- No upstream remote in this clone (only `origin` = `skchaudr/grok-build`). Last synced upstream rev `2bdd1d6a` (`scripts/upstream-sync.rev`).
- History is squashed ("Synced from monorepo"): per-feature history is not recoverable from git.

## Top recommendations for Sab

- Decide on memory: enable `[memory_v2] enabled = true` deliberately, since it is off by default. Accept that capture does not cover Claude/Codex/Cursor turns (see Memory findings).
- Bridge `/traces` into `/learn`: feed `collect_sessions` from the traces index so learning covers all four harnesses. Proposal, not built.
- Enable `[compat.claude]` and `[compat.codex]` sessions so the foreign-session picker lists them.
- Implement the Cursor foreign-session scan (currently a stub at `foreign-sessions/src/lib.rs:101,109`) using our traces Cursor reader.
- Keybinding remap is not supported. Treat it as a permanent gap and document per-harness shortcuts instead.
- Use `/transcript` (`$PAGER`) as the keyboard-copy path for long outputs, since `y` covers only single blocks.
