#!/usr/bin/env python3
"""
acpx-repair.py: DRAFT repair slices for the acpx mode and /acpx modal defects in .handoffs (pi-workflows 001).

A repair slice differs from a build slice (acpx-mode.py, acpx-tui.py):
  - the model is given the LIVE file from this checkout, whole and uncut (several were fixed by hand after they
    came back, so the staged copy in *_out/ is not the code);
  - it is told the cause of the defect at file:line, not only what was seen on screen;
  - it returns ONE unified diff against that one file, not the file.

Nothing is applied to the source tree. The diff lands in task-scripts/acpx-repair_out/<id>.diff, the raw return in
acpx-repair_out/_returns/<id>.md, and the runner reports whether `git apply --check` accepts it.

A slice whose "open" list is not empty cannot be fired: those are choices Sab has not made yet. Settle a choice by
writing it into the slice's "Behavior" text and emptying "open".

  python3 task-scripts/acpx-repair.py --dry-run          slices, open choices, prompt sizes, estimate; no API call
  python3 task-scripts/acpx-repair.py --show ledger      print the exact prompt one slice would send
  python3 task-scripts/acpx-repair.py --only agent       fire one slice
  python3 task-scripts/acpx-repair.py                    fire every slice that has no open choice

Not slices (existing pager files, not slice output; edited by hand): the dock pinning steps (app/agent_view/panes.rs
`dock_subagent_rows`), the preview cap setting (scrollback/blocks/subagent.rs `PREVIEW_ROWS` + appearance/cache.rs), and
the key that opens the modal (actions/mod.rs, actions/defaults.rs, app/agent_view/input.rs).
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
// views/acpx_modal/ledger.rs  (slice `ledger` adds this; slice `modal` fills it)
/// The gate this run answered: read from this run's own input, written by `bundle::gate_input` when a gate answer
/// starts the next flow file. `projections/run.json` -> `input.gate = { "runId": <the waiting run>, "node": .., "choice": .. }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateAnswer {
    pub run_id: String,   // input.gate.runId: the run that was waiting at the gate
    pub choice: String,   // input.gate.choice: the label that was picked
}

pub struct RowExtra {
    pub rail: Vec<NodeState>,
    pub agents: String,
    pub stalled: bool,
    pub question: Option<String>,
    pub answers: Option<GateAnswer>,   // NEW. None when this run's input has no `gate` object
}

Why it lives on the answering run: acpx cannot resume a run that stopped at a gate, so the waiting run's folder never
changes again. `AcpxModal::load_extras` caches each RowExtra keyed on that run's own `updated_ms`; a flag stored on
the waiting run would never refresh. A run's input never changes, so `answers` on the answering run is cache-safe.
The ledger derives "which waiting runs were answered" from all rows' `answers` every time `set_extras` is called.
"""

TASKS = [
    {
        "custom_id": "project",
        "file": f"{BIN}/project.rs",
        "issue": "The mermaid graph in the run heading never marks finished steps.",
        "open": [
            "When is the marked graph drawn: (A) a new graph after every step that finishes, or (B) one graph when the run ends or stops at a gate?",
            "Footer model: should this file also send the current step's model to the pager when an agent step starts (see slice `agent`)?",
        ],
        "prompt": f"""
Repair `project.rs` of the binary grok-acpx-acp (it maps acpx run events to ACP session updates for the pager).

Defect seen on screen: the mermaid graph under the run heading shows every step as "○" for the whole run.

Cause: the graph is built once, in the `DriverEvent::RunStarted` arm of `on_event`, from `let states = HashMap::new();`
so every node gets the default glyph. `mermaid()` has no other caller. The heading is an `AgentMessageChunk`; the pager
only appends to its transcript and cannot edit a message it already drew, so the heading graph cannot be updated later.
`self.plan_states` (`HashMap<String, PlanNodeState>`) already holds the live state of every node, and `taken_path()`
already maps states to glyphs.

Behavior:
- Leave the RunStarted heading as it is (all "○" is correct before anything has run).
- Add a private method that turns `self.plan_states` into the `HashMap<String, char>` that `mermaid()` takes, using
  the same glyph for each state that `taken_path()` uses. A node with no state is left out (mermaid() defaults it).
- <OPEN A or B: emit a parent message holding a ```mermaid fence built from that map (A) at the end of every
  `StepFinished` arm that changes a node's state, or (B) only in the `Waiting` arm and the `RunFinished` arm, placed
  before the existing closing text.>
- The message goes through `self.parent_message(..)` like the heading does.
- Nothing else about the Plan update, `taken_path()` text, subagent blocks or tool calls changes.

Tests (add; keep `run_started_emits_mermaid_message_and_initial_plan` and
`mermaid_renders_glyphs_sanitized_ids_and_edge_labels` passing unchanged):
- after RunStarted then a successful StepFinished for node X, the marked graph that is emitted contains "✓ X" and
  still shows "○" for a node that has not started;
- a failed step's node carries the failed glyph in the marked graph;
- the RunStarted heading still contains only "○" nodes.
{RULES}""",
    },
    {
        "custom_id": "agent",
        "file": f"{BIN}/agent.rs",
        "issue": "The pager footer shows the model as \"unknown\".",
        "open": [
            "What should the footer say: (A) a fixed label \"acpx\", (B) the flow's default model, or (C) the current step's model (needs the `project` slice to send it per step, with \"acpx\" shown between steps)?",
        ],
        "prompt": f"""
Repair `agent.rs` of the binary grok-acpx-acp (the ACP agent the pager talks to).

Defect seen on screen: the pager footer shows the model as "unknown".

Cause: the pager prints "unknown" when the session has no current model. It takes the current model from
`NewSessionResponse.models` / `LoadSessionResponse.models`. `new_session` returns `acp::NewSessionResponse::new(session_id)`
and `load_session` returns `acp::LoadSessionResponse::new()`, both without `models`, so the pager never gets one.
(The `"unknown"` string in driver.rs is a run status and is not involved.)

acp types to use (agent_client_protocol, already imported as `acp`):
    acp::SessionModelState::new(current_model_id: impl Into<acp::ModelId>, available_models: Vec<acp::ModelInfo>)
    acp::ModelInfo::new(model_id: impl Into<acp::ModelId>, name: impl Into<String>)
    acp::NewSessionResponse::new(id).models(state)      acp::LoadSessionResponse::new().models(state)
The pager drops a current model that is not in `available_models`, so the list must contain the current id.

Behavior:
- Add one private function that builds the `acp::SessionModelState` for a session: <OPEN A, B or C decides the id
  and display name; for A and C it is one entry, id "acpx", name "acpx">.
- `new_session` and `load_session` both attach it with `.models(..)`. Nothing else in either function changes.

Tests (add):
- `new_session` returns `models` whose `current_model_id` is not empty and is present in `available_models`;
- `load_session` for an unknown session id returns the same.
{RULES}""",
    },
    {
        "custom_id": "ledger",
        "file": f"{SRC}/views/acpx_modal/ledger.rs",
        "issue": "\"N needs you\" counts gates that were already answered; the gate question is cut off in the run list.",
        "open": [
            "If the run that answered a gate later failed or was cancelled, does the gate still count as answered? (Draft says yes.)",
            "What does an answered gate's row show as its state: \"answered: <choice>\", or something else?",
            "Cut-off question in the list: (A) leave the row one line and rely on the right pane, or (B) wrap the selected row's question onto extra rows when the right pane is hidden?",
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
- Add `GateAnswer` and the `answers` field exactly as above (update `Default` and every place a `RowExtra` is built).
- In `set_extras`, rebuild a private map `answered: HashMap<String, String>` (waiting run id -> choice) from every
  extra whose `answers` is `Some`. When two runs answered the same gate, the one that started later wins.
- A run is "answered" when its status is Waiting and its id is a key of `answered`. An answered run:
  is not counted in "N needs you"; is placed under `SectionKind::Finished` by `run_section`; is not active for
  `is_run_active`; and `flow_state_text` returns <OPEN: "answered: <choice>"> instead of the gate question.
- A Waiting run that is not answered behaves exactly as today.

Defect 2 seen on screen: a long gate question is cut off in a run's row.

Cause: in `render_flow_row` the state text is cut to one line by
`truncate_str(&raw_state_text, flex_budget - 1 - title_col)`, and dropped entirely when `flex_budget <= 17`.

Behavior for defect 2: <OPEN A or B. A: no change in this file. B: when the row is selected and it is an unanswered
Waiting run, the full question is shown wrapped on continuation rows under the row, indented to the state column;
unselected rows stay one line. B changes row heights, so scrolling and hit-testing must use the new heights.>

Tests (add; `render_at_120x14_shows_tabs_needs_you_question_rail_and_count` must still pass):
- two Waiting runs where another run's `answers.run_id` names the older one: the summary reads "1 needs you" and the
  older run is listed under Finished;
- a Waiting run with no answering run is still counted and still under Needs you;
- extras replaced by a set without the answering run: the gate counts as needing you again.
{RULES}""",
    },
    {
        "custom_id": "modal",
        "file": f"{SRC}/views/acpx_modal.rs",
        "issue": "Supplies the data the ledger needs to tell an answered gate from a live one.",
        "open": [],
        "prompt": f"""
Repair `views/acpx_modal.rs` in crate xai-grok-pager (the /acpx modal; it loads run folders and feeds the run list).

Defect seen on screen: the run list counts gates as "needs you" after the user answered them. The list cannot tell,
because nothing passes it the link between an answering run and the gate it answered.

Shared contract (slice `ledger` adds these to ledger.rs; write against them, do not redefine them here):
{ANSWERED_CONTRACT}
Cause in this file: `row_extra` builds a `RowExtra` from `bundle::load_run(&summary.dir)` and never reads `run.input`.

Behavior:
- In `row_extra`, set `answers` from `run.input`: when `input.gate` is an object whose `runId` is a non-empty string,
  `Some(GateAnswer {{ run_id, choice }})` with `choice` from `input.gate.choice` (empty string when absent or not a
  string); otherwise `None`. Read with `serde_json::Value` accessors; never panic on a malformed input.
- `load_extras` and its cache are unchanged: `answers` describes the run that owns the RowExtra, so the existing
  `updated_ms` cache key stays correct.
- Import `GateAnswer` from the same path `RowExtra` is already imported from.

Tests (add): a run folder fixture whose run.json input has `gate.runId` = "r1" and `gate.choice` = "approve" yields
`answers == Some(GateAnswer {{ run_id: "r1".into(), choice: "approve".into() }})`; an input without `gate` yields `None`;
an input where `gate` is a string yields `None`.
{RULES}""",
    },
    {
        "custom_id": "detail",
        "file": f"{SRC}/views/acpx_modal/detail.rs",
        "issue": "The run pane shows the last result and the input as raw JSON.",
        "open": [
            "Layout for a result or input that is a JSON object: draft says one `key  value` row per top-level field, nested values indented under their key, no braces or quotes. Needs a mockup marked up by Sab.",
            "Do the row caps stay (20 for the last result, 12 for the input), or does the pane show everything and scroll?",
        ],
        "prompt": f"""
Repair `views/acpx_modal/detail.rs` in crate xai-grok-pager (the right-hand pane of the /acpx modal).

Defect seen on screen: under "Last result" and "Input" the run pane prints raw pretty-printed JSON.

Cause: in `build_run_lines` both sections call `value_text(&serde_json::Value)`, which returns `to_string_pretty` for
anything that is not a string, and pass the text to `push_wrapped_capped` (20 rows for the last result, 12 for the input).

Behavior:
- Add a private function that turns a `&serde_json::Value` into display lines for a given width:
  <OPEN layout. Draft: a string is wrapped as text; a number, bool or null is one line; an object is one row per
  top-level field as `key  value`, keys in the object's own order and padded to the longest key, a nested object or
  array indented two columns under its key; an array is one row per item prefixed "- "; no braces, brackets, quotes
  or trailing commas anywhere.>
- "Last result" and "Input" in `build_run_lines` use it. <OPEN caps: keep the existing 20 and 12 row caps with the
  existing "… +N more rows" line, or remove them.>
- The step pane and every other caller of `value_text` are unchanged. Copy actions still copy the raw JSON.

Tests (add; `test_run_target_lists_stages_and_waiting_gate_choices` must still pass):
- a last-step output `{{"summary":"all good","files":["a.rs","b.rs"]}}` renders a row containing `summary` and
  `all good`, a row for each file, and no line containing `{{`, `}}`, `[` or `"summary":`;
- a string output renders as wrapped text exactly as before;
- an empty object and `null` render no section at all.
{RULES}""",
    },
    {
        "custom_id": "run_view",
        "file": f"{SRC}/views/acpx_modal/run.rs",
        "issue": "The gate question is cut off in the run view's banner.",
        "open": [
            "Should the banner wrap to as many rows as the question needs, or to a fixed number of rows?",
            "A run whose gate was already answered still shows \"NEEDS YOU\" here. Fix it in this slice too (needs the answered flag passed into the run view), or leave it?",
        ],
        "prompt": f"""
Repair `views/acpx_modal/run.rs` in crate xai-grok-pager (the single-run view of the /acpx modal).

Defect seen on screen: at a gate the banner reads "◐ NEEDS YOU at <node>: <question>" and a long question is cut off.

Cause: `build_banner` returns one `Line` cut by `truncate_str(&text, width)`, and the render code gives the banner
exactly one row.

Behavior:
- `build_banner` returns the banner as wrapped lines (`Vec<Line<'static>>`, empty when there is no banner) using the
  file's existing wrap helper, every line in the same colour and weight as today. <OPEN: no row limit, or a limit.>
- The layout gives the banner as many rows as it returns and takes them from the area below it; with no banner the
  layout is exactly as today. A terminal too short for the banner plus one row of content shows the banner's first
  rows and keeps one content row.
- Stalled, failed and running banners go through the same wrapping.

Tests (add; `render_wide_160x40_shows_banner_stages_separator_and_clean_detail_area` must still pass):
- a Waiting run whose question is 120 characters, rendered 80 wide, shows every word of the question;
- a short question still takes exactly one row and the rows below it are where they are today.
{RULES}""",
    },
]

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
    """The whole live file, uncut, then the task. A missing file is an error, never a silent skip."""
    path = repo_root / task["file"]
    if not path.is_file():
        sys.exit(f"{task['custom_id']}: target file not found: {path}")
    return (
        f"=== THE FILE AS IT IS NOW: `{task['file']}` ===\n\n```rust\n{path.read_text(encoding='utf-8')}\n```\n\n"
        f"=== TASK ===\n{task['prompt']}"
    )


def extract_diff(content: str) -> str:
    content = re.sub(r"^<think>[\s\S]*?</think>\s*", "", (content or "").replace("\r\n", "\n"))
    m = re.search(r"```(?:diff|patch)?[ \t]*\n([\s\S]*)\n```", content)
    body = m.group(1) if m else content
    return body.strip("\n") + "\n" if body.strip() else ""


def check_diff(repo_root: Path, task: dict, diff_path: Path) -> str:
    """Empty string when the diff is acceptable, else the reason. Never changes the working tree."""
    touched = set(re.findall(r"^\+\+\+ b/(\S+)", diff_path.read_text(encoding="utf-8"), flags=re.M))
    if touched != {task["file"]}:
        return f"diff touches {sorted(touched) or 'no file'}, expected only {task['file']}"
    res = subprocess.run(
        ["git", "apply", "--check", "--recount", str(diff_path)], cwd=repo_root, capture_output=True, text=True
    )
    return res.stderr.strip()[:400] if res.returncode else ""


def fire(api_key: str, endpoint: str, model: str, task: dict, repo_root: Path, out: Path) -> dict:
    payload = {
        "model": model,
        "temperature": 0.0,
        "max_tokens": engine.MAX_TOKENS,
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": build_prompt(repo_root, task)}],
    }
    headers = {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}
    start = time.time()
    try:
        res = engine.make_http_request(endpoint, headers=headers, data=payload, method="POST")
        choices = res.get("choices", [])
        if not choices:
            return {"task": task, "ok": False, "error": "no response choices returned"}
        raw = choices[0].get("message", {}).get("content", "") or ""
        returns = out / "_returns" / f"{task['custom_id']}.md"
        returns.parent.mkdir(parents=True, exist_ok=True)
        returns.write_text(raw, encoding="utf-8")
        finish = choices[0].get("finish_reason") or choices[0].get("native_finish_reason")
        if str(finish).lower() in ("length", "max_tokens"):
            return {"task": task, "ok": False, "error": f"TRUNCATED at max_tokens; partial return kept in {returns}"}
        diff = extract_diff(raw)
        if not diff:
            return {"task": task, "ok": False, "error": f"EMPTY response; raw return in {returns}"}
        diff_path = out / f"{task['custom_id']}.diff"
        diff_path.write_text(diff, encoding="utf-8")
        reason = check_diff(repo_root, task, diff_path)
        if reason:
            return {"task": task, "ok": False, "error": f"diff does not apply ({diff_path}): {reason}"}
        return {"task": task, "ok": True, "path": diff_path, "elapsed": time.time() - start, "lines": diff.count("\n")}
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

    if args.show:
        task = next((t for t in TASKS if t["custom_id"] == args.show), None)
        if not task:
            sys.exit(f"--show matched nothing; ids: {ids}")
        print(build_prompt(repo_root, task))
        return

    tasks = [t for t in TASKS if not args.only or t["custom_id"] in args.only.split(",")]
    if not tasks:
        sys.exit(f"--only matched nothing; ids: {ids}")

    if args.dry_run:
        total = 0
        for t in tasks:
            n = len(build_prompt(repo_root, t))
            total += n
            state = f"{len(t['open'])} open" if t["open"] else "ready"
            print(f"  {t['custom_id']:<9} {state:<7} {n:>7,} chars in  {t['file']}\n            {t['issue']}")
            for q in t["open"]:
                print(f"            OPEN: {q}")
        est = total / 4 * engine.PRICE_IN_PER_M / 1e6 + len(tasks) * 3000 * engine.PRICE_OUT_PER_M / 1e6
        print(
            f"\n[dry run] {len(tasks)} slice(s), ~{total // 4:,} tokens in, ~$ {est:.3f} at acpx-mode.py PRICE_* rates "
            f"(assumes ~3k tokens out per diff; an estimate, not a ceiling). Model {args.model}. No API call made."
        )
        return

    blocked = [t["custom_id"] for t in tasks if t["open"]]
    if blocked:
        sys.exit(f"Not fired: open choices remain in {', '.join(blocked)}. Run --dry-run to see them.")

    key, endpoint = engine.resolve_api_config(args.key, args.endpoint)
    if not key:
        sys.exit("API key required via --key, CHEAPERINFERENCE_API_KEY, or OPENROUTER_API_KEY.")
    out = Path(args.output).resolve()
    print(f"Firing {len(tasks)} repair slice(s) on {args.model} ({endpoint}); diffs -> {out}")
    failed = []
    with ThreadPoolExecutor(max_workers=len(tasks)) as pool:
        for fut in as_completed([pool.submit(fire, key, endpoint, args.model, t, repo_root, out) for t in tasks]):
            r = fut.result()
            if r["ok"]:
                print(f"  [ok]   {r['task']['custom_id']}: {r['path']} ({r['lines']} lines, {r['elapsed']:.0f}s), applies cleanly")
            else:
                failed.append(r["task"]["custom_id"])
                print(f"  [FAIL] {r['task']['custom_id']}: {r['error']}")
    if failed:
        sys.exit(f"{len(failed)} of {len(tasks)} failed: {', '.join(failed)}")


if __name__ == "__main__":
    main()
