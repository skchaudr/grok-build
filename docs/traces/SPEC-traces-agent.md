# SPEC: Ask, in the traces side panel

Status: rev 4, 2026-11-11. Nothing built. Review of rev 1: `REVIEW-traces-agent.md`.

## What it is

The trace view shows a session's stream; Ask is the F3 panel page beside that stream. You type a question and the agent answers about the session on screen. The answer streams into the panel.

The agent is whatever the pager is already running: `grok-fork` gives the fork's agent on your models, `grok claude` gives Claude, `grok pi` gives Pi. Ask opens one more session on the connection the pager already has. No picker, no second backend, no new config.

## Built from what exists

| Piece | Already in the fork |
|---|---|
| Panel beside the trace | F3 side panel, pages `∿ Traces` `◆ Info` (`views/side_panel.rs`); Ask is a third page |
| Answer shown in the panel | `paint_peek_live_tail` paints any agent's chat into a box (`views/dashboard/peek_tail.rs:49`) |
| Typing the question | the Ask session's own prompt editor (`PromptWidget`), drawn at the bottom of the page |
| Quoting | block viewer: Enter on a line, `v` / Shift-j for several (`viewer.rs` `try_take_idle_enter_quote`, `insert_quoted_reply`) |
| Stepping while quoting | block viewer `←`/`→` |
| Sending without leaving the trace | the dashboard's prompt-to-an-agent-without-attach path (`dispatch/dashboard.rs`) |
| The Ask session | a normal new session on the current connection (`Effect::CreateSession`), resumed by id like any session |
| Answers arriving while you browse | updates already reach off-screen sessions (`dispatch/ctx.rs` `find_agent_by_session_id`) |
| Full history | the Ask session is a normal tab; existing tab switching |
| Reaching it from a live session | follow's `v` already opens the grok view |

## Behaviour

- **Context is the stream.** Every question sent from Ask carries the session on screen: harness, title, transcript file path. The agent reads the file as it stands then, so "what is it doing right now?" works with nothing selected. The Ask page shows this as one line above the input:

  ```
   claude · "make the traces follow view…"
  ╭──────────────────────────────────────────────╮
  │ why did it just retry that?▏                 │
  ╰──────────────────────────────────────────────╯
  ```

- **Narrowing is optional.** Enter in the block viewer (a line, or a `v` / Shift-j selection) adds that quote to the Ask prompt under a line naming its entry; the viewer stays open so you can step and quote more. Nothing is selected automatically.
- **Keys:** F3 focuses the panel. On the Ask page all keys type; Enter sends; Esc returns to the trace. Page switching and resize work from the other two pages.
- **One session per backend,** shared by every trace view: id saved in `~/.grok/traces/ask-session.<agent>`, folder `~/.grok/traces/ask/`. Ctrl+N on Ask starts a new one.
- **Identity** rides in the first message of a new Ask session, so it works the same on every backend:
  > You answer questions about agent session traces. Each question names a session: its harness, title and transcript file. Read the transcript file to answer; it may still be growing. Quoted lines, when present, are what the question is about. Questions are unrelated unless they say otherwise. Be brief.
- The transcript is never pasted in. The agent has its normal tools and opens the file itself.

## New code

1. `Page::Ask`: peek tail on top, the Ask session's prompt below.
2. Keys on Ask go to that prompt; send uses the dashboard path.
3. Every send is headed by the session line (harness, title, transcript path); a trace-view quote goes to the Ask prompt under its entry line.
4. Create-or-resume the Ask session; identity on first send; Ctrl+N.

## Done means

- Unit tests: a send with nothing quoted still carries the session line with the transcript path; a quote lands in the Ask prompt under its entry line; a send carries the Ask session's id; the saved id is reused; keys on Ask reach the editor.
- Builder reads the real screen: a Claude trace opened in `grok-fork traces`, the Ask page showing the session line; then two entries quoted and showing.
- Sab sends the first question and sees the answer stream in beside the trace, once under `grok-fork` and once under `grok claude` or `grok pi`. Until then the report says the answer has not been seen.

## Who decided

Sab: the side panel as the panel to the stream, the session on screen as default context with no auto-selection, quoting from the block viewer, one ongoing session, the agent reads the transcript itself, works under grok-fork, grok claude and grok pi by using what exists. Claude (Fable 5.2): the page name, source-line format, file paths, identity wording and carrying it in the first message, Ctrl+N.
