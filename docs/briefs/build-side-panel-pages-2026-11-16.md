# Build: F3 side panel pages (2026-11-16)

Status: dispatched 2026-11-16 as Rex tab `side-panel-lead`.

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

## Pages (drawn from Sab's list above)

1. Session: everything known about this session (id, start time, cwd, branch, model, effort, mode, context,
   tokens, cache read/write and hit share, rate limits, status).
2. Steps: this session by turn; one line per step, numbered consecutively within the turn, with duration. Nothing
   expands in place.
3. Jobs and agents: background commands and subagents, each with state, model and elapsed time.
4. Recent: recent sessions, newest first; Enter opens one.

Every row on every page can be moved to with the arrow keys and copied with `y`.

## How the run is shaped

Sab, 2026-11-16: "I don't want those three sub agents all testing and building ... the one Sonnet agent is going to
be the only one who tests and builds, and it can resume and send work back to those agents. They must be resumable"

- You are the one Sonnet 5.5 lead. You are responsible for three Sonnet 5.5 builders and for everything they write.
- Builders must be resumable: start each once and keep sending work and fixes back to that same agent (SendMessage,
  or a named interactive Rex tab you type into). Never replace a builder with a fresh one to deliver a fix.
- Builders run no cargo at all: no build, test or check. They review their change from code and write tests without
  running them. Only you run cargo.
- You own the shared parts (page enum, navigation, row cursor, copy) and land them first so builders do not collide.
- Each page gets a render snapshot test at half width, for its empty and filled states.
- One cycle at the end, in the existing `target` folder: targeted `cargo test --release` for the pager crate, send
  failures back to the builder that owns them, then `scripts/install-fork.sh` once.
- After install, open the panel in `grok claude` on sab-air through Rex, capture each page, and look at it in every
  shipped theme (`GROK_THEME=<name>`). Send no model turn.
- Everyone commits and pushes their own work to main with the trailer `Co-authored-by: claude-code sonnet-5.5`.
- No process kills by name, `ps -o pid,comm` only, nothing CPU-heavy on sab-air, never tmux.
- Report to `docs/BUILD-side-panel-pages-2026-11-16.md`: each page's real capture, what landed, what did not.
