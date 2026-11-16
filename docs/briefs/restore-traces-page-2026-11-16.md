# Restore the Traces page in the F3 side panel (2026-11-16)

You are a Sonnet 5.5 builder. You alone run cargo and the install. Read `docs/HARNESS-FACTS.md` and `AGENTS.md` first.

## Intent (Sab's words, 2026-11-16)

> The current traces side panel was actually pretty dang good
> Then you introduced a version which ... massively regressed it
> All you had to do was make some slight changes to this current one, accomodate the fact its in a side panel now that may also zoom out
> you watered down and stripped away what makes grok build so good, what makes tool calls and sessions streaming through it so good
> Tool calls are grouped together and expandable and collapsable
> Your zoom and copy is good
> "Steps" ... whatever the fuck that is ... Traces got totally removed in favor of "Steps"
> Tab needs to ensure it can move tabs, because cant really collapse and expand with h and l in that specific tab

## What to do

1. The page named Steps (`views/side_panel/steps.rs`, `Page::Steps`) goes away. A page named **Traces** takes its place and draws the real follow view: `Follow` from `views/traces_follow.rs` (open/bare/poll/apply/rows/visible/step/fold/render), with its grouped calls, `e` expand, `Enter` open, replies, `+N lines` folding and live/paused state. Do not write a new list renderer. The previous page wiring is `git show ee2fab67^:crates/codegen/xai-grok-pager/src/views/side_panel.rs`; fit it into the module directory `views/side_panel/` (it must still work in the side panel and zoomed).
2. Keep pages Session, Agents, Recent, and what `489730c2` added: workspace-grouped Recent, `y` copies `Row.copy`, zoom.
3. Keys. On the Traces page `h`/`l`/Left/Right belong to the follow view (collapse/expand, as in the old page), so **Tab and BackTab move between pages there**. On the other pages `h`/`l`/Left/Right and Tab/BackTab all switch pages. Update the footer hint text for the Traces page to say so. Reuse grok-build's own bindings; invent none.
4. Delete everything only Steps used. Leave no dead code and no second list.
5. Tests: targeted `cargo test --release -p xai-grok-pager side_panel` (insta snapshots; review and accept only snapshots that changed because of this work). Add tests for: Tab moves page on Traces while `l` does not; the Traces page draws a grouped call and a `+N lines` fold.
6. After tests pass, one `scripts/install-fork.sh` run (one cargo run, both Macs, checksums). Then look at the real panel: open `grok-fork` in a floated Rex block (see memory recipe: `session.new_layer`, minimized float, explicit `window_id`), open a Claude session, F3, Traces page, `rex capture --trim`, and read it. Send no prompt. Quote what the screen shows in your report.

## Rules

- No foreground wait loops past ~20 s: background the long commands.
- Never tmux, no kills by name, no hidden subagents, nothing on sab-air except what install-fork.sh does.
- Commit and push with the trailer `Co-authored-by: claude-code sonnet-5.5`.
- Report to `docs/BUILD-restore-traces-page-2026-11-16.md`: what changed (files), test results, the install checksums, and the screen quote. If the live panel shows `(waiting for the transcript)`, say when and for how long.
