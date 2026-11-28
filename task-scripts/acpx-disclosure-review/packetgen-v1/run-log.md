# Packet-writer pilot: can a one-shot worker write the slice packet?

Task chosen: packet 7, inspector body cursor and range (contract in `acpx-disclosure-packets.md`, 3b landed in `92f80438`).
Generator: `gemini-3.8-flash-high` via `slices acp` (zero tools; the runner rejects any tool call).
Implementer (step 4): the existing path, `gemini-4-neon` medium, run only if the returned packet passes review.

## Preparation (wall clock, 2026-11-27)

| Step | Clock |
|---|---|
| Request received | 21:42 |
| Read the runbook, packets index, one rendered packet, the task file's `task()` shape, `agy models` | 21:42 to 21:44 |
| Bundle built (`build_bundle.sh`, `gen-tasks.py`) and dry-rendered | 21:44:04 to 21:45:29 |

Bundle: `_bundle/objective.md` (2.4 KB, verbatim packet 7 text and the shared key contract), `_bundle/repo-map.md` (13.6 KB, generated item outline with line numbers for three files), 7 verbatim source ranges of `inspector.rs`, `detail.rs` and `acpx_modal.rs`, and 4 exemplar ranges of `acpx-disclosure-slices.py` (helpers, `attempt-detail`, `bare-command-picker`, `detail-focus`: task text plus its range list). Rendered: 13 excerpts, 61.4 KB, about 15.7k input tokens.

Scout allowance used: 0 tool calls (none allowed). The repository map is the substitute for discovery.

## Open before dispatch

- `slices acp` checks `effective.thought_level` against `--thought-level`. Whether `gemini-3.8-flash-high` reports a `thought_level` option is not known until a call is made.
- The runner's scratch directory comes from `tempfile.mkdtemp`; the packets index requires it repo-local, so the live run sets `TMPDIR` to a folder under `packetgen-v1/`.
