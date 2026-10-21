# Packet: implement the `grok traces` live screen

From: Opus traces lead (Herdr pane `w36:p5`), 2026-10-21. For: the custom-fork Opus lead (compacted), with Sonnet 5.5 subagents.

## The goal

Sab wants a full-screen live view on a second screen. It shows every running Claude Code, Pi and Cursor agent on the mini and on sab-air. Without opening an agent's pane, he should be able to see:
- what it's doing now;
- whether it's waiting on him;
- what its subagents are doing;
- how full its context is.

The spec is `docs/traces/SPEC-live.md` rev 5.1 (743c35c2), and it's binding. `SPEC-live-review{,-2,-3,-4,-5}.md` explain why the rules look the way they do. If the code contradicts the spec, stop and send one message to the traces lead in pane `w36:p5`; don't improvise. Cosmetic issues in the spec (test-table order, the `grok live` title) are not stop conditions.

## How you work

- **Role:**
  - You lead; Sonnet 5.5 subagents do all the code, tests and searches. Keep three in parallel where the steps allow.
  - You write their briefs, review their diffs, and decide.
- **Briefs:** every subagent brief gets a Sonnet review before dispatch. UI briefs must define every visible state and require `TestBackend` snapshots.
- **Worktree:** `git worktree add .worktrees/traces-live -b traces-live main`.
- **Commits:**
  - One commit per step: Gate 0, then 1, 2, 3.
  - Every commit ends with `Co-authored-by: claude-code <claude-code@anthropic.com>` and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Sonnet subagent commits use `Claude Sonnet 5.5`.
- **Tests, debug only:**
  - Never run a release build. The traces lead builds after merge, one build at a time on the mini.
  - `cargo test -p xai-grok-traces`
  - `GROK_HOME=$(mktemp -d) RUST_MIN_STACK=33554432 cargo test -p xai-grok-pager --lib traces_modal`
  - Before handoff, run the full pager lib once with an empty `GROK_HOME`. The baseline is 10391 passed, 0 failed (`PROOF-2026-10-20.md`).
- **Air:** read-only for you, so no builds and no deploys there. The traces lead does step 3's Air deploy (backup, `uname -m`, rsync to a temp name and `mv`, `live --json` check) after merge.
- **Data and sizing:** never commit capture files or real session content. Use `rg`, not `grep -r`, which a hook blocks. No time estimates.

## Order

1. **Gate 0: run Step 0b.**
   - The capture hooks are already wired on both Macs. Output is in `~/.grok/traces/capture/<source>/`; on the Air, run commands as `ssh sab-air 'cd ~/.grok/traces && …'`.
   - Close captures 1–17. Capture 10 closes at the end of step 1, and capture 18 belongs to the traces lead's deploy.
   - Write each yes/no and the fallback used into `docs/traces/MAP.md`, replace the matching (U) markers in `SPEC-live.md`, and commit.
   - Some captures need Sab to act in a session, such as a permission grant or denial, or a Cursor chat. Send the traces lead one message listing them, then apply the spec's fallback and keep going. Row 15's "no" is Sab's decision; use its fallback until he decides.
2. **Step 1.**
   - It opens with the `is_terminal` fix.
   - It is schema 7 plus a `PARSER_VERSION` bump. There is no ALTER path: the old index is replaced via the existing `.rebuild.lock` rebuild. Note this in `MAP.md`.
   - It includes the installer events, the Cursor per-event replies and the Pi extension rewrite (`hooks/pi/traces.ts`).
   - It ends with the spec's real-session replay.
3. **Step 2:** `Mode::Live` render, ordering, and the spec's render snapshots, except those it assigns to step 5.
4. **Step 3:** the `grok traces` subcommand, Esc quits, remote `live --json` with the envelope, and the pty proof. Put the proof beside the existing `pty_e2e` traces test in `crates/codegen/xai-grok-pager-pty-harness/tests/`, run with `--ignored`.
5. **Tests:** sequence tests 1–55 go in the locations the spec names. Unit tests go in `live.rs`; tests 40–45 and 53 go in `tests/remote.rs`, using fake ssh. Index-backed and injected-`alive` tests follow the spec's notes.

## Done means

- `traces-live` is pushed to origin with `git status` clean, and steps 1–3 are ready to merge.
- The tests above are green, and the pty proof passes.
- You've sent the traces lead in `w36:p5` a one-screen report listing:
  - what you verified;
  - every (U) still open after Gate 0, with the fallback in use;
  - the captures that need Sab.
- Steps 4 and 5 are a follow-up packet.
