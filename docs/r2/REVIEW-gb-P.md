# Adversarial review — Grok Build slice P (codemode pane)

**Verdict: accept with fixes** (scenarios 1–4 after-capture needle fidelity matches owner; **do not merge** until `merge_tool_meta` regression is fixed and `cargo test -p xai-grok-pager external_execute_terminal_output_deltas_accumulate` passes).

**Reviewer:** cursor-agent adversarial pass on `r2/codemode-pane` @ `7e6a6644` (host `khoj-38w`).  
**Pins checked:** R `f0fafcdf` (old needle `fidelity.mjs`), R `origin/r2/pi-recordings` `0e9b3c8` (new `fidelity.mjs` + structural facts), A `8ca225c` (`acp-after.jsonl` for scenarios 1–4).  
**Pager binary:** existing `target/release/xai-grok-pager` from this worktree (not rebuilt; owner build ~13:55 UTC).

---

## Owner verification (re-run)

Commands match `docs/r2/codemode-pane/FIDELITY.md` (rail-stripped `.plain.txt`, R `f0fafcdf` `fidelity.mjs`).

| Scenario | Before (pinned `grok-fork`) | After (`xai-grok-pager`) |
|---|---|---|
| codemode-nested-bash | 4/6 | 6/6 |
| codemode-jev | 2/7 | 7/7 |
| codemode-mixed | 4/9 | 9/9 |
| codemode-fail | 3/5 | 5/5 |

R **new** `fidelity.mjs` (`0e9b3c8`) on A’s `acp-after.jsonl` with `--strict`: old-score and new-score both full for scenarios 1–4 (e.g. nested-bash new-score **2/2** `parent_link` facts). On TUI `.plain.txt` captures, **new-score is always 0/0** (structural facts are not defined for plain text); pane proof still relies on needles + manual/adversarial reads.

---

## What the pane drew (harness `scripts/acp-replay.sh`, 120×40, `ACP_REPLAY_EXPAND=1`, after = `SELECT_DOWN=2`)

| Scenario | Stream | Pane (summary) | Needle fidelity (after) |
|---|---|---|---|
| codemode-nested-bash | A `acp-after.jsonl` | Expanded `◆ codemode` with full script + output; nested bash as `◆ Run ↳ <parent> <status> <id> <cmd> [stdout]`; final user line `R2NESTED1 R2NESTED2`; `agent_settled` row | 6/6 |
| codemode-jev | A `acp-after` | Script + classify output; header `Model calls: $0.0`; nested classify rows under parent | 7/7 |
| codemode-mixed | A `acp-after` | Two codemode scripts (store/load + bash/jev); nested rows; `agent_settled` | 9/9 |
| codemode-fail | A `acp-after` | `Script failed` / `Error: boom-after-bash` in codemode body; nested bash `completed` with `FAILOK1`; separate `Error: boom-after-bash` message row | 5/5 |
| codemode-cancel | R `acp.jsonl` only (no `acp-after` on A) | Orphan nested `printf CAN CEL1` (expanded) + sibling `sleep 30`; **no** codemode row, **no** `Script aborted` / `agent_settled` | Not scored (no after file) |
| plain-bash | R `acp.jsonl` | Single execute row `PLAINOK1`; no codemode nesting | N/A (control) |
| acp-subagents-parent-e2e | fixture | Same as owner: `PONG-B-STEEREDPONG-A` on one line; thought blocks present | N/A |

---

## Adversarial mutations (nested-bash `acp-after` unless noted)

| Mutation | Pane behavior | Lie? |
|---|---|---|
| Strip `_meta.pi.parentToolCallId` on **all** updates for `call_28131/1` | `/1` becomes top-level `◆ Run printf…`; `/2` stays nested | No — orphan when meta absent |
| Set parent to `call_EVIL` on `/1` | Header shows `↳ call_EVIL … call_28131/1` | Displays attacker meta faithfully (wire lie, not invented) |
| Change `Model calls:` to `$999` in jev `acp-after` | Header stays `Model calls: $0.0` | **Stale cost** — does not reflect late content change |
| Remove `Model calls:` line from stream | No `Model calls:` in capture | No |
| Set nested `/1` status `failed` | `↳ call_28131 failed call_28131/1 …` | No — not shown as success |
| Duplicate `tool_call` for `/1` | No second row; id appears twice in text | No extra row (dedup by id) |
| Unicode bidi in script text | Script still renders; bidi may not appear in plain capture | Not evaluated as injection |

Partial drop of `parentToolCallId` on only the initial `tool_call` (updates still carry parent) **still nests** — pane follows last merged meta, not first event alone.

---

## Issues (named)

| ID | What | Where | Blocks? |
|---|---|---|---|
| P-1 | **`merge_tool_meta` breaks pi-acp terminal delta accumulation** — `external_execute_terminal_output_deltas_accumulate` fails (expects `1\n2\n3\n`, got `3\n`). Base `541561c3` passes; branch fails. Shallow top-level merge lets later `_meta.terminal_output` replace accumulated data. | `tracker.rs` `merge_tool_meta` / `merge_tool_call_update` | **Yes** (regression test) |
| P-2 | **TUI fidelity cannot score R’s new structural facts** (`parent_link`, `call_cost`). `FIDELITY.md` only documents old needle scoring on `.plain.txt`. | `docs/r2/codemode-pane/FIDELITY.md` | **Yes** for full R `0e9b3c8` contract on pane evidence |
| P-3 | **`Model calls:` summary not updated** when `tool_call_update` changes the line (jev `$999` mutation). Header stays at first value. | `tracker.rs` `model_calls_line` only on full block build; update path may not refresh summary on in-flight Other rows | **Yes** for cost tampering on stream |
| P-4 | **`Model calls: $0.0` shown** when adapter puts that in content (jev). Spec asked zero vs missing — pane does not interpret; it echoes the line. | After jev capture | No (faithful to adapter; A/R semantics) |
| P-5 | **codemode-cancel not proven after adapter** — no `acp-after.jsonl`; unpatched `acp.jsonl` does not paint codemode failure/`Script aborted`/`agent_settled`. | Scenario 5 gap (A not P) | No for slice 1–4; yes for full spec scenario list |
| P-6 | **Fail/cancel semantics on unpatched cancel stream** — nested bash without parent meta renders as top-level runs; cancelled `sleep 30` row remains a sibling without cancelled styling in capture. | `review-codemode-cancel.txt` | No for after-path; documents before-pipe behavior |

---

## Other checks

- **Scope:** `git diff --stat 541561c3` — only `xai-grok-pager` `tracker.rs`, `scripts/`, `docs/r2/`, `WORKLOG.md` (matches brief).
- **No store:** replays report `sessions_delta=none`, `isolated_grok_home_session_files=0`.
- **Secrets:** no API keys in slice-added paths (`scripts/`, `docs/r2/codemode-pane`, `tracker.rs` diff). Pre-existing repo canary strings unchanged.
- **Branch:** `r2/codemode-pane` @ `7e6a6644` on `origin/r2/codemode-pane`; HEAD commit includes `Co-authored-by: cursor-agent <cursor-agent@cursor.com>`.
- **Tests:** `cargo test -p xai-grok-pager acp::tracker --lib -j 4` → **198 passed, 1 failed** (`external_execute_terminal_output_deltas_accumulate`).

---

## Required fixes before accept

1. Fix `merge_tool_meta` (deep-merge `terminal_output.data` or preserve accumulated terminal meta across updates) and re-run the failing test.
2. Refresh `Model calls:` on `tool_call_update` when content text changes (codemode parent).
3. Extend verification doc (or harness) to state that structural fidelity is proven on `acp-after.jsonl` with R `0e9b3c8` `--strict`, and what pane captures prove beyond needles.

---

## Review artifacts

Adversarial replays wrote untracked files under `docs/r2/codemode-pane/review-*` and `mut-*` (not committed). Evidence for this review is in those paths on the reviewer host.
