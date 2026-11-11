# SPEC: the traces agent (ask about what you are looking at)

Status: draft for Sab's review, 2026-11-11. Nothing built.

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

## Shape

The trace stays the main view; the agent answers in a third F3 page, `✦ Ask` (Phoenix's layout: trace left, chat right). Sketch, not a screen:

```
 claude · grok-build · 43d0aff9                 │ ∿ Traces  ◆ Info  ✦ Ask
 ◆ Bash  cargo test -p xai-grok-pager    2.1s   │ trace: claude 43d0aff9 · Bash · grok-build
 ◆ Read  traces_follow.rs                       │ > error[E0599]: no method named `select_row`
 ◆ Edit  traces_follow.rs                       │ why did this fail?
                                                │
                                                │ It called select_row before the impl ...
                                                │ ╭────────────────────────────────────────╮
                                                │ │ ▏                                      │
                                                │ ╰────────────────────────────────────────╯
```

- The Ask page shows the traces agent's own scrollback (its answers, streamed) above an input editor.
- In a trace view, the block viewer's Enter quote goes to the Ask input instead of the hidden composer, opens the panel on Ask if closed, and leaves the viewer open, so `←`/`→` and more Enters keep adding quotes. Each quote is headed by one source line: `trace: <harness> <short id> · <entry: tool name, Thought, Message, Prompt> · <project>`.
- Sending happens only from the Ask input (Enter sends, Opt+Enter newline, as the main composer). F3 focuses the panel; Esc returns keys to the trace.

## The traces agent

- One grok session for all trace views. Folder `~/.grok/traces/agent/`, id kept in `~/.grok/traces/agent-session`; the first send creates it, later sends resume it.
- Identity: `~/.grok/traces/agent/AGENTS.md`, a short placeholder Claude writes and Sab edits: it answers one-off questions about quoted trace excerpts, says when the quote is not enough, and does not go looking for the transcript.
- Context is only what is quoted plus the source lines. No transcript path is sent.
- It lives in `app.agents` like any session, so its answers arrive while you browse.

## States (each gets a render snapshot test)

1. Ask page, agent never used: one line, `Enter on a line in the block viewer quotes it here`.
2. Composing: quotes with source lines, then typed text.
3. Running: streaming answer, running clock as grok's.
4. Answered: history scrolls with the panel's keys.
5. Failed: the agent's error line (auth, credit, model) in the page; the trace is unaffected.
6. Narrow: panel at its 24-col minimum still shows input and the last answer lines; `z` widens it.
7. Standalone `grok-fork traces` and `/traces` inside grok behave the same.

## Build pieces

1. `Page::Ask` in `views/side_panel.rs`.
2. Draw the traces agent's chat (scrollback + editor) into the panel rect of the trace view: the one new mechanism, since today a view draws only its own agent.
3. Key routing to the Ask input when the panel is focused.
4. Trace-view quote redirect plus source line.
5. Traces-agent lifecycle: create or resume by saved id, folder, placeholder `AGENTS.md`.

## Unknown until built

- Whether grok's folder-trust prompt blocks `~/.grok/traces/agent/` or its `AGENTS.md`.
- Whether piece 2 can reuse `draw_chat` at panel width or needs a smaller chat draw.
- Whether a grok session can run with tools off; `SessionMode::Ask` exists but what it allows is not read yet.

## Tests

- Render snapshots for states 1-7.
- Quote in a trace view lands in the Ask input with its source line, never in the hidden composer.
- A send uses the traces agent's session id, never the traced session's.
- The saved id is reused on the second send.
- Screen check: open a Claude trace, quote two entries, read the Ask input off the pane; stop before send (paid turn; the first send is Sab's).

## Decisions for Sab

- Layout: trace left, Ask in the F3 panel (recommended), or the traces agent full width with the trace in the panel.
- Tools: none (read-only answers from quotes) or grok's normal tools.
- Scope: trace views only, or also the Ask page in ordinary grok sessions (asking about your own session's calls).
- The list/follow modal (no editor today) stays out of this slice.

## Who decided

Claude Opus 5.5, not Sab: the `✦ Ask` name, the source-line format, the folder and id file paths, quotes leaving the viewer open, the placeholder identity wording, the test list.
