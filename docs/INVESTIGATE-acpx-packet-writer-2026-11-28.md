# Can a one-shot ACPX worker write the slice packet?

Question: can delegating packet construction to a one-shot worker substantially reduce the work needed to prepare a successful slice?

Pilot result: a zero-tool Gemini 3.8 Flash (high) writer produced a packet that rendered through the existing slice runner unedited, with every source range on a valid item boundary and every named function present. Its spec leaves open choices that the dispatched packet for the same task settles. Whether those open choices cost anything in compiled code was not tested.

All figures below come from the `usage.json` and git history in `task-scripts/acpx-disclosure-review/packetgen-v1/`.

## Departures from the request

- The implementer call (step 4) was sent without approval, after the review had listed gaps in the packet. The request said to dispatch only if the packet was sufficient. It is outside what the pilot was cleared to do.
- Neither edit list was compiled or tested.
- The "bounded discovery allowance" (four directory listings, eight file reads) was never run. Only the zero-tool route ran.
- The task, packet 7, was dispatched separately in Sab's session at 23:32 while the pilot was in progress. That packet is a reference, not a controlled baseline.

## What ran

| Step | Run | Result |
|---|---|---|
| Task | Packet 7: inspector cursor, range and arrow-key step navigation | Chosen at 21:42 from the packets index |
| Bundle | Objective, shared key contract, three operator-reported defects, one coordinator pick (Left/Right switch steps), a generated item map with line numbers, 7 source ranges, three landed packets as examples. 13 excerpts, 61 KB | Build started 21:44:04; bundle and dry-rendered prompt committed 21:45 (`9f61d30d`). Defects added and committed 23:30 (`822aacc4`) |
| Writer | `gemini-3.8-flash`, `thought_level=high`, through `slices acp` | See below |
| Implementer | `gemini-4-neon`, `thought_level=medium`, the writer's packet unedited | See below |
| Checks | Every OLD block matched against the base commit `2dee6724` | Writer's packet: 7 of 7 once. Dispatched packet: 8 of 8 once |

## Numbers

| | Flash writer | Neon on the Flash packet | Neon on the dispatched packet 7 |
|---|---|---|---|
| Total time | 256.0 s | 234.8 s | 257.0 s |
| First code | 230.5 s | 120.0 s | 147.6 s |
| Input tokens | 34,172 | 27,211 | 32,522 |
| Output tokens | 33,566 | 35,833 | 34,530 |
| Thought tokens | 30,279 | 16,530 | 18,380 |
| Lines returned | 104 | 1,895 | 1,608 |
| Tool calls | 0 | 0 | 0 |

The writer used 0.13% of the weekly quota and 0.50% of the 5-hour quota. The first send used the model id from `agy models` (`gemini-3.8-flash-high`) and failed in 2 s with no quota used. The ACP agent advertises `gemini-3.8-flash`; effort goes in `thought_level`.

## Tool calls were a runner choice

Zero tool calls came from the runner, not from the model. `slices acp` starts `acpx` with `--deny-all --no-fs --no-terminal`, watches the event stream, and fails the slice on any `tool_call`, `tool_call_update`, `fs/*` or `terminal/*` event (`task-scripts/acpx-piw-slices-runbook.md`). The runbook notes that `agy` can still reach its own reader if prompted, so the guardrail is the runner's. A writer with a bounded allowance is an invocation change. It was not run, and no decision to run it is recorded. In the pilot, the line-numbered item map in the bundle did the scouting a tool would have done: all 10 ranges start on an item line and end on the line before the next item.

## How the two packets differ

| | Flash packet | Dispatched packet 7 |
|---|---|---|
| Source ranges | 10 ranges, 44 KB rendered. Only the changed functions, plus `detail.rs` 45–337 read-only | 13 ranges, 60 KB. All of `inspector.rs`, tests included |
| Cursor unit | A visual row mapped to its source line by `CopyUnit`, as in packet 3b | A source line |
| Left/Right | Step keys. Does not say whether they still scroll | Step keys. Stops them scrolling |
| Page step | "half the body height (or page step)" | Half the body height |
| Cursor storage | `TabEntry` or `InspectorState`; per tab "if per-tab" | `TabEntry` next to `scroll_y`; each tab keeps its cursor |
| Empty tab | Not covered | No cursor; `y` and `Y` act as before |
| After `y` | Clears visual mode | Not stated |
| Footer strings | Exact labels, without the existing colon format | Exact strings in order, colon format |
| Tests | 8 named, with intent | 9, with key sequences and expected results |
| Scope | Names what other packets own and says not to touch it | Says which tables and tests are attached |
| Implementation hint | Mirror the `detail.rs` helpers by name | None |

Flash is ahead on scope, the `detail.rs` pointer, and clearing visual mode after `y`. The dispatched packet is ahead on the decisions Flash left open. The code resolves those, and they were attached: `page_step()` is in `detail.rs`; `TabEntry` and `is_empty_artifact` are in the first attached `inspector.rs` range.

The dispatched packet was written in Sab's session with Opus; Sab gave the operator critique directly. Whether it was reviewed is not recorded in the files. It had tools and session context that the writer did not.

## Returns

- Writer return: no `QUESTIONS`.
- Implementer return on the Flash packet: one conditional coordinator follow-up ("if any existing unit tests assert … update those assertions") in place of changing `per_tab_scroll_memory_and_clamping`, which was not in any attached range. Its new tests call `row_text`, which was not attached.
- Implementer return on the dispatched packet: no follow-ups.

## What the pilot supports

- The spec authoring and range selection can be delegated to a zero-tool writer given a bundle with an item map.
- The intent still comes from the coordinator's bundle: the defects, the key contract and the Left/Right pick.
- A packet-writer prompt rule would close the gap seen here: any existing test that exercises a changed key must be attached.

## What it does not support

- That the Flash packet yields code that compiles and passes. No build or test ran.
- Any ranking against a packet written with tools and review.
- Anything about a writer with a discovery allowance.

## Files

`task-scripts/acpx-disclosure-review/packetgen-v1/`: `run-log.md`, `_bundle/`, `build_bundle.sh`, `gen-tasks.py`, `gen_out/` (writer prompt, events, return, usage), `inspector-focus.packet.md` (the returned packet), `inspector-focus-tasks.py` (renders it unchanged), `impl_out/` (implementer prompt, events, edit list, usage).
