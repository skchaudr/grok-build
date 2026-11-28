"""Reviewable ACPX task packets, including the reviewed opening screen.

This data-only task file uses the installed `slices acp` runner. Code workers
return exact old/new edit lists; design workers return Markdown artifacts.
Loading this file runs no sessions and applies no edits. Review the rendered
packets before dispatch and the staged edit lists before integration. The
coordinator matches each OLD block exactly once against the pinned base, applies
the edits, lets Git produce the diff, and owns integration and the runnable UI
checks.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPOS = {"grok": str(ROOT), "piw": str(Path.home() / "repos/pi-workflows")}
MODAL = "crates/codegen/xai-grok-pager/src/views/acpx_modal"
ACPX = "crates/codegen/xai-grok-pager/src/acpx"
APP_MODAL = f"{MODAL}.rs"
CHAT = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs"
CHAT_DRIVER = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs"
GATE = "crates/codegen/xai-grok-pager/src/piw/gate.rs"
ARTIFACTS = "task-scripts/acpx-disclosure-returns"

SYSTEM = """Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, or release behavior. Change ledger routing only
where the task explicitly owns a reviewed presentation or focus change."""

EDIT_FORMAT = """
Edit list format. The file is Markdown. List the edits in application order;
each edit has exactly this shape:

## Edit <N>: <short purpose>
Path: <repository-relative path>
OLD:
~~~rust
<existing lines copied verbatim from one attached excerpt>
~~~
NEW:
~~~rust
<complete replacement>
~~~

Rules:
- Copy OLD character for character from an attached excerpt, including
  indentation, blank lines and comments. Never retype, reformat, abbreviate or
  elide it. OLD must occur exactly once in the file: include enough lines to be
  unique, normally the whole function, impl item, struct or test.
- Every OLD quotes the original attached source, never the result of an
  earlier edit. No two OLD blocks overlap.
- To insert new code (a field, function or test), quote a nearby existing
  anchor as OLD and give the anchor plus the new code as NEW.
- NEW is complete code with no "...", "unchanged" or elided bodies.
- Prefer replacing whole functions and whole test functions to small fragments.
- Inside the file use only ~~~ fences, never triple backticks.
- Never edit code outside the attached excerpts. If the change needs that, add
  a final "## Coordinator follow-ups" section naming each location and reason.

The coordinator matches each OLD exactly once on the pinned base and stops on
zero or several matches, then applies the edits and runs format, compile and
the targeted tests.
"""


def source(path, *ranges, repo="grok"):
    return [f"{repo}:{path}#L{start}-{end}" for start, end in ranges]


def task(slice_id, context, spec, *, design=False):
    if design:
        extension, shape, tail = "md", "<complete Markdown artifact>", ""
    else:
        extension, shape, tail = "edits.md", "<complete Markdown edit list>", EDIT_FORMAT
    artifact = f"{ARTIFACTS}/{slice_id}.{extension}"
    return {
        "custom_id": slice_id,
        "context_files": context,
        "output_files": [f"grok:{artifact}"],
        "prompt": f"Slice ID: {slice_id}\n\n{spec.strip()}\n{tail}\n"
        f"Return exactly:\n=== FILE: {artifact} ===\n```markdown\n{shape}\n```",
    }


OPENING_PREVIEW_INTERFACE = """
Shared ledger/preview contract (ledger owns the definitions, preview consumes):
- LedgerState::selected_waiting_group(&self) -> Option<(&str, usize)> returns
  the selected waiting group's display label and number of members, only on
  Flows. On that group selected_run() returns None; on a finished group it
  still returns the representative. The group itself has no gate or run ID.
- LedgerState::selected_run_is_finished_group(&self) -> bool is true only for
  a selected finished group representative, not an ordinary run or child.
- LedgerState::set_inline_preview(&mut self, lines: Option<Vec<InlinePreviewLine>>)
  sets the selected row's inline preview for the next render. In ledger.rs,
  pub(super) struct InlinePreviewLine has pub(super) fields text: String,
  cursor: bool, selected: bool. Each item is one logical line. None preserves
  the existing ledger continuation behavior; Some, including Some(empty),
  replaces default question/path continuations so they appear at most once.
  The ledger wraps for display and scrolls the focused cursor into view; the
  modal owns unwrapped copy text and never copies display indentation or wraps.
  Only the modal supplies lines, per render, in the narrow Flows view. No new
  I/O in the ledger, no new dependencies, and no new gate authority.
"""

TASKS = [
    task(
        "executed-timeline",
        source(f"{MODAL}/run.rs", (1, 305), (400, 539), (700, 1075), (2025, 2757))
        + source(f"{MODAL}/modal_view.rs", (153, 225))
        + source(f"{ACPX}/bundle.rs", (1, 350))
        + source(f"{ACPX}/step_digest.rs", (307, 351))
        + source("crates/codegen/xai-grok-pager/src/views/traces_fold.rs", (1, 162)),
        rf"""
Ownership: only {MODAL}/run.rs, including its tests. Work with the existing
RunViewState, FoldTree and FoldState; don't add another generic fold model.

Goal: on entry the stage pane reads as the executed timeline. Finished
attempt roots start collapsed with their result and trace children one key
away. The active stage and the genuine pending gate are outside that default;
the waiting choices stay discoverable and the initial selection still lands
on the waiting stage. A waiting stage whose gate has been answered
(set_answered) keeps its current expanded display: answering does not collapse
it and does not count as a newly finished attempt. Preserve the existing core
key actions and any user choice to expand an attempt across refresh and noise
toggles. Apply a default once per newly observed finished attempt, not on every
rebuild. If the current live attempt has a recorded step, distinguish it from
earlier finished loop attempts by position and the step timestamps. Do not
derive the run's terminal status from an attempt's completion or change
existing answer handling.

Keep one executed root per step in run.steps order, including repeated node
IDs. Preserve stable root keys and selected detail targets. Use
prune_run_for_graph from the existing modal_view module as the source of
eligible flow nodes for the skipped section; its branch rule is
active_branch_nodes in step_digest, attached. Within that already-pruned set
count or list only NotRun nodes. Preserve the current treatment of live
nodes, incomplete flow information and noise unless required by the change.

Tests: update the existing expanded-default assertion, then prove that
Space or Right reveals evidence, the noise toggle keeps the expansion,
initial selection and the pending choice remain accessible, and distinct
attempts of one node stay in order, e.g. build -> repair -> build. Cover a
Running run whose current node already has an earlier finished step (build ->
repair -> build with the second build live): the live attempt reads as its own
stage after repair. Cover switch pruning with a fixture that follows
active_branch_nodes, so an untaken branch does not appear under Skipped. Cover
preservation of manual expansion after sync, and an answered gate whose stage
still shows its choices after set_answered. Update other affected tests in the
attached test module that relied on implicit expansion by expanding explicitly
before navigation. Do not rewrite the tree, input dispatcher or rendering code
wholesale.
""",
    ),
    task(
        "acpx-opening-ledger-delta",
        source(f"{MODAL}/ledger.rs", (1, 445), (446, 747), (748, 1177),
               (1178, 1412), (1413, 1637), (1638, 2057), (2367, 2514),
               (2515, 2785), (2986, 3238), (3239, 3498), (3499, 3848))
        + source(APP_MODAL, (311, 352), (695, 747), (1439, 1517))
        + source(f"{ACPX}/bundle.rs", (1, 135), (261, 350), (1347, 1355))
        + source("docs/acpx-modal-opening-screen.md", (1, 62)),
        rf"""
Opening-screen delta after the accepted run2 packet 2 (1f7fe9b1). The
attached source already has finished grouping, rail/count/path and list copy.
Preserve those implementation and test contracts. Do not regenerate them.
Own Flows row grouping, selection and progress in {MODAL}/ledger.rs and its
unit tests, including the narrow inline render hook below. In {APP_MODAL}
edit only the row_extra producer/cache and its tests. Leave modal preview
state, input and rendering to packet 4. Keep LedgerOutcome routing compatible.

The default Grouped order has Waiting for input, Running and Recent outcomes;
omit empty sections. Stalled Running remains Running with its stalled label.
Preserve the existing run_section/is_answered behavior for native ACPX: a
Waiting run whose continuation records an answer goes under Recent outcomes as
an individual answered: <choice> row. Exclude it from waiting groups, section
counts and the waiting header total. Do not change the saved status, fabricate
PIW sent/accepted status, or change gate control. Keep running runs individual and group repeated
Waiting runs separately from finished groups. Group waits with the same
nonempty flow_name and source root (the run directory's parent); when an
absolute recorded flow path is available, also key on it so same-name runs from
different projects stay separate. Add an optional PathBuf to RowExtra for the
recorded path, supplied via bundle::flow_path inside cached row_extra; never
require that a recorded file exists. Missing paths only group with other
missing paths from the same source root. Waiting groups and their members sort
by updated_ms newest first with a stable tie; age and build association do not
change priority or visibility. Leave empty workflow names as individual runs.
Keep section counts in units of runs. Fold repeated waits in the default
Grouped order. Recent and Name continue to let operators browse waits by those
orders without the waiting-group fold; preserve existing sort and active
filter controls.

Extend the existing Space group toggle to waiting groups. Keep existing
shortcut/help behavior until packet 4 routes preview focus; no changes to
Ledger-mode key dispatch or help in {APP_MODAL} in this packet.
Enter on a waiting group expands it without opening a member; Enter on a run
or a finished group opens the same run as before. Expanded groups expose every
member. On a waiting group header the list y emits no copy outcome, because
there is no selected run ID. When search matches an older ID, reveal it even
if previously folded. Refresh keeps selection by run ID even if a newer
member arrives. On explicit collapse of a selected child, select the visible
group row or finished representative as appropriate. Leave the Agent Turns tab
(LedgerTab::Sessions; its label now reads Agent Turns) behavior alone.

Implement the shared inline interface using the existing continuation routing
in build_flow_rows, append_continuations and continuation_lines_for and the
existing wrapping and scroll paths in render_flows and render_flow_row. Some
replaces the selected run's existing question/path presentation, including
its first question fragment or inline row path when duplicated by the supplied
preview. None leaves packet 2's row and continuation rendering unchanged.
Something the modal supplies for the selected waiting group must also render
below its group row, without resolving a member. Give focused preview lines a
visible cursor/range style and include wrapping in height, scroll and overflow.
Keep the selected row reachable even if the preview is taller than the viewport.
Keep recorded rail, count, path and concrete-run y untouched. Update RowExtra
literals inside attached excerpts for the new optional path; name any remaining
literal as a coordinator follow-up. Do not add render I/O or widen cache limits.

Tests: keep packet 2's finished grouping, filter, recent/name sorts, retry,
unknown/empty, width and copy assertions. Add five ship waits and two mixed-talk
waits as two groups; source/project separation; one older wait still reachable;
Space/Enter expansion and no group y; individual run ID survives a refresh;
five unanswered native waits plus two with recorded continuation answers show a
waiting count and group size of five, with both answered rows under Recent
outcomes; stalled Running stays Running;
no closed or empty section placeholder; inline focused cursor visible with
wrapping and no duplicate question or path. Update existing assertions for the
changed section labels and classification only; leave gate evidence intact.
""" + OPENING_PREVIEW_INTERFACE,
    ),
    task(
        "attempt-detail",
        source(f"{MODAL}/detail.rs", (1, 243), (436, 533), (680, 1102),
               (1370, 1649), (1650, 2035), (2104, 2377), (2548, 2639))
        + source(f"{MODAL}/inspector.rs", (1, 219), (320, 358))
        + source(f"{MODAL}/run.rs", (552, 611))
        + source(APP_MODAL, (795, 858))
        + source(f"{ACPX}/bundle.rs", (1, 350))
        + source(f"{ACPX}/step_digest.rs", (1, 85), (153, 259), (353, 460))
        + source(f"{ACPX}/step_evidence.rs", (32, 337), (339, 505)),
        rf"""
Ownership: the selected-attempt view in {MODAL}/detail.rs, including tests.
Edit no other file. Preserve the existing run and node views, inspector, ledger
rows, run copy behavior, gates, entry routing, and the Flows preview call in
{APP_MODAL} (attached read-only to show how DetailPanel is called). Keep
DetailTarget's public variants unchanged so the timeline slice can proceed
without touching its match arms. Preserve the existing scroll and narrow-width
behavior. Detail-pane focus and linewise copy are a later packet built on this
one; don't add a cursor or selection here.

Goal: show a compact attempt digest first, with the full evidence available
through current inspector, timeline expansion, and conversation replay.
For a failed shell attempt put the relevant bounded failure tail before the
command and cwd. Reuse step_digest, including effective outcome for a failed
shell attempt. Show attempt number, elapsed/timeout when available, and a
recorded exit code when available. Use the step position already provided by
the caller and the existing render time passed into build_lines; no invented
timeout, exit code, verdict or scope count. Preserve
any step/receipt settings that provide the actual context for the attempt.

For a finished agent step show a verdict or validator evidence when present,
a file-scope summary only when grounded in existing evidence
(digest_step_files with tools from read_attempt_tool_calls, called only inside
the cached build_lines path), and a short human-readable note if available. No
raw wrapper JSON or repeated transcript in the first view. Include accurate,
conditional directions for the current Enter inspector and r replay. Existing
details for a genuinely waiting gate must remain reachable and its
question/choices must stay conspicuous; leave its current answer routing and
labels untouched.

The digest goes first; the existing conversation, artifact and event sections
may follow it. Do not delete or rename any function: callers outside the
excerpts use them (append_conversation, parse_conversation_items,
read_last_512k and append_value_lines live outside the excerpts;
value_plain_text is used by the inspector and value_inline_summary by run.rs).
Keep meaningful evidence for other kinds of attempts available. Preserve
narrow-width wrapping and cache behavior. Don't add extra reads on every render.

Tests: failed shell orders tail before command and cwd without saying success
on a nonzero exit; repeated attempt shows its own number and timing; agent
with brief evidence does not dump its prompt/output wrappers/transcript in the
first view, and agent without evidence doesn't invent it; a pending gate still
presents the question and choices; full evidence affordances remain accurate.
Adapt tests that explicitly asserted the old detail order; keep unrelated
formatter and navigation expectations intact.
""",
    ),
    task(
        "bare-command-picker",
        source(CHAT, (1, 121), (1125, 1257), (1258, 1355), (1375, 1393), (1825, 2023),
               (2104, 2252), (2293, 2350), (2375, 2478), (2531, 2597), (2598, 2783))
        + source(CHAT_DRIVER, (394, 530), (1499, 1527), (2168, 2219)),
        rf"""
Ownership: {CHAT} and {CHAT_DRIVER}, including their tests. Edit nothing else.

Goal: in Grok ACPX chat a bare /open or bare /flow shows a pick list instead
of the help text. Today parse_command returns Command::Help for both; the
attached parse_command_unknown_slash_and_acpx_help test asserts that and must
change for these two commands only.

Bare /open: show one question through the existing x.ai/ask_user_question
machinery (ask_question_raw, shaped like ask_gate_question, noFreeform true)
listing the 20 most recent saved runs from list_saved_runs(&runs_dir,
runtime::mirror_root().as_deref(), 20), the same list /runs shows, newest
first, native ACPX and mirrored PIW runs together. Option id and label are the
run ID. The description carries what the operator needs to choose: flow name,
title when it differs from the flow name, source (SavedRunSource::label),
status, and start time formatted with the existing helper. Choosing a run does
exactly what `/open <run_id>` does now, through resolve_saved_run and
replay_run_into_session with the resolved source, so a mirrored run stays
read-only and a native run keeps its gate restore; move the current Open arm
into one helper that both paths call, keeping its "Already shown above." and
not-found replies. Dismissing the question, an unknown choice, or a cancelled
session opens nothing and sends one short line. No runs: reply
"No acpx runs found." without a question. Opening stays inspection only.

Bare /flow: list the configured default_flow() first when it exists, then
the flowPath values recorded in the manifests of the 20 most recent runs from
both sources, runs_dir() and runtime::mirror_root(), newest first, each with
the flow name, source, last status and last start time of its most recent
run. /flow <file>.workflow.ts already routes to PIW (is_workflow_file,
run_piw_flow), so a mirrored run's workflow is a valid candidate. Add one
driver function next to list_runs that reads one runs directory's manifests,
reusing read_json_file, json_str and parse_ts, and call it for each source;
skip manifests without a flowPath. Resolve every candidate, the default flow
included, the way explicit /flow resolves its file (resolve_flow_path against
session_cwd); skip a candidate whose resolved path is not an existing file
(recorded paths under /private/tmp are often gone); deduplicate by resolved
path, keeping the newest run's description. Selecting a candidate does exactly
what typing `/flow <resolved path>` does today: it goes through the existing
Flow arm with input {{}} and starts a new run, the .workflow.ts route included.
Move that arm's body into one helper both paths call. Dismissing the question,
an unknown choice or a cancelled session starts nothing and sends one short
line. No candidates left after filtering: reply with the help text as today.
Mirrored /open stays read-only.

Keep `/open <id>`, `/flow <file> ...`, /runs, /answer (bare /answer still
Help) and plain-text handling unchanged. Update available_commands and
help_markdown so both commands say the argument is optional. Keep the
question's toolCallId distinct from gate tool call IDs.

Tests: keep the attached saved_runs_list_both_sources_and_only_native_replay_restores_a_gate
test passing unchanged. parse_command returns the new bare variants for "/open" and "/flow"
(and still Help for "/runs extra" and bare "/answer"); build the option lists
through pure functions you add and test them: newest-first order across both
sources, run ID as id and label, description content including the source, title omitted when equal to the flow name,
flow candidates from both sources, a relative flowPath resolved against the
session cwd, a missing file skipped (pass the file check in as a closure so the
pure function needs no real paths), two runs whose paths resolve to the same
file collapsed to the newest, default flow first and not repeated, manifests
without flowPath skipped (driver test with temp dirs like the attached
test_list_runs_order_and_title_fallback), selection mapped to the same
Command::Flow {{ file, input: {{}} }} that typing /flow <path> produces, and the
empty cases.
""",
    ),
    task(
        "acpx-opening-preview",
        source(APP_MODAL, (1, 310), (354, 396), (504, 558), (651, 694),
               (748, 927), (1004, 1164), (1193, 1249), (1275, 1296), (1415, 1437))
        + source(f"{MODAL}/ledger.rs", (18, 58), (233, 353), (955, 1125), (1178, 1215))
        + source(f"{MODAL}/detail.rs", (1369, 1434))
        + source(f"{ACPX}/bundle.rs", (1, 135), (150, 187), (237, 361), (617, 625), (1356, 1370))
        + source(f"{ACPX}/step_digest.rs", (157, 306))
        + source("docs/acpx-modal-opening-screen.md", (1, 62)),
        rf"""
This implements the reviewed opening-screen preview and replaces the old
packet 4 design assignment. This draft must be re-rendered after
acpx-opening-ledger-delta is integrated; the preview input must contain the
landed definitions of InlinePreviewLine and all three ledger methods in the
shared interface, with exact line ranges verified before dispatch. Own only
{APP_MODAL}: preview state, selected
preview evidence, Ledger-mode rendering and input, help and relevant tests.
Keep Flows as the initial view. Do not edit ledger.rs, detail.rs or Run/Step
behavior; the ledger supplies the shared interface below. No new landing mode.
Do not edit row_extra or its tests owned by packet 2; attach the new code to
anchors in the preview/input/render regions. The ledger packet adds no APP_MODAL
imports; it uses qualified paths for new helpers there. If a shared function is
edited, leave its non-Ledger arms intact. Packet 3b must render from the merged
base before editing shared input/render functions.

Replace the current ledger DetailPanel render with one compact, read-only
preview built from the existing selected-run bundle cache. Reuse recorded
status and fields; a waiting group shows its label, count and Space hint,
never a member's question/choices. A finished group previews the representative
and identifies it as such; obtain that indicator via the ledger interface.
An individual wait shows its saved question and choices as plain evidence when
present, with no pick shortcuts or answer action. A native ACPX run with a
recorded continuation answer shows that answer as history, not a request for
input. Leave PIW answer state to packet 5. Running shows the known current step.
Failed shows the failed attempt, available exit status and a short diagnostic
using existing step evidence; prefer a complete original diagnostic line to
a rewritten or summarized copy payload. Completed shows a short recorded
result, without raw structured wrappers; cancellations stay cancellations.
Use actual ordered Step attempts for the path and do not show unrun nodes.
State missing question or diagnostic plainly. Do not read artifacts on every
render, add new fetches, or widen bundle/cache limits. Keep preview identity
separate from DetailPanel; clear/clamp the cursor and range on selection,
refresh or mode changes so stale content cannot be copied.

At wide widths show the summary on the right; at narrow widths pass its
logical lines and selection to the ledger inline hook, with one visible copy
of the selected path (suppress standalone continuation when replaced). Use
the same logical lines for copy even when the screen wraps them. Tab/Shift+Tab
move between list and a nonempty preview with visible focus. When no preview
exists leave navigation usable. h/l or Left/Right from the list still switch
tabs. Agent Turns behavior stays as-is, including its existing Tab route.
Keep filter capture and help above preview input. When focused, j/k and
Up/Down move the line cursor, v starts/ends the range, y copies the exact
logical line/range, Y copies all preview lines, and Esc clears a selection
first then returns to the same list row. No render-only wraps, indentation,
ANSI or truncation in the copied payload. Use AcpxOutcome::Copy. List y keeps
packet 2 behavior. Route wants_esc accordingly, update displayed shortcuts
and help, and keep mouse wheel scrolling the pane under pointer without
mutating the unrelated run-detail panel. Enter from the list still opens the
selected run (after expanding a waiting group if necessary); Esc from Run
returns to the same row. No preview key can answer, launch, or cancel.

Tests: failed cargo_check with a multiline diagnostic, a waiting group and
individual question, running and finished summaries, unavailable evidence,
wide and narrow focus and wrapping, exact y/v/Y, Esc ordering, selection after
refresh/open/back, tab reachability and filter capture, empty/tiny layouts and
no workflow-control outcome from preview input. Coordinator validates the two
opening packets together and checks real clipboard paste and those views at
both widths before shipping; do not claim live verification.
""" + OPENING_PREVIEW_INTERFACE,
    ),
    task(
        "cross-surface-gate",
        source(GATE, (1, 373), (475, 813))
        + source(APP_MODAL, (397, 558), (1514, 1613))
        + source(f"{MODAL}/run.rs", (1143, 1195))
        + source(f"{MODAL}/detail.rs", (244, 271))
        + source(f"{MODAL}/ledger.rs", (310, 389))
        + source(CHAT, (680, 781), (896, 1050), (1125, 1218), (1258, 1355),
                 (1394, 1532), (1844, 1962), (2104, 2163), (2388, 2450))
        + source("tui/src/ui/run_command.rs", (1, 85), (177, 215), repo="piw")
        + source("tui/src/render.rs", (271, 416), repo="piw")
        + source("tui/src/protocol.rs", (1, 34), repo="piw"),
        """
Produce an implementation contract and operator walkthrough for the same
existing PIW run in Grok /acpx, standalone PIW, and Grok ACPX chat. No patch
in this packet: cross-repo ownership and the live fixture are review gates.
Treat the engine's request and run state as authoritative. Quote the exact
source functions that support each observed behavior; label anything the
excerpts cannot establish as a verification item. Do not assume standalone
PIW can submit a gate because the protocol lists an operation.

Provide a per-surface read/submit matrix and one transition table covering
pending, submitting, accepted while the engine is still waiting, next live
state, terminal, and stale/rejected. In the matrix keep what each client
actually does (its handlers and requests) separate from what protocol.rs
lists. Chat has more than one submit path: /answer <label>, and plain text
while a run is occupied (handle_occupied_run_input, answer_waiting_gate);
cover each, and the question path that reveals choices. Ground submission,
refusal and reconnection in answer_request, the AnswerMachine and its refusal
and connection-loss handling. Distinguish checkpoint input from a protected
decision and the PIW route from native ACPX replay. Current state: chat
/open <id> resolves native and mirrored runs (resolve_saved_run); a mirrored
replay shows the question and choices read-only and arms no answer
(saved_run_events, replay_run_into_session with SavedRunSource::PiwMirror), and
native replay still restores its gate. Live PIW answers after /open are this
packet's design question; note which current
functions would need changes and which repo owns them. Separate gate
inspection, an explicit answer, and native flow launch. Bare /answer may
reveal choices and must never submit, cancel or start a run. No speculative
cause for the earlier cancellation report. Indicate how to recover after an
uncertain submission by reconciling server state before any resend.

Specify a fresh parked fixture whose gate guards something the operator
would want to stop for: a build or test step attached to the next branch, or a
result worth checking before going on. A gate that guards neither is not worth
a fixture. Give it at least one meaningful next step and an observable
alternate branch. Give a walkthrough that inspects one run on all
three surfaces, compares state before and after inspection and bare /answer,
submits exactly one allowed answer through an authorized surface, and checks
that all three show the engine's branch and next step without reopening if
live watching is supported. Add a separate stale/rejected test. Include
exact expected evidence and mark any commands needing confirmation rather
than guessing. Do not answer a real protected decision or run a workflow.
""",
        design=True,
    ),
]
