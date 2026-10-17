# Real Pi in the codemode pane

**Verdict:** a call cancelled in flight reads as cancelled, and an earlier finished Failed read stays failed. Host paths are redacted in recordings, including when the model splits `/home/<user>` across stream deltas.

Pi: `@earendil-works/pi-coding-agent` 1.3.0 at `~/wt/pi-acp-r2-recorder/.pi13/package/dist/bundle/cli.js` (`--version` prints `1.3.0`). Tarball sha256 `ae2785e870b81a8c8e1d44510a57b5ee0bfc9e974679ac2faaada4aabee3d3c0`. Adapter: stock `/usr/bin/pi-acp` via `PI_ACP_PI_COMMAND`. Model: `google-antigravity/gemini-3.8-flash` (`gemini-3.6-flash` returns 429).

## PNGs

| Path | What it shows |
| --- | --- |
| `fix80-done.png` | Live 80×24 after this fix. `codemode cancelled`, `Cancelled sleep 25`, `Read 1 file · 1 failed`. No `/home/`. |
| `fix80-stream.png` | Same run, sleep still in flight, read already failed. |
| `cancel-after-120-done.png` | 120×40 after the cancel-row fix. Same three labels. |
| `cancel-after-120-stream.png` | Sleep still `Run sleep 25` while the turn is live. |
| `cancel-before-120-stream.png` | Before the cancel-row fix: sleep stays `Run`. Extension lines are `~/.pi/agent/extensions/…`. |
| `cancel-before-120-done.png` | Before: `Run sleep 25` after "Turn cancelled". |
| `baseline-120-done.png` | Real Pi nested printf, reply `W3NESTED1 W3NESTED2`. |
| `baseline-120-hold-done.png` | Same, with the `sleep 6` row between the prints. |
| `known-screen.png` | Renderer check: color bars and the pager glyphs. |

## Replay vs live

Round 2 replayed a fake ACP peer. These panes are live Pi. The nested bash rows and the cancel row come from that session, not from `acp-replay.sh`.

## Open

Five pager tests still fail and this branch does not touch them: `every_registered_feature_reaches_the_operator`, `defaults_round_trip_through_registry`, the two `combine_queued_prompts` tests, `reset_overlay_dims_all_rows_except_target`. They are not the seven cancel-panel tests (those fail on r2 and pass here). They do not block.
