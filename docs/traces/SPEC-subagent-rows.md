# SPEC: subagent rows in follow (who, which model, how it's going, and a way in and out)

Status: draft 2026-11-12, for a Sonnet 5.5 task review. Replaces the tag look from batch 1 (`e6d0d124`, snapshots `b60ee0ec`). Install waits for this spec to land, so batch 1 installs once with it.

## Sab's words
- On the tag snapshot: "It also does not signal this is a subagent"
- "getting immediate details on the subagent would be killer good, there is a lot of screen real estate. Like I should be able to know the model of the agent running"
- "I am debating on two features, both important, tracing the subagent, *or* being immediately sent to the parent agents traces"
- Earlier: "seeing what a subagent received is so fucking important"
- Pipeline: "use a Sonnet 5.5 task reviewer. Then builder, then another reviewer to land it."

## Departures from Sab's words (until he rules)
1. Sab is debating between the two navigation features. The lead's choice is to do both, as two ends of one link. Sab hasn't picked.
2. The keys (`f` in, `p` out) are the lead's pick, not Sab's. The reviewer confirms both are free in follow (`traces_modal/deck.rs:90-117`, `Follow::key`).
3. The layout below is the lead's mockup. Sab saw a version of it in chat. He said "Closer" to the one before it and asked for the model and details on top.
4. The stale `… 0s` timer on finished Agent rows is an older bug, folded in because it sits on this screen.

## Target (width 120)
```
◆ Agent  Audit traces: UI/UX lens          haiku-4-5 · 14 calls · ✓ 2m14s
◆ Agent  Audit traces: observability lens  haiku-4-5 · 9 calls · ● 1m50s
↳ 1 UI/UX lens · haiku-4-5           ◆ Read  src/views/traces_follow.rs  0.1s
↳ 2 observability lens · haiku-4-5   › You are one of two auditors. …
◆ Bash  cargo test                                                       0.4s
```

## Rows
1. **Subagent tag** (`traces_follow.rs`, `lead()`): `↳ {n} {name} · {model}`.
   - `n` and `name` keep batch 1's rules: ordinal from 1, shared word prefix stripped.
   - `model` is the short model id, the same text `Body::Start` prints (`↳ haiku-4-5 started`). If the model is unknown, the tag omits ` · {model}`.
   - If the name already is the model (an unlabelled child), the tag shows `↳ {n} {model}` once.
   - Column width is the widest tag among visible subagents plus 2, capped at `width / 3`, and at least 2 spaces always separate the tag from the row's text. The tag column is the same for every subagent row on screen.
   - Over the cap, drop ` · {model}` first (`↳ {n} {name}`), then truncate the name with `…`. An unlabelled child's name is its model, so it keeps the model.
   - `↳` and `{n}` use the theme's accent the band already uses for `↳ … started`. The rest of the tag is `gray_dim`. No new colour.
2. **Alignment:** every row under a tag starts in the same column. That includes the prompt band (`› …`, currently one column right) and its wrapped lines.
3. **Agent call row** (`◆ Agent  {label}`): when the call is linked to a child agent, the right side shows `{model} · {N} calls · {status} {duration}`.
   - The link is the receipt that makes a receipt-born child (`xai-grok-traces/src/follow.rs:706`). The reviewer confirms how the pager maps a call row to its child agent.
   - `{N}` counts the child's call rows the pager holds.
   - `●` and a running duration while the child is live. `✓` and the final duration once the call has a result or the child ended (`■ ended`).
   - An unlinked Agent call keeps today's right side.
   - Narrow widths drop the parts in this order: `calls`, then `model`.
4. **Timer bug:** a finished Agent call never shows `… Ns`. The reviewer finds where the running timer is chosen and why a call with a result still gets it. If that cause sits outside `traces_follow.rs`/`traces_modal.rs`, the builder reports it and doesn't fix it.

## Navigation
- **`f` into a subagent:** on a tagged row or a linked `◆ Agent` row in root follow, `f` opens follow for that subagent alone. It's the same view `f` opens from the list, with the brief at the top (batch 1, task 2). On a root row, `f` does nothing.
- **`p` to the parent:** in a subagent's follow, `p` opens the parent's follow with the cursor on the `◆ Agent` row that spawned it. If that row isn't loaded (backlog cut), the cursor goes to the bottom and the status line says `spawn row not loaded`.
  - This works however the subagent follow was opened (from root follow, the list, or Tab).
  - In a root session, `p` does nothing.
- **Esc** keeps today's meaning (back to the list).
- Add `f` and `p` to the follow hint line and to `TOOL-traces-modal.md` (Keys, Follow screen).

## States the tests must cover (written by the builder, not run)
Snapshots in `traces_follow.rs`, replacing `subagent_tags_width_{120,60}`:
- two labelled subagents, one live and one done, at 120 and 60
- an unlabelled subagent (model only)
- a subagent with no model yet (receipt-born, before its `model` item)
- a name too long for the cap at 60

Unit tests:
- `f` on a tagged row and on a linked Agent row selects that subagent; `f` on a root row does nothing
- `p` from a subagent lands on its spawn row; `p` without a loaded spawn row lands at the bottom with the status text
- a finished Agent call shows `✓` and no `…`
- the prompt band and its wrapped lines start in the tag column

## Pipeline (Sab)
1. **Sonnet 5.5 task reviewer:** checks every location and claim here against code. Writes `REVIEW-subagent-rows.md` with replacement text where needed, plus "Departures" for Sab. Read-only, no cargo.
2. **Sonnet 5.5 builder:** implements the reviewed text. No cargo (standing build policy). Commits its own files by path with trailers.
3. **Sonnet 5.5 landing reviewer:** reviews the builder's diff against this spec and the review, from code. Fixes go back to the builder via message.
4. **Batch cycle** (builder, Rex `batch1-cycle`): `cargo test --release -p xai-grok-pager traces_` and `extension_shortcuts`; accept the new snapshots; STOP and show Sab the snapshots. On his OK: `scripts/install-fork.sh`.
5. **Screen check:** P4 of `SPEC-batch-1-finish.md` (fresh Haiku, every theme, full and half width, plus `f`/`p`), then Sab sees it.

Rules for every agent: kill only your own PIDs and Rex sessions, never pkill/killall. Never `r` on a Claude row. Never confirm a dashboard launch.
