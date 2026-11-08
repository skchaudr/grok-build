# Review: Claude command forwarding / fork (gc/claude-parity-r2)

Read only. Nothing compiled or run. Paths under `crates/codegen/xai-grok-pager/`.

## Findings

1. **major** `src/app/dispatch/session/fork.rs:45-60` (`dispatch_fork`, `open_fork_question`), not touched by the diff.
   `/fork` in a Claude session inside a git repo still opens "Run this fork in an isolated git worktree?" (default `fork_worktree_mode = Ask`, `current_branch` is set pager-side). "Yes", `--worktree`, or `Always` emits `Effect::CreateWorktreeSession` (effects/mod.rs:349), a Grok shell path with no Claude equivalent. The new `claude` branch only covers `worktree == false`.
   Fix: in `dispatch_fork`, when `app.is_claude_session()`, skip the modal and call `dispatch_fork_resolved(app, false, ..)`. For `--worktree`, toast "worktree forks aren't supported with Claude".

2. **minor** `src/app/effects/mod.rs:5019-5043`. `new_session_id` (the `--fork` startup flag) is silently dropped for Claude, since `session/fork` can't take a client id. Acceptable. Add a one-line comment, or fail loudly if `Some`.

3. **minor** `src/app/effects/mod.rs:5026-5041`. Duplicates the `acp_send(ForkSessionRequest)` + Ready/Failed mapping from `Effect::ClaudeRewindFork` (effects/mod.rs:4922-4937), but the plumbing is not duplicated where it matters. Result handling goes through the same `handle_fork_session_ready` (fork.rs ~455), which rewind also calls (rewind.rs:572). The only copy is a ~12-line request wrapper. Fine; not worth a helper.

4. **minor** `src/slash/commands/effort.rs:21-22` (`args_required: true`) with `src/slash/mod.rs:1475` (`is_command_complete`). A bare `/effort` is "incomplete", so Enter does not submit it. In the Claude CLI a bare `/effort` reports the current level ("Effort level: auto"), but the user can't send it. The forward filter in prompt.rs:1022 only applies once a submission reaches dispatch. I did not trace the Enter handler in `agent_view/prompt.rs:157` to its end.
   Fix: make `is_command_complete` treat forwarded builtins as complete for Claude, or have the registry report `args_required == false` in Claude sessions.

5. **minor** Tests. No test covers (a) the `.filter` in prompt.rs:1022-1028 (a Claude session's `/effort high` becomes `PassThrough("/effort high")`, a native session's still runs `EffortCommand`), or (b) the Claude branch of `Effect::ForkSession`. Only `claude_forwards_builtin` and `is_claude_label` are tested. Add one dispatch test for (a), since non-Claude invariance is the pass bar.

6. **minor** `src/slash/external_common.rs:51-56`. The list is correct for what exists, but `fork` is intentionally not forwarded (the CLI's `/fork` spawns a background agent, which is the wrong semantics). Good. One doc nit: the comment says "grok-only ACP methods (`x.ai/hooks/list`...)" but `/effort` fails for a different reason, covered under "Checked" item 3 below. Reword.

## Checked, no problem

1. **Compile by reading.**
   - `acp::ForkSessionRequest::new(impl Into<SessionId>, impl Into<PathBuf>)` (schema 0.11.4 `agent.rs:1283`) matches the call.
   - `resp.session_id` exists on `ForkSessionResponse` (`agent.rs:1336`).
   - `acp_send` with this request type is already used at effects/mod.rs:4922.
   - `claude: bool` is moved into the async block. `parent_session_id.clone()` / `parent_cwd.clone()` are taken before the move into `ForkSessionReady`. The early `return` leaves the non-Claude tail untouched, so there is no borrow or move conflict. `sanitize_user_error` and `session_flags.external_agent_label` are in scope (used at effects/mod.rs:1318).
   - prompt.rs: `external_claude` is a `bool` defined at :853 in the same fn (`dispatch_send_prompt_submission`, :747). The `.filter(|_| ..)` on `Option<&Arc<dyn SlashCommand>>` followed by `.cloned()` type-checks. `invocation.token` is `&str`.
   - external_common.rs: `claude_forwards_builtin` is plain; tests fine. `agent_label_from_cmd("env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=... /x/claude-agent-acp")` gives `claude-agent` (env skipped, `-u` arg skipped, `NAME=value` skipped, `-acp` stripped), so the new test passes.

2. **Fork binding.** After `session/fork` returns, `TaskResult::ForkSessionReady` goes to `handle_fork_session_ready` (task_result.rs:549). It does `bind_session_id`, begins the replay window, then emits `Effect::LoadSession`. This is the same function rewind reuses (rewind.rs:572), so binding is correct and shared. The adapter with no `_meta` fork point does `forkClaudeSession(..)` of the whole transcript (`dist/fork-session.js`, `if (!point)` branch). Failure maps to `ForkSessionFailed` with a sanitized "fork failed: ..." message, not "Method not found". The pre-existing `StartupForkSession` path now works for Claude too.

3. **`/effort`.**
   - Pager-native `/effort` would fail for Claude. `EffortCommand` needs `models.reasoning_effort_options()` (`model_state.rs:278`), which reads `supportsReasoningEffort` model meta. The adapter exposes effort as a separate session config option (`session-effort.js` `buildEffortConfigOption`), which the pager does not map. So there is no working pager picker for Claude to break, and forwarding is the right call.
   - The adapter sends the prompt text unmodified to the CLI. `promptToClaude` only rewrites `/mcp:`, and `LOCAL_ONLY_COMMANDS` is just `/context`, `/heapdump`, `/extra-usage`. The CLI binary contains the "Set effort level to ..." / "Effort level: auto" handler.
   - Output comes back as `local_command_output`, which sends an `agent_message_chunk` (acp-agent.js:2771-2787). If the CLI returns it only as `result` text, the zero-output-token fallback (acp-agent.js:3695-3705) forwards it.
   - A silent turn is covered by `push_silent_forwarded_slash_notice` (agent_view/queue.rs:324).
   - Not verified live: I did not run the CLI. Also the adapter's effort config option and `effortPinnedLevel` are not updated by a CLI-side `/effort`, so a later model switch may re-seed effort (acp-agent.js ~5539-5590).

4. **`/mode`.** The "Unknown command" claim is wrong for this user. `~/.claude/commands/mode.md` exists, and the CLI advertises it, so `mode` appears in the adapter's `available_commands` (`getAvailableSlashCommands`, acp-agent.js:6960-7000; `mode` is also in `claude_menu::ADAPTER_COMMANDS`). The pager has no builtin named or aliased `mode` (rg over `src/slash`), so `/mode x` resolves to the Acp command and goes out as `PassThrough` or `InjectSkill` (acp_command.rs:133-150). It reaches the user's command. Wherever the agent wrote "no mode command in the CLI", correct it to "mode is the user's own command/skill".

5. **`is_claude_label`.** `rsplit('/').next().unwrap_or(l).starts_with("claude")` returns true for `@agentclientprotocol/claude-agent-acp` and false for `pi`, `None`, and any non-claude basename (`cursor-agent`, `codex`, `/path/pi`). This matters because the registry label can be the adapter's `session_agent_name` (app_view.rs:5425-5430), not the cmd-derived label. Only a path-style label whose last segment is not `claude*` but whose first is (e.g. `claude/foo`) flips from true to false; none ship.

6. **Non-Claude behavior.** The prompt.rs filter is gated on `external_claude` (`app.external_agent && is_claude_label`), and the effects change on `session_flags.external_agent && is_claude_label`. Native grok, pi, cursor and codex take the old paths unchanged. Item 1 (the worktree modal) is the only gap, and it is Claude-only.

7. **Pass-bar commands.**
   - `/effort`, `/hooks`, `/plugin`, `/plugins`, `/marketplace`: forwarded as text. `/hooks` and `/plugin` are `local-jsx` in the CLI and answer "opens an interactive panel and isn't available in this environment" (CLI binary string), which says why.
   - `/fork`: the pager builtin plus standard `session/fork`; see item 1.
   - `/mode`, `/advisor`, `/reload-plugins`: adapter-advertised, so they go as Acp commands. `/reload-plugins` depends on the menu change's `BLOCKED_ACP_NAMES` exemption (registry.rs `!claude && ...`), which is consistent with this change.
   - `/status`, `/statusline`, `/remote-control`: not pager builtins. Not advertised (terminal-bound), so a typed one is an unknown command and `PassThrough`s as text to the CLI (prompt.rs:1060-1063). Whether the CLI answers with a clear message or a model turn is unverified. `status` is `local-jsx` (message path), `statusline` is a prompt command flagged `terminalOriented`, `remote-control` is not checked. No pager "Method not found" is possible on this path.

8. **Conflicts with the menu change.** None. `marketplace` is menu-hidden (claude_menu `HIDDEN`) and forwarded here, so typed in full it reaches the CLI. `effort`, `hooks`, `plugin` and `fork` appear in `TOP`, with the same dispatch outcome as described above. The queue-edit and plan-approval paths (`is_complete_builtin_invocation`) re-enter `dispatch_send_prompt_submission`, so the filter applies there too.

## Verdict

Compiles by reading and non-Claude paths are unchanged. Fix the `/fork` worktree modal for Claude (item 1) before merge, and add the dispatch test (item 5).
