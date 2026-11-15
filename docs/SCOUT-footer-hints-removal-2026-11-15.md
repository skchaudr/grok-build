# Scout: dropping the default-key hints from the agent footer

Read-only scout for `docs/briefs/scout-footer-hints-removal-2026-11-15.md`. No source edits. Paths are under `crates/codegen/xai-grok-pager/src/` unless shown. "inferred" marks a claim not read line by line.

## Departure from the brief

- Brief says `with_right_text` is called from `app/agent_view/render.rs` around 3888-3900. It is not. `rg` over `src` finds one match, the definition at `views/shortcuts_bar.rs:190`. `render.rs:3884-3903` calls only `with_pending` and `compact`. The right-hand slot is dead code today, so area D starts from a slot nothing fills.

## A. Every footer state on the agent screen

Routing: `render.rs:599-602` (`shortcuts_bar_content`) picks the state through `KeyOwner` (`app/agent_view/key_owner.rs:42-55`). Pane states go through `normal_pane_hints` (`render.rs:692`) into `agent::build_hints` (`views/agent.rs:1150`), then render with `compact(5, help)` at `render.rs:3888-3902`.

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `app/agent_view/render.rs:3876-3880` | Active modal: returns before the bar draws. No footer row. | Nothing. | 0 | none |
| `app/agent_view/render.rs:624` | `KeyOwner::SidePanel` returns `Hidden`; the panel paints its own key footer. | Nothing. | 0 | none |
| `app/agent_view/render.rs:625` + `673-689` | Line viewer: `Hidden`, except plan-approval (`604-614`) and casual commenting, which shows `Enter:save comment  Esc:cancel` (`683-686`). Viewer paints its own footer later (`render.rs:3909`, not opened). | Nothing unless casual-comment hints are wanted gone. | 0 | `render.rs:3909-3920` not opened |
| `app/agent_view/render.rs:626-631` | `KeyOwner::BlockViewer`: `viewer.shortcuts_hints()`, shown as `Surface` (no help hint appended). | Nothing. Not a typing or nav state. | 0 | the `shortcuts_hints` fn is not opened |
| `app/agent_view/render.rs:632-635` | Permission card: `permission_shortcut_hints(perm)`. | Nothing. Listed in B. | 0 | the fn body not opened |
| `app/agent_view/render.rs:637-642`, `604-614` | Plan approval: `plan_approval_shortcut_hints`. Empty means `Hidden`. | Nothing. Listed in B. | 0 | `plan_approval_shortcut_hints` not opened |
| `app/agent_view/render.rs:643-648` | Cancel-turn card, hard-coded: `1-4:select  Tab:next choice  Enter:confirm  <esc>`. | Nothing. Listed in B. | 0 | none |
| `app/agent_view/render.rs:649-653` | Question card: `question_shortcut_hints(qv)`. | Nothing. Listed in B. | 0 | fn not opened |
| `app/agent_view/render.rs:654-660` | MCP elicitation card, hard-coded: `Enter:accept / toggle  d:decline  Ctrl+c:cancel  Tab:next field`. | Nothing. Listed in B. | 0 | none |
| `app/agent_view/render.rs:661-668` | `KeyOwner::Pane`: `normal_pane_hints`, drops the `prompt` label when the composer is hidden (`664-666`), returns `Pane(hints)`. **This is the single point every typing, queue, tasks, dock, todo and scrollback state passes through.** | Filter here with an allowlist (see C). | 6-10 | none |
| `views/agent.rs:1184` | `ActivePane::Catalog`: empty list. | Nothing. | 0 | none |
| `views/agent.rs:1185-1191` | Todo pane: `h:show done / hide done`. | Covered by the C filter. | 0 (via C) | none |
| `views/agent.rs:1193-1211` | Dock pane: `j/k:navigate`, `h:show done`, `Enter:<label>`, `x:kill`, `Tab:<tab_label>`. | Covered by C filter. | 0 (via C) | none |
| `views/agent.rs:1212-1226` | Queue pane, read-only: `y:copy`. Per-row kind: `x:delete row  e:edit  J/K:reorder  y:copy`, plus `InterjectPrompt` when running (`1221-1223`). | `J/K`, `x`, `e`, `y` are default-key hints and fall to C. Interject is B. | 0 (via C) | none |
| `views/agent.rs:1227-1234` | Editing a queued message: `Enter:save` (only if `can_send`), `Esc:cancel`. | Keep (B). | 0 | none |
| `views/agent.rs:1235-1251` | History search (prompt): `Up/Down:nav  PgUp/PgDn:page  Enter:select  Esc:cancel`. | Keep (B). Arguably default keys. Decide. | 0 | none |
| `views/agent.rs:1253-1290` | Typing (prompt focused): `Enter:send` / `:queue` / `:expand` / `:send now` (`1261-1270`), `Shift+Enter:newline` or `Alt+Enter` (`1272-1277`), `:lines` (`1279-1281`), `Tab/Right:accept suggestion` pinned (`1282-1287`), `BackTab:mode` (`1288-1290`). | Default-key hints, fall to C except Tab/Right suggestion (B). | 0 (via C) | none |
| `views/agent.rs:1291-1303` | Registry loop over `PromptFocused`, `AgentScreen`, `Always` with `hint_priority`. Skips `SendPrompt`, `CycleThinkingLevel`, `CommandPalette`, `Quit`, `VoiceToggle`, `EnableVoiceMode`. Of the `hint_priority: Some` defs in `actions/defaults.rs` (lines 78-816 region, checked by `rg`), the only ones in those contexts are `CommandPalette` (`845-846`), `Quit` (`830-831`) and `VoiceToggle` (`728-729`). All three are skipped. **Inferred: this loop adds nothing today.** | Nothing, but it is a trap for future defaults. | 0 | inferred from `rg`, not a full read of `defaults.rs` |
| `views/agent.rs:1306-1321` | Tasks pane: `Enter:view`, `y:copy output`, `x:kill`, `h:show done`. | Covered by C filter. | 0 (via C) | none |
| `views/agent.rs:1323-1346` | Scrollback search active: vim `n/N:next/prev` or `Enter:go` while composing; non-vim `Down/Up:next/prev`; `Esc:cancel`. | Keep (B) or treat as default-key. Decide. | 0 | none |
| `views/agent.rs:1347-1471` | Scrollback navigation. Hints gated on vim mode: `j/k:nav` pinned (`1425-1432`), `h/l:turn` pinned (`1433-1440`), `g/G:top/btm` (`1446-1453`), `y:copy` (`1454-1460`, `1369-1376`), `CopyBlockMeta` (`1461-1466`). Other: `Enter:open` (`1415-1419`), `←:expand/collapse` fold label (`1401-1413`), `trace` (`1420-1424`), thinking (`1441-1445`), `x:kill` (`1467-1469`), focus hint `Space:prompt` (`1356-1368`, `prompt_focus_hint` at `1136-1145`). | Default-key hints, fall to C. | 0 (via C) | none |
| `views/agent.rs:1473-1479` | Turn running: `CancelTurn` hint, key swapped to Esc when `esc_would_cancel_turn`. Applies to every pane. | Keep (B). | 0 | none |
| `views/agent.rs:1480-1486` | Interject, when the prompt is focused and interjection is possible. | Keep (B). | 0 | none |
| `views/agent.rs:1487-1492` | `send to bg` on the root surface when `can_demote`. | Keep (B). | 0 | none |
| `views/shortcuts_bar.rs:229-249` | Pending "press again to X" replaces all hints. Source of `pending_hint` is `render.rs:976`/`1018` parameters, origin not traced. | Keep (B). | 0 | origin of `pending_hint` not traced |
| `app/agent_view/render.rs:3889-3902` | Pane path appends the `ShortcutsHelp` hint (`Ctrl+.` or `Ctrl+x`) to every pane state, and truncates to `compact(5, …)`. Dashboard overlay swaps to the alt key (`3891-3896`). | Remove the help hint from the typing and nav states (C). | 14 | Is `ShortcutsHelp` reachable by `?` alone? `actions/defaults.rs:850` says `?` opens search in scrollback, not help. Unverified. |
| `actions/defaults.rs:853-862` | `ShortcutsHelp` default key: `Ctrl+x` if `ctrl_dot_unreliable`, else `Ctrl+.`. | Nothing, unless the help key itself is removed. | 0 | `ctrl_dot_unreliable` source not opened |

## B. Hints that carry state the user needs in the moment (keep)

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `app/agent_view/render.rs:632-635` | Permission approval keys. | Keep. | 0 | fn body not opened |
| `app/agent_view/render.rs:637-642`, `604-614` | Plan-approval choices. | Keep. | 0 | not opened |
| `app/agent_view/render.rs:643-648` | Cancel-turn card choices `1-4`, Tab, Enter, Esc. | Keep. | 0 | none |
| `app/agent_view/render.rs:649-653` | Question card choices. | Keep. | 0 | fn not opened |
| `app/agent_view/render.rs:654-660` | MCP elicitation accept/decline/cancel. | Keep. | 0 | none |
| `app/agent_view/render.rs:626-631` | Block viewer keys. | Keep. | 0 | fn not opened |
| `app/agent_view/render.rs:682-686` | Casual comment `Enter:save comment  Esc:cancel`. | Keep. | 0 | none |
| `views/agent.rs:1227-1234` | Queued-edit `Enter:save  Esc:cancel`. | Keep. | 0 | none |
| `views/agent.rs:1235-1251` | History search navigation. | Keep, or C if treated as default keys. | 0 | owner decision |
| `views/agent.rs:1323-1346` | Scrollback search keys. | Keep, or C. | 0 | owner decision |
| `views/agent.rs:1473-1479` | `CancelTurn` while a turn runs. | Keep. | 0 | none |
| `views/agent.rs:1480-1486` | `Interject` (queue a message mid-turn). | Keep. | 0 | none |
| `views/agent.rs:1487-1492` | `send to bg`. | Keep. | 0 | none |
| `views/agent.rs:1282-1287` | `Tab/Right: accept suggestion` (pinned, shown only while a suggestion exists). | Keep. | 0 | none |
| `views/agent.rs:1212-1226` | Queue `Interject` when running. | Keep. | 0 | none |
| `views/shortcuts_bar.rs:229-249` | `press again to X` double-press confirm. | Keep. | 0 | none |

## C. Removal: the smallest change

Stop default-key hints in typing and navigation while keeping B. One filter covers every Pane state, because all of them pass through `render.rs:661-668`.

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `app/agent_view/render.rs:661-668` | `KeyOwner::Pane` returns `normal_pane_hints` unfiltered, except the `prompt` label drop. | Replace `retain(|h| h.label != "prompt")` with an allowlist keep-set of B labels (CancelTurn, Interject, send to bg, `queue`, `send now`, Tab accept suggestion, `save`, `cancel`, `select`, `view`/`copy`, etc.). Labels, not `ActionId`s, are what `HintItem` carries (`views/shortcuts_bar.rs:19-34`), so match on label or add a `kind` flag. | 6-10 | Label matching is fragile: `"copy"` appears in both B and C contexts. A `keep: bool` on `HintItem` is cleaner. Adds a field, so more lines. Owner choice. |
| `app/agent_view/render.rs:3889-3902` | Pane path appends `ShortcutsHelp` hint and `compact(5, …)`. | Remove the help hint from Pane, or keep it and note Sab's complaint that it is a default. Sab's 2026-11-14 quote names `Ctrl-,` as overruled, so removal is consistent with his words. | 14 | Is help reachable another way? Command palette (`actions/defaults.rs:839-840`) is Sab's stated alternative. |
| `views/agent.rs:1288-1290` | `BackTab:mode` pushed on root surface. | Covered by the C filter. Only remove the push if the filter is not used. | 0 (via C) | none |

Smallest viable: C row 1 with the allowlist, plus C row 2. About 20 lines across one file. Sab can drop the help hint alone for about 14 lines.

## D. Row as status

Current paint logic in `views/shortcuts_bar.rs`:

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `views/shortcuts_bar.rs:257` | Hints paint from `area.x`, so the row is already left-aligned. | Nothing for left alignment. | 0 | none |
| `views/shortcuts_bar.rs:259-292` | Hint loop breaks at the first hint that does not fit (`265`, `273`, `279`, `287`). At half width, status placed after hints is dropped. | Put status first, before the hint loop, so it survives narrow width. | 10-15 | none |
| `views/shortcuts_bar.rs:190-193` | `with_right_text` setter. Never called (see departure). | Either delete it or repurpose it as the status setter. Repurposing removes dead code. | 4-8 | none |
| `views/shortcuts_bar.rs:295-306` | Right-aligned text only paints when `rx > x + 1` (`301`), so it vanishes once hints fill the row. | Remove, or keep for a different item. | 0-12 | none |
| `views/shortcuts_bar.rs:229-249` | Pending "press again" replaces the whole row. | Keep; it is B. | 0 | none |
| `app/agent_view/render.rs:3884-3903` | Passes only `with_pending` and `compact`. | Add `.with_status(...)`. The status value comes from a source not in this scout (cache warmth or 5h usage, per `docs/SCOUT-usage-footer-areas-2026-11-15.md`, not opened). | 3-5 | source of status data not opened |

Half-width behaviour: status must not be clipped by the hint loop. Painting it first, with its own width check, keeps it at 40 columns. Inferred: no test yet covers this.

## E. Tests and snapshots that render these footers

| file:line | what is there now | what would change | rough lines changed | unknowns |
|---|---|---|---|---|
| `app/agent_view/snapshots/xai_grok_pager__app__agent_view__trace_view_tests__trace_view_half_width.snap:42` | Footer `y:copy  │  Enter:open  │  j/k:nav  │`. | Changes with C. Re-record. | 1 snapshot | `trace_view_full_width.snap` line not opened; expected to match same pattern (inferred) |
| `app/agent_view/snapshots/xai_grok_pager__app__agent_view__trace_view_tests__trace_view_full_width.snap` | Same footer family (matched by `rg`). | Changes with C. Re-record. | 1 snapshot | line not opened |
| `app/agent_view/snapshots/xai_grok_pager__app__agent_view__side_panel__tests__closed.snap:26` | Footer `Shift+Tab:mode  │  Ctrl+.:shortcuts`. | Changes with C (`BackTab` and help hint). Re-record. | 1 snapshot | none |
| `app/agent_view/snapshots/xai_grok_pager__app__agent_view__side_panel__tests__open_unfocused_lets_the_chat_type.snap` | Matched by `rg` (`Enter.*open`/`j/k.*nav`/`shortcuts`). Footer line not opened. | Probably changes. | 1 snapshot | footer line not opened |
| `app/agent_view/snapshots/xai_grok_pager__app__agent_view__side_panel__tests__narrow_terminal_unfocused_panel_hides.snap` | Matched by `rg`. Footer line not opened. | Probably changes. | 1 snapshot | footer line not opened |
| `views/agent.rs:1529-2419` | 13 `build_hints` call sites in the test module, all with `ActivePane` set, asserting labels (`first_two_labels` at `1559`). | Unaffected if change is in `render.rs`. Affected if change is in `build_hints` itself. | 0 if C in render.rs | individual assertions not read |
| `views/shortcuts_bar.rs:368-420+` | Tests of `compute_effective_hints` (compact truncation, pinned, help hint). | Unaffected by C. Add a test for D (status at half width). | +20 new | none |
| `app/agent_view/key_owner_tests.rs`, `subagent_takeover_tests.rs`, `render.rs` tests | Matched by `rg` on footer strings (7, 5, and 126 hits respectively). Not read. | Check each; likely only the ones asserting `Ctrl+.`, `j/k`, `Enter`. | unknown | not opened |
| `views/dashboard/render_tests.rs`, `layout_tests.rs` | Matched by `rg` for shortcut bar code. Dashboard footer, not agent screen. | Out of scope unless the dashboard shares the bar path. | 0 | not opened |

## Slices

1. **Hint filter (C row 1)**: `app/agent_view/render.rs:661-668`. Allowlist of B hints. One file, about 10 lines, plus its tests. Depends on Sab deciding whether `copy`/`cancel` count as B.
2. **Help hint removal (C row 2)**: `app/agent_view/render.rs:3889-3902`. Independent of slice 1. About 14 lines. Touches the same file as slice 1, so merge them.
3. **Status row (D)**: `views/shortcuts_bar.rs` (paint order `257-292`, setter `190-193`, right text `295-306`) plus `app/agent_view/render.rs:3884-3903` to pass status. Independent of slices 1-2 at file level, but the same render call site. Needs the status data source, which is not in this scout.
4. **Snapshots**: the five files in E. Re-record after slices 1-3. Depends on 1-3.
5. **Tests**: `views/shortcuts_bar.rs` tests for status at half width. Depends on 3.

Open decisions for Sab: whether history search (`agent.rs:1235-1251`) and scrollback search (`1323-1346`) count as B; whether help (`Ctrl+.`) should stay reachable from the footer at all; which data source feeds the status slot.
