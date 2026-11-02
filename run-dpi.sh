#!/usr/bin/env bash
# grok-pi attached to a durable dpi-backend agent instead of a local Pi.
# usage: DPI_SESSION=<name|id> [DPI_AGENT=main] ./run-dpi.sh [project-dir]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PI_BIN="$ROOT/tools/dpi-rpc/dpi-rpc.mjs"
export GROK_PI_DPI=1
# Remote TUI is a Pi extension; the durable agent does not load Pi extensions.
export PI_GROK_REMOTE_TUI=0
exec "$ROOT/run-local.sh" "${1:-$HOME/dpi-work}" "${@:2}"
