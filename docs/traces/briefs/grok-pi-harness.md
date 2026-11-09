# grok-pi as a grok-fork harness (2026-11-09)

Repo: /Volumes/Mini-SSD/offload/repos/grok-build, branch main. Work in place.

Plan: `docs/grok-pi-dual-agent-map.md`, written by an earlier Opus agent. Execute its "What is left, in order" table, steps 1 to 6, in its order. Step 1 runs the adapter against the unchanged pager first; what renders wrong is the port list for the later steps.

Sab's words: "I meant for grok pi to be one harness grokbuild our fork chooses from, like completion." "Leave the themes but sure they actually made their own settings." So step 6 = grok-pi's settings, no themes. The dpi shim is not asked for.

Rules:
- Answer the map's "Open, unverified" items as you reach them.
- Tests and builds run visibly in herdr pane w3E:p4 (`herdr pane run w3E:p4 "clear; cd /Volumes/Mini-SSD/offload/repos/grok-build && nice -n 10 cargo ... && echo TESTS-DONE || echo RUN-FAILED"`, then `herdr pane wait-output w3E:p4 --regex 'TESTS-DONE|RUN-FAILED' --timeout 3600000`). One build at a time, --release.
- Verify on the real screen in your own herdr tab (`herdr tab create --workspace w3E --cwd <repo> --label grok-pi-check`), read it with `herdr pane read`. Never send a prompt to any agent (paid turns are Sab's). Never type letters into the traces list. Close your tab when done.
- Do not install binaries or edit ~/.grok, ~/.pi, ~/.claude.
- Commit on main and push as each step lands; every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Report tightly per step: what's on screen, tests read from the pane, commits, unverified items, any departure from the map.
