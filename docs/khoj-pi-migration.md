# Pi migration brief for Kimi K3 (Sab's decision 2026-10-19: wipe khoj Pi, replace with the Air's Pi)

Target state on khoj = the Air's Pi, verbatim:
- Pi 1.3.0 managed install (`~/.pi/agent/install/releases/1.3.0`, `managed-install.json`, `current-version`), entrypoint symlink `~/.local/bin/pi` -> `~/.pi/agent/bin/pi`.
- `~/.pi/agent/settings.json` copied verbatim from the Air. KEEP the four `-dist/...` disabled entries under `npm:pi-extensible-workflows`; they prevent the duplicate `workflow` tool that makes `pi --mode rpc` exit and breaks every `grok pi` session (grok-build OBSERVATIONS.md:587).
- `~/.pi/agent/models.json` from the Air with the cliproxy provider pointed at `http://127.0.0.1:8317/v1` (khoj reaches the Air's CLIProxyAPI through the systemd user unit `cliproxy-air-tunnel.service`; keep that unit).
- Air packages only: pi-typesafe, pi-jev-context, @narumitw/pi-typesafe-compact, pi-extensible-workflows (as above), @osolmaz/pi-workflows, pi-cursor-sdk. One local extension: `extensions/codemode-command.ts`.
- Node 26 (Air) vs 22 (khoj): check Pi 1.3.0's engines field; upgrade khoj node if required.

Wipe on khoj: `~/.pi` (all of it), `~/.local/share/pi` (0.87.1), `~/.local/pi-1.0.4` (pin), the bash wrapper at `~/.local/bin/pi`, `~/.pi/agent/scripts/ensure-host-settings.py` (goes with ~/.pi).

Must be restored after the wipe (grok-build depends on them):
- `~/.pi/pi-acp-patched/` : patched pi-acp 0.0.33 (index.js, run, subagents.mjs, subagent-wrapper.mjs, pi-install-resolve.mjs, package.json). Source of truth: `~/repos/Pi-Coding-Agent/pi-acp-patched/`. The Air's `run` script resolves node_modules from a global `pi-acp` install (`npm i -g pi-acp@0.0.33`); do the same on khoj.
- `~/.pi/pi-acp/session-map.json` (back it up before the wipe; it maps picker sessions to cwds).
- `pi-install-resolve.mjs:107` falls back to `~/.local/share/pi/bin/pi` (the old path). Harmless once `pi` is on PATH; do not recreate the old path to satisfy it.
- Do not touch `~/.grok` or the `grok-leader` systemd unit; its PATH already has `~/.local/bin` first.

Known consequence (Sab accepted): GDAD dispatch in `~/gddp-runtime` / `~/gddp-config` (`runtime/settings.env`, `scripts/test_gdad_run.py`) points at `~/.local/pi-1.0.4` and pi-durable-subagents; it breaks until repointed.

Proof of done (all three on disk in a repo, never /tmp). All three proofs go in this repo under `docs/`, next to `picker-live-proof.md`, where the Air and Mini proofs already live. Any adapter change goes to `~/repos/Pi-Coding-Agent/pi-acp-patched/`, not to grok-build.
1. `pi --version` on khoj prints 1.3.0 and `pi --mode rpc` starts without exiting.
2. `grok pi` on khoj via the leader completes one turn (the picker-live script's khoj case).
3. `diff <(ssh sab-air cat ~/.pi/agent/settings.json) ~/.pi/agent/settings.json` is empty.

## Fourth proof of done (added 2026-10-19, revised 10:50 UTC)

`scripts/picker-live.sh khoj local` (default Pi model is now
`openrouter/inception/mercury-2.5`) must be PASS 4/4 on the fresh install, with
every `resumed=yes`. It passes today only because khoj's settings.json disables
`extensions/harness-state.ts`, whose status line trips Mercury's refusal filter.
The wipe removes that extension and the Air settings.json carries no such entry;
this proof shows Mercury still answers on the clean install. See
docs/picker-live-proof.md, 10:47 UTC section.
