# SPEC: Ask, in the traces side panel

Status: rev 3, 2026-11-11. Nothing built. Review of rev 1: `REVIEW-traces-agent.md`.

## What it is

While looking at a trace, quote lines from it and ask an agent about them. The answer streams into the F3 panel beside the trace.

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

- **Quote:** in a trace view, Enter in the block viewer puts the quote in the Ask prompt, under one line naming the harness, the entry and the transcript file. The panel opens on Ask. The viewer stays open, so you can step and quote more.
- **Ask:** F3 focuses the panel. On the Ask page all keys type; Enter sends; Esc returns to the trace. Page switching and resize work from the other two pages.
- **One session per backend,** shared by every trace view: id saved in `~/.grok/traces/ask-session.<agent>`, folder `~/.grok/traces/ask/`. Ctrl+N on Ask starts a new one.
- **Identity** rides in the first message of a new Ask session, so it works the same on every backend:
  > You answer one-off questions about agent session traces. Each question quotes lines from a trace and names the harness, the entry and the transcript file. Answer from the quote first; read the transcript file when the quote is not enough. Questions are unrelated unless they say otherwise. Be brief.
- The transcript is never pasted in. The agent has its normal tools and can open the file.

## New code

1. `Page::Ask`: peek tail on top, the Ask session's prompt below.
2. Keys on Ask go to that prompt; send uses the dashboard path.
3. Trace-view quote goes to the Ask prompt with its source line.
4. Create-or-resume the Ask session; identity on first send; Ctrl+N.

## Done means

- Unit tests: the quote lands in the Ask prompt with its source line; a send carries the Ask session's id; the saved id is reused; keys on Ask reach the editor.
- Builder reads the real screen: a Claude trace opened in `grok-fork traces`, two entries quoted, Ask page showing them.
- Sab sends the first question and sees the answer stream in beside the trace, once under `grok-fork` and once under `grok claude` or `grok pi`. Until then the report says the answer has not been seen.

## Who decided

Sab: the side panel, quoting from the block viewer, one ongoing session, the agent reads the transcript itself, works under grok-fork, grok claude and grok pi by using what exists. Claude (Fable 5.2): the page name, source-line format, file paths, identity wording and carrying it in the first message, Ctrl+N.
