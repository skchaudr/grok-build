# Workflow operator map: Sab's Mac mini

Grok Build is where Sab is building the keyboard-first TUI to **start, follow, inspect, and intervene in Pi Workflows directly**. Pi Workflows (PIW) owns workflow execution and durable run state. ACPX connects workers to compatible agent harnesses. Standalone PIW and Grok Build should show the same run and accepted decisions.

## Project map

| Project on the Mini | Start here |
|---|---|
| **Grok Build** (`~/repos/grok-build`, the SSD checkout) | Owns the `/acpx` modal, Grok ACP chat, PIW client and mirror, and Grok-specific slice packets under `task-scripts/`. Start here for operator controls, keyboard routes, or evidence presentation. |
| **Pi Workflows** (`~/repos/pi-workflows`) | Owns graph execution, worker routing, gate semantics, server protocol, and recovery. Start with `src/workflows/`, `src/server/`, `src/state/`, and `docs/SQLITE_STATE.md`. Shared slice tooling lives under `skills/parallel-slices/`. `docs/GROK_BUILD.md` is the hands-on integration guide. |
| **Grok Build ACPX source checkout** (`~/repos/grok-build-acpx-src`) | Separate checkout referenced by Handoff 004 for the beta8 merge workflow. Confirm the intended checkout and worktree before editing. |
| **Grok Pi** (`~/repos/grok-pi`) | Secondary Pi-to-Pager bridge. Its F2 feature labeled Pi workflows runs Rhai; PIW in this map means the separate `~/repos/pi-workflows` engine. |
| **Pi Durable Lab** (`~/repos/pi-durable-lab`) | Durable session and worker experiments; see `.pi/workflows/` and `bin/worker.sh`. |
| **Sab Orchestrate** (`~/repos/sab-orchestrate`) | Fleet and worktree orchestration; start with `bin/cell` and `FLEET.md`. PIW owns the graph inside a single run. |

ACPX owns ACP dispatch and session lifecycle. Resolve the installed binary and its source checkout before changing transport. A dispatch swap should preserve the slice packet, expected return, and acceptance checks.

## Grok Build entry points

- `/acpx` ledger, graph, detail, and evidence: `crates/codegen/xai-grok-pager/src/views/acpx_modal/`.
- PIW client, gate handling, and mirror: `crates/codegen/xai-grok-pager/src/piw/`. The mirror is a projection; use the PIW server protocol for live state and accepted decisions.
- Grok ACP chat driver: `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/`.
- Grok-specific workflows and design artifacts: `.pi/workflows/`; slice packets and staged returns: `task-scripts/`. Read `.pi/agents/delivery-policy.md` for delegation and review. Record dated progress in `.handoffs/`.

## Operator and workflow design

- **First screen:** Show run identity, current state, blockers, and the next available action. Let the operator drill into attempts, failure output, and recorded evidence and return without losing their place.
- **Inspect and act:** Opening a gate or typing bare `/answer` should reveal the question and choices. Give each consequential action an explicit path. Show the accepted decision and resulting branch from every surface watching the run.
- **Stop and resume:** Show what is already accepted, whether this run can resume, and what would rerun. Preserve accepted work and surface uncertain side effects before recovery.
- **Design before dispatch:** Agree on scope, authority, acceptance checks, and stop points. Run independent slices in parallel once their contracts are clear, stage the returns, and check integration independently against the agreed behavior.

## Live acceptance check

Handoff 004 (Nov 26) records seven concurrent Antigravity slices through the standalone ACPX runner and an open PIW operator walkthrough. For that walkthrough, park a PIW run with parallel workers at a human gate. Inspect the gate from `/acpx`, standalone `piw`, and Grok ACP chat. Submit one explicit answer, then verify that all three show the same accepted branch and keep watching through the next step without reattaching. Record the outcome in the dated handoff.
