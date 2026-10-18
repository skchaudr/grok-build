#!/usr/bin/env bash
# Live proof that one Grok pager can hold two Claude and two Pi sessions and
# move between them from the dashboard, each resuming where it left off.
# Drives the real pager, real claude and real pi-acp in a scripted tmux PTY
# (own socket: tmux -L picker). Nothing is faked: a session counts as resumed
# only when a NEW reply, produced after re-entering it, contains its token.
#
# Usage: scripts/picker-live.sh <machine-label>      e.g. mini | air
# Env:   PICKER_PAGER   pager binary (default ~/.grok/bin/grok-fork)
#        PICKER_OUT     evidence dir (default /tmp/picker-proof-out/<label>)
#        PICKER_LEADER_SOCKET  private leader socket (default: the shared one)
#        PICKER_LEADER_LOG  leader log to excerpt (default ~/.grok/leader.log)
set -uo pipefail

[[ $# -eq 1 ]] || { echo "Usage: $0 <machine-label>" >&2; exit 2; }
LABEL="$1"
PAGER_BIN="${PICKER_PAGER:-$HOME/.grok/bin/grok-fork}"
OUT="${PICKER_OUT:-/tmp/picker-proof-out/$LABEL}"
LEADER_LOG="${PICKER_LEADER_LOG:-$HOME/.grok/leader.log}"
WORK="/tmp/picker-live-work-$LABEL"
SOCK=picker
SESS=picker-live
RUN="$RANDOM$RANDOM"
TMUX_BIN="$(command -v tmux)"
T() { "$TMUX_BIN" -L "$SOCK" "$@"; }

[[ -x "$PAGER_BIN" ]] || { echo "pager missing: $PAGER_BIN" >&2; exit 1; }
mkdir -p "$OUT" "$WORK"
rm -f "$OUT"/pane-*.txt "$OUT"/result.env
: >"$OUT/steps.log"

# Same adapter selection as ~/.dsh/scripts/gb.
if [[ -x "$HOME/.local/bin/claude-agent-acp" ]]; then CLAUDE_ACP=$HOME/.local/bin/claude-agent-acp
elif command -v claude-agent-acp >/dev/null 2>&1; then CLAUDE_ACP=claude-agent-acp
elif command -v claude-acp >/dev/null 2>&1; then CLAUDE_ACP=claude-acp
else CLAUDE_ACP=claude-code-acp; fi
PI_ACP="${GB_PI_ACP:-pi-acp}"
if [[ -z "${GB_PI_ACP:-}" && -x "$HOME/repos/Pi-Coding-Agent/pi-acp-patched/run" ]]; then
  PI_ACP="$HOME/repos/Pi-Coding-Agent/pi-acp-patched/run"
fi
CLAUDE_CMD="env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=$(command -v claude) $CLAUDE_ACP"

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
  local want="$1" i row idx=0 n=0
  while IFS= read -r row; do
    [[ "$row" == *"New session"* || "$row" == *"enter select"* || "$row" == *"Machine"* ]] && continue
    [[ "$row" =~ ^[^[:alnum:]]*(\>)?[[:space:]]*([a-z0-9._-]+)[[:space:]]*[^[:alnum:]]*$ ]] || continue
    [[ "${BASH_REMATCH[2]}" == "$want" ]] && idx=$n
    n=$((n + 1))
  done < <(vis | sed -n '/New session/,/enter select/p')
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
  vis | grep -m1 -E 'Claude|Fable|Opus|Sonnet|Haiku|Gemini|gemini|gpt|grok-|Pi|antigravity' | sed 's/^[[:space:]]*//' || true
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
  pick_machine "$machine" || return 1
  if [[ "$via" == dash ]]; then
    wait_for "Type a message" 60 || wait_for "❯" 30 || true
    send_line "remember the token $token and reply only with it"
  fi
  # The reply echoes the token a second time (prompt line + reply line).
  local end=$((SECONDS + 150)) n
  while ((SECONDS < end)); do
    n=$(all | grep -c "$token" || true)
    ((n >= 2)) && break
    alive || return 1
    sleep 2
  done
  snap "$name-opened"
  SID[$name]="$(session_id)"
  MODEL[$name]="$(status_model)"
  ((n >= 2)) || { log "FAIL $name never replied with $token"; return 1; }
  log "$name replied; model='${MODEL[$name]}' sid='${SID[$name]}'"
}

# From a conversation open the dashboard (Ctrl+\).
open_dashboard() {
  keys 'C-\'
  wait_for "+ New Agent" 20 || { log "FAIL dashboard did not open"; snap dashboard-fail; return 1; }
}

# Dashboard row order, top to bottom, by which token each session row shows.
# Echo "<down-count>" to reach the row for <token> from the + New Agent button.
row_for() {
  local token="$1" n=0 line
  while IFS= read -r line; do
    if [[ "$line" == *ALPHA-* || "$line" == *BRAVO-* ]]; then
      n=$((n + 1))
      [[ "$line" == *"$token"* ]] && { echo "$n"; return 0; }
    fi
  done <<<"$(vis)"
  return 1
}

# Re-enter <name> from the dashboard, ask for its token, require a fresh reply.
reenter_and_ask() {
  local name="$1" token="$2" nonce="Q${RUN}${name//[^A-Za-z0-9]/}" down i
  open_dashboard || return 1
  snap "dash-before-$name"
  down="$(row_for "$token")" || { log "FAIL no dashboard row shows $token"; ANSWERED[$name]=no; return 1; }
  for ((i = 0; i < down; i++)); do keys Down; sleep 0.25; done
  keys Right
  sleep 3
  T clear-history -t "$SESS" 2>/dev/null || true
  wait_for "Type a message" 30 || true
  send_line "$nonce what token did I give you earlier in this session? Reply with only the token."
  if wait_reply "$token" "$nonce" 150; then
    ANSWERED[$name]=yes; log "$name RESUMED: fresh reply carried $token"
  else
    ANSWERED[$name]=no; log "$name NOT resumed: no fresh reply with $token"
  fi
  MODEL[$name]="${MODEL[$name]:-$(status_model)}"
  snap "$name-resumed"
  all >"$OUT/scrollback-$name.txt"
}

T kill-server 2>/dev/null || true
log "launch pager=$PAGER_BIN label=$LABEL"
T -f /dev/null new-session -d -s "$SESS" -x 120 -y 40 \
  "cd $WORK && exec env -u CLAUDECODE GROK_AGENT_DASHBOARD=1 $PAGER_BIN --cwd $WORK --leader ${PICKER_LEADER_SOCKET:+--leader-socket $PICKER_LEADER_SOCKET} --always-approve --trust --fullscreen --no-alt-screen --debug-file $OUT/pager-debug.log --agent-choice 'claude=$CLAUDE_CMD' --agent-choice 'pi=$PI_ACP'"
T set-option -t "$SESS" history-limit 20000
T resize-window -t "$SESS" -x 120 -y 40

if ! { wait_for "Type a message" 120 || wait_for "❯" 30; }; then
  snap launch-fail; log "BLOCKER pager never showed a composer"; exit 1
fi
snap launch

ok=1
open_session claude1 claude ALPHA-1 home || ok=0
open_dashboard && open_session pi1 pi BRAVO-1 dash || ok=0
open_dashboard && open_session claude2 claude ALPHA-2 dash || ok=0
open_dashboard && open_session pi2 pi BRAVO-2 dash || ok=0

# Cycle through all four from the dashboard, in an order that never stays put.
for pair in "claude1:ALPHA-1" "pi2:BRAVO-2" "pi1:BRAVO-1" "claude2:ALPHA-2" "claude1:ALPHA-1"; do
  reenter_and_ask "${pair%%:*}" "${pair##*:}" || ok=0
done

# Leader route evidence (paths only, no secrets: grep route/session lines).
if [[ -r "$LEADER_LOG" ]]; then
  grep -aiE 'route|backend|external|agentCmd|session/new|session/load' "$LEADER_LOG" | tail -80 >"$OUT/leader-route.txt" || true
fi
grep -a 'session/load' "$OUT/pager-debug.log" >"$OUT/session-load.txt" 2>/dev/null || true
all >"$OUT/scrollback.txt"

{
  echo "label=$LABEL"
  echo "pager=$PAGER_BIN"
  for k in claude1 pi1 claude2 pi2; do
    echo "$k.resumed=${ANSWERED[$k]:-not-run}"
    echo "$k.model=${MODEL[$k]:-}"
    echo "$k.sid=${SID[$k]:-}"
  done
  echo "all_ok=$ok"
} >"$OUT/result.env"
cat "$OUT/result.env"
T kill-server 2>/dev/null || true
((ok)) || exit 1
