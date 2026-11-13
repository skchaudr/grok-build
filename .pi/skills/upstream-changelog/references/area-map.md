# Area Map — crate/path → human-readable area

Use this to group `git diff --numstat` output into the Areas-touched table and
to map upstream `Changes:` prefixes (e.g. `Pager:`, `Shell:`) to crate paths.

| Area label | Crate paths / glob | Upstream prefix |
|---|---|---|
| Pager (TUI) | `xai-grok-pager`, `xai-grok-pager-bin`, `xai-grok-pager-render`, `xai-grok-pager-minimal`, `xai-grok-pager-pty-harness` | `Pager:` |
| Shell (agent runtime) | `xai-grok-shell`, `xai-grok-shell-base`, `xai-grok-shell-session-support` | `Shell:` |
| Tools | `xai-grok-tools`, `xai-grok-tools-api` | `Tools:` |
| Workspace / Permission | `xai-grok-workspace`, `xai-grok-workspace-client`, `xai-grok-workspace-types` | `Workspace:`, `Security:` |
| Sandbox | `xai-grok-sandbox` | `Sandbox:` |
| Workflow (new crate) | `xai-workflow` | (workflow bullets) |
| Prompt queue | `xai-prompt-queue` | `Pager:` (queue-related) |
| Voice | `xai-grok-voice` | `Voice:` |
| Worktree / GC | `xai-fast-worktree` | `Worktree:` |
| Auth / Secrets | `xai-grok-auth`, `xai-grok-secrets` | `Security:` |
| Config | `xai-grok-config`, `xai-grok-config-types` | `Config:` |
| Models / Sampling | `xai-grok-models`, `xai-grok-sampler`, `xai-grok-sampling-types` | `Shell:` (model-related) |
| MCP | `xai-grok-mcp` | `MCP:` |
| Hooks / Plugins | `xai-grok-hooks`, `xai-hooks-plugins-types`, `xai-grok-plugin-marketplace` | `Hooks:`, `Plugins:` |
| Memory | `xai-grok-memory` | `Memory:` |
| Markdown / Mermaid | `xai-grok-markdown`, `xai-grok-markdown-core`, `xai-grok-mermaid` | `Pager:` (rendering) |
| Update / Version | `xai-grok-update`, `xai-grok-version` | `Update:` |
| Telemetry / Mixpanel | `xai-grok-telemetry`, `xai-mixpanel` | `Telemetry:` |
| ACP / Protocol | `xai-acp-lib`, `xai-tool-protocol`, `xai-tool-runtime`, `xai-tool-types` | `Proto:`, `ACP:` |
| Agent lifecycle | `xai-agent-lifecycle`, `xai-grok-agent`, `xai-grok-subagent-resolution` | `Shell:` (agent-related) |
| Chat state | `xai-chat-state` | `Chat:` |
| Hunk tracker | `xai-hunk-tracker` | `Pager:` (diff-related) |
| Textarea / Inline | `xai-ratatui-textarea`, `xai-ratatui-inline` | `Pager:` (editing) |
| Token estimation | `xai-token-estimation` | `Shell:` (context-related) |
| Compaction | `xai-grok-compaction` | `Shell:` (compaction) |
| Computer Hub | `xai-computer-hub-core`, `xai-computer-hub-mcp-adapter`, `xai-computer-hub-sdk` | `Hub:` |
| Pi-Grok adapter (fork-only) | `pi-grok-adapter` | (fork-specific, not upstream) |
| Extensions (fork-only) | `extensions/` | (fork-specific) |
| Website | `website/` | (docs/site) |
| Third-party / vendored | `third_party/` | (vendored deps) |
| Root / meta | `Cargo.toml`, `Cargo.lock`, `SOURCE_REV`, `AGENTS.md`, `README.md` | (meta) |

## Usage notes

- When a `Changes:` bullet has an explicit prefix (e.g. `Pager: add ...`), use
  the corresponding area label directly.
- When a bullet has no prefix, infer the area from the diff paths or keywords.
- The "Areas touched" table in the changelog should list only areas that
  actually have file changes in the range, sorted by descending +/- magnitude.
- `pi-grok-adapter` and `extensions/` are fork-only; upstream diffs never touch
  them. If they appear in a diff, something is wrong — flag it.
