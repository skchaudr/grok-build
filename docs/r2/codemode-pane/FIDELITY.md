# Codemode pane fidelity

Host `khoj-38w`. Pager branch `r2/codemode-pane`.

Pins:

- Recorder `origin/r2/pi-recordings` `0e9b3c87a130722fe1a56ce8cd77cc7d55e3f5db`. `facts.json` for scenarios 1–4. `fidelity.mjs` blob `57262ee5c1f9d073002ad712c3ef69733819e7a3` (`pi-acp-patched/test/tools/fidelity.mjs` at that commit). The earlier needle-only blob at `f0fafcdf719f0f1b6a7dfd9368ae6b2232ce51ca` is not the scorer for this pin.
- Adapter `origin/r2/codemode-trace` `8ca225c1c426cd07fe00396e76a8878cf0b6dcdf`. `acp-after.jsonl` for scenarios 1–4.
- Before captures: pinned `~/.grok/bin/grok-fork` (Oct 14). Not re-replayed after the tracker fix; re-scored with the blob above.
- After captures: `target/release/xai-grok-pager`, rebuilt `2026-10-16 14:31 UTC` with `cargo build -j 4 -p xai-grok-pager-bin --release` (`Cargo.toml` `[[bin]] name = "xai-grok-pager"`).

## Structural fidelity (JSONL, not the pane)

`parent_link` and `call_cost` are scored only on a JSON capture. `--strict` exits 1 when either score is short. Run on A's `acp-after.jsonl` against R's facts:

```bash
fid=57262ee5c1f9d073002ad712c3ef69733819e7a3
git -C ~/repos/Pi-Coding-Agent show "$fid" > /tmp/fidelity.mjs
# facts.json from 0e9b3c87, acp-after.jsonl from 8ca225c1, per scenario
node /tmp/fidelity.mjs --strict facts.json acp-after.jsonl
```

Output, 2026-10-16:

```
===== codemode-nested-bash =====
old-score: 6/6
missing:
new-score: 2/2
missing:
===== codemode-jev =====
old-score: 7/7
missing:
new-score: 3/3
missing:
===== codemode-mixed =====
old-score: 9/9
missing:
new-score: 3/3
missing:
===== codemode-fail =====
old-score: 5/5
missing:
new-score: 1/1
missing:
```

Each `--strict` run exited 0. nested-bash new-score is the two `parent_link` facts. jev new-score is three `call_cost` facts (`cost: 0`). mixed is two `parent_link` plus one `call_cost`. fail is one `parent_link`.

Plain-text pane captures do not carry `_meta.pi.parentToolCallId` or `rawOutput.details.calls`. The same scorer on a `.plain.txt` reports `new-score: 0/N` (those facts missing). That is the contract of this blob, not a short pane.

## What the pane proves beyond needles

Needle `old-score` is a substring check (script fragments, `R2NESTED1\nR2NESTED2`, `alpha=first\nbeta=second\ngamma=third`, `MXBASH1:first\nMXBASH2`, `models.classify`, `Script failed`, `Error: boom-after-bash`, `agent_settled`). It does not mention the parent id or the cost line.

The after panes also show:

- Nested rows as `↳ <parent> <status> <id> <command> <stdout>`. Read back from this rebuild: nested-bash `↳ call_28131 completed call_28131/1` and `/2` with `R2NESTED1` / `R2NESTED2` on the header; fail `↳ call_36707 completed call_36707/1` with `FAILOK1`; mixed `↳ call_917984 completed call_917984/1` with `MXBASH2`. That line is the visible `parent_link`. The fact id is not a needle.
- `Model calls: $0.0` on the codemode header for jev and mixed. That is the adapter's text for the cost, not the numeric `call_cost` field. A later `tool_call_update` that changes the line replaces the header (`model_calls_header_refreshes_on_tool_call_update`: `$0.0`, then an in-flight update to `$999`, then the completed text).
- The expanded script body and `agent_settled` (fail: `Script failed` plus a separate `Error: boom-after-bash` row).

## Pane needle scores

`fidelity.mjs` old-score treats a fact as present when `fact.needle` is a substring of the capture. Multiline needles fail on the raw pane because every row is prefixed with the scrollback rail (`│┃`). The scored file is the same pane text with that rail and trailing `█` stripped. Long bodies are taller than the 120x40 viewport, so the after haystack for scenarios 1–3 is several `PageDown` captures concatenated with the first viewport. Scenario 4 fits in the first viewport. Mixed also expands the following rows so the second script's output is on a later page; the scored file is the stripped first viewport plus those pages.

Replay:

```bash
# before, pinned pager (ACP_PAGER default ~/.grok/bin/grok-fork)
ACP_REPLAY_DELAY_MS=0 ACP_REPLAY_EXPAND=1 \
  scripts/acp-replay.sh <acp.jsonl> real-before-<scenario>

# after, rebuilt pager. SELECT_DOWN=2 lands on the codemode row
# (the session-title row is the first scrollback entry).
ACP_REPLAY_DELAY_MS=0 ACP_REPLAY_EXPAND=1 ACP_REPLAY_SELECT_DOWN=2 \
  ACP_PAGER=$PWD/target/release/xai-grok-pager \
  scripts/acp-replay.sh <acp-after.jsonl> real-after-<scenario>
```

Expand keys with vim mode off: Escape, Tab (focus scrollback), Down (select), Right (expand). A second Tab returns to the prompt. `g`/`l` type into the prompt.

Score (old-score is the needle fraction):

```bash
fid=57262ee5c1f9d073002ad712c3ef69733819e7a3
git -C ~/repos/Pi-Coding-Agent show "$fid" > /tmp/fidelity.mjs
node /tmp/fidelity.mjs facts.json real-before-<scenario>.plain.txt
node /tmp/fidelity.mjs facts.json real-after-<scenario>.plain.txt
```

No new file under the pager session directory (`sessions_delta=none`, isolated `GROK_HOME` session file count 0 on every after replay). No `workflow_run_id` write. No `pi-durable`.

## Results

Needle `old-score`, re-scored 2026-10-16 with blob `57262ee5`. Before is the existing pinned-pager plain. After is the replay from the 14:31 UTC binary.

| Scenario | Before (pinned pane) | After (rebuilt pane) |
|---|---|---|
| codemode-nested-bash | 4/6 | 6/6 |
| codemode-jev | 2/7 | 7/7 |
| codemode-mixed | 4/9 | 9/9 |
| codemode-fail | 3/5 | 5/5 |

Before is missing the script text (the unpatched ACP content is output only), parent linkage, and `agent_settled` (`_meta.pi.runState` on `session_info_update`). After shows the script inside the expanded codemode row, nested bash rows as `↳ <parent> <status> <id> <command> <stdout>`, `Model calls: $x` on the codemode header when the adapter sent that line, and `agent_settled` as its own row.

Pinned-binary probes (before the paint), in this directory:

- `probe-long-content.txt`: a long script is not truncated. `script-line-079`, `SCRIPT-TAIL-ZZZ`, and `OUTPUT-TAIL-YYY` are reachable by paging inside the expanded row. The row starts collapsed.
- `probe-update-replaces.txt`: `tool_call_update` replaces the body (`AFTER-TEXT-REPLACED`, `Model calls: $0.02`).
- `probe-meta-parent.txt`: a `tool_call` with `_meta.pi.parentToolCallId` is a sibling `Run echo nested` row. The pinned binary does not indent it.
