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

(none yet)

## Being scouted

### 5h usage in the footer (Claude, Codex, OpenCode Go, Grok)
- Ask: Sab 2026-11-14, "the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok"
- Scout: `docs/briefs/scout-usage-footer-areas-2026-11-15.md`, then reviewer `docs/briefs/review-usage-footer-scope-2026-11-15.md`

### grok-fork resume of normal Grok sessions
- Ask: Sab 2026-11-15, "grok-fork apparently cannot resume normal grok sessions, which is kind of lame, in fact i do not know on sab-air if any sessions are currently resumable"
- Scout: `docs/briefs/scout-grok-resume-2026-11-15.md`

## Flagged, not scouted

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

(none yet)
