# ACPX + Antigravity ACP (`gemini-4-neon` Medium) Slice Runbook

## Overview

This runbook documents the one-shot ACP dispatch path (`slices acp` -> `acpx --agent 'agy acp --sandbox'` -> `gemini-4-neon` at `thought_level=medium`) and the downstream Grok 4.7 integration sequence for the seven `acpx-piw-slices.py` slices.

- **Task definition file**: `/Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py`
- **Official runner**: `/Users/sab-mini/repos/pi-workflows/skills/parallel-slices/scripts/slices.py` (installed in `PATH` as `/Users/sab-mini/.local/bin/slices`)
- **Rendered packets**: `/Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices_out/_packets/<slice_id>.md` (and `_prompts/<slice_id>.md`)
- **Raw ACP events & returns**: `/Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices_out/_returns/<slice_id>.{events.jsonl,md,usage.json}`
- **Staged Rust outputs**:
  - `/Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices_out/grok-build/<target>`
  - `/Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices_out/pi-workflows/<target>`

## 1. Dry-render all seven packets locally (zero quota used)

```sh
slices acp /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py \
  --model gemini-4-neon --thought-level medium --dry-run
```

Each rendered packet contains:
1. Shared worker `SYSTEM` instructions (`"Use only this packet; do not use tools, do not read or write files, and do not run shell commands..."`).
2. `=== REPOSITORY CONTEXT (N excerpts) ===` (each excerpt `< 32 KB`).
3. `=== TASK SPECIFICATION ===` ending with `=== FILE: <target> ===` and one fenced `rust` code block.

## 2. Launch the first pilot slice (`step_evidence` alone at Neon medium) in a named Rex pane

To launch `step_evidence` in a new Rex terminal block (`--keep-open`, label `slice-step_evidence`):

```sh
slices acp /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py \
  --only step_evidence --model gemini-4-neon --thought-level medium --rex
```

Or to run directly inside an already-open Rex pane:

```sh
slices acp /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py \
  --only step_evidence --model gemini-4-neon --thought-level medium
```

Under the hood, `slices acp` creates an empty scratch directory outside the repo and runs:

```sh
acpx --agent 'agy acp --sandbox' --cwd "$scratch/work" \
  --model gemini-4-neon \
  --deny-all --no-fs --no-terminal --prompt-retries 0 \
  --timeout 360 --format json \
  exec --config-option thought_level=medium \
  --file /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices_out/_packets/step_evidence.md
```

### Guardrails enforced by `slices acp`
- **No silent fallback**: Verifies `effective.model == "gemini-4-neon"` and `effective.thought_level == "medium"` from the ACP `configOptions` metadata. Any mismatch fails with exit code `1`.
- **No automatic resend**: `--prompt-retries 0` and single-slice dispatch per invocation.
- **Tool-call detection and rejection**: Because `--deny-all --no-fs --no-terminal` does not prevent `agy` from invoking its own internal reader if prompted, `slices acp` watches the JSON-RPC event stream for any `tool_call`, `tool_call_update`, `fs/*`, or `terminal/*` event. If any tool call occurs, it prints `TOOL CALL ... [REJECTED]`, unstages any output file, and exits `1`.
- **Clean source separation**: Raw JSON-RPC events go to `_returns/<id>.events.jsonl`, raw unformatted text chunks go to `_returns/<id>.md`, and the extracted Rust file is written directly from the unformatted buffer to the staged path so terminal ANSI codes never touch the `.rs` file.

## 3. Remaining six slices (after `step_evidence` pilot review)

Dispatch each slice into its own named Rex pane (`--keep-open`):

```sh
for id in step_digest workflow_preflight run_path modal_view run_status_bar pane_zoom; do
  slices acp /Volumes/Mini-SSD/offload/repos/grok-build/task-scripts/acpx-piw-slices.py \
    --only "$id" --model gemini-4-neon --thought-level medium --rex
done
```

## 4. Downstream Grok 4.7 Integration Sequence

Once the seven staged Rust files are generated and verified:

1. **Stage 1 (`xai-grok-pager` + C3)**:
   - Grok 4.7 reproduces and applies C3 in `crates/codegen/xai-grok-pager/src/{acpx/bundle.rs,piw/mirror.rs}`:
     - non-zero `action.exitCode` / `output.ok == false` mapping to `Outcome::Failed`,
     - resolving paged `view.workflow.content`,
     - post-C3 `FlowNode` (`timeout_ms`, `mount_path`, `local_node_id`, `include_transition`), `FlowMount` (`mount_path`, `workflow_name`, `max_steps`), and `FlowDef` (`max_steps`, `mounts`),
     - preserving `rawInput`, `rawOutput`, and `truncated` on `tool_call` and `tool_call_update` in `sessions/main/events.ndjson`.
   - Grok 4.7 wires the five `xai-grok-pager` slices:
     - `crates/codegen/xai-grok-pager/src/acpx/step_evidence.rs`
     - `crates/codegen/xai-grok-pager/src/acpx/step_digest.rs`
     - `crates/codegen/xai-grok-pager/src/acpx/workflow_preflight.rs`
     - `crates/codegen/xai-grok-pager/src/acpx/run_status_bar.rs`
     - `crates/codegen/xai-grok-pager/src/views/acpx_modal/modal_view.rs`
   - Register `pub mod step_evidence; pub mod step_digest; pub mod workflow_preflight; pub mod run_status_bar;` in `src/acpx/mod.rs` and `pub(crate) mod modal_view;` in `src/views/acpx_modal.rs`, wire into `run.rs` and `detail.rs`, and run `cargo test -p xai-grok-pager`.
2. **Stage 2 (`xai-grok-pager-bin` + C1/C2)**:
   - After Stage 1 passes `cargo test -p xai-grok-pager`, Grok 4.7 reproduces and applies C1 (`gate.rs:804` `unwatch_session` fix + session re-watch in `piw_driver.rs` and `mirror.rs` + default `"pi"` agent label) and C2 (`agent.rs:633` persistent `Projector` in `PiwWatch`, no auto-pop-up modal on `DriverEvent::Waiting`, and `agent.rs:1463` plain-text gate answer matcher + mid-run steering).
   - Grok 4.7 wires `crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/run_path.rs` into `main.rs`, `project.rs`, and `agent.rs`, runs `cargo test -p xai-grok-pager-bin`, and verifies the four `beta8-merge` acceptance checks on Mini run `20261124T063602457Z-beta8-merge-8814563d`.
3. **Stage 3 (`piw` TUI, independent of Stages 1–2)**:
   - Grok 4.7 wires `tui/src/ui/pane_zoom.rs` into `/Users/sab-mini/repos/pi-workflows/tui/src/ui/{mod.rs,help.rs}` and runs `cargo test` in `/Users/sab-mini/repos/pi-workflows/tui`.
