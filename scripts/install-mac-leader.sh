#!/bin/sh
# Install the fork leader as a named, always-on launchd agent on a Mac.
#
# Idempotent: re-running with nothing changed and the leader already listening
# is a no-op. Run it ON the Mac (it never builds anything):
#   sh scripts/install-mac-leader.sh
#   ssh sab-mini@<mac> 'sh -s' < scripts/install-mac-leader.sh
#
# What it installs:
#   ~/.local/bin/grok-leader                       named wrapper (the item macOS shows)
#   ~/Library/LaunchAgents/com.sab.grok-leader.plist  ProgramArguments[0] = the wrapper,
#                                                  never an interpreter with -c
# The leader listens on ~/.grok/leader-fork.sock, beside any stock leader that owns
# ~/.grok/leader.sock.
#
# KeepAlive is plain `true`, not {SuccessfulExit=false}: the leader exits 0 on SIGTERM
# (ShutdownReason::Manual), so a SuccessfulExit=false job is never restarted after a
# `pkill`, a stray `leader stop` or a socket sweep. A deliberate stop is
# `launchctl bootout gui/$(id -u)/com.sab.grok-leader`.
#
# Env: FORCE=1 restarts even when sessions are attached.
set -eu

LABEL=com.sab.grok-leader
BIN="$HOME/.grok/bin/grok-fork"
WRAPPER="$HOME/.local/bin/grok-leader"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
SOCK="$HOME/.grok/leader-fork.sock"
DOMAIN="gui/$(id -u)"
STAMP=$(date +%Y%m%d-%H%M%S)

say() { printf 'install-mac-leader: %s\n' "$*"; }
die() { printf 'install-mac-leader: FAIL: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "not a Mac"
[ -x "$BIN" ] || die "$BIN missing or not executable (copy the Darwin fork pager there first; this script never builds)"
mkdir -p "$HOME/.local/bin" "$HOME/Library/LaunchAgents"

WRAPPER_NEW=$(mktemp "${TMPDIR:-/tmp}/grok-leader.XXXXXX")
PLIST_NEW=$(mktemp "${TMPDIR:-/tmp}/grok-leader-plist.XXXXXX")
trap 'rm -f "$WRAPPER_NEW" "$PLIST_NEW"' EXIT

cat >"$WRAPPER_NEW" <<'EOF'
#!/bin/sh
# Grok Build fork leader: clear a stale fork socket, then become grok-fork.
export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
SOCK="$HOME/.grok/leader-fork.sock"
rm -f "$SOCK"
exec "$HOME/.grok/bin/grok-fork" agent leader --no-exit-on-disconnect --no-auto-update --leader-socket "$SOCK"
EOF

cat >"$PLIST_NEW" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$WRAPPER</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>/tmp/grok-leader.out</string>
  <key>StandardErrorPath</key><string>/tmp/grok-leader.err</string>
</dict>
</plist>
EOF

plutil -lint "$PLIST_NEW" >/dev/null || die "generated plist does not lint"
first_arg=$(/usr/libexec/PlistBuddy -c 'Print :ProgramArguments:0' "$PLIST_NEW")
case "$first_arg" in
  */sh | */zsh | */bash | */herdr | */env) die "ProgramArguments[0] is an interpreter ($first_arg); macOS would name the item after it" ;;
esac

# Back up anything we are about to overwrite; leave identical files alone.
changed=0
install_file() { # src dst mode
  if [ -e "$2" ] && cmp -s "$1" "$2"; then return 0; fi
  if [ -e "$2" ]; then cp -p "$2" "$2.bak-$STAMP"; say "backed up $2 -> $2.bak-$STAMP"; fi
  cp "$1" "$2"; chmod "$3" "$2"; changed=1; say "wrote $2"
}
install_file "$WRAPPER_NEW" "$WRAPPER" 755
install_file "$PLIST_NEW" "$PLIST" 644
sh -n "$WRAPPER" || die "wrapper has a syntax error"

# True when a grok-fork process holds a listening fork socket.
listening() {
  /usr/sbin/lsof -nP -U 2>/dev/null | awk -v s="$SOCK" '$1 ~ /^grok-fork/ && index($0, s) {f=1} END {exit !f}'
}
loaded() { launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; }

if loaded && [ "$changed" = 0 ] && listening; then
  say "already installed, unchanged and listening on $SOCK; nothing to do"
else
  if loaded; then
    sessions=$(tr -d ' \n[]' <"$HOME/.grok/active_sessions.json" 2>/dev/null || true)
    if [ -n "$sessions" ] && [ "${FORCE:-0}" != 1 ]; then
      die "sessions are attached (active_sessions.json non-empty); re-run with FORCE=1 to restart anyway"
    fi
    if [ "$changed" = 1 ]; then
      launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
      sleep 1
      launchctl bootstrap "$DOMAIN" "$PLIST" || die "launchctl bootstrap failed"
    else
      launchctl kickstart -k "$DOMAIN/$LABEL" || die "launchctl kickstart failed"
    fi
  else
    launchctl bootstrap "$DOMAIN" "$PLIST" || die "launchctl bootstrap failed"
  fi
  i=0
  while [ "$i" -lt 20 ]; do
    listening && break
    i=$((i + 1)); sleep 1
  done
  listening || die "grok-fork is not listening on $SOCK after 20s (see /tmp/grok-leader.err)"
fi

pid=$(launchctl print "$DOMAIN/$LABEL" 2>/dev/null | awk '/^[[:space:]]*pid = /{print $3; exit}')
say "OK: $LABEL pid=${pid:-?} listening on $SOCK"
pgrep -fl 'grok-fork agent leader' || true
