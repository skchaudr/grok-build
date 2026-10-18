# Picker live proof: Claude and Pi sessions switched from the Grok dashboard

## Verdict

**PASS on the Air. PASS on khoj (Linux), once. Mac mini: NOT PROVEN, and the rerun against the durable leader still failed, with two different errors.**

- **Mac rerun (02:45 to 02:53 UTC), `PICKER_START_LEADER=0`, against S1's launchd leader (pid 50705, `~/.grok/leader-fork.sock`, gui domain):** all four sessions failed. Evidence `/tmp/picker-proof-out/mini/`, log `/tmp/picker-mini2.log`.
  - **Claude (claude1 and claude2):** the session was created (`sdk-initialize` and `register` in `/tmp/grok-leader.err`), then the first turn failed after 29s with `Failed to authenticate: OAuth session expired and could not be refreshed` (`errorKind: authentication_failed`). This is a different message from the ssh-leader run, which failed in 0.1s with `Not logged in`. So the gui-domain leader does get as far as trying to refresh the Claude token, and the refresh fails. I cannot tell from here whether the Claude credential is genuinely expired or the refresh is blocked: `security show-keychain-info` on the login keychain over ssh says "User interaction is not allowed", and `claude auth status` over ssh says `loggedIn: false`, but ssh is the Background domain, so neither is conclusive for the gui domain. The earlier "Claude OAuth expired" suspicion is therefore **not ruled out**, even though the first Mac run's "Not logged in" was an artifact of the ssh-started leader.
  - **Pi (pi1 and pi2):** `Session creation timed out after 180s`. `pi-acp` (pid 51191) and `pi` children were running, but no `register` line for a Pi session appears in `/tmp/grok-leader.err`. Pi replied in about 8s on the ssh-started leader, so what differs is the leader's environment or domain. Not diagnosed. One candidate: the Pi pin, `env PI_ACP_PI_COMMAND=/tmp/picker-live-pi-mini.sh pi-acp`, is the agent command the pager sends and the process listing shows `pi` children whose arguments I could not confirm were the wrapper. I did not verify that the pin reaches `pi-acp` under the launchd leader.
- **First Mac run (02:29 UTC) retracted, as before:** the "Not logged in" came from a leader my script started over ssh (Background launchd domain, no keychain). The "ask Sab to `/login`" call was premature.
- **Cause of the 02:29:33 shutdown (S1, 02:45 UTC):** the "socket removed and no client attached" line came from the **stock 1.0.62 relay leader** on `leader.sock`, not the fork. The fork leader exited 0 via the client-requested relaunch path at `crates/codegen/xai-grok-shell/src/leader/server.rs:1982-2009`, which fires when a **newer** client attaches (stock 1.0.62 pager against fork 1.0.45). My earlier reading that my script's `rm -f` caused it is superseded.
- **Script fixes (committed):** `PICKER_START_LEADER` defaults to 0 (never starts or removes a leader); `stop_leader` no longer runs `pkill -f "grok-fork agent leader"`, which also matched the launchd leader. It closes only its own tmux session.
- **Air:** passed the full sequence with two Claude and two Pi sessions, each re-entered from the dashboard and answering a fresh nonce-tagged question with its token.
- **khoj (optional):** passed once (`khojlocal`); a rerun failed on `claude1` with "The agent did not accept your prompt within 120s", cause undiagnosed.
- The bonus (`khoj=ssh ...` entry) was **not run**.

## How the proof works

`scripts/picker-live.sh <label> <ssh-target|local>` drives one Grok pager in a scripted tmux PTY (own socket `tmux -L picker`, 120x40, `env -u CLAUDECODE`). For the Macs it runs the pager over `ssh -tt`.

1. Open Claude #1 from the home screen (`ALPHA-1`), then use the dashboard (`Ctrl+\`, `+ New Agent`, "New session / Machine" picker) for Pi #1 (`BRAVO-1`), Claude #2 (`ALPHA-2`) and Pi #2 (`BRAVO-2`).
2. Re-enter claude1, pi2, pi1, claude2, claude1 from the dashboard.
3. After each re-entry, ask a nonce-tagged question ("what token did I give you?"). It counts as resumed only if a reply line **after the nonce line** carries the token. Re-rendered scrollback does not count.
4. Evidence per machine: pane captures, status-line model, session ids, the leader's route log, and `ps` listings of `claude-agent-acp` and `pi-acp` while sessions are live.

No stubs, fakes or replay judges are involved. Every reply came from a live agent.

## Topology and why

Each Mac is self-sufficient (Sab's FINAL topology): its own pager, its own fork leader, its own local agents. Home base is the machine in Sab's hands, and khoj is an ephemeral VM. The embedded pager hosts only the first `--agent-choice`; multi-backend routing lives in the leader, so the pager runs with `--leader --leader-socket <sock> --agent-choice claude=... --agent-choice pi=...` and `GROK_AGENT_DASHBOARD=1`. The Air used the Darwin binary rsynced from the Mac mini. Nothing was built or deleted on the Air. The thin-client `ssh -L` idea was proposed and then cancelled by the lead; it was not used.

Pi was pinned to `deepseek/deepseek-flash` through `PICKER_PI_MODEL`, because Pi's default model returned HTTP 429 (quota). The pin is a model choice only; the agent is still the real `pi-acp`.

## Air (sab-air): PASS

Pager: `/Users/sab-mini/.grok/bin/grok-fork` against the Air's own leader `~/.grok/leader-fork.sock`. Evidence: `/tmp/picker-proof-out/air/`.

| Session | Token | Re-entered and answered with token | Status-line model | Session id | Evidence |
|---|---|---|---|---|---|
| claude1 | ALPHA-1 | yes | Fable 5.1 | not captured | `air/pane-claude1-opened.txt`, `air/pane-claude1-resumed.txt`, `air/scrollback-claude1.txt` |
| pi1 | BRAVO-1 | yes | deepseek/DeepSeek V4.1 Flash | 01a14ccc-1080-770c-ae7d-9764ab417284 | `air/pane-pi1-*.txt`, `air/scrollback-pi1.txt` |
| claude2 | ALPHA-2 | yes | Fable 5.1 | b507661a-7036-4931-b0e8-cc4a0ccb505b | `air/pane-claude2-*.txt`, `air/scrollback-claude2.txt` |
| pi2 | BRAVO-2 | yes | deepseek/DeepSeek V4.1 Flash | 01a14ccc-86a0-739e-a56e-07ab976f5383 | `air/pane-pi2-*.txt`, `air/scrollback-pi2.txt` |

- Leader log (`air/leader-route.txt`): each session shows `phase=sdk-initialize` then `phase=register`, so every session was created on the Air's own leader. The file also includes sessions from earlier runs.
- Processes while live (`air/pgrep-claude-live.txt`, `air/pgrep-pi-live.txt`): `node .../claude-agent-acp/current/dist/index.js` (pid 75964) and two `node /opt/homebrew/bin/pi-acp` (pids 76112, 77157), all local to the Air. The Claude backend is one process serving both Claude sessions. The listing also shows an unrelated Zed `claude-acp` process (pid 4030), which is not ours.
- The leader was started by S1 as the named launchd job `com.sab.grok-leader`. I started no foreground leader on the Air.

## Mac mini: NOT PROVEN (two runs, two different failures)

Pager `/Users/sab-mini/.grok/bin/grok-fork`. Evidence: `/tmp/picker-proof-out/mini/`.

| Session | Token | Re-entered and answered with token | Status-line model | Session id | Evidence |
|---|---|---|---|---|---|
| claude1 | ALPHA-1 | no (failed on my ssh-started leader, which cannot read the keychain) | Default (recommended) | none | `mini/pane-claude1-opened.txt` |
| pi1 | BRAVO-1 | not run (opened and answered, re-entry not reached) | deepseek/DeepSeek V4.1 Flash | 01a14cd8-3142-7348-bffe-ebbdd9045170 | `mini/pane-pi1-opened.txt` |
| claude2 | ALPHA-2 | no (same cause) | none | none | `mini/pane-claude2-opened.txt` |
| pi2 | BRAVO-2 | not run (opened and answered, re-entry not reached) | deepseek/DeepSeek V4.1 Flash | 01a14cd8-a2bd-771f-973d-44dc3e920b41 | `mini/pane-pi2-opened.txt` |

`mini/pgrep-claude-live.txt`, `mini/pgrep-pi-live.txt` and `mini/leader-route.txt` hold the process and route evidence.

Leader: the Mac's `com.sab.grok-leader` job was not running (`launchctl list` showed `-`), so the script started a **foreground** leader on `~/.grok/leader.sock`. After the run I stopped that leader and the leftover pager by pid (both confirmed gone). The stale socket file `~/.grok/leader.sock` remains on disk; I did not delete it. The Mac's stock leader `com.sab.grok-hub` (`leader-hub.sock`) was untouched.

## khoj (Linux, optional third target): PASS once

Evidence: `/tmp/picker-proof-out/khojlocal/` (local pager, systemd `grok-leader`). All four sessions re-entered and answered with their tokens (Claude status line `Opus 5`, Pi `deepseek/DeepSeek V4.1 Flash`). Caveat: the final repeat re-entry of claude1 in that run was a **false positive** (the nonce was reused, so an old reply matched). The driver now requires a fresh nonce per ask. A later rerun (`/tmp/picker-proof-out/khoj/`) failed on claude1 with a 120s prompt timeout, cause undiagnosed. Treat khoj as "passed once, flaky since".

## Issues

| # | Issue | Location | Blocks? |
|---|---|---|---|
| 1 | Mac, gui-domain leader: Claude turn fails `OAuth session expired and could not be refreshed`; Pi session creation times out at 180s | environment / unknown; leader log `/tmp/grok-leader.err` | **Blocks the Mac mini proof.** Cause open (see Verdict) |
| 1b | The first Mac run's "Not logged in" was an artifact of my ssh-started leader | `scripts/picker-live.sh:240-253` | Fixed: leader start is opt-in |
| 2 | Embedded pager hosts only the first `--agent-choice`; picker needs a leader | `app/cli.rs:898`, `app/mod.rs:572`; multi-backend routing at `leader/server.rs:3394` (`AGENT_CMD_META`), `:3412` (`resolve_backend`) | No. Works as designed with `--leader`; a pager without a fork leader shows no machine picker |
| 3 | `synthetic_session_load` fires only if the agent advertises `loadSession`, so `session/load` may never fire for external agents (`docs/acp-matrix.md` suspicion) | `leader/server.rs:936-1000` | No for this proof: re-entry from the dashboard attached the live session and gave fresh correct replies. I did not observe whether `session/load` was sent; the question stays open for a cold restart |
| 4 | khoj rerun: "The agent did not accept your prompt within 120s" on claude1 | khoj leader (systemd `grok-leader`), cause undiagnosed | No for the required proof (khoj is optional); unexplained |
| 5 | Pi default model returns 429 | Pi provider config, not source | No; pinned `deepseek/deepseek-flash` via `PICKER_PI_MODEL` |
| 6 | `stop_leader` ran `pkill -f "grok-fork agent leader"`, which also matches the launchd leader | `scripts/picker-live.sh:233` | Fixed (commit f2e51cfc): own tmux session only |
| 6c | The Mac `leader-route.txt` is taken from the shared `~/.grok/leader.log` (stock 1.0.62 lines), not the fork leader's `/tmp/grok-leader.err` | `scripts/picker-live.sh` leader-evidence step | No; evidence for the Mac must be read from `/tmp/grok-leader.err` |
| 6b | After the dashboard-open failure my leader and pager were left running on the Mac; I stopped them by pid | `scripts/picker-live.sh:227`, `:239`, `:176` | No; cause not confirmed |
| 7 | The attached view does not always show the session title; the script only warns | `scripts/picker-live.sh` ("WARN attached view does not show the title") | No |

## Leaders I started

On the Air, none: S1's named launchd leader was used. On khoj, I used the existing systemd `grok-leader`. On the Mac mini I started one foreground leader and **stopped it** (see above). No foreground leader of mine is running.

## Re-running

```
scripts/picker-live.sh air sab-mini@sab-air
scripts/picker-live.sh mini sab-mini@sab-mini     # against the durable gui-domain leader (PICKER_START_LEADER=0 is the default)
scripts/picker-live.sh khojlocal local
```

Env: `PICKER_PAGER`, `PICKER_PI_MODEL` (default `deepseek/deepseek-flash`, empty string uses Pi's default), `PICKER_START_LEADER=1`, `PICKER_LEADER_SOCK`, `PICKER_OUT`.

## Haiku delegation log

1. Haiku 5.5: mapped the dashboard / resume code path (`dispatch/dashboard.rs`, leader routing) for the script design. Returned file:line references that I verified.
2. Haiku 5.5: extracted session ids, models and fresh-reply lines from the pane captures.
