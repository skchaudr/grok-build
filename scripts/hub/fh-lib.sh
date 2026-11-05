# Shared by fh, fh-stop, fh-clean, gb-build. Sourced, not run.
# Hosts: "local" = this machine; anything else is an ssh alias.
FH_HOSTS=${FH_HOSTS:-"local sab-air sab-mini"}

# run HOST 'script'  — run a bash snippet on HOST (stdin untouched).
fh_run() {
  if [ "$1" = local ]; then
    bash -c "$2"
  else
    ssh -o BatchMode=yes -o ConnectTimeout=6 "$1" "bash -c $(printf %q "$2")"
  fi
}

# Paths on every machine; the PATH line makes cargo/brew visible in non-interactive ssh.
FH_ENV='export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.local/bin:$PATH"'
