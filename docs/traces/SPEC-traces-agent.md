# SPEC: Ask, in the traces side panel

Status: rev 5, 2026-11-11. Nothing built. Rev 4 was reviewed against the code by Claude Opus 5.5 (`REVIEW-traces-agent-rev4.md`); rev 5 takes all 15 findings. Paths are under `crates/codegen/xai-grok-pager/src/`.

## What it is

The trace view shows a session's stream; Ask is the F3 panel page beside that stream. You type a question and the agent answers about the session on screen. The answer streams into the panel.

The agent is whatever the pager is already running: `grok-fork` gives the fork's agent on your models, `grok claude` gives Claude, `grok pi` gives Pi. Ask opens one more session on the connection the pager already has. No picker, no second backend, no new config.

## Built from what exists

| Piece | Already in the fork |
|---|---|
| Panel beside the trace | F3 side panel, split off before the chat draws (`app/agent_view/render.rs:587`); Ask is a third `Page` (`views/side_panel.rs:27`) |
| Typing the question | the trace view's own prompt editor. Every trace view is an `AgentView` with a `PromptWidget` that is only hidden (`app/agent_view/role.rs:73`); Ask draws it in the page. The draft belongs to the trace it is about |
| Quoting | block viewer Enter / `v` / Shift-j (`app/agent_view/viewer.rs:1093`); `insert_quoted_reply` already writes to that same prompt (`:1115`) |
| Stepping while quoting | `step_block_viewer` (`viewer.rs:1067`) |
| Sending without leaving the trace | `agent.session.enqueue_prompt(text)` (`app/agent.rs:970`) then `queue::maybe_drain_queue_and_note_peek(app, ask_id)` (`app/dispatch/queue.rs:1113`): works on any agent by id, and holds the prompt while the session starts or is busy |
| The Ask session | `Effect::CreateSession` on the pager's one connection, any backend (`app/effects/mod.rs:252`); `Effect::LoadSession` to resume (`:588`) |
| Answer shown in the panel | `paint_peek_live_tail(&ScrollbackState, rect, buf)`, a pure read that works for any agent (`views/dashboard/peek_tail.rs:49`); shows the current turn |
| Answers arriving while you browse | updates are routed by session id whatever view is active (`app/dispatch/ctx.rs:332`) |
| Full history | `Ctrl+\` opens the dashboard, where the Ask session is a row |
| Reaching it from a live session | follow's `v` opens the trace view (`views/traces_modal/deck.rs:107`) |

## Behaviour

- **Context is the stream.** Every question carries the session on screen. The page shows it as one line above the input:

  ```
   claude · "make the traces follow view…"
  ╭──────────────────────────────────────────────╮
  │ why did it just retry that?▏                 │
  ╰──────────────────────────────────────────────╯
  ```

- **What is sent,** in this order (the panel pins the first line, so the question leads):

  ```
  <the question as typed, quotes included where they were inserted>

  session: <harness> · <title>
  transcript: <path>
  ```

  `<path>` is the local file the view was read from; for a grok row, the session folder; for a live row on another machine, `<host>:<remote path> (not on this machine)`. The first message of a new Ask session ends with the identity text.
- **Quotes narrow, optionally.** In a trace view, Enter in the block viewer inserts the quote into the prompt under `from: <entry>`, where `<entry>` is the tool name and target (`from: Bash cargo test -p …`), or `Thought`, `Message`, `Prompt`. The viewer stays open, visual mode is cleared, and focus does not move, so `←`/`→` and more Enters keep working. Nothing is selected automatically.
- **Keys.** F3 focuses the panel. On Ask, keys and pastes go to the prompt, Ctrl editing chords included; Enter sends; Esc gives keys back to the trace and does not leave it. `←`/`→` switch pages while the prompt is empty. Ctrl+N starts a new Ask session.
- **One session per backend,** shared by every trace view. Id in `~/.grok/traces/ask-session.<agent>`, where `<agent>` is `app.external_agent_label` or `fork` when there is none. Folder `~/.grok/traces/ask/`. If resuming the saved id fails, a new session replaces it.
- **Permissions.** The Ask session starts in the pager's default permission mode, like any new session. If it still raises a permission or question card, the page shows `Ask is waiting for your answer · Ctrl+\ to open it`.
- **Failure.** A create, load or send error (not logged in, credit, model) shows as the page's one line.
- **Identity:**
  > You answer questions about agent session traces. Each question names a session: its harness, title and transcript file. Read the transcript file to answer; it may still be growing. Quoted lines, when present, are what the question is about. Questions are unrelated unless they say otherwise. Be brief.
- The transcript is never pasted in. The agent has its normal tools and opens the file itself.

## New code

1. **Row kept on the view.** `open_feed_in` / `open_live_in` (`app/dispatch/standalone_traces.rs`) store the `SessionRow` and the local path they read on the `AgentView`. `clear_trace_content` also clears the prompt (standalone reuses one view).
2. **`Page::Ask`.** Session line, the view's prompt, and a body rect the panel records. After `agent.draw` returns (`app/app_view.rs:4843`), AppView paints the Ask agent's tail into that rect. Before the first answer the body shows `Ask about this session. Enter in the block viewer adds a quote.`
3. **Keys.** On Ask, the panel hands keys and pastes to the prompt before the Ctrl fall-through (`views/side_panel.rs:119`); Enter returns an Ask-send `Action` with the text; Ctrl+N is handled by Ask. The trace-view Esc guard (`app/app_view.rs:2721`) also requires `!a.side_panel.has_keys()`.
4. **Quote in a trace view.** Does not dismiss the viewer (`viewer.rs:1111`), clears visual mode, leaves the pane as it is (`:1146`), adds the `from:` line, opens the panel on Ask.
5. **Ask session.** On first send: `create_dir_all` the folder; create through `dispatch_new_session_inner_with_id` with the `UnregisterActiveSession` dropped and the `CreateSession` cwd and the agent's `session.cwd` patched to the Ask folder (as `set_create_permission_mode` patches effects, `app/dispatch/dashboard.rs:684`), or load with `session_cwd = Some(folder)`; then set `app.active_view` back to the trace view. Save the id on `handle_session_created`. The send enqueues and drains as in the table.

## Done means

- Unit tests: a send with nothing quoted carries the question first, then the session and transcript lines; a quote lands in the trace view's prompt under its `from:` line with the viewer still open; a send goes to the Ask agent's session id and the view stays on the trace; the saved id is reused; Esc on a focused Ask page stays in the trace; a blocking card shows the waiting line.
- Render snapshots: empty, composing with a quote, answering, waiting line, error line, 24 columns.
- Builder reads the real screen: a Claude trace opened in `grok-fork traces` and through `/traces`, the Ask page with its session line, two entries quoted. The page is looked at in every shipped theme (`GROK_THEME=<name>`).
- Sab sends the first question and sees the answer stream in beside the trace, once under `grok-fork` and once under `grok claude` or `grok pi`. Until then the report says the answer has not been seen.

## Who decided

Sab: the side panel as the panel to the stream, the session on screen as default context with no auto-selection, quoting from the block viewer, one ongoing session, the agent reads the transcript itself, works under grok-fork, grok claude and grok pi by using what exists. Claude (Fable 5.2 spec, Opus 5.5 review): the page name, the sent format and `from:` line, file paths, identity wording and carrying it in the first message, Ctrl+N, `←`/`→` only on an empty prompt, the waiting line rather than auto-approving the Ask session.
