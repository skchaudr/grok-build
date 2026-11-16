# Build: F3 side panel pages (2026-11-16)

Status: DRAFT, not dispatched. Sab confirms the page list before launch.

## Intent (Sab's words, 2026-11-16)

> eight panels is like maybe a bit too much. Having like four panels that each do something very well is much better.
> Like recent sessions, current session information, like just fill it out with as much information. Session ID,
> start date. Models, sub-agents, context.

> the traces in the side panel is not good. Like the way DSH TUI does it, they don't expand anything. They keep it
> super short, like minimal one line. They do it by session, like by each agent turn, they show each step, they count
> consecutively the number of steps per turn. They have like a time duration trace

> we have Phoenix traces and we have Grok traces, so that's like two solid panels right there, maybe three.

> background tasks, background jobs, subagents ... just make like a few really good panels that are detailed and
> navigable.

> every single thing in the scrollback can be selected and copied ... you press the letter Y, it will copy the whole
> line ... anytime you open up something in a pop-up modal you can visually select it. So basically, just about
> everything in these panels should be selectable and copyable.

> dispatch one task to one Sonnet agent ... It is going to use three sub agents, and those three sub agents it is
> responsible for.

## What exists

- `crates/codegen/xai-grok-pager/src/views/side_panel.rs` (712 lines): the F3 panel, a real layout split with two
  pages, `Page::Traces` and `Page::Info`. F3 toggles, Esc returns to chat, left/right switch pages, `z` zooms,
  `+`/`-` resize. `Info` already carries title, session id, cwd, model, effort, mode, context, Claude rate limits,
  status, jobs, subagents.
- `docs/SCOUT-dsh-tui-panel-2026-11-13.md`: all eight dsh-tui panels as captured on screen. Reference for density
  and the one-line trace style, not a layout to copy.
- Keys: read the pager's key handling and `docs/TOOL-traces-modal.md` first. Reuse existing keys; `y` already
  copies a line in the scrollback and modals already have visual select. Find that code and reuse it.

## Pages (lead's proposal, Sab to confirm)

1. Session: everything known about this session (id, start time, cwd, branch, model, effort, mode, context,
   tokens, cache read/write and hit share, rate limits, status).
2. Steps: this session by turn; one line per step, numbered consecutively within the turn, with duration. Nothing
   expands in place.
3. Jobs and agents: background commands and subagents, each with state, model and elapsed time.
4. Recent: recent sessions, newest first; Enter opens one.

Every row on every page can be moved to with the arrow keys and copied with `y`.

## How the run is shaped

- One Sonnet 5.5 lead in a visible Rex tab. It reads this brief, splits the pages across three Sonnet 5.5 builders,
  each in its own named visible Rex tab with a short committed brief of its own. No hidden subagents.
- The lead owns the shared parts (page enum, navigation, row cursor, copy) and lands them first so the builders
  do not collide.
- Builders run no cargo. The lead runs one targeted `cargo test --release` for the pager crate at the end.
- Each page gets a render snapshot test at half width, for its empty and filled states.
- Each agent commits and pushes its own work with the trailer `Co-authored-by: claude-code sonnet-5.5`.
- No model turns inside any grok TUI, no process kills by name, `ps -o pid,comm` only, nothing CPU-heavy on sab-air.
- The lead does not install. It reports to `docs/BUILD-side-panel-pages-2026-11-16.md` with what landed and what
  did not; install and the on-screen check in every theme follow from the main session.
