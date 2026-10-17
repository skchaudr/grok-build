#!/usr/bin/env bash
# Live pager session: real pi-acp -> real Pi. Not a replay.
# Usage: scripts/w3-live.sh <120x40|80x24> <baseline|cancel> <out-name>
set -euo pipefail

SIZE="${1:?size 120x40 or 80x24}"
MODE="${2:?baseline or cancel}"
NAME="${3:?out name}"
case "$SIZE" in
  120x40) COLS=120; ROWS=40 ;;
  80x24) COLS=80; ROWS=24 ;;
  *) echo "size must be 120x40 or 80x24" >&2; exit 2 ;;
esac
case "$MODE" in
  baseline|cancel) ;;
  *) echo "mode must be baseline or cancel" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PAGER="${ACP_PAGER:-$ROOT/target/release/xai-grok-pager}"
PI_CLI="${PI_ACP_PI_COMMAND:-$HOME/wt/pi-acp-r2-recorder/.pi13/package/dist/bundle/cli.js}"
AGENT_DIR="${PI_CODING_AGENT_DIR:-/tmp/pi-w3-agent}"
WORK="${W3_WORKDIR:-$ROOT/docs/w3/codemode-real-pi/sandbox}"
OUT="$ROOT/docs/w3/codemode-real-pi"
SESSION="w3-gb-${NAME}"
GROK_HOME_DIR="${TMPDIR:-/tmp}/w3-grok-home-${NAME}"
DEBUG_FILE="$OUT/${NAME}.debug.log"
STREAM_ANSI="$OUT/${NAME}-stream.ansi"
DONE_ANSI="$OUT/${NAME}-done.ansi"

if [[ ! -x "$PAGER" ]]; then
  echo "pager missing: $PAGER" >&2
  exit 1
fi
if [[ ! -x "$PI_CLI" ]]; then
  echo "pi cli missing: $PI_CLI" >&2
  exit 1
fi

mkdir -p "$WORK" "$OUT" "$GROK_HOME_DIR"
: >"$DEBUG_FILE"

if [[ "$MODE" == "baseline" ]]; then
  PROMPT=$'Use the codemode tool exactly once. Do not call bash, read, edit, or write yourself.\n\nPass this script as the code argument, unchanged:\n\nconst a = await tools.bash({ command: "printf \'%s%s%s\' W3 NES TED1" });\nconst b = await tools.bash({ command: "printf \'%s%s%s\' W3 NES TED2" });\nreturn a.output + "\\n" + b.output;\n\nAfter the tool returns, reply with only the script output. If the tool returns an error, stop and quote the error. Do not retry. No other tools.'
  FINISH_NEEDLE="W3NESTED2"
else
  PROMPT=$'Use the codemode tool exactly once. Do not call bash, read, edit, or write yourself.\n\nPass this script as the code argument, unchanged:\n\nconst a = await tools.bash({ command: "printf \'%s%s\' W3 CAN" });\nawait tools.bash({ command: "sleep 25" });\nreturn "not-the-cancel-marker";\n\nDo not retry if the tool is cancelled.'
  FINISH_NEEDLE="cancelled"
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

inner="cd $(printf '%q' "$WORK") && \
  export PI_CODING_AGENT_DIR=$(printf '%q' "$AGENT_DIR") && \
  export PI_ACP_PI_COMMAND=$(printf '%q' "$PI_CLI") && \
  export GROK_HOME=$(printf '%q' "$GROK_HOME_DIR") && \
  exec $(printf '%q' "$PAGER") \
    --cwd $(printf '%q' "$WORK") \
    --always-approve \
    --trust \
    --no-leader \
    --fullscreen \
    --no-alt-screen \
    --debug-file $(printf '%q' "$DEBUG_FILE") \
    --agent-cmd pi-acp"

tmux -f /dev/null new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
  "bash -lc $(printf '%q' "$inner")"
tmux set-option -t "$SESSION" status off
tmux set-option -t "$SESSION" history-limit 5000
tmux resize-window -t "$SESSION" -x "$COLS" -y "$ROWS"

cleanup() {
  if [[ -z "${W3_KEEP:-}" ]]; then
    tmux kill-session -t "$SESSION" 2>/dev/null || true
  fi
}
trap cleanup EXIT

capture() {
  tmux capture-pane -e -p -t "$SESSION" 2>/dev/null || true
}

ready=0
deadline=$((SECONDS + 150))
while ((SECONDS < deadline)); do
  cap="$(capture)"
  if [[ "$cap" == *"Run Grok Build in a project directory?"* ]]; then
    tmux send-keys -t "$SESSION" 1
    sleep 0.3
    tmux send-keys -t "$SESSION" Enter
  fi
  if [[ "$cap" == *"always-approve"* && "$cap" == *"❯"* ]] || [[ "$cap" == *"Type a message"* ]]; then
    ready=1
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    break
  fi
  sleep 1
done

if [[ "$ready" -ne 1 ]]; then
  capture >"$DONE_ANSI"
  echo "composer never ready; saved $DONE_ANSI" >&2
  exit 1
fi

# Composer is focused and empty. Literal paste, then Enter.
tmux send-keys -t "$SESSION" -l -- "$PROMPT"
sleep 0.3
tmux send-keys -t "$SESSION" Enter

stream_saved=0
cancelled=0
deadline=$((SECONDS + 180))
while ((SECONDS < deadline)); do
  cap="$(capture)"
  if [[ "$cap" == *"Run Grok Build in a project directory?"* ]]; then
    tmux send-keys -t "$SESSION" 1
    sleep 0.2
    tmux send-keys -t "$SESSION" Enter
    continue
  fi
  if [[ "$MODE" == "baseline" && "$stream_saved" -eq 0 ]]; then
    if [[ "$cap" == *"codemode"* || "$cap" == *"W3NESTED1"* ]]; then
      printf '%s\n' "$cap" >"$STREAM_ANSI"
      stream_saved=1
    fi
  fi
  if [[ "$MODE" == "cancel" && "$cancelled" -eq 0 ]]; then
    if [[ "$cap" == *"codemode"* || "$cap" == *"sleep"* || "$cap" == *"W3CAN"* ]]; then
      printf '%s\n' "$cap" >"$STREAM_ANSI"
      # Pager cancel key is Ctrl+C when the composer is empty.
      tmux send-keys -t "$SESSION" C-c
      sleep 0.6
      cap2="$(capture)"
      if [[ "$cap2" == *"Stop running"* ]]; then
        tmux send-keys -t "$SESSION" Enter
      fi
      cancelled=1
    fi
  fi
  if [[ "$MODE" == "baseline" && "$cap" == *"$FINISH_NEEDLE"* ]]; then
    sleep 1
    capture >"$DONE_ANSI"
    break
  fi
  if [[ "$MODE" == "cancel" && "$cancelled" -eq 1 ]]; then
    sleep 2
    capture >"$DONE_ANSI"
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    printf '%s\n' "$cap" >"$DONE_ANSI"
    break
  fi
  sleep 0.4
done

if [[ ! -s "$DONE_ANSI" ]]; then
  capture >"$DONE_ANSI"
fi
size_now="$(tmux display-message -t "$SESSION" -p '#{window_width}x#{window_height}' 2>/dev/null || echo missing)"
echo "window=${size_now} stream=${stream_saved} cancel_sent=${cancelled} done=$(wc -l <"$DONE_ANSI")"
