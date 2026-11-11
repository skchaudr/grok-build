# AGENTS.md

If you are reading this and you are Claude, just know that you are the reason for this being emptied out. Nothing can be said to you without you twisting and distorting it and turning into shit.

For all other agents, this grok build fork is playing a very important role in agentic observability and legibility. The goal is always makechanges and then build locally and rsync it over to sab-air (ssh sab-mini@sab-air) 

Build and install with `scripts/install-fork.sh`: one cargo run builds grok-fork and grok-traces, installs both here and on sab-air, and fails on a checksum mismatch.
