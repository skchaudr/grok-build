# grok-pi vs pig parity scout (2026-11-12, read-only)

## Caveat first

- Dwsy's standalone `grok-pi` is not installed. `~/.local/bin/grok-pi` does not exist, and `which -a grok-pi` finds nothing. I could not capture it live.
- The grok-pi side is from Sab's screenshots plus source at `719ba57f` (`git archive`, extracted to `/tmp/parity-scout/pi`).
- The pig side is live (Rex capture) plus fork source at `HEAD` (`/tmp/parity-scout/fork`).
- Not run: the pig `/tutorial` tour and the pig theme live-preview. Both are source-only.

## What pig already matches

| Surface | Evidence |
|---|---|
| Welcome menu | Live pig: `New worktree ctrl+w`, `Resume session ctrl+r`, `Changelog`, `Quit ctrl+q`. Same four rows as grok-pi's screenshot. |
| Shortcuts overlay | Live pig `Ctrl+x`: six groups (Essentials, Input, Conversation Navigation, Conversation Actions, Panels, Session, Dashboard). grok-pi maps `/hotkeys` to the same overlay at `719ba57f:crates/codegen/xai-grok-pager-bin/src/bin/grok-pi.rs:146`. |
| F2 settings modal | Live pig `F2` opens Settings. Sections: Appearance, Mouse, Editor & Input, Agent & Approval, Privacy, Models, Session, Advanced. grok-pi has the same set plus Popups. |
| Theme picker | Live pig `Settings > Theme`, footer `↑/↓ try | Enter select | double-click select | Esc revert`. grok-pi's footer is `↑/↓ try | Enter select | Esc revert`. |
| Slash commands | Live pig `/`: `/resume /compact /settings /dashboard /theme /quit /effort`. Source has `/tree`, `/tutorial`, `/plan-mode` (`fork/.../slash/commands/{tree,tutorial,plan_mode}.rs`). |
| Theme count | Pig's picker lists 19 names in source (`fork/.../settings/defs.rs:33-124`). grok-pi's picker lists 9 (`pi/.../settings/defs.rs:83-124`). |

## Gap table

File paths: `pi/` means `719ba57f` under `crates/codegen/`. `fork/` means `HEAD` under `crates/codegen/`.

| feature | grok-pi (file:line) | grok-fork equivalent (file:line or none) | pig shows it? | size |
|---|---|---|---|---|
| Pi logo and "grok-pi" welcome brand | `xai-grok-pager-bin/src/bin/grok-pi.rs:217-232` (block-art π, subtitle "Pi agent core in Grok Build's native terminal UI"); `:1317-1326` (`ExternalUiProfile` with `logo` and `welcome_brand`) | none. `ExternalUiProfile`, `ExternalLogoArt`, `ExternalWelcomeBrand` are absent from `HEAD`. They exist only at `pi/xai-grok-pager/src/acp/mod.rs:89-106` and `pi/xai-grok-pager/src/views/welcome/logo.rs:119`. Fork welcome: `fork/xai-grok-pager/src/views/welcome/logo.rs:23` | no. Shows `Grok Build 1.0.45` and the ASCII G | medium. Pager: `acp/mod.rs`, `views/welcome/logo.rs`, `views/welcome/hero_box.rs`. Plus the transport. The standalone passes the profile in-process (`grok-pi.rs:1253-1333`). The stdio `grok-pi-acp` (pig's backend) has no path for it. |
| "Welcome to grok-pi" tour, 18 topics | `xai-grok-pager-bin/src/bin/grok_pi/tutorial_profile.rs:19` (topics), `:113` (title), `:134` (asserts 18); 18 files in `grok_pi/tutorial/*.md` | `fork/xai-grok-pager/src/tutorial_docs.rs` (9 `topic!` entries); `fork/xai-grok-pager/src/views/tutorial.rs:411` (title "Welcome to Grok Build") | partly. Same mechanism, 9 topics, Grok Build copy | small to medium. Content: 18 md files. Code: call `set_tutorial_profile` (`tutorial_docs.rs`) from the pig launch path. Pig needs to know it is running Pi. |
| F2 tabbed settings (Appearance, Popups, Mouse, Editor, Agent, Privacy, Models, Advanced) | `xai-grok-pager/src/views/pi_settings/` (7 files, about 139 KB; `layout.rs:15` tab_label, `render.rs:90` render_pi_settings); `settings/registry.rs:33` (Popups), `:47` (ALL) | flat list with section headers: `fork/xai-grok-pager/src/views/settings_modal/render.rs` (about 102 KB); categories `fork/.../settings/registry.rs:31,44` | no. Live pig is one scrolling list, not tabs. No Popups section | large. Tab row, input and render paths, and a new Popups category. Largest item. |
| Popups category | `pi/.../settings/registry.rs:33,47`; `pi/.../settings/defs.rs:1358` | none (0 hits for `SettingCategory::Popups` in fork) | no | medium. New category plus its rows in `defs.rs`. |
| Transparent themes | `pi/.../settings/defs.rs:88` (`Transparent`), `:93` (`Transparent Light`); canonical `pi:transparent`, `pi:transparent-light` (`:87,:92`) | none. Fork's picker has no Transparent entry | no | small. Two theme JSONs plus wiring, using `pi/.../theme/pi/map.rs:296-303` as the pattern. |
| 102 embedded Pi themes | `pi/xai-grok-pager-render/src/theme/pi/registry.rs:24` onward (`builtin_theme!` list); assets `assets/pi-themes/*.json` (102 files) | fork picker has 19 named themes, `fork/.../theme/mod.rs:247-256` | no | small code, large asset set. Open question: grok-pi's picker shows only 9 entries. I did not find where the other 102 appear in the UI. Check before porting. |
| Tool-card settings: Ctrl+O tool expansion, Bash run display, Side-by-side edit diffs, Thinking border colors, Other tool args, Format Bash/Eval display, Terminal tab progress, Remote TUI footer | `pi/.../settings/defs.rs` (labels present in Appearance and Tool output sections; 2 to 3 hits each); screenshot rows | none. 0 hits for these labels in fork's pager src. Fork has `Group tool calls` and `Collapsed edit blocks` | no | medium. Each row needs a runtime knob in the fork renderer. The render code is the cost, not the rows. |
| Pi F2 extension toggles: `pi_subagents`, `pi_todo`, `pi_goal`, `pi_loop`, `pi_btw`, `pi_herdr`, `pi_ask_user_question`, `pi_config_skill`, `pi_tree_file_rollback` | `xai-grok-pager-bin/src/bin/grok-pi.rs:366-448` (`[ui]` keys, restart required); `grok_pi/rollback_extension.rs:82`; `grok_pi/config_skill.rs:7` | none | no | medium. Each is an extension injected at startup. They need the Pi side (grok-pi-acp) and a settings row each. Not verified whether they render in the modal. |
| Pi web config: `/pi-config web`, `/pi-models web` | `xai-grok-pager-bin/src/bin/grok-pi.rs:515`; `grok_pi/web_config_extension.rs` (10 KB; baked catalog, line 29) | none (0 hits for `pi-config` in fork) | no | medium to large. Extension plus a web surface. Standalone-only today. |
| Pi slash commands: `/pi-models`, `/pi-config`, `/hotkeys` as native names | `grok-pi.rs:141-160+` (`PI_GROK_NATIVE_COMMANDS`) | `/tree`, `/tutorial`, `/plan-mode` exist in fork (`slash/commands/`). `/pi-models` and `/pi-config` do not | no for those two | small per command, once the Pi side exists. |

## Notes

- The welcome branding is the top visible gap. It is also the cheapest to diagnose. The hook exists only in the standalone bin, and the stdio path never sets it. Pig shows `Grok Build` because nothing overrides the default.
- On the theme picker, pig is ahead on count (19 versus 9). grok-pi's extra value is the Transparent pair and the embedded Pi set, and I did not confirm the Pi set is reachable from the picker.
- Capture caveat: in the pig slash menu, `down` produced `Running shortcut shift+down` toasts instead of moving the selection. I did not investigate whether that is a Rex key mapping or a pig keymap bug.
- Sessions: `session:01a1d2b0-f569-7812-b320-8c80e4271fb8` (parity-grok-pi, failed to start because the binary is missing; killed). `session:01a1d2b0-f592-7b2b-9c52-61f17fe611d8` (parity-pig; killed). Pig pager PID `85896` (cwd `/tmp/parity-scout`) confirmed gone after the kill. No prompt was submitted.
