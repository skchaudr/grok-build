# grok-picker.zsh — bare `grok` is normal Grok. Named variants stay explicit.
# Source from ~/.config/zsh/local.zsh:  source ~/.grok-hub/scripts/hub/grok-picker.zsh
#   grok                 -> normal Grok (stock binary). No pickers.
#   grok "prompt"        -> same, prompt and flags included
#   grok pi              -> Pi in the fork pager
#   grok grok-pi         -> Pi through the fork's own grok-pi-acp
#   grok claude          -> Claude in the fork pager
#   grok fork            -> fork pager hosting `grok agent stdio` (was the picker id `grok`)
#   grok all             -> one fork pager offering claude and pi (GB_ALL); `grok claude pi` too
#   grok +khoj ...       -> same, plus remote machine choices
#   grok <agent> [...]   -> that agent; remaining args go to the pager
#   grok normal|native   -> same binary as bare `grok`
#   grok pick            -> machine picker, then agent picker (local -> gb; remote -> gk)
#   grok --resume X      -> gb finds which agent owns session X, then resumes it
#   grok --resume=X      -> same as --resume X
#   grok -r X            -> same as --resume
#   grok -- <args>       -> fork pager with raw args (e.g. --leader --leader-socket ...)
# gb is the copy next to this file (a worktree source uses that worktree's gb).
unalias grok 2>/dev/null
typeset -g GROK_PICKER_DIR="${${(%):-%x}:A:h}"
function grok {
  local gb="${GROK_PICKER_DIR}/gb" entry="${GROK_PICKER_DIR}/grok-entry.sh"
  local stock="${GB_NATIVE:-$HOME/.grok/bin/grok}"
  case "${1:-}" in
    normal|native)
            shift
            "$stock" "$@"
            ;;
    pick)   shift
            local machine agent
            machine=$("$entry" pick-machine) || return 0
            agent=$("$entry" pick-agent) || return 0
            "$entry" launch "$machine" "$agent" "$@"
            ;;
    all|+?*|fork|claude|cursor|pi|grok-pi|codex|dsh|dsh-stock|prime)
            "$gb" "$@"
            ;;
    --)     shift; env -u PAGER "$(GB_PRINT_PAGER=1 "$gb")" "$@" ;;
    --resume=*|--resume|-r) "$gb" "$@" ;;
    *)      "$stock" "$@" ;;
  esac
}
