# SPEC: usable traces (Track A) and a native Pi host (Track B) in grok-build

Draft 2 (Sonnet 5.5, 2026-10-26). Sab's direction: "of course I want a working Pi in the GROK TUI with all the features it comes with"; "Pi should be as native as possible home for it"; "I do not care how it's done I just want it to work just as much as I want Claude in it to work." Two independent tracks run in parallel: **Track A** makes `/traces` usable (slices T0-T10, Phases 1-3); **Track B** brings the native Pi host into grok-build (slices B0-B9). They share no files. Codex is a note (B9), not a port.

Per-harness facts (log paths, intent fields, tool shapes, hook deny formats) live in `docs/HARNESS-FACTS.md`; check and update it instead of re-researching. Sourcing: facts below come from code, repo history, or CLI output run on sab-mini. A claim that comes from an agent-written doc is phrased "an agent wrote X (file)" and is context, not a constraint.

Read `MAP.md` first, then the slice you were handed. Paths under `crates/codegen/`. `P` = `xai-grok-pager/src`, `T` = `xai-grok-traces/src`, `MODAL` = `P/views/traces_modal.rs` (7058 lines). (U) = unverified; Step 0 of that slice closes it before code.

Constraints on every slice:
- Verification runs on sab-mini. No load tests, builds-in-a-loop or soak runs on sab-air.
- Every commit ends with `Co-authored-by: <tool> <model>` naming the executing tool (e.g. `Co-authored-by: cursor-agent grok 4.6`, `Co-authored-by: pi <model>`). No trailer = invalid.
- Pager tests: an agent wrote in `MAP.md` ("Pager rules learned") that they need `RUST_MIN_STACK=33554432` and an empty `GROK_HOME`; T0 and B0 re-measure the baseline before relying on it.
- Real session text is never committed. Fixtures are synthetic.
- The traces index has no `ALTER TABLE` (`rg 'ALTER TABLE' xai-grok-traces/src` is empty); `SCHEMA_VERSION = 7`, `PARSER_VERSION = 6` (`T/index.rs:24-26`). A bump rebuilds the db on open (MAP.md describes that path). T1 and T6 share one bump (schema 8, parser 7). Land both before either merges, or let T1 own the bump and T6 rebase onto it.

# Track A: usable traces

## 1. Problems, tied to the screenshots

| # | Complaint | Root cause (verified) |
|---|---|---|
| 1 | 25 rows read `bash  python3 -c "` | `T/classify.rs:234-265` `call_detail` takes `command|cmd|script` first (243-244) and `first_line` (208-216) returns line 1 of the script. Claude `description` is read only when there is no command (253). Pi `task_name` is never read. MAP.md:44 (agent-written) describes the first line as the intended label; the code does that, and Sab's complaint 1 supersedes it for inline interpreters. |
| 2 | Expanded row is raw JSON with `\n` escapes; viewport jumps | `MODAL` `body_text` 2866-2875: `command` is not in `TEXT_KEYS` (2860); `INPUT_KEYS` (2862) values are `to_string_pretty`'d (2887-2889), so newlines stay escaped. A test pins this (~5093-5100). Jump: toggling changes no scroll; `ensure_bodies` (1338) loads bodies after the fact, heights change, and `fit_rows` (2776-2783) + `row_span` (2786) count the selected row's full height, so a tall block pins the header to the top. Remote bodies show `loading…` (1446) then grow. |
| 3 | Bare `3005`, `0`; double-spaced results | `collect_text` (2877) pretty-prints any non-string value under an `INPUT_KEYS` key, so Pi's `usage.input` / `cost.input` (normalize.rs test shape, `usage: {input, output, cacheRead...}`) leak as bare numbers. Spacing: blank pushed before the result (1440), separator wrapped in `\n\n` by `compose_body` (2920-2929), chunks joined by `\n\n` (2874), `wrap_body` keeps blank lines (2730-2760), plus `row_gaps` (2808-2846). |
| 4 | Enter shows raw event JSON, command truncated | `open_inspector` 1487-1496 stores `pretty(raw line)`; `render_inspector` 4306-4321 truncates each line with no wrap. An agent wrote in `PACKET-traces-opus-lead.md:119` that it is raw by design; Sab's complaint 4 supersedes that. |
| 5 | No semantic view, no grouping, hooks cannot require intent | The chat TUI reads `description` from `raw_input` (`P/acp/tracker.rs:2100, 2515`, `extract_raw_field` 2864-2870) and groups via `scrollback/state/verb_group.rs`. Traces has neither; the index drops the intent (`detail` is the command line). |
| 6 | No inline diffs | The index keeps no edit payload and no +N/-M. The chat TUI's renderer exists (`scrollback/blocks/tool/edit.rs`) but takes ACP tool calls, not trace lines. |
| 7 | Dead or misleading UI | `r resume`: `resume_target` (`MODAL` 351-361) returns a target only for claude/codex/grok; Pi/OMP/Cursor/DSH get "no resume for Pi". `H host` is shown but does nothing when `~/.grok/traces/hosts` is empty (3156). "indexed N changed files" (`P/app/effects/mod.rs:2788`, scan on every `OpenTraces`, `P/app/dispatch/router.rs:1441-1453`) never clears. Empty `› Assistant` rows: assistant records with no text and no tool call fall through to the role title (`T/normalize.rs` title match ~494-498, text := title ~547-548). Working but unlisted keys: Space (alias of `e`, 3175/3248), `g`/`G`, trajectory `y` (copy prompt, 3298), Live `c`. Inspector `y` copies the raw line. All other hint keys are real. |
| 8 | Newest hosts | grok-build reaches Pi only through external `pi-acp` (agent choice `name = pi, cmd = pi-acp`, as in the `P/app/cli.rs:1425` test; `scripts/acp-matrix.sh` lists `pi-acp`). The native adapter, the Pi extension bundle and the workflow, eval and F2 pieces exist only in grok-pi (Track B). Codex runs through external `codex-acp` and imports via `xai-grok-foreign-sessions`. |

## 2. Architecture rules (Track A)

1. **New rendering goes in new modules** under `P/views/`, not `MODAL`: `traces_body.rs` (blocks, command/diff/result rendering), `traces_payload.rs` (Enter view), `traces_rows.rs` (display-row model, labels, grouping), `traces_hints.rs` (hint list). `MODAL` keeps state, keys and calls into them. Each slice's edits to `MODAL` are limited to: `mod` lines, delegations, and new state fields.
2. **Pure functions in `xai-grok-traces`** for anything derived from harness JSON (labels, edit payloads). The pager crate consumes typed values; it does not re-parse harness shapes.
3. **Display rows are a view model.** `traces_rows.rs` builds `Vec<DisplayRow>` from `EventRow`s: `Event(ix)`, `Group{verb_counts, range, failed}`. Selection, fold and scroll operate on display rows. Raw view = identity mapping.
4. **Reuse the chat TUI's renderers where they take plain data; extract where they take blocks.** Step 0 of T5/T7 reads the cited functions and records "reused" or "extracted (copied N lines, parity test X)". Never fork silently.
5. **Heights are computed before paint.** A body whose text is unknown reserves exactly 1 row (`loading…`); its arrival must not move the selected header on screen (T3).

## 3. Phase 1: inline scripts and tool rows

### T0: extraction (serial, first; no behaviour change)

- **Goal:** make the parallel work possible. Move code out of `MODAL` verbatim.
- **Files:** `MODAL` (shrinks), new `P/views/traces_body.rs` (`body_text`, `collect_text`, `push_chunk`, `body_of`, `compose_body`, `wrap_body`, `split_result`, `continuation`, `continuation_source`, `RESULT_SEP/ERR_SEP`, `NO_MORE`, `pretty`), new `P/views/traces_hints.rs` (`shortcuts()` 3425-3470 incl. `plain`), `P/views/mod.rs`.
- **Behaviour:** none. Same public surface re-exported with `pub(super) use`.
- **Accept:** `git diff --stat` shows only moves; every existing `traces_modal` test passes unchanged; pty `traces_modal_ledger_trajectory_inspector` passes.
- **Tests:** none new.
- **Verify:** `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`.

### T1: labels and intent in the index (`xai-grok-traces`)

- **Goal:** a row says what the call is for, and two different inline scripts never look alike.
- **Files:** `T/classify.rs`, `T/normalize.rs`, `T/index/*` (schema), `T/query.rs` (`EventRow`), `tests/legibility.rs`, `hooks/` untouched.
- **Data:** schema 8 adds `trace_events.intent TEXT` (NULL when none). `EventRow.intent: Option<String>`. `detail` stays, but is now the **headline** (below), not line 1.
- **Intent precedence** (first non-empty, clipped by `clip_line`): `description` > `task_name` / `taskName` > `justification` > `why`/`reason` > a leading comment in the command (`# why: …` or first line starting `#` before code, shebang excluded) > none. Keys are read from the call arguments object (`input`, `arguments`, `args`, `rawInput`). Per harness, field seen in sampled logs:

| Harness | Field | Coverage in samples |
|---|---|---|
| Claude | `input.description` | ~78% of Bash; Agent/Monitor 100%; Edit/Write/Read none |
| Cursor | `input.description` | ~99.6% of Shell |
| Grok | `rawInput.description` | 100% of `run_terminal_command` |
| Codex | `justification` on exec_command | ~1-2.5% |
| Pi | `task_name` when the pi-grok-bash extension is active; stock Pi has `{command}` only | per install |

- **Headline** (`command_headline(cmd) -> String`): skip blank lines, comment lines, `set -…`, `export …`, and `cd … &&` prefixes; take the first remaining line. If that line is an inline-interpreter opener (matches `^(python3?|node|ruby|perl|bash|sh|zsh)\s+(-c|-e)\b` and ends in an open quote, or contains `<<`), append the first following non-blank line that is not an `import`/`from`/`#`/`set` line, joined with ` ⏎ `. Clip to `clip_line` width. Examples: `python3 -c " ⏎ data = json.load(open('x'))"`, `cat <<EOF ⏎ server {`.
- **Not changed:** `description` stays the fallback inside `call_detail` for non-command tools (existing behaviour).
- **UI states touched:** none directly (data only). Rows with `intent = None` are the "unlabeled call" state in T5.
- **Accept:** a Pi fixture with 3 `bash` calls `python3 -c "` + different bodies yields 3 distinct `detail`s; `intent` is `task_name` when present; Claude fixture with `description` yields it; a command whose line 1 is `# why: x` yields intent `x`; a plain `git status` has `intent None` and headline `git status`.
- **Tests (`tests/legibility.rs`, new `intent_*`):** precedence table (one case per row above plus conflicts: description and task_name both present -> description); `command_headline` cases (opener, heredoc, `cd /x && python3 -c "`, comment-first, empty, 10k-char line); remove-regression: existing `call_detail` tests still pass; schema 8 open of a schema 7 db rebuilds (existing rebuild test pattern).
- **Verify:** `cargo test -p xai-grok-traces`; then `cargo run --release -p xai-grok-traces --example scan -- /tmp/t.sqlite` on mini real data and report: share of tool events with intent per source (target table above), count of distinct `detail`s among `bash` events starting `python3 -c`.

### T2: expanded body rendering (`traces_body.rs`)

- **Goal:** an expanded tool row shows the command as code and the result as output, nothing else.
- **Files:** `P/views/traces_body.rs`, `MODAL` (`body_lines` ~1427-1450 delegates).
- **Model:** `parse_body(raw: &str) -> Vec<Block>`; `Block = Prose(String) | Command{lang, text} | Args(Vec<(String,String)>) | Result{text, error: bool} | Thinking(String)`. Parse walks content blocks (`tool_use`, `toolCall`, `function_call`, `tool_calls[]`, Codex `payload.arguments`/`cmd`) and only descends into argument objects under a block that has a tool name. It never reads message-level `usage`, `cost`, `info`, `metadata`, `model`, `api`, `provider` (fixes complaint 3 at the source).
- **Command block:** key `command`/`cmd`/`script` (or `command` array joined by space) rendered with **real newlines**. `lang`: `python` if the opener is `python3? -c|-`, `node`/`js`, `ruby`, `perl`, else `bash`. Highlight with `xai-grok-pager-render` `syntax::get_syntect` + `highlight_line` (per line, keep highlighter state across lines); bash fallback `views/tasks_pane.rs:32 highlight_bash_command`. Interpreter body inside `-c "…"` is highlighted as the interpreter language; the opener line stays bash.
- **Args block:** remaining scalar arguments as `key: value` lines, dim key (`description`, `timeout`, `path`, ...). Multi-line string values render with real newlines under the key. `task_name`/`description` are not repeated when they are already the row's intent.
- **Result block:** separator line is one row, `── result ──` (dim) or `── result ✗ ──` (error color). Result text rendered as written; no blank line inside; one blank row before the separator only if the command block did not already end with one.
- **Spacing rule (the single source):** exactly one blank row between blocks; zero blank rows added inside a block; user-authored blank lines inside a command/result are kept but runs of blanks > 1 collapse to 1. Remove the `\n\n` joins at 2874 and 2928, the `rows.push(String::new())` at 1440, and the blank handling in `wrap_body` 2730-2760 (it wraps one block at a time).
- **Left gutter:** body rows are prefixed with a dim `│ ` under the owning header row so the block is visibly attached to its row.
- **Long scripts:** inline body is capped at `INLINE_MAX_LINES = 40` command lines and 40 result lines; beyond that one dim row `… +N lines · Enter for all`. Wrapping: soft-wrap at width - 2 with a `↪` continuation marker; never truncates with `…` mid-line.

UI states:

| State | Rendering |
|---|---|
| Collapsed | header row only (T5 defines the header) |
| Expanded, body loading | header + one row `│ loading…` |
| Expanded, read failed | header + `│ (could not read, press l to retry)` (existing text) |
| Expanded, no body text | `│ (no body)` (replaces `NO_MORE`) |
| Tool call, no result yet | command block, no separator |
| Tool call, result ok | command + `── result ──` + text |
| Tool call, result error / failed tool | command + `── result ✗ ──` (error style) + text; header shows failed mark |
| Result empty | `── result ──` then `│ (no output)` |
| Very long script / result | capped at 40 lines + `… +N lines · Enter for all` |
| Narrow width (< 60) | gutter drops to `│`, args block drops to one `key: value` per row, wrapping at width - 2; never panics on width < 8 (min body width 8) |
| Non-tool event (prose, thinking) | prose wrapped; thinking dim italic; no gutter label |
| Unlabeled call (no intent) | no change to the body |
| Remote body | identical, `loading…` until fetched |

- **Accept:** screenshot case 2 (inline `python3 -c "…"` script) renders as readable multi-line code with no `\n` escapes; no bare numbers; result lines are single-spaced.
- **Tests (unit, in `traces_body.rs`):** `parse_body` per harness fixture (Claude `tool_use`+`tool_result`, Pi `toolCall`+usage, Codex `exec_command`, Cursor `Shell`, Grok `tool_calls`); usage/cost keys never appear (assert no chunk equals a bare integer); newline-preserving command; blank-run collapse; 40-line cap boundary (40 and 41 lines); wrap at widths 8, 20, 120; error vs ok separator. Replace the test that pins escaped `\n` (~5093-5100) with its inverse.
- **PTY:** `xai-grok-pager-pty-harness/tests/pty_e2e/traces_body.rs` (register in `tests/pty_e2e_config_ui.rs` via `#[path]`): seed a synthetic Pi session (inline python bash call, a failing call, a 100-line script) via `GROK_HOME`; assert screen text: command lines present as lines, no literal `\n`, one `── result ──`, `… +60 lines`. Snapshot at 120x40 and 50x20.
- **Verify:** `cargo test -p xai-grok-pager --lib traces_body`; build both bins; `cargo test -p xai-grok-pager-pty-harness --test pty_e2e_config_ui traces_body`.

### T3: keep your place on expand/collapse (core of `MODAL`)

- **Goal:** pressing `h/l/e/E` or a body arriving never moves the selected row's header on screen.
- **Files:** `MODAL` (`fit_rows` 2776, `row_span` 2786, `row_gaps` 2808, `ensure_bodies` 1338, fold handlers 3175/3248). Reference pattern: `scrollback/state/selection.rs` `capture_fold_anchor` 345 / `rebuild_with_fold_anchor` 360.
- **Behaviour:**
  - Before a toggle, record `anchor = (selected display row, screen y of its header)`. After heights recompute, set `scroll` so that header is at the same `y`, clamped so the header stays visible. Same rule when `ensure_bodies` completes for a visible row (height change from placeholder to body) and for `E` expand-all (anchor = selected row).
  - `fit_rows` no longer requires the whole selected block on screen. The header is always visible; the block is clipped at the viewport bottom with a dim last row `▼ N more · J/K scroll body`. While the selected row is expanded, `J`/`K` scroll within the body (body offset per selected row, reset on selection change); `j/k` still change selection.
  - A block taller than the viewport never pins the header to the top; it keeps the anchored `y`.
  - Rows above the selection never change height on toggle of the selection.
- **UI states:** collapsed->expanded near bottom (header stays, block clipped, `▼` row); expanded->collapsed (header stays at same y, rows below pull up); loading->loaded; very tall block; `E` all; last row; first row (anchor clamps to 0); terminal resize while expanded (re-anchor, clamp); empty list.
- **Accept:** in the 25-identical-rows case, pressing `l` on row 12 leaves row 12's text on the same screen line before and after, and again after the body loads; `E` does the same for the selected row.
- **Tests:** unit on a pure `anchor_scroll(old_y, new_heights, sel, viewport) -> scroll` function (extract it; 10 cases: grow/shrink/clamp-top/clamp-bottom/taller-than-viewport/resize); PTY `traces_body.rs` extension: assert header line index unchanged across `l`, `l` again after wait, `h`.
- **Verify:** same commands as T2.

### T4: Enter focuses the payload (`traces_payload.rs`)

- **Goal:** Enter on a tool call shows the full command and its result; metadata is one key away.
- **Files:** new `P/views/traces_payload.rs`; `MODAL` `open_inspector` (1487), `Mode::Inspector` arm (3315), `render_inspector` (4306) become thin delegations; `traces_hints.rs`.
- **Behaviour:** `Mode::Inspector` keeps its name; content becomes `Payload`:
  - Header (2 rows): `<tool> · <intent or headline> · exit N | ✗ error | running · <duration> · <time>`.
  - Body: the same `Vec<Block>` as T2, **uncapped**, soft-wrapped (`w` toggles wrap/horizontal scroll with `H`/`L` for columns), line numbers in the left margin for command and result blocks, highlighted command.
  - A tool call and its result (paired by tool id, as `compose_body` does today) are one payload. Enter on a result row opens the same payload.
  - `m` toggles **metadata**: the current raw pretty JSON view (type/id/parentId/usage/cost) of the selected line. `Esc` leaves metadata first, then the modal back to the list.
  - Keys: `j/k` scroll, `Ctrl-d/u` page, `g/G` ends, `w` wrap, `m` metadata, `y` copy **payload** (command text, or the focused block when `Tab` cycles blocks: command -> result -> args), `Y` copy the raw line (previous `y` behaviour), `Esc` back.
  - Enter on a non-tool event shows its prose/thinking uncapped; on a dispatch row keeps today's "open agent" behaviour (`Enter inspect/open agent`).
- **UI states:** loading body (`loading…` centered, `Esc` works); read failed (message; `r` retries, the hint says so); tool with no result (command only, header `running`); error tool (header ✗, result styled); empty result (`(no output)`); very long script (scrolls; scroll position `n/N` in the footer); narrow width (< 60: header collapses to tool + status; margin line numbers drop); wrap on/off; metadata on/off; remote session (body fetched async; same states); copy success/failure -> status line, which auto-clears (T8).
- **Accept:** screenshot case 4: Enter shows the full python script with line numbers, then the result; `m` shows the old JSON; `y` puts the script in `$GROK_COPY_FILE`.
- **Tests:** unit: payload block selection for call+result, result-only, prose; scroll clamp; wrap toggle line counts; `y` copies exactly the command text (no gutter/line numbers). PTY `traces_payload.rs`: open trajectory, Enter, assert full multi-line script visible and no `…`-truncated line; `m` shows `"parentId"`; `y` file content equals the script (use the existing `GROK_COPY_FILE` check).
- **Verify:** as T2, test filter `traces_payload`.

## 4. Phase 2: the semantic layer

### T5: semantic rows, toggle, grouping (`traces_rows.rs`)

- **Goal:** the trajectory reads like the chat TUI: `◆ Run <intent>`, `◈ Read 3 files`, `◆ Thought for 0.4s`; one key flips to the raw view.
- **Files:** new `P/views/traces_rows.rs`; `MODAL` (use `Vec<DisplayRow>`; state `view: RowView {Semantic, Raw}`), `traces_hints.rs`.
- **Toggle key:** `v` (free in all modes; `s` is sort, `e` fold, `t` tool filter). Default `Semantic`. Remembered for the modal's lifetime (not persisted). Hint `v semantic/raw`. Live mode ignores it.
- **Row formats:**

| View | Row |
|---|---|
| Semantic, call with intent | `◆ Run <intent>` (verb below) |
| Semantic, call without intent | `◆ Run <headline>` in normal color; headline dim if it is a path/pattern only (`◆ Read parent/base.ts`) |
| Semantic, reasoning | `◆ Thought for 0.4s` (duration from event usage; `◆ Thought` if absent) |
| Semantic, tool failed | verb row with red `✗` suffix and the exit/first error line dim after `·` |
| Raw | `<tool>  <headline>` (today), plus `· <intent>` dim suffix when intent exists |
| Edit/Write | `◆ Edit index.ts +6/-2`, `◆ Creating scope.ts +57/-0` (T7 supplies counts; without them no suffix) |

  Verb map (tool name, case-insensitive): Bash/Shell/exec_command/run_terminal_command -> `Run`; Read/read/ReadFile -> `Read`; Edit/MultiEdit/StrReplace/edit/search_replace/apply_patch -> `Edit`; Write/write/create -> `Creating`; Grep/Glob/Find/LS -> `Search`/`Listed`; Agent/Task/Subagent -> `Agent`; WebFetch/WebSearch -> `Fetch`/`Search`; MCP `server.tool` -> the name; unknown -> the tool name. Mirrors the chat TUI's labels; the table lives in one `const` in `traces_rows.rs` with a test that every `kind=tool` title in the legibility fixtures maps to a non-empty verb.
- **Grouping:** consecutive display rows that are read-only calls (Read/Search/List/Fetch, and Run with no intent that is `ls`/`cat`/`rg`/`git status|log|diff`) of >= 2 collapse to `◈ Read 3 files` / `◈ Listed 1 dir, Read 2 files · 2 failed`. Any non-read event (edit, prose, run, user turn) ends the group. Group is a `DisplayRow::Group`; `h/l/e` expands it into its member rows (members then render per the active view); failed members are counted via the same predicate as the single rows. Step 0: read `scrollback/state/verb_group.rs` (`scan_run_forward` 104, `verb_group_header_label` 182, `push` 263, `into_label` 314, `failed` 353-354, `block_failed` 370). If its builder takes plain counts, reuse it; if it requires `ToolCallBlock`, extract the label-composition part into `xai-grok-pager-render` or copy it into `traces_rows.rs`, and add a parity test on the same inputs. Raw view never groups.
- **Selection over groups:** `j/k` step over display rows; `n` noise, `x` errors, `t` tool filters apply to events first, then grouping runs on the filtered list; a group containing the search hit auto-expands.
- **UI states:** semantic on/off; collapsed group; expanded group (members indented 2); group with failures (`· N failed` in red); group of 2; single read (no group); unlabeled call; reasoning without duration; tool with long intent (clip at width - 6 with `…`); narrow width (verb + clipped intent, never wraps); filters active (`t bash` shows only Run rows, ungrouped); empty trajectory (existing empty state); noise rows (dim, unchanged); `v` pressed during search prompt (typed into the query, not a toggle); remote sessions identical.
- **Accept:** a Pi session with 25 `python3 -c` calls shows 25 distinct `◆ Run <task_name>` rows; `v` flips to `bash  python3 -c " ⏎ …` rows and back with selection preserved (same event stays selected).
- **Tests:** unit: `build_rows` (grouping boundaries, failures, filter interaction, selection mapping both ways: `display_to_event`, `event_to_display`), verb map, label formats, `v` toggle preserves selected event. PTY `traces_rows.rs`: seeded session; assert `◆ Run `, `◈ Read 3 files`, failed suffix, `v` toggle text change, expand group with `l`.
- **Verify:** as T2, filter `traces_rows`.

### T6: edit payloads per harness (`xai-grok-traces/src/edits.rs`, pure)

- **Goal:** one typed value for "what this call changed", from any harness, plus counts at index time.
- **Files:** new `T/edits.rs`, `T/lib.rs`, `T/normalize.rs` (call), `T/index/*` (schema 8 adds `trace_events.added INTEGER`, `removed INTEGER`, NULL for non-edits), `tests/edits.rs` + fixtures `tests/fixtures/edits/*.jsonl` (synthetic).
- **API:** `pub fn edit_payload(source: &str, tool: &str, args: &Value) -> Option<EditPayload>`; `EditPayload { path: String, kind: Create | Edit | Delete, parts: Vec<EditPart> }`; `EditPart = Replace{old, new} | Content(String) | Patch(String)`. Counts: `similar::TextDiff` line diff over Replace pairs; Content = lines added; Patch = `+`/`-` line counts. `similar` is already a workspace dependency (`Cargo.toml:272`; used by `xai-grok-pager-diff`).
- **Per-harness source (verify each exists in logs before coding; fixtures copy the verified shape):**

| Harness | Tool | Old/new text from | Verified shape |
|---|---|---|---|
| Claude | `Edit` | `input.old_string` / `input.new_string`, `file_path`, `replace_all` | seen: `{"replace_all":false,"file_path":…,"old_string":…,"new_string":…}` |
| Claude | `Write` | `input.content`, `file_path` -> Create. The log cannot say whether the file existed; show `Creating` unless the session has an earlier Read/Edit of that path, then `Edit` with all lines added | `{"file_path":…,"content":…}` seen |
| Claude | `MultiEdit` | `input.edits[]` each `{old_string,new_string}` | (U): no `MultiEdit` in sampled logs. `rg -m1 '"name":"MultiEdit"' ~/.claude/projects` |
| Cursor | `StrReplace` | `input.path`, `old_string`, `new_string` | seen |
| Cursor | `Write` | `input.path`, `input.contents` | seen (`contents`, not `content`) |
| Grok | `search_replace` | `rawInput.file_path`, `old_string`, `new_string` | seen in `updates.jsonl` |
| Codex | `apply_patch` | patch text (`*** Begin Patch` / `*** Update File:` / `*** Add File:`), `payload.input` of a `custom_tool_call` (U) | verify on zstd rollouts: `zstd -dc <rollout>.jsonl.zst \| rg -m1 apply_patch` ; note `classify.rs` `patch_path` already parses these headers |
| Pi | `edit` | `arguments.path` + `arguments.edits[]` each `{oldText,newText}`; legacy top-level `{path,oldText,newText}` also accepted | verified: Pi schema + real Air log, see `docs/HARNESS-FACTS.md` |
| Pi | `write` | `arguments.path`, `arguments.content` | verified: see `docs/HARNESS-FACTS.md` |

  If a shape is absent in real logs, keep the parser, mark the fixture "synthetic from schema", and say so in the test name (`*_schema_only`). Cases the payload cannot support: Codex patch with binary hunks (skip, no counts); tool result says failure (counts still index the *attempted* change; the row shows `✗`).
- **Accept:** each row of the table with a verified shape has a passing fixture test; counts equal `git diff --numstat` on the fixture's real before/after.
- **Tests:** per-row fixture; `replace_all`; empty `new_string` (pure delete -> `+0/-N`); CRLF; 1 MB content (counts computed, payload not stored); malformed args -> `None`.
- **Verify:** `cargo test -p xai-grok-traces edits`; scan example prints edit-event counts per source on mini data.

### T7: inline diffs (`traces_body.rs`, `traces_rows.rs`)

- **Goal:** `◆ Edit index.ts +6/-2` expands to a line-numbered, syntax-highlighted diff; a created file shows highlighted all-added.
- **Files:** `P/views/traces_body.rs` (`Block::Diff`), `traces_rows.rs` (suffix), `traces_payload.rs` (full diff).
- **Behaviour:** body load calls `edit_payload` on the call's args; each `Replace` becomes hunks via `xai_grok_pager_diff::diff_hunks_from_strings(old, new, 1)` (`xai-grok-pager-diff/src/lib.rs:169`) with line numbers from the file if known else 1; `Content` -> `diff_hunks_from_strings("", content, 1)`; `Patch` -> per-file hunks parsed from the patch text. Render with `render_diff_hunks_highlighted` (`P/scrollback/blocks/tool/edit.rs:137`, walker 149, spans 688), converting `DiffLineOutput` background colors to the modal buffer. Header suffix format copied from edit.rs ~945-965: green `+N`, red `-M`. Step 0: confirm `render_diff_hunks_highlighted` and `DiffRenderConfig` are `pub(crate)` or reachable from `views/`; if the module is private, widen visibility in a one-line change (no logic change).
- **Collapsed row:** `◆ Edit index.ts +6/-2` (semantic) or `edit  index.ts · +6/-2` (raw). **Expanded:** diff, capped at 60 diff lines inline with `… +N lines · Enter for all`; Enter shows the full diff in the payload view with `n`/`N` jumping hunks.
- **UI states:** edit ok; edit failed (header ✗, diff dim, result text below); create (all green, `Creating` verb); delete (all red); multi-hunk (separator rows); multi-file patch (one sub-header per file, collapsed hunks per file past 3 files); no payload available (old index / unknown shape: show the args block, no suffix); huge diff (> 5000 lines: skip highlight, plain +/- with a notice row); binary/unreadable; loading; narrow width (< 50: drop line numbers, keep +/- gutter); light and dark theme backgrounds (use `Theme` accessors the chat block uses; no hard-coded colors); raw view (same diff, label differs); remote session.
- **Accept:** Edit/Write/StrReplace/search_replace/apply_patch/Pi edit fixtures render with backgrounds on +/- lines and line numbers; counts in the row equal the diff.
- **Tests:** unit: hunk construction per payload kind, count == rendered +/- rows, cap boundary, theme backgrounds present (assert `bg.is_some()` on added/removed lines), narrow width. PTY `traces_diff.rs`: seeded Claude Edit + Write + Cursor StrReplace; assert `+6/-2`, a removed and added line each, line numbers; `Enter` full diff.
- **Verify:** as T2, filter `traces_diff`.

### T10: workflow runs in traces

- **Goal:** workflow phase and log events are rows in the session's trajectory (not only live chat blocks).
- **Data (U, Step 0):** grok-build persists a run at `<session_dir>/workflows/<run_id>/state.json` (`xai-grok-shell/src/session/workflow/store.rs:285`) holding `WorkflowRunState` (`tracker.rs:140`: `name, objective, status, phases, current_phase, agents_used, agent_budget, history[], agents[], result_summary`). Step 0: find a real session with a `workflows/` dir (none on mini today: `fd -t d workflows ~/.grok/sessions` is empty), or run one workflow in a scratch `GROK_HOME` on mini; record the `history` entry shape (`WorkflowHistoryEntry`) in MAP.md.
- **Files:** `T/roots.rs` (grok root also walks sibling `workflows/*/state.json`), new `T/workflow.rs` (state -> synthetic events), `T/index/*` (events get `line_number` from a reserved range `>= 1_000_000_000 + n` so they never collide with `chat_history.jsonl` lines; `raw_line` for them reads the state file), `T/normalize.rs`, `P/views/traces_rows.rs` (verb `Workflow`), `tests/workflow.rs`.
- **Events:** one `workflow` kind per `history` entry: phase entered (`◆ Phase 2/5 research`), log message (`  log: …`), agent spawned/finished (`agent-3 done · 12k tok`), run completed/failed/cancelled with `result_summary`. Timestamps from the entry; if absent, ordered at the run's mtime.
- **UI:** semantic view shows these rows in order, interleaved by timestamp with chat events; a run header row `◇ Workflow <name> · running|done|failed · phase N/M · agents used/budget`. Enter shows the state JSON field set (phase, agents table). Raw view shows `workflow  <phase|log>`.
- **States:** running (state.json re-read on the 1s live poll when status is running), done, failed, cancelled, paused (`pause_message`), no history yet (header only), agents list > 256 (cap note), state.json unreadable/partial (row `workflow state unreadable`, no panic), remote session (fetched via the existing `events` subcommand: events are in the index, so no new wire).
- **Accept:** a scratch run with 3 phases and 4 logs yields 1 header + 3 phase rows + 4 log rows + completion in order; the live modal shows rows appear while the run runs.
- **Tests:** unit fixture of `state.json` -> events (each status); index test for scan + append (state file changes size, not the jsonl); PTY `traces_workflow.rs` seeded `workflows/wf_1/state.json`.
- **Note:** this ingests what the shell persists. Live Pi workflow progress that never reaches the screen is the `gateway_rx` drain in Track B (B3), not something traces can fix; once B3 lands, T10 shows Pi-engine runs too (the run header carries the engine).

## 5. Phase 3: replace the non-functional UI, require intent

### T8: dead UI, hints, status, empty rows

- **Files:** `traces_hints.rs`, `MODAL` (status fields, resume), `P/app/effects/mod.rs` (~2788), `P/app/dispatch/router.rs` (1441-1453), `P/app/dispatch/session/load.rs` (`dispatch_traces_resume`), `T/normalize.rs`, `P/slash/commands/traces.rs`, `docs`: `xai-grok-pager/docs/user-guide/04-slash-commands.md` `### /traces`.
- **Hints:** `shortcuts(state)` builds from state; a key is listed only if it will do something now.
  - `H host`: only when `state.hosts` is non-empty.
  - `r resume`: only when `resume_action(row)` is `Some`.
  - `Enter inspect/open agent`: "open agent" only on dispatch rows, else `Enter payload`.
  - Add the real, unlisted keys: `g/G ends`, `y copy prompt` (trajectory), `c` where Live uses it (name from `live_toggle`), `v semantic/raw`, `w wrap` and `m meta` in payload. `Space` stays an undocumented alias (do not list).
  - Hint bar wraps to the 2 footer lines; if it overflows, drop in this order: `E`, `</>`, `n`, `x`.
- **Resume per harness:** `resume_action(row) -> Option<ResumeAction>` with `ResumeAction::InPager(ResumeTarget)` (claude, codex, grok: unchanged) or `CopyCommand(String)`:

| Source | Action | Command (flag verified from `--help` on mini) |
|---|---|---|
| pi | CopyCommand | `cd <cwd> && pi --session <file_path>` (`pi --help`: `--session <path\|id>`, `--resume/-r` picker, `--continue/-c`) |
| omp | CopyCommand | `cd <cwd> && omp --resume <file_path>` (`omp --help`: `-r, --resume=<value>` ID prefix or path) |
| cursor | CopyCommand | `cursor-agent --resume <chat id>` (`cursor-agent --help`: `--resume [chatId]`; chat id = transcript stem) |
| codex | InPager (existing) | `/resume-codex <id>` |
| dsh, bb | none: no `r` hint; `r` sets status `no resume command for dsh` | (U) flags not checked; run `dsh --help`, `bb --help` and promote to CopyCommand if a flag exists |

  `CopyCommand` copies via the existing `TracesCopy` path and sets status `copied: <cmd>`. Remote sessions: command prefixed `ssh <host> '…'`. Replace the "no resume for Pi" message.
- **Status line:** `status` becomes `Option<(String, Instant)>`; cleared 4 s after set (checked on the existing 1s poll tick and on every key), and on any navigation key or mode change. `indexed N changed files` is shown only when `N > 0` and `OpenTraces` triggered a scan the user can see; a scan with 0 changes shows nothing; failures (`trace scan failed`, `files failed`) persist until a key press.
- **Empty `› Assistant` rows:** in `normalize`, an assistant/message record with no text, no thinking and no tool block is `noise = true` (its token usage is still attached to the event so totals are unchanged). Test: a Claude `assistant` line with only `usage` yields noise, and `event_count`/token sums match before/after.
- **UI states:** hint bar with hosts/without; resume available/copy/none; status set/auto-clear/overwritten; scan 0/N/failed; every mode's hints (ledger, trajectory, live, payload) tested.
- **Accept:** no hint is shown for a key that does nothing in that state; `r` on a Pi row copies `pi --session …`; a status message disappears without input within 5 s; no empty Assistant rows in the default trajectory of the screenshot's Pi session.
- **Tests:** unit: `shortcuts()` per mode x state matrix (hosts empty/non-empty; row source pi/claude/dsh); `resume_action` per source with quoting of paths containing spaces/`'`; status expiry with an injected clock; normalizer noise test. PTY `traces_hints.rs`: ledger with no hosts has no `H host`; Pi row `r` -> status `copied: pi --session`, gone after 5 s.
- **Verify:** as T2, filter `traces_hints`; `cargo test -p xai-grok-traces`.

### T9: hooks that require intent

- **Goal:** the model cannot run an opaque shell call; it must say why.
- **Files:** `xai-grok-traces/hooks/claude/require-intent.sh`, `hooks/pi/require-intent.ts`, `hooks/install.sh` (new `--require-intent` flag, opt-in, with `--remove`), `hooks/test.sh`, `MAP.md` (Hooks table), `tests` none (shell tests in `hooks/test.sh`).
- **Policy (shared):** applies to shell tools only (Claude `Bash`, Pi `bash`, Cursor `Shell`, Codex `exec_command`). Allowed without intent: a command shorter than 40 chars with no newline whose first word is in `ls pwd cat head tail wc echo git(status|log|diff|branch) rg cd which`. Otherwise required: Claude `tool_input.description` >= 8 chars; Pi `task_name` >= 8 chars; or a leading `# why: …` line (any harness, because `command` is free text; `command_headline` in T1 reads it). Disabled by `GROK_TRACES_REQUIRE_INTENT=0`. Fails open: hook errors or unparseable input exit 0 with no output.
- **Claude (verified shape; same as `~/.claude/hooks/guard-bash.sh`):** `PreToolUse` matcher `Bash`; on violation print
  `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Add a brief description (what and why, 8+ chars) to this Bash call, then retry."}}` and exit 0. `install.sh` adds the entry beside the traces entries without touching other hooks (existing idempotent-edit rules).
- **Pi:** extension `pi.on("tool_call", …)` returns `{ block: true, reason }` for `bash` calls lacking intent (`extensions.md` ~282-290 in the installed Pi docs). (U) first: does stock Pi's `bash` schema accept an extra `task_name` argument, or strip/reject it? Test: run `pi -p` with the extension that logs `event.input` for a call the model made with `task_name`. If stripped, the Pi rule is the `# why:` comment only, and the reason text says so.
- **Codex:** `~/.codex/hooks.json` `PreToolUse` (shell only); deny = stdout `{"decision":"deny","reason":"…"}`, exit 2, documented in `agent-harness-hooks` (see `docs/HARNESS-FACTS.md`). Not yet live-tested: Step 0 runs one session proving the call is blocked and the reason reaches the model, then records it in HARNESS-FACTS.
- **Cursor:** `preToolUse`/`beforeShellExecution` reply `{"permission":"allow"}` today; no deny path exists in any repo (see `docs/HARNESS-FACTS.md`). Step 0: one live session testing `{"permission":"deny","userMessage":"…","agentMessage":"…"}`; record the result in HARNESS-FACTS before code.
- **Grok:** none; its bash tool fills `description` 100% already.
- **UI states (the hook's own):** allow (silent), deny (reason visible to the model), disabled via env, hook error (fail open, one stderr line), already-labeled call, exempt short command, retry after deny (second call with description passes), non-shell tool (never touched).
- **Accept:** in a Claude session, `Bash` with no description and a 5-line script is denied once; the retry with a description runs; traces then shows `◆ Run <description>`. `ls` runs unprompted.
- **Tests (`hooks/test.sh`):** table of stdin payloads -> expected stdout/exit for Claude (allow/deny/exempt/disabled/garbage input); Pi extension unit via node >= 22 with a fake `pi.on`.
- **Verify:** `bash crates/codegen/xai-grok-traces/hooks/test.sh`; then `install.sh --require-intent --dry-run` diff on mini's real configs, reviewed by Sab before `--apply`.


# Track B: native Pi host in grok-build

Independent of Track A: no shared files, no ordering between the tracks. Source tree `GP` = `/Volumes/Mini-SSD/offload/repos/grok-pi` (github `skchaudr/grok-pi`; diverged ~376 commits each side from merge-base `37949780`).

**Native-first rule.** The native host becomes the default way `grok pi` runs once the parity checks below pass (B8). `pi-acp` stays only as the fallback (`--acp`, and automatic when the native host cannot start), not as the goal. Until B8, the default is unchanged and `scripts/acp-matrix.sh pi` keeps passing.

**Superseded context.** An agent wrote in `docs/khoj-pi-migration.md` (2026-10-19) that adapter changes go to `~/repos/Pi-Coding-Agent/pi-acp-patched/`, not to grok-build. Superseded by Sab's direction that Pi be native in grok-build.

## B.1 Inventory (verified on disk, mini, 2026-10-26)

| Piece | Where in `GP` | Present in grok-build? |
|---|---|---|
| Adapter | `crates/codegen/pi-grok-adapter`: 32 `.rs` files, ~19.3k lines (largest: `pi_adapter.rs` 1774, `model.rs` 1745, `pi_adapter/agent.rs` 1231, `pi_adapter/tools.rs` 1023, `pi_adapter/session.rs` 1009, `pi_rpc.rs` 963, `tool_projection.rs` 944) | no |
| Composition binary | `xai-grok-pager-bin/src/bin/grok-pi.rs` (1598) + `bin/grok_pi/*.rs` (32 files, 7.9k lines: one `*_extension.rs` per bundled Pi extension, `cli.rs`, `runtime_config.rs`, `extension_self_heal.rs`, `pi_version.rs`, `migrate_home.rs`) | no |
| Pi extensions | `extensions/pi-grok-*` (23 dirs + `_archived`): `bash` (5.3k lines TS incl. eval/mcpeval), `subagents`, `workflows`, `tools`, `web-config`, `shortcut-manager`, `plan-mode`, `goal`, `loop`, `btw`, `todo`, `recap`, `rollback`, `herdr`, `auth`, `context`, `export`, `ask-user-question`, `native-commands`, `remote-tui`, `rpc-compat`, `rust-tui-bridge`, `00-profiler` | no |
| Shell | `xai-grok-shell/src/session/workflow/{backend.rs (218), external.rs (220)}`, `src/host_features.rs` (524) | no |
| Pager only in GP (`diff -rq`) | `native_feature_conflicts.rs`, `pi_model_config.rs`, `pi_resource_config.rs`, `pi_resource_policy.rs` (1261), `views/{pi_config (1840), pi_models (1982), pi_settings/, shortcut_manager, subagent_catalog_pane, session_tree (1567), tree_map, fork_picker, review, cache_graph}.rs`, `scrollback/blocks/tool/{eval (603), codemode (746), hook, side_by_side_edit}.rs`, `slash/commands/{create_workflow, eval_display, pi_config, pi_models, pi_shortcut_manager, plan_mode, reload, scoped_models, tree, tree_map, clone_cmd, review, hotkeys, notify}.rs`, `app/extension_shortcuts.rs`, `app/dispatch/session/{pi_fork,tree}.rs` | no |
| Overlapping (both forks edited) | `xai-workflow/src/{engine,journal,lib,meta}.rs`; shell `workflow/{manager,host_service,tracker,notify,registry,mod,schema_contract,store}.rs`; pager `acp/tracker.rs`, `acp/spawn.rs`, `views/tasks_pane.rs`, `views/workflows.rs`, `scrollback/blocks/workflow.rs`, `slash/commands/workflow{,s}.rs`, `app/acp_handler/settings.rs` | both |

Combined `diff` line counts between the two trees today (both sides' changes together; the per-side split needs the merge-base diff that B0 produces): `workflow/manager.rs` 322, `host_service.rs` 120, `tracker.rs` 41, `notify.rs` 21, `registry.rs` 20, `mod.rs` 15, `schema_contract.rs` 13, `store.rs` 8; `xai-workflow` 6/12/2/5; pager `acp/tracker.rs` 1083, `views/tasks_pane.rs` 1066, `acp/spawn.rs` 122, `views/workflows.rs` 111, `acp_handler/settings.rs` 82, `slash/commands/workflow.rs` 22, `scrollback/blocks/workflow.rs` 13.

**Conflict-resolution order** (each step compiles and passes its tests before the next): `xai-workflow` -> shell `workflow/` leaf files (`store`, `schema_contract`, `registry`, `notify`, `tracker`, `mod`) -> `manager.rs` + `host_service.rs` (three-way against `37949780`; keep grok-build behaviour where GP's change is Pi-only; grok-build's `host_service.rs:501` already maps `isolation_worktree`) -> add `backend.rs`, `external.rs`, `host_features.rs` -> pager workflow views and slash -> `acp/spawn.rs`, `acp_handler/settings.rs` -> `acp/tracker.rs` and `views/tasks_pane.rs` last.

## B.2 Parity checks

Measured against the B0 baseline. A check that is red in GP at baseline is fixed in grok-build as part of the slice that ports it; it is never accepted red. Run on sab-mini, real Pi where noted. `pi-acp` is the comparison only for PC1-PC6.

| ID | Check |
|---|---|
| PC1 | `scripts/acp-matrix.sh pi-native` (B1 adds this agent id beside `pi`): launch, prompt, assistant text, tool card |
| PC2 | a `bash` call with `task_name` shows it as the card title (GP `tool_projection.rs:402-413`); the traces label (T1) is the same string |
| PC3 | auth-missing and bad-model errors show the same user-facing messages as `pi-acp` (scenarios recorded in `docs/pi-acp-error-surface-proof/`, agent-written) |
| PC4 | `/resume` lists Pi sessions (`psm_session_catalog.rs`) and loads one |
| PC5 | Esc during a long tool call cancels it and the card shows cancelled |
| PC6 | `scripts/acp-matrix.sh pi` (fallback path) still passes after every slice |
| PC-W1 | `/workflow <rhai script>` with 3 phases: phase and log rows appear while running (the drain fix), final summary at the end |
| PC-W2 | a test engine (in-memory, implements `WorkflowEngine`) drives the unchanged pager workflow UI: no engine-specific pager code |
| PC-W3 | a Pi-side workflow package (see B3) runs and its phases appear in the same UI (U until Step 0 of B3 closes) |
| PC-W4 | schema contract: invalid output is retried once (`SCHEMA_CONTRACT_RETRIES = 1`, `schema_contract.rs:1`), then surfaced as a failed step |
| PC-W5 | pause and stop cancel running children (`cancel_run_children`) |
| PC-S1 | a `read-only` child cannot call `edit`, `write` or `bash` (tool list equals `CAPABILITY_TOOLS["read-only"]`) |
| PC-S2 | child sessions are written under the parent's session directory; the parent's session file is unchanged by the child |
| PC-S3 | `isolation = worktree` either runs the child in a separate worktree or fails loudly; never silently runs in the parent tree (today's GP behaviour, see B4) |
| PC-S4 | `x.ai/subagent/cancel` stops a running child; rows update |
| PC-E1 | a foreground `bash` call past its timeout is promoted to a background task shown in the task pane; `x.ai/task/kill` kills its process tree |
| PC-E2 | Eval v2: `store()`/`load()` across cells, `parallel()`, timeout, abort (the existing `test-v2.1.mjs` cases) |
| PC-E3 | a background Eval job reports completion and its output is retrievable |
| PC-F1 | every setting in the bundled `grok-pi.json` manifests appears in the settings UI under its section with its default, and a toggle writes its `configPath` |
| PC-F2 | a restart-required toggle says so and applies `startupEnv` on next start |
| PC-F3 | with `pi_workflows` on, a package listed under it in the conflicts file is refused at resource admission; with it off, admitted |
| PC-F4 | the user overlay `$GROK_HOME/native-feature-conflicts.toml` merges over the bundled defaults |

## B.3 Slices

### B0: baseline and seam gate (serial; first)

- **Goal:** know what is red in GP before measuring anything against it, and know exactly what grok-build lacks.
- **Baseline commands (GP, mini):** from `verify.sh`: `./scripts/cargo-shared.sh check -p xai-grok-pager-bin --bin grok-pi`; `./scripts/cargo-shared.sh test -p pi-grok-adapter`; `... test -p xai-grok-pager --lib external_builtin_filter_accepts_aliases_and_omits_product_commands`; `... test -p xai-grok-pager --lib slash_compact_with_context_enqueues_command`. Node suites: `node extensions/pi-grok-bash/test-v2.1.mjs`, `test-mcpeval-daemon.mjs`, `test-eval-pi-mcp.mjs`, `test-mcpeval-oauth.mjs`; the `*.test.ts` files in `extensions/pi-grok-subagents` and `extensions/pi-grok-web-config/tests` (runner not declared in `package.json`; B0 finds how they run).
- **Baseline measured by this spec's author (mini, 2026-10-26, in GP's own target dir, not the shared one):**
  - `cargo test -p pi-grok-adapter --lib`: **green, 191 passed, 0 failed, 0 ignored.** Only the lib target was run; the crate has no Rust integration tests (`tests/` holds `mock_pi_contract.py` and `mock_pi_rpc.py`). So the adapter's own unit tests are not broken in GP today.
  - `python3 -I crates/codegen/pi-grok-adapter/tests/mock_pi_contract.py --pi-source pi-main`: **red**, `FileNotFoundError: pi-main/packages/coding-agent/src/modes/rpc/rpc-types.ts`; `pi-main/` is empty in this checkout, so `verify.sh` cannot pass as written.
  - `node extensions/pi-grok-bash/{test-v2.1,test-eval-pi-mcp,test-mcpeval-daemon,test-mcpeval-oauth}.mjs`: **red, cannot run**, each fails resolving a module under `pi-main/node_modules` (`esbuild`, Pi packages). They need `pi-main` populated or their imports pointed at an installed Pi (B0 decides which).
  - `cargo test -p xai-grok-pager --lib external_builtin_filter_accepts_aliases_and_omits_product_commands` (and the second `verify.sh` pager test): **red, the pager lib test target does not compile in GP: 48 `error[...]` lines**, e.g. test code constructing `AppView` without `screen_mode_switch_hint` / `session_picker_detail_generation`, `SessionPickerEntry` without `first_message`, `name`, `parent_session_path`, `session_kind`; `dual_config` not found; `ScrollbackState::latest_turn_marker_accepting` missing; `SubagentInfo::is_background` missing (output kept in `/tmp/pi-pager-baseline.txt` on the mini during this run). So GP's pager unit tests cannot be inherited as parity evidence; each Track B pager slice writes its pager tests fresh against grok-build, where the pager lib tests build (an agent wrote 10389 passing in `MAP.md`; T0 re-measures).
  - Not run: `cargo check -p xai-grok-pager-bin --bin grok-pi` (long build; B0 runs it).
  - Consequence for parity: adapter-level behaviour (191 green tests) is a valid baseline; pager-level and node-suite behaviour have no working baseline, so those checks (PC-E2 especially) are measured against the running product on mini, not against a green test run.
- **Output:** a table in this section, one row per command: green/red/not-run, failing test names, cause. Plus `cargo check -p pi-grok-adapter` after copying the crate into grok-build, listing every missing shell/pager symbol (known from `use xai_*`: `session::workflow::{ExternalWorkflowRuntime, ExternalWorkflowRuntimeConfig, WorkflowNotifySender, WorkflowRunStore, workflow_session_notification_json, list_workflows, WorkflowAgentBackend, HostDrainOutcome, WorkflowAgentSpawnRequest, WorkflowAgentSpawnResult}`, `session::persistence::PersistenceMsg::WorkflowRunStateAndAck` (present), `session::SessionInfoResponse` (present, `acp_types.rs:566`), `config::load_effective_config{,_disk_only}` (U), `xai_workflow::{WorkflowOutcome, HostError}`, `xai_acp_lib::{AcpAgentGatewaySender, AcpClientMessage, acp_send}`), and the merge-base diffs for the overlap table.
- **States:** none (no UI). **Accept:** baseline table committed in this file; seam list complete. **Tests:** none new.

### B1: RPC, session core, launcher

- **Files:** adapter `lib.rs`, `pi_rpc.rs`, `pi_adapter.rs`, `pi_adapter/{agent,session,events,recovery,replay,notifications}.rs`, `model.rs` (+`model/tests.rs`), `psm_session_catalog.rs`; `Cargo.toml` workspace member; composition: a minimal `bin/grok_pi` port (`cli.rs`, `runtime_config.rs`, `pi_version.rs`, `session_paths.rs`, `home.rs`) behind `grok pi` (soft subcommand like `Command::Live` in `P/app/cli.rs`) and an agent id `pi-native` in `scripts/acp-matrix.sh`. Pi is found on PATH; its `--mode rpc` JSONL protocol is driven in-process; the adapter serves the pager over in-memory ACP.
- **Behaviour:** prompt, stream, cancel, model list and switch, session new/list/load/rename, `/tree` data, recovery after Pi exits. `--no-extensions` plus an explicit allowlist of bundled extensions (as GP's `grok-pi.rs` does at its lines ~634-650); user-installed packages load only through the F2/conflict policy (B6).
- **UI states:** Pi missing on PATH (error card with install hint, not a hang); Pi version below the supported floor (message with the version found; floor from `pi_version.rs`); Pi dies mid-turn (recovery card, prompt re-enabled); auth missing; model catalog empty; session load failure; fallback engaged (`pi-acp` started because native failed: one-line notice naming the reason).
- **Accept:** PC1, PC3, PC4, PC5, PC6. **Tests:** ported adapter unit tests for the files above; pty `host_pi_native_smoke.rs` using `tests/mock_pi_rpc.py` (GP) as the fake Pi so CI needs no Pi.
- **Verify:** `cargo test -p pi-grok-adapter`; `cargo test -p xai-grok-pager-pty-harness --test pty_e2e_config_ui host_pi`.

### B2: tool projection

- **Files:** `tool_projection.rs` (+`tool_projection/tests.rs`), `pi_adapter/tools.rs`, `context_projection.rs`, `cache_metrics.rs`; pager `scrollback/blocks/tool/side_by_side_edit.rs` and `hook.rs` (both ported).
- **Behaviour:** Pi tool events become ACP tool calls; `raw_input.description` := `task_name` when `description` is empty (`tool_projection.rs:402-413`); tool kinds and titles; terminal output deltas; grep alias `ignoreCase` -> `-i`.
- **States:** pending, running with streaming output, completed, failed, cancelled; very long output (truncation marker); unknown tool (generic card with raw input); `task_name` only / `description` only / both (description wins) / neither (command headline).
- **Accept:** PC2. **Tests:** `tool_projection/tests.rs` ported; add the four label cases.

### B3: workflow host, engine-pluggable (a)

- **Goal:** one workflow surface (`/workflow`, `/workflows`, `/create-workflow`, the `workflow` tool, phase/log rows, toasts) in front of any number of engines. Rhai (`xai-workflow`) is the first engine; Pi-side workflow packages are the second; a Rust-native engine can follow without touching the pager.
- **Facts:** the Rhai engine is the `xai-workflow` crate (in both trees). GP wraps it as `ExternalWorkflowRuntime` (`external.rs`: `launch_named`, `launch_inline`, `pause`, `cancel`, `list_runs`, `elapsed_ms`) and spawns children through the `WorkflowAgentBackend` trait (`backend.rs:72`: `spawn_and_await`, `cancel_run_children`). Adapter `workflow_host.rs` wraps that runtime. Pager rendering is driven by `WorkflowRunState` (`tracker.rs:140`: status, phases, current_phase, history, agents, agents_used/agent_budget, result_summary) via `build_workflow_updated` (`notify.rs:97`), which is engine-agnostic already.
- **Drain fix (root cause, `workflow_host.rs:45`):** `tokio::spawn(async move { while gateway_rx.recv().await.is_some() {} })` discards every message `WorkflowNotifySender` sends through `AcpAgentGatewaySender::new(gateway_tx)`, including live phase and log updates. Only a poll (`notification_payloads()`, `workflow_host.rs:107`, on the named-launch path) delivers progress, which is why Pi workflows only pulse. Replace the drain with a forwarder from `gateway_rx` to the adapter's ACP client channel (`xai_acp_lib::acp_send`) as the same `x.ai/workflow/*` notifications, keyed by `session_id`, de-duplicated by `WorkflowRunState.revision`; keep the poll as fallback.
- **Interface (new `xai-grok-shell/src/session/workflow/engine.rs`):**
  ```rust
  #[async_trait]
  pub trait WorkflowEngine: Send + Sync {
      fn id(&self) -> &'static str;                         // "rhai", "pi", ...
      fn discover(&self, cwd: &Path) -> Vec<WorkflowDescriptor>; // name, source, engine
      async fn launch(&self, req: WorkflowLaunch) -> Result<String, HostError>; // returns run_id
      async fn pause(&self, run_id: &str) -> bool;
      async fn cancel(&self, run_id: &str) -> bool;
      fn runs(&self) -> Vec<WorkflowRunState>;
      fn events(&self) -> mpsc::UnboundedReceiver<WorkflowEvent>; // push; PhaseEntered, Log, AgentSpawned/Finished, RunStarted/Completed/Failed/Cancelled
  }
  ```
  An `EngineRegistry` maps ids to engines; run ids are prefixed `<engine>:`. `WorkflowEvent` folds into `WorkflowRunState` in one place, so every engine reaches the pager through the existing notifications.
- **Engines:** (1) `RhaiEngine` = `ExternalWorkflowRuntime` unchanged behind the trait; its children go through `WorkflowAgentBackend` implemented by B4's executor. Schema validation and lifecycle stay as in `schema_contract.rs` (256 KiB schema cap, external `$ref` rejected, one retry) and `manager.rs`/`host_service.rs`. (2) `PiWorkflowsEngine`: adapter around workflow packages that run inside Pi. Both candidates exist on npm (`npm view`, 2026-10-26): `@osolmaz/pi-workflows` 0.17.7 ("Workflow and resource manager runtime with a live terminal viewer for the pi coding agent") and `pi-extensible-workflows` 6.3.0 ("Deterministic multi-agent workflow orchestration and codemode scripts for Pi"). **Neither is installed on the mini** (`pi list` shows only `npm:pi-clinepass-provider`; `~/.pi/agent/extensions` holds `herdr-agent-state.ts`, `traces-capture.ts`, `traces.ts`). An agent wrote in `docs/khoj-pi-migration.md` that the Air has both, and in grok-build `OBSERVATIONS.md:587` (cited there) that `pi-extensible-workflows` registers a second `workflow` tool that makes `pi --mode rpc` exit. (3) Future: a Rust-native engine implements the trait; nothing else changes.
- **Step 0 (closes the U markers before `PiWorkflowsEngine` code):** in a scratch project directory (`pi install <source> -l`, `-l` = project-local per `pi install --help`; do not touch `~/.pi/agent/settings.json`), install each candidate; record: how a workflow is defined and launched (tool, slash command, file format), whether progress is observable (Pi events, RPC, a state file), where state persists, what tool names it registers, and whether both active at once reproduce the `pi --mode rpc` exit. Write the answers into this section. Until recorded, `PiWorkflowsEngine::discover` returns empty and the registry hides it.
- **One `workflow` tool:** the host exposes exactly one `workflow` tool to the model. A Pi package's own `workflow` tool is disabled by the host through the resource-admission policy (B6) or the package's extension toggle, whichever Step 0 shows works.
- **Selection:** `/workflow <name>` resolves by name across engines, precedence project > user > bundled, then the order in `[ui].workflow_engines` (default `["rhai", "pi"]`). Same name in two engines: `/workflows` lists both with an engine badge and `/workflow rhai:<name>` / `pi:<name>` disambiguates; bare name runs the first by precedence and the row says which engine ran.
- **Chat surface** (`scrollback/blocks/workflow.rs`, `views/workflows.rs`): states: no workflows found (empty state with the project/user/bundled paths searched); launching; running with `Phase N/M <name>`; log lines appended live (ring of 200); agent rows with state and tokens; budget `used/budget` (hidden when none); paused with `pause_message`; completed with `result_summary`; failed (error line); cancelled; engine unavailable (badge dim, launch refused with reason); event channel closed mid-run (state from `runs()`); two runs at once. Toast on completed/failed/cancelled. Traces shows the same run via T10, with the `engine` in the run header row.
- **Accept:** PC-W1 to PC-W5. **Tests:** engine-registry unit tests (precedence, ambiguity, prefixing); a `MockEngine` driving `WorkflowEvent`s into `WorkflowRunState` (order, revision dedup, every status); forwarder test (messages pushed to `gateway_rx` arrive in order at the client channel; with the forwarder dropped the poll still yields the final state); schema-contract tests ported; pager reducer tests per state; pty `host_pi_workflow_live.rs` with the fake Pi.

### B4: subagent and bridge execution (b)

- **Files:** `extensions/pi-grok-subagents/{runtime (771), bridge (431), v2 (590), tools-v1 (276), definitions (205), teams, config-ui (355), shared, index}.ts` + `skills/`, `teams/`; `extensions/pi-grok-workflows/index.ts` (379, the SpawnAgent executor); adapter `subagent_projection.rs` (562), `subagent_transport.rs` (250, Unix socket events; Windows named pipe compiled, untested), `pi_workflow_backend.rs` (241); `bin/grok_pi/subagent_extension.rs`; pager `app/subagent.rs` (both trees), `views/subagent_catalog_pane.rs`.
- **Facts to fix (code):** (1) capability modes `read-only | read-write | execute | all` map to tool lists in `CAPABILITY_TOOLS` (workflows `index.ts:30-35`, subagents `shared.ts:24`); the Rust bridge collapses any requested mode to `"all"` (`pi_workflow_backend.rs:100`). (2) Subagents create the child with `SessionManager.create(ctx.cwd, join(dirname(parentSessionFile), "subagent"))` (`runtime.ts:419`); the workflow executor uses `SessionManager.create(ctx.cwd)` (`pi-grok-workflows/index.ts:187`), so two code paths isolate sessions differently. (3) `isolation_worktree` is declared on the spawn request (`index.ts:46`) and sent by Rust as a bool (`pi_workflow_backend.rs:111`) but not read anywhere in the TS executor, while grok-build's native path maps it (`host_service.rs:501`).
- **Behaviour:** one shared spawn kernel in `pi-grok-subagents` used by both the subagent tools and the workflow executor; `capability_mode` passed through as the real enum value; child session files always under the parent's `subagent/` directory; `isolation = worktree` creates a worktree for the child under the session directory, runs the child with that cwd, and removes it on completion unless it has changes (then keeps it and reports the path); if worktree creation fails the spawn fails with the error (never runs in the parent tree). `fork_context` forks the parent's context into the child. Cancel propagates to children and their process trees.
- **UI states:** child queued; running with live tool rows (via the transport); finished; failed (error line); cancelled; capability denial (a denied tool call shows `denied by read-only mode`, not a generic failure); worktree created/kept/removed note; transport socket unavailable (rows fall back to final result only, one notice); too many children (cap from `config-ui.ts`, queued state); catalog pane empty / populated (`subagent_catalog_pane.rs`).
- **Accept:** PC-S1 to PC-S4. **Tests:** TS: port `bridge.test.ts`, `runtime.test.ts`, `v2.test.ts` and add capability-denial and worktree cases; Rust: transport bind/accept/close in a tempdir, projection fixtures, mapping test that each `SubagentCapabilityMode` value reaches the request file unchanged; pty `host_pi_subagent.rs`.

### B5: Bash and Eval execution kernels (c)

- **Files:** `extensions/pi-grok-bash/{index (732), bash-tasks (459), eval (1271, `PersistentEvalKernel` at :967), eval-tasks (273), tool-bridge (441), prompts, shared}.ts`; optional resident MCP daemon `mcpeval-*.ts` + `eval-pi-mcp.ts` (B5b); adapter `background_bash_bridge.rs` (629), the `x.ai/terminal/background` and `x.ai/task/kill` handlers in `pi_adapter/agent.rs`; `bin/grok_pi/{bash_extension,tools_extension}.rs`; pager `scrollback/blocks/tool/{eval,codemode}.rs`, `slash/commands/eval_display.rs`, `views/tasks_pane.rs` (overlap file: take B3's merge first).
- **Behaviour:** managed Bash: foreground by default, auto-promoted to a background task after a timeout (`bash-tasks.ts:359`), user-promotable (`x.ai/terminal/background`), killable by process tree (`x.ai/task/kill`), output truncated with a marker, status pushed to the task pane. Eval v1 (compat) and v2 (default): Node and Python workers, top-level await, per-cell scope reset with explicit `store()`/`load()`, `parallel()`/`pipeline()`/`agent()`, calls into active Pi tools through `tool-bridge.ts`, background jobs via `eval-tasks.ts` (`promoteEvalTask`, `armEvalAutoBackground`, `killEvalTask`). Kernel lifetime: one `PersistentEvalKernel` per session, shut down on session end.
- **UI states:** bash running foreground; promoted to background (task pane row); background finished/failed/killed; output truncated; eval cell running / done with value / done with images (rendered or placeholder) / error with traceback / timed out / aborted; eval background job; kernel crashed (cell fails, next cell restarts it, one notice); worker missing (python not found: eval v2 Python disabled with reason, JS still works); eval disabled by F2 (tool absent, no empty card).
- **Accept:** PC-E1 to PC-E3. **Tests:** run the existing node suites `test-v2.1.mjs` (runtime), `test-eval-pi-mcp.mjs`; Rust: `background_bash_bridge` unit tests; pager block render snapshot tests for `eval.rs` and `codemode.rs` states above; pty `host_pi_bash_background.rs` (fake Pi emits a long bash call, test sends the promote action, asserts a task-pane row, kills it).
- **B5b (after B5, required):** resident MCP eval daemon (`mcpeval-daemon.ts` 536, `mcpeval-oauth.ts` 430, `eval-pi-mcp.ts`). Loopback only, key + OAuth. **Accept:** `test-mcpeval-daemon.mjs`, `test-mcpeval-oauth.mjs`, `test-eval-pi-mcp.mjs` green in grok-build (they are red-cannot-run in GP; B0 fixes their imports).

### B6: F2 toggles, settings catalog, resource policy (d)

- **Files:** shell `host_features.rs` (524: `HostFeatureSpec`, `HostFeatureManifest`, `HostFeatureCategory`, `resolve_enabled`, `startup_env_override`); pager `settings/registry.rs` (the manifest seam at ~411 and the `pi_builtin_tools.*` entries at ~559), `native_feature_conflicts.rs` + `assets/native_feature_conflicts.toml`, `pi_resource_policy.rs`, `pi_resource_config.rs`, `pi_model_config.rs`, `views/pi_settings/`, `views/pi_config.rs`, `views/pi_models.rs`, `views/shortcut_manager.rs`, `app/extension_shortcuts.rs`, `slash/commands/{pi_config,pi_models,pi_shortcut_manager,scoped_models,hotkeys,reload}.rs`; `xai-grok-pager-bin/build.rs` (`generate_host_ui_catalog`: scans `extensions/*/grok-pi.json` into `BUNDLED_HOST_UI_SOURCES`); 8 manifests declaring 10 keys (`pi_ask_user_question`, `pi_btw`, `pi_goal`, `pi_herdr`, `pi_loop`, `pi_subagents`, `pi_subagents_v2`, `pi_todo`, `pi_todo_v2`, `pi_workflows`); `extensions/pi-grok-web-config`, `extensions/pi-grok-shortcut-manager`.
- **Behaviour:** each manifest declares `settings` (key, label, description, keywords, `f2: {category, section, order}`, kind, default, `restartRequired`, `configPath` e.g. `["ui","pi_workflows"]`, `nativeFeatureKey`, `startupEnv`) and `palette` commands. The settings registry merges manifest entries with built-in ones; enabled state resolves from the config TOML then `startupEnv`. The conflicts file lists npm packages blocked only while a native feature is on (e.g. `pi_workflows` blocks `npm:@quintinshaw/pi-dynamic-workflows`); runtime overlays `$GROK_HOME/native-feature-conflicts.toml` then `$GROK_PROJECT_DIR/native-feature-conflicts.toml` merge (package union, later reason wins). Pi package admission (`pi_resource_policy.rs`) applies it to user-installed packages.
- **Adds for B3:** `[ui].workflow_engines`, and the conflicts entry that disables a third-party `workflow` tool while the native one is on (exact package ids from B3 Step 0).
- **UI states:** setting on/off; restart-required marker after change; setting forced by env (shown locked with the env name); package blocked by policy (listed with the feature and reason, one key to see why); overlay file invalid TOML (error row, bundled defaults still used); manifest invalid (that extension's settings omitted with one diagnostic, others load); search by keyword; palette command present only while its feature is on; Pi not installed (settings still editable, Pi-only sections dimmed).
- **Accept:** PC-F1 to PC-F4. **Tests:** manifest parsing (good, missing field, wrong kind, duplicate key); `resolve_enabled` precedence (default < TOML < env); conflicts merge (bundled + two overlays, invalid overlay); registry golden listing of keys/sections/order; pty `host_pi_settings.rs` toggling `pi_workflows` and asserting the restart marker and the TOML value.

### B7: plan, goal, loop and the small bridges

- **Files:** adapter `plan_mode.rs` (545), `goal_host.rs` (330), `loop_host.rs` (156), `queue_bridge.rs` (644), `pi_adapter/queue_runtime.rs` (339), `btw_bridge.rs` (332), `todo_bridge.rs` (304), `recap_bridge.rs` (219), `prompt_bridge.rs` (199); matching `bin/grok_pi/*_extension.rs` and `extensions/pi-grok-{plan-mode,goal,loop,btw,todo,recap,rollback,ask-user-question}`; pager `slash/commands/plan_mode.rs`, `views/session_tree.rs`, `views/tree_map.rs`, `app/dispatch/session/{pi_fork,tree}.rs`.
- **Split into four parallel packets** sharing only `lib.rs` module lines: (a) plan/goal/loop; (b) queue/btw/todo/recap/prompt; (c) rollback/tree/fork; (d) every remaining GP extension and view so nothing Pi ships is left out: `extensions/pi-grok-{herdr,auth,context,export,native-commands,remote-tui,rpc-compat,rust-tui-bridge,00-profiler}` with their `bin/grok_pi/*_extension.rs`, and pager `views/{review,cache_graph}.rs`, `slash/commands/{review,clone_cmd,notify}.rs`. B0's seam list names any of these that need a shell/pager seam.
- **UI states:** per feature: off (command absent), on idle, active, completed, failed/cancelled, F2 toggled mid-session (applies next session, marker shown).
- **Accept:** each feature's handtest recorded in its PR plus PC6. **Tests:** ported unit tests per file; one pty per packet.

### B8: default flip (after PC1-PC6 and the PC-* for every slice shipped)

- **Behaviour:** `grok pi` starts the native host; `grok pi --acp` (and `GROK_PI_BACKEND=acp`) forces `pi-acp`; if the native host fails to start the session falls back to `pi-acp` with a one-line notice. The `pi=pi-acp` agent choice stays selectable. Docs: `04-slash-commands.md` and `docs/acp-matrix.md` updated.
- **Accept:** a clean `grok pi` on mini opens native; `--acp` opens `pi-acp`; PC6. **Tests:** launcher unit tests for the three paths; pty `host_pi_default.rs`.

### B9: Codex (note)

Today: `codex-acp` via `--agent-cmd` (`scripts/acp-matrix.sh`); session import and `/resume-codex <id>` via `xai-grok-foreign-sessions` (`P/app/foreign_sessions.rs:252`); traces reads `~/.codex/{sessions,archived_sessions}` rollouts including zstd. An agent wrote in `docs/acp-matrix.md` that the last Codex smoke failed (`gpt-5.6-sol` needs a newer Codex, no tool card); re-run `scripts/acp-matrix.sh codex` on a current Codex before assuming a gap. Traces-side Codex work is T1 (`justification`) and T6 (`apply_patch`). No native Codex host is planned.

# Conflict map and parallelism (both tracks)

Shared hot spots: `MODAL` (T0 shrinks it), `tests/pty_e2e_config_ui.rs` (one `#[path]` + `mod` pair per new pty test), the traces schema bump (T1 owns, T6 joins, T10 adds no column), and on Track B the overlap files in B.1.

```
Track A:  T0 (serial, first)
           ├─ crate-only, parallel:      T1   T6   T9
           ├─ module-owned, after T0:    T2   T8      (T4 after T2's first commit)
           ├─ core of MODAL, serial:     T3 (after T2) -> T5 (after T3, T1)
           └─ after T5 + T6:             T7           T10 (after T1, T6; verb row after T5)
Track B:  B0 (serial, first) -> B1 -> { B2 ‖ B3 ‖ B4 ‖ B5 -> B5b ‖ B6 ‖ B7a ‖ B7b ‖ B7c ‖ B7d } -> B8
          B3 needs B4's executor interface (agree the `WorkflowAgentBackend` signature in B0; B3 uses MockEngine until B4 merges)
          B5 merges `views/tasks_pane.rs` after B3 (overlap file)
          B2 merges `acp/tracker.rs` before B3's last step
```

Rules:
- The tracks never touch each other's files. T10 reads the state files B3 writes (`workflows/<run_id>/state.json`) but depends on the file format only, which exists today.
- One owner per new file: `traces_body.rs` = T2 then T7; `traces_payload.rs` = T4; `traces_rows.rs` = T5 then T10's verb line; `traces_hints.rs` = T0 creates, T8 owns, others add one line in their own PR; `engine.rs` = B3; `host_features.rs` = B6.
- Overlap files in Track B are merged in the order in B.1, by one agent at a time; a slice that needs an overlap file waits for the slice before it in that order.
- Dispatch: one packet per slice id, each in its own worktree off current main (Cursor agents or Pi workflow workers). A packet contains only its slice section, the Constraints, and Architecture rules. The executing agent rebases, reruns its Verify commands, and ends every commit with its `Co-authored-by` trailer.

Full verification (mini only):
```
cargo test -p xai-grok-traces
RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib
cargo build -p xai-grok-pager-bin && cargo build -p xai-grok-traces --bin grok-traces
cargo test -p xai-grok-pager-pty-harness --test pty_e2e_config_ui
bash crates/codegen/xai-grok-traces/hooks/test.sh
cargo test -p pi-grok-adapter
node extensions/pi-grok-bash/test-v2.1.mjs
scripts/acp-matrix.sh pi && scripts/acp-matrix.sh pi-native
```
First pty launch of a fresh binary can time out on welcome (an agent wrote this in MAP.md; rerun before debugging).

# Done means

Track A: open `/traces` on the screenshot's Pi session: rows read as intents, `v` flips to raw, `l` keeps the row in place and shows the script as code, Enter shows the whole script and result with `m` for metadata, edits show `+N/-M` and expand to a highlighted diff, no hint lies, `r` on Pi copies a working command, and an opaque Bash call gets denied once with a reason. Track B: `grok pi` opens a native Pi with workflows (live phase and log rows), subagents with real capability modes, background Bash, Eval and the MCP eval daemon, the F2 feature toggles, and every other extension and view grok-pi ships (B7a-d), and `pi-acp` is the fallback. Verify each on the real screen, not on tests alone.
