# Picker live proof: Claude and Pi sessions switched from the Grok dashboard

## Verdict

**PASS on the Air. PASS on khoj (Linux), once. Mac mini: UNPROVEN.**

- **Correction (02:39 UTC, lead):** an earlier version of this doc called the Mac mini "BLOCKED on Claude OAuth" and asked for a `/login`. That claim was **wrong**. The "Not logged in" failures came from a leader that my script started over ssh. An ssh-started leader runs in the Background launchd domain and cannot read the login keychain, so the failure says nothing about Sab's login. The gui-domain leader reached Claude fine at 02:26 UTC (`/tmp/grok-leader.err`: `sdk-initialize` and `register`, no auth error). No `/login` is needed.
- **Why the Mac run happened on my own leader:** the Mac's `~/.grok/leader.log` shows `Leader socket removed and no client attached; shutting down path=/Users/sab-mini/.grok/leader.sock` at 02:29:33 UTC, and `com.sab.grok-leader` was left exited (status 0). My script's default `PICKER_START_LEADER=1` ran `rm -f $LSOCK` (`scripts/picker-live.sh:244`) at about 02:29:27, because `leader-fork.sock` did not exist and it fell back to `leader.sock`. Evidence for the script as the cause: it is the only actor known to have removed exactly that path in that window, and the shutdown line names that path about six seconds later. Evidence against being certain: the script only reaches line 244 after `pgrep -f "grok-fork agent leader"` finds nothing, and `pgrep -f` does work over ssh on the Mac (checked), so the leader that shut down was either not matching that pattern or was already exiting. The `com.sab.grok-hub` restart (pid 44253 to 47036) is a different socket (`leader-hub.sock`). I cannot decide between these from the data, and I do not claim to.
- **Fix:** `PICKER_START_LEADER` now defaults to **0**. By default the script never starts, stops or removes a leader or its socket; it requires a leader already listening. Starting one is opt-in (`PICKER_START_LEADER=1`).
- **Mac Pi half:** two Pi sessions opened and replied on the Mac (status line `deepseek/DeepSeek V4.1 Flash`), but only against my ssh-started leader. Re-entry was not reached. The Mac needs a rerun against S1's durable gui-domain leader: `PICKER_START_LEADER=0 scripts/picker-live.sh mini sab-mini@sab-mini`.
- The **Air** (own pager, own leader, local agents) passed the full sequence: two Claude and two Pi sessions, each re-entered from the dashboard, each answering a fresh nonce-tagged question with its token.
- **khoj** (optional) passed once (`khojlocal`). A rerun failed on `claude1` with "The agent did not accept your prompt within 120s"; cause not diagnosed.
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

## Mac mini: UNPROVEN (my ssh-started leader could not use the keychain)

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
| 1 | My script started a leader over ssh (Background domain, no keychain), so Claude said "Not logged in"; the earlier OAuth-expired claim was wrong | `scripts/picker-live.sh:240-253` | Blocks the Mac mini proof until rerun against S1's durable leader. Fixed: leader start is now opt-in |
| 2 | Embedded pager hosts only the first `--agent-choice`; picker needs a leader | `app/cli.rs:898`, `app/mod.rs:572`; multi-backend routing at `leader/server.rs:3394` (`AGENT_CMD_META`), `:3412` (`resolve_backend`) | No. Works as designed with `--leader`; a pager without a fork leader shows no machine picker |
| 3 | `synthetic_session_load` fires only if the agent advertises `loadSession`, so `session/load` may never fire for external agents (`docs/acp-matrix.md` suspicion) | `leader/server.rs:936-1000` | No for this proof: re-entry from the dashboard attached the live session and gave fresh correct replies. I did not observe whether `session/load` was sent; the question stays open for a cold restart |
| 4 | khoj rerun: "The agent did not accept your prompt within 120s" on claude1 | khoj leader (systemd `grok-leader`), cause undiagnosed | No for the required proof (khoj is optional); unexplained |
| 5 | Pi default model returns 429 | Pi provider config, not source | No; pinned `deepseek/deepseek-flash` via `PICKER_PI_MODEL` |
| 6 | `rm -f $LSOCK` ran against `leader.sock` while a leader may have been serving it, and the Mac leader shut down six seconds later (see Verdict) | `scripts/picker-live.sh:244` | Was the likely cause of the Mac run going wrong; default now 0 so it cannot happen unless opted in |
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
