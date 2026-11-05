# grok-hub: one pager, agents on other machines

Moved here from `~/.dsh/scripts` (dsh-home) on 2026-11-05; earlier history lives in dsh-home.
`~/.dsh/scripts/<name>` stay as symlinks into the pinned worktree below, so existing paths keep working.

## Layout

- The mini owns the hub: `~/.grok/bin/grok-team-hub agent leader --leader-socket ~/.grok/leader-hub.sock`.
- Other Macs run `~/.grok/bin/grok-team-client` against it.
- A remote agent is the leader running `ssh -T <user@host> <acp command>`, offered to the pager as
  `--agent-choice NAME=CMD` (`crates/codegen/xai-grok-pager/src/app/agent_choice.rs`).

## Entry points

| Command | What it does |
|---|---|
| `grok pick` | fzf machine, then agent, then launch (`grok-picker.zsh` → `grok-entry.sh`) |
| `gk [worker]` / `gk all` / `gk workers` | hub client: open a session on one or every worker |
| `gk add-worker NAME USER@HOST` | register a machine in `~/.config/gk/workers.tsv` |
| `gb <agent...> [+machine]` | one pager with one or more agents (`prime`, `prime-h2`, `claude`, `pi`, `codex`, ...) |
| `gb-build`, `grok-swap`, `grok-hub-sync`, `grok-sign` | build, swap, sync and sign the fork binaries |
| `fh`, `fh-clean`, `fh-stop` | fleet health, build debris, stop builds per machine |

Machines beyond the built-ins go in `~/.config/gk/workers.tsv` (gk) and `~/.config/gb/machines.tsv` (gb).

## Install on a machine

```sh
git -C ~/repos/grok-build worktree add -b hub-live .worktrees/hub origin/main
```

`~/.dsh/scripts` symlinks point at `/Users/sab-mini/repos/grok-build/.worktrees/hub/scripts/hub/`.
The worktree pins `main`, so switching branches in the main checkout never breaks `grok`.
Update with:

```sh
git -C ~/repos/grok-build/.worktrees/hub pull --ff-only
```
