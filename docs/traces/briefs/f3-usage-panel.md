Build the F3 usage panel in /Volumes/Mini-SSD/offload/repos/grok-build (grok-fork, Rust TUI). Report by 60 minutes after you start.

Read first: docs/SIZE-f3-usage-panel.md (file:line map of everything below).

Sab wants the F3 Info page to show Claude usage in dsh-tui's header style:

    ── 5h usage ──────────────
    ██████░░░░░░░░░ 38% · resets 2h32m
    ── 7d usage ──────────────
    ███░░░░░░░░░░░░ 21% · resets 130h42m
    ── Fable (7d) ────────────
    ...
    ── Context ───────────────
    █░░░░░░░░░░░░░░ 7% · 72k/1.0M

Data, both sources:
1. At start: the statusline files `~/.cache/claude-status/*.json` (newest under 24h). F3 reads only `rate_limits.five_hour.used_percentage` today (views/side_panel.rs:353-374). Read every window in `rate_limits` with `used_percentage` and `resets_at`.
2. Live: the Claude bridge's `usage_update` carries `_meta["_claude/rateLimit"]` (one window per event: `rateLimitType`, `utilization` 0-1, `resetsAt` epoch). The pager ignores it at app/acp_handler/mod.rs:253-262. Read it, keep a per-window map on the agent view, and pass it into `Info`. Live values replace file values for the same window.
- Show EVERY window that arrives, not a fixed 5h/7d pair. Labels: five_hour → 5h usage, seven_day → 7d usage, seven_day_opus → Opus (7d), seven_day_sonnet → Sonnet (7d), seven_day_omelette → Fable (7d) (believed to be Fable's codename; add a one-line comment saying so), anything else → its raw name.
- Reset text: `2h32m`, `130h42m`; no reset known → omit ` · resets …`.
- Don't let the rate-limit `usage_update` overwrite context used with a stale value (SIZE doc unknown 4): check it and guard if needed.

Tests: add a unit test that feeds a `usage_update` with `_meta` and checks the map, and one for the file parse with 5h + 7d. Then ONE cargo run in the shared warm target/ (no other target dirs, no install-fork.sh):
    cargo test --release -p xai-grok-pager -- side_panel write_all_until_delivers_and_restores_flags insert_block_before_rejects_an_already_committed_anchor
The last two names are pre-existing failures we want checked on this main; report their result separately and don't fix them. If the Info-page snapshot changes, accept it (cargo-insta isn't installed: rerun with INSTA_FORCE_PASS=1, delete the `assertion_line:` line, rename .snap.new → .snap). At most 2 cargo runs total. If cargo waits on a lock, that's normal: wait, don't report it as a blocker.

Commit by path (no `git add -A`), push to main, trailers exactly:
Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Kill only your own PIDs; never pkill/killall; never tmux. When done, print: SHA, test counts, the two pre-existing tests' results, and stop.
