# Sol: rewrite the opening-screen ledger delta (opening-v3)

From Fable 5.2, 2026-11-27. Base for this note: `321e9b07`. The review it rests on is `opening-v2/fable-5.2-review.md`; the packet reviewed is `opening-v2/_prompts/acpx-opening-ledger-delta.md` (`3dece007`).

## Sab's decision

**An answered wait does not go under "Waiting for input".** Sab agreed on 2026-11-27 to drop "answered Waiting remains Waiting" from the packet and to give the design doc the same exception.

Why: the ledger's "answered" state exists only for native ACPX gates. A native gate is answered by starting a new run whose `input.gate.runId` names the waiting run (`acpx_modal.rs:720`, `ledger.rs:361`), so the answered run reads `waiting` on disk for good. A PIW gate is answered in place (`acpx_modal.rs:537`) and never enters that map. On sab-mini today 9 native waits answered five to seven days ago still read `waiting` (7 `ship`, 2 `fix`). As rendered, Waiting for input would go from 10 runs to 19 and never shrink, the `ship` group would read 12 when 5 need an answer, and the header (`ledger.rs:1355`) would say 10 beside a section of 19.

## What to change

Edit `task-scripts/acpx-disclosure-slices.py` and `docs/acpx-modal-opening-screen.md`, then dry-render into a new `task-scripts/acpx-disclosure-review/opening-v3/`. Leave `opening-v2/` as it is. No excerpt needs to change; all 18 matched the source.

### 1. Answered waits (decided)

`acpx-disclosure-slices.py:178–181`, replace

> Use actual RunStatus: stalled Running remains Running with its stalled label, and answered Waiting remains Waiting with the recorded answer distinction.

with wording to this effect:

> A stalled Running run stays under Running with its stalled label. A Waiting run with a recorded answer (`is_answered`) stays under Recent outcomes as an individual row with its existing `answered: <choice>` label, as `run_section` files it now; it is not counted, grouped or listed under Waiting for input. Add no answered state for pi-workflows waits.

`acpx-disclosure-slices.py:226`, replace "waiting with a recorded answer stays Waiting and stalled Running stays Running" with:

> a waiting run with a recorded answer is listed under Recent outcomes and is absent from the Waiting for input count and from its flow's waiting group (five unanswered `ship` waits plus two answered ones read as a group of 5); stalled Running stays Running

`docs/acpx-modal-opening-screen.md`:

- Line 17 ("Keep a distinct status for answers in flight or accepted while the engine still reports waiting"): add that a native ACPX wait whose answer started a continuation run is listed under Recent outcomes as answered, because the native engine never changes that run's status.
- Line 57 ("The row moves to outcomes only when the engine reports a terminal state"): add the same exception. What an answered pi-workflows wait looks like stays with packet 5.

### 2–4. The other three findings in the review

These are mine, not Sab's rulings. Each is a sentence or two of task text plus a test; the evidence is in the review. Accept or contest each in the v3 disposition.

2. **Waiting and finished groups of one flow fold independently.** Fold state is `flow_expanded_groups` keyed by `flow_name` (`ledger.rs:186`); `mixed-talk` has 2 waiting and 11 finished runs today. Add a test with a flow that has both.
3. **A selected waiting-group header across refresh.** State the rule: the header stays selected by group key; when its group is no longer shown as a group (one member left, a filter, Recent or Name order), the newest member is selected. `reconcile_flow_selection` (`ledger.rs:782`) falls back to the first row today.
4. **Recorded path for runs past `EXTRAS_LIMIT` (80).** Say the path for those runs is read once per run through the cache that already holds their answers (`acpx_modal.rs:328–347`), and that this is not widening the cache limit. There are 75 runs now.

### Noticed while writing this, not in the review

The header summary counts running runs that are not stalled (`ledger.rs:1364`). With stalled runs now under Running, the header and the section count disagree. Say in the task text which one the header follows.

## Hand back

- `opening-v3/_prompts/` and `_packets/` from a dry run (`slices acp … --dry-run --output task-scripts/acpx-disclosure-review/opening-v3`), with `dry-run.log`.
- `opening-v3/review-disposition.md`: each of the five points above as accepted, changed or contested, with the reason.
- The base commit the excerpts were taken from.

Dry render only. No worker, build or product edit. The preview packet stays on hold until this one lands.
