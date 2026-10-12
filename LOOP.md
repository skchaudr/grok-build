# Loop owner brief: branch gb-tui-controls (task B)

You own this task end to end: worker AND verifier. Iterate until VERIFIED on screen, then merge. Do not stop at a report.

Read BRIEF.md and OBSERVATIONS.md. pi-acp side: ~/repos/Pi-Coding-Agent/.worktrees/gb-tui-controls (5a46272). A previous verifier found: slash menu and `/usage` under `grok pi` work; `/model` pick fails with "Method not found: session/set_model" (pi-acp lacks it, and the footer wrongly shows the new model); `/usage` and `/context` under `grok claude` print nothing ("Worked for 0.4s"). 19 xai-grok-pager lib failures are pre-existing on main; ignore those exact ones.

Loop:
1. Build (shared target ~/.cargo-target, debug=0; do not override; `df -h /` first, stop under 8G). Copy the pager binary out before running it.
2. Fix: implement `session/set_model` in pi-acp (patched copy) so the pick takes effect, and make the footer only change on success; make `/usage` `/context` under claude render the agent's reply or a visible "not supported by claude" notice. Failing test first for each.
3. Run in a pty (tmux): `grok pi` with the PATCHED pi-acp (if `grok pi` uses /usr/bin/pi-acp, find the config/env that selects the adapter binary and point it at the patched one for this run; fix node_modules symlink locally, uncommitted) and `grok claude`. Criteria from BRIEF.md: slash popup lists Pi commands + common set; `/model` lists Pi models and the NEXT reply really comes from the chosen model (check the session file); `/usage` shows output or an unsupported notice in both.
4. Fail → fix → repeat. Pass → VERIFIED with evidence in OBSERVATIONS.md, commit, push, merge into ~/repos/grok-build main (`pull --rebase`, `merge --no-ff gb-tui-controls`, push) and pi-acp branch into Pi-Coding-Agent main. Trailer: `Co-authored-by: DSH + Claude Opus 5.5`.
5. Append a one-line status to STATUS.log in this worktree at every step change (building / testing / pty-run / fixing <what> / VERIFIED / REJECTED / MERGED <hash>). This is how the parent watches you.

Rules: no open -a/osascript/send-keys to desktops; never print or commit API keys; no deleting outside this worktree and the shared target; `git stash` off limits. Another Opus works gb-session-resume in the same shared target; the cargo lock serialises you, wait for it.

# Delegation
Sab: "feel free to have Opus using Haiku subagents or Sonnet 5.5 subagents so that it's not doing all the heavy lifting". Use the Agent tool with model haiku for reads, greps, log tailing, test-output triage, and sonnet for bounded code edits and test writing. Keep the judgment (what passed on screen, merge decision) yourself.
