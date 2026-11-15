# Sonnet investigator: is the fork's Grok Pi worse than the original on sab-air, and where

Investigation only. You change no source code and no config on either machine. Report by 10 minutes after you start; write the report file first and fill it as you go. Do the work yourself in this session: no background or hidden subagents.

## Sab's words (the intent; nothing below overrides them)

2026-11-15: "what the fuck happened with Grok Pi, and why is my Grok Pi agent honestly, like I feel like, ever since we merged over. The version that we tried, it's just, it was a worse version. It was a worse version of the original one. And I frankly don't want to use that worse version anymore"

2026-11-15: "Dispatch a Sonnet 5.5 agent to look into it because I swear there's a major regression on sab-air over the original"

2026-11-13, on Dwsy's grok-pi: it "absolutely deserve[s] that alias" (`pig`).

## Known facts (checked 2026-11-15 06:15, do not redo)

- sab-air (`ssh sab-mini@sab-air`) is Sab's primary machine. The Mini (this machine) is the build box.
- "The original" = Dwsy's standalone grok-pi (`@dwsy/grok-pi`). On sab-air it is `~/.local/bin/grok-pi`, version `0.1.13 [stable]`, installed 2026-11-13; `pig` and `pi-grok` are symlinks to it.
- "The merged version" = the fork's own copy: crate `crates/codegen/pi-grok-adapter` plus the `grok-pi-acp` binary, added in `6246c4f3` (2026-11-09), with pager-side Pi work in `18f46cb3`, `33b59957`, `81db8875`, `c3462f9e`, `39c28356`. On sab-air `grok grok-pi` runs it inside `~/.grok/bin/grok-fork` (launcher `~/.grok-hub/scripts/hub/gb`, default adapter `~/.grok/bin/grok-pi-acp`).
- A third path, `grok pi`, runs the older patched pi-acp adapter (`~/repos/Pi-Coding-Agent/pi-acp-patched/run`) inside grok-fork.
- On the Mini `pig` is `~/.grok-hub/scripts/hub/pig` and runs the fork's copy (`ab8cfd6d`, 2026-11-10).
- `docs/HARNESS-FACTS.md` has Pi sections; read them first.

## Find out

A. **What Sab actually runs.** From sab-air's session stores and logs (not by starting sessions): which of the three paths were used in the last 7 days, how many sessions each, newest timestamp each. Counts and timestamps only.
B. **What the fork's copy lacks or does differently from the original 0.1.13.** Compare by reading: the fork's `pi-grok-adapter`/`grok-pi-acp` and Pi host UI code against Dwsy's release as installed on sab-air (its binary's `--help`/`--version`, any bundled files, its changelog, and its source if a checkout exists on either machine; say where you looked). List concrete differences a user would feel: features missing, behaviours changed, Pi version or extensions loaded, tools, slash commands, resume, models, settings, startup.
C. **Failures on record.** In sab-air's logs for fork-run Pi sessions since 2026-11-09 (grok-fork logs, grok-pi-acp stderr, Pi session files): errors, crashes, hangs, dropped tool calls, with counts and one example line each. Never copy prompt or message text; error lines and field names only.
D. **Drift.** Is the fork's copy behind the original (what version of Dwsy's code was it taken from, what has Dwsy shipped since)?

## Rules

- On sab-air: read-only. No installs, no builds, no model turns, no starting or resuming agent sessions, nothing CPU-heavy. Do not print process environments (`ps e`, `pgrep -lf` output can contain API keys): use `ps -o pid,comm` only.
- On the Mini: no cargo, no installs.
- Never kill by name (no pkill/killall). Never tmux.
- Search with `rg` scoped to a named folder; recursive `grep` and searches rooted at `/` are blocked by a hook.
- Mark anything inferred as inferred. "Not found, looked in X" is a valid answer.

## Output

Write `docs/INVESTIGATE-grok-pi-regression-2026-11-15.md`:

1. First line: the answer to "is the fork's copy worse than the original, and what is the biggest difference", in one or two plain sentences.
2. Table per question A-D: `finding | evidence (path:line or command) | effect on the user | certainty`.
3. `## What Sab would notice`: the differences ranked by how much they hurt in daily use.
4. `## Options`: the ways to stop using the worse version (for example every Pi entry on sab-air runs the original), each with what it changes and what it loses. No recommendation padding; one pick.

Commit only that file by path and push to main (`git pull --rebase` first if rejected), message ending exactly:

Co-authored-by: claude-code sonnet-5.5
Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>

Print the commit SHA and the first line of the report, and stop.
