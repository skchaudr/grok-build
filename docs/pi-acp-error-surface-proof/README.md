# pi-acp error surface proof (khoj, 2026-10-19)

Verdict: PASSED. A failed Pi turn now shows its error text in the grok-fork pane
instead of a blank turn. One named non-blocking issue (banner wording, pager side).

## What was wrong

Stock `pi-acp@0.0.33` (`/usr/bin/pi-acp`) swallowed turn failures. Pi reports a
provider auth failure or an upstream refusal as an assistant message with
`stopReason: "error"` and `errorMessage` (seen on `message_end` / `turn_end`,
then `agent_settled`). pi-acp's `agent_settled` handler resolved the turn as
`"end_turn"`, `prompt()` mapped any `"error"` result to `"end_turn"`, and no
text was ever emitted. The pager was told the turn ended normally with nothing
to show, so the Pi pane stayed blank (see `../picker-khoj-2026-10-19-mercury-keyfail/pane-pi1-opened.txt`: zero error lines).

## The change (patched copy only; stock install untouched)

Repo `~/repos/Pi-Coding-Agent`, dir `pi-acp-patched/`, commit
`ec66431de1d91687645752a940c7f5d1d5e75daf`
("pi-acp-patched: surface Pi turn errors as text + stopReason error").
Shipped as `patch/pi-acp-0.0.33c-error-surface.diff` on top of the two
existing diffs; `./apply.sh --check` passes; `test/` 27 pass, 5 skip (pre-existing
recording-fixture skips), 0 fail, identical to the pre-change baseline.

`index.js`, four edits (~20 lines):

- `handlePiEvent`: new `case "message_end"`: assistant message with
  `stopReason === "error"` emits `agent_message_chunk` text
  `Pi error: <errorMessage>` and sets `this.turnError = "error"`.
- `agent_settled`: resolves `this.turnError ?? "end_turn"` instead of always `"end_turn"`.
- `startTurn`: resets `turnError`; if the RPC `prompt` request itself rejects
  (non-auth), emits `Pi error: <message>` before resolving `"error"`.
- `prompt()`: returns `stopReason: "error"` unchanged (`"cancelled"` only when
  cancel was requested) instead of mapping it to `"end_turn"`.

Pi has no `refusal` stop reason: an upstream refusal arrives as
`stopReason: "error"` with the refusal text in `errorMessage`, so it is surfaced
the same way.

grok-build: `scripts/picker-live.sh` gained `PICKER_PI_ACP` (pi-acp entry point
on the target; default unchanged, `command -v pi-acp` = `/usr/bin/pi-acp` on khoj).

## Proof A: direct ACP over stdio (real Pi, dead key)

`pi-deadkey.sh` execs `~/.local/bin/pi --model inception/mercury-2`; that
provider's key on khoj is a macOS keychain command, so auth fails every time.
`acp-drive.mjs` sends `initialize` -> `session/new` -> `session/prompt` and
saves the raw transcript.

Patched (`PI_ACP_PI_COMMAND=/tmp/pi-deadkey.sh ~/repos/Pi-Coding-Agent/pi-acp-patched/run`),
`direct-patched.transcript.txt`:

```
<<< {"jsonrpc":"2.0","method":"session/update","params":{...,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Pi error: API key auth failed for provider inception: Failed to resolve API key for provider \"inception\" from shell command: security find-generic-password -s 'mercury-sab' -w"}}}}
<<< {"jsonrpc":"2.0","id":3,"result":{"stopReason":"error"}}
```

Stock (`/usr/bin/pi-acp`, same wrapper), `direct-stock.transcript.txt`: no
`agent_message_chunk` at all, response `{"stopReason":"end_turn"}`.

## Proof B: through grok-build (picker-live.sh, expected FAIL)

```
cd /tmp && PICKER_PI_ACP=~/repos/Pi-Coding-Agent/pi-acp-patched/run \
  PICKER_PI_MODEL=inception/mercury-2 PICKER_OUT=<this dir> \
  scripts/picker-live.sh khoj local
```

`launch-cmd.txt` confirms the pager ran with
`--agent-choice pi=env PI_ACP_PI_COMMAND=/tmp/picker-live-pi-khoj.sh /home/sab-mini/repos/Pi-Coding-Agent/pi-acp-patched/run`.
The run FAILs as expected (`result.env`: `pi1.resumed=no`, `pi2.resumed=no`,
`all_ok=0`; Claude sessions pass). What changed is the Pi pane.

`pane-pi1-opened.txt` lines 4-7:

```
    Pi error: API key auth failed for provider inception: Failed to resolve API key for provider             11:39 AM
    "inception" from shell command: security find-generic-password -s 'mercury-sab' -w
 ┃  Turn failed in 0.1s: Request failed: failed to deserialize response. Try sending again.
```

`pane-pi2-opened.txt` line 4 and 7 are identical. `steps.log`:

```
11:39:39 BLOCKER pi1 turn failed: Pi error: API key auth failed for provider inception: Failed to resolve API key for provider
11:40:15 BLOCKER pi2 turn failed: Pi error: API key auth failed for provider inception: Failed to resolve API key for provider
```

## Named issue (does not block)

The failed-turn banner says `Request failed: failed to deserialize response.
Try sending again.` rather than a clean error banner. Cause: the pager's typed
`PromptResponse` is deserialized by `agent-client-protocol-schema 0.11.4`, whose
`StopReason` enum is `end_turn | max_tokens | max_turn_requests | refusal | cancelled`
(no `error`), so `"error"` is rejected by serde and surfaced through
`error_display.rs` as an untyped request failure. `turn_completion.rs` already
maps the string `"error"` to `TurnStopReason::Error`, but the typed response
never reaches it. The error text and a failed-turn banner are both visible, which
was the goal; making the banner say "error" is a pager-side change (lenient
`stopReason` parsing in the ACP response path), not a pi-acp one. Returning
`refusal` instead would deserialize, but ACP semantics for `refusal` drop the
prompt from the next turn, which is wrong for an auth failure.

## Files

- `acp-drive.mjs`, `pi-deadkey.sh`: the driver and the dead-key wrapper used.
- `direct-patched.transcript.txt`, `direct-stock.transcript.txt`: raw stdio transcripts.
- `pane-*.txt`, `steps.log`, `result.env`, `launch-cmd.txt`, `leader-route.txt`,
  `proc-tree-end.txt`: the picker run's own captures.
