#!/bin/zsh
# Build grok-fork and grok-traces in one cargo run, install both on this Mac and on sab-air, and check the checksums.
# grok-traces is the hook/indexer binary the harness hooks run; it shares the index with grok-fork, so it ships with it.
#   scripts/install-fork.sh            build, then install
#   scripts/install-fork.sh --no-build install what target/release already holds
set -eu
cd "${0:A:h}/.."
REMOTE=${GROK_FORK_REMOTE:-sab-mini@sab-air}

if [[ ${1:-} != --no-build ]]; then
  if pgrep -x cargo >/dev/null; then echo "cargo is already running; not starting a second build" >&2; exit 1; fi
  nice -n 10 cargo build --release -p xai-grok-pager-bin -p xai-grok-traces
fi

# Copy then rename: copying over a running binary kills the sessions using it.
typeset -A bins=(xai-grok-pager grok-fork grok-traces grok-traces)
fail=0
for src dst in ${(kv)bins}; do
  cp "target/release/$src" ~/.grok/bin/$dst.new && mv ~/.grok/bin/$dst.new ~/.grok/bin/$dst
  rsync -a "target/release/$src" "$REMOTE:.grok/bin/$dst.new"
  ssh "$REMOTE" "mv ~/.grok/bin/$dst.new ~/.grok/bin/$dst"
  want=$(shasum -a 256 "target/release/$src" | cut -d' ' -f1)
  here=$(shasum -a 256 ~/.grok/bin/$dst | cut -d' ' -f1)
  there=$(ssh "$REMOTE" "shasum -a 256 ~/.grok/bin/$dst" | cut -d' ' -f1)
  if [[ $here == $want && $there == $want ]]; then
    echo "$dst ${want[1,12]} matches here and on $REMOTE"
  else
    echo "$dst MISMATCH: built ${want[1,12]} here ${here[1,12]} $REMOTE ${there[1,12]}" >&2
    fail=1
  fi
done
exit $fail
