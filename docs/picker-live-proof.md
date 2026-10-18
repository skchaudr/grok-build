# Picker live proof: Claude and Pi sessions switched from the Grok dashboard

## Verdict

**PASS on the Air. PASS on khoj (Linux), once. Mac mini: NOT PROVEN (0/4 sessions answered, in two runs against the durable launchd leader).** The Pi choice **does** reach `pi-acp` on the Mac; the stall is inside `pi`, which is a separate problem from routing.

### Why the Mac Pi choice looked misrouted, and was not

The lead read `result.env` (`pi1.model = pi2.model = "Default (recommended)"`, empty sids, a Claude error on pi2) as the `pi` choice going to the wrong backend. The evidence says otherwise:

1. **`pi-acp` was spawned by the leader at the moment each Pi session was opened.** Run 2: `pi-acp` pid 51191 (parent = leader 50705) started 19:46:14 local (02:46:14 UTC), when `pi1` was opened at 02:46:13; a `pi` child (51192) started a second later; a second `pi` child (51456) started 19:49:55, when `pi2`'s turn began. Run 3 (leader restarted meanwhile as pid 53520): `pi-acp` 54804 with two `pi` children (54805, 55111). `/tmp/picker-proof-out/mini/proc-tree-end.txt` holds the tree. A Claude backend never spawns `pi-acp`, so the `pi` entry was resolved to its own command. `ps eww` on 51191 also shows `PI_ACP_PI_COMMAND=/tmp/picker-live-pi-mini.sh` and `XPC_SERVICE_NAME=com.sab.grok-leader`, so the pin reached `pi-acp` as well.
2. **The empty model and the Claude error are driver artifacts of a session that never finished being created.** Each Pi open ends with `Session creation timed out after 180s` in the pager, so the pager never switches to a Pi view. The status line therefore still names the previous (Claude) view, which is where `Default (recommended)` and the Claude OAuth error come from. The empty session ids are the same cause: no Pi session id was ever issued.
3. **`pi` itself never started a session under the launchd leader.** `~/.pi/agent/sessions/--private-tmp--/` has no new file since the earlier ssh-leader run at 19:30 local. Both `pi` children sit idle (`sample` shows an idle libuv loop) and their stderr goes nowhere I can read.
4. **`pi-acp` works with the same bare PATH and the same wrapper from ssh.** I fed `initialize` and `session/new` over stdin to `env -i HOME=... PATH=/Users/sab-mini/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin PI_ACP_PI_COMMAND=/tmp/picker-live-pi-mini.sh /opt/homebrew/bin/pi-acp`: both returned results in under 3s, with `currentValue: deepseek/deepseek-flash`. So the Mac's PATH (the leader's PATH already includes `/opt/homebrew/bin`, unlike the "bare PATH" guess), the wrapper and the pin are all fine. What differs is only the launchd (gui-domain) context. I could not narrow it further: no hung `security` child exists (checked), TCC logs showed nothing, and the Pi provider for `deepseek/deepseek-flash` reads its key from `auth.json`, not the keychain.

Where the choice is carried: the pager sends the chosen command in the session request `_meta` as `x.ai/agentCmd` and the leader resolves it with `resolve_backend` (`crates/codegen/xai-grok-shell/src/leader/server.rs:3412`, `AGENT_CMD_META` at `:3394`). The process tree above shows that this worked for Pi. No fork bug in the choice path was found, so there was nothing to fix there.

### What the Mac runs showed

- **Run 1 (02:29 UTC)**: a leader started by my script over ssh (Background launchd domain, no keychain). Claude: `Not logged in`. Pi worked (8s replies). Re-entry not reached. That Claude failure was an artifact of the ssh-started leader.
- **Run 2 (02:45 UTC), run 3 (03:00 UTC)**, `PICKER_START_LEADER=0`, against the launchd leader on `leader-fork.sock`: Claude: session created, then `Failed to authenticate: OAuth session expired and could not be refreshed`. Pi: 180s creation timeout, as above. The Claude credential state under the gui domain is S1's lane and I did not chase it.
- Run 3 additionally failed every re-entry, because no session ever reached a Pi view and the Claude sessions never replied.
- **Leader restart note:** between runs 2 and 3 the leader was restarted (pid 50705 to 53520, by someone else). Pi children 51192, 51456 of the old `pi-acp` were orphaned (parent 1) and are still alive, as is 46225. They are not mine to kill.
- **Cause of the 02:29:33 leader shutdown (S1):** the stock 1.0.62 relay leader's "socket removed" line, and the fork leader's relaunch path at `leader/server.rs:1982-2009` (fires when a newer client attaches). My earlier `rm -f` theory is superseded.

### Script fixes (committed)

`PICKER_START_LEADER` defaults to 0; `stop_leader` kills only its own tmux session instead of pattern-killing; the script now also writes `proc-tree-end.txt`.

### Other targets

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
| 1 | Mac, launchd leader: Claude turn fails `OAuth session expired and could not be refreshed` (S1's lane); `pi` starts under `pi-acp` but never creates a session (180s timeout), while the same `pi-acp` works from ssh | launchd context; leader log `/tmp/grok-leader.err`, `proc-tree-end.txt` | **Blocks the Mac mini proof.** Cause open |
| 1b | The first Mac run's "Not logged in" was an artifact of my ssh-started leader | `scripts/picker-live.sh:240-253` | Fixed: leader start is opt-in |
| 2 | Embedded pager hosts only the first `--agent-choice`; picker needs a leader | `app/cli.rs:898`, `app/mod.rs:572`; multi-backend routing at `leader/server.rs:3394` (`AGENT_CMD_META`), `:3412` (`resolve_backend`) | No. Works as designed with `--leader`; a pager without a fork leader shows no machine picker |
| 3 | `synthetic_session_load` fires only if the agent advertises `loadSession`, so `session/load` may never fire for external agents (`docs/acp-matrix.md` suspicion) | `leader/server.rs:936-1000` | No for this proof: re-entry from the dashboard attached the live session and gave fresh correct replies. I did not observe whether `session/load` was sent; the question stays open for a cold restart |
| 4 | khoj rerun: "The agent did not accept your prompt within 120s" on claude1 | khoj leader (systemd `grok-leader`), cause undiagnosed | No for the required proof (khoj is optional); unexplained |
| 5 | Pi default model returns 429 | Pi provider config, not source | No; pinned `deepseek/deepseek-flash` via `PICKER_PI_MODEL` |
| 6 | `stop_leader` ran `pkill -f "grok-fork agent leader"`, which also matches the launchd leader | `scripts/picker-live.sh:233` | Fixed (commit f2e51cfc): own tmux session only |
| 6c | The Mac `leader-route.txt` mixes stale stock 1.0.62 lines from `~/.grok/logs/unified.jsonl` with the fork leader's `/tmp/grok-leader.err`, and `/tmp/grok-leader.err` lists only Claude sessions (the Pi adapter logs nothing there) | `scripts/picker-live.sh:292-296` | No; read the Pi routing from `proc-tree-end.txt` |
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
