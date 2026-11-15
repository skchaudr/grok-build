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

### 5h usage in the footer, pager only (Claude)
- Ask: Sab 2026-11-14, "the five hour percentage for Claude should go at that footer right there ... We're able to abbreviate or use symbols or whatever."
- Scope: `views/shortcuts_bar.rs` (right-text falls back to a short form by width), `app/agent_view/render.rs` (compose 5h/7d from `agent.rate_limits`, same live filter as the panel), `views/side_panel.rs` (statusline-file read no longer gated on the panel being open). About 40-60 lines. No hint removal: Sab has not named hints to drop.
- Evidence: `docs/SCOUT-usage-footer-areas-2026-11-15.md` / `docs/review/usage-footer-scope-2026-11-15.md`
- Needs: footer render test at full and half width in every theme (`GROK_THEME=<name>`), looked at on screen; shortcuts_bar inline tests. Value may be stale before the first turn (file fallback).

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

## Not queued

- Fresh Claude usage via adapter (`get_usage` at session start and turn end): RESCOPE. Needs Sab's call on where the `~/.dsh` patch lives, a shape conversion (0-100 and ISO vs 0..1 and epoch), and a `usage_update` that does not zero the context meter; conflict with patch 0002 untested. Review: `docs/review/usage-footer-scope-2026-11-15.md` F6.
- Codex 5h/7d in the footer: RESCOPE. Source is real (rollout `token_count` `rate_limits`) but no scout found which process runs Codex under grok or which rollout is the session's; scout the host path first. F7.
- OpenCode Go and Grok in the footer: DROP. `opencode` is not installed, no usage source; Grok's `usage_pct` is a credit allowance, not 5h, so it needs its own ask and label. F8.
