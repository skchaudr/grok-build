# SPEC: the traces agent (ask about what you are looking at)

Status: rev 2, 2026-11-11, after `REVIEW-traces-agent.md` and Sab's rulings. Nothing built.

## Intent (Sab's words, 2026-11-11)

- "Phoenix provides this too, in each of its traces view, it has a Phoenix chat! ... the agent chat basically automatically picks up the current tool traces as context! ... I can basically turn my traces navigating and exploring into an agentic traces"
- "press enter on a line, or visually select as many lines as I want, and it sends the input to the agent"
- "do we give the agent access to the entire traces transcript? I say no, but I do think it should all be one session across any number of traces views, the agent can be primed and have its identity written out so it can expect one off questions being asked and just holds the right context"
- "for a full session view, I do not think it will make it sense to leave the grok chat at the bottom of a traces run because where will the agent respond"
- "F3 pop up ... plus the popup modal with next and previous, also proved, we can easily create new surfaces and stream text to it"

## What exists (code read 2026-11-11)

- Trace view = grok view: a traced session (grok, Claude, Pi, dpi, Codex) loaded into a real `AgentView` (`app/dispatch/standalone_traces.rs` `open_feed_in`/`open_live_in`). Its composer is hidden (`app/agent_view/role.rs:74`, `ComposerRoute::Hidden` when `trace_return` is set). Correction to an earlier claim in chat: there is no prompt in a trace view to send from.
- Block viewer: Enter on an entry opens it fullscreen; `←`/`→` (`h`/`l`) step entries; `v`/`V` or `Shift-j`/`Shift-k` select lines (`views/list_pane/state/methods.rs:1543`); Enter quotes the line or selection (`viewer.rs` `try_take_idle_enter_quote`) and `insert_quoted_reply` appends it to the prompt, so quotes accumulate.
- F3 side panel: a real layout split beside the chat, pages `∿ Traces` and `◆ Info`, `←`/`→` pages, `z` swaps widths, `+`/`-` resize, Esc hands keys back (`views/side_panel.rs`). One `SidePanel` per `AgentView`.
- ACP updates reach agents that are not on screen (`dispatch/ctx.rs` `find_agent_by_session_id`, test `find_agent_by_session_id_finds_inactive_agent`), so a background grok session keeps streaming while you look at a trace.
- grok loads `AGENTS.md` from a session's folder (`xai-grok-agent/src/prompt/agents_md.rs`).

## Sab's rulings (2026-11-11, after the review)

- The agent is grok-fork's own agent on Sab's models. No Claude or Pi choice, no picker: "any model that I can get in Pi, I can almost certainly get through Grok Fork. So just fucking make it Grok Fork".
- So it is one agent and one session, across every trace view.
- Transcript: "obviously I meant no forced transcript dumping into the agents context. OF COURSE it needs the transcript access." Nothing is dumped; the agent may read the transcript itself.

## Shape

The trace stays the main view; the agent answers in a third F3 page, `✦ Ask`. Sketch, not a screen:

```
 claude · grok-build · 43d0aff9                 │ ∿ Traces  ◆ Info  ✦ Ask
 ◆ Bash  cargo test -p xai-grok-pager    2.1s   │ trace: claude · Bash · ~/.claude/projects/…/43d0aff9….jsonl
 ◆ Read  traces_follow.rs                       │ > error[E0599]: no method named `select_row`
 ◆ Edit  traces_follow.rs                       │ why did this fail?
                                                │
                                                │ It called select_row before the impl ...
                                                │ ╭────────────────────────────────────────╮
                                                │ │ ▏                                      │
                                                │ ╰────────────────────────────────────────╯
```

- Top of the page: the traces agent's latest exchange, painted by the dashboard's `paint_peek_live_tail(&ScrollbackState, area, buf)` (`views/dashboard/peek_tail.rs:49`). It is a tail; `o` on the page switches to the agent's own tab for the full history, and Esc there comes back.
- Bottom: the traces agent's own prompt editor, so a draft survives moving between traces.
- In a trace view, the block viewer's Enter quote (`viewer.rs:1179`) goes to that prompt through the existing `insert_quoted_reply`, headed by one source line: `trace: <harness> · <entry: tool name, Thought, Message, Prompt> · <transcript path>`. The panel opens on Ask; the viewer stays open so `←`/`→` and more Enters add quotes. This also ends today's behaviour, where the quote lands in the hidden composer unseen.
- Keys: F3 focuses the panel. On Ask, every key types into the editor; Enter sends, Opt+Enter is a newline, Esc gives keys back to the trace. Pages and resize (`←`/`→`, `z`, `+`, `-`) work from the other pages only. Ctrl+N on Ask starts a fresh session (the old one stays in grok's session list).
- Follow: `v` already opens the grok view of the followed session; the hint line says `v ask about this`. Follow itself gets no editor.

## The traces agent

- One grok-fork session. Folder `~/.grok/traces/agent/`; its id in `~/.grok/traces/agent-session`. The first send creates it on the pager's existing in-process connection (`Effect::CreateSession`, as any new tab); later launches resume it.
- Identity: `~/.grok/traces/agent/AGENTS.md`, written once if missing, Sab edits it. Placeholder:
  > You answer one-off questions about agent session traces. Each question quotes lines from a trace and names the harness, the entry and the transcript file. Answer from the quote first; read the transcript file when the quote is not enough. Questions are unrelated to each other unless they say otherwise. Be brief.
- Normal tools, default mode. It lives in `app.agents`, so answers stream while you browse (`dispatch/ctx.rs` `find_agent_by_session_id`).
- Standalone `grok-fork traces` skips the login screen (`event_loop.rs:1251`) but its agent thread still authenticates from `auth.json`. If a send fails (not logged in, credit, model), the Ask page shows the agent's error line.

## States

1. Never used: `Enter on a line in the block viewer quotes it here`.
2. Composing: source line, quote, typed text.
3. Running: streaming tail.
4. Answered.
5. Failed: the error line.
6. Narrow (24 cols): editor and the last answer lines still show.

## Build pieces

1. `Page::Ask` in `views/side_panel.rs`: peek tail + the traces agent's prompt, drawn from the trace view's panel rect.
2. Key routing on Ask to that prompt; send dispatches to the traces agent without switching views (the dashboard's prompt-without-attach path).
3. Quote redirect + source line in trace views.
4. Lifecycle: folder, placeholder `AGENTS.md`, create/resume by saved id, Ctrl+N.

## Tests and pass bar

- Unit: quote in a trace view lands in the traces agent's prompt with its source line, never the hidden composer; a send carries the traces agent's session id; the saved id is reused; Ask keys reach the editor.
- Render snapshots for states 1-6.
- Screen, by the builder: open a Claude trace in `grok-fork traces` and in `/traces`, quote two entries, read the Ask page off the pane.
- Pass bar is Sab's: his first send, an answer streaming into the panel beside the trace. Until he has seen that, the report says "answer streaming not seen".

## Not verified

- A session created from standalone traces (never done there before).
- The peek tail at 24-46 columns.
- Whether grok's folder trust asks about `~/.grok/traces/agent/` (standalone marks trust done; `/traces` inside grok does not).

## Who decided

Sab: the Ask panel, grok-fork as the agent, one session, quote by Enter/visual select, transcript readable but never dumped. Claude (Opus 5.5 draft, Fable 5.2 rev 2), not Sab: the `✦ Ask` name, the source-line format, folder and id paths, the placeholder wording, `o` and Ctrl+N, peek tail instead of a scrolling history, follow reaching Ask through `v`.
