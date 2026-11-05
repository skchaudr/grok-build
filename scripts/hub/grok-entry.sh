#!/usr/bin/env bash
# grok-entry.sh — machine + agent pickers for `grok pick` (sourced by grok-picker.zsh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GK="${GK_SCRIPT:-$ROOT/gk}"
GB="${GB_SCRIPT:-$ROOT/gb}"

trace_step() {
  if [[ -n ${GROK_PICKER_TRACE:-} ]]; then
    echo "$1" >>"$GROK_PICKER_TRACE"
  fi
}

machine_names() {
  printf 'local\n'
  "$GK" workers
}

pick_machine() {
  trace_step machine
  local pick
  if [[ -n ${GROK_MACHINE_PICK:-} ]]; then
    pick=$GROK_MACHINE_PICK
  else
    pick=$(machine_names | fzf --height=~40% --reverse --prompt='machine > ' \
      --header='where to run the agent' | awk '{print $1}') || return 1
  fi
  [[ -n $pick ]] || return 1
  printf '%s' "$pick"
}

pick_agent() {
  trace_step agent
  GB_PICK_ONLY=1 "$GB"
}

launch() {
  local machine=$1 agent=$2
  shift 2
  local extra=("$@")
  if [[ $machine == local ]]; then
    exec "$GB" "$agent" "${extra[@]}"
  fi
  export GK_FALLBACK_AGENT="$agent"
  export GK_FALLBACK_GB="$GB"
  exec "$GK" "$machine" "${extra[@]}"
}

case ${1:-} in
  pick-machine) pick_machine ;;
  pick-agent) pick_agent ;;
  launch) launch "${@:2}" ;;
  *)
    echo "grok-entry: usage: pick-machine | pick-agent | launch MACHINE AGENT [args...]" >&2
    exit 2
    ;;
esac
