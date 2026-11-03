# PROPOSAL: Stage 3, agent to agent

Status: proposal, waiting for Sab's approval. Nothing here is built. Source of truth for the requirement: SPEC-MVP.md, "Agent to agent".

## What scouting found (2026-11-02, two Sonnet scouts)

**Pi's subagent view** (`pi-subagents` 0.82.1, `~/.pi/agent/npm/node_modules/pi-subagents/src/tui/`), the one Sab rated good:
- A strip under the editor that opens on ↑/↓ only when the editor is empty, and the `/subagents-fleet` overlay: roster on the left, the selected agent's transcript on the right.
- Roster row: glyph, name, task, state on the right. Glyphs: `●` running, `◦` queued, `✓` done, `■` stopped, `✗` failed. `main` sits on top as the way back.
- Detail header: model, tokens, tool count, duration, current tool.
- Keys: `j`/`k` select, `J`/`K` and PgUp/PgDn scroll the transcript, `Esc`/`q` close. Enter hands off to a tmux/Herdr pane, which a pager can't do in place.
- Alive or dead comes from a status file plus a pid probe. Transcript reads are tail-only (200 lines, 64 KB).

**grok-build pager** (`crates/codegen/xai-grok-pager/src/`):
- The pager has subagent views (chat takeover, Tasks pane `Ctrl+G`, dock, dashboard `Ctrl+\`), but they work on live chat agents, not on traces rows. Only their key conventions carry over.
- The traces modal has no subagent view. `Mode::Live` rows are the closest look.
- Follow (`views/traces_follow.rs`) already keeps every agent (`agents`: id, model, label), each agent's start and end rows, and a one-agent filter (`filter = n`, what Tab cycles through).
- Missing: per-agent ended flag, current call, age, a parent link, a back stack, and a breadcrumb header.

## The proposal

All in `views/traces_follow.rs`, plus hints and Esc order in `views/traces_modal.rs`. No new crate, no new data source: everything comes from the rows follow already has.

**Keys** (today's follow keys stay; this adds to them):

| Where | Key | Does |
|---|---|---|
| Follow, live tail, no row highlighted, agent has subagents | `←` / `h` | open the dashboard |
| Same, agent has no subagents | `←` / `h` | nothing |
| A highlighted call row | `←` / `h`, `→` / `l` | close / open its body inline (shipped today) |
| A highlighted call that spawned a subagent | Enter | open that subagent's follow |
| Any other highlighted call | Enter | full payload view (today) |
| Dashboard | `j` / `k` | select a subagent |
| Dashboard | `→` / `l` / Enter | open that subagent's follow |
| Dashboard | `←` / `h` / Esc | back to the follow it was opened from |
| A subagent's follow | Esc | back one level (to the dashboard or parent it came from) |
| Anywhere | `q` | leave follow |

Esc closes the innermost thing first: dashboard, then a subagent's follow, then follow itself.

**Dashboard:** one row per subagent of the agent being followed, newest first: state glyph (Pi's set), name, model, current call (`Bash  cargo test`) or last state, age. It's a full-width list with no side pane, so it works at half width. When there are more rows than fit, it scrolls.

**A subagent's follow:** the same follow screen showing only that agent (today's `filter = n`), plus a back stack so Esc returns to where you came from. If that subagent has its own subagents, `←` at its live tail opens its dashboard. That's the same rule at every level.

**Header:** a breadcrumb, `root › child › grandchild`, each with its model.

**Data to add to `Agent`:** ended (from its end row), current call (its last call with no result yet), last-activity time (for age), and parent (from row depth). The link from a spawning call to its subagent comes from receipts (Claude's `SubagentStart` agent id, grok's and Pi's child start). The builder confirms which harnesses give that link. Where none exists, Enter on that call keeps opening the payload, and the report says so.

**Tests:** unit tests on the key table above (each row), and one pty script. It opens the dashboard on a parent with two children, enters one, goes back, and checks that `←` on a childless agent changes nothing. It checks the header text at each step. The hint line lists exactly the keys that have a test.

**Also in this change:** SPEC-MVP's "Rows carry the real thing" line still says the highlighted call opens inline. It gets updated to Sab's 2026-11-02 rule: `l` opens, `h` closes, highlighting never opens.

## How it gets built

One Sonnet builder on a branch, then one reviewer. If the reviewer's findings hold, the reviewer becomes the fixer. Tests run once at the end (traces unit tests and the follow pty tests). Then it merges to main, builds on the Mini, and installs on the Mini and sab-air. Sab verifies live.

## Not in this change (the rest of Stage 3, later slices)

One loud state line, noise rows hidden by default, the 80×20 layout pass, wake-driven redraw, footer = tested keys only, pid and session id on every agent row.
