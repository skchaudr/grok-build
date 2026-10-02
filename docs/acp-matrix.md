# ACP agent matrix

Smoke matrix for ACP agents launched through `xai-grok-pager --agent-cmd`.

## How to run

```bash
scripts/acp-matrix.sh <agent>
```

Agents (exact ids): `dsh`, `cursor`, `pi`, `codex`, `grok`.

| Agent | `--agent-cmd` |
| --- | --- |
| dsh | `dsh --profile acp-enhanced` |
| cursor | `cursor-agent acp` |
| pi | `pi-acp` |
| codex | `codex-acp` |
| grok | `grok agent stdio` |

Pager default: `/home/sab-mini/repos/grok-build/target/release/xai-grok-pager` (override with `ACP_PAGER=`). The script does not read `$PAGER`, because that variable is the system pager.

The script opens a detached tmux session at 120×36, working directory `/tmp/acp-matrix-<agent>`. `--always-approve` is passed only with that throwaway cwd. The prompt asks the agent to run `ls` and stop. Exit status is 0 only when the scrollback capture shows the prompt, assistant text, and a tool card. Slash-command and `session/load` resume probes are recorded and do not change the exit status.

### DSH route

For `dsh`, the script sets `DSH_ACP_PROVIDER=cliproxy` and `DSH_ACP_MODEL=grok-4.7`. CLIProxyAPI is `http://100.66.99.64:8317/v1`. `CLIPROXY_API_KEY` is read from `~/.dsh/.credentials.yaml` and is not printed.

## Results

Artifacts per run: `/tmp/acp-matrix-out/<agent>/` (`pane.txt`, `scrollback.txt`, `resume.txt`, `slash.txt`, `result.env`).

| Agent | launch | prompt | assistant text | tool card | slash commands | session/load resume | note |
| --- | --- | --- | --- | --- | --- | --- | --- |
| dsh | unknown | unknown | unknown | unknown | unknown | unknown | not run |
| cursor | unknown | unknown | unknown | unknown | unknown | unknown | not run |
| pi | unknown | unknown | unknown | unknown | unknown | unknown | not run |
| codex | unknown | unknown | unknown | unknown | unknown | unknown | not run |
| grok | unknown | unknown | unknown | unknown | unknown | unknown | not run |

A row is filled only from a pane capture, not from an unchecked exit status.
