#!/usr/bin/env bash
# Drive xai-grok-pager --agent-cmd for one ACP agent inside a detached
# 120x36 tmux session. Exit 0 only when the pane capture shows the prompt,
# assistant text, and a tool card for a harmless `ls`.
set -euo pipefail

usage() {
  echo "Usage: $0 <agent>" >&2
  echo "Agents: dsh | cursor | pi | codex | grok | claude" >&2
  exit 2
}

[[ $# -eq 1 ]] || usage

AGENT="$1"
case "$AGENT" in
  dsh | cursor | pi | codex | grok | claude) ;;
  *) usage ;;
esac

# Not $PAGER: that variable is the system pager (often `less`).
ACP_PAGER="${ACP_PAGER:-/home/sab-mini/repos/grok-build/target/release/xai-grok-pager}"
SESSION="acp-matrix-${AGENT}"
WORKDIR="/tmp/acp-matrix-${AGENT}"
OUT_DIR="/tmp/acp-matrix-out/${AGENT}"
PANE_PATH="${OUT_DIR}/pane.txt"
SCROLL_PATH="${OUT_DIR}/scrollback.txt"
DEBUG_FILE="${OUT_DIR}/pager-debug.log"
ENV_FILE="${OUT_DIR}/dsh.env"
NONCE="matrix${RANDOM}${RANDOM}"
PROMPT="Nonce ${NONCE}. Run ls with your shell tool, then reply done."

if [[ ! -x "$ACP_PAGER" ]]; then
  echo "Pager binary missing or not executable: $ACP_PAGER" >&2
  exit 1
fi

mkdir -p "$WORKDIR" "$OUT_DIR"
: >"$DEBUG_FILE"
printf '%s\n' sentinel >"${WORKDIR}/SENTINEL.txt"
rm -f "$ENV_FILE"

agent_cmd_for() {
  case "$1" in
    dsh) echo "dsh --profile acp-enhanced --patch ${OUT_DIR}/cliproxy.patch.yml" ;;
    cursor) echo 'cursor-agent acp' ;;
    claude) echo 'claude-code-acp' ;;
    pi) echo 'pi-acp' ;;
    codex) echo 'codex-acp' ;;
    grok) echo 'grok agent stdio' ;;
  esac
}

AGENT_CMD="$(agent_cmd_for "$AGENT")"

load_cliproxy_key() {
  python3 - <<'PY'
import sys
try:
    import yaml
except ImportError:
    sys.exit(2)
path = "/home/sab-mini/.dsh/.credentials.yaml"
try:
    with open(path, encoding="utf-8") as f:
        data = yaml.safe_load(f) or {}
except OSError:
    sys.exit(1)
key = (data.get("refs") or {}).get("CLIPROXY_API_KEY")
if not key or not str(key).strip():
    sys.exit(1)
sys.stdout.write(str(key).strip())
PY
}

if [[ "$AGENT" == "dsh" ]]; then
  if ! CLIPROXY_API_KEY="$(load_cliproxy_key)"; then
    echo "CLIPROXY_API_KEY missing from /home/sab-mini/.dsh/.credentials.yaml refs" >&2
    exit 1
  fi
  umask 077
  cat >"$ENV_FILE" <<EOF
DSH_ACP_PROVIDER=cliproxy
DSH_ACP_MODEL=gemini-3.8-flash-high
CLIPROXY_API_KEY=${CLIPROXY_API_KEY}
EOF
  unset CLIPROXY_API_KEY
  # acp-enhanced does not register a cliproxy adapter. This throwaway patch
  # only mounts the :8317 route for the matrix cwd; it is not a profile edit.
  cat >"${OUT_DIR}/cliproxy.patch.yml" <<'EOF'
- id: llm-pi-ai
  config:
    providers:
      cliproxy:
        displayName: CLIProxyAPI (sandbox :8317)
        apiKeyEnv: CLIPROXY_API_KEY
        api: openai-completions
        baseURL: http://100.66.99.64:8317/v1
        models:
          - id: gemini-3.8-flash-high
            name: gemini-3.8-flash-high
            input:
              - text
EOF
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

launch_pager() {
  local extra_args=("$@")
  local env_prefix=""
  if [[ -f "$ENV_FILE" ]]; then
    env_prefix="set -a; source $(printf '%q' "$ENV_FILE"); set +a; "
  fi
  local inner
  inner="${env_prefix}cd $(printf '%q' "$WORKDIR") && exec $(printf '%q' "$ACP_PAGER") \
    --cwd $(printf '%q' "$WORKDIR") \
    --always-approve \
    --trust \
    --no-leader \
    --fullscreen \
    --no-alt-screen \
    --debug-file $(printf '%q' "$DEBUG_FILE") \
    --agent-cmd $(printf '%q' "$AGENT_CMD")"
  if ((${#extra_args[@]})); then
    local arg
    for arg in "${extra_args[@]}"; do
      inner+=" $(printf '%q' "$arg")"
    done
  fi
  tmux -f /dev/null new-session -d -s "$SESSION" -x 120 -y 36 \
    "bash -lc $(printf '%q' "$inner")"
  tmux set-option -t "$SESSION" history-limit 10000
  tmux resize-window -t "$SESSION" -x 120 -y 36
}

capture_visible() {
  tmux capture-pane -p -t "$SESSION" 2>/dev/null || true
}

capture_scrollback() {
  tmux capture-pane -p -t "$SESSION" -S - 2>/dev/null || true
}

save_artifacts() {
  capture_visible >"$PANE_PATH"
  capture_scrollback >"$SCROLL_PATH"
}

write_result_env() {
  local size
  size="$(tmux display-message -t "$SESSION" -p '#{window_width}x#{window_height}' 2>/dev/null || echo missing)"
  {
    echo "launch_ok=${launch_ok:-0}"
    echo "prompt_ok=${prompt_ok:-0}"
    echo "assistant_ok=${assistant_ok:-0}"
    echo "tool_ok=${tool_ok:-0}"
    echo "slash_ok=${slash_ok:-0}"
    echo "resume_ok=${resume_ok:-0}"
    echo "window=${WINDOW_SIZE:-missing}"
  } >"${OUT_DIR}/result.env"
}

composer_ready() {
  local text="$1"
  # The composer placeholder is not always painted. The live prompt row is
  # the ❯ glyph plus the always-approve flag this script turns on.
  [[ "$text" == *"Type a message"* ]] && return 0
  [[ "$text" == *"always-approve"* && "$text" == *"❯"* ]] && return 0
  return 1
}

is_chrome_line() {
  local line="$1"
  [[ -z "${line//[[:space:]]/}" ]] && return 0
  [[ "$line" == *"Type a message"* ]] && return 0
  [[ "$line" == *"always-approve"* ]] && return 0
  [[ "$line" == *"Shift+Tab"* || "$line" == *"Ctrl+"* || "$line" == *"Enter:send"* ]] && return 0
  [[ "$line" == *"Esc:unselect"* || "$line" == *"Tab:scrollback"* ]] && return 0
  [[ "$line" == *"──"* || "$line" == *"━━"* ]] && return 0
  [[ "$line" =~ ^[[:space:]]*[┌└┐┘│├┤┬┴┼╭╮╯╰─━] ]] && return 0
  return 1
}

is_tool_line() {
  local line="$1"
  # A literal "$ ls" header. An unescaped $ in [[ =~ ]] is end-of-line, which
  # false-matched the prompt because the prompt ends in "ls".
  [[ "$line" =~ [$][[:space:]]*ls([^[:alnum:]_]|$) ]] && return 0
  [[ "$line" == *"Run "* && "$line" == *"ls"* && "$line" != *"/doctor"* ]] && return 0
  [[ "$line" == *"Running"* && "$line" == *"ls"* ]] && return 0
  [[ "$line" == *"Ran "* && "$line" == *"command"* ]] && return 0
  [[ "$line" == *"Bash"* && "$line" == *"ls"* ]] && return 0
  return 1
}

check_prompt_ok() {
  local text="$1"
  [[ "$text" == *"$NONCE"* && "$text" == *"reply done"* ]]
}

check_tool_ok() {
  local text="$1"
  local line seen_prompt=0
  while IFS= read -r line; do
    if [[ "$line" == *"$NONCE"* ]]; then
      seen_prompt=1
      continue
    fi
    [[ "$seen_prompt" -eq 1 ]] || continue
    if is_tool_line "$line"; then
      return 0
    fi
  done <<<"$text"
  return 1
}

check_assistant_ok() {
  local text="$1"
  local line seen_prompt=0 seen_tool=0
  while IFS= read -r line; do
    if [[ "$line" == *"$NONCE"* ]]; then
      seen_prompt=1
      continue
    fi
    [[ "$seen_prompt" -eq 1 ]] || continue
    if is_tool_line "$line"; then
      seen_tool=1
      continue
    fi
    # Prompt wraps and status lines sit above the tool card. A reply is text
    # after that card.
    [[ "$seen_tool" -eq 1 ]] || continue
    is_chrome_line "$line" && continue
    [[ "$line" == *"Grok Build"* || "$line" == *"Ask Grok"* || "$line" == *"/doctor"* ]] && continue
    [[ "$line" == *"SENTINEL.txt"* || "$line" == *"Waiting for"* || "$line" == *"Worked for"* ]] && continue
    [[ "$line" == *"Turn failed"* || "$line" == *"Internal error"* || "$line" == *"no content"* ]] && continue
    [[ "$line" == *"Try sending"* || "$line" == *"Request failed"* || "$line" == *"Dashboard"* ]] && continue
    [[ "$line" =~ [[:alpha:]] ]] || continue
    return 0
  done <<<"$text"
  return 1
}

launch_ok=0
prompt_ok=0
assistant_ok=0
tool_ok=0
slash_ok=0
resume_ok=0

launch_pager

size="$(tmux display-message -t "$SESSION" -p '#{window_width}x#{window_height}' 2>/dev/null || echo missing)"
WINDOW_SIZE="$size"
if [[ "$size" != "120x36" ]]; then
  save_artifacts
  write_result_env
  echo "Window size is ${size}, expected 120x36" >&2
  exit 1
fi

READY_DEADLINE=$((SECONDS + 120))
while ((SECONDS < READY_DEADLINE)); do
  cap="$(capture_scrollback)"
  if composer_ready "$cap"; then
    launch_ok=1
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    break
  fi
  sleep 2
done

if [[ "$launch_ok" -ne 1 ]]; then
  save_artifacts
  write_result_env
  echo "Ready gate failed: pane did not show the composer within 120s" >&2
  echo "Saved: $PANE_PATH" >&2
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  exit 1
fi

tmux send-keys -t "$SESSION" -l -- "$PROMPT"
sleep 0.4
tmux send-keys -t "$SESSION" Enter

# First submit from the home screen opens a project picker. Option 1 is the
# current throwaway cwd. Do not pick "Don't ask me again" (that writes config).
PICK_DEADLINE=$((SECONDS + 20))
while ((SECONDS < PICK_DEADLINE)); do
  cap="$(capture_visible)"
  if [[ "$cap" == *"Run Grok Build in a project directory?"* ]]; then
    tmux send-keys -t "$SESSION" 1
    sleep 0.3
    tmux send-keys -t "$SESSION" Enter
    break
  fi
  if check_prompt_ok "$cap"; then
    break
  fi
  sleep 1
done

PROMPT_DEADLINE=$((SECONDS + 180))
cap=""
while ((SECONDS < PROMPT_DEADLINE)); do
  cap="$(capture_scrollback)"
  if [[ "$cap" == *"Turn failed"* || "$cap" == *"server shut down"* ]]; then
    break
  fi
  if [[ -f "$DEBUG_FILE" ]] && grep -q 'session/prompt" error' "$DEBUG_FILE"; then
    break
  fi
  if check_prompt_ok "$cap" && check_assistant_ok "$cap" && check_tool_ok "$cap"; then
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    break
  fi
  sleep 2
done

sleep 1
save_artifacts
cap="$(cat "$SCROLL_PATH")"
check_prompt_ok "$cap" && prompt_ok=1 || prompt_ok=0
check_assistant_ok "$cap" && assistant_ok=1 || assistant_ok=0
check_tool_ok "$cap" && tool_ok=1 || tool_ok=0

failed=()
[[ "$prompt_ok" -eq 1 ]] || failed+=("prompt")
[[ "$assistant_ok" -eq 1 ]] || failed+=("assistant")
[[ "$tool_ok" -eq 1 ]] || failed+=("tool")

# Slash palette. Does not change the exit status.
if tmux has-session -t "$SESSION" 2>/dev/null; then
  tmux send-keys -t "$SESSION" -l -- "/"
  sleep 2
  slash_cap="$(capture_visible)"
  printf '%s\n' "$slash_cap" >"${OUT_DIR}/slash.txt"
  : >"${OUT_DIR}/slash-commands.txt"
  while IFS= read -r line; do
    # Paths like /tmp and /home are not slash commands.
    if [[ "$line" =~ (^|[[:space:]│])(/[a-z][a-z0-9-]{1,24})([^[:alnum:]/]|$) ]]; then
      cmd="${BASH_REMATCH[2]}"
      case "$cmd" in
        /tmp | /home | /data | /usr | /var | /opt | /etc) continue ;;
      esac
      echo "$cmd" >>"${OUT_DIR}/slash-commands.txt"
    fi
  done <<<"$slash_cap"
  if [[ -s "${OUT_DIR}/slash-commands.txt" ]]; then
    slash_ok=1
  fi
  tmux send-keys -t "$SESSION" Escape
  sleep 0.3
fi

# session/load via --continue. Does not change the exit status.
# scrollback.txt already holds the live turn; resume.txt is separate.
if tmux has-session -t "$SESSION" 2>/dev/null; then
  tmux send-keys -t "$SESSION" C-q
  sleep 0.6
  tmux send-keys -t "$SESSION" C-q
  sleep 2
fi
tmux kill-session -t "$SESSION" 2>/dev/null || true
launch_pager --continue

RESUME_DEADLINE=$((SECONDS + 90))
resume_cap=""
while ((SECONDS < RESUME_DEADLINE)); do
  resume_cap="$(capture_scrollback)"
  if composer_ready "$resume_cap" && [[ "$resume_cap" == *"$NONCE"* ]]; then
    resume_ok=1
    break
  fi
  if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    break
  fi
  sleep 2
done
printf '%s\n' "$resume_cap" >"${OUT_DIR}/resume.txt"

write_result_env
if [[ -f "$DEBUG_FILE" ]] && grep -q 'session/load' "$DEBUG_FILE"; then
  echo "debug_session_load=1" >>"${OUT_DIR}/result.env"
else
  echo "debug_session_load=0" >>"${OUT_DIR}/result.env"
fi

echo "${AGENT} launch=${launch_ok} prompt=${prompt_ok} assistant=${assistant_ok} tool=${tool_ok} slash=${slash_ok} resume=${resume_ok} pane=${PANE_PATH}"

tmux kill-session -t "$SESSION" 2>/dev/null || true

if ((${#failed[@]})); then
  echo "Checks failed: ${failed[*]}" >&2
  exit 1
fi
exit 0
