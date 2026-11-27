"""Handoff 004: five reviewable ACPX task packets.

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
authority, workflow state, ledger routing, or release behavior."""

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
        "finished-run-ledger",
        source(f"{MODAL}/ledger.rs", (1, 315), (317, 528), (550, 940),
               (943, 1036), (1120, 1555), (1647, 1696), (2124, 2278),
               (2279, 2421), (2499, 2710), (2756, 2813))
        + source(APP_MODAL, (1, 92), (291, 348), (675, 721), (1382, 1460))
        + source(f"{ACPX}/bundle.rs", (1, 50), (150, 187), (261, 350)),
        rf"""
Own Flows row grouping, selection and progress in {MODAL}/ledger.rs and its
unit tests. In {APP_MODAL} edit only the row_extra producer/cache and its tests
if needed for executed counts. Do not change preview or modal route. Keep
LedgerState::selected_run() and LedgerOutcome routing compatible.

Group two or more actual finished runs with the same nonempty flow_name.
The newest member represents a collapsed group: the greatest started_ms, and on
a tie the greatest run_id (run IDs begin with a sortable timestamp). Keep
active/waiting runs individually selectable. Do not group any run still marked
Running or Waiting by its actual summary, even if the legacy section label puts
it in Finished (a stalled Running run, an answered Waiting run). Preserve legacy
gate classification for now; call no answer operation. Leave empty workflow
names as individual runs. Keep total counts in units of runs. Preserve sort and
active filters.

Space toggles the selected group open or closed in Flows; it is unbound in the
attached Flows dispatcher. Tab, BackTab, Left, Right, h and l keep switching
tabs. Add the Space label to LedgerState::shortcuts(); the coordinator adds the
help-table row in {APP_MODAL}. Space on a row that is not a group is Unchanged.
Enter still opens the selected run. Expanding reveals older member runs in
newest-first order by the same rule. When search matches an older ID, keep it
selectable even if previously folded. Refresh keeps selection by run ID, even
if a newer member arrives. On explicit collapse of a selected child, move
selection to the visible representative. Leave the Agent Turns tab
(LedgerTab::Sessions; its label now reads Agent Turns) behavior alone.

Replace the visible dot rail with '<N> steps ran' where N is the number of
recorded Step entries, including repeat attempts. RowExtra contract: add
`pub steps_ran: Option<usize>` and keep the existing `rail` field (literals
outside the excerpts still set it); None means unknown. row_extra fills
steps_ran from the run it already loads. Update every RowExtra literal inside
the attached excerpts, and update the attached probe test's rail assertions
(the 5/262 count and the check marks) to the new text. The coordinator adds
`steps_ran` to the RowExtra literals outside the excerpts. Reuse the parent's
cached bundle read; show unknown when extras are missing, never fake 0. Do not
add I/O to render or widen the cache/read limit. Keep existing styles. For a
concrete Flows selection, y copies the canonical '/open <run_id>' command;
on the Agent Turns tab y still copies the selected record ID. The modal's copy notice is fixed separately by the
coordinator; the coordinator must also verify mirrored /open works before
shipping this copy behavior.

Tests: three finished runs of one name plus a same-name pending run; folded
and expanded navigation with Space; newest tie by run_id; matching and
preserving an older ID on refresh; a stalled Running run and an answered
Waiting run of the grouped name staying individual; a group under the Recent
and Name orders; counts including repetitions vs missing extras; exact y
behavior on both tabs. Stay out of unrelated existing gate-status tests.
""",
    ),
    task(
        "attempt-detail",
        source(f"{MODAL}/detail.rs", (1, 243), (436, 533), (680, 1102),
               (1370, 1649), (1650, 2035), (2104, 2377), (2548, 2639))
        + source(f"{MODAL}/inspector.rs", (1, 219), (320, 358))
        + source(f"{MODAL}/run.rs", (552, 611))
        + source(APP_MODAL, (775, 839))
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
        source(CHAT, (1, 120), (1124, 1256), (1370, 1388), (1820, 2018),
               (2099, 2247), (2288, 2345), (2370, 2410), (2463, 2528), (2529, 2640))
        + source(CHAT_DRIVER, (394, 530), (1499, 1527), (2168, 2219)),
        rf"""
Ownership: {CHAT} and {CHAT_DRIVER}, including their tests. Edit nothing else.

Goal: in Grok ACPX chat a bare /open or bare /flow shows a pick list instead
of the help text. Today parse_command returns Command::Help for both; the
attached parse_command_unknown_slash_and_acpx_help test asserts that and must
change for these two commands only.

Bare /open: show one question through the existing x.ai/ask_user_question
machinery (ask_question_raw, shaped like ask_gate_question, noFreeform true)
listing the 20 most recent runs from driver::list_runs on runs_dir(), newest
first. Option id and label are the run ID. The description carries what the
operator needs to choose: flow name, title when it differs from the flow name,
status, and start time formatted with the existing helper. Choosing a run does
exactly what `/open <run_id>` does now; move the current Open arm into one
helper that both paths call, keeping its "Already shown above." and
not-found replies. Dismissing the question, an unknown choice, or a cancelled
session opens nothing and sends one short line. No runs: reply
"No acpx runs found." without a question. Opening stays inspection only.

Bare /flow: list the configured default_flow() first when it exists, then the
distinct flowPath values from the manifests of the 20 most recent runs, newest
use first, each with the flow name, last status and last start time of its
most recent run. Add one driver function for that read next to list_runs,
reusing read_json_file, json_str and parse_ts; skip manifests without a
flowPath. Choosing a flow starts nothing yet: ask a second question for the
task text (free-form allowed, same machinery). A nonempty task then starts
exactly what `/flow <path> <task>` starts today through the existing Flow arm
logic, including the .workflow.ts route; an empty or dismissed answer starts
nothing and says so. No candidates: reply with the help text as today.

Keep `/open <id>`, `/flow <file> ...`, /runs, /answer (bare /answer still
Help) and plain-text handling unchanged. Update available_commands and
help_markdown so both commands say the argument is optional. Keep the
question's toolCallId distinct from gate tool call IDs.

Tests: parse_command returns the new bare variants for "/open" and "/flow"
(and still Help for "/runs extra" and bare "/answer"); build the option lists
through pure functions you add and test them: newest-first order, run ID as id
and label, description content, title omitted when equal to the flow name,
duplicate flowPath collapsed to its newest run, default flow first and not
repeated, manifests without flowPath skipped (driver test with temp dirs like
the attached test_list_runs_order_and_title_fallback), and the empty cases.
""",
    ),
    task(
        "acpx-landing",
        source(APP_MODAL, (1, 242), (291, 376), (484, 541), (631, 668),
               (730, 895), (1123, 1193))
        + source(f"{MODAL}/ledger.rs", (105, 168), (259, 310), (377, 445))
        + source(f"{ACPX}/bundle.rs", (261, 295)),
        """
Produce an operator-facing first-screen prototype, not a product patch.
Inputs: the current AcpxModalState routes and full Flows and Agent Turns lists in
this packet; a genuinely waiting run asking 'Approve integration?', a
failed beta8-merge run with cargo_check exit 101, and a running fixture.
The screen's first job is to identify work needing attention, open a run,
reach the full lists, or find how to start work. Opening is inspection only.
Do not invent a start route if the attached sources do not establish one.

Design against the current, ungrouped ledger shown here; grouping finished
runs is a separate packet. Which runs need attention depends on a separate
gate-state contract (for example, an answer the engine accepted while the run
still reports waiting). State the attention predicate you assume and mark it
as a dependency on that contract, not a decision.

Return one proposed screen at 80x24, the same state at 50x14, and an empty
case. Give the exact Tab/Shift+Tab, arrow, Enter, and Esc transitions, return
focus when closing a run, and an explicit route to both full lists. Name
which current mode, render branch, and handlers would change. Recommend
whether the full ledger keeps its compact preview and justify the choice
using these cases; a deferred preview packet waits on this recommendation.
Separate any design assumptions from observed behavior.
No changes to the current full ledger, gate authority, or keymap outside
this proposed entry route. List the interactive checks a reviewer can run.
""",
        design=True,
    ),
    task(
        "cross-surface-gate",
        source(GATE, (1, 373), (475, 813))
        + source(APP_MODAL, (377, 538), (1464, 1563))
        + source(f"{MODAL}/run.rs", (1143, 1195))
        + source(f"{MODAL}/detail.rs", (244, 271))
        + source(f"{MODAL}/ledger.rs", (310, 389))
        + source(CHAT, (679, 780), (895, 1049), (1124, 1217), (1257, 1350),
                 (1389, 1527), (1839, 1957), (2099, 2158))
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
decision and the PIW route from native ACPX replay; note which current
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
