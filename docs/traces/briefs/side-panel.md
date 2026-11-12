# dsh-tui-style side panel (2026-11-08)

Repo: /Volumes/Mini-SSD/offload/repos/grok-build, branch main. Work in place, commit on main.

## Sab's words (the intent)
- "the dsh-tui style sidebar that popped in and out, was adjustable"
- "There's still a traces modal that is full screen and full view. This is simply just a small side pop-up version. It's not the entire replacement version"
- "you can maximize the side panel… swapped or flipped the sizing" (panel big, chat small)
- "Grok build TUI is just more performant… true proper application" — a real ratatui layout split, not an overlay.
- Reference: dsh-tui's panel (tabs `◀ ≡ ▸ ◆ Info ∿ # ? ♥ ▸`; Info page with Session / Model / Context / Runtime sections incl. status, background jobs, subagents; footer `Esc chat · ←/→ panels · z zoom · +/- width`) and Sab's Claude Code mod pane (5h usage bar, context bar).

## Sab's choices (from his messages and the dsh-tui screens he showed)
- In the panel: Esc returns focus to chat, ←/→ switch pages, `z` swaps sizes (panel big, chat small), `+`/`-` width. Chat keeps working while the panel is open.
- Pages: a small **Traces** view (this session's live calls, reusing the live feed/live follow rows) and an **Info** page like dsh-tui's (session, model, effort, mode, context, status, background jobs, subagents), plus the 5h usage and context bars from his Claude Code mod pane when the harness reports them.

## Claude's only substitution (list it atop your report)
- Toggle key F3 instead of dsh-tui's Ctrl+B, because Ctrl+B is SendToBackground in grok-build.

## Code map (from the earlier scout; verify)
- `ActiveModal` is a single slot; the panel needs its own field, not a modal.
- Horizontal split via `AgentViewLayoutParams` (timeline rail is the precedent).
- `app/agent_view/render.rs` ~3194 early return must be bypassed for the panel.
- Add a `KeyOwner` slot for panel focus. The dashboard attached-session popup that keeps passing input is the precedent.
- Read the key handling and `TOOL-traces-modal.md` before binding keys; reuse existing keys.

## Rules
- Every panel state (closed, open-unfocused, focused, zoomed, each page, narrow terminal) gets a render snapshot test.
- Tests run visibly in herdr pane w3E:p4: `herdr pane run w3E:p4 "clear; cd /Volumes/Mini-SSD/offload/repos/grok-build && nice -n 10 cargo test --release -p xai-grok-pager -- <filters> && echo TESTS-DONE || echo RUN-FAILED"`, wait with `herdr pane wait-output w3E:p4 --regex 'TESTS-DONE|RUN-FAILED' --timeout 3600000`. One cycle at a time, --release. Known unrelated: `insert_block_before_rejects_an_already_committed_anchor` fails only in release; `tests/remote.rs` fake-ssh can time out under load.
- Verify on the real screen: `scripts/install-fork.sh` in w3E:p4, then run `target/release/xai-grok-pager` in your own herdr tab, open the panel on a session, read it with `herdr pane read`. Never type letters into the traces list (its keys resume sessions as paid turns). Never send a prompt to any agent. Close your tab when done.
- Do not install binaries or edit ~/.claude, ~/.pi, ~/.grok.
- Commit on main and push as you go; every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Report tightly: what's on screen per state (pasted `herdr pane read`), tests read from the pane, commit hashes, anything unverified.
