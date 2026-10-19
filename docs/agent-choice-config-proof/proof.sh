#!/usr/bin/env bash
# Live proof: plain grok-fork (no --agent-choice flags) offers the claude/pi picker from
# ~/.grok/config.toml [[agent_choice]]. Drives the real pager in a tmux PTY (own socket),
# same key sequences as scripts/picker-live.sh (open_dashboard / pick_machine).
set -uo pipefail
OUT="$1"; mkdir -p "$OUT"
PAGER="$HOME/.grok/bin/grok-fork"
SOCK=agentchoice; SESS=agent-choice-proof
T() { tmux -L "$SOCK" "$@"; }
vis() { T capture-pane -p -t "$SESS" 2>/dev/null || true; }
snap() { vis >"$OUT/pane-$1.txt"; }
keys() { T send-keys -t "$SESS" "$@"; }
log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*" | tee -a "$OUT/steps.log" >&2; }
wait_for() { local end=$((SECONDS + $2)); while ((SECONDS < end)); do [[ "$(vis)" == *"$1"* ]] && return 0; T has-session -t "$SESS" 2>/dev/null || return 1; sleep 1; done; return 1; }
: >"$OUT/steps.log"
"$PAGER" --version >"$OUT/version.txt"
grep -n 'agent_choice\|^name = \|^cmd = ' "$HOME/.grok/config.toml" >"$OUT/config-agent-choice-lines.txt"
LINE="env -u CLAUDECODE GROK_AGENT_DASHBOARD=1 $PAGER --leader --leader-socket $HOME/.grok/leader.sock --always-approve --trust --fullscreen --no-alt-screen"
echo "$LINE" >"$OUT/launch-cmd.txt"
T kill-session -t "$SESS" 2>/dev/null || true
log "launch (cwd=/tmp): $LINE"
T -f /dev/null new-session -d -s "$SESS" -x 120 -y 40 -c /tmp "exec $LINE"
T resize-window -t "$SESS" -x 120 -y 40
sleep 5; snap 5s-after-launch; log "captured pane 5s after launch"
wait_for "❯" 30 || { log "FAIL no composer"; snap fail-no-composer; exit 1; }
keys 'C-\'; log "ctrl+\\ to open the dashboard"
wait_for "+ New Agent" 30 || { log "FAIL dashboard did not open"; snap fail-no-dashboard; exit 1; }
for ((i = 0; i < 15; i++)); do keys Up; sleep 0.12; done
snap dashboard; log "dashboard open, cursor on + New Agent"
keys Enter
if wait_for "enter select" 30; then
  snap picker-open; log "PASS picker opened with no --agent-choice flags"
else
  log "FAIL picker did not open"; snap fail-no-picker; exit 1
fi
keys Down; sleep 0.3; snap picker-cursor-pi; log "cursor moved to second entry"
keys Escape; sleep 0.5; snap picker-cancelled; log "picker cancelled with Esc"
