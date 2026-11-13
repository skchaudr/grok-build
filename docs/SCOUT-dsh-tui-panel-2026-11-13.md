# SCOUT: dsh-tui claude backend side panel (2026-11-13)

Status: PARTIAL. Panel tabs 1-5, `h` help and `z` zoom were not captured. Source findings are complete.

## Incident

Keys sent on sab-air (tmux session `dshcap`, 120x40, `dsh-tui --backend claude`):

- Digits `1`, `2`, `3` landed in the chat input box (no panel had focus). Cleared with Escape. No Enter sent.
- `Tab`, `Right`, `Down` gave no visible focus change.
- `h` opened the Sessions picker. Escape backed out.
- `h` followed by `Enter` was sent into the chat as a prompt, against the brief. The reply was a short greeting from claude-sonnet-5-5 (about 28k cache write, 26 output tokens). Paraphrased: the assistant asked what the user wants to work on.
- Panel tabs need the panel focused. The keybinding that focuses it is `Ctrl+B`, which the brief did not allow. Captures stopped there. `tmux kill-session -t dshcap` ran; `hermes-proxies` and `main` were left alone.

## Captures

### Launchpad (first screen, before any key)

```
          ▀▀▀▀ ▀▀▀▀▀▀▀▀▀▀▄   (DSH logo block art, omitted)
                        ╭──────────────────────────────────────────────────────────────────────╮
                        │ ❯ Say something, or type / for commands…                             │
                        ╰──────────────────────────────────────────────────────────────────────╯
                          sonnet  ·  High
                       Sessions & workspaces    Settings    Kernel · Claude    Update available
  ● New version available: v0.20.0 (current v0.19.0) · type /update to align the TUI, dsh engine and launcher together
/Users/sab-mini                                                                                          dsh-tui v0.19.0
                                                                                              DSH · dsh-core v0.1.7-rc.2
                                                                                         ▸ Claude · claude-code v2.1.323
                                                                                              Codex · codex-cli v0.167.2
```

### Keys 1, 2, 3 (not panel tabs)

Each digit went into the input box: `❯ 1`, then `❯ 12`, then `❯ 123`. Escape cleared it.

### `h` (Sessions picker, not help)

```
 ▣ Sessions  This terminal hosts several sessions · switching does not stop them                          Claude Agent
 Workspaces 0 · History 59          │ Sessions in sab-mini  0 working · 0 live · 8 total
   ▢ History only · cass          6   / h
   ▢ History only · scratchpad    2    + New session
   ...  (session list, omitted)
 ←/→ switch pane · Enter enter ·     Type to filter · Enter enter · Esc clears
```

Footer after one Escape: `←/→ switch pane · Enter enter · Ctrl+R rename · Ctrl+D delete · Esc back`. Ctrl+D deletes, so it was not pressed again.

### Chat with side panel (after the accidental prompt)

Tab bar of the side panel, top right: `◀  ⌗  ?  ♥  Todo  ▸  ◆  ⓘ  ∿  ▶`. The panel body was blank in the capture. Footer shows `ctx 2.8% (28k/1.0m)` and `Ctrl+B focus panel`.

### Not captured

- Tabs 1 to 5: needs panel focus (`Ctrl+B`), outside the allowed keys.
- `h` help screen: `h` opened Sessions instead.
- `z` zoom: needs panel focus.

## Source

Install path: `/opt/homebrew/bin/dsh-tui` is a launcher that delegates to the profile copy `~/.dsh/profiles/dsh-tui` (package `@deepseek-harness-tui/dsh-tui` v0.19.0). The global package there is v0.11.2 and is only a shell. All paths below are under `~/.dsh/profiles/dsh-tui/node_modules/@deepseek-harness-tui/dsh-tui/lib/types/`. No repo was checked; this is the installed build.

### 5h and 7d usage %

Not fetched by dsh-tui. The Claude CLI stream sends `rate_limit_event`; dsh translates it.

- `backends/claude/translate/core.js:300-306`: `rate_limit_event` reads `message.rate_limit_info.unifiedWindows`, one window per entry, with `utilization` and `resetsAt`, returned as `{ type: 'rate-limit', info: { windows } }`.
- `backends/claude/translate/system.js:6-9`: window names map to labels: `five_hour` (5h), `seven_day` (7d), `seven_day_opus`, `seven_day_sonnet`.
- `screens/StatusLine.js:496-501` (`formatRateLimit`): renders `` `${label(window.name)} ${Math.round(window.utilization * 100)}%` `` joined with ` · `. Example: `5h 68% · 7d 87%`.

### Reset times

- `backends/claude/translate/system.js:114-115`: `const resetsAt = num(info?.resetsAt);` then `claudeText('claude-rate-limit-resets', …)`.
- `backends/claude/translate/core.js:304`: `resetsAt` attached to each window (line truncated in tool output).
- Warning/rejected notices at `system.js:116-119` use `claude-rate-limit-warning` (`"Approaching the {{window}} usage limit ({{percent}}%){{resets}}"`).

### Context %

- `screens/StatusLine.js:156-158`: `occupancy = channelContextOccupancy(channel)`; `usedTokens` and `contextWindow` from it.
- `screens/StatusMetrics.js:28-35`: returns `channel.contextOccupancy` if set, else `resolveContextOccupancy(undefined, channel.lastUsage, channel.contextWindow)`.
- `dsh-adapter/context-occupancy.js:162-178` (`resolveContextOccupancy`): order is projection, then backend usage, then sample. Sample formula: `usedTokens = sample.input + sample.cacheRead + sample.cacheWrite`.
- `backends/claude/translate/attempts.js:212-215`: `context.capacity` from the largest `modelUsage` `contextWindow`.

### What it calls

- Usage %, reset times: no HTTP call from dsh-tui in `backends/claude`. Values come from the Claude CLI's SDK stream (`rate_limit_event`). The endpoint behind it was not checked.
- Context: from SDK usage fields on each turn (`content.js:13-16`: `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`).

## Data source

Usage % and reset times: `rate_limit_event` from the Claude CLI (claude-code v2.1.323) stream, translated in `backends/claude/translate/core.js:300` and `system.js:113`.
