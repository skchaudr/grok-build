# Proof: `[[agent_choice]]` in config.toml drives the new-session picker

**What was proven (2026-10-19, khoj, live, nothing faked):** a plain pager launch with **no
`--agent-choice` flags** offers the `claude` / `pi` machine picker on "+ New Agent", read from
`~/.grok/config.toml`:

```toml
[[agent_choice]]
name = "claude"
cmd = "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/home/sab-mini/.local/bin/claude /home/sab-mini/.local/bin/claude-agent-acp"

[[agent_choice]]
name = "pi"
cmd = "/usr/bin/pi-acp"
```

Binary: `grok 1.0.45 (6ddfa579adc8)` (`version.txt`; the hash is the git HEAD at build time, the
build carried the uncommitted change on top of it). Installed at `~/.grok/bin/grok-fork`; the
previous `a22e167de374` binary is kept as `~/.grok/bin/grok-fork.a22e167d`. The systemd
`grok-leader` unit was not restarted (the picker is a pager/client concern).

Semantics: CLI `--agent-choice` flags, when present, replace the config list entirely (CLI
wins); the two are never merged. Order is file order, first entry is the default. `--agent-cmd`
alone is unchanged. Headless runs (`-p`, prompt files, memory flush) do not read the config list.
A malformed table (missing/empty/non-string `name` or `cmd`, a name that is not a machine name,
or `[agent_choice]` instead of `[[agent_choice]]`) is a launch error naming the table, not a
silent no-picker.

## Run

`proof.sh <out-dir>` drives the real pager in its own tmux server (`tmux -L agentchoice`) from
`/tmp`, same key sequences as `scripts/picker-live.sh`:

```
env -u CLAUDECODE GROK_AGENT_DASHBOARD=1 ~/.grok/bin/grok-fork --leader --leader-socket ~/.grok/leader.sock --always-approve --trust --fullscreen --no-alt-screen
```

1. capture 5 s after launch (`pane-5s-after-launch.txt`: home screen, header already reads
   `claude /tmp`, the first config choice)
2. `Ctrl+\` opens the dashboard, `Up` x15 clamps on `+ New Agent` (`pane-dashboard.txt`)
3. `Enter` -> the picker (`pane-picker-open.txt`): `New session / Machine / > claude / pi /
   enter select · esc cancel`
4. `Down` -> cursor on `pi` (`pane-picker-cursor-pi.txt`), `Esc` cancels
   (`pane-picker-cancelled.txt`, no picker text left)

`steps.log` is the timed trace, `launch-cmd.txt` the exact command, and
`config-agent-choice-lines.txt` the matching config lines (line numbers 624-630; no secrets).
The tmux session was killed afterwards. No agent session was created, so the leader's roster is
unchanged.

Unit tests beside the picker code (`cargo test -p xai-grok-pager --lib -- app::agent_choice app::cli`):
parse from TOML in order, CLI overrides config, empty config = no picker, malformed tables are
errors, headless skips the config read. 55 passed.
