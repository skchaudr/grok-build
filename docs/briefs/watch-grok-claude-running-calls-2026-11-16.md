# Watch a live `grok claude` session for one hour (2026-11-16)

You are a Haiku 5.5 watcher. Read-only. You watch one Rex pane that runs a `grok claude` session and record, from the real captured screen, what it shows while a tool call runs.

## Why (Sab's words, 2026-11-16)

> it is ridiculous how claude routinely skips looking at shit, for a family of models where each one has vision

> you were not responding, but there clearly was activity. And that's the problem is that you managed to do
> background activity that is invisible to me. So I have no clue what's going on

Context you may read first: `docs/INVESTIGATE-running-tool-visibility-2026-11-16.md`. It says what the code should draw; nobody has looked at a live screen yet. You are the looking.

## Your assignment

Your prompt names you watcher A or B:

- A: block `17e421`, label `watch-a`
- B: block `b8db76`, label `watch-b`

Both are `grok claude` TUIs with the F3 side panel (pages Session, Steps, Agents, Recent; the Steps page is the traces call list).

Clock: start time is whatever `date +%H:%M` says when you begin. Report-by is **15:30 PST** (the first file below is due by 14:50). Run `date +%H:%M` on every check and write the elapsed time next to each note.

## Mechanics

1. Start the snapshotter in the background. Replace NAME and BLOCK:

```
D=/private/tmp/claude-501/watch/NAME; mkdir -p $D
nohup sh -c 'while true; do rex capture -b BLOCK --trim > '$D'/cur.txt 2>/dev/null; cp '$D'/cur.txt '$D'/snap-$(date +%H%M%S).txt; sleep 15; done' >/dev/null 2>&1 &
echo $! > $D/pid
```

   When you finish, stop it with `kill $(cat $D/pid)` (that PID only; never `pkill` or `killall` by name).

2. Every few minutes, read the newest snapshots with the Read tool, whole file, and look at them. Wait between checks with `sleep 170`; if the harness refuses a foreground sleep, use the Monitor tool with an until-loop on the newest file in `$D`. Do not poll in a tight loop.

3. Whenever a snapshot shows a tool call in progress (a spinner or a running card), keep reading the following snapshots until it ends, so you see the whole call. For each call that lasts more than about 20 seconds, record:
   - clock time of first and last snapshot showing it, and its length;
   - the chat area: is there a card for the call, what is its header text (tool name and description), does it show any time;
   - the status row above the input box: spinner, label, timer;
   - the F3 panel, if it is open: which page is showing, and whether the call appears on Steps or Agents (row text, mark, timer);
   - what the screen looked like the moment after it ended.
   Quote the screen's own words for each, character for character.

4. Also record any moment where the model is working but the screen shows nothing to say so, and any moment where something is running that no part of the screen shows.

## Rules

- Never send input to the pane: no `rex send`, no `rex send-key`, no typing, no Enter. Only `rex capture`.
- Quote interface text only: status row, panel rows, card headers (tool name, description, time). Do not copy the conversation (Sab's messages, the model's replies, command output) into any committed file. The raw snapshots stay in `/private/tmp`.
- No code edits, no cargo, no installs, no tmux, no processes started other than the snapshotter, nothing on sab-air, no subagents.
- If a pane shows nothing useful for 20 minutes, say that plainly in your file. Do not pad.

## Deliver

Write `docs/WATCH-running-calls-A-2026-11-16.md` or `...-B-...` (your letter). Structure: one section per observed call, as in step 3, then a closing section "Seen / not seen" (what the screen drew, what it did not). First version by 14:50, update at least every 15 minutes, final by 15:30. Each time, commit and push only your file with the trailer `Co-authored-by: claude-code haiku-5.5`. If the push is rejected, `git pull --rebase` and push again.
