# Fix batch 1: traces observability (draft, under review)

Source: docs/traces/AUDIT-2026-11-12-observe.md (Haiku audit, unreviewed). Locations below are the audit's; verify before editing.

## Sab's words
"seeing what a subagent received is so fucking important because it's like, if it receives a slightly incorrect brief, then it's like we don't have to fucking audit the entire agents run. We just found the issue"
"if I just click the dashboard button, I can suddenly start spawning agents and work trees, and this is supposed to be a traces and observability tool."
"there's obvious fixes for this shit"

## Tasks
1. **Root follow names each subagent.** Today root follow merges all subagent rows into one stream; the only cue is a 2-space indent (`traces_follow.rs:358-359`, `:504`). Every subagent row must show which agent it came from.
2. **Subagent follow shows the brief it received, in full.** Today the band where the brief belongs is empty; the audit says the brief is indexed as line 0, cause not traced.
3. **Trace views offer no dashboard.** The replay view hides the input (`role.rs:73-76`, `trace_return` set at `standalone_traces.rs:189`) but still renders `[Dashboard]` (`agent_view/render.rs:1420-1433`), and `Ctrl+\` or a click opens it with `+ New Agent`, `Worktree Ctrl+w`, `Dispatch a new agent`. The router guards only CancelTurn and Rewind in trace views (`router.rs:1097-1100`); `OpenDashboard` (`router.rs:1595`) is unguarded. Remove the button in trace views and make OpenDashboard a no-op there (`router.rs`, `agent_view/render.rs`, `mouse.rs`).

## Build policy, standing for the whole cleanup sweep (Sab, 2026-11-12)
"we cannot have all three agents running tests and running builds and waiting on each other... we just have to do it all at once. They're gonna have to review their work from code first."
Builders run no cargo build/test/check. They review their own change from code, add or update tests without running them, and commit. One test+build cycle runs once for the whole batch, after all slices land.
