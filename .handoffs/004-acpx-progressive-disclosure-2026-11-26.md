# Handoff 004 — ACPX & Pi-Workflows TUI Progressive Disclosure + Live Beta8 Repair

Date: 2026-11-26
Repos:
- `/Volumes/Mini-SSD/offload/repos/grok-build` (`main` @ `7d5b381c`)
- `/Volumes/Mini-SSD/offload/repos/grok-build/.worktrees/beta8-rest` (`HEAD` `6a337a00` merging `d5b609c8` `v0.1.14-beta.8`)
- `/Users/sab-mini/repos/pi-workflows` (`main` @ `9250527`, `~/.cargo/bin/piw` rebuilt and installed)

---

## 1. Where We Are Right Now (Tail-End Summary)

All 7 Antigravity ACP (`gemini-4-neon`, `thought_level=medium`) slices and C1–C3 fixes are integrated, tested, committed on `main`, and installed on both `sab-mini` and `sab-air`:
- `7d5b381c` (`grok-build`): fixed `mirror.rs` paged `$artifact` workflow resolution so finished runs with large workflow definitions (`monitor` with 255 nodes and 22 mounts) wait for `Content::Ready` before marking `ever_full`, and added `live_beta8_merge_v1_acceptance_check`.
- Refreshed live mirror bundles in `/Users/sab-mini/.grok/cache/pi-workflows-mirror/` for both `piw-20261124T063602457Z-beta8-merge-8814563d` and `piw-20261124T064326952Z-monitor-cd63bbf0`.
- Confirmed in the live `/acpx` TUI on `beta8-merge`:
  - Branch-pruned DAG (`resolve_group1_air -> air_done -> stopped` pruned).
  - Full-pane graph zoom (`m` / `z`) and detail zoom (`M` / `Z`) with `Esc` restoring split view.
  - `7 ✗ cargo_check` now renders red `✗` with `115ms/1h30m · exit 101` instead of a false green `✓`.
  - Stage rows show verdicts, time budgets, file scope checks, and trace event ranges.

---

## 2. Immediate Next Step: 4 Small Progressive-Disclosure Fixes in `/acpx` and `grok acpx`

Hands-on inspection of the `/acpx` Ledger screen (`Flows 75`) and Run screen (`runs › beta8-merge`), together with GPT-Sol's design critique, identified 4 targeted fixes to eliminate info-overload without destabilizing the modal:

### Fix 1 — Collapse stage trace events by default in `Mode::Run` (`crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs`)
- **Problem seen in screenshot**: When opening `runs › beta8-merge`, stages `4`, `5`, `6`, and `7` start expanded (`▼`), filling the left pane with raw `+0ms node_started`, `+730ms action_completed`, and `+49m19s node_outcome` rows and pushing stages `1..3` off the top of the viewport.
- **Fix**:
  - In `RunViewState`, fold completed `StepStage` roots (`▸`) by default when building/initializing the fold tree so the left pane defaults to the clean, ordered executed timeline (`1 ✓ route_host` through `7 ✗ cargo_check`, including repeated loop attempts in execution order).
  - Pressing `Space` or `Right` on any stage still expands its raw trace events on demand.
  - Filter `c. Skipped / NotRun flow nodes` against `modal_view::pruned_flow(&self.run)` so untaken branch nodes are not counted as skipped.

### Fix 2 — Adaptive Step Digest & visible Run ID in `DetailPanel` (`crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs`)
- **Problem seen in screenshot**:
  - Neither the Ledger screen nor `runs › beta8-merge` displays the run ID (`piw-20261124T063602457Z-beta8-merge-8814563d`), while the right pane of `cargo_check` shows a 36-char internal attempt UUID (`ed61e682-b67d-4068-bf97-03a233c59335`) and raw JSON wrapper keys (`durationMs 105`, `exitCode 101`, `ok false`, `outputTail ...`).
  - For agent steps, `build_step_lines` dumps `Prompt`, `Command`, full `Conversation`, raw `Result`, `stdout`, `stderr`, and `Events` all at once into the right pane of `Mode::Run`, duplicating the `Enter` Step Inspector tabs.
  - On the Ledger preview (`build_run_lines`), `Stages` iterates unpruned `&run.flow.nodes` (showing dead `○ resolve_group1_air`, `○ air_done`, `○ stopped` rows) and dumps raw `Last result` JSON.
- **Fix**:
  - Show the run ID (and short copyable slug) in the `build_run_lines` header and `run.rs` header; replace the raw attempt UUID in `build_step_lines` with human step facts (`attempt N`, `elapsed / timeout`, `exit <code>`).
  - Make `build_step_lines` adaptive to the step kind and outcome:
    - **Failed shell/action step (`cargo_check`)**: lead immediately with the error / failure tail (e.g. `error: duplicate key xai-tty-utils in crates/codegen/xai-grok-update/Cargo.toml:38:1`), then the command and cwd; strip redundant JSON wrapper keys (`durationMs`, `exitCode`, `ok`, `outputTail`).
    - **Waiting gate**: lead with the gate question, numbered choices, and preceding failure tail.
    - **Agent / compute step**: lead with the step digest (verdict or validator rejections/corrections, file scope summary, and concise output notes), plus a footer hint (`Enter: open Output / Prompt / Stderr / Events tabs · r: replay step in pager`) instead of dumping the entire raw conversation and event log into the overview pane.
  - In `build_run_lines` (the Ledger preview), show only the executed timeline in order + active/waiting status + concise failure/outcome summary (no unrun `○` nodes and no raw `Last result` JSON dump).

### Fix 3 — Fold repeat finished runs & improve readability in Ledger (`crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs`)
- **Problem seen in screenshot**:
  - `FINISHED 65` lists 63 repetitive historical test runs (`mixed`, `mixed-talk`, `ship: slugify`) flat, and each row wastes horizontal space on an unpruned 24-dot Morse-code rail (`✓○✓○✓✓✓○✗○○○`).
  - Pressing `y` copies the full filesystem directory path instead of a runnable run ID.
- **Fix**:
  - Keep `NEEDS YOU` and `RUNNING` runs individually visible, and fold repeat finished runs of the same workflow under their latest run (`▸ mixed-talk (22 runs)`, expandable with `Space`/`Right`).
  - Replace the unpruned dot rail with truthful step progress (`7 steps ran` or active-branch progress) so workflow name and status have room to read.
  - Make `y` copy the run ID (or copyable `/open <id>` target).

### Fix 4 — Make `/runs` and `/open` in `grok acpx` find `pi-workflows` runs by name or ID prefix (`crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs`)
- **Problem**: `/runs` and `/open` only look in `~/.acpx/flows/runs` (`self.runs_dir()`) and ignore `~/.grok/cache/pi-workflows-mirror/`. Users cannot type `/open beta8-merge` or see `pi-workflows` runs in `/runs`.
- **Fix**:
  - Include `~/.grok/cache/pi-workflows-mirror` in `/runs` and `resolve_run_dir`.
  - Allow `/open <query>` to match by exact `run_id`, `piw-<run_id>`, unique ID prefix/suffix (e.g. `8814563d`), or workflow name (e.g. `/open beta8-merge` opens the most recent `beta8-merge` run).

---

## 3. State of `beta8-rest` Merge Worktree (For Follow-Up Live Run)

- Worktree: `/Volumes/Mini-SSD/offload/repos/grok-build/.worktrees/beta8-rest`
- Merge state: all 151 conflicted files resolved (`git diff --name-only --diff-filter=U` is `0`; `git diff --check` is clean).
- Compile state:
  - `cargo check --workspace --bins` passes with **0 errors** (`2m 22s`).
  - `cargo check --workspace --bins --tests --keep-going` has **150 compile error lines** across `--tests` targets in `xai-grok-login`, `xai-grok-shell`, `xai-grok-workspace`, `xai-grok-sampler`, `xai-grok-pager-minimal`, and `xai-grok-pager`.
- Live repair workflow ready to test after the 4 UI fixes:
  - `/Volumes/Mini-SSD/offload/repos/grok-build/.pi/workflows/beta8-repair.workflow.ts`
