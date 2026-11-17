# One resume list for every host and folder (2026-11-17)

Read `AGENTS.md`, `.pi/agents/delivery-policy.md` and `docs/EXEC-usage-and-running-calls-2026-11-16.md` section E first; they apply to you.

## Intent (Sab's words, 2026-11-17)

> on here and sab-air its a roll of dice whether or not grok grok-pi or grok claude will be able to resume a session

> I either get an error before the session opens, I struggle to even find the session in the first place

> we probably would want those sessions available and resumable across machines

Proposed to him and answered "Go": "One list for every host and folder. Any picker shows Claude, Pi and Grok sessions together, grouped by folder and tagged with the host. Picking a row from another host reopens under that host; `grok --resume <id>` already does exactly this, so the picker would hand off to it."

Other machines are a later step and not in this brief.

## What is true today (measured 2026-11-17, Mini)

- `grok claude` picker: ACP `session/list`, all folders, 167 of 177 Claude sessions (`app/effects/mod.rs:803-823`). Only Claude rows.
- `grok grok-pi` / `grok pi` picker: Pi catalog over `pi/session/list`. Start screen asks for every folder since `cd7e8e35`; the in-session `/resume` modal has two tabs, current folder and all (`app/dispatch/session/load.rs:1886-1990`, `app/app_view.rs:2260`). Only Pi rows.
- Plain `grok`: `x.ai/session/list` for the current folder, limit 30 (`app/effects/mod.rs:824`), plus a scan of other tools' stores (`app/foreign_sessions`, started in `app/dispatch/session/foreign.rs:100-115`). That scan is switched off when the host is Claude or Pi. Its Claude rows resume by importing through a skill prompt, which is a paid model turn.
- `~/.grok-hub/scripts/hub/gb --resume <id>` (source `scripts/hub/gb`) finds an id in any store, changes to the session's folder and starts the right host. `GB_PRINT_CMD=1` prints the command instead of running it. The pager is started with `GROK_RESUME_CMD="grok <agent>"`.
- A `~/.grok/sessions/<cwd>/<id>/` folder holding only `images/` is not a Grok session (see `5d8f3408`).

## The change

In every host (plain, Claude, Pi), in both pickers (start screen and the in-session `/resume` modal):

1. The list holds this host's own sessions as today, plus the sessions of the other two hosts read from their stores on disk: Claude `~/.claude/projects/*/<id>.jsonl`, Pi `~/.pi/agent/sessions/**/*_<id>.jsonl`, Grok `~/.grok/sessions/<cwd>/<id>/` with a `chat_history.jsonl`. Every folder, grouped by folder as the picker already groups, newest first inside a group.
2. Each row from another host carries a visible host tag (`claude`, `pi`, `grok`). Own-host rows look as they do today.
3. Enter on an own-host row loads it as today. Enter on another host's row hands off: the pager leaves the screen cleanly and replaces itself with `gb --resume <id>`. It never imports the session through a model prompt.
4. Reuse the existing foreign-scan lane and the existing source filter (`f`). Do not add a second list, a second picker or new keys.

## Every state (each needs a test; picker render tests at the widths the picker tests already use)

1. Own rows only (other stores empty or missing): the list is as today.
2. Mixed: own and other-host rows in one folder group, ordered by time, tags on the other-host rows only.
3. A folder that only has other-host sessions gets its own group.
4. The same id in two stores (a `grok claude` session with an images-only Grok folder): one row, the real host's.
5. The other-host scan is still running when the own list arrives: own rows show at once, the others join without moving the cursor off the selected row.
6. The other-host scan fails or a store is unreadable: own rows still show; no error modal.
7. Enter on another host's row with `gb` reachable: the handoff command is exactly `gb --resume <id>`, issued after the terminal is restored.
8. Enter on another host's row when `gb` cannot be found (`GROK_PICKER_DIR` unset and no `gb` on `PATH`): the picker stays open and says which command to run by hand.
9. Enter on another host's row while this session has a turn running: no handoff; the picker says a turn is running.
10. Source filter `f`: each existing filter value still shows what its name says; say in the report what each one shows now.
11. Search (`/`) matches other-host rows by title.
12. Narrow picker: the title truncates, the host tag and the age do not.

## Builder rules

- Edit and commit only; run no cargo. Review your change from the code. Write the tests; the lead's single test run will run them.
- Reuse grok-build's keys and picker code. Colours come from theme fields the picker already uses.
- Never tmux, no kills by name, no subagents, nothing on sab-air, no model turns, no `grok-fork <word>` launches (a bare word is sent as a prompt).
- Commit and push with the trailer `Co-authored-by: claude-code sonnet-5.5`. Report in `docs/BUILD-one-resume-list-2026-11-17.md`: files touched, how each store is read, how the handoff leaves the terminal, tests written, anything here you could not do as written.

## Spec reviewer (runs before the builder; a different agent)

Read this brief and the code it names. Do not edit code; run no cargo. Write `docs/REVIEW-brief-one-resume-list-2026-11-17.md`: states missing or ambiguous; what the foreign-scan lane already gives (stores, row fields, what Enter does on its rows today, with file and line) and what is missing for Pi and Grok stores; whether a clean exec handoff is possible from where the picker handles Enter (file and line) and what must be torn down first; what the `/resume` modal's Pi tabs should become; anything here that contradicts Sab's quoted words or existing behaviour he relies on. Per `.pi/agents/delivery-policy.md`: evidence, not a veto; zero findings is a valid result. Commit and push with the same trailer.
