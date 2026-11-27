# Handoff 004 — Operating Pi workflows through ACPX: fixes, progressive disclosure, beta8 repair

2026-11-26. Merges two handoffs written at the same time: one on `main` (`01b73d87`), one on `acpx-tui-slices` in `~/repos/grok-build-acpx-src` (`412ec679`). This is a handoff, not an approved UI spec or a request to implement everything below. Read the goal and the next fixes first.

Refs (recheck before implementing):
- `grok-build` `main`: `7d5b381c` / `bbca9e73` when written. The main worktree has unrelated uncommitted changes and untracked slice output; preserve them.
- `beta8-rest` worktree: `.worktrees/beta8-rest`, `HEAD` `6a337a00` merging `d5b609c8` (`v0.1.14-beta.8`).
- `pi-workflows`: `~/repos/pi-workflows` `main` @ `3ba95d8` (one note cited `9250527`, its parent); `~/.cargo/bin/piw` rebuilt and installed. Standalone `piw` builds separately from that repo.
- `~/repos/grok-build-acpx-src` is a separate, older worktree.

## Goal

Run robust `pi-workflows` (PIW) with any compatible ACP agent harness, including harnesses other than Pi. ACPX is the bridge to those agents. PIW owns workflow execution and state. The Grok UI lets the human start, follow, inspect, and intervene in the work directly, without asking another agent to interpret the run or act on the operator's behalf.

Design direction: **a substantially minimal first screen, with interactive, wizard-style disclosure into a lot of information**. Keep the full evidence available, reveal it at the point of use, and give consequential actions an explicit path of their own. Look at what the system could support; missing wiring in draft one is normal. Distinguish that from real limits such as unrecorded evidence or irreversible external effects.

## Where things stand

- All 7 Antigravity ACP slices (`gemini-4-neon`, `thought_level=medium`) and fixes C1–C3 are integrated and committed on `main` (integration `f52a8905`, whose subject says 6 slices; staged Rust in `task-scripts/acpx-piw-slices_out/`); one note reports them tested and installed on sab-mini and sab-air. The other note: those reported tests did not prove live gate usability, the three-surface operator walkthrough (item 3 below) was not verified, and no release build or upstream sync happened in that session.
- `7d5b381c`: `mirror.rs` paged `$artifact` workflow resolution now waits for `Content::Ready` before marking `ever_full`, so finished runs with large definitions (`monitor`: 255 nodes, 22 mounts) resolve; adds `live_beta8_merge_v1_acceptance_check`.
- Live mirror bundles refreshed in `~/.grok/cache/pi-workflows-mirror/` for `piw-20261124T063602457Z-beta8-merge-8814563d` and `piw-20261124T064326952Z-monitor-cd63bbf0`.
- Confirmed in the live `/acpx` TUI on `beta8-merge`: branch-pruned DAG (`resolve_group1_air -> air_done -> stopped` pruned); graph zoom (`m`/`z`) and detail zoom (`M`/`Z`), `Esc` restores split; `7 ✗ cargo_check` renders red `✗` with `115ms/1h30m · exit 101` instead of a false green `✓`; stage rows show verdicts, time budgets, file-scope checks, trace event ranges.
- **Seven-slice proof:** `slices acp → acpx exec → agy acp --sandbox` ran seven Gemini 4 Neon medium workers concurrently; the runner accepted all seven returns, no tool calls observed. It used the standalone slice runner, so it proved ACP transport and concurrency, not the full PIW operator loop. Use it as the model for a PIW exercise with parallel workers and a human decision. The single-slice restriction was removed in `pi-workflows` `a792390`.

## Start next

### Part 1 — Fixes and release checks

Keep these separate from the disclosure redesign. Release order: verify the fixes before upstream sync, retest the merged build, then install on mini and sab-air with `scripts/install-fork.sh`.

1. **Conversation keyboard focus is too faint.** Sab found it hard to see on sab-air in **Kanagawa Lotus and Monosami Ink Light**; strengthen both outlines (Lotus was visible in the mini capture; that does not override his report). Uncommitted, unbuilt, unverified palette edits are already in the main checkout: Lotus `#432a62`, Ink Light `#363229`, in `scripts/themes/zed_port.py` and generated `crates/codegen/xai-grok-pager-render/src/theme/zed_ports.rs`. The outline comes from `Theme::selection_border_visible()` in `scrollback/scrollback_pane.rs`; `bg_visual` is a different cue. Review focus before and after moving it, on the changed build, in every shipped theme per `AGENTS.md`, with both reported themes checked on the Air. HY4 Flash's earlier audit also flagged Monosami Light, Kanso Pearl, and Dayfox; keep those visible as follow-ups (the pending edit only touches Sab's two). Prior captures of the installed build, which do not verify the pending change: `/tmp/grok-nav-focus-review/final/png/`, `/tmp/acpx-focus-review/`.
2. **Queued-message double-send.** Capture a uniquely labelled queued prompt during a busy turn; compare outbound ACP `session/prompt` requests with agent receipt and rendered rows. Two requests = duplicate delivery; one request and two rows = duplicate display. Queue drain and one-shot echo suppression live in `app/dispatch/queue.rs`, `app/agent.rs`, `acp/tracker.rs` (Luna source review; not reproduced). Add a regression for the case that actually fails.
3. **Pending-gate interaction.** After a run has parked, open the ACPX modal, standalone PIW, and Grok ACPX chat. Verify the actual question and choices are discoverable; inspecting the gate or typing bare `/answer` never changes or cancels the run; an explicit answer from one surface yields the engine's real next state in the others. Distinguish `Answer sent` / `Answer accepted` from run completion — `detail.rs` currently maps waiting + local answer to completed. Confirmation copy must distinguish answering the existing PIW run from launching a native ACPX continuation flow (`run.rs` uses native-flow copy for mirrored answers). Ask for text or checkpoint input only when the gate requires it. The earlier `/answer` cancellation report still has no traced cause.

### Part 2 — Four progressive-disclosure fixes in `/acpx` and `grok acpx`

From hands-on inspection of the Ledger (`Flows 75`) and Run (`runs › beta8-merge`) screens plus GPT-Sol's critique; targeted to cut overload without destabilising the modal. Fix 3 refines the existing full-width ledger, while the design section below says to try a small entry screen before committing to one — decide whether Fix 3 is interim or superseded.

1. **Collapse stage trace events by default** (`views/acpx_modal/run.rs`). Stages `4`–`7` open expanded (`▼`) with raw `+0ms node_started` / `+730ms action_completed` / `+49m19s node_outcome` rows, pushing `1..3` off screen. In `RunViewState`, fold completed `StepStage` roots (`▸`) by default so the left pane shows the ordered executed timeline (`1 ✓ route_host` … `7 ✗ cargo_check`, repeated loop attempts in order); `Space`/`Right` still expands. Filter `c. Skipped / NotRun flow nodes` against `modal_view::pruned_flow(&self.run)` so untaken branches are not counted as skipped.
2. **Adaptive step digest and visible run ID** (`views/acpx_modal/detail.rs`). Today neither screen shows the run ID; the `cargo_check` pane shows a 36-char attempt UUID and raw JSON keys (`durationMs`, `exitCode`, `ok`, `outputTail`); agent steps dump Prompt, Command, Conversation, Result, stdout, stderr, Events at once, duplicating the `Enter` inspector tabs; the Ledger preview (`build_run_lines`) iterates unpruned `run.flow.nodes` (dead `○ resolve_group1_air`, `○ air_done`, `○ stopped`) and dumps `Last result` JSON. Fix:
   - Show the run ID (and a short copyable slug) in the `build_run_lines` and `run.rs` headers; replace the attempt UUID with `attempt N`, `elapsed / timeout`, `exit <code>`.
   - `build_step_lines` adapts to kind and outcome. **Failed shell step:** lead with the failure tail (e.g. `error: duplicate key xai-tty-utils in crates/codegen/xai-grok-update/Cargo.toml:38:1`), then command and cwd; drop the wrapper keys. **Waiting gate:** question, numbered choices, preceding failure tail. **Agent/compute step:** digest (verdict or validator rejections/corrections, file-scope summary, concise output notes) plus footer `Enter: open Output / Prompt / Stderr / Events tabs · r: replay step in pager`.
   - Ledger preview: executed timeline in order, active/waiting status, concise failure/outcome — no unrun `○` nodes, no raw JSON.
3. **Fold repeat finished runs in the Ledger** (`views/acpx_modal/ledger.rs`). `FINISHED 65` lists 63 repetitive test runs (`mixed`, `mixed-talk`, `ship: slugify`) flat, each with an unpruned 24-dot rail (`✓○✓○✓✓✓○✗○○○`); `y` copies a filesystem path. Keep `NEEDS YOU` and `RUNNING` rows individual; fold repeats under the latest (`▸ mixed-talk (22 runs)`, `Space`/`Right` expands); replace the rail with truthful progress (`7 steps ran` or active-branch progress); `y` copies the run ID (or `/open <id>`).
4. **`/runs` and `/open` in `grok acpx` find PIW runs** (`crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs`). Both only read `~/.acpx/flows/runs` (`self.runs_dir()`). Include `~/.grok/cache/pi-workflows-mirror` in `/runs` and `resolve_run_dir`; let `/open <query>` match exact `run_id`, `piw-<run_id>`, a unique ID prefix/suffix (`8814563d`), or workflow name (`/open beta8-merge` → most recent run).

## Design: start from each view's purpose

Sab's screenshots show a prettier table alone is not enough. Flows showed 75 runs, truncated the `Needs you` reasons, and gave the other pane to all 13 `beta8-merge` stages plus raw Cargo output. Sessions was easier to scan because every row did the same job — not offered as the design to copy. In code, the wide Flows ledger reuses the run-detail panel before the operator has entered the run.

| Surface | First job | Reveal on request |
| --- | --- | --- |
| ACPX landing / Flows | Find what needs attention, open a run, or start work. Try a small entry screen before a full-width table. | Full lists, repeated runs, filters, sorting, metadata. Age alone does not settle a pending decision. |
| Sessions | Find a conversation and choose what to do with it. | Usage, paths, saved transcript. Distinguish replay from attaching to / resuming a live agent if those are added. |
| Live run | Where the run is and what the human can do now; an actual pending question up front. | Executed attempt timeline, graph, supported controls. Keep repeated attempts in order (`cargo_check → repair_compile → cargo_check`); the DAG answers a different question. |
| Step inspector | Explain one attempt, most relevant evidence first. | Full output, logs, prompt, response, tool activity, metadata. A retry action must make its scope and effects explicit. |
| Replay | What happened at the selected attempt, with a route back to the current run. | Full conversation and tool evidence as recorded. The modal opens separate read-only playback; ACPX chat `/open <run>` projects a run into the chat and can restore gate state. Keep historical evidence and current authority distinct. |

Prototype the wizard on a real pending gate: **open run → read question → choose → supply required input if any → confirm effect → see the engine's response**. The operator can consult failure evidence and return to the same choice. Evaluate on the failed `beta8-merge` run and a run genuinely waiting on a human before settling columns or panes. Asking about a run, steering an agent, and controlling a workflow are distinct actions; inspecting evidence never takes a branch.

PIW already defines start, pause, resume, cancel, restart, supported settings changes, queued follow-ups, and gate answers (`~/repos/pi-workflows/src/client/protocol.ts`); these can back explicit UI actions where the run supports them. A fresh run and retry-from-here differ: exact continuation needs saved state and a policy for effects that already happened. Replay cannot recover evidence never captured or undo a push or other external effect.

## `beta8-rest` merge worktree

- All 151 conflicted files resolved (`git diff --name-only --diff-filter=U` empty; `git diff --check` clean).
- `cargo check --workspace --bins`: **0 errors** (2m 22s).
- `cargo check --workspace --bins --tests --keep-going`: **150 compile-error lines** in test targets of `xai-grok-login`, `xai-grok-shell`, `xai-grok-workspace`, `xai-grok-sampler`, `xai-grok-pager-minimal`, `xai-grok-pager`.
- Live repair workflow, ready to test after the Part 2 fixes: `.pi/workflows/beta8-repair.workflow.ts`.

## Code anchors

- Modal routing, ledger preview, gate submission, replay request: `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs`, `views/acpx_modal/{ledger,run,detail,inspector}.rs`. PIW gate model and projection: `src/piw/{gate,mirror}.rs`.
- Launch/cancel and separate replay: `crates/codegen/xai-grok-pager/src/app/dispatch/{acpx_run,standalone_traces}.rs`. ACPX chat command routing: `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/{agent,run_path,piw_driver}.rs`.
- Draft gaps, not architectural blockers: chat `run.set` / `run.steer` wiring; the inspector's 256 KiB artifact display cap.

## Future agent chains

The implementation agent hands off an exact build and a runnable fixture. A fresh vision reviewer (e.g. HY4 Flash) operates the UI, captures before/after images, and reports what it could verify. Failures route back to the original agent, and completion requires fresh evidence against the final build. The theme audit produced captures, but no automatic repair-handback chain was built.
