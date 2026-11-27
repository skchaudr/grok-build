---
title: "Handoff 004 — Operating Pi workflows through ACPX: fixes, progressive disclosure, beta8 repair"
date: 2026-11-26
tags: []
---

# Handoff 004 — Operating Pi workflows through ACPX: fixes, progressive disclosure, beta8 repair

## Goal

Run robust `pi-workflows` (PIW) with any compatible ACP agent harness, not only Pi. ACPX is the bridge to those agents; PIW owns workflow execution and state; the Grok UI lets the human start, follow, inspect and intervene directly, without asking another agent to interpret the run or act on their behalf.

Sab's design direction: **progressive disclosure of information, not complete dumps** Keep the full evidence available, reveal it at the point of use, and give consequential actions an explicit path of their own. 

## Where things stand

The seven Antigravity slices (Gemini 4 Neon, medium thinking) and fixes C1–C3 landed on `main`. The three-surface gate walkthrough below was never done, and that no release build or upstream sync happened in that session.

The seven-slice run itself proved ACP transport and concurrency — seven workers at once through `acpx` into `agy`, all returns accepted, no tool calls — but it used the standalone slice runner, not the PIW operator loop. It is the model for the next exercise: a PIW run with parallel workers and a human decision in the middle.

A mirror fix then let large finished runs (the 255-node `monitor` workflow) load fully, and the live `/acpx` TUI was checked on the failed `beta8-merge` run: untaken branches are pruned from the DAG, graph and detail panes zoom full-screen and `Esc` restores the split, and the failed `cargo_check` now shows a red `✗` with its exit code instead of a false green `✓`.

## Start next

### Part 1 — Fixes and release checks

Keep these separate from the disclosure redesign. Release order: verify the fixes, then upstream sync, retest the merged build, and only then install on mini and sab-air with `scripts/install-fork.sh`.

**The pending-gate interaction is unproven.** After a run has parked, open it from the ACPX modal, standalone PIW and Grok ACPX chat. The question and choices must be discoverable; inspecting the gate or typing bare `/answer` must never change or cancel the run; an explicit answer on one surface must show the engine's real next state on the others. Today the detail pane treats "waiting + answered locally" as completed, and mirrored answers get confirmation copy meant for launching a native ACPX continuation — the UI must tell `Answer sent` / `Answer accepted` apart from run completion, and answering an existing PIW run apart from starting a new flow. Ask for text or checkpoint input only when the gate needs it. An earlier report of `/answer` cancelling a run still has no traced cause.

### Part 2 — Four progressive-disclosure fixes in `/acpx` and `grok acpx`

These came from working through the Flows ledger and the `beta8-merge` run screen by hand, plus GPT-Sol's critique, aimed at cutting overload without destabilising the modal. Fix 3 polishes the existing full-width ledger, while the design section below says to try a small entry screen before committing to a table — decide whether Fix 3 is interim or superseded.

1. **Collapse stage trace events by default.** Opening the run, stages 4–7 come up expanded with raw `node_started` / `action_completed` / `node_outcome` rows and push stages 1–3 off the top. Fold finished stages so the left pane reads as the ordered executed timeline, repeated loop attempts included; `Space`/`Right` still expands. Count skipped nodes against the pruned flow so untaken branches aren't called skipped.
2. **Lead the detail pane with what matters, and show the run ID.** Neither screen shows the run ID, while the `cargo_check` pane shows a 36-character attempt UUID and raw JSON wrapper keys. Agent steps dump prompt, command, conversation, result, stdout, stderr and events all at once, duplicating the `Enter` inspector. The ledger preview lists dead unrun nodes and raw last-result JSON. Instead: show the run ID with a short copyable slug; describe the attempt as `attempt N · elapsed/timeout · exit code`; for a failed shell step lead with the failure tail (e.g. `duplicate key xai-tty-utils in …/xai-grok-update/Cargo.toml:38`), then command and cwd; for a waiting gate lead with the question, numbered choices and the preceding failure; for an agent step give a digest (verdict or validator corrections, file-scope summary, short notes) and a footer pointing to the inspector tabs and replay. The ledger preview shows only the executed timeline, live status and a short outcome.
3. **Fold repeat finished runs.** `FINISHED 65` is mostly 63 near-identical test runs listed flat, each spending its width on a dot rail that includes unrun nodes, and `y` copies a filesystem path. Keep `NEEDS YOU` and `RUNNING` rows individual, fold repeats under the latest run (`▸ mixed-talk (22 runs)`), replace the rail with honest progress (`7 steps ran`), and make `y` copy the run ID or an `/open <id>` target.
4. **Let `grok acpx` find PIW runs.** `/runs` and `/open` only look in ACPX's own runs folder and miss the PIW mirror, so `/open beta8-merge` fails. Include the mirror and let `/open` match by exact ID, unique ID prefix or suffix, or workflow name (most recent run wins).

## Design: start from each view's purpose

Sab's screenshots show a prettier table alone is not enough. Flows showed 75 runs, truncated the reasons under `Needs you`, and gave the other pane to all 13 `beta8-merge` stages plus raw Cargo output — the wide ledger reuses the run-detail panel before the operator has even entered a run. Sessions was easier to scan because every row did the same job, but it was not offered as the design to copy.

| Surface | First job | Reveal on request |
| --- | --- | --- |
| ACPX landing / Flows | Find what needs attention, open a run, or start work. Try a small entry screen before a full-width table. | Full lists, repeated runs, filters, sorting, metadata. Age alone does not settle a pending decision. |
| Sessions | Find a conversation and choose what to do with it. | Usage, paths, saved transcript. Distinguish replay from attaching to or resuming a live agent, if those are added. |
| Live run | Where the run is and what the human can do now; a real pending question up front. | Executed attempt timeline, graph, supported controls. Keep repeated attempts in order (`cargo_check → repair_compile → cargo_check`); the DAG answers a different question. |
| Step inspector | Explain one attempt, most relevant evidence first. | Full output, logs, prompt, response, tool activity, metadata. A retry action must make its scope and effects explicit. |
| Replay | What happened at the chosen attempt, with a way back to the current run. | Full recorded conversation and tool evidence. The modal opens read-only playback; chat `/open <run>` projects the run into the chat and can restore gate state. Keep historical evidence and current authority distinct. |

Prototype the wizard on a real pending gate: **open run → read question → choose → supply input if required → confirm effect → see the engine's response**, with the failure evidence one step away and a way back to the same choice. Judge it against the failed `beta8-merge` run and a run genuinely waiting on a human before settling columns or panes. Asking about a run, steering an agent and controlling a workflow are three different actions, and inspecting evidence must never take a branch.

PIW already has operations for start, pause, resume, cancel, restart, settings changes, queued follow-ups and gate answers, so explicit UI actions have something to stand on. A fresh run and retry-from-here are different things: exact continuation needs saved state and a policy for effects that already happened. Replay cannot recover evidence never captured, or undo a push. Chat `run.set` / `run.steer` wiring and the inspector's artifact size cap are draft gaps, not architectural blockers.

## `beta8-rest` merge

Every conflict is resolved and the binaries check clean; the test targets do not yet compile (150 error lines across six crates). The `beta8-repair` workflow is written and ready to run live once the Part 2 fixes make it watchable.

## Future agent chains

The implementing agent hands off an exact build and a runnable fixture; a fresh vision reviewer such as HY4 Flash operates the UI, captures before and after, and reports what it could verify; failures go back to the original agent, and nothing is complete without fresh evidence against the final build. The theme audit got as far as captures — the repair handback chain was never built.
