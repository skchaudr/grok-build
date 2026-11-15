# Scout: cache-warm signal under `grok claude`

Read-only scout per `docs/briefs/scout-cache-warmth-2026-11-15.md`. No source edits, no cargo, no session started, no model turns.

Adapter paths below are in `~/.local/share/claude-agent-acp/0.76.0/dist/acp-agent.js` (abbreviated `acp-agent.js`). Pager paths are under `crates/codegen/xai-grok-pager/src`.

## A. Adapter: does the SDK message stream carry cache data, and is it forwarded?

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| acp-agent.js:3432-3437 | Each SDK `result` message's `usage.cache_read_input_tokens` and `cache_creation_input_tokens` are summed into `session.accumulatedUsage.cachedReadTokens` / `cachedWriteTokens`. Comment at 3428-3432 says the accumulator resets on turn activation, so the tally is per turn. | Nothing; already per turn. | 0 | none |
| acp-agent.js:3878-3882 | Stream `message_delta` usage (cumulative) keeps cache read/write fields in `lastAssistantUsage` (inferred: feeds the context count). | Nothing. | 0 | none |
| acp-agent.js:3889-3894 | `usage_update` is sent with `used` and `size` only. No cache counts, no TTL, no expiry. | Only if the pager needs cache on this channel: add `_meta` here. Not needed for the pager-only path. | ~5 (adapter, optional) | none |
| acp-agent.js:6530-6537 (`sessionUsage`), sent at 6547 | PromptResponse `usage` carries `cachedReadTokens` and `cachedWriteTokens` for every turn, already forwarded over ACP. | Nothing. | 0 | none |
| acp-agent.js:6563-6588 (`quotaTokenCount`), used at 6565 and 6568 | `_meta.quota.token_count` carries `cachedInputTokens` and `cachedWriteTokens` per turn, per model. | Nothing. | 0 | none |
| `rg` over `dist/` for `cache_control`, `ephemeral`, `cache_ttl`, `prompt_cache` | No match. The adapter sets no cache TTL and emits no `prompt_cache` object. | An adapter patch cannot read TTL from the SDK without new code. | — | where the SDK sets TTL (see C) |

**Adapter verdict for A:** per-turn cache read and write counts are forwarded today in `usage` (6547) and `_meta.quota` (6565). No warm flag, no TTL, no expiry is sent.

## B. Pager: does it receive or store per-turn cache counts and last-response time?

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| app/dispatch/prompt.rs:1832-1836 | `response.usage` is read. For Claude (`usage_is_tally`, set at 1711) it is filtered out before `reported_context`. Cache fields are never read. | Read `response.usage.cached_read_tokens` / `cached_write_tokens` before the Claude filter, then store them on the agent. | ~10-15 | none |
| `~/.cargo/registry/.../agent-client-protocol-schema-0.11.4/src/agent.rs:3130` | ACP `Usage` type has `cached_read_tokens: Option<u64>` and a write-side sibling (3145, 3159 builders). The pager's typed `PromptResponse.usage` therefore already holds these fields. | None. | 0 | serde wire name not checked; adapter writes camelCase `cachedReadTokens` (6534) |
| `rg` over pager `src` for `cached_read`, `cachedRead`, `cache_read`, `cache_creation` | No reads on the TUI path. Only the headless reducer (`headless/reducer/messages/wire.rs:70-92`, `:212`) and `headless/ext_protocol.rs:281-402` hold them. Those are a separate headless protocol, not the `grok claude` TUI path (inferred). | None needed for the TUI. | 0 | none |
| app/agent_view/mod.rs:1002 | `turn_started_at: Option<Instant>` exists. | Not usable as last-response time: it is the turn start. | 0 | none |
| app/agent_view/session.rs:697-699 | `mark_turn_finished` clears `turn_started_at`. No last-response timestamp is stored. | Add `last_response_at: Option<Instant>`, set when a turn finishes (turn_completion.rs:529 is the existing finish point). | ~5 | `last_active_at` also exists; not checked whether it is set on each response |

**Pager verdict for B:** the pager does not store cache counts or a last-response time. Both are cheap to add. The counts already reach the pager in the typed `PromptResponse.usage`.

## C. Derivation: can warm/cold and time-left be worked out, and which TTL applies?

- **Interactive TTL, confirmed:** Claude Code's own statusline payload carries `prompt_cache.ttl = "1h"` with `expires_at` and `warm`. Example: `~/.cache/claude-status/97f30924-7af6-43a7-bcc9-4eaa1642606b.json`, 05:57:58 (see D). This is the interactive session, not `grok claude`.
- **SDK TTL, unknown:** the adapter sets no TTL (A). The bundled Claude binary `~/.local/share/claude/versions/2.1.323` contains the strings `extended_cache_ttl`, `prompt_cache_ttl`, `prompt_cache_ttl_reason`, `cache_ttl`, `prompt_cache_messages_ttl`, `ephemeral_1h_input_tokens`, `extended-cache-ttl-`. I did not trace which value an SDK query uses. Unknown: 5 minutes or 1 hour.
- **Derivation:** warm until `last_response_at + TTL`. A turn with `cachedReadTokens > 0` proves a cache hit; `cachedWriteTokens > 0` proves a write. Time-left needs `last_response_at` (B) and a TTL. The TTL can be an assumed constant (1h from the interactive evidence), labelled as assumed in the UI.
- **Limit:** this is a derived estimate. A cache can also expire on the server side without a turn, and no turn means no fresh signal. The statusline-style `warm` flag cannot be observed for `grok claude`.

## D. Statusline files for `grok claude` sessions

`~/.cache/claude-status/` holds 78 JSON files. The statusline script writes one per session (`~/.claude/statusline-command.sh:6-11`). The six newest, by mtime:

| file | mtime | cwd | `prompt_cache` |
|---|---|---|---|
| e5231d71-6099-41f7-b152-09251aea1649.json | 03:32:26 | grok-build | warm, ttl 1h |
| 517260d0-6fea-493b-8617-2d274cb137ca.json | 05:04:21 | grok-build | cold, ttl 1h |
| 97f30924-7af6-43a7-bcc9-4eaa1642606b.json | 05:57:58 | grok-build | warm, ttl 1h |
| 61b2d38b-cfe5-4908-b663-b2be67de11c3.json | 05:58:32 | pi-durable-lab | null |
| 4a6b289d-ae66-4064-99bc-a008d235c42a.json | 05:58:35 | grok-build | warm, ttl 1h |
| d32d57eb-4df6-4bdd-a5ce-93264541c532.json | 05:58:36 | grok-build | warm, ttl 1h |

- The last file is this scout's own session (its id matches the scratchpad path), so it was written by this interactive Claude Code session.
- No file is identifiable as a `grok claude` session. Matching by cwd fails: the `grok claude` cwd is also grok-build (inferred). Matching by session id would need the adapter's session id, which I did not check.
- The adapter does not run the statusline script (per brief), so `grok claude` sessions should not write here at all. Not verified by starting one, as the brief says.

## Verdict

**Derivable in the pager alone**, with the TTL as an assumed input.

Facts that decide it:

1. Per-turn cache read and write counts already reach the pager in the typed `PromptResponse.usage` (adapter `acp-agent.js:6530-6537` → `:6547`). The pager reads and drops them for Claude at `app/dispatch/prompt.rs:1835`. No adapter change needed.
2. Nothing on the wire gives a TTL or a warm flag. Warm/time-left must be derived from a stored `last_response_at` (pager, ~5 lines) plus a TTL. The 1h TTL is confirmed only for interactive Claude Code. The SDK TTL is unknown, so the TTL should be a labelled assumption until traced.

Rough size: pager-only, about 25-35 lines across `prompt.rs`, `agent_view/mod.rs`, `agent_view/session.rs`, plus tests. Adapter patch: 0 lines required.
