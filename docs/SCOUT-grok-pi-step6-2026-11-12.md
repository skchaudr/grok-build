# Scout: grok-pi step 6, and one house vs two (2026-11-12, read-only)

Sources: `main` at `8af717b7`, Dwsy snapshot `719ba57f`, and Dwsy's live `main` at `fd0bf8c1` (fetched 11-12 into a scratch repo at `/tmp/dwsy-scratch` that borrows this repo's objects; no ref was added here). GitHub API for commit history. No cargo runs. Nothing below has been compiled.

## Answer

- **A clean throughline exists. It runs through our fork.** `719ba57f` is Dwsy's branch `merge/upstream-2bdd1d6a-static-20261013`. It sits on xAI `2bdd1d6a`, the same base as our `main` (`git merge-base main 719ba57f` = `2bdd1d6a`). xAI has not pushed since `2bdd1d6a` (09-29). So all three lines share one upstream base right now.
- **Order:** merge `719ba57f` into `main` once (44 files conflict, 34 of them need real hand work). Then wire four stdio gaps. After that, merge Dwsy's `main` whenever you want his newer work (30 files and 46 hunks for his 72 newer commits, measured), and merge xAI syncs when they resume, all in one tree.
- **Is it one-to-one?** For step 6 itself, mostly yes: about 40k of its lines are new files or merge cleanly. The hand merge is in 15 files: 9 hot pager files that every feature hooks into (33 hunks), plus 6 step-6 files (8 hunks). The other 25 conflicted files are not step 6. In them, both sides built the same feature twice, for example subagent child steer in `dispatch/prompt.rs`.
- **Two houses (b) costs more and gets less.** Dwsy's live `main` is still on xAI `37949780` (09-09). His `2bdd1d6a` branch has not been merged in 4 weeks. Carrying our work onto his `main` measures 187 conflicted files, 542 hunks and 60,060 lines. That is about 7x the 77 hunks of option (a). Claude and Pi in one window, plus one traces surface over both, exist only in (a).
- **Recommendation: (a), finished in the 7 steps at the bottom.** The merged tree also contains Dwsy's standalone `bin/grok-pi.rs`, so the standalone build can come from the one house too. That build is not verified.

## 1. Step 6, area by area

Diffs are measured since `2bdd1d6a`. Status per file: **new** = only Dwsy changed it, so it copies clean. **same** = already identical on `main` (ported in steps 1-5). **auto** = both sides changed it and git merges it. **conflict** = needs a hand merge. Hunks and lines are counted from `git merge-tree --write-tree main 719ba57f`.

| Area | Main files (under `crates/codegen/`) | Dwsy +/- | new / same / auto / conflict | Conflict hunks (lines) |
|---|---|---|---|---|
| F2 tabbed settings | `xai-grok-pager/src/views/pi_settings/*` (7), `views/settings_modal/*`, `app/dispatch/settings/{ui,setters}.rs`, `views/modal.rs` | +8812 -837 (16 files) | 10 / 0 / 3 / 3 | 3 (48) |
| Registry, defs, Popups, tool-card knobs | `settings/{registry,defs,layout}.rs`, `xai-grok-shell/src/host_features.rs`, `settings_writes.rs`, `scrollback/blocks/tool/{side_by_side_edit,hook,other,edit}.rs`, `pager-render/src/appearance/*`, `effects/helpers.rs` | +4848 -59 (15) | 11 / 0 / 4 / 0 | 0 |
| Welcome logo, brand, menu | `views/welcome/*`, `acp/mod.rs` (`ExternalUiProfile`, :106) | +481 -71 (5) | 1 / 0 / 2 / 2 | 4 (37) |
| Tour | `tutorial_docs.rs`, `views/tutorial.rs`, `pager-bin/.../grok_pi/tutorial_profile.rs` + 18 md | +644 -26 (21) | 21 / 0 / 0 / 0 | 0 |
| Pi extension toggles | `extensions/*/grok-pi.json` (8), `pager-bin/build.rs` (manifest catalog), `grok_pi/{config_skill,rollback_extension,extension_self_heal}.rs` | +1126 -5 (13) | 4 / 9 / 0 / 0 | 0 |
| /pi-config, /pi-models, web config | `pi_resource_{config,policy}.rs`, `pi_model_config.rs`, `views/pi_{config,models}.rs`, `slash/commands/{pi_config,pi_models,scoped_models}.rs`, `grok_pi/web_config_extension.rs`, `extensions/pi-grok-web-config/*` | +14123 -0 (30) | 10 / 20 / 0 / 0 | 0 |
| Themes | `pager-render/src/theme/pi/*` (7), `assets/pi-themes/*.json` (102), `theme/mod.rs`, `slash/commands/theme.rs` | +9975 -55 (111) | 109 / 0 / 1 / 1 | 1 (62) |
| Standalone bin and home | `bin/grok-pi.rs`, `grok_pi/{migrate_home,home}.rs` | +2297 -0 (3) | 3 / 0 / 0 / 0 | 0 |
| **Hot files every area hooks into** | `app/{modals,app_view,actions,event_loop}.rs`, `effects/mod.rs`, `dispatch/router.rs`, `agent_view/{render,input}.rs`, `acp/tracker.rs`, `scrollback/blocks/tool/mod.rs` | +9036 -689 (10) | 0 / 0 / 1 / 9 | 33 (958) |
| dpi shim | `pi_adapter/session.rs` +50, `tools/dpi-rpc/dpi-rpc.mjs` +372, `run-dpi.sh` +22 (from `skchaudr/grok-pi` `sab/dpi-rpc-shim`, 4 commits) | +444 -0 | all adds. `pi-grok-adapter` is identical on `main` and `719ba57f` | 0 (likely clean. The commits are not in this repo, so not test-applied) |

### Why step 6 cannot be copied file by file

- The tool-card knobs run through the hot files. For example, `side_by_side_edit` is read in 13 files, including `dispatch/router.rs`, `appearance/cache.rs`, `scrollback/blocks/tool/mod.rs` and `views/block_viewer/mod.rs`. `show_other_tool_args` is also read in `acp/tracker.rs`, `app/event_loop.rs` and `pi-grok-adapter/src/tool_projection.rs:352`.
- `app/modals.rs` (+3604 -347) mixes step 6 (`PiSettings`, `PiConfig`, `PiModels` arms; 54 added-line mentions, some lines counted twice) with unrelated Dwsy features: notifications, session tree, tree map, model picker detail and session preview.
- So copying only step 6 means hand-porting its hooks out of hunks that also carry non-step-6 code. A real merge of `719ba57f` brings everything, step 6 included, in one move. It also leaves git knowing the lineage.

### Whole-merge totals (`main` x `719ba57f`)

| Bucket | Count |
|---|---|
| Dwsy files since base | 1061 (+173,758 -5,112) |
| Already identical on `main` (steps 1-5 copies) | 233 files |
| Dwsy-only, copy clean | 712 files (+86,776 -2,803) |
| Both changed, auto-merge | 72 files |
| Conflicts | 44 files, 77 hunks, 2,887 lines in markers |
| ...of which add/add (our step 1-5 copies vs his originals) | 6 files. Real diffs are tiny: `codemode.rs` 1/1 lines, `tree_map.rs` 4/2, `extension_shortcuts.rs` 29/2 (our arrow fix `5afdbfab`), `session_paths.rs` 11/9, `notify.rs` 7/65, `AGENTS.md`. Take ours. |
| ...root files | `.gitignore`, `AGENTS.md`, `Cargo.toml`, `Cargo.lock` (take ours or regenerate) |
| ...real content conflicts | 38 files, 71 hunks, 2,024 lines. The biggest are `acp/tracker.rs` 8 hunks/244 lines, `dispatch/prompt.rs` 149, `dispatch/task_result.rs` 125, `app_view.rs` 123, `agent_view/paste.rs` 123, `dispatch/transcript.rs` 119, `tool/mod.rs` 6 hunks/115, `effects/mod.rs` 108, `event_loop.rs` 101, `modals.rs` 87 |
| Dwsy pager changes outside step 6 that come along | 217 files (+27,333 -2,545), for example `views/review.rs` 2888, `cache_graph.rs` 881, `shortcut_manager.rs`, `subagent_catalog_pane.rs`, `inline_edit.rs`. They hold 25 of the 44 conflicts. |

`merge-tree` in the other direction (`719ba57f` x `main`) gives the same 44 files.

### Where it is not one-to-one, and why

1. **We ported steps 1-5 as copies plus our own glue, not as a merge.** Per-session `is_pi_agent` (`app_view.rs:1748`), `acp_handler/pi_ui.rs`, `pi_tree.rs`, `pi_session.rs`, `pi_remote_tui.rs`. Dwsy has his own versions of some of these in the same functions. Example: `dispatch/prompt.rs:858` has our Claude and Pi child-steer refusal against his `root_agent` rewrite.
2. **Our Claude parity and traces edits sit in the same hot files**, mainly `tracker.rs`, `effects/mod.rs` and `event_loop.rs`.
3. **F2 changes for everyone.** Dwsy binds F2 to the tabbed panel for every session (`agent_view/input.rs:1428`, `ActionId::OpenSettings => Action::OpenPiSettings`). In one tree, Grok and Claude sessions get the tabbed panel too. That fits your "one TUI, one style". If you want the flat panel kept for them, it is one `is_pi_session` check.
4. **Transport** (next section).

## 2. The transport gap: what each area needs over stdio

The standalone builds `ExternalUiProfile` in-process (`grok-pi.rs:1314-1330`: logo, `welcome_brand`, `hide_new_worktree`, `changelog_url`, `host_features`). `pig` runs `grok-pi-acp` as a stdio child. Its header says it drops the "pager, theme, tutorial, updater and F2 resource policy" (`main:.../bin/grok-pi-acp.rs:3`).

| Area | Lives in | Needs over stdio |
|---|---|---|
| F2 tabbed panel | pager | Nothing for the panel itself. Pi rows come from the host-feature manifest (`build.rs` catalog plus `SettingsRegistry::defaults_with_host_features`, `registry.rs:507`). `build.rs` is in the same crate as `grok-fork`, so grok-fork startup needs one call to build the registry from it. |
| Registry and knobs | pager | Nothing. The pager renderer reads them. |
| Extension toggles | Pi side | Already done on the backend. `grok-pi-acp` reads `[ui].pi_*` from `~/.grok` config at spawn (`grok-pi-acp.rs:86,118,138-175`). F2 writes the same file. "Restart required" means a new `grok-pi-acp` process. When Pi runs over `ssh -T`, the config is on the remote host, so F2 edits on the local machine do not reach it. |
| Welcome logo and brand | pager | No protocol needed. A pager-side table keyed on the launch agent (`--agent-cmd grok-pi-acp`) fills `ExternalUiProfile` the way `grok-pi.rs:1314` does. With the picker (`gb`), the welcome screen is app-wide and comes before any session, so it stays Grok Build unless you choose otherwise. |
| Tour | pager | Call `set_tutorial_profile` (`tutorial_docs.rs:125`) from the same launch-agent check. `tutorial_profile.rs:120` already does this for the standalone. Share the file via `#[path]`, as `grok-pi-acp` does with `grok_pi/*`. |
| /pi-config, /pi-models | pager edits Pi files directly | They write `~/.pi/agent/{settings.json,models.json}` from the pager process (`pi_resource_config.rs:184-192,939-946`). This works when the pager and Pi share a machine and does not work over `ssh -T`. Applying changes uses Pi's `/reload`, which is forwarded as a native command (`grok-pi.rs:166`). The F2 resource admission policy (`grok-pi.rs:544-586`) has to move into `grok-pi-acp`. |
| Web config | Pi side | Add `web_config_extension.rs` and `config_skill.rs` to `grok-pi-acp` (the standalone does this at `grok-pi.rs:516` and `:347`). The extension serves its page from the machine Pi runs on. |
| Themes | pager | One call at grok-fork startup: `theme::pi::init_discovery(&cwd)` (`grok-pi.rs:345`). Transport does not matter. |
| dpi shim | Pi side | Nothing. It is gated by env `GROK_PI_DPI=1` on the Pi process. |

## 3. One house (a) vs two houses (b)

### How fast each upstream moves (measured)

| Upstream | History |
|---|---|
| xAI `grok-build` | 51 "Synced from monorepo" commits from July to September (17 / 24 / 10 per month), each a drop of 68 to 962 files. **None since `2bdd1d6a` on 09-29** (`compare 2bdd1d6a...main` = identical). |
| Dwsy `grok-pi-tui` `main` | Commits not in xAI per month: Jul 187, Aug 115, Sep 50, Oct 44, Nov 35 (to 11-12). 72 commits since `719ba57f` forked off (259 files, +22,390 -1,954). He merged xAI at least 5 times (`37949780`, `07b2f714`, `e5fd481`, `75e73f3`, `a5589e9`). His `main` is still on `37949780`. His `2bdd1d6a` branch (`719ba57f`) has been idle since 10-14. |
| Ours | 566 non-merge commits since base (Oct 403, Nov 162). |

### Initial merge size (all from `merge-tree`)

| Merge | Files | Hunks | Lines in markers |
|---|---|---|---|
| (a) `main` x `719ba57f` | 44 | 77 | 2,887 (2,024 real) |
| (a) then catch up: (`main`+`719ba57f`) x Dwsy `main` (simulated, first merge auto-resolved to ours) | 30 | 46 | 2,986 |
| For comparison, Dwsy's own unmerged divergence: `719ba57f` x Dwsy `main` | 26 | 36 | 1,851 |
| (b) Dwsy `main` x our `main` | 187 | 542 | 60,060 (62 add/add, 3 modify/delete) |
| (b) built on `719ba57f` instead | 44 | 77 | the same tree as (a), on a branch Dwsy has not used for 4 weeks |

Most of the catch-up cost in (a) is Dwsy's own two lines of work (26 of the 30 files). Our part is small: 4 extra files and 10 extra hunks.

### What we would carry into a Dwsy-based house (b)

| Our work since base | Files | Lines | Shareable how |
|---|---|---|---|
| Traces (crate `xai-grok-traces` 50 files +19,004, plus pager views and feeds) | 83 | +39,049 | The crate can be shared as a crate or subtree. The pager half (`views/traces_*`, `acp/*_feed.rs`, `standalone_traces.rs`) is pager code and hooks into the hot files. It would be a patch series rebased onto Dwsy's pager. |
| Leader, picker, external agents | 12 | +6,380 | Not needed in (b): the standalone runs Pi in-process and has no leader. |
| Claude parity (`claude_subagents.rs`, `claude_feed.rs`, `claude_menu.rs`) | 6 | +3,156 | Not used in (b). Claude stays in our fork. |
| Our Pi glue (steps 2-5) | 7 | +2,692 | Replaced by Dwsy's in-process versions. |
| F3 side panel | 11 | +1,110 | Patch series (pager). |
| Themes (19 zed ports) | 19 | +7,744 | Assets plus one `theme/mod.rs` hunk. |
| Our edits inside files Dwsy also changed | 116 | +10,240 -537 | Re-resolved on every rebase. |

### What you get or lose

| | (a) one house | (b) two houses |
|---|---|---|
| Claude and Pi in one window, one dashboard, one traces view over both | yes | no. You get two apps. |
| Dwsy's standalone look (logo, tour, F2, themes) | yes, after the stdio wiring in section 2 | yes, native |
| Standalone `grok-pi` binary | in the tree (`bin/grok-pi.rs` comes with the merge). Not compiled. | yes |
| xAI sync work | merged once | merged twice: Dwsy merges into his line, we merge into ours, and the shared patches get redone |
| Every pager fix (example: `5afdbfab` arrows) | lands once | lands twice, or rides the patch series |
| Installs | one `install-fork.sh` | two build and install paths |
| Pi over `ssh -T` | works for chat. `/pi-config` and F2 toggles touch only local files | not offered (in-process only) |
| Dwsy's updater, release channels, `~/.grok-pi` home | not used (Pi uses `~/.pi`, the pager uses `~/.grok`) | yes |

## 4. Recommendation

Take (a). Three facts decide it:

1. Our `main` and Dwsy's `719ba57f` share xAI `2bdd1d6a`, and xAI has not moved since. The merge is 44 files and 77 hunks now. Carrying our work onto Dwsy's live `main` is 187 files and 542 hunks.
2. Catching up with Dwsy later costs about what Dwsy pays for his own unmerged branch (30 vs 26 files).
3. Claude and Pi in one window, with one traces surface, exists only in (a). The standalone binary comes along in the same tree.

Today is a half-migration. 233 files were copied by hand, git does not know they came from Dwsy, and that is why the 6 add/add conflicts exist. One real merge commit closes that.

### Steps to done (no half state left)

1. In a worktree off `main`, run `git merge 719ba57f` as one merge commit. Resolve the 4 root files (ours; regenerate `Cargo.lock`), the 6 add/add files (ours), and the 38 content files (71 hunks). Where both sides built the same thing (child steer, codemode hooks, subagent catalog), keep one, not both.
2. Settle F2: Dwsy's tabbed panel for every session (`input.rs:1428`) or only for Pi. Your "one style" points to every session.
3. grok-fork startup: build the settings registry from the host-feature manifest, call `theme::pi::init_discovery`, and set the welcome brand and tutorial profile when the launch agent is `grok-pi-acp`.
4. `grok-pi-acp`: add the resource policy, `web_config_extension` and `config_skill` from `grok-pi.rs`.
5. Fetch `skchaudr/grok-pi` and cherry-pick the dpi shim (`5c374deb`, `01a8039a`, `0859bfec`, `8f376ca1`).
6. One `--release` targeted-test pass, then `scripts/install-fork.sh`. Look at welcome, F2, tour and the theme picker on screen in every shipped theme (AGENTS.md rule), both in `pig` and in `gb` with Claude open beside Pi.
7. Upkeep: record Dwsy's merged rev next to `scripts/upstream-sync.rev`. Merge Dwsy's `main`, or his next `2bdd1d6a`-based release, as a normal merge when you want his newer work. Merge xAI syncs into the same tree when they resume. Point `grok-pi` on both Macs at the build from our tree and retire Dwsy's installed copy.

Not verified: whether the merged tree compiles, and whether `bin/grok-pi.rs` still builds after the hand merge. Step 6 of the list above answers both.
