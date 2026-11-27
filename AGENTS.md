# AGENTS.md

In Sab's parallel-slice generation workflow, the Fable 5.2 run cost about 15 times as much as any of his other runs. Check the context size of each slice packet before dispatch, and ask Sab before proceeding if it's substantially larger than usual.

For all other agents, this grok build fork is playing a very important role in agentic observability and legibility. The goal is always makechanges and then build locally and rsync it over to sab-air (ssh sab-mini@sab-air) 

Build and install with `scripts/install-fork.sh`: one cargo run builds grok-fork and grok-traces, installs both here and on sab-air, and fails on a checksum mismatch.

Any change to a theme or to how a TUI screen is coloured: look at it on screen in every shipped theme (`GROK_THEME=<name>`), not only at the contrast test (`visibility_tests`). Sab, 2026-11-11: every theme must be "distinct and clear and visually confirmed and verified by any agent working on it".

## Execution and delivery judgment

For kickoff, delegation, review and release decisions, read `.pi/agents/delivery-policy.md` and include its policy in delegated assignments; Rex does not auto-load role files. Co-design the actual task workflow with the operator before creating or launching it; the experimental kickoff/landing definitions and prompt are archived in `/Users/sab-mini/repos/pi-workflows/archive/grok-build/2026-11-17/`, not installed here. Do not mistake activity for progress or a reviewer finding for repair authority: disposition each claim, repair only accepted material delivery blockers, and preserve nonblocking notes/unknowns. Control execution with stage permissions and routing, not arbitrary agent tool/turn quotas. Existing build/install guidance applies only within the user's actual delivery authority; drafting a workflow does not authorize product execution.


For workflow operability work, read [the workflow operator map](docs/workflow-operator-map.md) before changing the Grok Build TUI, ACPX dispatch, or Pi Workflows integration.
