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

# Task B: in-TUI controls and common slash commands for Pi/Claude/DSH in Grok Build

Read COMMON.md first.

Context from Gemini-in-Pi (verbatim):
"In-TUI controls (/model, thinking effort, slash commands): Exposing Pi's available models, thinking modes (low/medium/high), and project commands directly to Grok's config dropdowns and slash popups."

Sab's words (verbatim): "Grok Build enhancements for claude code and Pi and DSH i.e. switching between those agents, using slash commands like usage, registering common slash commands, etc"

Goal:
1. pi-acp advertises Pi's models, thinking modes (low/medium/high) and project slash commands via ACP session config / available commands so Grok's config dropdown and slash popup show them.
2. Register a common slash-command set across the claude, pi and dsh adapters: `/usage`, `/context`, `/compact`, `/model`, `/login`. Each is forwarded to the agent if it supports it, otherwise a visible "not supported by <agent>" notice appears in the TUI. Never silently swallow.

Acceptance (failing tests first):
1. Slash popup in `grok pi` lists Pi's project commands plus the common set.
2. `/model` dropdown in `grok pi` lists Pi's models and selecting one changes the next turn's model.
3. `/usage` in `grok claude` and `grok pi` renders a visible result or a visible unsupported notice.

Verifier flow: build, run `grok pi` and `grok claude` in a pty, open the slash popup, trigger `/model` and `/usage`, confirm on screen.

## Sibling worktree (pi-acp)
pi-acp source repo is ~/repos/Pi-Coding-Agent (pi-acp-patched/). Your branch worktree for it: ~/repos/Pi-Coding-Agent/.worktrees/gb-tui-controls (branch gb-tui-controls). Commit/push pi-acp changes there.
