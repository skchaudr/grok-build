#!/usr/bin/env bash
# Live proof that one Grok pager can hold two Claude and two Pi sessions and
# move between them from the dashboard, each resuming where it left off.
# Drives the real pager, real claude and real pi-acp in a scripted tmux PTY
# (own socket: tmux -L picker). Nothing is faked: a session counts as resumed
# only when a NEW reply, produced after re-entering it, contains its token.
#
# Topology: each machine runs its OWN fork leader and its OWN agents (the
# pager talks to the default ~/.grok/leader.sock on that machine). This script
# can run on any host; for a Darwin box it drives that box's pager through
# `ssh -tt`, so the pager, leader and every claude-agent-acp / pi-acp process
# are on the target machine. No tunnel, no cross-machine agent paths.
#
# Usage: scripts/picker-live.sh <label> <ssh-target|local>
#        e.g. picker-live.sh mini sab-mini@sab-mini ; picker-live.sh air sab-mini@sab-air
# Env:   PICKER_PAGER    pager binary ON THE TARGET (default ~/.grok/bin/grok-fork)
#        PICKER_PI_MODEL Pi model to pin ("" = Pi's own default; default openrouter/inception/mercury-2.5; deepseek/deepseek-flash is the fallback)
#        PICKER_START_LEADER=0 (default) never starts, stops or touches a leader or its socket;
#                        it requires one already listening. 1 starts a FOREGROUND fork leader on
#                        the target when none is running, and stops it at the end
#        PICKER_OUT      evidence dir (default /tmp/picker-proof-out/<label>)
set -uo pipefail

[[ $# -eq 2 ]] || { echo "Usage: $0 <label> <client-ssh-target|local>" >&2; exit 2; }
LABEL="$1"
CLIENT="$2"
# R <cmd>: run a shell command on the target machine (login PATH + the dirs
# the leader's launchd/systemd environment also has).
TPATH='$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin'
R() { if [[ "$CLIENT" == local ]]; then sh -c "export PATH=$TPATH:\$PATH; $1"; else ssh -o BatchMode=yes "$CLIENT" "export PATH=$TPATH:\$PATH; $1"; fi; }
CLIENT_HOME="$(R 'printf %s "$HOME"')"
PAGER_BIN="${PICKER_PAGER:-$CLIENT_HOME/.grok/bin/grok-fork}"
OUT="${PICKER_OUT:-/tmp/picker-proof-out/$LABEL}"
WORK=/tmp
SOCK=picker
SESS=picker-live
RUN="$RANDOM$RANDOM"
TMUX_BIN="$(command -v tmux)"
T() { "$TMUX_BIN" -L "$SOCK" "$@"; }

R "test -x $(printf %q "$PAGER_BIN")" || { echo "pager missing on target: $PAGER_BIN" >&2; exit 1; }
mkdir -p "$OUT" "$WORK"
rm -f "$OUT"/pane-*.txt "$OUT"/result.env
: >"$OUT/steps.log"

# Agent commands are resolved ON THE TARGET, so the leader there spawns them.
# PICKER_CLAUDE_CMD: full claude --agent-choice command on the target (e.g. a
# named API-key wrapper such as ~/.local/bin/claude-agent-acp-api); used as is.
# PICKER_PI_BIN: pi binary the Pi pin wrapper execs (default: `command -v pi`).
CLAUDE_ACP="$(R 'command -v claude-agent-acp || command -v claude-acp || command -v claude-code-acp')"
CLAUDE_BIN="$(R 'command -v claude')"
PI_ACP="$(R 'command -v pi-acp')"
[[ -n "${PICKER_CLAUDE_CMD:-}" ]] && { CLAUDE_ACP="${PICKER_CLAUDE_CMD}"; CLAUDE_BIN="${CLAUDE_BIN:-none}"; }
[[ -n "$CLAUDE_ACP" && -n "$CLAUDE_BIN" && -n "$PI_ACP" ]] || { echo "agent binaries missing on target: claude-acp='$CLAUDE_ACP' claude='$CLAUDE_BIN' pi-acp='$PI_ACP'" >&2; exit 1; }
# Pi's default model (google-antigravity Gemini) was out of quota on 2026-10-18
# (429), so pin a working one through pi-acp's PI_ACP_PI_COMMAND. Set
# PICKER_PI_MODEL="" to use Pi's own default.
PI_MODEL="${PICKER_PI_MODEL-openrouter/inception/mercury-2.5}"
if [[ -n "$PI_MODEL" ]]; then
  PI_WRAP="/tmp/picker-live-pi-$LABEL.sh"
  R "PIBIN=${PICKER_PI_BIN:-\$(command -v pi)}; printf '#!/bin/sh\\nexec %s --model %s \"\$@\"\\n' \"\$PIBIN\" $PI_MODEL > $PI_WRAP; chmod +x $PI_WRAP"
  PI_ACP="env PI_ACP_PI_COMMAND=$PI_WRAP $PI_ACP"
fi
if [[ -n "${PICKER_CLAUDE_CMD:-}" ]]; then CLAUDE_CMD="env -u CLAUDECODE $CLAUDE_ACP"; else CLAUDE_CMD="env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=$CLAUDE_BIN $CLAUDE_ACP"; fi

log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*" | tee -a "$OUT/steps.log" >&2; }
vis() { T capture-pane -p -t "$SESS" 2>/dev/null || true; }
all() { T capture-pane -p -t "$SESS" -S - 2>/dev/null || true; }
snap() { vis >"$OUT/pane-$1.txt"; }
keys() { T send-keys -t "$SESS" "$@"; }
alive() { T has-session -t "$SESS" 2>/dev/null; }

# wait_for <needle> <timeout-s>: needle is a fixed string in the visible pane.
wait_for() {
  local needle="$1" end=$((SECONDS + $2))
  while ((SECONDS < end)); do
    [[ "$(vis)" == *"$needle"* ]] && return 0
    alive || return 1
    sleep 1
  done
  return 1
}

# send_line <text>: type into the composer and submit.
send_line() { keys -l -- "$1"; sleep 0.5; keys Enter; }

# pick_machine <name>: answer the "New session / Machine" picker.
pick_machine() {
  wait_for "enter select" 30 || { log "FAIL machine picker did not open for $1"; return 1; }
  local i idx=0
  [[ "$1" == pi ]] && idx=1   # --agent-choice order: claude, pi
  for ((i = 0; i < idx; i++)); do keys Down; sleep 0.2; done
  keys Enter
}

# First submit on the home screen may ask about the project directory.
dismiss_project_prompt() {
  local end=$((SECONDS + 15))
  while ((SECONDS < end)); do
    local v; v="$(vis)"
    if [[ "$v" == *"Run Grok Build in a project directory?"* ]]; then keys 1; sleep 0.3; keys Enter; return; fi
    [[ "$v" == *"enter select"* ]] && return
    sleep 1
  done
}

# wait_reply <token> <question-nonce> <timeout-s>: succeed only when the token
# appears on a line AFTER the line holding the question nonce. The original
# prompt that introduced the token sits above the question, so scrollback
# re-render cannot satisfy this; only a fresh assistant reply can.
reply_has_token() {
  local token="$1" nonce="$2" line seen=0
  while IFS= read -r line; do
    if [[ "$line" == *"$nonce"* ]]; then seen=1; continue; fi
    [[ "$seen" -eq 1 && "$line" == *"$token"* ]] && return 0
  done <<<"$(all)"
  return 1
}
wait_reply() {
  local end=$((SECONDS + $3))
  while ((SECONDS < end)); do
    reply_has_token "$1" "$2" && return 0
    alive || return 1
    sleep 2
  done
  return 1
}

declare -A SID MODEL ANSWERED BACKEND_LINE

# status_model: the pager status/header lines naming the model, from the pane.
status_model() {
  vis | grep -E 'always-approve' | tail -1 | sed -E 's/^[^A-Za-z]*//; s/ · always-approve.*//; s/[─╯╰]//g'
}
session_id() {
  # The pager prints the session id on /status-like surfaces; fall back to the
  # most recent route line in the leader log.
  grep -aoE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' <<<"$(all)" | tail -1
}

# Open a session on <machine> and teach it <token>.
open_session() {
  local name="$1" machine="$2" token="$3" via="$4"
  log "open $name ($machine) token=$token via=$via"
  if [[ "$via" == home ]]; then
    send_line "remember the token $token and reply only with it"
    dismiss_project_prompt
  else
    # Dashboard: focus + New Agent and press Enter on it.
    keys Enter
  fi
  if [[ "$via" == dash ]]; then
    pick_machine "$machine" || return 1
    wait_for "Starting session" 10 || true
    local e=$((SECONDS + 90))
    while ((SECONDS < e)) && [[ "$(vis)" == *"Starting session"* ]]; do sleep 1; done
    sleep 2
    send_line "remember the token $token and reply only with it"
  fi
  # The reply echoes the token a second time (prompt line + reply line).
  local end=$((SECONDS + 150)) n
  while ((SECONDS < end)); do
    # A reply is a line that ENDS with the token (plus the pager's clock); the prompt and the header
    # title both start with other words.
    n=$(all | grep -cE "$token\\.?[[:space:]]*([0-9]+:[0-9]+ [AP]M)?[[:space:]█▌▐▒░]*$" || true)
    ((n >= 1)) && break
    if [[ "$(vis)" == *"Turn failed"* ]]; then log "BLOCKER $name turn failed: $(vis | grep -m1 -E 'Failed to|Turn failed' | sed 's/^[^A-Za-z]*//')"; break; fi
    alive || return 1
    sleep 2
  done
  snap "$name-opened"
  SID[$name]="$(session_id)"
  MODEL[$name]="$(status_model)"
  ((n >= 1)) || { log "FAIL $name never replied with $token"; return 1; }
  log "$name replied; model='${MODEL[$name]}' sid='${SID[$name]}'"
}

# From a conversation open the dashboard (Ctrl+\).
open_dashboard() {
  keys 'C-\'
  wait_for "+ New Agent" 20 || { log "FAIL dashboard did not open"; snap dashboard-fail; return 1; }
  # Re-opening lands on the last attached row; Up clamps at the + New Agent button.
  local i
  for ((i = 0; i < 15; i++)); do keys Up; sleep 0.12; done
}

# goto_row <token>: walk the dashboard one row at a time from the top. The
# focused row is the one drawn with a leading "▏"; stop when it shows the token,
# and open any collapsed ("▸") section the cursor lands on. This does not
# assume how many other sessions or pinned rows the machine already has.
goto_row() {
  local token="$1" i f
  for ((i = 0; i < 80; i++)); do
    keys Down; sleep 0.3
    f="$(vis | grep -m1 '▏')"
    [[ "$f" == *"remember the token $token"* ]] && return 0
    [[ "$f" == *"▸"* ]] && { keys Enter; sleep 0.4; }
  done
  return 1
}

# Re-enter <name> from the dashboard, ask for its token, require a fresh reply.
reenter_and_ask() {
  local name="$1" token="$2" i nonce
  NCOUNT=$((${NCOUNT:-0}+1)); nonce="Q${RUN}${name//[^A-Za-z0-9]/}n$NCOUNT"
  open_dashboard || return 1
  snap "dash-before-$name"
  goto_row "$token" || { log "FAIL no dashboard row reached for $token"; snap "dash-fail-$name"; ANSWERED[$name]=no; keys Escape; return 1; }
  keys Right
  sleep 3
  [[ "$(vis)" == *"remember the token $token"* ]] || log "WARN attached view does not show the $token title"
  T clear-history -t "$SESS" 2>/dev/null || true
  wait_for "❯" 30 || true
  send_line "$nonce Reply with only the token I asked you to remember."
  if wait_reply "$token" "$nonce" 150; then
    ANSWERED[$name]=yes; log "$name RESUMED: fresh reply carried $token"; rc=0
  else
    ANSWERED[$name]=no; log "$name NOT resumed: no fresh reply with $token"; rc=1
  fi
  MODEL[$name]="${MODEL[$name]:-$(status_model)}"
  snap "$name-resumed"
  all >"$OUT/scrollback-$name.txt"
  return "${rc:-0}"
}

# The fork leader socket: ~/.grok/leader-fork.sock where S1 installed a named
# launchd leader beside a stock one, else the default ~/.grok/leader.sock.
LSOCK="${PICKER_LEADER_SOCK:-}"
if [[ -z "$LSOCK" ]]; then
  if R 'test -S "$HOME/.grok/leader-fork.sock"'; then LSOCK="$CLIENT_HOME/.grok/leader-fork.sock"; else LSOCK="$CLIENT_HOME/.grok/leader.sock"; fi
fi
LEADER_STARTED=0
stop_leader() {
  # Only a leader this script started (PICKER_START_LEADER=1) is stopped, by
  # closing its own tmux session; never a pattern kill, which would also hit
  # the launchd/systemd leader. Closing the ssh pty hangs up the leader.
  if ((LEADER_STARTED)); then
    T kill-session -t picker-leader 2>/dev/null || true
    sleep 2
    log "leader session closed (processes on target, for the record): $(R 'pgrep -f "grok-fork agent leader"' 2>&1 | tr '\n' ' ')"
    LEADER_STARTED=0
  fi
}
trap stop_leader EXIT
if [[ "${PICKER_START_LEADER:-0}" == 1 ]]; then
  if R 'pgrep -f "grok-fork agent leader"' >/dev/null 2>&1; then
    log "using the leader already running on the target (not started or stopped by this script)"
  else
  R "rm -f $LSOCK"
  # Foreground process under tmux; the process is named by its real binary.
  LEADER_LINE="exec $(printf %q "$PAGER_BIN") agent leader --no-exit-on-disconnect --no-auto-update --leader-socket $LSOCK"
  if [[ "$CLIENT" == local ]]; then LL="$LEADER_LINE"; else LL="exec ssh -tt $CLIENT $(printf %q "export PATH=$TPATH:\$PATH; $LEADER_LINE")"; fi
  T -f /dev/null new-session -d -s picker-leader "$LL"
  LEADER_STARTED=1
  for _ in $(seq 30); do R "test -S $LSOCK" && break; sleep 1; done
  R "test -S $LSOCK" || { echo "BLOCKER leader socket never appeared" >&2; exit 1; }
  log "foreground leader started"
  fi
fi
R "test -S $LSOCK" || { echo "no leader socket on target" >&2; exit 1; }
START_TS="$(date "+%Y-%m-%d %H:%M:%S")"
T kill-session -t "$SESS" 2>/dev/null || true
log "launch pager=$PAGER_BIN label=$LABEL"
PAGER_ARGS="--leader --leader-socket $LSOCK --always-approve --trust --fullscreen --no-alt-screen --cwd $WORK --agent-choice $(printf %q "claude=$CLAUDE_CMD") --agent-choice $(printf %q "pi=$PI_ACP")"
PAGER_LINE="env -u CLAUDECODE GROK_AGENT_DASHBOARD=1 $(printf %q "$PAGER_BIN") $PAGER_ARGS"
if [[ "$CLIENT" == local ]]; then
  LAUNCH="exec $PAGER_LINE"
else
  LAUNCH="exec ssh -tt $CLIENT $(printf %q "export PATH=$TPATH:\$PATH; exec $PAGER_LINE")"
fi
echo "$LAUNCH" | sed -E 's/(KEY|TOKEN)=[^ ]+/\1=<redacted>/g' >"$OUT/launch-cmd.txt"
T -f /dev/null new-session -d -s "$SESS" -x 120 -y 40 "$LAUNCH"
T set-option -t "$SESS" history-limit 20000
T resize-window -t "$SESS" -x 120 -y 40

if ! wait_for "❯" 60; then
  snap launch-fail; log "BLOCKER pager never showed a composer"; exit 1
fi
snap launch

ok=1
open_session claude1 claude ALPHA-1 home || ok=0
open_dashboard && open_session pi1 pi BRAVO-1 dash || ok=0
open_dashboard && open_session claude2 claude ALPHA-2 dash || ok=0
open_dashboard && open_session pi2 pi BRAVO-2 dash || ok=0

# Agents live on the target while the four sessions are open.
R 'hostname; uname -sm' >"$OUT/host.txt" 2>&1 || true
R 'ps -axo pid,args | grep -E "[c]laude-agent-acp" | cut -c1-200' >"$OUT/pgrep-claude-live.txt" 2>&1 || true
R 'ps -axo pid,args | grep -E "[p]i-acp" | cut -c1-200' >"$OUT/pgrep-pi-live.txt" 2>&1 || true
R 'ps -axo pid,args | grep -E "[g]rok-fork agent leader" | cut -c1-200' >"$OUT/pgrep-leader.txt" 2>&1 || true
# Cycle through all four from the dashboard, in an order that never stays put.
for pair in "claude1:ALPHA-1" "pi2:BRAVO-2" "pi1:BRAVO-1" "claude2:ALPHA-2" "claude1:ALPHA-1"; do
  reenter_and_ask "${pair%%:*}" "${pair##*:}" || ok=0
done

# Leader route evidence from the target's OWN leader (route/spawn lines only).
case "$(R 'uname -s')" in
  Linux) R "journalctl --user -u grok-leader --since '$START_TS' --no-pager" ;;
  *) R 'cat /tmp/grok-leader.out /tmp/grok-leader.err 2>/dev/null; tail -n 600 ~/.grok/logs/unified.jsonl 2>/dev/null' ;;
esac 2>/dev/null | grep -aE 'phase=register|phase=sdk-initialize|route|agentCmd|session/new|session/load|spawn|load_session' | cut -c1-300 | tail -150 >"$OUT/leader-route.txt" || true
R 'ps -axo pid,args | grep -E "[c]laude-agent-acp" | cut -c1-200' >"$OUT/pgrep-claude-end.txt" 2>&1 || true
R 'ps -axo pid,args | grep -E "[p]i-acp" | cut -c1-200' >"$OUT/pgrep-pi-end.txt" 2>&1 || true
R 'ps -axo pid,ppid,etime,command | grep -E "[p]i-acp|[c]laude-agent-acp|[a]gent leader|^ *[0-9]+ +[0-9]+ +[^ ]+ +pi *$" | cut -c1-200' >"$OUT/proc-tree-end.txt" 2>&1 || true
all >"$OUT/scrollback.txt"

{
  echo "label=$LABEL"
  echo "pager=$CLIENT:$PAGER_BIN"
  for k in claude1 pi1 claude2 pi2; do
    echo "$k.resumed=${ANSWERED[$k]:-not-run}"
    echo "$k.model=${MODEL[$k]:-}"
    echo "$k.sid=${SID[$k]:-}"
  done
  echo "all_ok=$ok"
} >"$OUT/result.env"
cat "$OUT/result.env"
T kill-session -t "$SESS" 2>/dev/null || true
stop_leader
((ok)) || exit 1
