# SPEC: finish batch 1 (subagent tags, subagent brief, pig arrow keys)

Status: draft 2026-11-12, unreviewed. Build policy: standing for the whole cleanup sweep (`briefs/fix-batch-1.md`).

## Departures from Sab's words (until he rules)
1. The subagent tag format (`1 UI/U…`, dim) is the Opus reviewer's design, not Sab's. The builder repeats the tag on every wrapped prompt line and every line of an opened call body.
2. (Resolved) Homebrew protobuf is 36.2 and `protoc` runs; the build needs no library-path workaround.
3. With the P2 fix, a real Shift+Up/Down while the slash menu is open still goes to Pi's workflow scroll shortcut (its intended binding). A named-key shortcut bound without Shift (`up`) no longer fires on Shift+Up.

## Sab's words
- "seeing what a subagent received is so fucking important… if it receives a slightly incorrect brief… We just found the issue"
- On pig: "The slash command behavior and it kept registering it as shift arrow and acting in bizarre ways"
- Build policy: "we cannot have all three agents running tests and running builds and waiting on each other… we just have to do it all at once. They're gonna have to review their work from code first."

## State now
- P1 is done. Tasks 1 and 2 of `briefs/fix-batch-1.md` (Opus review text) are committed: `e6d0d124` (pager: tags, brief expand and inspect) and `8f1c3e99` (traces crate: brief kept past the backlog cut).
- `cargo test --release -p xai-grok-traces`: 283 passed, 0 failed.
- Pager tests have not run. The first pager compile reached 1444/1454 under the shell workaround before it was stopped.
- Rex session `batch1-cycle` is open. Unset `DYLD_FALLBACK_LIBRARY_PATH` there before P3.

## Packets

### P2: plain arrows stop matching shift shortcuts (builder: the batch 1 Sonnet agent, no cargo)
Source: `docs/SCOUT-pig-2026-11-12.md` (`a6cbe3b5`).
- Cause: `key_id_matches_event` (`xai-grok-pager/src/app/extension_shortcuts.rs:136-150`, modifier check ending `:303`) drops SHIFT for named keys. The Char branch checks it (`:338-339`).
  - pi-workflows binds `shift+up`/`shift+down` by default, and `route_pi_input` (`app_view.rs:2310`, called at `:2391`) runs before the slash menu's `handle_menu_nav` (`:3793`). So plain arrows fire the shortcut and the menu never moves.
- Edit: after `:303`, before `// Check the key itself`:
  ```rust
  // Named keys (arrows, Enter, F-keys) have no shifted glyph, so SHIFT must match exactly.
  if !matches!(event.code, KeyCode::Char(_))
      && event.modifiers.contains(KeyModifiers::SHIFT) != required_mods.contains(KeyModifiers::SHIFT)
  {
      return false;
  }
  ```
- Test (written, not run), next to `match_ctrl_shift_combo` (~`:435`):
  - `"shift+up"` vs Up+NONE → false
  - `"shift+up"` vs Up+SHIFT → true
  - `"up"` vs Up+NONE → true
  - `"up"` vs Up+SHIFT → false
- Check every other caller of `key_id_matches_event` / `match_key` from the code. Commit by path with the sweep trailers.

### P3: the one batch cycle (same builder, Rex session `batch1-cycle`, unfiltered)
- Prerequisites:
  - P2 is committed.
  - Nothing else is building (`pgrep -fl 'cargo|rustc'`).
  - It reuses the warm `target/`.
- Steps:
  1. `cargo test --release -p xai-grok-pager traces_`.
  2. `cargo test --release -p xai-grok-pager extension_shortcuts`.
  3. Fix only failures caused by batch 1 commits, then re-run only those. List failures that predate batch 1.
  4. `cargo insta review` only if a snapshot changed because of batch 1. Name each one in the return.
  5. `scripts/install-fork.sh`: one cargo run, both Macs, checksums.
- If the build passes ~9 min, check rustc CPU with `ps`. Kill only its own PIDs/session.
- Returns:
  - test counts
  - fix SHAs
  - pre-existing failures
  - install checksum lines

### P4: screen check (fresh Haiku, read-only, Rex)
Runs after P3 installs. The binary is `grok-fork`; the TUI is `grok-fork traces`.
- Subagent tags:
  - Root follow of a session with 2+ subagents shows a distinct tag on each subagent's rows and none on root rows.
  - Check at full and half width, in every shipped theme (`GROK_THEME=<name>`, list in `xai-grok-pager-render/src/theme/cache.rs`).
  - Capture `--format=html` per theme.
- Subagent brief:
  - In subagent follow, `l` on the brief shows it in full and `h` folds it.
  - Enter opens it in the inspector, and Esc returns to follow.
  - Following a running subagent with >200 rows still shows its brief.
- pig arrows:
  - On the Mini and on sab-air (`ssh -t sab-mini@sab-air`), `pig`, `/`, plain Down×2, Up.
  - The highlight moves, and no `Running shortcut` line prints.
  - Shift+Down still prints it.
- Keys: never `r` on a Claude row. Never confirm a dashboard launch.
- Kill only its own sessions/PIDs; never pkill/killall.
- Output: `docs/traces/CHECK-batch-1.md` with captures per point, committed by path. Sab sees the tag look before anyone calls it done.

## Out of this batch (`SWEEP-backlog.md`)
`/model` picker ignores Esc. sab-air plain Up not firing. Status-line spam from shortcuts and daily-memory. Subagent chat in grok claude. Resumed subagents invisible in grok claude.
