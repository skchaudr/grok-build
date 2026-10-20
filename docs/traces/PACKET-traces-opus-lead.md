# TASK PACKET: `/traces` in the Grok Build pager, landed on the Air

**You are:** the Opus 5.5 lead, in your own Herdr workspace on the Mac mini. You own this end to end: design, build, tests, pty proof, release build, install on the Mini, rsync to the Air, proof on the Air. Do not stop at a report. Do not hand the landing back.

**Outcome in one line:** `~/.grok/bin/grok-fork` on sab-air is a binary you built on the Mini, and `/traces` in it lists and opens the Air's own Claude, Codex, Pi, OMP and DeepSeek Harness sessions.

**Why this packet is blunt about landing:** three waves in October built things and never put a new Darwin binary on the Air. That was the point every time. The last two sections are the deliverable; the middle is how you earn them.

---

## Dispatch spec (Sab's seven answers)

| # | Item | Answer |
|---|---|---|
| 1 | Machine | **Mac mini** (`ssh sab-mini@sab-mini`, Darwin arm64). Build here. **Never build on the Air.** Verify on arrival: `hostname; uname -sm; cargo --version; ls ~/.grok/bin`. These were not re-verified from khoj while writing this packet (ssh from the VM to the Mini hung), so check them yourself before the first `cargo` call. |
| 2 | Interface | **Herdr, one new workspace on the Mini**, your own tab. Sab reads your pane after the fact with `herdr pane read`; the watcher uses `herdr agent wait`. Never close your pane. |
| 3 | Tool and model | Claude Code, `claude-opus-5-5`, autocompact is already set at 200k. Subagents: Claude Code Agent tool, `claude-sonnet-5-5`, **at most 3 concurrent**. |
| 4 | Time limit | **None.** The only limit is productivity: no hung agent, no circling. Rule: `STATUS.log` must change at least every 45 minutes of wall clock while you are working. A subagent that has not reported in 30 minutes gets interrupted and re-briefed, not waited on. |
| 5 | Compaction | Autocompact at 200k. Before any compaction you expect, write `docs/traces/HANDOFF.md` (what is done, what is next, the exact commands). If you are restarted, read `HANDOFF.md` and `STATUS.log` first. |
| 6 | Subagents | Up to 3 Sonnet 5.5. They read `docs/traces/MAP.md` first and never crawl the repo (see Step 0). Reads, greps, test triage, bounded edits, fixture writing. You keep judgment: design, what passed on screen, merge, install. |
| 7 | Output | Branch `traces` in worktree `.worktrees/traces` of `~/repos/grok-build` on the Mini. Commit and push often. Merge to fork `main`. Binary at `~/.grok/bin/grok-fork` on the Mini and on the Air. Proof files under `docs/traces/`. Sab reviews. |

Commit trailers, both, every commit:

```
Co-authored-by: claude-code <claude-code@anthropic.com>
Co-authored-by: DSH + Claude Opus 5.5
```

---

## Settled facts (do not re-investigate)

**The reference implementation** is the bb plugin "Traces" on the Air. Third-party, patleeman/bb-plugins, v0.1.1, TypeScript. Read it over ssh, read-only:

```
ssh sab-mini@sab-air
~/.bb/plugins/cache/git/github.com/patleeman/bb-plugins/05c933a272b7dd2077897a6a0fbb64a4e78dc1e3/packages/bb-plugin-traces/
  README.md          design rationale
  src/indexer.ts     the whole model: roots, discovery, normalizeRecord, sqlite schema, incremental parse
  app.tsx            the three views: ledger, trajectory, inspector
```

Its live index on the Air: `~/.bb/plugins/traces/data.db`, tables `trace_roots`, `trace_files`, `trace_sessions`, `trace_events`, `trace_meta`. Read with `sqlite3`. Do not write to it.

**Its event model** (port this, do not reinvent it):

```
type, kind{message,tool,step,turn,reasoning,telemetry,system},
role{user,assistant,tool,tool_result,reasoning,system}?,
title, summary(clipped), timestamp, duration_ms, input_tokens, output_tokens,
usage_is_total, turn, step, depth(0..2), model, cwd
```

Session id = `source + ":" + sha1(source + "\0" + file_path)[..24]`. Summary clipped to 1,024 bytes in the index, raw line re-read from the source file on demand. The index is disposable; source files are the truth.

**Roots** (paths on each Mac, same layout on Mini and Air):

| source | path | format |
|---|---|---|
| claude | `~/.claude/projects/**/*.jsonl` | jsonl |
| codex | `~/.codex/sessions`, `~/.codex/archived_sessions` | jsonl |
| pi | `~/.pi/agent/sessions` | jsonl |
| omp | `~/.omp/agent/sessions` | jsonl |
| dsh | `~/.dsh/sessions/<cwd-slug>/<uuid>/session.v2.jsonl.zstd` | zstd jsonl |

A DSH file starts `{"type":"session","version":2,"id":...,"cwd":...}` then `{"type":"<ns>/<event>","seq":n,"time":ms,"data":{...}}` lines.

**What grok-build already has** (all under `crates/codegen/`):

| crate | what it gives you |
|---|---|
| `xai-grok-foreign-sessions` | Claude, Codex, Cursor store discovery, metadata only, NFS-safe. Pager already uses it for the resume picker (`xai-grok-pager/src/app/foreign_sessions.rs`). Extend, do not duplicate. |
| `xai-grok-session-search` | FTS5 SQLite with "rebuildable cache, source is truth" layering. Copy its `fts` / `recovery` / `bootstrap` shape for the trace index. |
| `xai-sqlite-journal` | Journal-mode selection. Use it to open the index DB. |
| `xai-grok-session-events` | Grok's own `events.jsonl`. Add it as a sixth source so grok sessions appear in the ledger too. |
| `xai-grok-mermaid` | Mermaid to PNG in a sandboxed child via the pager's `mermaid_worker`. Phase 3 uses it. |
| `xai-grok-pager/src/slash/commands/*.rs` and `slash/registry.rs` | How a slash command is declared. `/traces` goes here. |
| `xai-grok-pager/src/app/modals.rs` | Existing modal pattern (the session picker). The ledger is a modal of the same kind. |
| `xai-grok-pager-pty-harness` | Scripted pty scenarios. Your on-screen proof is one of these. |

`zstd` and `rusqlite` are already in `Cargo.lock`. No new dependencies are needed. If you believe you need one, stop and ask Sab.

**Build and binary rules** (from memory, proven the hard way):

- The pager binary comes from `-p xai-grok-pager-bin`, not `-p xai-grok-pager`.
- Copy the binary out of the target dir before you run it, and `grep -a -c '<marker>'` on the copy to prove it is from your branch. Marker for this work: the literal string `traces-ledger-v1` placed in the `/traces` help text.
- `df -h /` before every build; stop under 8G free and report.
- Fork config has `auto_update = false` on the Macs. Confirm on each Mac before running the pager: `grep auto_update ~/.grok/config.toml`. If it is not `false`, stop and tell Sab; the pager will otherwise replace itself with stock mid-proof.

---

## Step 0: project map before any subagent runs (Sab's "session read cache")

Sab's words: up to 3 Sonnet subagents *if there is a session read cache strategy or project map*. So:

1. You, the lead, write `docs/traces/MAP.md` yourself in the first hour. One section per crate in the table above: purpose, the 5 to 10 files that matter, the public types and functions you will call, and the test that shows how it is used. Include the pager modal and slash registry entry points with `file:line`.
2. Every subagent brief starts with "Read `docs/traces/MAP.md`. Do not grep outside the files it names unless the brief says so."
3. Keep `MAP.md` current as you learn. It is the cache; subagents that ignore it are wasting Sab's money.
4. Optional, if present on the Mini: Sab mentioned graphify, Benzi, and "the MyGraph treatment" for the touched areas. Check `ls ~/repos/MyGraph`, `which benzi`, and `ls ~/.claude/skills/graphify` on the Mini. If one exists and takes under 20 minutes, run it on the crates in the table and link the output from `MAP.md`. If none exists, `MAP.md` alone is the required minimum. Do not install anything to get them.

---

## Design

New crate `crates/codegen/xai-grok-traces/`:

- `roots.rs`: default roots for the six sources above plus custom roots from config. Claude and Codex path resolution comes from `xai-grok-foreign-sessions`.
- `discover.rs`: bounded walk (50k files, 32 levels, ignore `.git node_modules target dist build .cache`), skipping unchanged files by `(size, mtime)` and a SHA-256 of completed files.
- `normalize.rs`: the 15-field `NormalizedEvent` ported from `indexer.ts`. Heuristic on type and role names, no per-harness special cases except the DSH `data` envelope and the Codex `payload` envelope.
- `index.rs`: SQLite via `xai-sqlite-journal`, the five tables, incremental append by byte offset for a growing file, full re-parse when the prefix hash changes. Status `active` if mtime < 5 minutes ago.
- `query.rs`: `list_sessions(filter)`, `session_facets(id)`, `list_events(id, filter)`, `raw_line(id, line)`.
- `zstd.rs`: streaming decode of `session.v2.jsonl.zstd`.

Pager:

- `/traces` slash command opens the **ledger modal**: source tabs (All, DeepSeek, Claude, Pi, OMP, Codex, Grok), search box, sort by updated, one compact row per session: source chip, title, status, event count, tool count, duration, cwd, tokens, time. Keyboard only: `j k` move, `/` search, `tab` cycle source, `enter` open, `esc` back.
- **Trajectory view**: the three-row timeline strip (INPUT, MODEL, TOOLS dots over time) above the event list. Rows: `#line`, role chip, summary, time. Nested tool rows indented by `depth`. `e` toggles error-only, `t` filters by tool name, `/` filters text.
- **Inspector**: `enter` on a row shows the raw JSON re-read from the source file, pretty printed, scrollable. `y` yanks it to the clipboard through the pager's existing copy path.
- **Act**: `r` on a session resumes it in its harness where the pager already knows how (Claude, Codex, Cursor via the foreign resume path; grok native). For sources with no resume path, `r` shows "no resume for <source>" and does nothing. No silent fallback to a different agent.
- **Phase 3**: `m` on a session renders a Mermaid sequence diagram (user, assistant, each tool as a participant, subagent fan-out as nested activations) through `xai-grok-mermaid`'s worker and shows it the way the pager already shows diagrams. `l` toggles live tail on an active session (re-read new lines on the index's watcher tick).

Privacy: the index lives under `~/.grok/traces/traces.sqlite`, never leaves the machine, is never uploaded. Test fixtures are **synthetic**. Never copy a real session file into the repo.

---

## Phases and acceptance (write the failing tests first)

**P1: crate and index.** Tests with synthetic fixtures, one tiny file per source:

- [ ] Each of the six sources discovers its fixture and produces the expected session row (title, counts, model, cwd, status).
- [ ] `normalize` on a Claude user line, a Codex `response_item` function call, a Pi tool result, an OMP message, a DSH `data`-enveloped event, and a grok `events.jsonl` entry yields the expected `kind`, `role`, `depth`, tokens.
- [ ] Appending a line to a fixture re-indexes only that line (assert indexed_lines and no duplicate event ids). Rewriting the prefix triggers a full re-parse.
- [ ] A corrupt DB is quarantined and rebuilt (copy the `recovery` test shape from session-search).
- [ ] A zstd fixture decodes and indexes identically to the same lines uncompressed.
- [ ] Walk guards: a root with more than the file cap reports the limit and keeps existing rows.

**P2: pager views.** Pty-harness scenarios:

- [ ] `/traces` opens the ledger listing fixture sessions from a temp HOME; source tab filters; search narrows.
- [ ] `enter` opens the trajectory; the timeline strip renders; `j` moves; `e` shows only error rows.
- [ ] Inspector shows raw JSON matching the fixture line byte for byte; `y` puts it on the clipboard path.
- [ ] `r` on a Claude session takes the existing foreign resume path (assert the same launch struct the picker builds). `r` on a DSH session shows the no-resume notice.
- [ ] `/traces --help` contains `traces-ledger-v1`.
- [ ] Existing pager and shell suites stay green, minus the known pre-existing failures already listed in `OBSERVATIONS.md` on main. Name them in your STATUS.log when you skip them.

**P3: diagram and live tail.** Only after P4 has landed once. Land P1+P2 first; P3 is a second landing.

- [ ] `m` renders a sequence diagram for a 3-tool fixture session; the PNG is non-empty and the render goes through the worker, not in process.
- [ ] Live tail: appending to an `active` fixture while the trajectory is open shows the new row within one watcher tick.

**P4: LANDING.** See the next section. This is the phase everyone forgot. It is not optional and it is not "after review".

---

## Landing (the whole point)

Do this after P2 passes, and again after P3. Write every command and its first line of output into `docs/traces/PROOF-<date>.md` as you go.

**On the Mini:**

1. `git checkout main && git pull --ff-only && git merge --no-ff traces && git push`. Record the merge hash.
2. `df -h /`. Then `cargo build --release -p xai-grok-pager-bin`. Record wall time and exit code.
3. Copy the binary out: `cp <target>/release/xai-grok-pager /tmp/grok-fork.new`. Prove it: `grep -a -c traces-ledger-v1 /tmp/grok-fork.new` must print 1 or more. `/tmp/grok-fork.new --version`.
4. Back up and install: `OLD=$(~/.grok/bin/grok-fork --version | awk '{print $3}' | tr -d '()')`; `cp ~/.grok/bin/grok-fork ~/.grok/bin/grok-fork.$OLD`; `cp /tmp/grok-fork.new ~/.grok/bin/grok-fork`; `chmod +x`; `~/.grok/bin/grok-fork --version`.
5. Restart the Mini's fork leader without touching its plist: `launchctl kickstart -k gui/$(id -u)/com.sab.grok-leader`. Then `launchctl list | grep grok` and confirm the PID's exe is `~/.grok/bin/grok-fork` (`ps -o comm= -p <pid>`). Never rewrite `~/Library/LaunchAgents/com.sab.grok-leader.plist`.
6. Proof on the Mini: in a Herdr pane, `~/.grok/bin/grok-fork` (or `~/.dsh/scripts/gb claude` if that is how Sab runs it), `/traces`, see real sessions from the Mini's own home, open one, inspector shows JSON. `herdr pane read` that pane into the proof file.

**To the Air (never build there, never delete there):**

7. `rsync -av --progress ~/.grok/bin/grok-fork sab-mini@sab-air:~/.grok/bin/grok-fork.new`
8. `ssh sab-mini@sab-air 'OLD=$(~/.grok/bin/grok-fork --version | awk "{print \$3}" | tr -d "()"); cp ~/.grok/bin/grok-fork ~/.grok/bin/grok-fork.$OLD && mv ~/.grok/bin/grok-fork.new ~/.grok/bin/grok-fork && chmod +x ~/.grok/bin/grok-fork && ~/.grok/bin/grok-fork --version && grep -a -c traces-ledger-v1 ~/.grok/bin/grok-fork'`
9. Air leader: `ssh sab-mini@sab-air 'launchctl list | grep -i grok'`. Kickstart only the label you see there that points at `grok-fork`. If there is no such label, do not create one; note it and move on. Air's `auto_update` must read `false` before step 10.
10. Proof on the Air: `ssh -t sab-mini@sab-air` into a Herdr pane there (`herdr --remote sab-mini@sab-air` from the Mini if you want it observable), run `~/.grok/bin/grok-fork`, `/traces`, and confirm the ledger shows the Air's own DSH, Claude and Pi sessions (the Air has about 949 DSH, 273 Claude and 989 Pi session files as of 2026-10-20). Open one DSH session. Capture the pane into the proof file.
11. Commit `docs/traces/PROOF-<date>.md` and `MAP.md`, push. Append `LANDED <merge-hash> mini+air` to `STATUS.log`.

**Done means:** step 11 is committed and pushed, both Macs answer `grok-fork --version` with your merge hash, and the Air proof shows a DSH session open in `/traces`.

---

## Rules

- Commit early, commit often, push. The VM and the Macs can go away at any moment.
- Never build, compile, `npm install`, or delete on the Air. If a step on the Air fails, stop and report; there is no fallback.
- Do not overwrite any launchd plist or register any new service on either Mac. Restart by `launchctl kickstart` of the existing label only.
- Never print or commit API keys or real session content. Fixtures are synthetic.
- No `git stash`, no deleting outside `.worktrees/traces` and your own temp files.
- If you kill a build or a subagent, interrupt the agent first, then the process, then confirm it did not relaunch.
- `STATUS.log` in the worktree: one line per state change (`mapping / P1 tests red / P1 green / P2 pty / fixing <what> / MERGED <hash> / BUILT / INSTALLED mini / RSYNCED air / LANDED`). This is how the watcher and Sab see you.
- Anything outside this packet: ask Sab. Anything inside it: do it, do not pause for confirmation.
