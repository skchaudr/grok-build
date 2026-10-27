# Upstream sync

Last synced upstream commit: `scripts/upstream-sync.rev`. The next xAI dump is one squashed commit on `upstream/main`. This does not edit pager code.

Prerequisites: an `upstream` remote must be configured once (`git remote add upstream <url>`). The URL is intentionally not checked into the repo.

- `scripts/upstream-sync.sh check` — `git fetch upstream`, say whether `upstream/main` moved, print `git diff --stat <last>..upstream/main` for files in `git diff --name-only <last>..main`.
- `scripts/upstream-sync.sh start` — refresh `sync/upstream-<utc-date>` from `main`, autostash a dirty tree (see Autostash), `git merge upstream/main`, append a per-file ours-hunks vs theirs summary to `OBSERVATIONS.md`, and stop on conflicts.
- `scripts/upstream-sync.sh verify` — `df -h /`, refuse under 20G free, then `cargo check -p xai-grok-pager-bin` and `cargo test -p xai-grok-pager --lib --no-run` into this worktree's `target/`. `--dry-run` prints those commands. `--release` prints `run gb-build sab-mini instead` and refuses on hostname `khoj-38*`. Never `cargo clean`.
- `scripts/upstream-sync.sh finish` — record the upstream hash, `git merge --no-ff` the sync branch into `main`, `git pull --ff-only` before `git push`. Requires a clean worktree.

`check --replay 2bdd1d6a..main` reprints that historical touched-file list and restricted stat. `UPSTREAM_SYNC_REF` points `start` at a throwaway ref such as `upstream-sim`.

## Autostash

`start` no longer requires a clean tree. Uncommitted changes, including untracked files, are stashed as `grok-upstream-autostash-<utc>` before the upstream merge and restored after a clean merge. The helper is `scripts/lib/upstream-autostash.sh`. The stash is identified by its commit SHA, never `stash@{0}`.

If the upstream merge itself conflicts, the stash is not applied. The merge is left in progress for a human, and the preserved stash SHA is printed.

If the post-merge restore conflicts, the failed apply is wiped with `git reset --hard HEAD` (the merge commit stays), the stash is preserved, and the SHA is printed with `git stash apply <sha>` guidance.

Entries older than 7 days are reported at the start of `start`. They are never auto-deleted.

Safety contract: this flow never hard-resets the branch to origin or upstream, never force-pushes, and never stashes a conflicted or in-progress merge (it refuses instead).
