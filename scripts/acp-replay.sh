#!/usr/bin/env bash
# Replay an ACP jsonl into the real pager binary and capture the pane.
#
#   scripts/acp-replay.sh <acp.jsonl> [<out-name>]
#
# Starts ACP_PAGER (default ~/.grok/bin/grok-fork) in a detached tmux 120x40.
# The agent is scripts/acp-fake-peer.mjs: it answers initialize and session/new,
# then streams the file's session/update notifications.
#
# GROK_HOME is a fresh temp directory for this process only, so the replay
# does not write the user's ~/.grok. The script still diffs ~/.grok/sessions
# before and after and prints the result.
#
# Scroll keys. The pinned pager boots with vim mode off, so bare letters
# (g/G/h/l/j/k/e) do not reach scrollback actions; they type into the prompt.
# Keys that work in that mode (actions/defaults.rs):
#   Escape       dismiss an overlay
#   Tab          focus scrollback when the prompt is focused
#   Space        focus the prompt when scrollback is focused
#   Down / Up    select next / previous entry
#   Right        expand the selected entry (Left collapses)
#   PageDown / PageUp, Ctrl-d / Ctrl-u   move the viewport
# A second Tab while scrollback is focused returns to the prompt.
# No terminal screenshot tool is on this host; the .ansi file is the picture.
set -euo pipefail

usage() {
  echo "Usage: $0 <acp.jsonl> [<out-name>]" >&2
  exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || usage

JSONL="$1"
if [[ ! -f "$JSONL" ]]; then
  echo "ACP file not found: $JSONL" >&2
  exit 1
fi
JSONL="$(cd "$(dirname "$JSONL")" && pwd)/$(basename "$JSONL")"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PEER="${ROOT}/scripts/acp-fake-peer.mjs"
ACP_PAGER="${ACP_PAGER:-${HOME}/.grok/bin/grok-fork}"
NAME="${2:-$(basename "$JSONL" .jsonl)}"
OUT_DIR="${ROOT}/docs/r2/codemode-pane"
SESSION="acp-replay"
WORK="${TMPDIR:-/tmp}/acp-replay-${NAME}-$$"
GROK_HOME_DIR="${WORK}/grok-home"
DONE_FILE="${WORK}/done"
LOG_FILE="${WORK}/peer.log"
DEBUG_FILE="${WORK}/pager-debug.log"
PROMPT="${ACP_REPLAY_PROMPT:-replay ${NAME}}"
NEEDLE="${ACP_REPLAY_NEEDLE:-}"
DELAY_MS="${ACP_REPLAY_DELAY_MS:-15}"

if [[ ! -x "$ACP_PAGER" ]]; then
  echo "Pager binary missing or not executable: $ACP_PAGER" >&2
  exit 1
fi
if [[ ! -f "$PEER" ]]; then
  echo "Fake peer missing: $PEER" >&2
  exit 1
fi

mkdir -p "$OUT_DIR" "$WORK" "$GROK_HOME_DIR"
: >"$LOG_FILE"
: >"$DEBUG_FILE"
rm -f "$DONE_FILE"

snapshot_sessions() {
  local dest="$1"
  if [[ -d "${HOME}/.grok/sessions" ]]; then
    find "${HOME}/.grok/sessions" -type f | sort >"$dest"
  else
    : >"$dest"
  fi
}

snapshot_sessions "${WORK}/sessions-before.txt"

tmux kill-session -t "$SESSION" 2>/dev/null || true

AGENT_CMD="node ${PEER} ${JSONL}"
inner="cd $(printf '%q' "$WORK") && \
  export GROK_HOME=$(printf '%q' "$GROK_HOME_DIR") && \
  export ACP_REPLAY_DELAY_MS=$(printf '%q' "$DELAY_MS") && \
  export ACP_REPLAY_DONE=$(printf '%q' "$DONE_FILE") && \
  export ACP_REPLAY_LOG=$(printf '%q' "$LOG_FILE") && \
  exec $(printf '%q' "$ACP_PAGER") \
    --cwd $(printf '%q' "$WORK") \
    --always-approve \
    --trust \
    --no-leader \
    --fullscreen \
    --no-alt-screen \
    --debug-file $(printf '%q' "$DEBUG_FILE") \
    --agent-cmd $(printf '%q' "$AGENT_CMD")"

tmux -f /dev/null new-session -d -s "$SESSION" -x 120 -y 40 \
  "bash -lc $(printf '%q' "$inner")"
tmux set-option -t "$SESSION" status off
tmux set-option -t "$SESSION" history-limit 50000
tmux resize-window -t "$SESSION" -x 120 -y 40

cleanup() {
  if [[ -z "${ACP_REPLAY_KEEP:-}" ]]; then
    tmux kill-session -t "$SESSION" 2>/dev/null || true
  fi
}
trap cleanup EXIT

capture_visible() {
  tmux capture-pane -p -t "$SESSION" 2>/dev/null || true
}

composer_ready() {
  local text="$1"
  [[ "$text" == *"Type a message"* ]] && return 0
  [[ "$text" == *"always-approve"* && "$text" == *"❯"* ]] && return 0
  return 1
}

size="$(tmux display-message -t "$SESSION" -p '#{window_width}x#{window_height}' 2>/dev/null || echo missing)"
echo "window=${size}"
if [[ "$size" != "120x40" ]]; then
  echo "Window size is ${size}, expected 120x40" >&2
  capture_visible >&2 || true
  exit 1
fi

READY_DEADLINE=$((SECONDS + 120))
launch_ok=0
while ((SECONDS < READY_DEADLINE)); do
  cap="$(capture_visible)"
  if composer_ready "$cap"; then
    launch_ok=1
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    break
  fi
  sleep 1
done

if [[ "$launch_ok" -ne 1 ]]; then
  echo "Ready gate failed" >&2
  capture_visible >"${OUT_DIR}/${NAME}.txt" || true
  echo "Saved partial: ${OUT_DIR}/${NAME}.txt" >&2
  echo "peer log: ${LOG_FILE}" >&2
  echo "pager debug: ${DEBUG_FILE}" >&2
  exit 1
fi

tmux send-keys -t "$SESSION" -l -- "$PROMPT"
sleep 0.4
tmux send-keys -t "$SESSION" Enter

PICK_DEADLINE=$((SECONDS + 25))
while ((SECONDS < PICK_DEADLINE)); do
  cap="$(capture_visible)"
  if [[ "$cap" == *"Run Grok Build in a project directory?"* ]]; then
    tmux send-keys -t "$SESSION" 1
    sleep 0.3
    tmux send-keys -t "$SESSION" Enter
    break
  fi
  if [[ -f "$DONE_FILE" ]]; then
    break
  fi
  sleep 0.5
done

STREAM_DEADLINE=$((SECONDS + 180))
while ((SECONDS < STREAM_DEADLINE)); do
  if [[ -f "$DONE_FILE" ]]; then
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "tmux session died before the stream finished" >&2
    break
  fi
  sleep 0.5
done

if [[ ! -f "$DONE_FILE" ]]; then
  echo "Stream did not finish (no ${DONE_FILE})" >&2
  echo "--- peer log ---" >&2
  cat "$LOG_FILE" >&2 || true
  echo "--- pager debug (tail) ---" >&2
  tail -n 40 "$DEBUG_FILE" >&2 || true
  capture_visible >"${OUT_DIR}/${NAME}.txt" || true
  exit 1
fi

# Let the last chunks paint.
sleep 1

# Focus scrollback without a second Tab (that one returns to the prompt).
focus_scrollback() {
  tmux send-keys -t "$SESSION" Escape
  sleep 0.15
  tmux send-keys -t "$SESSION" Tab
  sleep 0.3
}

scrollback_focused=0

# Other-tool rows default to collapsed (title only). Right expands the
# selected entry. The user prompt is the first selected row, so Down once
# lands on the first tool. ACP_REPLAY_SELECT_DOWN overrides that count.
if [[ -n "${ACP_REPLAY_EXPAND:-}" ]]; then
  focus_scrollback
  scrollback_focused=1
  downs="${ACP_REPLAY_SELECT_DOWN:-1}"
  for _ in $(seq 1 "$downs"); do
    tmux send-keys -t "$SESSION" Down
    sleep 0.12
  done
  tmux send-keys -t "$SESSION" Right
  sleep 0.4
fi

scroll_needle() {
  local needle="$1"
  [[ -n "$needle" ]] || return 0
  local cap
  cap="$(capture_visible)"
  if [[ "$cap" == *"$needle"* ]]; then
    echo "needle already visible: ${needle}"
    return 0
  fi
  echo "scrolling to needle: ${needle}"
  if [[ "$scrollback_focused" -ne 1 ]]; then
    focus_scrollback
    scrollback_focused=1
  fi
  local i
  for i in $(seq 1 50); do
    cap="$(capture_visible)"
    if [[ "$cap" == *"$needle"* ]]; then
      echo "needle visible after ${i} pages"
      return 0
    fi
    tmux send-keys -t "$SESSION" PageDown
    sleep 0.12
  done
  echo "needle not found on screen: ${needle}" >&2
  return 1
}

scroll_ok=0
if scroll_needle "$NEEDLE"; then
  scroll_ok=1
fi

sleep 0.3
tmux capture-pane -e -p -t "$SESSION" >"${OUT_DIR}/${NAME}.ansi"
tmux capture-pane -p -t "$SESSION" >"${OUT_DIR}/${NAME}.txt"

snapshot_sessions "${WORK}/sessions-after.txt"
if cmp -s "${WORK}/sessions-before.txt" "${WORK}/sessions-after.txt"; then
  echo "sessions_delta=none (~/.grok/sessions unchanged)"
else
  echo "sessions_delta=CHANGED" >&2
  diff -u "${WORK}/sessions-before.txt" "${WORK}/sessions-after.txt" >&2 || true
fi

iso_count=0
if [[ -d "${GROK_HOME_DIR}/sessions" ]]; then
  iso_count="$(find "${GROK_HOME_DIR}/sessions" -type f | wc -l | tr -d ' ')"
fi
echo "isolated_grok_home_session_files=${iso_count}"
echo "isolated_grok_home=${GROK_HOME_DIR}"
echo "captured ${OUT_DIR}/${NAME}.ansi"
echo "captured ${OUT_DIR}/${NAME}.txt"
echo "peer_log=${LOG_FILE}"

if [[ -n "$NEEDLE" && "$scroll_ok" -ne 1 ]]; then
  exit 1
fi
exit 0
