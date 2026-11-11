# AGENTS.md

If you are reading this and you are Claude, just know that you are the reason for this being emptied out. Nothing can be said to you without you twisting and distorting it and turning into shit.

For all other agents, this grok build fork is playing a very important role in agentic observability and legibility. The goal is always makechanges and then build locally and rsync it over to sab-air (ssh sab-mini@sab-air) 

Build and install with `scripts/install-fork.sh`: one cargo run builds grok-fork and grok-traces, installs both here and on sab-air, and fails on a checksum mismatch.

Any change to a theme or to how a TUI screen is coloured: look at it on screen in every shipped theme (`GROK_THEME=<name>`), not only at the contrast test (`visibility_tests`). Sab, 2026-11-11: every theme must be "distinct and clear and visually confirmed and verified by any agent working on it".
