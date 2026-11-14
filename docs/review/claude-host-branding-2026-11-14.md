# Review: grok claude resume fix + Claude host branding (2026-11-14)

Scope: `git show eca9236f`, `git diff eca9236f ef07864b`. Read from code only; nothing built or run except `grok-fork --version` (installed binary).

## Findings, ranked

### 1. The Claude welcome version is `0.0.0-dev (<commit>)`, not a real version (FIX-FIRST)
- `xai-grok-pager-bin/src/main.rs:1830` sets `version: env!("VERSION_WITH_COMMIT")`. `xai-grok-pager-bin/build.rs:84-95` builds that from `GROK_PI_VERSION` or `git describe --match v*`. The repo has no `v*` tags (`git describe` fails: "No names found"), and `scripts/install-fork.sh:18` sets neither variable.
- What it looks like: the installed binary prints `grok 0.0.0-dev (17a1ac9bdf7c) [stable]`, so the hero line reads `Claude Code  0.0.0-dev (17a1ac9bdf7c)`. The launch brand also wins over the adapter's `agentInfo` (`views/welcome/mod.rs:777-779`), so whatever version claude-agent-acp reports never shows.
- Sab's words: "grok-pi … shows how to do it (… real version …)". grok-pi shows its own release version, and this shows a dev placeholder.
- Fix: use `version: ""` for Claude, and in `agent_identity` fall back to `self.agent_version` when `brand.version` is empty. That is two lines. I have not checked what version string the adapter reports, so look at it on screen after the fix.

### 2. `/release-notes` and `/changelog` still open Grok's changelog under Claude (FIX-FIRST)
- `slash/commands/release_notes.rs:15-21` always fetches Grok's CDN changelog. The command is not in the hidden list (`slash/claude_menu.rs:81-106`), so a Claude session's slash menu still offers it. The `ChangelogFetched` early return (`dispatch/task_result.rs:1113-1118`) only changes the welcome hero bullets.
- Sab's words: "no more generic grok changelog".
- Fix: at the top of `ReleaseNotesCommand::run`, add `if let Some(url) = welcome_menu_override().and_then(|m| m.changelog_url) { return CommandResult::Action(Action::OpenUrl(url.into())); }`. Pi gets the same behaviour, matching its welcome Changelog row.

### 3. The title test now reads process-wide state and can flake (fix with the tests)
- `terminal_title_string` (`app/mod.rs:2159-2161`) and `SourceFilter::label` (`views/session_picker.rs:223-224`) read the global brand cell. `welcome_brand_override_round_trips` (`views/welcome/logo.rs:576-592`) sets it to `"grok-pi"` while other tests run in parallel.
- How it fails: `terminal_title_strips_control_characters` (`app/mod.rs:2237`) sees `"My chat - grok-pi"` and fails at random.
- Fix: split out a pure `terminal_title_for(title, product: Option<&str>)`, test that, and have `terminal_title_string` pass in the override.

### 4. Pi behaviour changed in three ways the commit does not mention (keep all three, but say so)
- **Subtitle:** Pi-only launches now draw the brand subtitle (`views/welcome/hero_box.rs:326-328`). Before, `identity.is_some()` hid it. This matches Sab's description of grok-pi ("subtitle line").
- **Window title:** Pi window titles now end in `- grok-pi` instead of `- grok` (`app/mod.rs:2160`).
- **Changelog bullets:** Pi no longer shows Grok release-note bullets in the hero, because Pi sets `changelog_url` (`main.rs:1804`, `task_result.rs:1114`). This matches "no changelog dump".
- Stock Grok is unchanged: no profile is set, so every override is `None`.

### 5. The resume picker under Claude can still reach Pi-only pages (inherited from the Dwsy merge; not blocking)
- `f` cycles Grok → Headless → External → All → Local → Remote (`views/session_picker.rs:232-241`). Every ACP row has `source: "local"` (`app/effects/session_list.rs:62`), so under Claude the Headless, External and Remote pages are always empty.
- The External page also turns on the Pi "Current folder / All" tabs (`app/modals.rs:1461-1476`). Their Tab key sends `RefreshExternalSessionCatalog`, which is now a no-op for Claude (`dispatch/session/load.rs:1953`), so the tabs do nothing.
- With `psm_resume_index` on, Ctrl+F on that page (`modals.rs:1514`) opens Pi's PSM search. Picking a hit goes to the Pi load path (`load.rs:1163-1165`, gated on `external_agent` rather than on Pi).
- Fix: hide the filter (`filter_key_hint`/`filter_label` = None) for external hosts that are not Pi, and change `load.rs:1163` to `!app.is_pi_session()`. Leave this for a later pass unless Sab wants it now.

### 6. Tests that are missing and cheap to add
- **Covered:** `claude_host_resume_lists_through_acp_not_the_pi_catalog` pins the right things for the in-session `/resume`: the ACP fetch, no Pi catalog, and the Grok filter.
- **Missing:**
  - Welcome-screen Resume for Claude, with no agent: expect `FetchSessionList { host: Welcome }` and no `FetchExternalSessionCatalog`. The path does go through the fix (`app_view.rs:4843` → `ShowSessionPicker` → `load.rs:1914-1923` → `foreign.rs:48`), but nothing pins it.
  - The Claude-only gate in `install_claude_host_ui` (`main.rs:1815-1820`). This is the only guard against the process-wide leak. Extract `fn claude_only(args) -> bool` and test three launches: Claude alone (true), Claude plus Pi (false), Pi alone (false).
  - The pure title function from finding 3, with and without a brand.
  - The `SourceFilter::Grok` label with and without a brand. This needs a serial guard, or a pure helper as in finding 3.

## Answers to the brief's questions
- **Q1 (root cause and resume paths):** The root cause is right. `pi_catalog_host` = `is_pi_session()` (`app_view.rs:1982`) covers all three Pi-catalog entry points in `load.rs` (1885, 1931, 1953). `FetchSessionList` lists through ACP when `external_agent` is set (`effects/mod.rs:800-822`). The `~/.claude` foreign scan is skipped for external hosts (`foreign.rs:104`), so rows are not duplicated. Welcome-screen Resume goes through the same fixed path. `load.rs:434` (external row → `dispatch_load_session` in the row's cwd) is correct for Claude. The only Pi-only branch still reachable is the one in finding 5.
- **Q1 (detection):** Claude detection works for the real launcher. gb runs `~/.local/bin/claude-agent-acp` (`~/.grok-hub/scripts/hub/gb:53-54`), which gives the label `claude-agent`, and `is_claude_label` accepts it.
- **Q2 (global state):** There is no global-state leak. The Pi and Claude install conditions cannot both be true, the launch profile is first-wins (`acp/mod.rs:143-145`), and in-process External profiles go through `apply_welcome_profile(Some(profile))` (`event_loop.rs:1107`). A mixed picker (`grok all`) gets no brand and keeps the Grok welcome. Pi changes are listed in finding 4.
- **Q3 ("Grok" strings still reachable under a Claude-only launch):**
  - Fixed:
    - trust prompt (`welcome/mod.rs:1054-1057`)
    - hero name and subtitle
    - window title (`claude code`)
    - picker chip (`Claude Code`)
  - Already suppressed for external hosts:
    - announcements and tips (`event_loop.rs:1551-1556`, `acp_handler/settings.rs:480-486`)
    - `/feedback`, and with it the HERO_SUBTITLE "/feedback" prompt (`claude_menu.rs:85`)
  - Fix now: `/release-notes` and `/changelog` (finding 2).
  - Leave, because each is Grok content rather than a label, or is queued:
    - `/tutorial` ("…Grok Build", `slash/commands/tutorial.rs:17`). Its content is Grok's tour. A Claude tour is a feature like `pi_tutorial_profile`.
    - `/docs` tip "Ask Grok about the docs" (`views/modal.rs:1539`). Those are Grok's docs; hide `/docs` if Sab wants them gone.
    - Settings modal: Sab queued it.
    - CLI `--help` "Grok Build TUI": not shown in the TUI.
  - Not checked whether reachable:
    - "Import Claude settings" welcome row (`welcome/mod.rs:1818`). It is meaningless under Claude; hide it when `external_agent` is set.
    - ZDR and SuperGrok gate text (`welcome/mod.rs:873`, `2120`)
    - "Help improve Grok" privacy banner (`views/privacy_banner.rs:12`, gated on xAI's `privacy_notice_rollout`)
- **Q4 (changelog early return):** It does not break `/release-notes`, which fetches on its own (finding 2 is the separate leak). The welcome Changelog row is still sensible: it is drawn whether or not the markdown has arrived (`welcome/mod.rs:1798`), and with a URL set it opens that URL (`app_view.rs:4852-4853`, `4612-4616`). The CDN fetch still runs at startup (`event_loop.rs:1870`); the result is wasted but harmless.
- **Q5 (window title):** `apply_welcome_profile` is called only at startup (`event_loop.rs:1098-1107`); no other non-test caller sets the cells. Titles cannot flip mid-session. Only Pi's pre-profile title `grok-pi — starting Pi` (`app/mod.rs:861`) can carry the old `- grok` suffix before the first real title replaces it.
- **Q6 (tests):** See finding 6.
- **Q7 (premise):** This is what Sab asked for: resume lists Claude sessions, and Claude's logo, name, subtitle and changelog link replace Grok's. It departs from his words in two places: the version is not real (finding 1), and the "generic grok changelog" is still one slash command away (finding 2).
- **Not checked by this review:** the Claude logo has not been seen on screen. The 05:28 note says it was blank at 80×24 in a verify tab. Per AGENTS.md it needs a real-screen check before this counts as done.

## Verdict
FIX-FIRST. Findings 1 and 2 depart from Sab's words. Finding 3 is a flake the new code introduced. Findings 4-6 are not blocking.
