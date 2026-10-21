# Multi-agent, one directory: worktrees and concurrency audit

Follow-up to `AUDIT.md`. Read-only code audit, 2026-10-21. Nothing built or run. Paths are under `crates/codegen/`.

## Worktrees

- **Entry points:**
  - `ctrl+w` on the welcome screen or dashboard.
  - `grok -w [NAME] [--ref REF]`.
  - `grok worktree create|list|show|rm|gc|db`.
  - `/fork --worktree`.
  - `hints.new_session_worktree_mode` and `hints.fork_worktree_mode` take ask, always or never.
  - There is no `/worktree` slash command.
  - Evidence: `xai-grok-pager/src/app/cli.rs:131,613-617`, `views/welcome/mod.rs:1779`.
- **Layout:**
  - Worktrees live at `~/.grok/worktrees/<repo_slug>/<label>`. Subagent worktrees go in `subagent-<id>` under the same root.
  - The registry is `~/.grok/worktrees.db`.
  - Evidence: `xai-grok-workspace/src/worktree/mod.rs:742-870`.
- **No branches:**
  - Worktrees are created with `git worktree add --detach --no-checkout`, so HEAD is detached.
  - They are seeded from the source's dirty tree by default, or from `--ref` for a clean checkout.
  - Evidence: `xai-fast-worktree/src/git/worktree.rs:10-20`, `xai-grok-pager/src/app/worktree_session.rs:21-50`.
- **Cleanup:**
  - Auto-GC is off unless `[worktree.auto_gc] enabled = true`. Its defaults are a 7d max age and a 6h interval, and `max_age_by_kind` overrides per kind.
  - Manual `grok worktree gc --max-age 7d` keeps worktrees holding work that would be lost.
  - Evidence: `xai-fast-worktree/src/auto_gc.rs:39-52`, `xai-grok-pager/src/worktree_cmd/mod.rs:52-65`.
- **Subagent isolation:**
  - Set `isolation: worktree` on `task`, or `default_isolation = "worktree"` per role or persona. The default is none.
  - If creating the worktree fails, the subagent silently falls back to the shared directory.
  - Merge-back (`x.ai/git/worktree/apply`) is documented, but no user command was found.
  - Evidence: `xai-grok-subagent-resolution/src/overrides.rs:112-120`, `xai-grok-shell/src/agent/subagent/handle_request.rs:751-765`.
- **Memory scope:**
  - Memory is keyed by the normalized `origin` remote, so worktrees share memory only when the repo has an origin. Without one, scope falls back to a hash of the path.
  - This corrects `AUDIT.md`, which says worktrees share scope unconditionally.
  - Evidence: `xai-grok-memory/src/storage.rs:714-759`.
- **External agents:** no path was found that isolates a bridged Claude, Codex or Cursor agent in a worktree, other than launching it from a worktree directory. `grok -w --agent-cmd` is unverified.

## Same-directory concurrency

Nothing protects two agents editing the same file or git index in one directory.

| Area | Protection | Gap |
|---|---|---|
| Edit tool (`grok_build/search_replace/mod.rs:138-260`) | Re-reads the file and requires an exact `old_string` match | No mtime or hash guard. Read-before-edit is not enforced. The write is non-atomic, so another agent's non-overlapping edit made after the read is lost. |
| Path lock (`xai-grok-tools/src/util/lock_path.rs:1-12`) | Serializes writes within one session's parallel batch | No lock across sessions or processes |
| Rewind (`xai-grok-workspace/src/session/file_state.rs:780-880`) | Detects external modification | Overwrites anyway, so it can wipe another agent's later edits |
| Git (`session/git.rs:2071-2100`) | Stashes before a destructive session restore | Nothing serializes `commit`, `add` or `checkout` between agents |
| `config.toml` | Atomic tmp+rename | Last writer wins |
| `session_search.sqlite` | Multi-process safe | Grok-native sessions only |
| Dashboard db, memory inbox | No lock found | Races unverified |
| Leader (`leader/lock.rs`) | One flocked leader per machine | Bridged external agents run outside it |

## Other multi-agent features

- **`task` subagents:**
  - Supports a model override, background runs and `resume_from`.
  - Concurrency is capped at 32 by `GROK_MAX_CONCURRENT_SUBAGENTS`. Excess spawns queue by default.
  - Depth is capped at 1, so subagents cannot nest.
  - Evidence: `task/admission.rs:6-50`, `task/mod.rs:43`.
- **Workflows:** the `workflow` tool runs Rhai scripts from `.grok/workflows/` and `~/.grok/workflows/`.
- **Scheduler and monitor tools.** No agent-teams feature was found.
- **Per-agent `permissionMode`:** Default, AcceptEdits, Auto, DontAsk, BypassPermissions, Plan (`xai-grok-agent/src/config.rs:716`).
- **Notifications:**
  - Configured under `[ui.notifications]`. Events are TurnComplete, ApprovalRequired, TaskComplete and AgentError.
  - `[[ui.notifications.hooks]]` runs a command per event.
  - Whether bridged agents emit these events is unverified.
  - Evidence: `xai-grok-pager/src/notifications/config.rs:5-124`.

## Recommendations

- Give each long-lived agent its own worktree (`cd "$(grok worktree create x)"`), and launch Claude, Codex and Cursor from there. This is the only real isolation.
- In `config.toml`:
  - `hints.new_session_worktree_mode = "always"`.
  - `[worktree.auto_gc] enabled = true`, with short `max_age_by_kind` for Subagent worktrees.
  - `default_isolation = "worktree"` for write-capable subagent roles.
- Route `[[ui.notifications.hooks]]` (approval_required, agent_error) to Herdr or push notifications.
- Before running `/rewind` in a directory another agent also edits, check the conflict list.
- Keep an `origin` remote so memory is shared across worktrees.

## Gaps worth building

1. A cross-process edit guard: refuse the write if the file's mtime or hash changed since it was read. Make rewind skip conflicted files.
2. A git-operation lock per repo, or a `pre_tool_use` hook that denies git writes while another live session shares the repo and directory.
3. A warning when several agents share a directory ("N agents in this dir"), fed by the leader, the dashboard and `/traces`, with an offer to spin up a worktree.
4. A `/worktree` command and a merge-back command for subagent worktrees.
5. Named-branch worktrees (`-b`), with GC that never deletes unmerged work.
6. Notifications for bridged agents, and `/traces` data feeding the dashboard.
