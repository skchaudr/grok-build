# ACP agent matrix

Smoke matrix for ACP agents launched through `xai-grok-pager --agent-cmd`.

## How to run

```bash
scripts/acp-matrix.sh <agent>
```

Agents (exact ids): `dsh`, `cursor`, `pi`, `codex`, `grok`, `claude`.

| Agent | `--agent-cmd` |
| --- | --- |
| dsh | `dsh --profile acp-enhanced` |
| cursor | `cursor-agent acp` |
| pi | `pi-acp` |
| codex | `codex-acp` |
| grok | `grok agent stdio` |
| claude | `env CLAUDE_CODE_EXECUTABLE=$(command -v claude) claude-code-acp` |

Pager default: `/home/sab-mini/repos/grok-build/target/release/xai-grok-pager` (override with `ACP_PAGER=`). The script does not read `$PAGER`, because that variable is the system pager.

The script opens a detached tmux session at 120×36, working directory `/tmp/acp-matrix-<agent>`. `--always-approve` is passed only with that throwaway cwd. The prompt asks the agent to run `ls` and stop. Exit status is 0 only when the scrollback capture shows the prompt, assistant text, and a tool card. Slash-command and `session/load` resume probes are recorded and do not change the exit status.

### DSH route

For `dsh`, the script sets `DSH_ACP_PROVIDER=cliproxy` and `DSH_ACP_MODEL=gemini-3.8-flash-high`. CLIProxyAPI is `http://100.66.99.64:8317/v1`. `CLIPROXY_API_KEY` is read from `~/.dsh/.credentials.yaml` and is not printed. `acp-enhanced` does not register a cliproxy adapter, so the script also passes a throwaway `--patch` that mounts that one model. The patch is written under `/tmp/acp-matrix-out/dsh/` and is not a profile edit.

## Results

Artifacts per run: `/tmp/acp-matrix-out/<agent>/` (`pane.txt`, `scrollback.txt`, `resume.txt`, `slash.txt`, `result.env`).

| Agent | launch | prompt | assistant text | tool card | slash commands | session/load resume | note |
| --- | --- | --- | --- | --- | --- | --- | --- |
| dsh | yes | yes | yes | yes | `/dashboard` `/resume` `/copy` `/rename` `/quit` `/always-approve` `/compact` `/settings` | no | `◆ Run ls` and the reply `done` on `cliproxy/gemini-3.8-flash-high`. |
| cursor | yes | yes | yes | yes | `/dashboard` `/resume` `/copy` `/rename` `/quit` `/always-approve` `/compact` `/settings` | no | Reply was `I'll run ls and then reply.` then `Done.` after `◆ Run ls`. Status line showed `grok-4.7`. |
| pi | yes | yes | yes | yes | `/dashboard` `/resume` `/copy` `/rename` `/quit` `/always-approve` `/compact` `/settings` | no | Reply was `done.` after `◆ Run ls`. Status line showed `google-antigravity/Gemini 3.6 Flash (Antigravity)`. |
| codex | yes | yes | no | no | `/dashboard` `/resume` `/copy` `/rename` `/quit` `/always-approve` `/compact` `/settings` | no | Turn failed: `gpt-5.6-sol` requires a newer Codex. Pane also said model metadata for that id was not found. |
| claude | yes | yes | yes | yes | `/dashboard` `/resume` `/copy` `/rename` `/quit` `/always-approve` `/compact` `/settings` | no | `Running ls now.` then `◆ Run List files in current directory`. Status line showed `Fable 5.1`. Needs pager ≥ 8a5fd08d (stub-authenticate fix). `CLAUDE_CODE_EXECUTABLE` must point at the system `claude` (2.1.292); the adapter's bundled SDK CLI (2.1.44) rejects current models. Run outside a Claude Code session (`CLAUDECODE` unset) or the nested-session guard kills `session/new`. |
| grok | no | no | no | no | none | no | Pager exits 1 during initialize: `unable to receive 'initialize' response, channel closed`. No composer. |

`--continue` was probed after every agent that stayed up. None of the debug logs contain `session/load`, and the resume captures were empty.

A row is filled only from a pane capture, not from an unchecked exit status.
