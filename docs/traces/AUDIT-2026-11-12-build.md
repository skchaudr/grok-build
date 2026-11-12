# AUDIT 2026-11-12: buildability of grok-traces (Haiku 5.5, read-only)

No cargo was run. Evidence is file:line, command output, or dated notes. "Guessing" marks inference.

## Causes of slow builds, ranked by impact

1. **Release builds go cold when agents disable incremental or delete its cache.**
   - Symptom: same pager-bin release target, 2m30s (OBSERVATIONS.md:199, 2026-10-09) vs 11m01s (OBSERVATIONS.md:239, 2026-10-09) vs 14m54s (docs/picker-live-proof.md:148, 2026-10-19). The 11m01s build ran with `CARGO_INCREMENTAL=0` (OBSERVATIONS.md:235). Three data points, correlation only; the 2m30s run's env is not recorded.
   - Cause: `[profile.release] incremental = true` (Cargo.toml:406-407) is overridden by `CARGO_INCREMENTAL=0`, which agents set in 9 unique build runs (OBSERVATIONS.md:98, 109, 172, 177, 765, 776 and others). Agents also `rm -rf target/release/incremental` to free disk (OBSERVATIONS.md:44, 93, 124, 711, 760). The disk reason was real then: 2.4G to 6.6G free (OBSERVATIONS.md:93, 153, 172, 235). Today `df -h /` shows 61Gi available (checked 2026-11-12), so the reason is gone.
   - Fix: stop setting `CARGO_INCREMENTAL=0` and stop deleting `target/release/incremental` for release builds. Docs only: OBSERVATIONS.md is agent-written; the rule belongs in AGENTS.md. Touches 1 doc.

2. **Traces lives inside the 580k-line pager graph, so every traces edit rebuilds the pager.**
   - Symptom: a 12.7k-line change in traces triggers a rebuild of the whole pager. `xai-grok-pager/src` is 728 files, 579,558 lines (`rg -c` count, 2026-11-12); `xai-grok-traces/src` is 12,735 lines.
   - Cause: `crates/codegen/xai-grok-pager/Cargo.toml:126` depends on `xai-grok-traces`. `scripts/install-fork.sh:12` builds `-p xai-grok-pager-bin -p xai-grok-traces` in one run, so the pager always recompiles. The `grok-traces` binary alone only needs the traces crate (`crates/codegen/xai-grok-traces/src/bin/grok-traces.rs`). The alone path exists: `docs/traces/TOOL-grok-traces-cli.md:31` (`cargo build --release -p xai-grok-traces --bin grok-traces`).
   - Fix: a `--traces-only` flag in `scripts/install-fork.sh` (touches 1 file, about 5 lines). Trade-off: grok-fork is then not rebuilt from the same traces source. Sab decides whether that is acceptable against AGENTS.md's "one cargo run" rule.

3. **Test builds use the debug profile, which compiles every dependency a second time.**
   - Symptom: `target/debug/` and `target/release/` both exist (`ls target`, 2026-11-12). The docs recommend debug builds: `docs/traces/SPEC-usable.md:431` (`cargo build -p xai-grok-pager-bin && cargo build -p xai-grok-traces --bin grok-traces`, no `--release`).
   - Cause: `[profile.dev]` (Cargo.toml:456-463) uses `opt-level = 0`, `codegen-units = 128`, `line-tables-only`. Its artifacts do not share with release. The first `cargo test` after a release install is a cold dependency compile. OBSERVATIONS.md:115 and :782 record a 4m00s dev build.
   - Fix: run targeted tests with `cargo test --release -p xai-grok-traces` so they reuse the release dependency artifacts. Docs only: 3 recipes (SPEC-usable.md:70, 431; MAP.md:89).

4. **Shared target dir and cargo's file lock make a build look hung.**
   - Symptom: "This task waited" (OBSERVATIONS.md:57, 724) when another cargo in the same target dir held it. `target/release/.cargo-lock` exists.
   - Cause: worktrees share `target/` (OBSERVATIONS.md:53, 561). `scripts/install-fork.sh:11` refuses to start if any `cargo` runs on the machine (`pgrep -x cargo`), exits 1, and does not wait. A fresh agent sees a failure, not a queue.
   - Fix: per-worktree `CARGO_TARGET_DIR`, or make the script wait on the lock and print who holds it. Touches `scripts/install-fork.sh` only.

5. **Every install transfers the full binary, and the steps run in sequence.**
   - Symptom: `~/.grok/bin/grok-fork` is 188.9M and `grok-traces` 4.3M (`ls -la`, 2026-11-12). Both go over the wire every run.
   - Cause: `scripts/install-fork.sh:20` rsyncs to `$dst.new`, then line 21 renames it. The `.new` file is gone after the rename, so rsync has no basis file and sends the full binary. `nice -n 10` (line 12) also lowers build priority below every agent process (guessing: this matters when agents run).
   - Fix: keep the copy-then-rename (the comment at line 15 is correct) and accept the transfer, or add `--compress`. Touches `scripts/install-fork.sh`.

6. **sccache wraps every build, and its effect on release is unmeasured.**
   - Cause: `~/.cargo/config.toml:1-2` sets `rustc-wrapper = "/Users/sab-mini/.local/bin/sccache"` for all cargo builds. `sccache --show-stats` reports a 10 GiB cache and zero requests, which is meaningless because the server had just started (my query). Guessing: sccache does not cache incremental rustc invocations, so release builds with `incremental = true` get little benefit.
   - Fix: measure before deciding. Run `sccache --show-stats`, do one traces-only build, run it again, and compare "Non-cacheable compilations". Global config, not the repo.

7. **Latent: nested worktrees would double RUSTFLAGS.** Not in play today.
   - Evidence: `git worktree list` shows only `/Volumes/Mini-SSD/offload/repos/grok-build` (main) and `~/.grok-hub` (hub-live). The repo `.cargo/config.toml` repeats every `[target.*] rustflags` section and says rustflags "are not additive" (its comment). Old nested worktrees left 15G targets under `.worktrees/` (OBSERVATIONS.md:557). Guessing: cargo concatenates array config from every ancestor directory, so a checkout nested under another doubles the flags and forces a full rebuild.
   - Fix: keep worktrees outside the repo tree, as `~/.grok-hub` is.

8. **The build recipe is written three ways.** The fresh-agent path is discoverable (AGENTS.md:7 names the script, and its header at lines 3-5 explains `--no-build`), but the docs disagree.
   - `docs/traces/TOOL-grok-traces-cli.md:31-32`: a traces-only release build plus a manual `cp` to `~/.grok/bin`. It skips sab-air and the checksum.
   - `docs/traces/SPEC-usable.md:431`: debug builds (see cause 3).
   - `docs/traces/briefs/*`: release, both packages, no install.
   - Fix: point TOOL-grok-traces-cli.md:31-32 and SPEC-usable.md:431 at `scripts/install-fork.sh`. Docs only.

## Not causes (checked)
- Release profile has no LTO (Cargo.toml:406-408). The heavy thin-LTO profiles are `release-dist` and `x-prod`, which the script does not use.
- No `target/cargo-timings/` exists, so no timing reports were ever written.
- `.remember/` holds no build durations.
- `CARGO_BUILD_JOBS=4` (docs/traces/STATUS.log:6) was set on a Linux build host (khoj). No such cap appears in `~/.zshrc`, `~/.zprofile`, or `~/.zshenv`.
- Disk is not the cause today (61Gi free). It was the cause in October.
- `rust-toolchain.toml` pins 1.94.0. This costs one install, then nothing.

## The fastest correct loop for one traces change

```
cd /Volumes/Mini-SSD/offload/repos/grok-build
# edit crates/codegen/xai-grok-traces/src/...
cargo test --release -p xai-grok-traces <filter>   # targeted tests, release artifacts
scripts/install-fork.sh                            # one cargo run, both bins, both machines, checksums
```

Settings the loop needs: incremental on (default for release), no `CARGO_INCREMENTAL=0`, no deletion of `target/release/incremental`, one cargo at a time. The pager recompile in the install step is unavoidable under the current script (cause 2). Timing is not measured here (no cargo run allowed), so no time is claimed.
