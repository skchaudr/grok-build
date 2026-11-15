# Scout: OpenCode and Codex usage sources on sab-air (2026-11-15)

Brief: `docs/briefs/scout-opencode-codex-usage-2026-11-15.md`. Read-only on sab-air over ssh. Commands run: `opencode --version`, `opencode stats`, `opencode --help`, `sqlite3 -readonly` on `opencode.db` (schema, counts, JSON key names, aggregates), top-level key names of the config and auth JSON files, match counts in the OpenCode logs, and a read-only Python parse of ten Codex rollouts. No writes, no model turns, no `opencode run`, no `codex` sessions. No message text, prompts or credential values were read or copied.

## A. OpenCode (1.19.4, `/opt/homebrew/bin/opencode`)

| source | fields | example values | how fresh | unknowns |
|---|---|---|---|---|
| `opencode stats` (CLI) | Sessions, Messages, Days, Input, Output, Cache Read, Cache Write, Total Cost, Avg Cost/Day, tokens per session, tool counts | Sessions 36, Messages 700, Days 129, Input 6.6M, Output 285.3K, Cache Read 36.3M, Total Cost $0.00 | Computed from the DB at run time. Newest message row 2026-10-20 12:25 UTC, newest session update 2026-10-28 07:24 UTC (inferred: no later activity recorded) | Box-drawing text, not JSON |
| `~/.local/share/opencode/opencode.db`, table `message` (`data` JSON) | keys: role, providerID, modelID, cost, tokens.{input, output, reasoning, cache, total}, time.{created, completed}, finish, error, mode, agent, summary, path, parentID | 583 assistant rows, 117 user rows. Assistant providers: zai-coding-plan glm-5.2 (212), zai-coding-plan glm-5-turbo (289), opencode deepseek-v4.1-flash-free (81), opencode step-5-preview-free (1). `cost` is 0.0 on every assistant row | Newest row 2026-10-20 12:25 UTC | Whether the zero cost is a subscription effect is inferred, not confirmed |
| `opencode.db`, table `part` | id, message_id, session_id, time_created, time_updated, data | 3045 rows | Newest 2026-10-20 12:25 UTC | Not read beyond counts and timestamps |
| `opencode.db`, table `session` | id, project_id, workspace_id, parent_id, slug, directory, path, title, version, share_url, summary_additions, summary_de… (output cut off) | 36 rows | Newest time_updated 2026-10-28 07:24 UTC | Columns after summary_de… not read |
| `opencode.db`, table `event` | id, aggregate_id, seq, type, data | Types: message.part.updated.1 (981), message.updated.1 (351), session.updated.1 (108), session.created.1 (18); latest event_sequence seq 1428 | As above | No rate or window event types |
| `opencode.db`, other tables | account 0 rows, credential 0, session_message 0, permission 0, todo 28. control_account not counted | Counts only | Not applicable | Not read (credentials) |
| `~/.config/opencode/opencode.json`, `config.json`, `oh-my-opencode-slim.json`, `cli.json` (top-level key names only) | opencode.json: $schema, lsp, model, plugin, provider. config.json: $schema, mcp, provider. oh-my-opencode-slim.json: $schema, preset, presets. cli.json: plugins | No usage, limit or window key in any file | File mtimes Jul to Oct 2026 | Values not read; `provider` contents not inspected |
| `~/.local/share/opencode/auth.json` (provider ids only) | Provider ids: openai, openrouter, zai-coding-plan | No usage field named by the ids | File mtime Jun 21 2026 | Key and token values not read |
| `~/.local/share/opencode/log/` (opencode.log plus two oh-my-opencode-slim logs dated Nov 14 and Nov 15) | Match counts for plan-window terms: "usage" 2 lines, "rate-limiter" 2 lines, "percent" 0, "weekly" 0, "five.hour" 0, "window" 0 | No percentage or window value found | Logs dated Nov 14 and 15 | The two "usage" and two "rate-limiter" matches were not interpreted (inferred: not plan-window figures) |

No `opencode-go` provider id appears in the DB, the config files or auth.json. Whether the OpenCode Go subscription is the `opencode` or `zai-coding-plan` provider is unknown.

## B. Codex rollouts (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, 15 files total)

Ten newest files by mtime. Five carry `rate_limits` records; five are one-line `session_meta` files.

| source | fields | example values | how fresh | unknowns |
|---|---|---|---|---|
| `2026/11/05/rollout-2026-11-05T13-32-42-…jsonl` (originator codex-tui, CLI 0.166.0; 588 lines, 67 rate_limits lines) | Record path `$.payload.rate_limits` on `type` event_msg, `payload.type` token_count | Last record 2026-11-11T07:40:35Z: primary used_percent 23.0, window_minutes 300, resets_at 1794395171 (2026-11-11 11:06Z); secondary used_percent 35.0, window_minutes 10080, resets_at 1794423546 (2026-11-11 18:59Z) | About 4 days old at 2026-11-15 | Freshness depends on the last turn in that session |
| `2026/11/09/rollout-2026-11-09T17-28-30-…jsonl` (Codex Desktop, CLI 0.167.0-alpha.3.2; 164 lines, 18 rate_limits lines) | Same as above | Last record 2026-11-10T01:38:41Z: primary 12.0 / 300, resets_at 1794277160 (2026-11-10 02:19Z); secondary 31.0 / 10080, resets_at 1794423546 | About 5 days old | None |
| `2026/10/28/rollout-2026-10-28T00-35-21-…jsonl` (Codex Desktop, CLI 0.164.0-alpha.11.1; 16 lines, 1 rate_limits line) | Same as above, with primary and secondary null | Record 2026-10-28T07:35:25Z: primary null, secondary null, credits only | About 18 days old | Why primary is null is unknown |
| `2026/08/12/rollout-2026-08-12T06-53-48-…jsonl` (Codex Desktop, CLI 0.147.0-alpha.6.5; 339 lines, 46 rate_limits lines) | Same as above | primary and secondary present; values not extracted | Not checked | Values not extracted |
| `2026/10/03/rollout-2026-10-03T17-04-25-…jsonl` (Codex Desktop, CLI 0.160.0; 1856 lines, 206 rate_limits lines) | Same as above | primary and secondary present; values not extracted | Not checked | Values not extracted |
| Five one-line files: `2026/11/13/…03-35-38`, `2026/11/10/…22-42-48`, `2026/11/10/…22-06-06`, `2026/11/10/…01-37-35`, `2026/11/09/…17-39-44` | Only a `session_meta` line (originator codex_chatgpt_ios_remote for 11-13, Codex Desktop for the rest) | No rate_limits record | Not applicable | The 11-13 file is the one the brief flagged; it has no turn events, so "none found" is explained |

Full `rate_limits` field names: limit_id (string), limit_name (null), primary {used_percent, window_minutes, resets_at}, secondary {used_percent, window_minutes, resets_at}, credits {has_credits, unlimited, balance}, individual_limit, spend_control_reached, plan_type (string), rate_limit_reached_type. Primary is the 5h window (window_minutes 300). Secondary is the weekly window (window_minutes 10080). resets_at is epoch seconds.

Which sessions write it (inferred from the ten files): sessions that ran at least one turn (they have token_count event lines) write `rate_limits`. One-line session_meta-only sessions do not. Originator alone does not decide it: codex-tui and Codex Desktop files both write records, and the one-line files include both originators.

## C. Cheapest local reader

**OpenCode.** No plan-window figure exists locally, so there is nothing to read for a percentage. For tokens, the reader is a read-only query against `opencode.db`, for example `SELECT sum(json_extract(data,'$.tokens.total')) FROM message WHERE json_extract(data,'$.role')='assistant'`. Not executed. Freshness is bounded by the newest row, 2026-10-20.

**Codex.** Scan rollouts newest-first and stop at the first file with a line containing `"rate_limits"`. Parse that line as JSON and read `payload.rate_limits.primary.used_percent` (5h), `payload.rate_limits.secondary.used_percent` (weekly) and `resets_at` (epoch seconds). Staleness is the age of the last turn in that file. No record is written without a turn, so there is no upper bound. The newest observed record is 2026-11-11T07:40:35Z, about 4 days before this scout. Not executed as a reader; the parse above was run by hand on the sampled files.

## Verdict

- **OpenCode:** has tokens and cost only. Cost is $0.00 on every row, so in effect tokens only. No 5h or weekly percentage found in the DB, config files, auth file or logs.
- **Codex:** has a 5h/weekly percentage, but only in sessions that ran at least one turn. Primary is 5h (300 min) and secondary is weekly (10080 min).

## Unknowns

- An OpenCode Go plan percentage may exist only server-side. Inferred, not checked.
- `opencode providers` and `opencode db` were not run. `providers` may print credentials.
- The Codex primary-null record in the 2026-10-28 file is unexplained.
- Codex values were not extracted for the 2026/08/12 and 2026/10/03 files.
- The `session` table's columns after summary_de… were not read.
- The two "usage" and two "rate-limiter" log lines were not interpreted.
