#!/usr/bin/env python3
"""
acpx-repair.py: DRAFT repair slices for the acpx mode and /acpx modal defects in .handoffs (pi-workflows 001).

A repair slice differs from a build slice (acpx-mode.py, acpx-tui.py):
  - the model is given the LIVE file from this checkout, whole and uncut (several were fixed by hand after they
    came back, so the staged copy in *_out/ is not the code);
  - it is told the cause of the defect at file:line, not only what was seen on screen;
  - it returns ONE unified diff against that one file, not the file.

Nothing is applied to the source tree. Each firing gets a UTC attempt stamp S. Per slice it writes the raw return to
acpx-repair_out/_returns/<id>.S.md and the diff to acpx-repair_out/<id>.S.diff when it touches only its own file and
`git apply --check` accepts it, else to <id>.S.rejected.diff. A rejected diff never takes the accepted name. Every
outcome is appended to acpx-repair_out/attempts.log.

A slice cannot be fired while its "open" list is not empty or its prompt still holds an `<OPEN` marker: those are
choices Sab has not made yet. Settle a choice by writing it into the slice's "Behavior" text, removing the marker and
emptying "open". `ledger` and `modal` share one contract and fire only together.

  python3 task-scripts/acpx-repair.py --dry-run          slices, open choices, prompt sizes, estimate; no API call
  python3 task-scripts/acpx-repair.py --show ledger      print the exact prompt one slice would send
  python3 task-scripts/acpx-repair.py --only agent       fire exactly these slices; refuses if any of them cannot fire
  python3 task-scripts/acpx-repair.py                    fire every slice that can fire; report the others as skipped

Not slices (existing pager files, not slice output; edited by hand): the dock pinning steps (app/agent_view/panes.rs
`dock_subagent_rows`), the preview cap setting (scrollback/blocks/subagent.rs `PREVIEW_ROWS` + appearance/cache.rs), and
the key that opens the modal (actions/mod.rs, actions/defaults.rs, app/agent_view/input.rs).

Not covered by these slices (defects from the handoff list that stay after all six land):
  - Other raw-JSON sites: the Stages one-liners (detail.rs `step_one_line_summary`), the step pane's "Result"
    (detail.rs), the step inspector (inspector.rs) and the transcript block (scrollback/blocks/acpx_flow.rs).
  - An answered gate still looks live in the right pane (detail.rs shows its question and gate choices) and in the run
    view (banner and the 1-9 gate keys) unless run_view's answered-gate choice is "fix".
  - The graph in the run heading stays all "○" by design: slice `project` leaves it and draws a marked graph later.
"""

BIN = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp"
SRC = "crates/codegen/xai-grok-pager/src"

RULES = """
Hard rules: Rust 2024 edition, toolchain 1.94. No `unsafe`. No `unwrap()` / `expect()` / indexing that can panic on
anything read from disk, the network or the user. No new crates, no new `use` of a crate the file does not already use.
Change only what this task names. Do not reformat, rename, reorder or "tidy" any other line. Keep every existing test
passing unless the task says a test's expectation changes, and then change only that expectation.
New tests go in the file's existing `#[cfg(test)] mod tests`, built with the fixtures already there.

Return format: ONE ```diff block and nothing else. A unified diff against the file exactly as given above, with
`--- a/<path>` and `+++ b/<path>` headers using the repository-relative path, and 3 lines of unchanged context around
each hunk, copied character for character from the file. Only this one file. No omitted hunks, no "..." placeholders,
no prose before or after the block.
"""

# Shared by `ledger` (owns the types) and `modal` (fills them). Identical text goes to both.
ANSWERED_CONTRACT = """
// views/acpx_modal/ledger.rs  (slice `ledger` adds GateAnswer and the field; slice `modal` fills it)
/// The gate this run answered: read from this run's own input, written by `bundle::gate_input` when a gate answer
/// starts the next flow file. `projections/run.json` -> `input.gate = { "runId": <the waiting run>, "node": .., "choice": .. }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateAnswer {
    pub run_id: String,   // input.gate.runId: the run that was waiting at the gate
    pub choice: String,   // input.gate.choice: the label that was picked
}

#[derive(Debug, Clone, Default, PartialEq)]   // the existing derives, unchanged
pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
    pub answers: Option<GateAnswer>,   // NEW. None when this run's input has no `gate` object
}

Where a RowExtra is built: the struct literal in `row_extra` (views/acpx_modal.rs; slice `modal` adds `answers`) and
three struct literals in ledger.rs tests (slice `ledger` adds `answers: None`). Neither diff compiles without the
other; the coordinator applies both before building.

Why it lives on the answering run: acpx cannot resume a run that stopped at a gate, so the waiting run's folder never
changes again. `AcpxModalState::load_extras` (views/acpx_modal.rs) caches each RowExtra keyed on that run's own
`updated_ms`; a flag stored on the waiting run would never refresh. A run's input never changes, so `answers` on the
answering run is cache-safe.

Call order in the modal: `AcpxModalState::rescan_runs` calls `load_extras`, which calls `ledger.set_extras`, BEFORE it
calls `ledger.set_runs`. Inside `set_extras` the ledger's `runs` is still the previous snapshot (empty on first open).
"""

# Shared by `ledger` and `modal`: one product choice, each option touches at most one of the two files.
EXTRAS_OPEN = """
EXTRAS_LIMIT (slice `modal` handles this; slice `ledger` changes nothing for it): `load_extras` reads a RowExtra only
for the newest EXTRAS_LIMIT (80) runs by start time, so a gate whose answering run is outside the newest 80 would count
as "needs you" again. In slice `modal`: for every listed run outside the newest EXTRAS_LIMIT, put
`RowExtra { answers, ..RowExtra::default() }` into the extras handed to the ledger, with `answers` read from
`<run dir>/projections/run.json` alone (`std::fs::read_to_string` + `serde_json::from_str::<serde_json::Value>`, named
by path; `input.gate` read exactly as in `row_extra`), cached by run id for the life of the modal because a run's input
never changes. `rail` stays empty, `stalled` false, `question` None.
"""

TASKS = [
    {
        "custom_id": "project",
        "file": f"{BIN}/project.rs",
        "issue": "The mermaid graph in the run heading never marks finished steps.",
        "open": [
        ],
        "prompt": f"""
Repair `project.rs` of the binary grok-acpx-acp (it maps acpx run events to ACP session updates for the pager).

Defect seen on screen: the mermaid graph under the run heading shows every step as "○" for the whole run.

Cause: the graph is built once, in the `DriverEvent::RunStarted` arm of `on_event`, from `let states = HashMap::new();`
so every node gets the default glyph. `mermaid()` has no other caller outside the tests. The heading is an
`AgentMessageChunk`; the pager only appends to its transcript and cannot edit a message it already drew, so the heading
graph cannot be updated later. `self.plan_states` (`HashMap<String, PlanNodeState>`) already holds the live state of
every node (RunStarted sets every node to `Pending`), and `taken_path()` already maps states to glyphs.

Behavior:
- Leave the RunStarted heading as it is (all "○" is correct before anything has run).
- Add a private method that turns `self.plan_states` into the `HashMap<String, char>` that `mermaid()` takes, using
  the same glyph for each state that `taken_path()` uses: InProgress '◐', Completed '✓', Failed '✗'. A node that is
  `Pending`, or has no state, is left out (mermaid() draws it '○'). As the code stands, `taken_path()` uses '◐' for
  every InProgress node, running or waiting, while the doc comment on `mermaid()` says '●' running and '◐' waiting.
  Use `taken_path()`'s glyphs and leave that doc comment as it is.
- Only in the `Waiting` arm and the non-waiting `RunFinished` arm: prepend the ```mermaid fence built from that map
  to the text of the message that arm already sends (one message, not a new one), so the Out counts asserted by
  `checkpoint_waiting_shows_question_once_and_marks_plan_in_progress` (`outs.len() == 2`) and
  `run_finished_formats_completed_failed_and_waiting` (`ok_outs.len() == 1`) stay unchanged. Do not add a message
  after each step.
- The message goes through `self.parent_message(..)` like the heading does.
- Footer model: in the `StepStarted` arm for `StepKind::Acp`,
  when `self.acp_step_desc_and_model(node)` returns `Some(model)`, also push
  `Out::XaiUpdate {{ session_id: self.parent_session_id.clone(), update: json!({{"sessionUpdate": "served_model", "display_name": model}}) }}`
  as the last Out of that arm; in the `StepFinished` arm for `StepKind::Acp`, push the same update with
  `"display_name": "acpx"` as the last Out of that arm. Pager side (not in this file): it decodes the update as
  `XaiSessionUpdate::ServedModel {{ display_name }}` (enum tagged "sessionUpdate", snake_case variant names, field
  names not renamed) and stores it as the footer's served model name, which the footer shows in preference to the
  session model. It is cleared only when a new prompt turn starts, so without the "acpx" update the footer would keep
  the last step's model between steps. Slice `agent` must still advertise a session model, or the footer reads
  "unknown" before the first step. This changes one existing expectation: in
  `acp_step_order_spawned_before_child_updates`, `start_outs.len()` goes from 3 to 4 (sample_flow's `review-code`
  node has model "grok-4"); change only that number.
- Nothing else about the Plan update, `taken_path()` text, subagent blocks or tool calls changes.

Tests (add; keep `run_started_emits_mermaid_message_and_initial_plan` and
`mermaid_renders_glyphs_sanitized_ids_and_edge_labels` passing unchanged; every other existing test stays unchanged
except where an item above says so). Build them with `sample_flow()`, `chunk_text(..)` and `DriverEvent` literals like
the existing tests:
- after RunStarted and a successful StepFinished for node X (followed by
  `RunFinished {{ status: "completed".to_string(), error: None, at_ms: .. }}`), the marked graph that is emitted
  contains "✓ X" and still shows "○" for a node that has not started;
- a failed step's node carries '✗' in the marked graph (followed by a RunFinished with status "failed");
- the RunStarted heading still contains only "○" nodes;
- a StepStarted for an Acp node with a model emits a `served_model` XaiUpdate naming
  that model, and its StepFinished emits one with "acpx".
{RULES}""",
    },
    {
        "custom_id": "agent",
        "file": f"{BIN}/agent.rs",
        "issue": "The pager footer shows the model as \"unknown\".",
        # Settled for this file: A and C need the same code here (one entry, "acpx"). Whether a step's own model
        # replaces it while the step runs is still open, and is decided in slice `project` (footer model).
        "open": [],
        "prompt": f"""
Repair `agent.rs` of the binary grok-acpx-acp (the ACP agent the pager talks to).

Defect seen on screen: the pager footer shows the model as "unknown".

Cause: the pager prints "unknown" when the session has no current model. It takes the current model from
`NewSessionResponse.models` / `LoadSessionResponse.models`. `new_session` returns `acp::NewSessionResponse::new(session_id)`
and `load_session` returns `acp::LoadSessionResponse::new()`, both without `models`, so the pager never gets one.
(The `"unknown"` string in driver.rs is a run status and is not involved.)

acp types to use (agent_client_protocol, already imported as `acp`). This workspace resolves agent-client-protocol
0.10.4 with agent-client-protocol-schema 0.11.4. These types and the `.models(..)` builders exist only with the
schema's `unstable_session_model` feature; the workspace enables it (root Cargo.toml:
`agent-client-protocol = {{ version = "0.10.4", features = ["unstable"] }}`), so use them with no `cfg`:
    acp::SessionModelState::new(current_model_id: impl Into<acp::ModelId>, available_models: Vec<acp::ModelInfo>)
    acp::ModelInfo::new(model_id: impl Into<acp::ModelId>, name: impl Into<String>)
    acp::NewSessionResponse::new(id).models(state)      acp::LoadSessionResponse::new().models(state)
    acp::ModelId implements From<&'static str>, From<String> and From<Arc<str>>.
The pager drops a current model that is not in `available_models`, so the list must contain the current id.

Behavior:
- Add one private function that builds the `acp::SessionModelState` for a session: one entry, id "acpx", name "acpx",
  which is also the current model. (No flow is chosen when a session is created; the flow arrives later with `/flow`,
  so there is no flow model to show here.)
- `new_session` and `load_session` both attach it with `.models(..)`. Nothing else in either function changes.

Tests (add):
- the new function returns a `current_model_id` that is not empty and is present in `available_models`. Test it
  directly; this is the required test.
- Optional, only if you also call `new_session` / `load_session` (load with an unknown session id): `new_session`
  calls `tokio::task::spawn_local`, which panics outside a `LocalSet`, and every existing test here is a plain sync
  `#[test]`. Use `#[tokio::test(flavor = "current_thread")]` with the body inside
  `tokio::task::LocalSet::new().run_until(async {{ .. }}).await`. Build the agent with
  `AcpxAgent::new(AgentConfig {{ driver: crate::driver::DriverConfig {{ acpx_bin: PathBuf::from("acpx"), runs_dir: <tempdir path>, cwd: <tempdir path>, extra_path: vec![], poll_ms: 100 }}, default_flow: None }}, tx)`
  where `(tx, _rx) = tokio::sync::mpsc::unbounded_channel()` (`DriverConfig` lives in driver.rs with exactly those five
  public fields; `tempfile::tempdir()` is available to tests). Bring the trait into scope with `use acp::Agent as _;`
  and build requests with `acp::NewSessionRequest::new(cwd)` and `acp::LoadSessionRequest::new(session_id, cwd)`.
{RULES}""",
    },
    {
        "custom_id": "ledger",
        "file": f"{SRC}/views/acpx_modal/ledger.rs",
        "issue": "\"N needs you\" counts gates that were already answered; the gate question is cut off in the run list.",
        "open": [
        ],
        "prompt": f"""
Repair `views/acpx_modal/ledger.rs` in crate xai-grok-pager (the run list on the left of the /acpx modal).

Defect 1 seen on screen: the summary says "N needs you" for gates the user already answered.

Cause: acpx cannot resume a run after a gate. Answering a gate starts the NEXT flow file as a new run, and the old run
stays `RunStatus::Waiting` forever. `build_summary_spans` counts every `status == RunStatus::Waiting`, `run_section`
puts every Waiting run under `SectionKind::NeedsYou`, and `is_run_active` and `flow_state_text` treat it the same way.

Shared contract (another slice fills `answers`; do not change these names):
{ANSWERED_CONTRACT}
Behavior for defect 1:
- Add `GateAnswer` and the `answers` field exactly as above. `RowExtra` already derives `Default`, so `Default` needs
  no change. Add `answers: None` to the three `RowExtra {{ .. }}` struct literals in this file's tests. The literal in
  views/acpx_modal.rs belongs to slice `modal`; this diff compiles only once that diff is applied too.
- Keep a private field `answered: HashMap<String, String>` (waiting run id -> choice). Rebuild it at the end of both
  `set_runs` and `set_extras` from every extra whose `answers` is `Some`. When two runs answered the same gate, the
  answering run with the larger `started_ms` in `self.runs` wins; if either answering run is missing from `self.runs`,
  the larger run id wins.
- Every `Some` answer counts, whatever the answering run's status: a gate stays answered even if the run that
  answered it later failed.
- A run is "answered" when its status is Waiting and its id is a key of `answered`. An answered run: is not counted in
  "N needs you"; is placed under `SectionKind::Finished` by `run_section`; is not active for `is_run_active`; and
  `flow_state_text` returns `answered: <choice>` (just `answered` when the choice is empty) instead of the gate
  question. An answered run's row no longer looks like it needs attention: in `render_flow_row` its status glyph and
  colour are the ones a Completed run gets, and where its rail would draw the waiting ◐ it draws the Completed glyph in
  the Completed colour.
- Add `pub fn answered_choice(&self, run_id: &str) -> Option<&str>`: the choice for an answered run, `None` for any
  other run. Slice `modal` reads it to tell the run view (slice `run_view`) that its gate was answered.
- A Waiting run that is not answered behaves exactly as today.
{EXTRAS_OPEN}
Defect 2 seen on screen: a long gate question is cut off in a run's row.

Cause: in `render_flow_row` the state text is cut to one line by
`truncate_str(&raw_state_text, flex_budget - 1 - title_col)`, and dropped entirely when `flex_budget <= 17`.

Behavior for defect 2: when the row is selected and it is an unanswered Waiting run, the full question is shown
wrapped on continuation rows under the row, indented to the state column; unselected rows stay one line. The ledger
cannot tell whether the modal shows the right pane, so wrap whenever such a row is selected. This changes row heights,
so `render_flows` (keeping the selection visible, the maximum scroll) and `scroll` must count the new heights. There is
no click hit-testing in the ledger; the mouse only scrolls.

Tests (add; `render_at_120x14_shows_tabs_needs_you_question_rail_and_count` must still pass). Build them with this
file's fixtures (`make_run`, `LedgerState::new`, `set_runs`, `set_extras`, `render`, `all_text`) and give every Waiting
run in them its own `RowExtra`:
- two Waiting runs where another run's `answers.run_id` names the older one: the summary reads "1 needs you" and the
  older run is listed under Finished;
- a Waiting run with no answering run is still counted and still under Needs you;
- extras replaced by a set without the answering run: the gate counts as needing you again;
- `set_extras` called before `set_runs` (the modal's order) gives the same result as the reverse order;
- two runs answering the same gate: the one with the larger `started_ms` supplies the choice;
- `answered_choice` returns the choice for the answered run and `None` for an unanswered one;
- an answered run's rendered row contains `answered: <choice>` and not the gate question;
- a selected unanswered Waiting run whose question is 200 characters, rendered 100 wide and 20 high, shows every word
  of the question; the same run unselected takes one row.
{RULES}""",
    },
    {
        "custom_id": "modal",
        "file": f"{SRC}/views/acpx_modal.rs",
        "issue": "Supplies the data the ledger needs to tell an answered gate from a live one.",
        "open": [
        ],
        "prompt": f"""
Repair `views/acpx_modal.rs` in crate xai-grok-pager (the /acpx modal; it loads run folders and feeds the run list).

Defect seen on screen: the run list counts gates as "needs you" after the user answered them. The list cannot tell,
because nothing passes it the link between an answering run and the gate it answered.

Shared contract (slice `ledger` adds these to ledger.rs; write against them, do not redefine them here):
{ANSWERED_CONTRACT}
Cause in this file: `row_extra` builds a `RowExtra` from `bundle::load_run(&summary.dir)` and never reads `run.input`
(a `serde_json::Value` that `bundle::load_run` reads from `projections/run.json` -> `input`; `Value::Null` when absent).

Behavior:
- In `row_extra`, add `answers` to the `RowExtra {{ .. }}` literal: when `input.gate` is an object whose `runId` is a
  non-empty string, `Some(GateAnswer {{ run_id, choice }})` with `choice` from `input.gate.choice` (empty string when
  absent or not a string); otherwise `None`. Read with methods on the value (`.get(..)`, `.as_str()`); never panic on a
  malformed input. This file does not import serde_json; serde_json is a dependency of this crate, so name it by path
  (`serde_json::Value`) where you need the type, and add no new `use` of it.
- This diff compiles only once slice `ledger`'s diff (which adds `GateAnswer` and the field) is applied too.
- `load_extras` and its cache are unchanged except as the EXTRAS_LIMIT item below says: `answers` describes the run
  that owns the RowExtra, so the existing `updated_ms` cache key stays correct.
- Import `GateAnswer` from the same path `RowExtra` is already imported from.
{EXTRAS_OPEN}
- Tell the run view when its gate was answered. Slice `ledger` adds
  `pub fn answered_choice(&self, run_id: &str) -> Option<&str>` on the ledger state; slice `run_view` adds
  `pub fn set_answered(&mut self, choice: Option<String>)` on `RunViewState`. Call
  `view.set_answered(self.ledger.answered_choice(<that run's id>).map(str::to_string))` in `open_run` right after
  `RunViewState::new(run)`, and again for an open run view every time `rescan_runs` has refreshed the ledger. This
  diff compiles only once those two diffs are applied too.

Tests (add), built on the existing `write_fixture(..)` fixture, which writes no `projections/run.json`:
- after `write_fixture("completed")`, write
  `{{"input":{{"gate":{{"runId":"r1","node":"g","choice":"approve"}}}}}}` to
  `fx.run_dir.join("projections").join("run.json")` with `fs::write`, then call `row_extra` on the summary from
  `bundle::list_runs(&fx.runs_base).first()`; expect
  `answers == Some(GateAnswer {{ run_id: "r1".into(), choice: "approve".into() }})`;
- an input without `gate` yields `None`; an input where `gate` is a string yields `None`;
- a run outside the newest EXTRAS_LIMIT gets a RowExtra with its `answers` and an empty rail.
Write test JSON as raw string literals (no `serde_json::json!`).
{RULES}""",
    },
    {
        "custom_id": "detail",
        "file": f"{SRC}/views/acpx_modal/detail.rs",
        "issue": "The run pane shows the last result and the input as raw JSON.",
        "open": [
        ],
        "prompt": f"""
Repair `views/acpx_modal/detail.rs` in crate xai-grok-pager (the right-hand pane of the /acpx modal).

Defect seen on screen: under "Last result" and "Input" the run pane prints raw pretty-printed JSON.

Cause: in `build_run_lines` both sections call `value_text(&serde_json::Value)` and pass the text to
`push_wrapped_capped` (20 rows for the last result, 12 for the input). `value_text` is imported from
`crate::acpx::bundle` and is not defined in this file: `Value::Null` -> "", a string -> the string itself, anything
else -> `serde_json::to_string_pretty`.

Behavior:
- Add a private function that turns a `&serde_json::Value` into display lines for a given width:
  a string is wrapped as text; a number, bool or null is one line; an object is one row per
  top-level field as `key  value`, keys in the object's own order and padded to the longest key, a nested object or
  array indented two columns under its key; an array is one row per item prefixed "- "; no braces, brackets, quotes
  or trailing commas anywhere.
- "Last result" and "Input" in `build_run_lines` use it. These two sections are no longer capped: every
  line is shown (this pane already scrolls). Stop calling `push_wrapped_capped` for them; if that leaves it with no
  caller, remove it.
- A value that renders no lines (`null`, an empty object) shows no section and no rule. Today an empty object prints
  `{{}}` under its rule; that changes.
- The step pane and every other caller of `value_text` are unchanged. Copy actions still copy the raw JSON.
- Out of scope here: the Stages rows still print compact JSON through `step_one_line_summary`, and the step pane's
  "Result" still uses `value_text`.

Tests (add; `test_run_target_lists_stages_and_waiting_gate_choices` must still pass). Build them with
`make_test_run(..)`, a `Step {{ .. }}` literal like the ones in that test, and `DetailPanel::render` + `buffer_lines`
(or `build_run_lines` directly):
- a Running run whose last step's output is `{{"summary":"all good","files":["a.rs","b.rs"]}}` renders, between the
  `── Last result` rule and the next section rule, a row containing `summary` and `all good` and a row for each file,
  and no line in that range contains `{{`, `}}`, `[` or `"summary":` (the Stages rows above still print compact JSON;
  do not test them);
- a string output renders as wrapped text exactly as before;
- an empty object and `null` render no section at all;
- an output that is an array of 40 short strings yields all 40 rows and no "more rows" line (check `build_run_lines`,
  not the clipped buffer).
{RULES}""",
    },
    {
        "custom_id": "run_view",
        "file": f"{SRC}/views/acpx_modal/run.rs",
        "issue": "The gate question is cut off in the run view's banner.",
        "open": [
        ],
        "prompt": f"""
Repair `views/acpx_modal/run.rs` in crate xai-grok-pager (the single-run view of the /acpx modal).

Defect seen on screen: at a gate the banner reads "◐ NEEDS YOU at <node>: <question>" and a long question is cut off.

Cause: `build_banner` returns one `Line` cut by `truncate_str(&text, width)`, and the render code gives the banner
exactly one row.

Behavior:
- `build_banner` returns the banner as wrapped lines (`Vec<Line<'static>>`, empty when there is no banner), every line
  in the same colour and weight as today. run.rs has no wrap helper (the one in detail.rs is private to that module).
  Add a private `fn wrap_banner(text: &str, width: usize) -> Vec<String>` that breaks on spaces, hard-breaks words
  wider than `width`, and measures with `unicode_width` (already imported at the top of this file; add
  `UnicodeWidthChar` to that import). There is no row limit.
- The layout gives the banner as many rows as it returns and takes them from the area below it; with no banner the
  layout is exactly as today. A terminal too short for the banner plus one row of content shows the banner's first
  rows and keeps one content row.
- Stalled, failed and running banners go through the same wrapping.
- Answered gate. Add a private field `answered: Option<String>` (None by default),
  `pub fn set_answered(&mut self, choice: Option<String>)` and `pub fn answered(&self) -> Option<&str>`. Slice `modal`
  calls the setter (it reads the choice from the ledger); outside tests this file never calls it. While it is
  `Some(choice)` and the run is Waiting: the banner reads "✓ answered: <choice>" ("✓ answered" when the choice is
  empty) in the style this file uses for a completed run, in place of "◐ NEEDS YOU at <node>: <question>"; and
  `handle_key` no longer answers the gate: the 1-9 keys and Enter on a gate row return the same outcome as a key that
  does nothing. Everything else in the view is unchanged.

Tests (add; `render_wide_160x40_shows_banner_stages_separator_and_clean_detail_area` must still pass). Build them with
`sample_waiting_run(..)`, setting `summary.status_detail` before `RunViewState::new`, then `render` and `buf_text`:
- a Waiting run whose question is 120 characters, rendered 80 wide, shows every word of the question;
- a short question still takes exactly one row and the rows below it are where they are today;
- after `set_answered(Some("approve".to_string()))` on a Waiting run, the render contains "answered: approve" and no
  "NEEDS YOU", and `handle_key` for `1` returns the do-nothing outcome where it returned a gate answer before.
{RULES}""",
    },
]

# Slices that share a contract: each fires only together with its partner.
PAIRS = [("ledger", "modal"), ("ledger", "run_view"), ("modal", "run_view")]

# ═══════════════════════════════════════════════════════════════════════
# Engine. HTTP, retry, key and endpoint resolution, model and prices are reused from acpx-mode.py.
# ═══════════════════════════════════════════════════════════════════════
import argparse, importlib.util, re, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("acpx_mode", HERE / "acpx-mode.py")
engine = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(engine)

SYSTEM = (
    "You are a principal software engineer repairing one file of an existing Rust codebase. Make the smallest change "
    "that satisfies the task. Return exactly one unified diff in one ```diff block and nothing else."
)


def build_prompt(repo_root: Path, task: dict) -> str:
    """The whole live file, uncut, then the task. A missing file is an error, never a silent skip.
    Called on the main thread only, so its sys.exit ends the run before any request is sent."""
    path = repo_root / task["file"]
    if not path.is_file():
        sys.exit(f"{task['custom_id']}: target file not found: {path}")
    return (
        f"=== THE FILE AS IT IS NOW: `{task['file']}` ===\n\n```rust\n{path.read_text(encoding='utf-8')}\n```\n\n"
        f"=== TASK ===\n{task['prompt']}"
    )


def open_reasons(task: dict) -> list:
    """Why a slice cannot fire on its own merits: open choices listed, or an <OPEN marker left in the prompt."""
    reasons = []
    if task["open"]:
        reasons.append(f"{len(task['open'])} open choice(s)")
    if "<OPEN" in task["prompt"]:
        reasons.append("<OPEN> marker still in the prompt")
    return reasons


def blocked_slices(tasks: list) -> dict:
    """id -> reason, for every selected slice that cannot fire, including a pair partner left behind."""
    blocked = {t["custom_id"]: "; ".join(open_reasons(t)) for t in tasks if open_reasons(t)}
    selected = {t["custom_id"] for t in tasks}
    for a, b in PAIRS:
        for x, y in ((a, b), (b, a)):
            if x not in selected or x in blocked:
                continue
            if y not in selected:
                blocked[x] = f"fires only together with `{y}`, which is not selected"
            elif y in blocked:
                blocked[x] = f"fires only together with `{y}`, which is blocked"
    return blocked


def extract_diff(content: str) -> str:
    content = re.sub(r"^<think>[\s\S]*?</think>\s*", "", (content or "").replace("\r\n", "\n"))
    m = re.search(r"```(?:diff|patch)?[ \t]*\n([\s\S]*)\n```", content)
    body = m.group(1) if m else content
    return body.strip("\n") + "\n" if body.strip() else ""


def diff_paths(text: str) -> set:
    """Every path a diff names in a file header: `---`/`+++` header pairs, `diff --git`, rename/copy lines.
    a/ and b/ prefixes are stripped; /dev/null stays as is (a created or deleted file)."""
    def norm(p: str) -> str:
        p = p.split("\t")[0].strip()
        return p[2:] if p.startswith(("a/", "b/")) else p

    lines = text.split("\n")
    paths = set()
    for i, line in enumerate(lines):
        if line.startswith("--- ") and i + 1 < len(lines) and lines[i + 1].startswith("+++ "):
            paths.add(norm(line[4:]))
            paths.add(norm(lines[i + 1][4:]))
        elif line.startswith("diff --git "):
            paths.update(norm(p) for p in line[len("diff --git "):].split(" ") if p)
        else:
            m = re.match(r"^(?:rename|copy) (?:from|to) (.+)$", line)
            if m:
                paths.add(norm(m.group(1)))
    return paths


def check_diff(repo_root: Path, task: dict, diff_path: Path) -> str:
    """Empty string when the diff is acceptable, else the reason. Never changes the working tree."""
    touched = diff_paths(diff_path.read_text(encoding="utf-8"))
    if touched != {task["file"]}:
        return f"diff names {sorted(touched) or 'no file'}, expected only {task['file']}"
    res = subprocess.run(
        ["git", "apply", "--check", "--recount", str(diff_path)], cwd=repo_root, capture_output=True, text=True
    )
    return res.stderr.strip()[:400] if res.returncode else ""


def fire(api_key: str, endpoint: str, model: str, task: dict, prompt: str, repo_root: Path, out: Path, stamp: str) -> dict:
    cid = task["custom_id"]
    payload = {
        "model": model,
        "temperature": 0.0,
        "max_tokens": engine.MAX_TOKENS,
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": prompt}],
    }
    headers = {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}
    start = time.time()
    try:
        res = engine.make_http_request(endpoint, headers=headers, data=payload, method="POST")
        choices = res.get("choices", [])
        if not choices:
            return {"task": task, "ok": False, "error": "no response choices returned"}
        raw = choices[0].get("message", {}).get("content", "") or ""
        returns = out / "_returns" / f"{cid}.{stamp}.md"
        returns.parent.mkdir(parents=True, exist_ok=True)
        returns.write_text(raw, encoding="utf-8")
        finish = choices[0].get("finish_reason") or choices[0].get("native_finish_reason")
        if str(finish).lower() in ("length", "max_tokens"):
            return {"task": task, "ok": False, "error": f"TRUNCATED at max_tokens; partial return kept in {returns}"}
        diff = extract_diff(raw)
        if not diff:
            return {"task": task, "ok": False, "error": f"EMPTY response; raw return in {returns}"}
        pending = out / f"{cid}.{stamp}.pending.diff"
        pending.write_text(diff, encoding="utf-8")
        reason = check_diff(repo_root, task, pending)
        if reason:
            rejected = pending.rename(out / f"{cid}.{stamp}.rejected.diff")
            return {"task": task, "ok": False, "error": f"diff rejected ({rejected}): {reason}"}
        accepted = pending.rename(out / f"{cid}.{stamp}.diff")
        return {"task": task, "ok": True, "path": accepted, "elapsed": time.time() - start, "lines": diff.count("\n")}
    except Exception as e:
        return {"task": task, "ok": False, "error": str(e)}


def main():
    ids = ", ".join(t["custom_id"] for t in TASKS)
    parser = argparse.ArgumentParser(description="Repair slices: one live file in, one unified diff out")
    parser.add_argument("--key", default=None)
    parser.add_argument("--endpoint", default=None)
    parser.add_argument("--model", default=engine.MODEL, help=f"Model identifier (default: {engine.MODEL})")
    parser.add_argument("--repo", default=str(HERE.parent))
    parser.add_argument("--output", default=str(HERE / "acpx-repair_out"))
    parser.add_argument("--only", help=f"Comma-separated slice ids ({ids})")
    parser.add_argument("--dry-run", action="store_true", help="List slices, open choices and prompt sizes; call nothing")
    parser.add_argument("--show", metavar="ID", help="Print the exact prompt one slice would send; call nothing")
    args = parser.parse_args()
    repo_root = Path(args.repo).resolve()
    known = {t["custom_id"] for t in TASKS}

    if args.show:
        task = next((t for t in TASKS if t["custom_id"] == args.show), None)
        if not task:
            sys.exit(f"--show matched nothing; ids: {ids}")
        print(build_prompt(repo_root, task))
        return

    if args.only:
        wanted = [s.strip() for s in args.only.split(",") if s.strip()]
        unknown = [w for w in wanted if w not in known]
        if unknown or not wanted:
            sys.exit(f"--only: unknown id(s) {', '.join(unknown) or '(none given)'}; ids: {ids}")
        tasks = [t for t in TASKS if t["custom_id"] in wanted]
    else:
        tasks = list(TASKS)

    if args.dry_run:
        total = 0
        pair_of = {x: y for a, b in PAIRS for x, y in ((a, b), (b, a))}
        for t in tasks:
            n = len(build_prompt(repo_root, t))
            total += n
            state = f"{len(t['open'])} open" if t["open"] else "ready"
            pair = f"  (fires only with {pair_of[t['custom_id']]})" if t["custom_id"] in pair_of else ""
            print(f"  {t['custom_id']:<9} {state:<7} {n:>7,} chars in  {t['file']}{pair}\n            {t['issue']}")
            for q in t["open"]:
                print(f"            OPEN: {q}")
            if "<OPEN" in t["prompt"] and not t["open"]:
                print("            OPEN: <OPEN> marker left in the prompt text")
        est = total / 4 * engine.PRICE_IN_PER_M / 1e6 + len(tasks) * 3000 * engine.PRICE_OUT_PER_M / 1e6
        print(
            f"\n[dry run] {len(tasks)} slice(s), ~{total // 4:,} tokens in, ~$ {est:.3f} at acpx-mode.py PRICE_* rates "
            f"(assumes ~3k tokens out per diff; an estimate, not a ceiling). Model {args.model}. No API call made."
        )
        return

    blocked = blocked_slices(tasks)
    for cid, why in blocked.items():
        print(f"  [skip] {cid}: {why}")
    if args.only and blocked:
        sys.exit(f"Not fired: --only names slice(s) that cannot fire: {', '.join(blocked)}.")
    tasks = [t for t in tasks if t["custom_id"] not in blocked]
    if not tasks:
        sys.exit("Not fired: every slice is blocked. Run --dry-run to see the open choices.")

    prompts = {t["custom_id"]: build_prompt(repo_root, t) for t in tasks}
    key, endpoint = engine.resolve_api_config(args.key, args.endpoint)
    if not key:
        sys.exit("API key required via --key, CHEAPERINFERENCE_API_KEY, or OPENROUTER_API_KEY.")
    out = Path(args.output).resolve()
    out.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    print(f"Firing {len(tasks)} repair slice(s) on {args.model} ({endpoint}); attempt {stamp}; diffs -> {out}")
    failed = []
    log_lines = []
    with ThreadPoolExecutor(max_workers=len(tasks)) as pool:
        futures = [pool.submit(fire, key, endpoint, args.model, t, prompts[t["custom_id"]], repo_root, out, stamp) for t in tasks]
        for fut in as_completed(futures):
            r = fut.result()
            cid = r["task"]["custom_id"]
            if r["ok"]:
                print(f"  [ok]   {cid}: {r['path']} ({r['lines']} lines, {r['elapsed']:.0f}s), git apply --check passes")
                log_lines.append(f"{stamp}\t{cid}\tok\t{r['path']}")
            else:
                failed.append(cid)
                print(f"  [FAIL] {cid}: {r['error']}")
                log_lines.append(f"{stamp}\t{cid}\tfail\t{r['error']}")
    for cid, why in blocked.items():
        log_lines.append(f"{stamp}\t{cid}\tskipped\t{why}")
    with (out / "attempts.log").open("a", encoding="utf-8") as fh:
        fh.write("\n".join(log_lines) + "\n")
    if failed:
        sys.exit(f"{len(failed)} of {len(tasks)} failed: {', '.join(failed)}")


if __name__ == "__main__":
    main()
