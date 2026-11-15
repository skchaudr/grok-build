# Scout: fresh Claude 5h/7d numbers under `grok claude` (2026-11-15)

## Answer (question 4)
1. The SDK already has a no-turn call: `query.usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` (control request `get_usage`) returns live 5h and 7d `utilization` + `resets_at`; no interactive Claude Code needed.
2. Smallest change: in the adapter (`claude-agent-acp` `dist/acp-agent.js`), call it once after session init and again at each turn end, send the result as a `usage_update` `_meta`; in the pager, read both windows from it (`acp_handler/mod.rs` `UsageUpdate` branch, `views/side_panel.rs`) and keep the statusline file only as fallback.
3. Unknown: the call is marked experimental (shape may change); the adapter lives outside this repo (installed under `~/.local/share/claude-agent-acp/0.76.0`), so how the patch survives adapter upgrades is undecided; whether a `usage_update` sent before any turn renders (the adapter currently supplies `used`/`size` from a turn).

## Q1. Where interactive Claude Code gets numbers at startup
- The native `claude` binary (SDK package `claude-agent-sdk-darwin-arm64/claude`, manifest version 2.1.257; found with `strings -a`) contains `GET /api/oauth/usage` (`fetchUtilization`, function `BO`, 5 s timeout, OAuth refresh). It backs `/usage`, extra-usage checks, and the `get_usage` response.
- It also reads response headers `anthropic-ratelimit-unified-5h-utilization`, `-5h-reset`, `-7d-reset` from model calls (same dump); that is the per-turn path that feeds `rate_limit_event`.
- Inferred: statusline `rate_limits` at startup come from the `/api/oauth/usage` fetch (no turn needed; matches Sab's observation and the 07c1c6ae file written at launch with no turn). The statusline command is a TUI feature (`statusLine` setting with `refreshInterval`); I found no evidence the SDK path runs it, and the Mini `grok claude` session wrote no file.

## Q2. Can an SDK session get them with no turn?
- Yes. `sdk.d.ts:3696` `SDKControlGetUsageRequest {subtype:'get_usage'}`; response type at `sdk.d.ts:3703+` has `rate_limits.five_hour/seven_day {utilization, resets_at}` and `rate_limits_available`. `sdk.mjs` exposes it as `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()`.
- Measured: a throwaway script (scratchpad `u.mjs`) opened `query()` on the Mini with an idle prompt stream (no prompt sent, no model turn, no interactive claude running) and called it: 1835 ms, `five_hour.utilization 12`, resets `2026-11-15T09:09:59Z`, `seven_day.utilization 51`, resets `2026-11-18T19:59:59Z`, `rate_limits_available true`. `cstat` on sab-air at the same moment showed 9% / 51%, so the statusline file was stale on 5h and the SDK call was the fresh one.
- The SDK emits `rate_limit_event` per change during a turn (`rate_limit_info`). The adapter drops it unless a turn has reported usage: `acp-agent.js:4285-4297` (`if (lastAssistantTotalUsage !== null)`), and forwards one window per event as `_meta["_claude/rateLimit"]`.
- The adapter never calls `get_usage`: no match for `get_usage` or `usage_EXPERIMENTAL` in `dist/acp-agent.js`.

## Q3. ccusage
- `ccusage blocks --json` on sab-air reads local transcript JSONL and returns token/cost blocks (e.g. block 2026-09-30, 52 entries, $1.34). `blocks --active` returned `{"blocks": []}` at test time. It has no plan percentage or reset time from the server (only `--token-limit` for a self-supplied cap). It does not match `cstat` and cannot supply 5h/7d percent.
- `cstat` (sab-air, same minute): 5h 9% resets 01:10, 7d 51% resets Nov 18 12:00. 7d agrees with `get_usage`; 5h was behind (12%).

## Not done / inferred
- Did not patch or run the adapter; the change in the answer is untested. That the pager renders a usage-only update is inferred from reading, not run.
