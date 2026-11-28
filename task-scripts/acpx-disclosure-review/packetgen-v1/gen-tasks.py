"""One-shot packet writer for packet 7 (inspector body cursor and range).

Loads the existing task file for its helpers and SYSTEM-style conventions, and
defines one task whose worker returns a task packet, not an implementation.
Loading this file runs nothing.
"""

import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent
BASE_PATH = HERE.parents[1] / "acpx-disclosure-slices.py"
_spec = importlib.util.spec_from_file_location("acpx_disclosure_slices", BASE_PATH)
base = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(base)

REPOS = base.REPOS
BUNDLE = "task-scripts/acpx-disclosure-review/packetgen-v1/_bundle"
OUT = "task-scripts/acpx-disclosure-review/packetgen-v1/inspector-focus.packet.md"
SLICES = "task-scripts/acpx-disclosure-slices.py"
MODAL = base.MODAL
APP_MODAL = base.APP_MODAL

SYSTEM = """Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. Treat the attached excerpts as evidence;
lines outside the excerpts are not available to you. Do not invent function
names, line numbers or files that the repository map and excerpts do not show."""

SPEC = f"""
You write task packets for other one-shot workers. You do not implement the
task. A worker you never talk to will receive exactly: a fixed system note, the
source excerpts you list, and the task text you write. It has no tools and
cannot ask questions. The attached examples in {SLICES} (attempt-detail,
bare-command-picker, detail-focus) are packets that were dispatched and landed;
match their shape. The objective for your packet is in objective.md. The
repository map gives every item with its line number.

Write the packet for packet 7 (inspector body cursor and range). The worker
will return an exact old/new edit list; the harness adds the edit-list format
and the return block to your task text automatically, so do not write them.

Your task text must contain, as the examples do:
- Ownership: the files, and the exact functions, structs, constants and tests the
  worker may edit; what it must not edit and why.
- Goal in the operator's terms, with the acceptance sentence.
- The behaviour contract as exact keys and states: what each key does with the
  body focused and unfocused, what Esc does at each step, what the footer and
  help show, and what must keep its current meaning.
- Representation guidance only where the code forces a choice. Say which landed
  3b types and methods in detail.rs to reuse or mirror, by their real names.
- Tests: the new tests in the existing styles, and which existing tests change
  (for example copy_returns_active_tab_full_text) and how.
- Anything the worker would otherwise have to guess, stated once.

Your source list is the evidence the worker will see. Every OLD block the worker
quotes must come from it. List ranges as one entry per line in the form
`grok:<repo-relative path>#L<start>-<end>`. Each range must start at an item's
first line (include its doc comment) and end at the line before the next item
in the map, so whole functions, structs and tests are included. Include every
item the worker must replace, every anchor it inserts beside, the read-only
helpers it must call, and the tests it must change. Each range is at most 32 KB.
Keep the list as small as the task allows.

If something the worker needs is not in the map or the excerpts, do not guess:
put it under QUESTIONS.

Return exactly the file block below. The markdown has five sections in this order:
- `SLICE_ID:` one line, kebab-case.
- `SOURCE_RANGES:` the entries, one per line, nothing else.
- `SPEC:` the complete task text for the worker.
- `RANGE_NOTES:` one line per range saying which items it covers and why.
- `QUESTIONS:` open questions for the coordinator, or `none`.
"""

CONTEXT = [
    f"grok:{BUNDLE}/objective.md",
    f"grok:{BUNDLE}/repo-map.md",
    # Evidence: the inspector state, key handler and renderer as they are now.
    f"grok:{MODAL}/inspector.rs#L17-78",
    f"grok:{MODAL}/inspector.rs#L224-440",
    f"grok:{MODAL}/inspector.rs#L581-690",
    # Evidence: the landed 3b copy units and panel focus, to mirror.
    f"grok:{MODAL}/detail.rs#L40-330",
    # Evidence: how the modal routes inspector keys and lists them.
    f"grok:{APP_MODAL}#L634-725",
    f"grok:{APP_MODAL}#L750-775",
    f"grok:{APP_MODAL}#L1004-1014",
    # Examples: helpers, and three landed packets (ranges plus text).
    f"grok:{SLICES}#L14-90",
    f"grok:{SLICES}#L236-292",
    f"grok:{SLICES}#L293-361",
    f"grok:{SLICES}#L489-573",
]

TASKS = [
    {
        "custom_id": "packet-writer-inspector-focus",
        "context_files": CONTEXT,
        "output_files": [f"grok:{OUT}"],
        "prompt": f"Slice ID: packet-writer-inspector-focus\n\n{SPEC.strip()}\n\n"
        f"Return exactly:\n=== FILE: {OUT} ===\n```markdown\n<complete packet file>\n```",
    }
]
