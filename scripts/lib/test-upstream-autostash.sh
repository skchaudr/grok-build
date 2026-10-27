#!/usr/bin/env bash
# Tests for scripts/lib/upstream-autostash.sh.
# Prints one line per assertion and exits non-zero if any fail.
set -u

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=upstream-autostash.sh
source "$ROOT/scripts/lib/upstream-autostash.sh"

failures=0
tmps=()

cleanup() {
  local d
  if ((${#tmps[@]})); then
    for d in "${tmps[@]}"; do
      rm -rf "$d"
    done
  fi
}
trap cleanup EXIT

check() {
  local desc="$1"
  shift
  if "$@"; then
    echo "PASS $desc"
  else
    echo "FAIL $desc"
    failures=$((failures + 1))
  fi
}

is_sha40() { [[ "$1" =~ ^[0-9a-f]{40}$ ]]; }
porcelain_empty() { [[ -z "$(git status --porcelain)" ]]; }
stash_empty() { [[ -z "$(git stash list)" ]]; }
stash_contains() { git stash list --format='%H' | grep -qx "$1"; }
lists_name() { grep -Fxq "$1" <<<"$2"; }
no_conflict_markers() { ! grep -q '<<<<<<<' "$1"; }
warns_about() { grep -F -q "$1" <<<"$2"; }
silent_about() { ! grep -F -q "$1" <<<"$2"; }

new_repo() {
  local d
  d=$(mktemp -d)
  tmps+=("$d")
  git init -q -b main "$d"
  git -C "$d" config user.email "autostash-test@example.com"
  git -C "$d" config user.name "autostash-test"
  git -C "$d" config commit.gpgsign false
  cd "$d"
}

# 1. Clean tree: nothing printed, success, no stash.
new_repo
git commit -q --allow-empty -m init
out=$(autostash_if_needed 2>/dev/null)
rc=$?
check "clean: autostash_if_needed returns 0" test "$rc" -eq 0
check "clean: autostash_if_needed prints nothing" test -z "$out"
check "clean: git stash list is empty" stash_empty

# 2. Dirty tracked file plus an untracked file, then restore.
new_repo
printf 'base\n' > tracked.txt
git add tracked.txt
git commit -q -m init
printf 'dirty\n' > tracked.txt
printf 'secret\n' > untracked.txt
out=$(autostash_if_needed 2>/dev/null)
rc=$?
check "dirty: autostash_if_needed prints a 40-char SHA" is_sha40 "$out"
check "dirty: autostash_if_needed returns 0" test "$rc" -eq 0
check "dirty: tree is clean after autostash" porcelain_empty
shown=$(git stash show --include-untracked --name-only "$out")
check "dirty: stash lists the untracked file" lists_name untracked.txt "$shown"
restore_autostash "$out" >/dev/null 2>&1
rc=$?
check "dirty: restore_autostash returns 0" test "$rc" -eq 0
check "dirty: tracked file is back" cmp -s tracked.txt <(printf 'dirty\n')
check "dirty: untracked file is back" cmp -s untracked.txt <(printf 'secret\n')
check "dirty: stash entry was dropped" stash_empty

# 3. Unmerged index from a real merge conflict: refuse, create no stash.
new_repo
printf 'base\n' > F
git add F
git commit -q -m base
git checkout -q -b other
printf 'other\n' > F
git commit -q -am other
git checkout -q main
printf 'main\n' > F
git commit -q -am main
git merge --no-edit other >/dev/null 2>&1 || true
out=$(autostash_if_needed 2>/dev/null)
rc=$?
check "unmerged: autostash_if_needed returns 2" test "$rc" -eq 2
check "unmerged: creates no stash" stash_empty
check "unmerged: prints no SHA" test -z "$out"

# 4. Restore that conflicts is wiped; HEAD and the stash entry stay.
new_repo
printf 'alpha\nbeta\ngamma\n' > F
git add F
git commit -q -m base
printf 'alpha\nSTASH\ngamma\n' > F
git stash push -q -m case4
sha=$(git rev-parse --verify refs/stash)
printf 'alpha\nCOMMIT\ngamma\n' > F
git add F
git commit -q -m committed
head=$(git rev-parse HEAD)
restore_autostash "$sha" >/dev/null 2>&1
rc=$?
check "conflict: restore_autostash returns 1" test "$rc" -eq 1
check "conflict: working tree is clean" porcelain_empty
check "conflict: no conflict markers in F" no_conflict_markers F
check "conflict: HEAD still holds the committed change" test "$(git rev-parse HEAD)" = "$head"
check "conflict: F matches the committed change" cmp -s F <(printf 'alpha\nCOMMIT\ngamma\n')
check "conflict: stash entry still exists" stash_contains "$sha"

# 5. Stale message date warns; a same-day entry does not.
new_repo
printf 'base\n' > F
git add F
git commit -q -m base
printf 'old\n' > F
git stash push -q -m grok-upstream-autostash-20000101-000000
old=$(git rev-parse --verify refs/stash)
printf 'fresh\n' > F
today=$(date -u +%Y%m%d-%H%M%S)
git stash push -q -m "grok-upstream-autostash-${today}"
fresh=$(git rev-parse --verify refs/stash)
out=$(warn_stale_autostash 7 2>/dev/null)
check "stale: warns naming the old stash" warns_about "$old" "$out"
check "stale: fresh entry produces no warning line" silent_about "$fresh" "$out"

exit $((failures > 0))
