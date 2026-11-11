# REVIEW: SPEC-traces-agent.md rev 4

Reviewer: Claude Opus 5.5, 2026-11-11. Code read, nothing run. Paths are under `crates/codegen/xai-grok-pager/src/` unless they say otherwise.

Verdict: ready after the changes listed. The shape holds. Two table rows (typing, sending) cite code that does not do what the spec relies on, and the trace view does not have the session line's data.

## Findings

### 1. The "send" row points at a path that makes a new session and leaves the trace (high, verified in code)
Spec: sending without leaving the trace is "the dashboard's prompt-to-an-agent-without-attach path (`dispatch/dashboard.rs`)".
Code: `dispatch_dashboard_dispatch` makes a **new** session on every send (`app/dispatch/dashboard.rs:1111`), reads and clears the dashboard's own `dispatch` widget (`:1098-1108`, `:1125-1126`), and with `attach=false` sets `app.active_view = ActiveView::AgentDashboard` (`:1139-1140`), which takes you off the trace. Nothing in dashboard.rs sends to an agent that already exists.
What does exist: `agent.session.enqueue_prompt(text)` (`app/agent.rs:970`), then `queue::maybe_drain_queue_and_note_peek(app, ask_id)` (`app/dispatch/queue.rs:1113`). It works on any agent by id, active or not. The drain needs a bound session id, an idle state and no replay loading (`queue.rs:335-375`). Otherwise the prompt stays queued, and `handle_session_created` drains it once the session exists (`app/dispatch/session/lifecycle.rs:1486`). That covers "send before the session exists" for free, as the dashboard relies on at `dashboard.rs:1114`.
Fix: change the row to "enqueue on the Ask agent, then `maybe_drain_queue_and_note_peek` (`queue.rs:1113`); it queues while the Ask session starts or is busy".

### 2. The Ask session's prompt cannot be drawn or fed keys from the side panel, and a shared draft sends quotes under the wrong session (high, verified in code)
Spec: "the Ask session's own prompt editor (`PromptWidget`), drawn at the bottom of the page"; "keys on Ask go to that prompt"; a quote "goes to the Ask prompt".
Code:
- The panel is drawn inside `AgentView::draw(&mut self, ..)` (`app/agent_view/render.rs:571-607`, via `draw_side_panel`, `app/agent_view/side_panel.rs:67-73`).
- Panel keys are handled inside the trace view's `handle_input` (`app/agent_view/input.rs:366-372`).
- In both places the caller holds `agents.get_mut(&id)` for the trace view (`app/app_view.rs:2732`, `:4821`), so another agent's `prompt` cannot be reached.
- `insert_quoted_reply` writes to `self.prompt`, the trace view's own prompt (`app/agent_view/viewer.rs:1115-1147`).
- The Ask agent is shared by every trace view, so a draft in its prompt carries quotes from trace A into a send headed by trace B's session line.

Simpler route already in place: every trace view is an `AgentView` with its own `PromptWidget`, hidden only by `ComposerRoute::Hidden` (`app/agent_view/role.rs:73-76`). Use that prompt as the Ask input:
- drawing and keys stay on `&mut self`;
- `insert_quoted_reply` works as it is;
- the draft belongs to the trace whose session line heads it.

Only two steps cross to the Ask agent, and both happen at AppView level:
- send: Enter returns an `Action` with the text, which finding 1's enqueue path handles;
- the answer paint: after `agent.draw` returns (`app_view.rs:4843-4874`), paint `paint_peek_live_tail(&agents[&ask].scrollback, rect, buf)` over the panel body rect the panel recorded.

Standalone reuses one AgentView for every trace (`app/dispatch/standalone_traces.rs:176-178`), and `clear_trace_content` does not clear the prompt (`app/agent_view/session.rs:651-659`). Add `self.prompt.set_text("")` there.
Fix: change the "Typing the question" row and New code 1-3 to say: the trace view's own hidden prompt is drawn in the page; Enter emits an Ask-send action; the answer tail is painted at AppView level.

### 3. The trace view does not keep the harness, title or transcript path (high, verified in code)
Spec: every send carries "harness, title, transcript file path" of the session on screen.
Code: `SessionRow` has `source`, `title` and `file_path` (`crates/codegen/xai-grok-traces/src/query.rs:104-106`). Neither opening function keeps the row on the AgentView:
- `open_feed_in` uses it only for an error message, `cwd` and the live worker (`standalone_traces.rs:170`, `:193-195`, `:210`);
- `open_live_in` moves `source` and `file_path` into the worker's `Job` (`:289-300`), and `TraceLive` keeps only `session_id`, `life`, `running` and `phoenix_offline` (`acp/live_feed.rs:64-71`).

The AgentView holds `trace_return`, `trace_origin` and `trace_live` (`app/agent_view/mod.rs:1413-1417`), and none of them carries the row.
The right path also differs by row:
- a row from another machine must use the local copy (`traces_live::local_copy`, `app/dispatch/traces_live.rs:172-179`, as used at `standalone_traces.rs:92`, `:115`);
- a live remote row has no local transcript at all (`standalone_traces.rs:252-256`);
- for grok rows, `file_path` is `chat_history.jsonl`, but the stream is replayed from `updates.jsonl` in the same folder (`standalone_traces.rs:112-113`, `:120-131`).

Fix: add to New code: "`open_feed_in` / `open_live_in` store `trace_row: Option<SessionRow>` plus the local path they read on the AgentView". State which file a grok row sends (the session folder or `updates.jsonl`), and what a live remote row sends (host and remote path, marked not local).

### 4. Esc on a focused panel in a trace view leaves the trace (high, verified in code)
Spec: "Esc returns to the trace."
Code: before any agent key handling, AppView turns Esc into `Action::TracesLeaveView` whenever the trace view `is_bare_scrollback()` (`app/app_view.rs:2717-2730`). `is_bare_scrollback` does not check the side panel (`app/agent_view/input.rs:65-79`), and trace views always sit in the scrollback pane (`standalone_traces.rs:191`, `:245`). Esc on the Ask page therefore drops you back to the sessions list. Esc on the existing pages does the same today.
Fix: add to New code: "the trace-view Esc guard at `app_view.rs:2721` also requires `!a.side_panel.has_keys()`".

### 5. The Ask page has no way out, and Ctrl+N does not reach it (high, verified in code)
Spec: "On the Ask page all keys type; Enter sends; Esc returns to the trace. Page switching and resize work from the other two pages." "Ctrl+N on Ask starts a new one."
Code:
- Pages change only on `←`/`→` (`views/side_panel.rs:121-130`), and `toggle()` keeps `page` (`:99-109`). Once on Ask, every later F3 focuses Ask again, and there is no way back to Traces or Info.
- Every Ctrl chord returns `false` and falls through to the chat (`views/side_panel.rs:119`). Ctrl+N is the global `NewSession` (`actions/defaults.rs:766-776`, `When::Always`), resolved for unconsumed keys at `app/app_view.rs:3005-3009`. Ctrl+N on Ask would arm and then open an ordinary new session, leaving the trace.
- The prompt's own Ctrl editing chords also never reach it.

Fix: state the rule. Smallest version: on Ask, `←`/`→` switch pages while the prompt is empty. Ask handles Ctrl+N itself, and every other key goes to the prompt before the Ctrl fall-through.

### 6. The create and load helpers switch view, take the traced session's folder, and unregister the trace id (high, verified in code)
Spec: "a normal new session on the current connection (`Effect::CreateSession`), resumed by id like any session"; folder `~/.grok/traces/ask/`.
Code: the effect is fine (finding list below). The dispatchers that emit it are not, as they stand:
- `dispatch_new_session_inner_with_id` takes the active agent's cwd, which in a trace view is the traced session's cwd (`lifecycle.rs:395-402`; set at `standalone_traces.rs:193-195`). It emits `UnregisterActiveSession` for the trace's bound id (`lifecycle.rs:403`) and calls `switch_to_agent` to the new agent (`:464`).
- `dispatch_load_session` also switches view (`app/dispatch/session/load.rs:119`, `:301`). It defaults the cwd to `app.cwd` (`load.rs:190`; `app/effects/mod.rs:580`), so a session made in the Ask folder is loaded from the wrong folder unless `session_cwd` is passed. Claude keys its stored sessions by project folder (inferred).

Fix: one line in New code 4:
- create through `dispatch_new_session_inner_with_id`, then drop the `UnregisterActiveSession`;
- patch the returned `CreateSession`'s `cwd` and the agent's `session.cwd` to the Ask folder, the same way `set_create_permission_mode` patches effects (`dashboard.rs:684-703`);
- load with `session_cwd = Some(ask folder)`;
- after either, set `app.active_view` back to the trace view;
- `create_dir_all` the folder first;
- if the load fails, create a new session and overwrite the saved id.

### 7. Permission and question cards from the Ask session cannot be seen, so its turn stalls (high, cards verified in code, Claude prompting inferred)
Spec: no rule.
Code: permission and question cards live on the asking agent's own `permission_queue` and `question_view` (`app/agent_view/key_owner.rs:97-108`) and are drawn by that agent's own view. `paint_peek_live_tail` paints scrollback entries only (`views/dashboard/peek_tail.rs:137-149`). The Ask agent is never the drawn view, so a card it raises is never shown.
Inferred: Claude asks before reading a file outside its working folder, and every transcript lives outside `~/.grok/traces/ask/`, so on Claude the first Read of a transcript raises a permission card.
Fix: one rule. Smallest: while the Ask agent has a `blocking_card()`, the page shows one line, "Ask is waiting for your answer · Ctrl+\ to open it" (the dashboard key, `actions/defaults.rs:895-898`). The other existing knob is `permission_mode_override` on `CreateSession` (`app/effects/mod.rs:256`, set by the dashboard at `dashboard.rs:684-703`). Sab picks which.

### 8. The panel pins the first line of the sent text, so the question is never shown (medium, verified in code)
Spec: every send is headed by the session line; the first send also carries the identity text.
Code: the tail pins only the first line of the last user prompt (`peek_tail.rs:66-71`, `.next()`) and shows only what comes after it (`:59-63`). The question line itself is never painted. The first ask pins "You answer questions about agent session traces…", and every later ask pins the session line.
Fix: put the question first, then the quotes, then the session line, then the identity text on the first send.

### 9. Keeping the block viewer open after a quote changes existing code the spec does not list (medium, verified in code)
Spec: "the viewer stays open so you can step and quote more."
Code:
- `try_take_idle_enter_quote` closes the viewer (`app/agent_view/viewer.rs:1111`);
- `insert_quoted_reply` moves focus to the prompt pane (`:1146`), which in a trace view is hidden, and which also defeats the Esc guard in finding 4;
- `←`/`→` stepping works only when visual mode is off (`:1162`), so after a `v` quote it stops until visual mode is cleared.

Fix: add to New code 3: "in a trace view, quoting does not dismiss the viewer, clears visual mode, and leaves the pane as it is".

### 10. `ask-session.<agent>` does not name its key (medium, verified in code)
Spec: id saved in `~/.grok/traces/ask-session.<agent>`.
Code: the pager already has the label: `app.external_agent_label` (`app/app_view.rs:611`), taken from the agent command by `agent_label_from_cmd` (`slash/external_common.rs:23`). It is `None` under the fork's own agent.
Fix: "`<agent>` is `external_agent_label`, or `fork` when there is none".

### 11. Standalone traces skips login, so the first Ask under grok-fork can fail with no message on the page (medium, the skip is verified in code, the failure is inferred)
Code: `start()` forces login, consent and folder trust to done (`standalone_traces.rs:49-51`), and the event loop skips interactive login in standalone (`app/event_loop.rs:1250-1251`). This is rev 1's finding 2, and rev 4 does not answer it. If the fork agent is not logged in, `session/new` fails and the error goes to the Ask agent, which is off screen.
Fix: "a create or load failure shows its error as the page's one line".

### 12. Paste does not reach the panel (medium, verified in code)
Code: the panel routes key events only (`app/agent_view/input.rs:364-372`). A paste goes down the trace view's ordinary paste path.
Fix: say that a paste on a focused Ask page goes to its prompt. With finding 2's route, that is the trace view's own prompt.

### 13. Done means has no theme check (medium, verified in AGENTS.md)
`AGENTS.md`: every change to a TUI screen's colouring must be looked at in every shipped theme (`GROK_THEME=<name>`). The Ask page is a new screen.
Fix: add "the Ask page, with a quote and an answer, looked at in every shipped theme".

### 14. The sent header and the quote's "entry line" have no exact format (low, inferred)
The mock shows `claude · "title"` on screen; the sent header (which includes the path) and the line naming a quoted entry have no exact text. A builder would invent both.
Fix: give both strings literally. The entry line can be built from what the viewer has (`viewer.kind` and the entry's tool and target).

### 15. "Full history: the Ask session is a normal tab" (low, verified in code)
The pager has no tabs; other sessions are reached through the dashboard, `Ctrl+\` (`actions/defaults.rs:895-898`). Also, `handle_session_created` writes "Session <id>, use /dashboard to switch between sessions" into the Ask scrollback (`lifecycle.rs:1449-1456`). That line is what the page shows until the first answer.
Fix: "Full history: `Ctrl+\` opens the dashboard, where the Ask session is a row".

## Table rows confirmed

- **Panel beside the trace.** The panel is split off before the chat draws (`app/agent_view/render.rs:587-606`); pages at `views/side_panel.rs:27-43`. Adding a third page holds.
- **Answer shown in the panel.** `paint_peek_live_tail` is a pure read of `&ScrollbackState` (`views/dashboard/peek_tail.rs:49`). It renders with `output_uncached` (`:170`), needs no viewport lease or `begin_peek_viewport`, and its tests call it on a bare scrollback (`:263`). It works for any agent. Limit: it shows only the current turn (`:56-63`).
- **Quoting.** `try_take_idle_enter_quote` and `insert_quoted_reply` (`app/agent_view/viewer.rs:1093-1147`); a visual selection is quoted through `selected_plain_text` (`views/block_viewer/selection.rs:87-97`). See findings 2 and 9 for where the quote goes.
- **Stepping while quoting.** `step_block_viewer` (`viewer.rs:1067-1091`, `:1162-1174`).
- **Creating the Ask session, the effect itself.** `Effect::CreateSession` sends `session/new` on the pager's one `acp_tx`, whatever the backend: in-process, `--agent-cmd`, or a leader with `stamp_agent_choice` (`app/effects/mod.rs:252-310`). Standalone has a live connection (`app/event_loop.rs:1239-1251`); it only skipped sending `session/new` for its own view (`standalone_traces.rs:52-53`). `handle_session_created` binds the session by agent id and drains the queue without switching view (`lifecycle.rs:1463`, `:1486`). Finding 6 covers the dispatchers around the effect.
- **Resuming.** `Effect::LoadSession` sends `session/load` (`app/effects/mod.rs:588-591`). `gb` already resumes Claude and Pi sessions through `--agent-cmd … --resume` (`scripts/hub/gb:358`), so loading works on each backend (inferred from that launcher). The folder and the view switch are in finding 6.
- **Answers arriving while you browse.** Updates are routed by session id (`app/dispatch/ctx.rs:332-338`), and the event loop's ACP arm does not check which view is active (`app/event_loop.rs:2322-2360`).
- **Reaching it from a live session.** Follow's `v` (`views/traces_modal/deck.rs:107`, `:170-185`) goes through `board::grok_action` (`views/traces_modal/board.rs:470-476`) to `open_claude_in`, then `open_live_in`, which is a trace view (`standalone_traces.rs:87-90`).

Rows that do not hold: Typing the question (finding 2), Sending without leaving the trace (finding 1), Full history (finding 15).
