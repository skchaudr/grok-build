`trace-deck` is the full viewer product. Its commit `942056d` restored the pre-deletion tree (`ffe5674`); trace-deck PRs #31–#33 and the live-only `lookout`/`fake-lookout` naming are archived history, not the current product direction.

# grok-traces CLI

Written 2026-10-28 by Sonnet 5.5 from `git log`, `gh pr view` and the code. Decisions are attributed in `## Who decided what`; a lead-agent decision is not a requirement from Sab.

## What it is

The `grok-traces` binary (`crates/codegen/xai-grok-traces/src/bin/grok-traces.rs`). Subcommands: `hook <source>`, `scan`, `list`, `events <id> --json`, `raw`, `live --json [--watch]`, `follow`, `search`, `doctor`, `version`. It is the data source for `trace-deck` (`~/repos/trace-deck`) and for `/traces`. Installed `97fc76fe` (2026-11-11) at `~/.grok/bin/grok-traces` on sab-mini and sab-air; `scripts/install-fork.sh` installs it with grok-fork every time. Since then a Codex code-mode `exec` call's `follow --json` `what` is its first shell command (`+N` for more), not the JS's first line.

## How it works

- Harness event -> `grok-traces hook <source>` (stdin JSON, always exit 0, 250 ms busy timeout) -> a receipt row plus that one transcript file indexed (`hook::ingest`) into `$GROK_HOME/traces/traces.sqlite`.
- `live --json [--recent N]`: one fold of the index into `{"live_schema":1,"rows":[...]}`, then exit. No loop; the caller polls. `--recent N` (default 0) appends the N most recent ended, non-ping root sessions after the live rows: same fields plus `ended_at` (ms), `state:"ended"`.
- `follow [<id>]`: opens the index (2 s busy timeout) and ticks on each wake signal (see `## Wake signal`), at least every 2 s. Default target is the newest active root session. `--json` prints one item per line, `v=1`, kinds `user text reasoning call result model agent_start agent_end agent_resume`. `--backlog N` sets rows replayed at start (default 10).
- `follow` with no live session prints `… waiting for an agent` (or `… waiting for session <id>`) to stderr and attaches when one appears; after the followed root ends it prints the end line and waits for the next agent. `--once` exits on the first end, and exits 1 at once with `no active session` when nothing is live. `--json` also emits `agent_end` for the root.
- With no id and nothing live, `follow` first prints the 10 most recent ended sessions (id, model, age since end, title) to stderr once. `follow <id>` attaches to an ended session too: tail, then `■ ended` / `agent_end`, and stays attached; if the session resumes it emits `agent_resume` (text: `↻ resumed`) and keeps going.

## What changed today

- #26 sessions with no prompt titled `(no prompt) · <cwd>`; PARSER_VERSION 8.
- #27 `intent` column on events, SCHEMA_VERSION 9.
- #28 Pr Link records, interrupt markers, image-only prompts marked noise; PARSER_VERSION 9.
- #34 `follow`. #35 `follow --json`, `--backlog`, `agent_resume`; `live --json` rows gain `model` and `started_at` (LIVE_SCHEMA stays 1).
- #36 `follow` waits for an agent instead of exiting, keeps following across ends; `--once` keeps the old exit. Merged as `4eb85116`.
- `live --json --recent N`, `follow` idle listing, `follow <id>` on ended sessions (resumes in place; an explicit id no longer hops to the next agent after its end). `live_schema` stays 1.

## Basic workflow

```
scripts/install-fork.sh
grok-traces doctor
grok-traces list --json --active --limit 20
grok-traces live --json
grok-traces follow --backlog 10
grok-traces follow <id> --json
```

Hooks are installed with `crates/codegen/xai-grok-traces/hooks/install.sh`. `doctor` exits 1 unless the index is healthy.

## search (cass over ssh)

`grok-traces search <query words...> [--json] [--limit N] [--mode lexical|semantic|hybrid] [--host <ssh target>]` asks the `cass` index on another machine and joins each hit to the matching local trace session. Code: `src/cass.rs`; tests: `tests/cass.rs`.

- Host: `--host`, else the first valid line of `~/.grok/traces/cass` (same rules as `hosts`: `#` comments, no leading `-`, no whitespace). Neither: exit 2, `no cass host: put an ssh target in ~/.grok/traces/cass`.
- Transport: `ssh -o BatchMode=yes -o ConnectTimeout=2 -o ServerAliveInterval=2 -o ServerAliveCountMax=2 <target> 'sh -s'`, with one line on stdin: `exec env CASS_AUTO_REFRESH=0 CASS_DAEMON_AUTO_SPAWN=0 cass search '<query>' --limit N --json [--mode M]`. The query is POSIX single-quoted and never on the ssh command line (the remote login shell is fish). Limit default 20, max 200; wall clock 15 s, child killed at the deadline.
- Join: a local session matches when its file path and cass `source_path` share at least the last two path components (a trailing `.zst` is ignored); the most shared wins, a tie is no match. The trace `line` is the event whose `timestamp` equals the hit's `created_at` exactly, else null. cass `line_number` is a message index and is never used as a trace line.
- Output: `--json` prints one object per hit, in cass order: `id line source session_title agent origin score created_at snippet cass_path` (null where there is no local match). Without it: `<id|-> TAB <line|-> TAB <origin> TAB <snippet, 120 chars>`. Stderr ends with `<n> hits, <m> on this machine, <total> total`.
- Errors print to stderr and exit 1: `<host> unreachable (...)` (ssh failure, timeout), `<host>: exit N: ...` (cass failed; 127 = cass not installed there), `<host>: bad response (...)`. If the local index cannot be opened the hits are printed unjoined.

## Wake signal

- After a hook writes its event into the index, it sends one empty datagram to every `$GROK_HOME/traces/wake*.sock` (`wake::notify`). It never blocks, never creates a socket, and ignores every error: no listener is the normal case.
- `follow` and `live --json --watch` use `wake::Waker`. The first binds `wake.sock`; a datagram socket has one receiver, so a viewer that finds it held by a live process binds `wake.<pid>.sock` instead. Dead socket files are removed when a send fails (ECONNREFUSED/ENOENT) or at bind.
- `Waker::wait(timeout)` returns true on a datagram (a burst counts once), false on timeout. If the socket cannot be bound it sleeps instead.
- Fallback timers: `follow` and its idle wait use 2 s; `live --json --watch` prints one envelope at start, then one per wake (at most one per 100 ms) and at least every 2 s. Each envelope honors `--recent N`.
- `grok-traces doctor` prints a `wake` line: the number of listeners reached, or `no listener`.
- Decided by the lead agent (Claude Fable 5.1), not Sab, from Sab's 21:52 remark above.

## Who decided what

**Sab said**
- 05:00 "review how far along we got in grok build's traces feature and then the other repo trace-deck ... spin up some Haiku and Sonnet agents ... knock out the rest"
- 12:56 "do one thing really well which is following along a live agent (not past session, *live sessions*) and keeping up with their tool calls, and decision making"
- 21:50 the trace-deck follow alias must not exit when no agent is live: "that is so not what I would design"
- 21:52 "every fast-paced proper observability tool does the shortest path between agent's hook to it being logged"

**Lead agent (Claude Fable 5.1) decided, not Sab**
- Follow text glyphs; `--backlog` default 10; `--json` `v=1` item kinds.
- One list where live is a row state, not a filter: ended sessions follow the live rows (`live --json --recent`), and `follow` lists them when idle (Sab asked why past sessions are not displayed or loaded as traces).
- Adding `model` and `started_at` to `live --json` without bumping LIVE_SCHEMA.
- A hook -> viewer wake over a local datagram socket (design by the lead agent, Claude Fable 5.1, not Sab; built as `wake.rs`). Not decided by Sab: the 2 s fallback timer, the 100 ms `live --watch` coalescing, the `wake.<pid>.sock` fan-out for a second viewer.
- Judging tool calls needs full tool args on `follow` call items (stated in MAP.md). Any decision-model design must be reviewed separately against restored trace-deck main; its current tree has no `DECISION-MODELS.md`.
- Pre-today agent decision, no measurement, no input from Sab: the 500 ms follow tick (replaced by the wake signal; 2 s fallback).

## Open/unverified

- trace-deck's `follow <id> --json` child: an explicit `<id>` now stays attached across its end (changed from #36, which hopped to the next agent). Not checked against trace-deck's respawn logic.
- The 500 ms tick was never measured against the hook path. The reverted live-only trace-deck PR #32 recorded a backend-hop measurement of median 328 ms, p95 551 ms (n=28); that measurement is not in restored trace-deck main.
- Wake latency was measured once locally (fixture session, one Waker); not measured through a real harness hook, and not on sab-air.
