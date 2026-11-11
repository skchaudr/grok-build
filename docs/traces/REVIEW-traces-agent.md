# REVIEW: SPEC-traces-agent.md (adversarial)

Reviewer: Claude Fable 5.2, 2026-11-11, at Sab's request. Spec author: Claude Opus 5.5. Code read, nothing run.

Verdict: the shape (trace left, Ask in the F3 panel) holds up. Not ready to build: two architecture questions are filed as "unknown until built" and both decide the design.

## Departures from Sab's words (he has not ruled on these)

- "it should all be one session across any number of traces views" became one session per agent choice.
- "do we give the agent access to the entire traces transcript? I say no" became a request in the identity text.
- "any number of traces views" became grok view only; follow and the list are out.

## Findings

1. **Claude and Pi as the Ask agent is unread, and it is the biggest piece.** A pager connection hosts one backend: the fork's in-process agent (`acp/mod.rs:232-236`) or one external command. A second backend needs a leader (what `gb` starts per launch, commit 2d60e6d6) or a second connection. The fork's own agent needs neither. Read this before approving; build fork-agent first, Claude/Pi second.
2. **Standalone traces skips login, consent and folder trust** (`standalone_traces.rs:48-51`: "the traces modal reads local files only, so none applies") and sends no `session/new`. Ask's first send is a real session with model calls, so that shortcut stops being true. The spec lists only folder trust.
3. **Live sessions do not reach Ask.** Enter on a running local row opens follow (`board.rs:478`), which has no block viewer and no quoting. Ask works only after `v`. The session you are most likely watching is the one it skips.
4. **"No transcript" is not enforced.** The source line carries harness and session id; Claude and Pi have full tools; the fork's Ask mode is read-only, which still reads files (`plan_mode.rs:400`). Any of them can open the transcript. Cheap tightening: drop the id from the source line. Real enforcement (no tools) is unread.
5. **Piece 2 can be reuse, and the spec missed it.** `paint_peek_live_tail(&ScrollbackState, area, buf)` already paints another agent's scrollback into a rect (dashboard peek, `views/dashboard/peek_tail.rs:49`), and the dashboard already sends a prompt to an agent without attaching (`MachinePickIntent::DashboardPrompt`). Reusing `draw_chat` (modals, banners, takeover) at 24-46 columns is the fragile route. Limit: peek paints a tail, so state 4 ("history scrolls") is not free; a key that jumps to the agent's own tab covers long answers.
6. **Where the draft lives is unspecified.** A trace view is a new `AgentView` per open, each with its own `SidePanel`; a draft stored there dies when you open the next trace. Keep it in the traces agent's own prompt: it persists, and `insert_quoted_reply` works on it unchanged.
7. **Key clash.** A focused panel uses `←`/`→` for pages and `z` `+` `-` for size (`side_panel.rs`); an editor needs all of them for text. The spec gives no rule.
8. **Tests never see an answer.** Seven snapshots check the spec's own states; the screen check stops before send. Streaming into the panel, the core claim, would ship unseen. State 7 is not a render state.
9. **One session forever fills its context.** No "new chat" or compaction rule.
10. **Live today:** in a trace view, Enter in the block viewer closes it and inserts the quote into the hidden composer (`viewer.rs:1179`). The text piles up unseen.

## Before build

- Read: how a second backend starts with no leader; what creating a session needs in standalone traces.
- Rewrite pieces 2 and 5 around findings 1, 2, 5, 6; add the key rule (7) and a follow path (3).
- Sab rules on the three departures.
