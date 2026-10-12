# Loop owner brief: branch gb-session-resume (task A)

You own this task end to end: worker AND verifier. Iterate until VERIFIED on screen, then merge. Do not stop at a report.

Read BRIEF.md and OBSERVATIONS.md. Current state: worker commit 2e25f334 (merger fixes applied); pi-acp side at ~/repos/Pi-Coding-Agent/.worktrees/gb-session-resume (37b8407, real context window). 19 xai-grok-pager lib test failures are pre-existing on main; ignore those exact ones.

Loop:
1. Build (shared target ~/.cargo-target, debug=0 profile; do not override; run `df -h /` first, stop under 8G). Copy the pager binary out of the shared target before running it.
2. Run the real flow in a pty (tmux): `grok pi` pointed at the PATCHED pi-acp (~/repos/Pi-Coding-Agent/.worktrees/gb-session-resume/pi-acp-patched; if `grok pi` uses /usr/bin/pi-acp, find the config/env that selects the adapter binary and point it at the patched one for this run; fix node_modules symlink locally, uncommitted). Two turns, quit, resume via picker (F3) and via `--resume`. Criteria: history replayed in order without duplicates, cwd shown, token counter non-zero.
3. If anything fails: fix it, add/adjust a test, commit, repeat from 1.
4. When all three criteria hold on screen: write VERIFIED with the evidence (what you saw, commands) to OBSERVATIONS.md, commit, push, then merge into main in ~/repos/grok-build (`git pull --rebase`, `git merge --no-ff gb-session-resume`, push) and the pi-acp branch into Pi-Coding-Agent main likewise. Trailer: `Co-authored-by: DSH + Claude Opus 5.5`.
5. Append a one-line status to STATUS.log in this worktree at every step change (building / testing / pty-run / fixing <what> / VERIFIED / REJECTED / MERGED <hash>). This is how the parent watches you.

Rules: no open -a/osascript/send-keys to desktops; never print or commit API keys; no deleting outside this worktree and the shared target; `git stash` is off limits. Another Opus is working branch gb-tui-controls in the same shared target dir; the cargo lock serialises you, wait for it.

# Delegation
Sab: "feel free to have Opus using Haiku subagents or Sonnet 5.5 subagents so that it's not doing all the heavy lifting". Use the Agent tool with model haiku for reads, greps, log tailing, test-output triage, and sonnet for bounded code edits and test writing. Keep the judgment (what passed on screen, merge decision) yourself.
