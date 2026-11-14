# Review brief: grok claude resume fix + Claude host branding (2026-11-14)

You are the reviewer. REVIEW ONLY: do not edit source files, do not run cargo, do not build, do not kill processes.
Write your findings to `docs/review/claude-host-branding-2026-11-14.md` and nothing else. Report by 15 minutes after you start.

## What Sab said (his words outrank anything an agent wrote)
- "on ssh sab-mini@sab-air the TUI resume does not show anything at all or any sessions" (grok claude)
- "doing whatever we can to personalize ... at the very least adding the logo ... Adding some personality to the TUI. this no longer has to be a Grok built TUI ... it does not have to continually feature Grok, Grok, Grok everywhere"
- "no more generic grok changelog"
- "grok-pi again on ssh sab-mini@sab-air shows how to do it" (Dwsy's grok-pi welcome: logo, name, subtitle line, real version, no changelog dump)
- "no build until all of these changes are made and reviewed by Opus agent"

## Diff to review
- Commit eca9236f (already pushed): `git show eca9236f`
- Uncommitted-then-committed follow-ups after it: `git diff eca9236f HEAD` (hero_box subtitle gate, VERSION_WITH_COMMIT as Claude version, SourceFilter::Grok chip label under a host brand)

## Root cause claimed for resume
The Dwsy merge made `dispatch_show_session_picker` (crates/codegen/xai-grok-pager/src/app/dispatch/session/load.rs) route every external host to the Pi catalog (`FetchExternalSessionCatalog`, External filter). Under `grok claude` that showed "No matches". Fix: `pi_catalog_host(app)` = `app.is_pi_session()` gates the Pi catalog; Claude uses `Effect::FetchSessionList`, whose external_agent branch (effects/mod.rs) sends ACP `session/list`.

## Questions to answer, with file:line evidence
1. Is the root cause right, and is any other `app.external_agent` branch in the picker/resume path still Pi-only (modals.rs, app_view.rs, router.rs, load.rs `session_picker_external_filter_active`, `dispatch_pick_content_session`)? Does Welcome-screen "Resume session" for Claude hit the same path?
2. Do Pi and stock Grok hosts behave exactly as before (overrides are None for them; `welcome_brand_override` must only be Some for Pi-only or Claude-only launches)? Any global-state leak (the override cells are process-wide; a Pi/Claude picker with mixed agent choices)?
3. Branding: list every user-visible "Grok"/"grok" string still reachable under a Claude-only launch (welcome, hero, trust prompt, footer, tips, settings modal, /help, errors, window title, picker chips). For each: fix now, or leave and why. Do not invent features.
4. `ChangelogFetched` early return keys on `welcome_menu_override().changelog_url`: does it wrongly suppress the Grok changelog for Pi, or break `/release-notes`? Is the welcome Changelog row still sensible with a URL?
5. Window-title change in `terminal_title_string`: any caller where `welcome_brand_override` being set mid-session changes titles unexpectedly (Pi sets "grok-pi — starting Pi" separately)?
6. Tests: do the new/changed tests actually pin the behaviour (claude_host_resume_lists_through_acp_not_the_pi_catalog)? What is untested and cheap to cover (hero subtitle with a brand; SourceFilter label with and without brand; title string with brand)?
7. Premise: is this what Sab asked for (Claude host personalised, resume works), or a silent substitution? Name any departure.

## Output format
`docs/review/claude-host-branding-2026-11-14.md`: findings ranked, each with file:line, the failing scenario, and a one-line fix. End with SHIP / FIX-FIRST. Nothing else.
