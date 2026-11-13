Size one feature in /Volumes/Mini-SSD/offload/repos/grok-build (grok-fork, Rust TUI). Read-only on code, NO cargo. Report by 25 minutes after you start.

Feature (Sab): grok-fork's F3 side panel shows Claude usage the way dsh-tui does:

    ── 5h usage ──  ██████░░░░ 38% · resets 2h32m
    ── 7d usage ──  ███░░░░░░░ 21% · resets 130h42m
    ── Context ──   █░░░░░░░░░ 7% · 72k/1.0M

Reference: docs/SCOUT-dsh-tui-panel-2026-11-13.md (dsh-tui gets 5h/7d from the Claude CLI's `rate_limit_event` stream messages; context from per-turn usage).

Find, with file:line:
1. The F3 side panel code in crates/codegen/xai-grok-pager: what it shows now, and where a usage section would go.
2. The Claude agent bridge (how grok-fork runs Claude over ACP; find the adapter / agent command). Does `rate_limit_event` (or 5h/7d utilization and reset times) reach grok-fork today, or is it dropped? Where would it have to be passed through? Is context % already available?
3. Every file/function a coder would change, roughly how many lines each, and the unknowns.

Then estimate how many tool calls and how many minutes a Haiku 5.5 coder needs to write it (no cargo), and the risky part that could blow that up. Be concrete.

Write docs/SIZE-f3-usage-panel.md with the above. Commit by path, push to main, trailers exactly:
Co-authored-by: claude-code haiku-5.5
Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>

Kill only your own PIDs; never pkill/killall. When done, print the SHA and the estimate in one line, and stop.
