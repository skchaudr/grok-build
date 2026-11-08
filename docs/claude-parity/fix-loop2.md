# Fix task (loop 2) — branch gc/claude-parity-r2, this worktree

Do NOT run cargo (one test + build happens after you). Commit when done with a message ending in:
`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`

Read `.review-loop2.md` (the review) and `.claude-menu-undecided.md` (kept/hidden table). Pager crate: `crates/codegen/xai-grok-pager/src/`. Apply exactly:

1. `slash/registry.rs` ~597: only clone into `acp_catalog` when the catalog changed (`if self.acp_catalog.as_slice() != commands`). Do not change `set_external_agent`'s return value.
2. `slash/claude_menu.rs` HIDDEN: the rule is "hide only commands that fail in a Claude session because they send a grok-only `x.ai/*` request the stock claude-agent-acp lacks". Remove from HIDDEN every builtin that is pager-local, including `announcements`, `tutorial`, `docs`, `release-notes`, `import-claude`, `voice`, and any other pager-local entry from the earlier hand-picked list (`login`, `logout`, `privacy`, `feedback`, `marketplace`, `personas`, `config-agents`) — check each: keep hidden only if it sends an `x.ai/*` request or is grok-account-only in a way that errors. Keep `remember` visible (it will be rerouted later). Update the table file with the final status and evidence.
3. Hidden builtins typed in full in a Claude session (and their aliases, e.g. `/summarize` → recap) must not send the `x.ai/*` request: show one system line "/<name> isn't available in Claude sessions" instead. Exception: `recap`/`summarize` when the adapter advertises `recap` → route to the adapter's command. Smallest change at the dispatch point; add one unit test.
4. Leave `/dashboard`, `/history`, `/settings` as they are.
5. Add a code comment where the startup `--fork` path fails for Claude (chosen session id unsupported) — no behavior change.

Return (as your final message): files changed, one line each; anything unverified.
