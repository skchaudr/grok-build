# Queue: changes to make before the next install run

One build cycle ships everything under `## Queued`: targeted `--release` tests, then `scripts/install-fork.sh`. An item is queued only after a scout mapped it and a reviewer validated its size and scope.

Entry format:

```
### <short name>
- Ask: <Sab's words, dated>
- Scope: <files, rough size>
- Evidence: <scout doc> / <review doc>
- Needs: <tests to add or regenerate; anything outside this repo>
```

## Queued

### Footer shows status, not keybinding hints (cache, Claude usage first)
- Ask: Sab 2026-11-14, "4 useless keybindings that `?` and `Ctrl-,` easily overrule in terms of value ... the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok". Sab 2026-11-15, "Its even worse: y : copy Enter: open j/k:nav Shift+l/h:turn Ctrl+x: shortcut. There is the input editr footer and then navigational one where it shows defaults. I already have a command palette, what I do not have is a warm cache signal. Why not include cache, claude usage, code, opencode"
- Decided by those words: both footers (typing and scrollback navigation) stop showing default-key hints; the command palette and shortcuts help cover them. The row carries cache warmth, then Claude 5h/7d, then Codex and OpenCode as each gets a source.
- Scope reviewed so far (Claude usage only): `views/shortcuts_bar.rs`, `app/agent_view/render.rs`, `views/side_panel.rs`, about 40-60 lines, pager only. Evidence: `docs/SCOUT-usage-footer-areas-2026-11-15.md` / `docs/review/usage-footer-scope-2026-11-15.md`. That review kept the hints because its brief left out Sab's hint sentence; removing them was not sized (hints are built in `views/agent.rs` around line 1286 for typing, and from `When::ScrollbackFocused` actions in `actions/defaults.rs` for navigation).
- Not sized yet, cache warmth: interactive Claude Code hands its statusline a `prompt_cache` object (`caching_observed`, `warm`, `expires_at`); `~/.claude/statusline-command.sh:16` and `cstat` print "warm 59m" / "cold" from it. Nothing in the pager holds an equivalent; whether the Claude adapter can report it under `grok claude` is not traced.
- Needs: footer render test at full and half width in every theme (`GROK_THEME=<name>`), looked at on screen, typing and navigation footers both.

## Being scouted

(none)

## Flagged, not scouted

### Grok sessions in the resume picker
- Ask: Sab 2026-11-15, "grok-fork apparently cannot resume normal grok sessions, which is kind of lame, in fact i do not know on sab-air if any sessions are currently resumable"
- Scouted: `docs/SCOUT-grok-resume-2026-11-15.md`. Plain `grok-fork` lists Grok sessions (8 of 21 on the Mini; the 13 hidden ones have 2 lines of history or fewer). `grok claude` lists only Claude sessions, because it asks only the Claude adapter (`app/effects/mod.rs` external branch, `dispatch/session/foreign.rs:65`). sab-air has 21 Grok sessions on disk. Loading a session was not exercised.
- Open: showing Grok rows under `grok claude` is a listing change, but opening one there means switching the running agent from Claude to Grok; that part is not mapped.

### Slash commands in grok claude go through the model
- Ask: Sab 2026-11-14, "If I want to make a deterministic change like changing effort level, it HAS to go the model first, it gets queued"
- Known: `docs/HARNESS-FACTS.md`; the settings rows already change effort with no model turn. Where a typed `/effort` gets forwarded as prompt text is not traced.

### 7d usage has no fallback
- Seen on screen 2026-11-14: F3 Info shows "7d usage not reported" until Claude sends it live; the statusline file has a 7d figure the panel does not read (`views/side_panel.rs`, `claude_five_hour_pct`). Likely absorbed by the footer usage change.

### Pi feature rows show under Claude
- Seen on screen 2026-11-14: Settings, Agent tab under `grok claude` still lists "Pi features" (Ask-Question timeout, Pi config skill, Pi resources). Those rows are not marked Pi-only.

### Footer snapshot tests depend on the terminal
- Three side-panel snapshots (`closed`, `narrow_terminal_unfocused_panel_hides`, `open_unfocused_lets_the_chat_type`) expect `Ctrl+.:shortcuts`; run inside Rex they render `Ctrl+x:shortcuts` (`actions/defaults.rs`, `terminal/mod.rs` `kitty_skip_reason`). Will be touched anyway if the footer changes.

### One tailored TUI per host
- Sab 2026-11-15: "I am surprised we are sticking to ONE TUI for grokclaid. Like grok claude could have a full claude talored settings modal, grok pi could have its own, prime agent has its own, which we will get to eventually, recent changes were made for prime"
- Today: host profiles exist for Pi and Claude (`install_pi_host_ui`, `install_claude_host_ui` in `xai-grok-pager-bin/src/main.rs`; `HostFeatureManifest` in `xai-grok-shell/src/host_features.rs`). They set brand, welcome and changelog; settings are one shared modal with per-host rows shown or hidden. Prime has rendering and resume fixes (`caec749c`, `2f969e3b`) and no host profile.

## Not queued

- Fresh Claude usage via adapter (`get_usage` at session start and turn end): RESCOPE. Needs Sab's call on where the `~/.dsh` patch lives, a shape conversion (0-100 and ISO vs 0..1 and epoch), and a `usage_update` that does not zero the context meter; conflict with patch 0002 untested. Review: `docs/review/usage-footer-scope-2026-11-15.md` F6.
- Codex 5h/7d in the footer: RESCOPE. Source is real (rollout `token_count` `rate_limits`) but no scout found which process runs Codex under grok or which rollout is the session's; scout the host path first. F7.
- OpenCode in the footer: reopened 2026-11-15. The DROP was wrong: the reviewer and scout looked only on the Mini. On sab-air `opencode` 1.19.4 is installed (`/opt/homebrew/bin/opencode`), with `~/.local/share/opencode/opencode.db` and an `opencode stats` command (token usage and cost). No 5h percentage found yet; needs a scout on sab-air.
- Grok in the footer: `usage_pct` (`credit_bar.rs`) is a credit allowance, not a 5h window; needs its own label if shown.
