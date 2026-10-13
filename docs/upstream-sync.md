# Upstream sync

Last synced upstream commit: `scripts/upstream-sync.rev`. The next xAI dump is one squashed commit on `upstream/main`. Run from a clean worktree. This does not edit pager code.

- `scripts/upstream-sync.sh check` — `git fetch upstream`, say whether `upstream/main` moved, print `git diff --stat <last>..upstream/main` for files in `git diff --name-only <last>..main`.
- `scripts/upstream-sync.sh start` — refresh `sync/upstream-<utc-date>` from `main`, `git merge upstream/main`, append a per-file ours-hunks vs theirs summary to `OBSERVATIONS.md`, and stop on conflicts.
- `scripts/upstream-sync.sh verify` — `df -h /`, refuse under 20G free, then `cargo build --release -p xai-grok-pager-bin` and `cargo test -p xai-grok-pager --lib` with `CARGO_TARGET_DIR` set to this worktree's `target/`. Never `cargo clean`.
- `scripts/upstream-sync.sh finish` — record the upstream hash, `git merge --no-ff` the sync branch into `main`, `git pull --ff-only` before `git push`.

`check --replay 2bdd1d6a..main` reprints that historical touched-file list and restricted stat. `UPSTREAM_SYNC_REF` points `start` at a throwaway ref such as `upstream-sim`.
