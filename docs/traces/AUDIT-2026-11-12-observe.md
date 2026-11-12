# grok-traces observability audit, 2026-11-12

Scope: observability only. Driven in Rex (`grok-fork traces`, wide 80 cols and narrow 40 cols). Evidence is from captures and `grok-traces events`/`list`. Code refs are to `crates/codegen/xai-grok-pager/src/views/`.

Note: `~/.grok/bin/grok-traces` is the CLI (list, live, events, follow). The TUI is `grok-fork traces`.

## Ranked findings

### 1. Root follow merges every subagent into one unlabelled stream
- Problem: Enter on the root "Herdr Grok traces TUI audit" row (header `traces Herdr Grok t… ● live`) showed `Check cargo config, workspace members, and nested checkout` and `Inspect grok-traces crate manifest` under the root. Index check: root has 0 cargo calls; the buildability subagent has 13. Rows differ only by 2-space depth indent; no agent name on any row.
- Cause: `traces_follow.rs:358-359` `visible()` returns all rows when `filter == 0`. Render indents by depth only (`traces_follow.rs:504`). `Body::Start` (`:644`) is the only agent marker and did not appear in the captures.
- Close: prefix each child row with a short agent name, or make default filter root-only and show "all" as opt-in. Touches `traces_follow.rs` only. Small.

### 2. Subagent follow shows no prompt
- Problem: Tab to "Audit traces: observability lens" shows header then an empty band, then `◆ Read rex/SKILL.md`. The index has the brief as line 0 (`grok-traces events`: `"type":"user"`, preview "You are one of three auditors…").
- Cause: guessing. `Body::User` is emitted from `"user"` items (`traces_follow.rs:255`) and rendered at `:528`. The blank band suggests the item is dropped or its text is empty; not traced further.
- Close: find where `grok-traces follow` emits subagent user items, then add a test. Small to medium.

### 3. Half-width pane loses the metadata and the controls
- Problem (40 cols, `rex block call resize`): tabs cut to `Live Longest Failed Proje`; `Harness` gone. Row meta dropped: `Claude · grok-build · 9m` becomes `now`. Titles cut to `Herdr Grok traces TUI aud…`. Follow footer cut at `l:`. Hook errors cut to `✗ PreToolUse:Bas…`.
- Cause: fixed-width layout in `traces_modal.rs`; exact width rules not located (guessing).
- Close: a narrow layout mode in `traces_modal.rs` and `traces_follow.rs` (footer wraps or drops keys). Medium, two files.

### 4. "Failed" counts hook errors and the observer's own failing calls
- Problem: the Failed tab (24 rows) lists `Audit traces: observability lens` (live, 1m) because of my own `rex` exit-1 calls. Its `failed` flag is false; `error_count` is 2. Hook errors (`✗ PreToolUse:Bash hook error`) appear in the same column.
- Cause: `traces_modal.rs:933` filters on `r.error_count > 0`. Error counting is in the index, not located.
- Close: count only failed tool results, not hook errors. Touches the index writer and `traces_modal.rs`. Small to medium.

### 5. Titles are not identities
- Problem: `https://www.npmjs.com/package/@visuli…` (URL as title), `==========…`, `(no prompt) · grok-build`, two rows both titled `Grok Claude parity gaps` with the same last text. Nothing tells them apart.
- Cause: title = first prompt line, no fallback. Index derivation not located (guessing: the `grok-traces` crate).
- Close: skip URL and separator lines, add a session-id suffix on collision. Small.

### 6. Ledger key labels and mismatches
- Footer `c:compact` runs `live_toggle` (`traces_modal.rs:4004`, `:2829`), a collapse toggle, not compaction. The `Longest` tab header reads `Most active time`. The `9m` duration on live rows does not equal session span (root started ~35 min earlier; `Longest` shows `16h24m` for other rows). Meaning: guessing.
- Close: relabel the footer and the header; decide what the duration means. Small.

### 7. The "live" view is honest (positive)
- The five active rows match `⸬ 5 running`. Their last-event ages were 0–3 min (checked with `grok-traces list --json`). Remote rows aged 15m→18m during the capture, so the 10 s remote refresh is working.

### 8. Traces view leads to the agent dashboard, which can spawn agents and worktrees
- Problem: the modal itself has no spawn control, but Enter on a finished session opens a replay in an agent view. Its status bar shows `[Dashboard]` (`~/.config/prime-agent  [Dashboard]`). `Ctrl+\` opens the dashboard. It offers `+ New Agent`, `Open Previous /resume`, `Worktree Ctrl+w`, and a `❯ Dispatch a new agent` box with `Enter:create`. I did not confirm any of them.
- Cause: see section "Dashboard and agent input" below.
- Close: remove the dashboard entry and `Ctrl+\` from trace views (guard in the router), or hide the status-bar button when `trace_return` is set. Touches `router.rs`, `agent_view/render.rs`, `app_view.rs`, `mouse.rs`. Small to medium.

### 9. CLI follow hangs
- Observed: `grok-traces follow <subagent-id> --json --once` ran past 120 s with no output. Killed. Cause not traced.

## Not verified
- Assistant text in follow (the root's last text message is indexed at line 172 but no `◆ Text` row appeared in the captured windows).
- Colour (left to the UI slice).
- Sab's remote hosts beyond the row ages.

## Dashboard and agent input (Sab's 2026-11-12 complaint)

Observed in Rex, session `audit-observe2`, `grok-fork traces`:
- Ledger (Live tab) has no dashboard button and no prompt. Esc at the top level exits the process.
- Enter on a finished Claude session (`Prime-playbook.md rewrite`, sab-air) opens a replay. Header row reads `~/.config/prime-agent    [Dashboard]`. The footer is `y:copy │ Enter:open │ j/k:nav │ Shift+l/h:turn │ Ctrl+e:`. There is no input box. Activate the dashboard with the mouse click or `Ctrl+\`.
- `Ctrl+\` on the replay opens the dashboard: `+ New Agent`, `Open Previous /resume │ Worktree Ctrl+w`, `❯ Dispatch a new agent`, `Enter:create │ Tab:input │ ?:shortcuts`. Esc closed it and the replay returned. Not confirmed: New Agent, Worktree, Enter in the dispatch box.

Code, dashboard wiring:
- `app/agent_view/render.rs:1420-1433`: `[Dashboard]` status-bar button, drawn when `dashboard_available`. Not gated on trace view.
- `app/agent_view/render.rs:1452-1453`: `hit_dashboard` set for clicks.
- `app/mouse.rs:229-234`: click on `hit_dashboard` returns `Action::OpenDashboard`.
- `app/app_view.rs:2562-2576`: while the overlay is up, `OpenDashboard` closes it.
- `app/actions.rs:834-835`: `OpenDashboard` is the action behind `/dashboard`, `Ctrl+\` and `grok dashboard` (doc comment).
- `app/dispatch/router.rs:1595`: `Action::OpenDashboard` → `dispatch_open_dashboard`. `router.rs:1097-1100` guards only `CancelTurn`, `Rewind`, `RewindShowPicker` in trace views. `OpenDashboard` is not guarded.
- `app/dispatch/dashboard.rs:160-180`: gates on `dashboard_enabled()`, auth, consent, folder trust. Nothing checks trace view.
- `app/dispatch/dashboard.rs:572-580`: worktree toggle (Ctrl+W). `:634-643`: worktree-label dialog.
- `app/dispatch/router.rs:1600`: `Action::DashboardDispatch { text, attach }` creates agents from the dispatch box.
- Not the cause: `views/dock/mod.rs` has no dashboard wiring (only a test string at 546/639).

Code, agent input turned off:
- `app/dispatch/standalone_traces.rs:189` and `:244`: `agent.trace_return = Some(state)` on replay open.
- `app/agent_view/role.rs:73-76`: `composer_route()` returns `ComposerRoute::Hidden` when `trace_return.is_some()`. This is the switch.
- `app/agent_view/render.rs:321-322`: the `prompt` hint is removed from the shortcuts bar when hidden.
- `app/agent_view/render.rs:1020-1024`: prompt height is 0 when hidden.
- `app/agent_view/input.rs:1352-1353`: focusing the prompt is refused when hidden.
- `app/dispatch/standalone_traces.rs:372`: `trace_return.take()` on leave.
- `app/dispatch/standalone_traces.rs:395`: `read_only_view` builds the replay view with `session_id: None`, no sends.
- The follow screen (`traces_follow.rs`) has no composer at all.

Why it is inconsistent: the composer is hidden by a per-view flag, while the dashboard is a global overlay with its own input box. The flag is checked for the composer but not for the dashboard path. So the same screen hides the input and still offers the launch actions.

Close: in trace views, drop `[Dashboard]` (render.rs:1420) and make `OpenDashboard` a no-op (router.rs guard next to 1097). Keep Esc and `v` as the way out. Touches 3 files: `router.rs`, `agent_view/render.rs`, `app/mouse.rs`. Small.
