#!/usr/bin/env bash
# Fetch, merge, verify, and record an upstream dump of this fork.
# Does not edit pager source. Never runs cargo clean.
set -euo pipefail

usage() {
  echo "Usage: scripts/upstream-sync.sh <check|start|verify|finish>" >&2
  echo "  check [--replay <a>..<b>]   fetch upstream; stat overlap with files touched since last sync" >&2
  echo "  start                       branch sync/upstream-<date> from main and merge upstream" >&2
  echo "  verify                      release pager build and pager lib tests, or refuse under 20G free" >&2
  echo "  finish                      merge --no-ff into main, pull before push, record the upstream hash" >&2
  exit 2
}

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"

LAST_FILE="$ROOT/scripts/upstream-sync.rev"
OBS="$ROOT/OBSERVATIONS.md"
REMOTE="${UPSTREAM_SYNC_REMOTE:-upstream}"
UP_REF="${UPSTREAM_SYNC_REF:-${REMOTE}/main}"
MAIN_REF="${UPSTREAM_SYNC_MAIN:-main}"
PUSH_REMOTE="${UPSTREAM_SYNC_PUSH_REMOTE:-origin}"
DATE_UTC=$(date -u +%Y-%m-%d)
BRANCH="${UPSTREAM_SYNC_BRANCH:-sync/upstream-${DATE_UTC}}"
MIN_FREE_KB=$((20 * 1024 * 1024))

die() {
  echo "upstream-sync: $*" >&2
  exit 1
}

read_last() {
  [[ -f "$LAST_FILE" ]] || die "missing tracked last-sync file $LAST_FILE"
  local last
  last=$(awk 'NF && $1 !~ /^#/ { print $1; exit }' "$LAST_FILE")
  git rev-parse --verify "${last}^{commit}" >/dev/null
  echo "$last"
}

fetch_upstream() {
  if [[ "${UPSTREAM_SYNC_SKIP_FETCH:-}" == 1 ]]; then
    echo "fetch: skipped"
    return
  fi
  git fetch "$REMOTE"
}

require_clean() {
  local dirty
  dirty=$(git status --porcelain)
  [[ -z "$dirty" ]] || die "worktree is dirty; commit or stash before $1"
}

unmerged_files() {
  git diff --name-only --diff-filter=U
}

hunks_between() {
  local left=$1 right=$2
  local have_left=0 have_right=0
  git cat-file -e "$left" 2>/dev/null && have_left=1
  git cat-file -e "$right" 2>/dev/null && have_right=1
  if [[ $have_left -eq 0 && $have_right -eq 0 ]]; then
    echo 0
    return
  fi
  if [[ $have_left -eq 0 || $have_right -eq 0 ]]; then
    echo 1
    return
  fi
  git diff -U0 "$left" "$right" | grep -c '^@@' || true
}

conflict_regions() {
  local file=$1
  if [[ ! -f "$file" ]]; then
    echo 0
    return
  fi
  grep -a -c '^<<<<<<<' "$file" || true
}

write_obs() {
  local body=$1
  if [[ -f "$OBS" ]] && grep -a -q '^<<<<<<<' "$OBS"; then
    local side="$ROOT/upstream-sync-conflict-summary.md"
    printf '%s\n' "$body" >>"$side"
    echo "OBSERVATIONS.md is in conflict; summary appended to $side"
    return
  fi
  printf '%s\n' "$body" >>"$OBS"
  echo "wrote $(basename "$OBS")"
}

# git diff here rejects --pathspec-from-file (exit 129), so path lists travel as argv.
read_nul_paths() {
  local -n _out=$1
  local file=$2
  _out=()
  local p
  while IFS= read -r -d '' p; do
    [[ -n "$p" ]] && _out+=("$p")
  done <"$file"
}

diff_stat_restricted() {
  local left=$1 right=$2
  shift 2
  if [[ $# -eq 0 ]]; then
    return 0
  fi
  git diff --stat "$left" "$right" -- "$@"
}

overlap_stat() {
  local left=$1 right=$2 list_base=$3 list_tip=$4
  local zlist count
  zlist=$(mktemp)
  git diff --name-only -z "$list_base" "$list_tip" >"$zlist"
  count=$(git diff --name-only "$list_base" "$list_tip" | grep -c . || true)
  echo "touched-files: $count ($list_base..$list_tip)"
  git diff --name-only "$list_base" "$list_tip"
  echo "overlap-stat: git diff --stat ${left}..${right} restricted to those files"
  local paths=()
  read_nul_paths paths "$zlist"
  rm -f "$zlist"
  if [[ ${#paths[@]} -eq 0 ]]; then
    echo "(no touched files)"
    return
  fi
  local stat_out
  stat_out=$(diff_stat_restricted "$left" "$right" "${paths[@]}")
  if [[ -z "$stat_out" ]]; then
    echo "(empty)"
  else
    printf '%s\n' "$stat_out"
  fi
}

cmd_check() {
  local replay=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --replay)
        [[ $# -ge 2 ]] || usage
        replay=$2
        shift 2
        ;;
      --replay=*)
        replay=${1#--replay=}
        shift
        ;;
      *)
        usage
        ;;
    esac
  done

  fetch_upstream
  local last up
  last=$(read_last)
  up=$(git rev-parse --verify "${UP_REF}^{commit}")
  echo "last-synced: $last"
  echo "upstream: $up ($UP_REF)"
  if [[ "$up" == "$last" ]]; then
    echo "moved: no (upstream has not moved past last-synced)"
  elif git merge-base --is-ancestor "$last" "$up"; then
    echo "moved: yes ($(git rev-list --count "$last".."$up") commit(s) $last..$up)"
  else
    echo "moved: diverged ($last is not an ancestor of $up)"
  fi

  echo "---- touched files and overlap stat ----"
  overlap_stat "$last" "$up" "$last" "$MAIN_REF"

  if [[ -n "$replay" ]]; then
    [[ "$replay" == *..* && "$replay" != *...* ]] || die "replay range must be A..B"
    local base=${replay%%..*} tip=${replay#*..}
    base=$(git rev-parse --verify "${base}^{commit}")
    tip=$(git rev-parse --verify "${tip}^{commit}")
    local zlist
    zlist=$(mktemp)
    git diff --name-only -z "$base" "$tip" >"$zlist"
    local paths=()
    read_nul_paths paths "$zlist"
    rm -f "$zlist"
    local n_names=${#paths[@]}
    if [[ $n_names -eq 0 ]]; then
      echo "---- replay ${base:0:12}..${tip:0:12} ----"
      echo "replay-touched-files: 0"
      echo "replay-restricted-files: 0"
      echo "replay-stat: match"
      return
    fi
    local full restricted
    full=$(mktemp)
    restricted=$(mktemp)
    git diff --stat "$base" "$tip" >"$full"
    diff_stat_restricted "$base" "$tip" "${paths[@]}" >"$restricted"
    local n_restricted
    n_restricted=$(git diff --name-only "$base" "$tip" -- "${paths[@]}" | grep -c . || true)
    echo "---- replay ${base:0:12}..${tip:0:12} ----"
    echo "replay-touched-files: $n_names"
    echo "replay-restricted-files: $n_restricted"
    if cmp -s "$full" "$restricted" && [[ "$n_names" == "$n_restricted" ]]; then
      echo "replay-stat: match"
    else
      echo "replay-stat: mismatch" >&2
      rm -f "$full" "$restricted" "$zlist"
      exit 1
    fi
    tail -n 1 "$full"
    rm -f "$full" "$restricted" "$zlist"
  fi
}

summarize_conflicts() {
  local files=() file
  while IFS= read -r file; do
    [[ -n "$file" ]] || continue
    files+=("$file")
  done < <(unmerged_files)

  local ours=0 theirs=0 regions=0
  local rows="" file_ours file_theirs file_regions
  for file in "${files[@]}"; do
    file_ours=$(hunks_between ":1:${file}" ":2:${file}")
    file_theirs=$(hunks_between ":1:${file}" ":3:${file}")
    file_regions=$(conflict_regions "$file")
    ours=$((ours + file_ours))
    theirs=$((theirs + file_theirs))
    regions=$((regions + file_regions))
    rows+="| ${file} | ${file_ours} | ${file_theirs} | ${file_regions} |"$'\n'
  done

  local when
  when=$(date -u +%Y-%m-%dT%H:%MZ)
  local up
  up=$(git rev-parse --verify "${UP_REF}^{commit}")
  local body
  body=$(
    cat <<EOF

## ${when} upstream-sync start

Branch \`${BRANCH}\` from \`${MAIN_REF}\` ($(git rev-parse --short "$MAIN_REF")).
Merged \`${UP_REF}\` (${up}).
${#files[@]} conflicted files, ours-hunks ${ours}, theirs-hunks ${theirs}, conflict-hunks ${regions}.
ours-hunks and theirs-hunks are \`git diff -U0\` hunks of each side against the merge base. conflict-hunks are \`<<<<<<<\` regions.

| file | ours-hunks | theirs-hunks | conflict-hunks |
| --- | ---: | ---: | ---: |
${rows}
EOF
  )
  printf '%s\n' "$body"
  write_obs "$body"
  echo "stopped: resolve conflicts, commit the merge, then verify and finish"
}

cmd_start() {
  [[ $# -eq 0 ]] || usage
  require_clean start
  if git rev-parse -q --verify MERGE_HEAD >/dev/null; then
    die "a merge is already in progress"
  fi
  fetch_upstream
  git rev-parse --verify "${UP_REF}^{commit}" >/dev/null
  git rev-parse --verify "${MAIN_REF}^{commit}" >/dev/null
  echo "start: checkout -B $BRANCH $MAIN_REF"
  git checkout -B "$BRANCH" "$MAIN_REF"
  set +e
  git merge --no-edit "$UP_REF"
  local merge_rc=$?
  set -e
  if [[ $merge_rc -eq 0 ]]; then
    local when up body
    when=$(date -u +%Y-%m-%dT%H:%MZ)
    up=$(git rev-parse --verify "${UP_REF}^{commit}")
    body=$(
      cat <<EOF

## ${when} upstream-sync start

Branch \`${BRANCH}\` from \`${MAIN_REF}\` merged \`${UP_REF}\` (${up}) with no conflicts.
HEAD $(git rev-parse --short HEAD).
EOF
    )
    printf '%s\n' "$body"
    write_obs "$body"
    exit 0
  fi
  if [[ -z "$(unmerged_files)" ]]; then
    die "merge failed without conflicted files (exit $merge_rc)"
  fi
  summarize_conflicts
  exit 1
}

cmd_verify() {
  [[ $# -eq 0 ]] || usage
  if [[ -n "$(unmerged_files)" ]]; then
    die "unmerged files remain; resolve them before verify"
  fi
  df -h /
  local avail_k free_gb
  avail_k=$(df -Pk / | awk 'NR==2 { print $4 }')
  [[ "$avail_k" =~ ^[0-9]+$ ]] || die "could not read free space on /"
  free_gb=$((avail_k / 1024 / 1024))
  if (( avail_k < MIN_FREE_KB )); then
    echo "refuse: / has ${free_gb}G free, need 20G" >&2
    exit 1
  fi
  echo "disk: ${free_gb}G free on / (floor 20G)"
  export CARGO_TARGET_DIR="$ROOT/target"
  export CARGO_INCREMENTAL=0
  if [[ -x "${HOME}/.cargo/bin/dotslash" ]]; then
    case ":$PATH:" in
      *":${HOME}/.cargo/bin:"*) ;;
      *) export PATH="${HOME}/.cargo/bin:${PATH}" ;;
    esac
  fi
  if ! command -v dotslash >/dev/null 2>&1 && [[ -x "${HOME}/.local/protoc-29.3/bin/protoc" ]]; then
    export PROTOC="${HOME}/.local/protoc-29.3/bin/protoc"
    export PATH="${HOME}/.local/protoc-29.3/bin:${PATH}"
  fi
  echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
  local build_log test_log build_rc test_rc
  build_log=$(mktemp)
  test_log=$(mktemp)
  set +e
  cargo build --release -p xai-grok-pager-bin 2>&1 | tee "$build_log"
  build_rc=${PIPESTATUS[0]}
  cargo test -p xai-grok-pager --lib 2>&1 | tee "$test_log"
  test_rc=${PIPESTATUS[0]}
  set -e
  local test_line
  test_line=$(grep -E '^test result:' "$test_log" | tail -n 1 || true)
  echo "verify: build=${build_rc} test=${test_rc} free=${free_gb}G target=${CARGO_TARGET_DIR}"
  if [[ -n "$test_line" ]]; then
    echo "$test_line"
  fi
  rm -f "$build_log" "$test_log"
  if [[ $build_rc -ne 0 || $test_rc -ne 0 ]]; then
    exit 1
  fi
}

worktree_for_branch() {
  local branch=$1
  git worktree list --porcelain | awk -v ref="refs/heads/${branch}" '
    /^worktree / { wt = substr($0, 10) }
    /^branch / { if ($2 == ref) print wt }
  '
}

record_hash_on_sync_branch() {
  git rev-parse --verify "${BRANCH}^{commit}" >/dev/null || die "missing sync branch $BRANCH"
  local current
  current=$(git rev-parse --abbrev-ref HEAD)
  if [[ "$current" != "$BRANCH" ]]; then
    git checkout "$BRANCH"
  fi
  if [[ -n "$(unmerged_files)" ]] || git rev-parse -q --verify MERGE_HEAD >/dev/null; then
    die "sync branch still has an unresolved merge"
  fi
  local merge_commit up
  merge_commit=$(git log --merges -1 --format=%H "$BRANCH" "^${MAIN_REF}" || true)
  if [[ -n "$merge_commit" ]]; then
    up=$(git rev-parse "${merge_commit}^2")
  else
    up=$(git rev-parse --verify "${UP_REF}^{commit}")
  fi
  printf '%s\n' "$up" >"$LAST_FILE"
  git add -- "$LAST_FILE"
  if git diff --cached --quiet; then
    echo "last-synced already $up"
    return
  fi
  git commit -m "$(cat <<EOF
Record last upstream sync ${up:0:12}.

EOF
)"
  echo "recorded last-synced $up"
}

merge_into_main_at() {
  local wt=$1
  echo "finish: pull --ff-only $PUSH_REMOTE $MAIN_REF in $wt"
  git -C "$wt" pull --ff-only "$PUSH_REMOTE" "$MAIN_REF"
  echo "finish: merge --no-ff $BRANCH into $MAIN_REF"
  git -C "$wt" merge --no-ff --no-edit "$BRANCH"
  echo "finish: push $PUSH_REMOTE $MAIN_REF"
  git -C "$wt" push "$PUSH_REMOTE" "$MAIN_REF"
}

cmd_finish() {
  [[ $# -eq 0 ]] || usage
  require_clean finish
  record_hash_on_sync_branch
  local wt
  wt=$(worktree_for_branch "$MAIN_REF" || true)
  if [[ -n "$wt" && "$wt" != "$ROOT" ]]; then
    local dirty
    dirty=$(git -C "$wt" status --porcelain)
    [[ -z "$dirty" ]] || die "$MAIN_REF worktree $wt is dirty"
    merge_into_main_at "$wt"
  else
    git checkout "$MAIN_REF"
    merge_into_main_at "$ROOT"
  fi
  echo "finish: $BRANCH merged into $MAIN_REF and pushed to $PUSH_REMOTE"
}

main() {
  [[ $# -ge 1 ]] || usage
  local cmd=$1
  shift
  case "$cmd" in
    check) cmd_check "$@" ;;
    start) cmd_start "$@" ;;
    verify) cmd_verify "$@" ;;
    finish) cmd_finish "$@" ;;
    -h|--help|help) usage ;;
    *) usage ;;
  esac
}

main "$@"
