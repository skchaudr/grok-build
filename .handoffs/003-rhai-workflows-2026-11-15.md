# Handoff: Rhai workflows in `grok grok-pi` vs the Pi workflow packages

Written 2026-11-15 by Claude (Fable 5.2) for Sab, to start a fresh session on Rhai-style workflows. Facts were read from code and from sab-air today; nothing here was run as a workflow.

Sab, 2026-11-15: "rhai workflows I thought it would work in theory but did not".

## What happened today

- 07:26 on sab-air, `grok grok-pi` stopped starting in every folder: `unable to receive 'initialize' response, channel closed`, then `server shut down unexpectedly`.
- Cause: `pi_workflows = true` under `[ui]` in `~/.grok/config.toml` (file last written 07:18). With it on, `grok-pi-acp` loads its bundled workflows extension, which registers a Pi tool named `workflow`. The installed Pi package `npm:@osolmaz/pi-workflows` registers a tool with the same name. Pi exits 1 on the duplicate; the pager shows only the channel error.
- Applied on sab-air: `pi_workflows = false` (backup `~/.grok/config.toml.bak-20261115-0730`). `grok grok-pi` starts again. So **Rhai workflows are currently off in `grok grok-pi`**, and `@osolmaz/pi-workflows` is the one loaded.

## The two systems

| | Rhai workflows (the fork's own, from xAI + Dwsy) | `@osolmaz/pi-workflows` 0.17.7 (Pi package) |
|---|---|---|
| Language | Rhai script, one `.rhai` file with a `meta` map (name, description, when_to_use, phases) | TypeScript graph of steps |
| Engine | Rust crate `crates/codegen/xai-workflow` (engine, host, journal, validate), run by the Grok session host | Runs inside Pi as an extension |
| Where agent steps run | Each `agent(prompt, opts)` call spawns a separate agent through the host (`__pi_workflow_spawn` in `extensions/pi-grok-workflows/index.ts`); output can be forced to a JSON schema | Inside the current Pi conversation, so the model keeps the discussion's context; structured output comes back through the `workflow` tool |
| Script calls | `phase`, `agent`, `log`, `pause`, `complete`, `budget`, `json_encode`, `fingerprint` (`xai-workflow/src/engine.rs`) | Its own TypeScript API; ships six skills (pi-workflows, monitor, autoplan, autodoc, autoimplement, sanity-check) |
| Limits | 64 phases, default 128 agents per run (max 1,024), 10,000 host calls (`xai-workflow/src/lib.rs`) | not read |
| Commands | `/workflow`, `/workflows`, `/create-workflow`, plus one command per named script | `/workflow`, plus a standalone live terminal viewer |
| Where scripts live | `~/.grok/workflows/`, `<repo>/.grok/workflows/`, and bundled ones (`xai-grok-shell/src/session/workflows/deep_research.rhai`) | Inside the package (`examples/workflows`) and Pi's resources |
| Switch | `[ui] pi_workflows` in `~/.grok/config.toml` (the "Pi workflows" row in Settings) | Listed in `packages` in `~/.pi/agent/settings.json` |
| Clash | Both register a Pi tool called `workflow`; both own `/workflow` | same |

A third package is also installed: `pi-extensible-workflows` 6.3.0, with its main extensions switched off in `~/.pi/agent/settings.json`. It registers `workflow` too (`docs/HARNESS-FACTS.md`, "Duplicate `workflow` tool").

## To use Rhai workflows in `grok grok-pi`

1. Take `"npm:@osolmaz/pi-workflows"` out of Pi's package list (or disable its extension) in `~/.pi/agent/settings.json` on sab-air. This also removes it from `pig` and plain `pi`, which share that file.
2. Set `pi_workflows = true` in `~/.grok/config.toml`.
3. Put scripts in `~/.grok/workflows/` or `<repo>/.grok/workflows/`. Note that `hello.rhai` (the spike-0 script) is in `~/.grok-pi/workflows/`, which only Dwsy's standalone binary reads; `~/.grok/workflows/` does not exist yet.
4. Start `grok grok-pi`; `/workflows` should list them.

Dwsy's standalone (`pig`) avoids the clash by itself: `~/.grok-pi/native-feature-conflicts.toml` blocks `@osolmaz/pi-workflows` while its Pi workflows switch is on, and `~/.grok-pi/config.toml` has `[pi.resources] block = ["npm:@osolmaz/pi-workflows"]`. So in `pig`, Rhai workflows are already the active system.

## Known gaps in the fork

- The built-in conflict list (`crates/codegen/xai-grok-pager/assets/native_feature_conflicts.toml`) names only `npm:@quintinshaw/pi-dynamic-workflows` for `pi_workflows`. It does not name `@osolmaz/pi-workflows` or `pi-extensible-workflows`.
- With the `grok-pi-acp` installed on sab-air (built 2026-11-09), neither Dwsy's conflicts file nor a `[pi.resources] block` in `~/.grok` stopped the crash (tested against a throwaway copy of the home). The resource-policy code (`c3462f9e`, 2026-11-12) is newer than that binary; a 2026-11-14 build exists in `target/release/` and is not installed, because `scripts/install-fork.sh` does not ship `grok-pi-acp`.
- When Pi exits at startup, the pager does not show Pi's error text.
- The fork's Pi adapter comes from Dwsy's 2026-10-13 branch, 91 commits behind his main; his workflow work since then is not in the fork.

These are in `docs/QUEUE-next-install.md`.

## Where to read

- Engine and script API: `crates/codegen/xai-workflow/src/` (`engine.rs`, `validate.rs`, `meta.rs`).
- A full example: `crates/codegen/xai-grok-shell/src/session/workflows/deep_research.rhai`.
- Script discovery: `crates/codegen/xai-grok-shell/src/session/workflow/registry.rs`.
- Pi side of the spawn: `extensions/pi-grok-workflows/index.ts`; adapter `crates/codegen/pi-grok-adapter/src/{pi_workflow_backend.rs,workflow_host.rs}`.
- Feature row: `docs/FEATURE_MATRIX.md` line 53.
- The package: `~/.pi/agent/npm/node_modules/@osolmaz/pi-workflows/README.md` on sab-air.
