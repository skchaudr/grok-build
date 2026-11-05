# grok-hub: one pager, agents on other machines (2026-11-05)

Sab built this Oct 9–22 and lost it: it lived in `~/.dsh/scripts` (dsh-home), outside this repo. It now lives in `scripts/hub/` (README there). Commits: `6bef30b7` move, `14a833b6` drop khoj.

**Shape.** The pager (`grok-team-client`) runs where Sab sits. The leader (`grok-team-hub agent leader --leader-socket ~/.grok/leader-hub.sock`) runs on the Mini only. The Air reaches it through `grok-mini-tunnel` (tmux `hermes-proxies`, window `grok-mini`), which forwards it to `~/.grok/leader-mini-hub.sock`. For a remote session the leader runs `ssh -T user@host <ACP agent cmd>`; the pager side is `--agent-choice NAME=CMD` (`515d9529`, `crates/codegen/xai-grok-pager/src/app/agent_choice.rs`).

**A worker needs only:** sshd reachable from the Mini (host key in the known_hosts file named on its worker line), an ACP agent binary, and that agent's auth. No hub scripts.

**Workers (verified 2026-11-05 with `gk ask`, each answered with its own hostname):**

| gk name | Host | Agent cmd |
|---|---|---|
| mini | local | `grok-team-hub agent --no-leader stdio` (built-in) |
| air | sab-mini@100.93.242.91 | `grok-team-client agent --no-leader stdio` (built-in) |
| old-air | saboor@100.109.233.105 | stock grok 1.0.70 `agent --no-leader stdio` |
| pi-big | sab-ssd@100.78.181.120 | stock grok 1.0.70 (aarch64) |
| pi-small | sab-ssd@100.73.28.125 | stock grok 1.0.70 (aarch64); its OS hostname is `ssd-big` |

The last three are in `~/.config/gk/workers.tsv` on the Air and the Mini. khoj is retired for good. sabibp-1 was offline (Tailscale last seen 48 days ago, LAN `192.168.1.212` times out), so it is not set up. old-air also has `pi-acp` and `claude-agent-acp`.

**Install layout.** `~/.dsh/scripts/<name>` are symlinks to `/Users/sab-mini/repos/grok-build/.worktrees/hub/scripts/hub/<name>`, a worktree on branch `hub-live` that tracks `origin/main`, so branch switches in the main checkout don't break `grok`. On the Mini, `~/repos/grok-build` links to `/Volumes/Mini-SSD/offload/repos/grok-build`. Update: `git -C ~/repos/grok-build/.worktrees/hub pull --ff-only`.

**Open:**
- `fh-clean` lists clean worktrees under `~/repos/*/.worktrees/` as deletable, which includes the hub worktree. On the Mini the worktree also needs the USB SSD mounted. Proposed fix, not done: move the pinned worktree to `~/.grok-hub` and repoint the dsh-home symlinks.
- `gk add-worker` still defaults `--script` to the khoj-era `grok-khoj-worker`; pass `--script` for every new machine.
- Interactive `gk <worker>` sessions were not opened in this pass; only `gk ask` (headless, same leader path).
- sabibp-1 and the future VM: power on, then `gk add-worker NAME USER@HOST --script '<abs path> agent --no-leader stdio' --known-hosts /Users/sab-mini/.ssh/known_hosts`.
- grok-pi is installed only on the Air (0.1.11, older than its source); the Mini has none.
