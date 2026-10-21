#!/bin/sh
# Step 0 payload capture (SPEC-live): save one hook payload plus the hook's process chain.
# Usage in a hook config: capture.sh <source> [event]. Prints only the neutral reply Cursor needs; always exits 0.
# Output stays local in ~/.grok/traces/capture/<source>/ — it holds real session text, never commit it.
src="${1:-unknown}"
dir="$HOME/.grok/traces/capture/$src"
mkdir -p "$dir" 2>/dev/null || exit 0
f="$dir/$(date +%Y%m%dT%H%M%S)-$$-${2:-event}.txt"
{
  cat
  printf '\n--- process chain (pid ppid lstart comm) ---\n'
  p=$$
  while [ -n "$p" ] && [ "$p" -gt 1 ] 2>/dev/null; do
    ps -o pid=,ppid=,lstart=,comm= -p "$p" 2>/dev/null || break
    p=$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')
  done
} >"$f" 2>/dev/null
# Cursor's gating events need an explicit neutral reply (MAP.md, Cursor stdout contract).
case "$src:${2:-}" in
  cursor:beforeSubmitPrompt) printf '{"continue":true}\n' ;;
  cursor:preToolUse|cursor:beforeShellExecution|cursor:beforeMCPExecution|cursor:beforeReadFile) printf '{"permission":"allow"}\n' ;;
  cursor:*) printf '{}\n' ;;
esac
exit 0
