#!/usr/bin/env bash
# Autostash helpers for scripts/upstream-sync.sh.
#
# Stash uncommitted work before an upstream merge and restore it afterwards.
# A conflicted restore is reset to HEAD and the stash is kept. Stale-entry
# warnings are printed only; nothing is dropped. Safe to source: no set -e,
# no side effects at source time, set -u tolerant.
#
#   source scripts/lib/upstream-autostash.sh
#   sha=$(autostash_if_needed) || exit $?   # stdout: stash SHA, or empty if clean
#   restore_autostash "$sha"                # apply and drop; on conflict, keep it
#   warn_stale_autostash                    # stdout: entries older than 7 days
#   warn_stale_autostash 14

# Print the new stash SHA, or nothing when the worktree is clean.
# Returns 2 when a merge is in progress or the index has unmerged paths.
autostash_if_needed() {
  local dirty=""
  dirty=$(git status --porcelain) || return 1
  if [[ -z "$dirty" ]]; then
    return 0
  fi
  if [[ -n "$(git ls-files --unmerged)" ]] || git rev-parse -q --verify MERGE_HEAD >/dev/null; then
    echo "refusing to stash a conflicted/in-progress merge" >&2
    return 2
  fi
  local msg=""
  msg="grok-upstream-autostash-$(date -u +%Y%m%d-%H%M%S)"
  if ! git stash push --include-untracked -m "$msg" >&2; then
    return 1
  fi
  local sha=""
  sha=$(git rev-parse --verify refs/stash) || return 1
  printf '%s\n' "$sha"
}

# Apply a stash commit and drop it. On conflict, reset to HEAD and keep the stash.
restore_autostash() {
  local sha="${1:-}"
  if [[ -z "$sha" ]]; then
    echo "restore_autostash: missing stash sha" >&2
    return 2
  fi
  local apply_rc=0
  git stash apply "$sha" >&2 || apply_rc=$?
  local conflicts=""
  conflicts=$(git diff --name-only --diff-filter=U || true)
  if [[ "$apply_rc" -ne 0 || -n "$conflicts" ]]; then
    # Wipes the failed apply. HEAD stays on the current commit (the merge).
    if ! git reset --hard HEAD >&2; then
      echo "autostash: git reset --hard HEAD failed; the stash was not dropped" >&2
    fi
    if [[ -n "$conflicts" ]]; then
      printf '%s\n' "$conflicts" >&2
    fi
    printf 'stash preserved: %s — restore later with: git stash apply %s\n' "$sha" "$sha" >&2
    return 1
  fi
  local line="" sel="" entry_sha="" found=""
  while IFS= read -r line || [[ -n "${line:-}" ]]; do
    [[ -n "$line" ]] || continue
    sel=${line%% *}
    entry_sha=${line#* }
    if [[ "$entry_sha" == "$sha" ]]; then
      found=$sel
      break
    fi
  done < <(git stash list --format='%gd %H')
  if [[ -z "$found" ]]; then
    echo "autostash: applied $sha but no stash entry matched it; left it in place. Clean up with: git stash list" >&2
    return 0
  fi
  if ! git stash drop "$found" >&2; then
    echo "autostash: applied $sha but could not drop $found; left it in place. Drop it manually: git stash drop $found" >&2
    return 0
  fi
  return 0
}

# Print stale grok-upstream-autostash-* entries older than N days (default 7).
warn_stale_autostash() {
  local days="${1:-7}"
  if [[ ! "$days" =~ ^[0-9]+$ ]]; then
    echo "warn_stale_autostash: days must be a non-negative integer" >&2
    return 2
  fi
  days=$((10#$days))
  local now="" gnu_date=0
  now=$(date -u +%s) || return 1
  if date --version >/dev/null 2>&1; then
    gnu_date=1
  fi
  local line="" sel="" rest="" sha="" gs="" ymd="" hms="" iso="" when="" elapsed=0 age=0
  while IFS= read -r line || [[ -n "${line:-}" ]]; do
    [[ -n "$line" ]] || continue
    sel=${line%% *}
    rest=${line#* }
    sha=${rest%% *}
    gs=${rest#* }
    [[ "$gs" =~ grok-upstream-autostash-([0-9]{8})-([0-9]{6}) ]] || continue
    ymd=${BASH_REMATCH[1]}
    hms=${BASH_REMATCH[2]}
    iso="${ymd:0:4}-${ymd:4:2}-${ymd:6:2} ${hms:0:2}:${hms:2:2}:${hms:4:2}"
    if [[ "$gnu_date" -eq 1 ]]; then
      when=$(date -u -d "$iso UTC" +%s 2>/dev/null) || continue
    else
      when=$(date -u -j -f '%Y-%m-%d %H:%M:%S' "$iso" +%s 2>/dev/null) || continue
    fi
    elapsed=$((now - when))
    (( elapsed > days * 86400 )) || continue
    age=$((elapsed / 86400))
    printf '%s %s %s days — Review: git stash show -p %s | Restore: git stash apply %s | Discard: git stash drop %s\n' \
      "$sel" "$sha" "$age" "$sel" "$sha" "$sel"
  done < <(git stash list --format='%gd %H %gs')
}
