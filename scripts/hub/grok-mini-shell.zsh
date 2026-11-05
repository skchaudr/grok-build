# Air-local integration: source after hermes-proxies.zsh, outside shared dotfiles.
# Preserve the existing explicit legacy-host route for rollback.
if (( ${+functions[gk-tunnel-legacy]} == 0 && ${+functions[gk-tunnel]} )); then
  functions[gk-tunnel-legacy]=$functions[gk-tunnel]
fi
function gk-tunnel {
  local name="${1:-mini}"
  if [[ "$name" != mini ]]; then
    gk-tunnel-legacy "$@"
    return
  fi
  while true; do
    "$HOME/.dsh/scripts/grok-mini-tunnel"
    print -r -- "[$(date +%H:%M:%S)] Mini hub link down (exit $?); reconnecting in 5s. Ctrl-C stops it."
    sleep 5
  done
}
unalias gk 2>/dev/null
function gk {
  "$HOME/.dsh/scripts/gk" "$@"
}
