# grok-traces CLI

Written 2026-10-28 by Sonnet 5.5 from `git log`, `gh pr view` and the code. Decisions are attributed in `## Who decided what`; a lead-agent decision is not a requirement from Sab.

## What it is

The `grok-traces` binary (`crates/codegen/xai-grok-traces/src/bin/grok-traces.rs`). Subcommands: `hook <source>`, `scan`, `list`, `events <id> --json`, `raw`, `live --json`, `follow`, `doctor`, `version`. It is the data source for `lookout` (TOOL-lookout in trace-deck) and for `/traces`. Installed `4eb85116` at `~/.grok/bin/grok-traces` on sab-mini and sab-air.

## How it works

- Harness event -> `grok-traces hook <source>` (stdin JSON, always exit 0, 250 ms busy timeout) -> a receipt row plus that one transcript file indexed (`hook::ingest`) into `$GROK_HOME/traces/traces.sqlite`.
- `live --json`: one fold of the index into `{"live_schema":1,"rows":[...]}`, then exit. No loop; the caller polls.
- `follow [<id>]`: opens the index (2 s busy timeout) and ticks every 500 ms. Default target is the newest active root session. `--json` prints one item per line, `v=1`, kinds `user text reasoning call result model agent_start agent_end agent_resume`. `--backlog N` sets rows replayed at start (default 10).
- `follow` with no live session prints `… waiting for an agent` (or `… waiting for session <id>`) to stderr and attaches when one appears; after the followed root ends it prints the end line and waits for the next agent. `--once` exits on the first end, and exits 1 at once with `no active session` when nothing is live. `--json` also emits `agent_end` for the root.

## What changed today

- #26 sessions with no prompt titled `(no prompt) · <cwd>`; PARSER_VERSION 8.
- #27 `intent` column on events, SCHEMA_VERSION 9.
- #28 Pr Link records, interrupt markers, image-only prompts marked noise; PARSER_VERSION 9.
- #34 `follow`. #35 `follow --json`, `--backlog`, `agent_resume`; `live --json` rows gain `model` and `started_at` (LIVE_SCHEMA stays 1).
- #36 `follow` waits for an agent instead of exiting, keeps following across ends; `--once` keeps the old exit. Merged as `4eb85116`.

## Basic workflow

```
cargo build --release -p xai-grok-traces --bin grok-traces
cp target/release/grok-traces ~/.grok/bin/grok-traces.new && mv ~/.grok/bin/grok-traces.new ~/.grok/bin/grok-traces
grok-traces doctor
grok-traces list --json --active --limit 20
grok-traces live --json
grok-traces follow --backlog 10
grok-traces follow <id> --json
```

Hooks are installed with `crates/codegen/xai-grok-traces/hooks/install.sh`. `doctor` exits 1 unless the index is healthy.

## Who decided what

**Sab said**
- 05:00 "review how far along we got in grok build's traces feature and then the other repo trace-deck ... spin up some Haiku and Sonnet agents ... knock out the rest"
- 12:56 "do one thing really well which is following along a live agent (not past session, *live sessions*) and keeping up with their tool calls, and decision making"
- 21:50 `follo` must not exit when no agent is live: "that is so not what I would design"
- 21:52 "every fast-paced proper observability tool does the shortest path between agent's hook to it being logged"

**Lead agent (Claude Fable 5.1) decided, not Sab**
- Follow text glyphs; `--backlog` default 10; `--json` `v=1` item kinds.
- Adding `model` and `started_at` to `live --json` without bumping LIVE_SCHEMA.
- A hook -> viewer wake over a local datagram socket (design only, not built).
- Judging tool calls needs full tool args on `follow` call items (stated in MAP.md; judgments themselves are specified in trace-deck DECISION-MODELS.md).
- Pre-today agent decision, no measurement, no input from Sab: the 500 ms follow tick.

## Open/unverified

- trace-deck's `follow <id> --json` child: after the explicit `<id>` ends, the CLI now attaches to whichever agent goes live next (`id` is cleared). Not checked against trace-deck's respawn logic.
- The 500 ms tick was never measured against the hook path. lookout's backend hop was measured at median 328 ms, p95 551 ms (CONFORMANCE.md S7, n=28).
- The datagram wake is not built; the tick is still a poll.
