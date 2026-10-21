# Scope: bb-run Claude on the live screen (post-v1)

Traces lead, 2026-10-21. Builds on SPEC-live 5.2's post-v1 note (adc22819). The facts below were checked read-only on the Air.

## Facts that change the picture

- **The bb you run is not your fork.** `/opt/homebrew/bin/bb` execs `/Applications/bb.app/…/app.asar.unpacked/node_modules/bb-app/dist/bb.js`, the installed desktop-app release. `~/repos/bb` (skchaudr/bb, db23ea4) is source only, so a patch there does nothing until bb runs from the fork.
- **bb threads also run on the mini**, through its host daemon. Any bb-side fix would have to be in the bb running on both Macs.
- **V1 rule (SPEC-live):** "sessions with no receipts are never on the live screen". bb-run Claude sessions have no receipts, so they're invisible live today. They still appear in `/traces` history.

## Options

| | A: patch bb (Fable's) | B: our side only |
|---|---|---|
| Change | About 20 lines in `plugins/provider-claude-code/src/bridge/bridge.ts`: one more SDK hook callback that spawns `grok-traces hook claude`, plus `SessionStart`, `UserPromptSubmit`, `Stop` and `SessionEnd` registrations. | `links.rs`: link the Claude session to its bb thread via `bb.db` `events.provider_thread_id`. Live rule: a Claude session linked to an active bb thread shows `via bb`, taking state from bb thread receipts (`thread.active` → Working, `thread.idle` → Prompt, `archived/deleted` → End) and `now` from the transcript. |
| What you see | Exactly what a terminal Claude shows: tools, `⚠`, subagents. | Working or your turn, plus the last tool or text from the transcript. No `⚠`, no `+N`. Subagents come from transcript files. |
| Cost | Running bb from the fork on both Macs. That means building `bb.app` from the fork, or running it from source with `pnpm install`, which is a build on the Air. Redo it on every bb update; the app's auto-update would replace the patch. | Our crate only, a Rust change. Needs one rebuild of ours and nothing touching bb. Survives bb updates. |
| Unknowns | none | whether the `hooks/bb` plugin fires on the Air today (it has 6 receipts there, so probably yes); liveness = the bb daemon PID per host, same shape as Cursor |
| Limit | | Threads that bb runs on the other host stay unlinked: the transcript and `bb.db` are on different machines. |

## Recommendation

Do **B** as live step 6, after v1 ships and you've used it. Do **A** only if B's coarse states aren't enough once you've used them, and only if you're willing to run bb from your fork. If you go with A, put the patch in its own file, called from one line in `addPermissionEscalationTrackingHooks`, so upstream merges stay trivial.
