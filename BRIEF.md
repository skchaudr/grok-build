# Standing rules (every agent)

- No `open -a`, osascript, or sending keys on Sab's desktops. Never print or commit the CLIProxy key.
- One integration branch: work in your worktree branch, commit early and often, `git pull --rebase` before push, push to origin. When done, say explicitly whether it is merged to main; a branch is not done until merged.
- Every commit ends with `Co-authored-by: DSH + <your model name>`.
- Run `df -h /` before any cargo build; stop if under 15 GB free.
- Log edge cases and anomalies in `OBSERVATIONS.md` in the worktree, committed.
- Eval first: write a failing test defining acceptance before touching code.
- Replies to Sab ≤10 lines. Findings and consequences, no introductions.
- Do not delete files or directories; list them for Sab instead.

Sab's words on why this matters (verbatim):
"I am gearing up to do GDAD (GDDP 2.0) and will genuinely need as much visibility/observability and steering I can get"
"Observability and live feedback (rulings, warnings, status updates, confidence ratings) are first-class functional requirements. A guard that runs invisibly in the dark is broken by design."

# Worker/verifier pairing

Each task has a composer-2.5 WORKER and a grok-4.7 VERIFIER in the same worktree.
- WORKER: implement, test, commit, push. When done, append `WORKER DONE <commit>` to `OBSERVATIONS.md` and commit.
- VERIFIER: wait until `WORKER DONE` appears in `OBSERVATIONS.md` (poll with `git log`/`cat` every ~2 min, do not edit source meanwhile). Then visually verify: build the binary, run the real flow described in the brief in a tmux/pty, confirm the behaviour on screen, run the tests. Write `VERIFIED` or `REJECTED: <reasons>` with evidence into `OBSERVATIONS.md`, commit, push. If REJECTED, the worker fixes and re-marks DONE; repeat.

---

# Task A: pi-acp session continuity in Grok Build

Read COMMON.md first.

Context from Gemini-in-Pi (verbatim):
"Session continuity & replay (loadSession / listSessions): Tightening session resumption so switching or resuming via Grok's picker (F3 / /new) restores the full turn history, tokens, and working directory state seamlessly."

Sab chose this as first priority.

Goal: when a Pi session is resumed through Grok Build's picker (F3) or `grok --resume <id>` / `/new` with the pi-acp adapter, the full turn history, token counts and working directory are restored in the TUI. Implement `loadSession` / `listSessions` on the pi-acp side and make the Grok TUI consume them.

Acceptance (write failing tests first):
1. `listSessions` returns past Pi sessions with id, title, cwd, last-updated.
2. `loadSession(id)` replays prior user/assistant/tool turns as ACP updates in order, with no duplicates.
3. After resume, the TUI shows the cwd of the original session and the token counter is non-zero.

Verifier flow: build, start `grok pi` in a pty, send two turns, quit, resume via picker, confirm history and cwd visible on screen.

## Sibling worktree (pi-acp)
pi-acp source repo is ~/repos/Pi-Coding-Agent (pi-acp-patched/). Your branch worktree for it: ~/repos/Pi-Coding-Agent/.worktrees/gb-session-resume (branch gb-session-resume). Commit/push pi-acp changes there.
