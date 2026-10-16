# Codemode pane fidelity

Host `khoj-38w`. Pager branch `r2/codemode-pane`.

Pins:

- Recorder `origin/r2/pi-recordings` `f0fafcdf719f0f1b6a7dfd9368ae6b2232ce51ca`. `facts.json`, `acp.jsonl`, and `pi-acp-patched/test/tools/fidelity.mjs` for scenarios 1–4 match the earlier `357bee4e42cc4440d6bf6e287aad320b2441099f` blobs.
- Adapter `origin/r2/codemode-trace` `8ca225c1c426cd07fe00396e76a8878cf0b6dcdf`. `acp-after.jsonl` for scenarios 1–4.
- Before captures: pinned `~/.grok/bin/grok-fork` (Oct 14).
- After captures: `target/release/xai-grok-pager`, the binary from `xai-grok-pager-bin` (`Cargo.toml` `[[bin]] name = "xai-grok-pager"`). Built with `cargo build -j 4 -p xai-grok-pager-bin --release`.

`fidelity.mjs` treats a fact as present when `fact.needle` is a substring of the capture. Multiline needles (`R2NESTED1\nR2NESTED2`, `alpha=first\nbeta=second\ngamma=third`, `MXBASH1:first\nMXBASH2`) fail on the raw pane because every row is prefixed with the scrollback rail (`│┃`). The scored file is the same pane text with that rail and trailing `█` stripped. Long bodies are taller than the 120x40 viewport, so the after haystack for scenarios 1–3 is several `PageDown` captures concatenated with the first viewport. Scenario 4 fits in the first viewport.

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

Score:

```bash
fid=$(git -C ~/repos/Pi-Coding-Agent show f0fafcdf719f0f1b6a7dfd9368ae6b2232ce51ca:pi-acp-patched/test/tools/fidelity.mjs)
# rail-stripped capture:
node "$fid" facts.json real-before-<scenario>.plain.txt
node "$fid" facts.json real-after-<scenario>.plain.txt
```

No new file under the pager session directory (`sessions_delta=none`, isolated `GROK_HOME` session file count 0). No `workflow_run_id` write. No `pi-durable`.

## Results

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
