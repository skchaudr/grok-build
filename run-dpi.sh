#!/usr/bin/env bash
# grok-pi attached to a durable dpi-backend agent instead of a local Pi.
#
#   ./run-dpi.sh [project-dir]                 continue the dpi agent you last attached to
#   ./run-dpi.sh [project-dir] --session NAME  attach to a session (name, id, dpi://<id>/<agent>)
#
# DPI_SESSION / DPI_AGENT env also pick the target. ctrl+r lists every dpi agent.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${GROK_PI_BIN:-$ROOT/target/debug/grok-pi}"
[[ -x "$BIN" ]] || { echo "error: grok-pi is not built: $BIN (run ./build.sh)" >&2; exit 1; }

PROJECT_DIR="$HOME/dpi-work"
if [[ $# -gt 0 && "$1" != -* ]]; then PROJECT_DIR="$1"; shift; fi
PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd)"

export GROK_PI_DPI=1
# Without --session, attach like --continue so history replays instead of Welcome.
flags=("$@")
[[ " ${flags[*]-} " == *" --session "* || " ${flags[*]-} " == *" --continue "* ]] || flags+=(--continue)

exec "$BIN" --pi-bin "$ROOT/tools/dpi-rpc/dpi-rpc.mjs" --pi-cwd "$PROJECT_DIR" "${flags[@]}"
