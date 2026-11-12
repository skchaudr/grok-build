# Cleanup sweep backlog (Sab, 2026-11-12)

Items Sab raised during the sweep; not yet specced or reviewed.

- **Subagent chat in grok claude.** "in claude code I can interact with subagents but in grok claude, the subagent view, it could easily have an input prompt box for chat in the subagent view but doesn't." Same hidden-prompt pattern as Ask (`SPEC-traces-agent.md`).
- **Resumed subagents vanish in grok claude.** A Claude Code subagent continued via SendMessage after it finished is visible in Claude Code but has no row in grok claude's agent view (seen 13:37-13:39 with two resumed background agents).
- **protoc on the Mini.** Homebrew protobuf 35.1 links abseil 20260107; abseil 20260817 is linked now, so `xai-grok-tools-api`'s build script fails. `brew upgrade protobuf` (36.2) awaits Sab.
- **pig: `/model` ignores Esc.** The next typed command becomes the model argument (`Unknown model: /settings`), on both Macs. Fix location untraced (`docs/SCOUT-pig-2026-11-12.md`).
- **pig on sab-air runs Homebrew pi 1.4.0** (inferred), not the managed 1.5.0; `grok-pi-acp` calls bare `pi`.
- **pig status-line spam.** `Running shortcut …` (`extensions/pi-grok-shortcut-manager/dispatch.ts:24`), plus `daily-memory` and `Usage cleanup` at startup.
