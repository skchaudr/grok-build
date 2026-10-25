Verdict: PASS

bb threads are on `grok live`. A Claude session whose id is `events.provider_thread_id` is one row, `via bb`, titled with the bb thread. Other providers stay `bb` rows. `thread.active` is working, `thread.idle` / `thread.failed` is your turn, `thread.archived` / `thread.deleted` drops the row.

## Commands

- `cargo test -p xai-grok-traces --offline` (full crate; live_replay ignored)
- `cargo build -j 4 --release -p xai-grok-traces --bin grok-traces -p xai-grok-pager-bin --bin xai-grok-pager`
- Installed those binaries on sab-air as `~/.grok/bin/grok-traces` and `~/.grok/bin/grok-fork` (backups `*.bak-20261024-205820`), copied `hooks/bb/server.ts`, `bb plugin reload grok-traces-hook`
- `ssh sab-air ~/.grok/bin/grok-traces live --json`
- Air tmux `grok-fork live` at 80x24 and 120x40
- `bb thread spawn --project proj_g52ng3r92c --title grok-live-probe --prompt "Reply with exactly the word ready."` → `thr_pd3mri3ram`
- `bb thread tell thr_pd3mri3ram "say pong"` → JSON state `working` at 0.00s after tell returned, and again at 1.34s / 1.78s; `prompt` (your turn) after
- `bb thread delete thr_pd3mri3ram --yes` → row absent on the next poll
- Mini tmux `grok-fork live` at 120x40 and 80x24, filter `Get BB`

## Seen

- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-80x24.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-80x24.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-120x40.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-120x40.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-working-80x24.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-working-80x24.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-working-120x40.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-working-120x40.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-idle-80x24.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-idle-80x24.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-idle-120x40.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-idle-120x40.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-gone-80x24.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-gone-80x24.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-gone-120x40.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/air-probe-gone-120x40.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/mini-80x24.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/mini-80x24.txt`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/mini-120x40.ansi`
- Seen: `/Volumes/Mini-SSD/offload/repos/sab-orchestrate/cells/gb-bb-live/captures/mini-120x40.txt`

Air 120x40 steady: `Get BB running in Zed via bb` is `○ your turn`; `DSH-W02B/W03B Canary Ingest` is a `bb` row. Mini 120x40: `sab-air: Get BB… via bb` is `○ your turn`. Probe 120x40 working: `● grok-live-probe via bb` / `pong`. After delete the filter is `no match`.
