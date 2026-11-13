# SIZE: F3 side panel Claude usage (5h / 7d / context)

Read-only sizing. No code changed, no cargo run. Source: grok-fork working tree on main, plus the installed `claude-agent-acp` 0.76.0 adapter.

## Answers

### 1. F3 panel today

- Toggle: `TOGGLE_KEY = F3`, `views/side_panel.rs:21`.
- Two pages: `Page::Traces` (default) and `Page::Info`, `views/side_panel.rs:28-45`.
- The Info page is `info_lines()`, `views/side_panel.rs:423`. Sections: Session, Model, Context, Runtime.
- Context meter: `views/side_panel.rs:465-476`, fed by `Info.context` (`(used, total)`).
- 5h row: `views/side_panel.rs:477-479`, `meter("5h usage", pct, format!("{pct}%"))`. No 7d row, no reset times.
- Panel data gather: `app/agent_view/side_panel.rs:13-60` (`side_panel_info`), which fills `context` (line 51, from `context_state`) and `five_hour_pct` (line 52).
- Refresh: `views/side_panel.rs:196-204` (`poll`) re-reads `~/.cache/claude-status/*.json` once per second through `claude_five_hour_pct` (`views/side_panel.rs:353-374`). It takes the newest file under 24h that has `rate_limits.five_hour.used_percentage`. This is a file side channel written by an external statusline, not by ACP. It has no 7d and no reset time.

Where a usage section goes: inside `info_lines`, between Context and Runtime. Replace the single 5h row (477-479) with a Usage block. Mock layout in the brief (`── 5h usage ──  bar 38% · resets 2h32m`) does not match the current Info rows (label, bar, tail). Layout needs Sab's call: keep the existing row style, or add the brief's header style.

### 2. Claude bridge: does `rate_limit_event` reach grok-fork?

Yes, but it is not used. It is forwarded and then dropped.

- Command: `app/cli.rs:1435` `cmd: "claude-agent-acp"`. Spawned at `acp/spawn.rs:395-410`, stdio wrapped by `acp/claude_subagents.rs:666` (`spawn_stdio_bridge`). That bridge rewrites subagent notifications only; I did not verify that it drops other lines (unknown, see 3).
- Adapter, installed at `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js:4285-4296`:
  - `case "rate_limit_event"` sends `session/update` with `sessionUpdate: "usage_update"`, `used: lastAssistantTotalUsage`, `size: contextWindowSize`, and `_meta: {"_claude/rateLimit": message.rate_limit_info}`.
  - Gate: only when `lastAssistantTotalUsage !== null`. So nothing arrives until the session has had one assistant turn.
- Pager receives it: `app/acp_handler/mod.rs:253-262`. The branch reads `usage.used` and `usage.size` only. `usage.meta` (`_meta`, present on `UsageUpdate` in agent-client-protocol-schema 0.11.4) is never read. Grep for `_claude/` in the pager: zero hits outside tests.
- Context % is already live: `apply_context_used` (`app/agent_view/session.rs:1265`) from the same `usage_update`.
- SDK shape, `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts:4843-4858`: `rate_limit_info` has one `rateLimitType` (`five_hour` | `seven_day` | ...), one `utilization` (0-1 float), one `resetsAt` (epoch) per event. There is no `unifiedWindows` array. The scout doc (`docs/SCOUT-dsh-tui-panel-2026-11-13.md`) describes dsh's `unifiedWindows`, which comes from a different CLI build. Do not port that shape.
- Consequence: each event carries one window. 5h and 7d arrive in separate events, so the pager must keep a per-window map and merge them.

Where it has to be passed through: `acp_handler/mod.rs:253` (read `usage.meta["_claude/rateLimit"]`), store on `AgentView` (new field near `context_state`, `session.rs`), copy into `Info` in `app/agent_view/side_panel.rs:52`.

Context %: already available. The Info page shows it today when `context_state` is set (`side_panel.rs:465`).

### 3. Files, lines, unknowns

| File | Change | Lines (est.) |
|---|---|---|
| `app/acp_handler/mod.rs:253-262` | Read `_meta["_claude/rateLimit"]`, parse `rateLimitType`, `utilization`, `resetsAt`, store per window | 15-20 |
| `app/agent_view/session.rs` (near 1265) | New field `rate_limits: HashMap<window, (pct, resets_at)>` plus a setter | 10 |
| `views/side_panel.rs:47-61` (`Info`) | Replace `five_hour_pct: Option<u8>` with the window map, or add `seven_day` and `resets` | 5-10 |
| `views/side_panel.rs:353-374` | Keep as fallback, or delete if ACP path is trusted (decision) | -20 to 0 |
| `views/side_panel.rs:423-480` (`info_lines`) | Usage block: 5h and 7d meters, reset text formatted `2h32m` / `130h42m` | 25-35 |
| `app/agent_view/side_panel.rs:52` | Pass the map into `Info` | 3 |
| `app/agent_view/side_panel_tests.rs` | Update fixture (line 29 sets `five_hour_pct`); add test that feeds a `usage_update` with `_meta` | 20-30 |
| `app/agent_view/snapshots/...side_panel__tests__focused_info_page.snap` | Snapshot changes (`focused_info_page` test, line 123). No cargo means a hand-edited .snap, which is risky | 2-40 |
| `views/side_panel.rs` tests (527-552) | Keep the `claude_five_hour_pct` tests if the fallback stays | 0-10 |

Total: about 80-120 changed lines across 4-5 source files and one snapshot.

Unknowns:
1. Whether `claude-agent-acp` strips or keeps `_meta` on `usage_update` when it reaches the pager: the adapter sends it, the pager schema has the field, but no test in the repo covers this path. Needs a fixture from a real session.
2. Whether `acp/claude_subagents.rs` `spawn_stdio_bridge` passes non-subagent lines through untouched. Not read in full.
3. Session start shows nothing until one assistant turn completes (adapter gate). Decide whether the panel should show "—" or keep reading the file snapshot until then.
4. The rate-limit `usage_update` reuses `used = lastAssistantTotalUsage`. Line 253 applies it as context used. Could overwrite a newer context value after a compact. Needs a check.
5. Whether 7d is worth showing when the file fallback has no 7d at all.

### 4. Haiku 5.5 coder estimate (no cargo)

- Tool calls: about 35-45 total. Reads of 8-10 files at the ranges above (~12), edits across 5 files (~12), a hand-written snapshot update (~4-8 reads and edits, the expensive part), a re-read for consistency (~5), a commit and push (~3).
- Minutes: not measured. I have no run data for Haiku on this repo. Treat any number as a guess, and expect the top end if the snapshot is wrong.

Risky part that blows this up: the `.snap` file. Insta snapshots compare whole rendered rows. Without cargo the coder must reproduce exact column widths for the Info page at the test width. A single space off fails the test, and the fail only shows on the next cargo run. Mitigation: skip the snapshot update in this change and mark the test `#[ignore]` with a shortcut comment, or let a cargo run regenerate the snapshot with `INSTA_UPDATE`. Sab decides which. Second risk: unknown 1 above. If `_meta` is stripped, the whole ACP path is dead and the coder has to switch to a different carrier.

## Open questions for Sab

1. Layout: keep the existing `label bar tail` rows, or use the brief's `── 5h usage ──` header style?
2. Keep the `~/.cache/claude-status` file fallback, or ACP-only?
3. OK to skip the snapshot update in the first cut and fix it with a cargo run later?
